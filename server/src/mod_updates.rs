use super::*;

const COOLDOWN: i64 = 120;
const PERIOD: i64 = 86400;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS mod_update_control(id INTEGER PRIMARY KEY CHECK(id=1),manual_at INTEGER NOT NULL); INSERT OR IGNORE INTO mod_update_control VALUES(1,0); CREATE TABLE IF NOT EXISTS mod_update_state(source TEXT PRIMARY KEY,checked INTEGER NOT NULL,status TEXT NOT NULL,detail TEXT NOT NULL,mod_id TEXT NOT NULL);")
}
pub async fn request(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let last: i64 = tx.query_row(
        "SELECT manual_at FROM mod_update_control WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if now() < last + COOLDOWN {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Update checks have a shared 120-second cooldown",
        ));
    }
    tx.execute(
        "UPDATE mod_update_control SET manual_at=?1 WHERE id=1",
        [now()],
    )?;
    tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'request-mod-updates','All supported external projects',?2)",params![actor,now()])?;
    tx.commit()?;
    Ok(axum::Json(json!({"queued":true,"retry_at":now()+COOLDOWN})))
}
pub async fn status(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let last: i64 = db.query_row(
        "SELECT manual_at FROM mod_update_control WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    let rows=db.prepare("SELECT checked,status,detail,mod_id FROM mod_update_state ORDER BY checked DESC LIMIT 50")?.query_map([],|r|Ok(json!({"checked":r.get::<_,i64>(0)?,"status":r.get::<_,String>(1)?,"detail":r.get::<_,String>(2)?,"mod_id":r.get::<_,String>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(
        json!({"retry_at":last+COOLDOWN,"checks":rows,"interval_seconds":30,"automatic_period_seconds":PERIOD}),
    ))
}
fn next(app: &App) -> ApiResult<Option<(String, String, i64)>> {
    let db = app.db.lock().unwrap();
    let manual: i64 = db.query_row(
        "SELECT manual_at FROM mod_update_control WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    let mut stmt=db.prepare("SELECT m.id,m.user_id,d.data FROM mods m JOIN mod_details d ON d.mod_id=m.id JOIN users u ON u.id=m.user_id WHERE json_extract(d.data,'$.provider') IN ('thunderstore','modrinth','curseforge','github') ORDER BY m.rowid DESC")?;
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, user, raw) = row?;
        let data: Value = serde_json::from_str(&raw).unwrap_or_default();
        let url = data["source_url"].as_str().unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        let Some(key) = catalog::project_key(&data) else {
            continue;
        };
        let subscriber:Option<i64>=db.query_row("SELECT s.user_id FROM mod_subscriptions s JOIN users u ON u.id=s.user_id WHERE s.source=?1 AND u.banned=0 ORDER BY s.created LIMIT 1",[&key],|r|r.get(0)).optional()?;
        let user = subscriber.unwrap_or(user);
        if db.query_row("SELECT banned FROM users WHERE id=?1", [user], |r| {
            r.get::<_, bool>(0)
        })? {
            continue;
        }
        let source = hex::encode(Sha256::digest(key));
        if !seen.insert(source.clone()) {
            continue;
        }
        let checked: i64 = db
            .query_row(
                "SELECT checked FROM mod_update_state WHERE source=?1",
                [&source],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if checked < manual || checked <= now() - PERIOD {
            candidates.push((checked, source, id, user));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_, source, id, user)| (source, id, user)))
}
pub fn start(app: Shared) {
    tokio::spawn(async move {
        loop {
            // Wait after the previous job too; slow providers never cause catch-up bursts.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let Ok(Some((source, id, user))) = next(&app) else {
                continue;
            };
            let Ok(_permit) = app.upload_gate.try_acquire() else {
                continue;
            };
            let result = external::refresh_existing(&app, user, &id).await;
            let (status, detail, new_id) = match result {
                Ok(Some(new_id)) => (
                    "updated",
                    "New version imported; scan/review policy applies",
                    new_id,
                ),
                Ok(None) => ("current", "No newer compatible version", id.clone()),
                Err(_) => (
                    "error",
                    "Provider unavailable, rate limited, incompatible or download restricted; existing version retained",
                    id.clone(),
                ),
            };
            let db = app.db.lock().unwrap();
            let _=db.execute("INSERT INTO mod_update_state VALUES(?1,?2,?3,?4,?5) ON CONFLICT(source) DO UPDATE SET checked=excluded.checked,status=excluded.status,detail=excluded.detail,mod_id=excluded.mod_id",params![source,now(),status,detail,new_id]);
            let _=db.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'mod-update-check',?2,?3)",params![user,json!({"mod":id,"status":status,"new_mod":new_id}).to_string(),now()]);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture};
    #[tokio::test]
    async fn manual_cooldown_is_shared_and_requires_authentication() {
        let (_dir, app) = fixture();
        let a = account(&app, "checker-a", false);
        let b = account(&app, "checker-b", false);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/mods/updates/check",
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
                "/api/v1/mods/updates/check",
                json!({}),
                Some(&a)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/mods/updates/check",
                json!({}),
                Some(&b)
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_update_control SET manual_at=?1", [now() - 121])
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/mods/updates/check",
                json!({}),
                Some(&b)
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn queue_deduplicates_versions_and_remembers_manual_checks() {
        let (_dir, app) = fixture();
        account(&app, "owner", true);
        let d = json!({"provider":"modrinth","source_url":"https://modrinth.com/mod/fixture","update_profile":{"loader":"fabric","game_version":"1.21.1"}});
        let old = external::store(
            &app,
            1,
            0,
            "Fixture",
            "1",
            "",
            "modrinth:fixture:1",
            &d,
            b"PK\x03\x04one",
        )
        .await
        .unwrap();
        let new = external::store(
            &app,
            1,
            0,
            "Fixture",
            "2",
            "",
            "modrinth:fixture:2",
            &d,
            b"PK\x03\x04two",
        )
        .await
        .unwrap();
        let (key, id, _) = next(&app).unwrap().unwrap();
        assert_eq!(id, new);
        assert_ne!(id, old);
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO mod_update_state VALUES(?1,?2,'current','fixture',?3)",
                params![key, now(), new],
            )
            .unwrap();
        assert!(next(&app).unwrap().is_none());
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_update_control SET manual_at=?1", [now() + 1])
            .unwrap();
        assert!(next(&app).unwrap().is_some());
    }
}
