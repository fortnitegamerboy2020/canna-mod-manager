use super::*;
const TTL: i64 = 7 * 86400;
static FETCH: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS provider_archives(mod_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE,expires INTEGER NOT NULL,evicted INTEGER NOT NULL DEFAULT 0);
 INSERT OR IGNORE INTO provider_archives(mod_id,expires) SELECT mod_id,strftime('%s','now')+604800 FROM mod_details WHERE json_extract(data,'$.provider') IN ('thunderstore','modrinth','curseforge') AND json_type(data,'$.release_id')='text';")
}
pub fn track(db: &Connection, id: &str) -> rusqlite::Result<()> {
    db.execute("INSERT OR IGNORE INTO provider_archives(mod_id,expires) SELECT mod_id,?2 FROM mod_details WHERE mod_id=?1 AND json_extract(data,'$.provider') IN ('thunderstore','modrinth','curseforge')",params![id,now()+TTL])?;
    Ok(())
}
pub async fn ensure(app: &App, id: &str) -> ApiResult<()> {
    Uuid::parse_str(id).map_err(|_| bad("Invalid archive ID"))?;
    let managed: bool = app.db.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM provider_archives WHERE mod_id=?1)",
        [id],
        |r| r.get(0),
    )?;
    if !managed {
        return Ok(());
    }
    let _permit = FETCH
        .acquire()
        .await
        .map_err(|_| bad("Provider cache unavailable"))?;
    if app.files.join(format!("{id}.zip")).is_file() {
        app.db.lock().unwrap().execute(
            "UPDATE provider_archives SET expires=?2 WHERE mod_id=?1 AND expires<=?3 AND evicted=0",
            params![id, now() + TTL, now()],
        )?;
        return Ok(());
    }
    let (data, expected, size) = {
        let db = app.db.lock().unwrap();
        security::approved(&db, id)?;
        let (raw,hash,size):(String,String,i64)=db.query_row("SELECT d.data,m.sha256,m.size FROM mods m JOIN mod_details d ON d.mod_id=m.id WHERE m.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        (
            serde_json::from_str::<Value>(&raw).map_err(|_| bad("Invalid provider record"))?,
            hash,
            size,
        )
    };
    let input = external::Link {
        url: data["source_url"].as_str().unwrap_or_default().into(),
        version: data["release_id"].as_str().unwrap_or_default().into(),
        loader: data["update_profile"]["loader"]
            .as_str()
            .unwrap_or_default()
            .into(),
        game_version: data["update_profile"]["game_version"]
            .as_str()
            .unwrap_or_default()
            .into(),
        include_optional: false,
    };
    if input.version.is_empty() {
        return Err(bad(
            "The original version cannot be retrieved; import a new reviewed release",
        ));
    }
    let (project, release) = external::selection(&input).await?;
    if release.id != input.version || project.id != data["id"].as_str().unwrap_or_default() {
        return Err(bad("Provider returned a different project or version"));
    }
    let bytes = external::download_release(&project, &release).await?;
    restore(app, id, &bytes, &expected, size).await
}
async fn restore(app: &App, id: &str, bytes: &[u8], expected: &str, size: i64) -> ApiResult<()> {
    if bytes.len() as i64 != size || hex::encode(Sha256::digest(&bytes)) != expected {
        return Err(bad(
            "The original archive changed; cached approval cannot be reused. Import and review it again.",
        ));
    }
    let temporary = app.files.join(format!("{id}.{}.cache", Uuid::new_v4()));
    let result = async {
        let file = tokio::fs::File::create(&temporary).await?;
        let mut writer = crypto::Writer::new(file, &app.upload_key, id.to_owned()).await?;
        writer.write(&bytes).await?;
        writer.finish().await?;
        tokio::fs::rename(&temporary, app.files.join(format!("{id}.zip"))).await?;
        app.db.lock().unwrap().execute(
            "UPDATE provider_archives SET expires=?2,evicted=0 WHERE mod_id=?1",
            params![id, now() + TTL],
        )?;
        Ok::<(), ApiError>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}
pub fn cleanup(app: &App) -> ApiResult<usize> {
    let candidates=app.db.lock().unwrap().prepare("SELECT c.mod_id FROM provider_archives c JOIN mod_reviews r ON r.mod_id=c.mod_id WHERE c.expires<=?1 AND c.evicted=0 AND r.approved=1 AND NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=c.mod_id AND s.status IN ('queued','running'))")?.query_map([now()],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    let mut removed = 0;
    for id in candidates {
        if Uuid::parse_str(&id).is_err() {
            continue;
        }
        let db = app.db.lock().unwrap();
        let expired:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM provider_archives WHERE mod_id=?1 AND expires<=?2 AND evicted=0)",params![id,now()],|r|r.get(0))?;
        if !expired {
            continue;
        }
        let path = app.files.join(format!("{id}.zip"));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        db.execute(
            "UPDATE provider_archives SET evicted=1 WHERE mod_id=?1 AND expires<=?2",
            params![id, now()],
        )?;
        removed += 1;
    }
    Ok(removed)
}
pub fn start(app: Shared) {
    tokio::spawn(async move {
        loop {
            let _permit = FETCH.acquire_many(2).await;
            let _ = cleanup(&app);
            drop(_permit);
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn expiration_preserves_records_and_restoration_checks_original_bytes() {
        let (_dir, app) = crate::tests::fixture();
        let _token = crate::tests::account(&app, "cache-owner", true);
        let bytes = b"PK\x03\x04provider fixture";
        let id=external::store(&app,1,1557740,"Cache fixture","1","","cache:1",&json!({"provider":"thunderstore","id":"Author-Mod","release_id":"1","source_url":"https://thunderstore.io/c/rounds/p/Author/Mod/"}),bytes).await.unwrap();
        let manual = external::store(
            &app,
            1,
            1557740,
            "Manual fixture",
            "1",
            "",
            "manual:1",
            &json!({}),
            bytes,
        )
        .await
        .unwrap();
        {
            let db = app.db.lock().unwrap();
            track(&db, &id).unwrap();
            track(&db, &manual).unwrap();
            db.execute("UPDATE mod_reviews SET approved=1", []).unwrap();
            db.execute("UPDATE provider_archives SET expires=0", [])
                .unwrap();
        }
        assert_eq!(cleanup(&app).unwrap(), 1);
        assert!(!app.files.join(format!("{id}.zip")).exists());
        assert!(app.files.join(format!("{manual}.zip")).exists());
        assert!(security::approved(&app.db.lock().unwrap(), &id).is_ok());
        catalog::audit(&app).await.unwrap();
        let hash = hex::encode(Sha256::digest(bytes));
        assert!(
            restore(&app, &id, b"changed", &hash, bytes.len() as i64)
                .await
                .is_err()
        );
        assert!(!app.files.join(format!("{id}.zip")).exists());
        restore(&app, &id, bytes, &hash, bytes.len() as i64)
            .await
            .unwrap();
        assert!(app.files.join(format!("{id}.zip")).exists());
        assert_eq!(cleanup(&app).unwrap(), 0);
        let details = external::details(&app.db.lock().unwrap(), &id).unwrap();
        assert_eq!(details["release_id"], "1");
    }
}
