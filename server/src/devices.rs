use super::*;
pub const COOKIE_AGE: i64 = 400 * 86400;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    let db = &tx;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='session_devices')",
        [],
        |r| r.get(0),
    )?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS session_devices(hash TEXT PRIMARY KEY REFERENCES sessions(hash) ON DELETE CASCADE,id TEXT NOT NULL UNIQUE,name TEXT NOT NULL,kind TEXT NOT NULL,created INTEGER NOT NULL,last_seen INTEGER NOT NULL,trust_hash TEXT);")?;
    if !exists {
        db.execute("UPDATE sessions SET expires=-1 WHERE expires>?1", [now()])?;
        let hashes = db
            .prepare("SELECT hash FROM sessions WHERE expires=-1")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for hash in hashes {
            db.execute("INSERT INTO session_devices(hash,id,name,kind,created,last_seen) VALUES(?1,?2,'Previously signed-in device','unknown',?3,?3)",params![hash,Uuid::new_v4().to_string(),now()])?;
        }
    }
    tx.commit()?;
    Ok(())
}
pub fn name(headers: &HeaderMap) -> String {
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    let os = if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPhone") || ua.contains("iPad") {
        "iOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Mac") {
        "macOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "Unknown device"
    };
    format!("{browser} on {os}")
}
pub fn record(
    db: &Connection,
    raw: &str,
    user: i64,
    name: &str,
    kind: &str,
) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO sessions VALUES(?1,?2,-1)",
        params![digest(raw), user],
    )?;
    db.execute("INSERT INTO session_devices(hash,id,name,kind,created,last_seen) VALUES(?1,?2,?3,?4,?5,?5)",params![digest(raw),Uuid::new_v4().to_string(),name,kind,now()])?;
    Ok(())
}
pub fn trust(headers: &HeaderMap) -> Option<String> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|p| p.trim().strip_prefix("__Host-canna_trust="))
        .filter(|v| v.len() == 64)
        .map(digest)
}
pub async fn list(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    let current = digest(auth_token(&headers).ok_or_else(denied)?);
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT d.id,d.name,d.kind,d.created,d.last_seen,d.hash FROM session_devices d JOIN sessions s ON s.hash=d.hash WHERE s.user_id=?1 AND (s.expires=-1 OR s.expires>?2) ORDER BY d.last_seen DESC")?;
    let rows=stmt.query_map(params![user,now()],|r| {let seen:i64=r.get(4)?;let hash:String=r.get(5)?; Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"created":r.get::<_,i64>(3)?,"last_seen":seen,"current":hash==current,"state":if now()-seen<=120 {"Active recently"} else {"Idle"}}))})?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(rows)))
}
#[derive(Deserialize)]
pub struct Rename {
    name: String,
}
pub async fn rename(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<Rename>,
) -> ApiResult<StatusCode> {
    let (user, _) = app.auth(&headers)?;
    let name = input.name.trim();
    if name.is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        return Err(bad("Use a device name of 1–80 characters"));
    }
    let count=app.db.lock().unwrap().execute("UPDATE session_devices SET name=?1 WHERE id=?2 AND hash IN (SELECT hash FROM sessions WHERE user_id=?3)",params![name,id,user])?;
    if count != 1 {
        return Err(ApiError(StatusCode::NOT_FOUND, "Device not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub async fn revoke(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let (user, _) = app.auth(&headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let row:Option<(String,Option<String>)>=tx.query_row("SELECT d.hash,d.trust_hash FROM session_devices d JOIN sessions s ON s.hash=d.hash WHERE d.id=?1 AND s.user_id=?2",params![id,user],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let (hash, trust) = row.ok_or(ApiError(StatusCode::NOT_FOUND, "Device not found"))?;
    if let Some(trust) = trust {
        tx.execute(
            "DELETE FROM trusted_devices WHERE hash=?1 AND user_id=?2",
            params![trust, user],
        )?;
    }
    tx.execute("DELETE FROM sessions WHERE hash=?1", [hash])?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'logout-device',?2,?3)",
        params![user, id, now()],
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn revoke_others(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<StatusCode> {
    let (user, _) = app.auth(&headers)?;
    let current = digest(auth_token(&headers).ok_or_else(denied)?);
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("DELETE FROM trusted_devices WHERE user_id=?1 AND hash<>COALESCE((SELECT trust_hash FROM session_devices WHERE hash=?2),'')",params![user,current])?;
    tx.execute(
        "DELETE FROM sessions WHERE user_id=?1 AND hash<>?2",
        params![user, current],
    )?;
    tx.execute("DELETE FROM login_codes WHERE user_id=?1", [user])?;
    tx.execute("DELETE FROM desktop_pairings WHERE user_id=?1", [user])?;
    tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'logout-other-devices','all other devices',?2)",params![user,now()])?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn persistent_devices_are_private_and_revocation_preserves_current_only() {
        let (_dir, app) = crate::tests::fixture();
        let raw = crate::tests::account(&app, "devices-user", false);
        let stranger = crate::tests::account(&app, "devices-other", false);
        let user: i64 = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT user_id FROM sessions WHERE hash=?1",
                [digest(&raw)],
                |r| r.get(0),
            )
            .unwrap();
        let second = app.session(user, &HeaderMap::new()).unwrap().0["token"]
            .as_str()
            .unwrap()
            .to_owned();
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE session_devices SET last_seen=?1,created=?1 WHERE hash=?2",
                params![now() - 3 * 86400, digest(&second)],
            )
            .unwrap();
            db.execute(
                "INSERT INTO trusted_devices VALUES('fixture-trust',?1,?2)",
                params![user, now() + 86400],
            )
            .unwrap();
            db.execute(
                "UPDATE session_devices SET trust_hash='fixture-trust' WHERE hash=?1",
                [digest(&second)],
            )
            .unwrap();
        }
        let items = crate::tests::value(
            crate::tests::call(
                app.clone(),
                "GET",
                "/api/v1/devices",
                Value::Null,
                Some(&raw),
            )
            .await,
        )
        .await;
        assert_eq!(items.as_array().unwrap().len(), 2);
        let old = items
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["current"] == false)
            .unwrap();
        assert_eq!(old["state"], "Idle");
        assert!(old.get("hash").is_none());
        let path = format!("/api/v1/devices/{}", old["id"].as_str().unwrap());
        assert_eq!(
            crate::tests::call(app.clone(), "DELETE", &path, Value::Null, Some(&stranger))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            crate::tests::call(
                app.clone(),
                "POST",
                &path,
                json!({"name":"Bedroom PC"}),
                Some(&raw)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            crate::tests::call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&second))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            crate::tests::call(
                app.clone(),
                "POST",
                "/api/v1/devices/logout-others",
                json!({}),
                Some(&raw)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            crate::tests::call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&second))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            crate::tests::call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&raw))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            crate::tests::call(
                app.clone(),
                "GET",
                "/api/v1/me",
                Value::Null,
                Some(&stranger)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM trusted_devices WHERE user_id=?1",
                    [user],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let items = crate::tests::value(
            crate::tests::call(
                app.clone(),
                "GET",
                "/api/v1/devices",
                Value::Null,
                Some(&raw),
            )
            .await,
        )
        .await;
        let path = format!("/api/v1/devices/{}", items[0]["id"].as_str().unwrap());
        assert_eq!(
            crate::tests::call(app.clone(), "DELETE", &path, Value::Null, Some(&raw))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            crate::tests::call(app, "GET", "/api/v1/me", Value::Null, Some(&raw))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[test]
    fn upgrade_preserves_valid_sessions_without_reviving_expired_ones() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE sessions(hash TEXT PRIMARY KEY,user_id INTEGER,expires INTEGER);INSERT INTO sessions VALUES('valid',1,9999999999),('expired',1,0);").unwrap();
        initialize(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT expires FROM sessions WHERE hash='valid'", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            -1
        );
        assert_eq!(
            db.query_row(
                "SELECT expires FROM sessions WHERE hash='expired'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        initialize(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM session_devices", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
