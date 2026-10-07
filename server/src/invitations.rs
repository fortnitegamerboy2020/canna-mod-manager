use super::*;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS invitation_history(hash TEXT PRIMARY KEY, code TEXT, issued_by INTEGER, created INTEGER NOT NULL, expires INTEGER NOT NULL, wave TEXT, label TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'active', redeemed_by INTEGER, resolved INTEGER);
      CREATE INDEX IF NOT EXISTS invitation_history_issuer ON invitation_history(issued_by,created);
      INSERT OR IGNORE INTO invitation_history(hash,issued_by,created,expires,wave) SELECT hash,issued_by,0,expires,wave FROM invites;
      CREATE TRIGGER IF NOT EXISTS invitation_history_closed AFTER DELETE ON invites BEGIN UPDATE invitation_history SET status=CASE WHEN OLD.expires<=strftime('%s','now') THEN 'expired' ELSE 'closed' END,resolved=strftime('%s','now') WHERE hash=OLD.hash AND status='active'; END;")
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
        "SELECT h.hash,h.code,COALESCE(u.username,'Unknown issuer'),h.created,h.expires,h.wave,h.label,CASE WHEN h.status='active' AND h.expires<=?3 THEN 'expired' ELSE h.status END,COALESCE(r.username,''),COALESCE(counts.total,0) AS issuer_count{base} ORDER BY {order} LIMIT 50 OFFSET ?4"
    );
    let items=db.prepare(&sql)?.query_map(params![q.q,q.status,now(),(q.page-1)*50],|r|Ok(json!({"id":r.get::<_,String>(0)?,"code":r.get::<_,Option<String>>(1)?,"issuer":r.get::<_,String>(2)?,"created":r.get::<_,i64>(3)?,"expires":r.get::<_,i64>(4)?,"wave":r.get::<_,Option<String>>(5)?,"label":r.get::<_,String>(6)?,"status":r.get::<_,String>(7)?,"redeemed_by":r.get::<_,String>(8)?,"issuer_count":r.get::<_,i64>(9)?})))?.collect::<Result<Vec<_>,_>>()?;
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
