use super::*;
#[cfg(test)]
mod wallet_tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn only_owner_can_change_kash_with_a_logged_reason() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let member = account(&app, "member", false);
        let path = "/api/v1/admin/wallets/2";
        let body = json!({"balance":200,"earned":500,"reason":"Owner gift"});
        assert_eq!(
            call(app.clone(), "POST", path, body.clone(), Some(&member))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET role='admin',admin=1 WHERE id=2", [])
            .unwrap();
        assert_eq!(
            call(app.clone(), "POST", path, body.clone(), Some(&member))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                path,
                json!({"balance":-1,"earned":500,"reason":"Owner gift"}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(app.clone(), "POST", path, body, Some(&owner))
                .await
                .status(),
            StatusCode::OK
        );
        let me =
            value(call(app.clone(), "GET", "/api/v1/me", json!({}), Some(&member)).await).await;
        assert_eq!(me["kash"], 200);
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM audit WHERE action='edit-kash'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}
pub async fn wallets(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let actor = allowed(&app, &headers)?;
    if community::role(&app, actor)? != "owner" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner permission required"));
    }
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT u.id,u.username,COALESCE(w.balance,0),COALESCE(w.earned,0) FROM users u LEFT JOIN bot_wallets w ON w.user_id=u.id ORDER BY u.username LIMIT 500")?;
    let rows=stmt.query_map([],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?,"balance":r.get::<_,i64>(2)?,"earned":r.get::<_,i64>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(rows)))
}
#[derive(Deserialize)]
pub struct WalletEdit {
    balance: i64,
    earned: i64,
    reason: String,
}
pub async fn wallet_edit(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    axum::Json(input): axum::Json<WalletEdit>,
) -> ApiResult<axum::Json<Value>> {
    let actor = allowed(&app, &headers)?;
    if community::role(&app, actor)? != "owner" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner permission required"));
    }
    if !(0..=10000).contains(&input.balance)
        || !(0..=1000000).contains(&input.earned)
        || !(5..=500).contains(&input.reason.trim().len())
    {
        return Err(bad(
            "Balance must be 0–10000, earned 0–1000000; record a reason (5–500 bytes)",
        ));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(bad("Member not found"));
    }
    tx.execute(
        "INSERT OR IGNORE INTO bot_wallets(user_id) VALUES(?1)",
        [id],
    )?;
    let before: (i64, i64) = tx.query_row(
        "SELECT balance,earned FROM bot_wallets WHERE user_id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    tx.execute(
        "UPDATE bot_wallets SET balance=?1,earned=?2 WHERE user_id=?3",
        params![input.balance, input.earned, id],
    )?;
    tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'edit-kash',?2,?3)",params![actor,json!({"user":id,"before":{"balance":before.0,"earned":before.1},"after":{"balance":input.balance,"earned":input.earned},"reason":input.reason.trim()}).to_string(),now()])?;
    tx.commit()?;
    app.live.hint("chat");
    Ok(axum::Json(json!({"ok":true})))
}
fn allowed(app: &App, headers: &HeaderMap) -> ApiResult<i64> {
    let (id, admin) = app.auth(headers)?;
    if !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator permission required",
        ));
    }
    Ok(id)
}
pub async fn overview(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    allowed(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let count = |sql: &str| -> rusqlite::Result<i64> { db.query_row(sql, [], |r| r.get(0)) };
    Ok(axum::Json(
        json!({"members":count("SELECT COUNT(*) FROM users")?,"banned":count("SELECT COUNT(*) FROM users WHERE banned=1")?,"unverified":count("SELECT COUNT(*) FROM users WHERE verified=0")?,"mods":count("SELECT COUNT(*) FROM mods")?,"pending":count("SELECT COUNT(*) FROM mod_reviews r WHERE approved=0 AND NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=r.mod_id AND s.status='rejected')")?,"topics":count("SELECT COUNT(*) FROM topics")?,"posts":count("SELECT COUNT(*) FROM posts")?,"sessions":count("SELECT COUNT(*) FROM sessions WHERE expires=-1 OR expires>strftime('%s','now')")?,"storage_bytes":count("SELECT COALESCE(SUM(size),0) FROM mods")?,"version":env!("CARGO_PKG_VERSION")}),
    ))
}
pub async fn reviews(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    allowed(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let mut statement=db.prepare("SELECT m.id,m.name,m.version,m.app_id,m.size,u.username FROM mods m JOIN mod_reviews r ON r.mod_id=m.id JOIN users u ON u.id=m.user_id WHERE r.approved=0 AND NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=m.id AND s.status='rejected') ORDER BY m.rowid DESC LIMIT 200")?;
    let values=statement.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"version":r.get::<_,String>(2)?,"app_id":r.get::<_,i64>(3)?,"size":r.get::<_,i64>(4)?,"author":r.get::<_,String>(5)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(values)))
}
pub async fn revoke_sessions(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(target): Path<i64>,
) -> ApiResult<StatusCode> {
    let actor = allowed(&app, &headers)?;
    let actor_role = community::role(&app, actor)?;
    let target_role = community::role(&app, target)?;
    if actor == target
        || target_role == "owner"
        || (actor_role != "owner" && target_role == "admin")
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Use your profile for your own devices; higher roles cannot be revoked",
        ));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("DELETE FROM sessions WHERE user_id=?1", [target])?;
    tx.execute("DELETE FROM trusted_devices WHERE user_id=?1", [target])?;
    tx.execute("DELETE FROM login_codes WHERE user_id=?1", [target])?;
    tx.execute("DELETE FROM codes WHERE user_id=?1", [target])?;
    tx.execute("DELETE FROM download_tickets WHERE user_id=?1", [target])?;
    tx.execute("DELETE FROM desktop_pairings WHERE user_id=?1", [target])?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'revoke-member-devices',?2,?3)",
        params![actor, target.to_string(), now()],
    )?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn administrator_tools_require_server_permissions() {
        let (_dir, app) = crate::tests::fixture();
        let member = crate::tests::account(&app, "review-member", false);
        let admin = crate::tests::account(&app, "review-admin", true);
        for path in [
            "/admin.js",
            "/api/v1/admin/overview",
            "/api/v1/admin/mod-reviews",
        ] {
            assert_eq!(
                crate::tests::call(app.clone(), "GET", path, Value::Null, None)
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                crate::tests::call(app.clone(), "GET", path, Value::Null, Some(&member))
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                crate::tests::call(app.clone(), "GET", path, Value::Null, Some(&admin))
                    .await
                    .status(),
                StatusCode::OK
            );
        }
    }
    #[tokio::test]
    async fn subordinate_revocation_removes_pending_transfers_and_preserves_actor() {
        let (_dir, app) = crate::tests::fixture();
        let owner = crate::tests::account(&app, "revoker", true);
        let target = crate::tests::account(&app, "subordinate", false);
        {
            let db = app.db.lock().unwrap();
            db.execute("INSERT INTO download_tickets(hash,user_id,kind,item,expires) VALUES('pending',2,'mods','fixture',?1)",[now()+300]).unwrap();
        }
        assert_eq!(
            crate::tests::call(
                app.clone(),
                "POST",
                "/api/v1/admin/users/1/sessions",
                Value::Null,
                Some(&target)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            crate::tests::call(
                app.clone(),
                "POST",
                "/api/v1/admin/users/2/sessions",
                Value::Null,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            crate::tests::call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&target))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            crate::tests::call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&owner))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM download_tickets WHERE user_id=2",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}
