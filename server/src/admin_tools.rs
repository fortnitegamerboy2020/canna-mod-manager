use super::*;
#[cfg(test)]
mod wallet_tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn review_cards_include_analysis_progress_and_unresolved_counts() {
        let (_dir, app) = fixture();
        let owner = account(&app, "review-owner", true);
        let member = account(&app, "review-member", false);
        let mut ids = std::collections::HashMap::new();
        for (name, status) in [
            ("Waiting", "pending"),
            ("Working", "queued"),
            ("Flagged", "complete"),
            ("Failed", "failed"),
        ] {
            let id = external::store(
                &app,
                1,
                1557740,
                name,
                "1",
                "",
                &format!("test-progress-{name}"),
                &json!({"provider":"thunderstore"}),
                format!("PK\x05\x06{name}").as_bytes(),
            )
            .await
            .unwrap();
            ids.insert(name, id.clone());
            let db = app.db.lock().unwrap();
            db.execute("UPDATE mod_reviews SET approved=0 WHERE mod_id=?1", [&id])
                .unwrap();
            if status != "pending" {
                let report = if status == "complete" {
                    json!({"findings":[{"title":"Generic heuristic","accepted":false},{"title":"Reviewed finding","accepted":true}]})
                } else {
                    json!({"error":"Retry this scan"})
                };
                db.execute(
                    "INSERT INTO mod_scans VALUES(?1,'hash',?2,?3,10)",
                    params![id, status, report.to_string()],
                )
                .unwrap();
            }
        }
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_details SET data=?1 WHERE mod_id=?2",
                params![
                    json!({"provider":"thunderstore","dependency_ids":[ids["Waiting"]]})
                        .to_string(),
                    ids["Flagged"]
                ],
            )
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/mod-reviews",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let result = value(
            call(
                app,
                "GET",
                "/api/v1/admin/mod-reviews?page=1",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(
            result["queue"],
            json!({"waiting":1,"scheduled":1,"failed":1})
        );
        let items = result["items"].as_array().unwrap();
        for (name, status) in [
            ("Waiting", "pending"),
            ("Working", "queued"),
            ("Flagged", "complete"),
            ("Failed", "failed"),
        ] {
            let item = items.iter().find(|m| m["name"] == name).unwrap();
            assert_eq!(item["analysis_status"], status);
            assert_eq!(
                item["unresolved_findings"],
                if name == "Flagged" { 1 } else { 0 }
            );
            if name == "Flagged" {
                assert_eq!(item["analysis_reason"], "Generic heuristic");
                assert_eq!(item["dependency_blockers"][0]["name"], "Waiting");
                assert_eq!(item["dependency_blockers"][0]["status"], "pending");
            }
        }
    }
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
            call(
                app.clone(),
                "POST",
                path,
                json!({"balance":20000,"earned":500,"reason":"Large balance adjustment"}),
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
                .query_row(
                    "SELECT COUNT(*) FROM audit WHERE action='edit-kash'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
    }
}
pub async fn wallets(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(page): Query<lists::Page>,
) -> ApiResult<axum::Json<Value>> {
    let actor = allowed(&app, &headers)?;
    if community::role(&app, actor)? != "owner" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner permission required"));
    }
    let db = app.db.lock().unwrap();
    let total: i64 = db.query_row(
        "SELECT COUNT(*) FROM users WHERE instr(lower(username),lower(?1))>0",
        [page.term()],
        |r| r.get(0),
    )?;
    let mut stmt=db.prepare("SELECT u.id,u.username,COALESCE(w.balance,0),COALESCE(w.earned,0) FROM users u LEFT JOIN bot_wallets w ON w.user_id=u.id WHERE instr(lower(u.username),lower(?1))>0 ORDER BY u.username,u.id LIMIT ?2 OFFSET ?3")?;
    let rows=stmt.query_map(params![page.term(),page.limit(500),page.offset()],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?,"balance":r.get::<_,i64>(2)?,"earned":r.get::<_,i64>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(page.response(rows, total)))
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
    if !(0..=cannabot::MAX_KASH).contains(&input.balance)
        || !(0..=1000000).contains(&input.earned)
        || !(5..=500).contains(&input.reason.trim().len())
    {
        return Err(bad(
            "Balance must be a nonnegative safe integer, earned 0–1000000; record a reason (5–500 bytes)",
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
    Query(page): Query<lists::Page>,
) -> ApiResult<axum::Json<Value>> {
    allowed(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let total:i64=db.query_row("SELECT COUNT(*) FROM mods m JOIN mod_reviews r ON r.mod_id=m.id JOIN users u ON u.id=m.user_id WHERE r.approved=0 AND NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=m.id AND s.status='rejected') AND instr(lower(m.name || u.username),lower(?1))>0",[page.term()],|r|r.get(0))?;
    let mut statement=db.prepare("SELECT m.id,m.name,m.version,m.app_id,m.size,u.username FROM mods m JOIN mod_reviews r ON r.mod_id=m.id JOIN users u ON u.id=m.user_id WHERE r.approved=0 AND NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=m.id AND s.status='rejected') AND instr(lower(m.name || u.username),lower(?1))>0 ORDER BY m.rowid DESC LIMIT ?2 OFFSET ?3")?;
    let mut values=statement.query_map(params![page.term(),page.limit(200),page.offset()],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"version":r.get::<_,String>(2)?,"app_id":r.get::<_,i64>(3)?,"size":r.get::<_,i64>(4)?,"author":r.get::<_,String>(5)?})))?.collect::<Result<Vec<_>,_>>()?;
    for item in &mut values {
        let id = item["id"].as_str().unwrap();
        let scan: Option<(String, String, i64)> = db
            .query_row(
                "SELECT status,report,started FROM mod_scans WHERE mod_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (status, report, started) = scan.unwrap_or_else(|| ("pending".into(), "{}".into(), 0));
        let report: Value = serde_json::from_str(&report).unwrap_or(Value::Null);
        let findings: Vec<_> = report["findings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| f["accepted"] != true)
            .collect();
        item["analysis_status"] = json!(status);
        item["analysis_started"] = json!(started);
        item["unresolved_findings"] = json!(findings.len());
        item["analysis_reason"] = json!(if status == "failed" {
            report["error"]
                .as_str()
                .unwrap_or("Analysis failed; retry the scan")
                .to_owned()
        } else {
            findings
                .iter()
                .take(3)
                .filter_map(|f| f["title"].as_str())
                .collect::<Vec<_>>()
                .join("; ")
        });
        let dependencies = db.prepare("WITH RECURSIVE deps(id) AS (SELECT dep.value FROM mod_details d JOIN json_each(d.data,'$.dependency_ids') dep WHERE d.mod_id=?1 UNION SELECT dep.value FROM deps JOIN mod_details d ON d.mod_id=deps.id JOIN json_each(d.data,'$.dependency_ids') dep) SELECT deps.id,COALESCE(m.name,'Missing dependency'),s.status,s.report FROM deps LEFT JOIN mods m ON m.id=deps.id LEFT JOIN mod_scans s ON s.mod_id=deps.id LIMIT 129")?.query_map([item["id"].as_str().unwrap()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?)))?.collect::<Result<Vec<_>,_>>()?;
        let blockers: Vec<_> = dependencies
            .into_iter()
            .filter_map(|(id, name, status, report)| {
                let report: Value = report
                    .and_then(|r| serde_json::from_str(&r).ok())
                    .unwrap_or(Value::Null);
                let unresolved = report["findings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|f| f["accepted"] != true)
                    .count();
                let status = status.unwrap_or_else(|| "pending".into());
                (status != "complete" || unresolved > 0)
                    .then(|| json!({"id":id,"name":name,"status":status,"unresolved":unresolved}))
            })
            .collect();
        item["dependency_blockers"] = json!(blockers);
        item["review_reason"] = json!(
            db.query_row(
                "SELECT reason FROM mod_submissions WHERE id=?1",
                [item["id"].as_str().unwrap()],
                |r| r.get::<_, String>(0)
            )
            .optional()?
            .unwrap_or_default()
        );
    }
    let mut response = page.response(values, total);
    if response.is_object() {
        let waiting:i64=db.query_row("SELECT COUNT(*) FROM mods m WHERE NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=m.id)",[],|r|r.get(0))?;
        let scheduled: i64 = db.query_row(
            "SELECT COUNT(*) FROM mod_scans WHERE status='queued'",
            [],
            |r| r.get(0),
        )?;
        let failed: i64 = db.query_row(
            "SELECT COUNT(*) FROM mod_scans WHERE status='failed'",
            [],
            |r| r.get(0),
        )?;
        response["queue"] = json!({"waiting":waiting,"scheduled":scheduled,"failed":failed});
    }
    Ok(axum::Json(response))
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
