use super::*;
use axum::extract::FromRequest;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS support_challenges(hash TEXT PRIMARY KEY,client TEXT NOT NULL,created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS support_tickets(id TEXT PRIMARY KEY,secret TEXT NOT NULL,user_id INTEGER REFERENCES users(id),client TEXT NOT NULL,subject TEXT NOT NULL,category TEXT NOT NULL,status TEXT NOT NULL DEFAULT 'open',created INTEGER NOT NULL,updated INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS support_client_time ON support_tickets(client,created);
    CREATE TABLE IF NOT EXISTS support_messages(id TEXT PRIMARY KEY,ticket TEXT NOT NULL REFERENCES support_tickets(id) ON DELETE CASCADE,staff INTEGER NOT NULL,body TEXT NOT NULL,created INTEGER NOT NULL);")
}
fn client_key(app: &App, request: &Request) -> String {
    digest(&format!(
        "{}:{}",
        hex::encode(*app.upload_key),
        security::client(request)
    ))
}
fn same_origin(headers: &HeaderMap) -> ApiResult<()> {
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Open the support form on Canna",
        ));
    }
    if let Some(origin) = headers.get("origin")
        && origin != "https://cannamods.vip"
        && origin != "https://api.cannamods.vip"
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Open the support form on Canna",
        ));
    }
    Ok(())
}
pub async fn challenge(
    State(app): State<Shared>,
    request: Request,
) -> ApiResult<axum::Json<Value>> {
    same_origin(request.headers())?;
    let client = client_key(&app, &request);
    app.limits
        .check(format!("support-challenge:{client}"), 10)?;
    let seed = token();
    let db = app.db.lock().unwrap();
    db.execute(
        "DELETE FROM support_challenges WHERE created<?1",
        [now() - 600],
    )?;
    let count: i64 = db.query_row("SELECT COUNT(*) FROM support_challenges", [], |r| r.get(0))?;
    if count >= 2000 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Support is busy; try again later",
        ));
    }
    db.execute(
        "INSERT INTO support_challenges VALUES(?1,?2,?3)",
        params![digest(&seed), client, now()],
    )?;
    Ok(axum::Json(
        json!({"seed":seed,"difficulty":3,"expires_in":600}),
    ))
}
#[derive(Deserialize)]
pub struct Submission {
    seed: String,
    nonce: u64,
    subject: String,
    body: String,
    category: String,
    #[serde(default)]
    website: String,
    #[serde(default)]
    fax: String,
}
pub async fn create(State(app): State<Shared>, request: Request) -> ApiResult<axum::Json<Value>> {
    same_origin(request.headers())?;
    let client = client_key(&app, &request);
    let user = app.auth(request.headers()).ok().map(|v| v.0);
    let axum::Json(input) = axum::Json::<Submission>::from_request(request, &app)
        .await
        .map_err(|_| bad("Invalid ticket form"))?;
    app.limits.check(format!("support-submit:{client}"), 5)?;
    let id = Uuid::new_v4().to_string();
    let secret = token();
    // Return the same shape for honeypot submissions, but store nothing.
    if !input.website.is_empty() || !input.fax.is_empty() {
        return Ok(axum::Json(json!({"id":id,"key":secret})));
    }
    if input.seed.len() != 64
        || !digest(&format!("{}:{}", input.seed, input.nonce)).starts_with("000")
    {
        return Err(bad("Refresh the support form and retry its verification"));
    }
    if !(3..=140).contains(&input.subject.trim().len())
        || !(10..=10000).contains(&input.body.trim().len())
        || !["help", "invite", "bug", "other"].contains(&input.category.as_str())
    {
        return Err(bad(
            "Use a 3–140 character subject and a 10–10000 character message",
        ));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let created = tx
        .query_row(
            "SELECT created FROM support_challenges WHERE hash=?1 AND client=?2",
            params![digest(&input.seed), client],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .ok_or_else(|| bad("Support verification expired; refresh the form"))?;
    if now() - created < 2 || now() - created > 600 {
        return Err(bad(
            "Please wait a moment, or refresh an expired support form",
        ));
    }
    let (hour,day,total):(i64,i64,i64)=tx.query_row("SELECT (SELECT COUNT(*) FROM support_tickets WHERE client=?1 AND created>?2),(SELECT COUNT(*) FROM support_tickets WHERE created>?3),(SELECT COUNT(*) FROM support_tickets)",params![client,now()-3600,now()-86400],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if hour >= 3 || day >= 200 || total >= 10000 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Support submission limit reached; please try later",
        ));
    }
    tx.execute(
        "DELETE FROM support_challenges WHERE hash=?1",
        [digest(&input.seed)],
    )?;
    tx.execute("INSERT INTO support_tickets(id,secret,user_id,client,subject,category,created,updated) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",params![id,digest(&secret),user,client,input.subject.trim(),input.category,now()])?;
    tx.execute(
        "INSERT INTO support_messages VALUES(?1,?2,0,?3,?4)",
        params![Uuid::new_v4().to_string(), id, input.body.trim(), now()],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"id":id,"key":secret})))
}
fn authorize(app: &App, headers: &HeaderMap, id: &str) -> ApiResult<bool> {
    let (user, admin) = app
        .auth(headers)
        .map(|v| (Some(v.0), v.1))
        .unwrap_or((None, false));
    let key = headers
        .get("x-canna-ticket")
        .and_then(|v| v.to_str().ok())
        .filter(|s| s.len() == 64);
    let db = app.db.lock().unwrap();
    let row = db
        .query_row(
            "SELECT secret,user_id FROM support_tickets WHERE id=?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?;
    if let Some((secret, owner)) = row
        && (admin || (user.is_some() && user == owner) || key.is_some_and(|s| digest(s) == secret))
    {
        return Ok(admin);
    }
    Err(ApiError(
        StatusCode::NOT_FOUND,
        "Ticket not found; use its private tracking link or sign in",
    ))
}
fn items(db: &Connection, sql: &str, user: Option<i64>) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = db.prepare(sql)?;
    let map = |r: &rusqlite::Row<'_>| {
        Ok(
            json!({"id":r.get::<_,String>(0)?,"subject":r.get::<_,String>(1)?,"category":r.get::<_,String>(2)?,"status":r.get::<_,String>(3)?,"updated":r.get::<_,i64>(4)?}),
        )
    };
    if let Some(user) = user {
        stmt.query_map([user], map)?.collect()
    } else {
        stmt.query_map([], map)?.collect()
    }
}
pub async fn mine(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    Ok(axum::Json(json!(items(
        &app.db.lock().unwrap(),
        "SELECT id,subject,category,status,updated FROM support_tickets WHERE user_id=?1 ORDER BY updated DESC LIMIT 100",
        Some(user)
    )?)))
}
pub async fn queue(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (_, admin) = app.auth(&headers)?;
    if !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator permission required",
        ));
    }
    Ok(axum::Json(json!(items(
        &app.db.lock().unwrap(),
        "SELECT id,subject,category,status,updated FROM support_tickets ORDER BY status='open' DESC,updated DESC LIMIT 200",
        None
    )?)))
}
pub async fn read(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    authorize(&app, &headers, &id)?;
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT staff,body,created FROM support_messages WHERE ticket=?1 ORDER BY created,rowid LIMIT 200")?;
    let messages=stmt.query_map([&id],|r|Ok(json!({"staff":r.get::<_,bool>(0)?,"body":r.get::<_,String>(1)?,"created":r.get::<_,i64>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
    let mut v=db.query_row("SELECT subject,category,status FROM support_tickets WHERE id=?1",[&id],|r|Ok(json!({"subject":r.get::<_,String>(0)?,"category":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?})))?;
    v["messages"] = json!(messages);
    Ok(axum::Json(v))
}
#[derive(Deserialize)]
pub struct Reply {
    body: String,
}
pub async fn reply(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<Reply>,
) -> ApiResult<StatusCode> {
    same_origin(&headers)?;
    let staff = authorize(&app, &headers, &id)?;
    if !(1..=10000).contains(&input.body.trim().len()) {
        return Err(bad("Reply must contain 1–10000 characters"));
    }
    app.limits.check(format!("support-reply:{id}"), 10)?;
    let actor = if staff {
        Some(app.auth(&headers)?.0)
    } else {
        None
    };
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let (state,count):(String,i64)=tx.query_row("SELECT status,(SELECT COUNT(*) FROM support_messages WHERE ticket=?1) FROM support_tickets WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if state == "closed" || count >= 200 {
        return Err(bad(
            "This ticket is closed or its reply limit has been reached",
        ));
    }
    tx.execute(
        "INSERT INTO support_messages VALUES(?1,?2,?3,?4,?5)",
        params![
            Uuid::new_v4().to_string(),
            id,
            staff,
            input.body.trim(),
            now()
        ],
    )?;
    tx.execute(
        "UPDATE support_tickets SET updated=?1 WHERE id=?2",
        params![now(), id],
    )?;
    if let Some(actor) = actor {
        tx.execute(
            "INSERT INTO audit(actor,action,target,created) VALUES(?1,'reply-ticket',?2,?3)",
            params![actor, id, now()],
        )?;
    }
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct Status {
    status: String,
}
pub async fn status(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<Status>,
) -> ApiResult<StatusCode> {
    let (actor, admin) = app.auth(&headers)?;
    if !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator permission required",
        ));
    }
    if !["open", "closed"].contains(&input.status.as_str()) {
        return Err(bad("Invalid ticket status"));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute(
        "UPDATE support_tickets SET status=?1,updated=?2 WHERE id=?3",
        params![input.status, now(), id],
    )? != 1
    {
        return Err(bad("Ticket not found"));
    }
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'ticket-status',?2,?3)",
        params![actor, id, now()],
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn delete(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let actor = community::owner(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute(
        "DELETE FROM support_tickets WHERE id=?1 AND status='closed'",
        [&id],
    )? != 1
    {
        return Err(bad("Close the ticket before deleting it"));
    }
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'delete-ticket',?2,?3)",
        params![actor, id, now()],
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    use tower::ServiceExt;
    async fn form(app: Shared) -> Value {
        let v = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/support/challenge",
                Value::Null,
                None,
            )
            .await,
        )
        .await;
        let seed = v["seed"].as_str().unwrap();
        let nonce = (0_u64..)
            .find(|n| digest(&format!("{seed}:{n}")).starts_with("000"))
            .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE support_challenges SET created=?1", [now() - 3])
            .unwrap();
        json!({"seed":seed,"nonce":nonce,"subject":"Need an invitation","category":"invite","body":"Can I request an invitation to Canna?"})
    }
    #[tokio::test]
    async fn guest_tickets_are_private_and_single_use() {
        let (_dir, app) = fixture();
        let member = account(&app, "ticket-member", false);
        let admin = account(&app, "ticket-admin", true);
        let input = form(app.clone()).await;
        let made = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/support/tickets",
                input.clone(),
                None,
            )
            .await,
        )
        .await;
        let path = format!("/api/v1/support/tickets/{}", made["id"].as_str().unwrap());
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, Some(&member))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        let req = Request::builder()
            .uri(&path)
            .header("x-canna-ticket", made["key"].as_str().unwrap())
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router(app.clone()).oneshot(req).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/support/tickets", input, None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/tickets",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, Some(&admin))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!("{path}/reply"),
                json!({"body":"We can help."}),
                Some(&admin)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!(
                    "/api/v1/admin/tickets/{}/status",
                    made["id"].as_str().unwrap()
                ),
                json!({"status":"closed"}),
                Some(&admin)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(
                app,
                "POST",
                &format!("{path}/reply"),
                json!({"body":"Blocked reply"}),
                Some(&admin)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn traps_invalid_proof_and_persistent_limits_reject_spam() {
        let (_dir, app) = fixture();
        let mut input = form(app.clone()).await;
        input["website"] = json!("spam.example");
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/support/tickets",
                input.clone(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM support_tickets", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        input["website"] = json!("");
        let seed = input["seed"].as_str().unwrap();
        let bad_nonce = (0_u64..)
            .find(|n| !digest(&format!("{seed}:{n}")).starts_with("000"))
            .unwrap();
        let mut invalid = input.clone();
        invalid["nonce"] = json!(bad_nonce);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/support/tickets",
                invalid,
                None
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let client: String = app
            .db
            .lock()
            .unwrap()
            .query_row("SELECT client FROM support_challenges LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        {
            let db = app.db.lock().unwrap();
            for n in 0..3 {
                db.execute("INSERT INTO support_tickets VALUES(?1,'secret',NULL,?2,'fixture','help','open',?3,?3)",params![n.to_string(),client,now()]).unwrap();
            }
        }
        assert_eq!(
            call(app, "POST", "/api/v1/support/tickets", input, None)
                .await
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let mut headers = HeaderMap::new();
        headers.insert("origin", "https://evil.example".parse().unwrap());
        assert!(same_origin(&headers).is_err());
    }
}
