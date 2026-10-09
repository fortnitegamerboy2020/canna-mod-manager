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
pub async fn history(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT body FROM chat_messages WHERE user_id=?1 AND bot=0 AND created>?2 ORDER BY id DESC LIMIT 50")?;
    let mut rows = stmt
        .query_map(params![actor, now() - 86400], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    rows.reverse();
    Ok(axum::Json(json!(rows)))
}
pub async fn tip_recipients(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(page): Query<lists::Page>,
) -> ApiResult<axum::Json<Value>> {
    let actor = app.auth(&headers)?.0;
    let term = page.term();
    let term = term.trim().strip_prefix('@').unwrap_or(term.trim());
    let db = app.db.lock().unwrap();
    let rows=db.prepare("SELECT id,username FROM users WHERE verified=1 AND banned=0 AND id<>?1 AND substr(lower(username),1,length(?2))=lower(?2) ORDER BY username COLLATE NOCASE,id LIMIT 8")?.query_map(params![actor,term],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(rows)))
}
#[derive(Deserialize)]
pub struct Message {
    body: String,
    #[serde(default)]
    request_id: Option<String>,
}
pub async fn send(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Message>,
) -> ApiResult<axum::Json<Value>> {
    let actor = gambling::mutation_actor(&app, &headers)?;
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
    let tip = body.split_whitespace().next() == Some("/tip");
    let request = if tip {
        let request = input
            .request_id
            .as_deref()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 80
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            })
            .ok_or_else(|| bad("Tips require a request_id; refresh Canna before tipping"))?;
        let cached: Option<(String, i64)> = tx
            .query_row(
                "SELECT body,message_id FROM bot_tip_requests WHERE user_id=?1 AND request_id=?2",
                params![actor, request],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((previous, id)) = cached {
            if previous != body {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "This tip request was already used for another message",
                ));
            }
            return Ok(axum::Json(json!({"id":id,"replayed":true})));
        }
        Some(request)
    } else {
        None
    };
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
    if let Some(request) = request {
        tx.execute(
            "INSERT INTO bot_tip_requests VALUES(?1,?2,?3,?4,?5)",
            params![actor, request, body, id, now()],
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
    #[tokio::test]
    async fn tip_autocomplete_is_private_prefix_only_bounded_and_excludes_inactive_members() {
        let (_dir, app) = fixture();
        let member = account(&app, "autocomplete-owner", false);
        {
            let db = app.db.lock().unwrap();
            db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<20) INSERT INTO users(username,password,verified,role) SELECT printf('Alpha%02d',x),'fixture',1,'member' FROM n; INSERT INTO users(username,password,verified,role,banned) VALUES('AlphaBanned','fixture',1,'member',1),('AlphaUnverified','fixture',0,'member',0),('XAlpha','fixture',1,'member',0);").unwrap();
        }
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/chat/tip-recipients?search=Al",
                Value::Null,
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let rows = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/chat/tip-recipients?search=%40al",
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(rows.as_array().unwrap().len(), 8);
        assert!(
            rows.as_array().unwrap().iter().all(|r| r["username"]
                .as_str()
                .unwrap()
                .starts_with("Alpha")
                && r.as_object().unwrap().len() == 2)
        );
        let rows = value(
            call(
                app,
                "GET",
                "/api/v1/chat/tip-recipients?search=autocomplete",
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(rows, json!([]));
    }
    #[tokio::test]
    async fn tips_are_conserved_atomic_audited_and_retried_without_chat_duplicates() {
        let (_dir, app) = fixture();
        let sender = account(&app, "tip-sender", false);
        account(&app, "tip-target", false);
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO bot_wallets(user_id,balance) VALUES(1,100),(2,10)",
                [],
            )
            .unwrap();
        }
        let input = json!({"body":"/tip @TIP-target 25","request_id":"safe-tip"});
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                input.clone(),
                Some(&sender),
            )
            .await,
        )
        .await;
        let replay =
            value(call(app.clone(), "POST", "/api/v1/chat", input, Some(&sender)).await).await;
        assert_eq!(first["id"], replay["id"]);
        assert_eq!(replay["replayed"], true);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/chat",
                json!({"body":"/tip @tip-target 26","request_id":"safe-tip"}),
                Some(&sender)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        for (i, body) in [
            "/tip @tip-sender 1",
            "/tip @missing 1",
            "/tip @tip-target 1000",
            "/tip @tip-target -1",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/chat",
                    json!({"body":body,"request_id":format!("bad-tip-{i}")}),
                    Some(&sender)
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        let db = app.db.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT sum(balance) FROM bot_wallets", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            110
        );
        assert_eq!(
            db.query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            75
        );
        assert_eq!(
            db.query_row("SELECT sum(earned) FROM bot_wallets", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM chat_messages", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM audit WHERE action='kash_tip'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn tips_reject_unverified_banned_and_overflow_without_changing_sender() {
        let (_dir, app) = fixture();
        let sender = account(&app, "tip-limits", false);
        account(&app, "tip-blocked", false);
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO bot_wallets(user_id,balance) VALUES(1,100),(2,?1)",
                [cannabot::MAX_KASH],
            )
            .unwrap();
        }
        for (i, update) in [
            "UPDATE users SET verified=1,banned=0 WHERE id=2",
            "UPDATE users SET banned=1 WHERE id=2",
            "UPDATE users SET banned=0,verified=0 WHERE id=2",
        ]
        .into_iter()
        .enumerate()
        {
            app.db.lock().unwrap().execute(update, []).unwrap();
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/chat",
                    json!({"body":"/tip @tip-blocked 1","request_id":format!("limits-{i}")}),
                    Some(&sender)
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
                "/api/v1/chat",
                json!({"body":"/tip @tip-blocked 1"}),
                Some(&sender)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT balance FROM bot_wallets WHERE user_id=1", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            100
        );
    }
    #[tokio::test]
    async fn history_returns_only_own_unexpired_sent_messages() {
        let (_dir, app) = fixture();
        let actor = account(&app, "history", false);
        account(&app, "otherhistory", false);
        {
            let db = app.db.lock().unwrap();
            for (user, body, bot, created) in [
                (1, "mine", false, now()),
                (2, "other", false, now()),
                (1, "bot", true, now()),
                (1, "expired", false, now() - 86401),
            ] {
                db.execute(
                    "INSERT INTO chat_messages(user_id,body,bot,created) VALUES(?1,?2,?3,?4)",
                    params![user, body, bot, created],
                )
                .unwrap();
            }
        }
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/chat/history", json!({}), None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            value(
                call(
                    app,
                    "GET",
                    "/api/v1/chat/history?user_id=2",
                    json!({}),
                    Some(&actor)
                )
                .await
            )
            .await,
            json!(["mine"])
        );
    }
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
