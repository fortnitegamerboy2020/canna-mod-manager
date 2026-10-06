use super::*;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS notifications(id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER NOT NULL REFERENCES users(id),kind TEXT NOT NULL,body TEXT NOT NULL,link TEXT NOT NULL,created INTEGER NOT NULL,read INTEGER NOT NULL DEFAULT 0,dedup TEXT NOT NULL,UNIQUE(user_id,dedup));
 CREATE TABLE IF NOT EXISTS notification_meta(key TEXT PRIMARY KEY);
 CREATE TABLE IF NOT EXISTS notification_dismissals(user_id INTEGER NOT NULL REFERENCES users(id),dedup TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(user_id,dedup));
 CREATE INDEX IF NOT EXISTS notification_user ON notifications(user_id,id);
 CREATE TABLE IF NOT EXISTS mod_submissions(id TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES users(id),name TEXT NOT NULL,version TEXT NOT NULL,created INTEGER NOT NULL,status TEXT NOT NULL DEFAULT 'pending',reason TEXT NOT NULL DEFAULT '',resolved INTEGER);
 CREATE TRIGGER IF NOT EXISTS mod_submission_insert AFTER INSERT ON mods BEGIN INSERT OR IGNORE INTO mod_submissions(id,user_id,name,version,created) VALUES(NEW.id,NEW.user_id,NEW.name,NEW.version,strftime('%s','now')); END;
 ")?;
    if db.execute(
        "INSERT OR IGNORE INTO notification_meta VALUES('submissions-backfilled')",
        [],
    )? == 1
    {
        db.execute("INSERT OR IGNORE INTO mod_submissions(id,user_id,name,version,created) SELECT id,user_id,name,version,strftime('%s','now') FROM mods",[])?;
    }
    Ok(())
}
pub fn notify(
    db: &Connection,
    user: i64,
    kind: &str,
    body: &str,
    link: &str,
    dedup: &str,
) -> rusqlite::Result<()> {
    db.execute("INSERT OR IGNORE INTO notifications(user_id,kind,body,link,created,dedup) SELECT ?1,?2,?3,?4,?5,?6 WHERE NOT EXISTS(SELECT 1 FROM notification_dismissals WHERE user_id=?1 AND dedup=?6)",params![user,kind,body,link,now(),dedup])?;
    db.execute("DELETE FROM notifications WHERE user_id=?1 AND id NOT IN (SELECT id FROM notifications WHERE user_id=?1 ORDER BY id DESC LIMIT 500)",[user])?;
    Ok(())
}
pub fn accepted(db: &Connection, id: &str) -> ApiResult<()> {
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT user_id,name FROM mod_submissions WHERE id=?1 AND status<>'accepted'",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((user, name)) = row {
        db.execute(
            "UPDATE mod_submissions SET status='accepted',reason='',resolved=?1 WHERE id=?2",
            params![now(), id],
        )?;
        notify(
            db,
            user,
            "mod-accepted",
            &format!("Your mod {name} was accepted."),
            "#submissions",
            &format!("accepted:{id}"),
        )?;
    }
    Ok(())
}
pub async fn list(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT id,kind,body,link,created,read FROM notifications WHERE user_id=?1 ORDER BY id DESC LIMIT 100")?;
    let rows=stmt.query_map([actor],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"body":r.get::<_,String>(2)?,"link":r.get::<_,String>(3)?,"created":r.get::<_,i64>(4)?,"read":r.get::<_,bool>(5)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(rows)))
}
#[derive(Deserialize)]
pub struct Read {
    #[serde(default)]
    id: Option<i64>,
}
pub async fn read(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Read>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    app.db.lock().unwrap().execute(
        "UPDATE notifications SET read=1 WHERE user_id=?1 AND (?2 IS NULL OR id=?2)",
        params![actor, input.id],
    )?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn submissions(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT s.id,s.name,s.version,s.status,s.reason,s.created,s.resolved,EXISTS(SELECT 1 FROM mods WHERE id=s.id),(SELECT status FROM mod_scans WHERE mod_id=s.id) FROM mod_submissions s WHERE s.user_id=?1 AND (s.resolved IS NULL OR s.resolved>?2) ORDER BY s.created DESC LIMIT 500")?;
    let rows=stmt.query_map(params![actor,now()-604800],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"version":r.get::<_,String>(2)?,"status":r.get::<_,String>(3)?,"reason":r.get::<_,String>(4)?,"created":r.get::<_,i64>(5)?,"resolved":r.get::<_,Option<i64>>(6)?,"available":r.get::<_,bool>(7)?,"analysis":r.get::<_,Option<String>>(8)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(rows)))
}
pub async fn clear(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("INSERT OR REPLACE INTO notification_dismissals(user_id,dedup,created) SELECT user_id,dedup,?1 FROM notifications WHERE user_id=?2",params![now(),actor])?;
    let cleared = tx.execute("DELETE FROM notifications WHERE user_id=?1", [actor])?;
    tx.commit()?;
    app.live.hint("notifications");
    Ok(axum::Json(json!({"ok":true,"cleared":cleared})))
}
#[derive(Deserialize)]
pub struct Deny {
    reason: String,
}
pub async fn deny(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<Deny>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, admin) = app.auth(&headers)?;
    if !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator permission required",
        ));
    }
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    let reason = input.reason.trim();
    if reason.len() < 5 || reason.len() > 2000 {
        return Err(bad("Explain the denial in 5–2000 bytes"));
    }
    {
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction()?;
        let (user, name): (i64, String) = tx
            .query_row("SELECT user_id,name FROM mods WHERE id=?1", [&id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "Mod not found"))?;
        tx.execute(
            "UPDATE mod_submissions SET status='denied',reason=?1,resolved=?2 WHERE id=?3",
            params![reason, now(), id],
        )?;
        notify(
            &tx,
            user,
            "mod-denied",
            &format!("Your mod {name} was denied: {reason}"),
            "#submissions",
            &format!("denied:{id}"),
        )?;
        tx.execute("DELETE FROM mods WHERE id=?1", [&id])?;
        tx.execute(
            "INSERT INTO audit(actor,action,target,created) VALUES(?1,'deny-mod',?2,?3)",
            params![actor, json!({"mod":id,"reason":reason}).to_string(), now()],
        )?;
        tx.commit()?;
    }
    let path = app.files.join(format!("{id}.zip"));
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    };
    Ok(axum::Json(json!({"ok":true})))
}
pub fn start(app: Shared) {
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            timer.tick().await;
            let result = (|| -> rusqlite::Result<()> {
                let db = app.db.lock().unwrap();
                db.execute(
                    "DELETE FROM mod_submissions WHERE resolved IS NOT NULL AND resolved<=?1",
                    [now() - 604800],
                )?;
                db.execute(
                    "DELETE FROM notifications WHERE created<=?1",
                    [now() - 30 * 86400],
                )?;
                db.execute(
                    "DELETE FROM notification_dismissals WHERE created<=?1",
                    [now() - 30 * 86400],
                )?;
                let mut stmt = db.prepare("SELECT user_id FROM bot_wallets WHERE daily<?1")?;
                let ids = stmt
                    .query_map([now() / 86400], |r| r.get::<_, i64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                for user in ids {
                    notify(
                        &db,
                        user,
                        "daily-ready",
                        "Your daily 100 Kash are ready. Type /daily in CannaBot chat.",
                        "#chat",
                        &format!("daily:{}", now() / 86400),
                    )?;
                }
                Ok(())
            })();
            if result.is_ok() {
                app.live.hint("notifications");
            }
        }
    });
}
pub async fn replies(State(app): State<Shared>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let reply = request.method() == axum::http::Method::POST
        && path.starts_with("/api/v1/topics/")
        && path.ends_with("/reply");
    let actor = if reply {
        app.auth(request.headers()).ok().map(|v| v.0)
    } else {
        None
    };
    let response = next.run(request).await;
    if response.status().is_success()
        && let Some(actor) = actor
    {
        let id = path
            .trim_start_matches("/api/v1/topics/")
            .trim_end_matches("/reply");
        let db = app.db.lock().unwrap();
        let row:Option<(i64,String,String)>=db.query_row("SELECT t.user_id,t.title,u.username FROM topics t JOIN users u ON u.id=?1 WHERE t.id=?2",params![actor,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().ok().flatten();
        if let Some((owner, title, author)) = row
            && owner != actor
        {
            let last:Option<String>=db.query_row("SELECT id FROM posts WHERE topic_id=?1 ORDER BY created DESC,rowid DESC LIMIT 1",[id],|r|r.get(0)).optional().ok().flatten();
            if let Some(last) = last {
                let _ = notify(
                    &db,
                    owner,
                    "reply",
                    &format!("{author} replied to your discussion: {title}"),
                    &format!("#thread/{id}"),
                    &format!("reply:{last}"),
                );
            }
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn clearing_all_is_private_complete_and_does_not_resurrect_events() {
        let (_dir, app) = fixture();
        let first = account(&app, "first", false);
        let second = account(&app, "second", false);
        {
            let db = app.db.lock().unwrap();
            for n in 0..150 {
                notify(
                    &db,
                    1,
                    "reply",
                    "Message",
                    "/notifications",
                    &format!("event:{n}"),
                )
                .unwrap();
            }
            notify(&db, 2, "reply", "Other member", "/notifications", "other").unwrap();
            db.execute(
                "UPDATE notifications SET read=1 WHERE user_id=1 AND id%2=0",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/notifications/clear",
                json!({}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let result = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/notifications/clear",
                json!({"user_id":2}),
                Some(&first),
            )
            .await,
        )
        .await;
        assert_eq!(result["cleared"], 150);
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/notifications",
                    json!({}),
                    Some(&first)
                )
                .await
            )
            .await,
            json!([])
        );
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/notifications",
                    json!({}),
                    Some(&second)
                )
                .await
            )
            .await
            .as_array()
            .unwrap()
            .len(),
            1
        );
        let db = app.db.lock().unwrap();
        notify(&db, 1, "reply", "Old event", "/notifications", "event:0").unwrap();
        notify(&db, 1, "reply", "New event", "/notifications", "new-event").unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM notifications WHERE user_id=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn replies_and_submission_decisions_are_private_and_retained_seven_days() {
        let (_dir, app) = fixture();
        let author = account(&app, "author", false);
        let other = account(&app, "other", false);
        let staff = account(&app, "staff", true);
        let topic=value(call(app.clone(),"POST","/api/v1/topics",json!({"title":"My question","body":"Please reply","category":"help","app_id":1686940}),Some(&author)).await).await;
        assert!(
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/topics/{}/reply", topic["id"].as_str().unwrap()),
                json!({"body":"A helpful reply"}),
                Some(&other)
            )
            .await
            .status()
            .is_success()
        );
        let mine = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/notifications",
                Value::Null,
                Some(&author),
            )
            .await,
        )
        .await;
        assert_eq!(mine.as_array().unwrap().len(), 1);
        assert_eq!(mine[0]["kind"], "reply");
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/notifications",
                    Value::Null,
                    Some(&other)
                )
                .await
            )
            .await,
            json!([])
        );
        call(
            app.clone(),
            "POST",
            "/api/v1/notifications/read",
            json!({"id":mine[0]["id"]}),
            Some(&other),
        )
        .await;
        let mine = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/notifications",
                Value::Null,
                Some(&author),
            )
            .await,
        )
        .await;
        assert_eq!(mine[0]["read"], false);
        let id = Uuid::new_v4().to_string();
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1686940,'Submission','1','','hash',10)",
                [&id],
            )
            .unwrap();
        }
        let path = format!("/api/v1/mods/{id}/deny");
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &path,
                json!({"reason":"Missing source explanation"}),
                Some(&other)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &path,
                json!({"reason":""}),
                Some(&staff)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(
            call(
                app.clone(),
                "POST",
                &path,
                json!({"reason":"Missing source explanation"}),
                Some(&staff)
            )
            .await
            .status()
            .is_success()
        );
        let mine = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/submissions",
                Value::Null,
                Some(&author),
            )
            .await,
        )
        .await;
        assert_eq!(mine[0]["status"], "denied");
        assert_eq!(mine[0]["reason"], "Missing source explanation");
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/submissions",
                    Value::Null,
                    Some(&other)
                )
                .await
            )
            .await,
            json!([])
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_submissions SET resolved=?1", [now() - 604800])
            .unwrap();
        assert_eq!(
            value(
                call(
                    app,
                    "GET",
                    "/api/v1/submissions",
                    Value::Null,
                    Some(&author)
                )
                .await
            )
            .await,
            json!([])
        );
    }
}
