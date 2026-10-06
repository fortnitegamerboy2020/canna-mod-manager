use super::*;
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
        json!({"members":count("SELECT COUNT(*) FROM users")?,"banned":count("SELECT COUNT(*) FROM users WHERE banned=1")?,"unverified":count("SELECT COUNT(*) FROM users WHERE verified=0")?,"mods":count("SELECT COUNT(*) FROM mods")?,"pending":count("SELECT COUNT(*) FROM mod_reviews WHERE approved=0")?,"topics":count("SELECT COUNT(*) FROM topics")?,"posts":count("SELECT COUNT(*) FROM posts")?,"sessions":count("SELECT COUNT(*) FROM sessions WHERE expires=-1 OR expires>strftime('%s','now')")?,"storage_bytes":count("SELECT COALESCE(SUM(size),0) FROM mods")?,"version":env!("CARGO_PKG_VERSION")}),
    ))
}
pub async fn reviews(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    allowed(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let mut statement=db.prepare("SELECT m.id,m.name,m.version,m.app_id,m.size,u.username FROM mods m JOIN mod_reviews r ON r.mod_id=m.id JOIN users u ON u.id=m.user_id WHERE r.approved=0 ORDER BY m.rowid DESC LIMIT 200")?;
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
