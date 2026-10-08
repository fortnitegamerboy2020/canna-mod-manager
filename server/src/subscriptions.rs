use super::*;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS mod_subscriptions(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,source TEXT NOT NULL,mod_id TEXT REFERENCES mods(id) ON DELETE SET NULL,name TEXT NOT NULL,provider TEXT NOT NULL,source_url TEXT NOT NULL,created INTEGER NOT NULL,PRIMARY KEY(user_id,source)); CREATE INDEX IF NOT EXISTS subscription_project ON mod_subscriptions(source);")
}
pub fn subscribe(db: &Connection, user: i64, id: &str) -> ApiResult<()> {
    let data = external::details(db, id)?;
    let source =
        catalog::project_key(&data).ok_or_else(|| bad("This is not a provider project"))?;
    let name: String = db.query_row("SELECT name FROM mods WHERE id=?1", [id], |r| r.get(0))?;
    db.execute("INSERT INTO mod_subscriptions VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(user_id,source) DO UPDATE SET mod_id=excluded.mod_id,name=excluded.name",params![user,source,id,name,data["provider"].as_str().unwrap_or(""),data["source_url"].as_str().unwrap_or(""),now()])?;
    Ok(())
}
pub fn accepted(db: &Connection, id: &str) -> ApiResult<()> {
    if security::approved(db, id).is_err() {
        return Ok(());
    }
    let data = external::details(db, id)?;
    let Some(source) = catalog::project_key(&data) else {
        return Ok(());
    };
    let (name, version): (String, String) =
        db.query_row("SELECT name,version FROM mods WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let users=db.prepare("SELECT user_id FROM mod_subscriptions WHERE source=?1 AND (mod_id IS NULL OR (SELECT rowid FROM mods WHERE id=mod_id)<(SELECT rowid FROM mods WHERE id=?2))")?.query_map(params![source,id],|r|r.get::<_,i64>(0))?.collect::<Result<Vec<_>,_>>()?;
    for user in users {
        notifications::notify(
            db,
            user,
            "mod-update",
            &format!("{name} {version} is ready to download"),
            "/subscriptions",
            &format!("mod-update:{id}"),
        )?;
    }
    db.execute(
        "UPDATE mod_subscriptions SET mod_id=?2,name=?3 WHERE source=?1 AND (mod_id IS NULL OR (SELECT rowid FROM mods WHERE id=mod_id)<(SELECT rowid FROM mods WHERE id=?2))",
        params![source, id, name],
    )?;
    Ok(())
}
#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub q: String,
}
pub async fn list(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<Page>,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    if q.page > 500 || q.q.len() > 120 {
        return Err(bad("Invalid subscription page"));
    }
    let page = q.page.max(1);
    let db = app.db.lock().unwrap();
    let mut rows=db.prepare("SELECT s.source,s.mod_id,s.name,s.provider,s.source_url,COALESCE(m.version,''),COALESCE(d.data,'{}') FROM mod_subscriptions s LEFT JOIN mods m ON m.id=s.mod_id LEFT JOIN mod_details d ON d.mod_id=m.id WHERE s.user_id=?1 AND instr(lower(s.name),lower(?2))>0 ORDER BY s.created DESC,s.source LIMIT 25 OFFSET ?3")?.query_map(params![user,q.q,(page-1)*24],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?)))?.collect::<Result<Vec<_>,_>>()?;
    // Reconcile only this page's projects; dependency review may finish after the root.
    for row in rows.iter_mut().take(24) {
        let candidates=db.prepare("SELECT m.id,m.version,d.data FROM mods m JOIN mod_details d ON d.mod_id=m.id JOIN mod_reviews r ON r.mod_id=m.id WHERE r.approved=1 AND json_extract(d.data,'$.source_url')=?1 ORDER BY m.rowid DESC LIMIT 50")?.query_map([&row.4],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
        for (id, version, details) in candidates {
            let data: Value = serde_json::from_str(&details).unwrap_or_default();
            if catalog::project_key(&data).as_deref() == Some(&row.0)
                && security::approved(&db, &id).is_ok()
            {
                accepted(&db, &id)?;
                row.1 = Some(id);
                row.5 = version;
                row.6 = details;
                break;
            }
        }
    }
    let more = rows.len() > 24;
    let items=rows.into_iter().take(24).map(|(source,id,name,provider,url,version,details)|{let approved=id.as_deref().is_some_and(|id|security::approved(&db,id).is_ok());json!({"source":source,"id":id,"name":name,"provider":provider,"source_url":url,"version":version,"approved":approved,"download_status":id.as_deref().and_then(|id|scans::download_state(&db,id).ok()),"details":serde_json::from_str::<Value>(&details).unwrap_or_default()})}).collect::<Vec<_>>();
    Ok(axum::Json(
        json!({"items":items,"page":page,"has_more":more}),
    ))
}
#[derive(Deserialize)]
pub struct Remove {
    pub source: String,
}
pub async fn remove(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(q): axum::Json<Remove>,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    if q.source.len() > 1000 {
        return Err(bad("Invalid subscription"));
    }
    app.db.lock().unwrap().execute(
        "DELETE FROM mod_subscriptions WHERE user_id=?1 AND source=?2",
        params![user, q.source],
    )?;
    Ok(axum::Json(json!({"ok":true})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn subscriptions_are_private_idempotent_and_follow_reviewed_updates() {
        use crate::tests::{account, call, fixture, value};
        let (_dir, app) = fixture();
        let token = account(&app, "subscriber", false);
        let other = account(&app, "other-subscriber", false);
        let data = json!({"provider":"modrinth","source_url":"https://modrinth.com/mod/fixture"});
        let first = external::store(
            &app,
            1,
            0,
            "Fixture",
            "1",
            "",
            "sub:1",
            &data,
            b"PK\x03\x04old",
        )
        .await
        .unwrap();
        {
            let db = app.db.lock().unwrap();
            subscribe(&db, 1, &first).unwrap();
            subscribe(&db, 1, &first).unwrap();
        }
        let path = "/api/v1/mods/subscriptions";
        assert_eq!(
            call(app.clone(), "GET", path, json!({}), None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            value(call(app.clone(), "GET", path, json!({}), Some(&other)).await).await["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        let new = external::store(
            &app,
            1,
            0,
            "Fixture",
            "2",
            "",
            "sub:2",
            &data,
            b"PK\x03\x04new",
        )
        .await
        .unwrap();
        assert_eq!(
            value(call(app.clone(), "GET", path, json!({}), Some(&token)).await).await["items"][0]
                ["id"],
            first
        );
        {
            let db = app.db.lock().unwrap();
            db.execute("UPDATE mod_reviews SET approved=1 WHERE mod_id=?1", [&new])
                .unwrap();
            accepted(&db, &new).unwrap();
        }
        assert_eq!(
            value(call(app.clone(), "GET", path, json!({}), Some(&token)).await).await["items"][0]
                ["id"],
            new
        );
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE mod_reviews SET approved=1 WHERE mod_id=?1",
                [&first],
            )
            .unwrap();
            accepted(&db, &first).unwrap();
        }
        assert_eq!(
            value(call(app.clone(), "GET", path, json!({}), Some(&token)).await).await["items"][0]
                ["id"],
            new
        );
        let source = catalog::project_key(&data).unwrap();
        call(
            app.clone(),
            "DELETE",
            path,
            json!({"source":source}),
            Some(&other),
        )
        .await;
        assert_eq!(
            value(call(app.clone(), "GET", path, json!({}), Some(&token)).await).await["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
