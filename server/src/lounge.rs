use super::*;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS chat_messages(id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER NOT NULL REFERENCES users(id),body TEXT NOT NULL,created INTEGER NOT NULL,bot INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS chat_created ON chat_messages(created);
        CREATE TABLE IF NOT EXISTS chat_quotas(user_id INTEGER PRIMARY KEY REFERENCES users(id),day INTEGER NOT NULL,count INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS bot_flip_limits(user_id INTEGER PRIMARY KEY REFERENCES users(id),day INTEGER NOT NULL,count INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS announcement(id INTEGER PRIMARY KEY CHECK(id=1),body TEXT NOT NULL,active INTEGER NOT NULL,revision INTEGER NOT NULL);
        INSERT OR IGNORE INTO announcement VALUES(1,'',0,0);")
}
fn staff(app: &App, headers: &HeaderMap) -> ApiResult<i64> {
    let (actor, admin) = app.auth(headers)?;
    if !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator permission required",
        ));
    }
    Ok(actor)
}
fn purge(db: &Connection) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM chat_messages WHERE created<=?1",
        [now() - 86400],
    )?;
    db.execute("DELETE FROM chat_quotas WHERE day<?1", [now() / 86400])?;
    Ok(())
}
pub fn start_cleanup(app: Shared) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = purge(&app.db.lock().unwrap()) {
                eprintln!("Chat retention cleanup failed: {error}");
            }
        }
    });
}
pub async fn messages(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    purge(&db)?;
    let mut stmt=db.prepare("SELECT c.id,c.user_id,u.username,u.role,c.body,c.created,c.bot,COALESCE(w.badge,'none') FROM chat_messages c JOIN users u ON u.id=c.user_id LEFT JOIN bot_wallets w ON w.user_id=c.user_id WHERE c.created>?1 ORDER BY c.id DESC LIMIT 100")?;
    let mut rows=stmt.query_map([now()-86400], |r|Ok(json!({"id":r.get::<_,i64>(0)?,"user_id":r.get::<_,i64>(1)?,"username":r.get::<_,String>(2)?,"role":r.get::<_,String>(3)?,"body":r.get::<_,String>(4)?,"created":r.get::<_,i64>(5)?,"bot":r.get::<_,bool>(6)?,"badge":r.get::<_,String>(7)?})))?.collect::<Result<Vec<_>,_>>()?;
    rows.reverse();
    Ok(axum::Json(json!(rows)))
}
#[derive(Deserialize)]
pub struct Message {
    body: String,
}
pub async fn send(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Message>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    app.limits.check(format!("chat:{actor}"), 10)?;
    let body = input.body.trim();
    if body.is_empty()
        || body.chars().count() > 1000
        || body
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(bad("Chat messages must contain 1–1000 characters"));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    purge(&tx)?;
    let day = now() / 86400;
    tx.execute("INSERT INTO chat_quotas VALUES(?1,?2,0) ON CONFLICT(user_id) DO UPDATE SET day=excluded.day,count=CASE WHEN day=excluded.day THEN count ELSE 0 END",params![actor,day])?;
    let count: i64 = tx.query_row(
        "SELECT count FROM chat_quotas WHERE user_id=?1",
        [actor],
        |r| r.get(0),
    )?;
    if count >= 500 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Daily chat limit reached",
        ));
    }
    let total: i64 = tx.query_row("SELECT COUNT(*) FROM chat_messages", [], |r| r.get(0))?;
    if total >= 9998 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Chat is busy; try again later",
        ));
    }
    let reply = cannabot::run(&tx, actor, body)?;
    tx.execute(
        "UPDATE chat_quotas SET count=count+1 WHERE user_id=?1",
        [actor],
    )?;
    tx.execute(
        "INSERT INTO chat_messages(user_id,body,created) VALUES(?1,?2,?3)",
        params![actor, body, now()],
    )?;
    let id = tx.last_insert_rowid();
    if let Some(reply) = reply {
        tx.execute(
            "INSERT INTO chat_messages(user_id,body,created,bot) VALUES(?1,?2,?3,1)",
            params![actor, reply, now()],
        )?;
    }
    tx.commit()?;
    Ok(axum::Json(json!({"id":id})))
}
pub async fn remove(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, admin) = app.auth(&headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let author: i64 = tx
        .query_row("SELECT user_id FROM chat_messages WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "Chat message not found"))?;
    if actor != author && !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "You can only delete your own messages",
        ));
    }
    tx.execute("DELETE FROM chat_messages WHERE id=?1", [id])?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'chat-delete',?2,?3)",
        params![actor, id.to_string(), now()],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn clear(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let actor = staff(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("DELETE FROM chat_messages", [])?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'chat-clear','global',?2)",
        params![actor, now()],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn announcement(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let result=db.query_row("SELECT body,active,revision FROM announcement WHERE id=1",[],|r|Ok(json!({"body":r.get::<_,String>(0)?,"active":r.get::<_,bool>(1)?,"revision":r.get::<_,i64>(2)?})))?;
    Ok(axum::Json(result))
}
#[derive(Deserialize)]
pub struct Announcement {
    body: String,
    active: bool,
    revision: i64,
}
pub async fn announce(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Announcement>,
) -> ApiResult<axum::Json<Value>> {
    let actor = staff(&app, &headers)?;
    let body = input.body.trim();
    if body.chars().count() > 2000
        || (input.active && body.is_empty())
        || body.chars().any(|c| c.is_control() && c != '\n')
    {
        return Err(bad(
            "An active announcement needs text, up to 2000 characters",
        ));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute(
        "UPDATE announcement SET body=?1,active=?2,revision=revision+1 WHERE id=1 AND revision=?3",
        params![body, input.active, input.revision],
    )? != 1
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Announcement changed; reload before editing",
        ));
    }
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'announcement',?2,?3)",
        params![
            actor,
            if input.active { "published" } else { "hidden" },
            now()
        ],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn chat_retention_authorization_and_announcement_revision() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let member = account(&app, "member", false);
        let other = account(&app, "other", false);
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/chat", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let v = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                json!({"body":"<script>hello</script>"}),
                Some(&member),
            )
            .await,
        )
        .await;
        let id = v["id"].as_i64().unwrap();
        assert_eq!(
            call(
                app.clone(),
                "DELETE",
                &format!("/api/v1/chat/{id}"),
                Value::Null,
                Some(&other)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE chat_messages SET created=?1", [now() - 86400])
            .unwrap();
        let rows = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/chat",
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(rows, json!([]));
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/chat/clear",
                json!({}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let banner = json!({"body":"Welcome","active":true,"revision":0});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/announcement",
                banner.clone(),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/announcement",
                banner.clone(),
                Some(&owner)
            )
            .await
            .status()
            .is_success()
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/announcement",
                banner,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let banner = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/announcement",
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(banner["body"], "Welcome");
    }
    #[tokio::test]
    async fn chat_rate_limit_and_persisted_daily_limit() {
        let (_dir, app) = fixture();
        let member = account(&app, "member", false);
        for _ in 0..10 {
            assert!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/chat",
                    json!({"body":"hello"}),
                    Some(&member)
                )
                .await
                .status()
                .is_success()
            );
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                json!({"body":"hello"}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        let other = account(&app, "other", false);
        {
            let db = app.db.lock().unwrap();
            let id: i64 = db
                .query_row("SELECT id FROM users WHERE username='other'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            db.execute(
                "INSERT INTO chat_quotas VALUES(?1,?2,500)",
                params![id, now() / 86400],
            )
            .unwrap();
        }
        assert_eq!(
            call(
                app,
                "POST",
                "/api/v1/chat",
                json!({"body":"hello"}),
                Some(&other)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
