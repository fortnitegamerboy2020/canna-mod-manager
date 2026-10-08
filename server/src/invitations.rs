use super::*;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS invitation_history(hash TEXT PRIMARY KEY, code TEXT, issued_by INTEGER, created INTEGER NOT NULL, expires INTEGER NOT NULL, wave TEXT, label TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'active', redeemed_by INTEGER, resolved INTEGER);
      CREATE INDEX IF NOT EXISTS invitation_history_issuer ON invitation_history(issued_by,created);
      INSERT OR IGNORE INTO invitation_history(hash,issued_by,created,expires,wave) SELECT hash,issued_by,0,expires,wave FROM invites;
      CREATE TRIGGER IF NOT EXISTS invitation_history_closed AFTER DELETE ON invites BEGIN UPDATE invitation_history SET status=CASE WHEN OLD.expires<=strftime('%s','now') THEN 'expired' ELSE 'closed' END,resolved=strftime('%s','now') WHERE hash=OLD.hash AND status='active'; END;")?;
    for (table, column, declaration) in [
        ("invites", "beta", "INTEGER NOT NULL DEFAULT 0"),
        ("invitation_history", "beta", "INTEGER NOT NULL DEFAULT 0"),
        ("invitation_history", "delivered_to", "INTEGER"),
    ] {
        let columns = db
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        if !columns.iter().any(|name| name == column) {
            db.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
            ))?;
        }
    }
    Ok(())
}
pub fn attributes(
    db: &Connection,
    code: &str,
    beta: bool,
    delivered_to: Option<i64>,
) -> ApiResult<()> {
    db.execute(
        "UPDATE invites SET beta=?2 WHERE hash=?1",
        params![digest(code), beta],
    )?;
    db.execute(
        "UPDATE invitation_history SET beta=?2,delivered_to=?3 WHERE hash=?1",
        params![digest(code), beta, delivered_to],
    )?;
    Ok(())
}
pub async fn mine(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    let (actor, _) = app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let items=db.prepare("SELECT h.code,h.beta,h.expires,h.wave,h.label FROM invitation_history h JOIN invites i ON i.hash=h.hash WHERE (h.issued_by=?1 OR h.delivered_to=?1) AND h.code IS NOT NULL AND h.status='active' AND i.expires>?2 ORDER BY h.created DESC,h.hash LIMIT 200")?.query_map(params![actor,now()],|r|Ok(json!({"code":r.get::<_,String>(0)?,"beta":r.get::<_,bool>(1)?,"expires":r.get::<_,i64>(2)?,"wave":r.get::<_,Option<String>>(3)?,"label":r.get::<_,String>(4)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok((
        [
            ("cache-control", "private, no-store"),
            ("vary", "Cookie, Authorization"),
        ],
        axum::Json(json!({"items":items})),
    )
        .into_response())
}
pub fn remember(
    db: &Connection,
    code: &str,
    issuer: i64,
    expires: i64,
    wave: Option<&str>,
    label: &str,
) -> ApiResult<()> {
    // The production database is SQLCipher encrypted. Codes stay on the server,
    // and only the Owner can retrieve this history.
    db.execute("INSERT INTO invitation_history(hash,code,issued_by,created,expires,wave,label) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![digest(code),code,issuer,now(),expires,wave,label])?;
    db.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'invite-generated',?2,?3)",
        params![
            issuer,
            json!({"wave":wave,"label":label}).to_string(),
            now()
        ],
    )?;
    Ok(())
}
#[derive(Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub sort: String,
    #[serde(default = "first")]
    pub page: u32,
}
fn first() -> u32 {
    1
}
pub async fn list(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<Filter>,
) -> ApiResult<axum::Json<Value>> {
    community::owner(&app, &headers)?;
    if q.q.len() > 120
        || q.page == 0
        || q.page > 100000
        || !matches!(
            q.status.as_str(),
            "" | "active" | "used" | "revoked" | "expired" | "closed"
        )
    {
        return Err(bad("Invalid invitation filter"));
    }
    let order = match q.sort.as_str() {
        "" | "newest" => "h.created DESC,h.hash",
        "oldest" => "h.created,h.hash",
        "user" => "u.username COLLATE NOCASE,h.created DESC,h.hash",
        "most" => "issuer_count DESC,u.username,h.created DESC,h.hash",
        _ => return Err(bad("Invalid invitation sort")),
    };
    let db = app.db.lock().unwrap();
    let base = " FROM invitation_history h LEFT JOIN users u ON u.id=h.issued_by LEFT JOIN users r ON r.id=h.redeemed_by LEFT JOIN (SELECT issued_by,COUNT(*) AS total FROM invitation_history GROUP BY issued_by) counts ON counts.issued_by=h.issued_by WHERE (instr(lower(COALESCE(u.username,'')||' '||COALESCE(r.username,'')||' '||COALESCE(h.code,'')||' '||COALESCE(h.wave,'')||' '||h.label),lower(?1))>0) AND (?2='' OR CASE WHEN h.status='active' AND h.expires<=?3 THEN 'expired' ELSE h.status END=?2)";
    let total: i64 = db.query_row(
        &format!("SELECT COUNT(*){base}"),
        params![q.q, q.status, now()],
        |r| r.get(0),
    )?;
    let sql = format!(
        "SELECT h.hash,h.code,COALESCE(u.username,'Unknown issuer'),h.created,h.expires,h.wave,h.label,CASE WHEN h.status='active' AND h.expires<=?3 THEN 'expired' ELSE h.status END,COALESCE(r.username,''),COALESCE(counts.total,0) AS issuer_count,h.beta,COALESCE((SELECT username FROM users WHERE id=h.delivered_to),''){base} ORDER BY {order} LIMIT 50 OFFSET ?4"
    );
    let items=db.prepare(&sql)?.query_map(params![q.q,q.status,now(),(q.page-1)*50],|r|Ok(json!({"id":r.get::<_,String>(0)?,"code":r.get::<_,Option<String>>(1)?,"issuer":r.get::<_,String>(2)?,"created":r.get::<_,i64>(3)?,"expires":r.get::<_,i64>(4)?,"wave":r.get::<_,Option<String>>(5)?,"label":r.get::<_,String>(6)?,"status":r.get::<_,String>(7)?,"redeemed_by":r.get::<_,String>(8)?,"issuer_count":r.get::<_,i64>(9)?,"beta":r.get::<_,bool>(10)?,"delivered_to":r.get::<_,String>(11)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(
        json!({"items":items,"total":total,"page":q.page,"has_more":total>q.page as i64*50}),
    ))
}
pub async fn revoke(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let n=tx.execute("UPDATE invitation_history SET status='revoked',resolved=?2 WHERE hash=?1 AND status='active' AND expires>?2",params![id,now()])?;
    if n == 0 {
        return Err(bad("Invitation is already used, revoked or expired"));
    }
    tx.execute("DELETE FROM invites WHERE hash=?1", [&id])?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'invite-revoked',?2,?3)",
        params![actor, id, now()],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"revoked":n})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn beta_links_are_authorized_single_use_and_add_beta_at_signup() {
        use crate::tests::{account, call, fixture, value};
        let (_dir, app) = fixture();
        let owner = account(&app, "betaowner", true);
        let member = account(&app, "plainmember", false);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/invites",
                json!({"beta":true}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            value(call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&member)).await).await["invites_remaining"],
            1
        );
        let invite = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/invite-waves",
                json!({"count":1,"beta":true}),
                Some(&owner),
            )
            .await,
        )
        .await;
        let registration = json!({"username":"betafriend","password":"testing-password-123","email":"betafriend@example.test","invite":invite["invites"][0]});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/register",
                registration.clone(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        {
            let db = app.db.lock().unwrap();
            let id: i64 = db
                .query_row(
                    "SELECT id FROM users WHERE username='betafriend'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(admin_settings::has_rebound(&db, id).unwrap());
            assert_eq!(
                admin_settings::roles(&db, id).unwrap(),
                vec!["member", "beta"]
            );
        }
        assert_eq!(call(app.clone(),"POST","/api/v1/register",json!({"username":"anotherfriend","password":"testing-password-123","email":"another@example.test","invite":invite["invites"][0]}),None).await.status(),StatusCode::BAD_REQUEST);
    }
    #[tokio::test]
    async fn beta_friend_waves_deliver_private_links_only_to_active_beta_members() {
        use crate::tests::{account, call, fixture, value};
        let (_dir, app) = fixture();
        let owner = account(&app, "waveowner", true);
        let beta = account(&app, "betamember", false);
        let plain = account(&app, "plainmember", false);
        app.db
            .lock()
            .unwrap()
            .execute("INSERT INTO user_roles VALUES(2,'beta')", [])
            .unwrap();
        let wave = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/invite-waves",
                json!({"count":2,"mode":"beta_members"}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(wave["delivered_to"], 1);
        assert_eq!(wave["invites"].as_array().unwrap().len(), 2);
        let mine = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/my-invites",
                Value::Null,
                Some(&beta),
            )
            .await,
        )
        .await;
        assert_eq!(mine["items"].as_array().unwrap().len(), 2);
        assert_eq!(mine["items"][0]["beta"], true);
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/my-invites",
                    Value::Null,
                    Some(&plain)
                )
                .await
            )
            .await["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        call(
            app.clone(),
            "DELETE",
            &format!("/api/v1/invite-waves/{}", wave["wave"].as_str().unwrap()),
            Value::Null,
            Some(&owner),
        )
        .await;
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/my-invites",
                    Value::Null,
                    Some(&beta)
                )
                .await
            )
            .await["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
    #[tokio::test]
    async fn history_retains_codes_and_revoke_blocks_redemption_and_member_access() {
        use crate::tests::{account, call, fixture, value};
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let member = account(&app, "member", false);
        let created = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/invite-waves",
                json!({"count":3,"mode":"codes","label":"Family"}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(created["wave"], "");
        let history = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/invites?q=Family&sort=most",
                json!({}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(history["items"].as_array().unwrap().len(), 3);
        assert!(
            history["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["code"] == created["invites"][0])
        );
        let id = history["items"][0]["id"].as_str().unwrap();
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/invites",
                json!({}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "DELETE",
                &format!("/api/v1/invites/{id}"),
                json!({}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "DELETE",
                &format!("/api/v1/invites/{id}"),
                json!({}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM invites WHERE hash=?1", [id], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let revoked = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/invites?status=revoked",
                json!({}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(revoked["items"].as_array().unwrap().len(), 1);
        assert!(!revoked["items"][0]["code"].is_null());
    }
}
