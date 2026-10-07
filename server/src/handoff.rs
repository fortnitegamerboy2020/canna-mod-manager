use super::*;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS download_tickets(hash TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES users(id),kind TEXT NOT NULL,item TEXT NOT NULL,expires INTEGER NOT NULL,state TEXT NOT NULL DEFAULT 'waiting',receipt TEXT UNIQUE); CREATE TABLE IF NOT EXISTS desktop_pairings(request TEXT PRIMARY KEY,proof TEXT NOT NULL,user_id INTEGER REFERENCES users(id),expires INTEGER NOT NULL,state TEXT NOT NULL DEFAULT 'waiting'); CREATE TABLE IF NOT EXISTS pairing_codes(request TEXT PRIMARY KEY REFERENCES desktop_pairings(request) ON DELETE CASCADE,hash TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS pairing_devices(request TEXT PRIMARY KEY REFERENCES desktop_pairings(request) ON DELETE CASCADE,name TEXT NOT NULL);")
}
#[derive(Deserialize)]
pub struct PairRequest {
    pub request: String,
    #[serde(default)]
    pub code: String,
}
#[derive(Deserialize)]
pub struct PairPoll {
    pub request: String,
    pub proof: String,
}
#[derive(Deserialize, Default)]
pub struct PairStart {
    #[serde(default)]
    name: String,
}
pub async fn pair_start(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<PairStart>,
) -> ApiResult<axum::Json<Value>> {
    let name = input.name.trim();
    if name.len() > 80 || name.chars().any(char::is_control) {
        return Err(bad("Invalid device name"));
    }
    let name = if name.is_empty() { "Windows PC" } else { name };
    let request = token();
    let proof = token();
    let code = token()[..6].to_uppercase();
    let db = app.db.lock().unwrap();
    db.execute("DELETE FROM pairing_codes WHERE request IN (SELECT request FROM desktop_pairings WHERE expires<?1)", [now()])?;
    db.execute("DELETE FROM desktop_pairings WHERE expires<?1", [now()])?;
    let count: i64 = db.query_row("SELECT COUNT(*) FROM desktop_pairings", [], |r| r.get(0))?;
    if count >= 100 {
        return Err(bad("Too many connection requests; try again shortly"));
    }
    db.execute(
        "INSERT INTO desktop_pairings(request,proof,expires) VALUES(?1,?2,?3)",
        params![digest(&request), digest(&proof), now() + 300],
    )?;
    db.execute(
        "INSERT INTO pairing_codes(request,hash) VALUES(?1,?2)",
        params![digest(&request), digest(&format!("{request}:{code}"))],
    )?;
    db.execute(
        "INSERT INTO pairing_devices VALUES(?1,?2)",
        params![digest(&request), name],
    )?;
    Ok(axum::Json(
        json!({"request":request,"proof":proof,"code":code,"expires":now()+300,"url":format!("https://cannamods.vip/connect?request={request}")}),
    ))
}
pub async fn pair_approve(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<PairRequest>,
) -> ApiResult<StatusCode> {
    let (user, _) = app.auth(&headers)?;
    valid(&input.request)?;
    app.limits.check(format!("pair-member:{user}"), 10)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let key = digest(&input.request);
    let row: Option<(String, i64)> = tx.query_row("SELECT c.hash,c.attempts FROM pairing_codes c JOIN desktop_pairings p ON p.request=c.request WHERE p.request=?1 AND p.state='waiting' AND p.expires>?2", params![key,now()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((expected, attempts)) = row else {
        return Err(bad(
            "Connection expired, locked or already used. Start again in Canna.",
        ));
    };
    let code = input.code.trim().to_ascii_uppercase();
    if code.len() != 6
        || !code.bytes().all(|b| b.is_ascii_hexdigit())
        || digest(&format!("{}:{code}", input.request)) != expected
    {
        let attempts = attempts + 1;
        tx.execute(
            "UPDATE pairing_codes SET attempts=?1 WHERE request=?2",
            params![attempts, key],
        )?;
        if attempts >= 5 {
            tx.execute(
                "UPDATE desktop_pairings SET state='locked' WHERE request=?1",
                [&key],
            )?;
            tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'pairing-bruteforce-blocked',?2,?3)",params![user,key,now()])?;
        }
        tx.commit()?;
        return Err(bad(if attempts >= 5 {
            "Too many incorrect codes. Connection locked; start again in Canna."
        } else {
            "Incorrect connection code. Copy the code shown in Canna."
        }));
    }
    tx.execute(
        "UPDATE desktop_pairings SET user_id=?1,state='approved' WHERE request=?2",
        params![user, key],
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn pair_poll(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<PairPoll>,
) -> ApiResult<axum::Json<Value>> {
    valid(&input.request)?;
    valid(&input.proof)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let row:Option<(String,Option<i64>)>=tx.query_row("SELECT state,user_id FROM desktop_pairings WHERE request=?1 AND proof=?2 AND expires>?3",params![digest(&input.request),digest(&input.proof),now()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((state, user)) = row else {
        return Err(denied());
    };
    if state == "waiting" {
        return Ok(axum::Json(json!({"state":"waiting"})));
    }
    if state != "approved" {
        return Err(denied());
    }
    let user = user.ok_or_else(denied)?;
    let active: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND verified=1 AND banned=0)",
        [user],
        |r| r.get(0),
    )?;
    if !active {
        return Err(denied());
    }
    let raw = token();
    tx.execute(
        "UPDATE desktop_pairings SET state='claimed' WHERE request=?1",
        [digest(&input.request)],
    )?;
    let name: String = tx
        .query_row(
            "SELECT name FROM pairing_devices WHERE request=?1",
            [digest(&input.request)],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| "Windows PC".into());
    devices::record(&tx, &raw, user, &name, "desktop")?;
    tx.commit()?;
    Ok(axum::Json(
        json!({"state":"connected","token":raw,"expires":null}),
    ))
}
#[derive(Deserialize)]
pub struct Create {
    pub kind: String,
    pub id: String,
}
pub async fn connect(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    Err(bad(
        "Start account connection in the updated Canna app and enter its code on /connect.",
    ))
}
pub async fn connect_claim(
    State(_app): State<Shared>,
    axum::Json(_input): axum::Json<Claim>,
) -> ApiResult<axum::Json<Value>> {
    Err(denied())
}
#[derive(Deserialize)]
pub struct Claim {
    pub ticket: String,
}
#[derive(Deserialize)]
pub struct Receipt {
    pub receipt: String,
    #[serde(default)]
    pub success: bool,
}
fn valid(s: &str) -> ApiResult<()> {
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(bad("Invalid download ticket"))
    }
}
pub async fn create(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Create>,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    Uuid::parse_str(&input.id).map_err(|_| bad("Invalid library item"))?;
    let db = app.db.lock().unwrap();
    let table = match input.kind.as_str() {
        "mods" => "mods",
        "packs" => "packs",
        _ => return Err(bad("Invalid download type")),
    };
    let exists: bool = db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1)"),
        [&input.id],
        |r| r.get(0),
    )?;
    if input.kind == "mods" {
        security::approved(&db, &input.id)?;
    }
    if !exists {
        return Err(ApiError(StatusCode::NOT_FOUND, "Library item not found"));
    }
    db.execute("DELETE FROM download_tickets WHERE expires<?1", [now()])?;
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM download_tickets WHERE user_id=?1",
        [user],
        |r| r.get(0),
    )?;
    if count >= 10 {
        return Err(bad("Too many pending downloads; wait a few minutes"));
    }
    if input.kind == "mods" && catalog::project_key(&external::details(&db, &input.id)?).is_some() {
        subscriptions::subscribe(&db, user, &input.id)?;
    }
    let raw = token();
    db.execute(
        "INSERT INTO download_tickets(hash,user_id,kind,item,expires) VALUES(?1,?2,?3,?4,?5)",
        params![digest(&raw), user, input.kind, input.id, now() + 120],
    )?;
    Ok(axum::Json(
        json!({"ticket":raw,"uri":format!("canna://download/{raw}")}),
    ))
}
pub async fn status(
    State(app): State<Shared>,
    Path(raw): Path<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    valid(&raw)?;
    let state: Option<String> = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT state FROM download_tickets WHERE hash=?1 AND user_id=?2 AND expires>?3",
            params![digest(&raw), user, now()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(axum::Json(
        json!({"state":state.unwrap_or_else(||"expired".into())}),
    ))
}
fn descriptor(db: &Connection, kind: &str, id: &str) -> ApiResult<Value> {
    if kind == "mods" {
        security::approved(db, id)?;
        let mut v=db.query_row("SELECT name,version,app_id,sha256,size,description FROM mods WHERE id=?1",[id],|r|Ok(json!({"name":r.get::<_,String>(0)?,"version":r.get::<_,String>(1)?,"app_id":r.get::<_,u32>(2)?,"sha256":r.get::<_,String>(3)?,"size":r.get::<_,i64>(4)?,"description":r.get::<_,String>(5)?}))).optional()?.ok_or(ApiError(StatusCode::NOT_FOUND,"Mod was removed"))?;
        let details = external::details(db, id)?;
        v["filename"] = json!(
            details["filename"]
                .as_str()
                .map(external::safe_filename)
                .unwrap_or_else(|| format!("{id}.zip"))
        );
        v["kind"] = json!(kind);
        v["id"] = json!(id);
        Ok(v)
    } else {
        let raw: String = db
            .query_row("SELECT manifest FROM packs WHERE id=?1", [id], |r| r.get(0))
            .optional()?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "Pack was removed"))?;
        Ok(
            json!({"kind":"packs","id":id,"filename":"Shared-Modpack.canna.json","sha256":digest(&raw),"size":raw.len()}),
        )
    }
}
pub async fn claim(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<Claim>,
) -> ApiResult<axum::Json<Value>> {
    valid(&input.ticket)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let row:Option<(String,String)>=tx.query_row("SELECT t.kind,t.item FROM download_tickets t JOIN users u ON u.id=t.user_id WHERE t.hash=?1 AND t.kind IN ('mods','packs') AND t.expires>?2 AND t.state='waiting' AND u.verified=1 AND u.banned=0",params![digest(&input.ticket),now()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (kind, id) = row.ok_or_else(denied)?;
    let mut info = descriptor(&tx, &kind, &id)?;
    let receipt = token();
    info["receipt"] = json!(receipt);
    tx.execute(
        "UPDATE download_tickets SET state='connected',receipt=?1,expires=?2 WHERE hash=?3",
        params![digest(&receipt), now() + 300, digest(&input.ticket)],
    )?;
    tx.commit()?;
    Ok(axum::Json(info))
}
pub async fn transfer(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<Receipt>,
) -> ApiResult<Response> {
    valid(&input.receipt)?;
    let (kind, id) = {
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction()?;
        let row:Option<(String,String)>=tx.query_row("SELECT t.kind,t.item FROM download_tickets t JOIN users u ON u.id=t.user_id WHERE t.receipt=?1 AND t.expires>?2 AND t.state='connected' AND u.verified=1 AND u.banned=0",params![digest(&input.receipt),now()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let row = row.ok_or_else(denied)?;
        tx.execute(
            "UPDATE download_tickets SET state='downloading' WHERE receipt=?1",
            [digest(&input.receipt)],
        )?;
        tx.commit()?;
        row
    };
    if kind == "mods" {
        let size: i64 =
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT size FROM mods WHERE id=?1", [&id], |r| r.get(0))?;
        security::approved(&app.db.lock().unwrap(), &id)?;
        provider_cache::ensure(&app, &id).await?;
        let file = tokio::fs::File::open(app.files.join(format!("{id}.zip"))).await?;
        Ok((
            [
                ("content-type", "application/octet-stream".into()),
                ("content-length", size.to_string()),
            ],
            Body::from_stream(crypto::read(file, Zeroizing::new(*app.upload_key), id)),
        )
            .into_response())
    } else {
        let raw: String = app.db.lock().unwrap().query_row(
            "SELECT manifest FROM packs WHERE id=?1",
            [&id],
            |r| r.get(0),
        )?;
        Ok(([("content-type", "application/json")], raw).into_response())
    }
}
pub async fn complete(
    State(app): State<Shared>,
    axum::Json(input): axum::Json<Receipt>,
) -> ApiResult<StatusCode> {
    valid(&input.receipt)?;
    let count=app.db.lock().unwrap().execute("UPDATE download_tickets SET state=?1 WHERE receipt=?2 AND state='downloading' AND expires>?3",params![if input.success {"complete"} else {"failed"},digest(&input.receipt),now()])?;
    if count == 0 {
        return Err(denied());
    }
    Ok(StatusCode::NO_CONTENT)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pairing_bruteforce_locks_even_when_correct_code_is_later_supplied() {
        let (_dir, app) = fixture();
        let user = account(&app, "pair-lock-user", false);
        let start = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/start",
                json!({}),
                None,
            )
            .await,
        )
        .await;
        assert!(
            !start["url"]
                .as_str()
                .unwrap()
                .contains(start["code"].as_str().unwrap())
        );
        let stored: String = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT hash FROM pairing_codes WHERE request=?1",
                [digest(start["request"].as_str().unwrap())],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(stored, start["code"].as_str().unwrap());
        let wrong = if start["code"] == "000000" {
            "111111"
        } else {
            "000000"
        };
        for _ in 0..5 {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/desktop/approve",
                    json!({"request":start["request"],"code":wrong}),
                    Some(&user)
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/approve",
                json!({"request":start["request"],"code":start["code"]}),
                Some(&user)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/poll",
                json!({"request":start["request"],"proof":start["proof"]}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let count: i64 = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action='pairing-bruteforce-blocked'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn pairing_requires_browser_approval_private_proof_and_single_use() {
        let (_dir, app) = fixture();
        let user = account(&app, "pairing-user", false);
        let start = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/start",
                json!({}),
                None,
            )
            .await,
        )
        .await;
        let request = json!({"request":start["request"],"code":start["code"]});
        let poll = json!({"request":start["request"],"proof":start["proof"]});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/approve",
                request.clone(),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let state = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/poll",
                poll.clone(),
                None,
            )
            .await,
        )
        .await;
        assert_eq!(state["state"], "waiting");
        assert!(state.get("token").is_none());
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/poll",
                json!({"request":start["request"],"proof":token()}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/approve",
                request.clone(),
                Some(&user)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/approve",
                request,
                Some(&user)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let connected = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/poll",
                poll.clone(),
                None,
            )
            .await,
        )
        .await;
        assert_eq!(connected["state"], "connected");
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/me",
                json!(null),
                connected["token"].as_str()
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/desktop/poll", poll, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let start = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/start",
                json!({}),
                None,
            )
            .await,
        )
        .await;
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE desktop_pairings SET expires=0", [])
            .unwrap();
        assert_eq!(
            call(
                app,
                "POST",
                "/api/v1/desktop/poll",
                json!({"request":start["request"],"proof":start["proof"]}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn account_connections_require_sign_in_and_cannot_be_used_as_file_tickets() {
        let (_dir, app) = fixture();
        let user = account(&app, "connector", false);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/connect",
                json!({}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/desktop/connect",
                json!({}),
                Some(&user)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app,
                "POST",
                "/api/v1/desktop/claim",
                json!({"ticket":token()}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn completed_transfers_preserve_checksum_and_cannot_replay() {
        let (_dir, app) = fixture();
        let owner = account(&app, "sender", true);
        let bytes = b"PK\x03\x04test fixture";
        let user = app
            .auth(&HeaderMap::from_iter([(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {owner}").parse().unwrap(),
            )]))
            .unwrap()
            .0;
        let id = external::store(
            &app,
            user,
            1686940,
            "Fixture",
            "1.0",
            "",
            "fixture",
            &json!({"filename":"fixture.zip"}),
            bytes,
        )
        .await
        .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_reviews SET approved=1 WHERE mod_id=?1", [&id])
            .unwrap();
        let ticket = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets",
                json!({"kind":"mods","id":id}),
                Some(&owner),
            )
            .await,
        )
        .await;
        let info = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets/claim",
                json!({"ticket":ticket["ticket"]}),
                None,
            )
            .await,
        )
        .await;
        let receipt = json!({"receipt":info["receipt"]});
        let response = call(
            app.clone(),
            "POST",
            "/api/v1/download-tickets/transfer",
            receipt.clone(),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let data = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(&data[..], bytes);
        assert_eq!(info["sha256"], hex::encode(Sha256::digest(data)));
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets/transfer",
                receipt.clone(),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets/complete",
                json!({"receipt":info["receipt"],"success":true}),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            value(
                call(
                    app,
                    "GET",
                    &format!(
                        "/api/v1/download-tickets/{}",
                        ticket["ticket"].as_str().unwrap()
                    ),
                    Value::Null,
                    Some(&owner)
                )
                .await
            )
            .await["state"],
            "complete"
        );
    }
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn tickets_are_scoped_single_use_and_revocation_is_checked() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let other = account(&app, "other", false);
        let id = Uuid::new_v4().to_string();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO packs SELECT ?1,id,'Test',?2 FROM users WHERE username='owner'",
                params![id, json!({"game":{"app_id":1686940}}).to_string()],
            )
            .unwrap();
        let t = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets",
                json!({"kind":"packs","id":id}),
                Some(&owner),
            )
            .await,
        )
        .await;
        let raw = t["ticket"].as_str().unwrap();
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    &format!("/api/v1/download-tickets/{raw}"),
                    Value::Null,
                    Some(&other)
                )
                .await
            )
            .await["state"],
            "expired"
        );
        let claim = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets/claim",
                json!({"ticket":raw}),
                None,
            )
            .await,
        )
        .await;
        assert!(claim["receipt"].as_str().is_some());
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets/claim",
                json!({"ticket":raw}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET banned=1 WHERE username='owner'", [])
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets/transfer",
                json!({"receipt":claim["receipt"]}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
