use super::*;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS beta_wave_reviews(hash TEXT PRIMARY KEY,owner INTEGER NOT NULL,proposal TEXT NOT NULL,expires INTEGER NOT NULL);")
}
#[derive(Deserialize)]
pub struct Input {
    #[serde(default)]
    count: u32,
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    review: String,
}
pub async fn wave(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Input>,
) -> ApiResult<Response> {
    let owner = community::owner(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("DELETE FROM beta_wave_reviews WHERE expires<=?1", [now()])?;
    if input.review.is_empty() {
        if !(1..=50).contains(&input.count) {
            return Err(bad("Choose 1–50 accounts"));
        }
        let order = match input.pattern.as_str() {
            "random" => "RANDOM()",
            "newest" => "u.id DESC",
            "oldest" => "u.id ASC",
            "most_posts" => "(SELECT COUNT(*) FROM posts p WHERE p.user_id=u.id) DESC,u.id",
            _ => return Err(bad("Choose random, newest, oldest or most_posts")),
        };
        let sql = format!(
            "SELECT u.id,u.username FROM users u WHERE u.id<>?1 AND u.verified=1 AND u.banned=0 AND NOT EXISTS(SELECT 1 FROM user_roles r WHERE r.user_id=u.id AND r.role='beta') ORDER BY {order} LIMIT ?2"
        );
        let candidates = tx
            .prepare(&sql)?
            .query_map(params![owner, input.count], |r| {
                Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?}))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if candidates.is_empty() {
            return Err(bad("No eligible accounts without Beta access"));
        }
        let proposal = json!({"pattern":input.pattern,"members":candidates});
        let review = token();
        tx.execute("DELETE FROM beta_wave_reviews WHERE owner=?1", [owner])?;
        tx.execute(
            "INSERT INTO beta_wave_reviews VALUES(?1,?2,?3,?4)",
            params![digest(&review), owner, proposal.to_string(), now() + 300],
        )?;
        tx.commit()?;
        return Ok((
            [
                ("cache-control", "private, no-store"),
                ("vary", "Cookie, Authorization"),
            ],
            axum::Json(
                json!({"review":review,"expires_in":300,"proposal":proposal,"applied":false}),
            ),
        )
            .into_response());
    }
    if input.review.len() != 64 {
        return Err(bad("Invalid Beta wave review"));
    }
    let raw: String = tx
        .query_row(
            "SELECT proposal FROM beta_wave_reviews WHERE hash=?1 AND owner=?2 AND expires>?3",
            params![digest(&input.review), owner, now()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| bad("Beta wave preview expired or was already applied; preview again"))?;
    let proposal: Value =
        serde_json::from_str(&raw).map_err(|_| bad("Invalid Beta wave preview"))?;
    let members = proposal["members"]
        .as_array()
        .ok_or_else(|| bad("Invalid Beta wave preview"))?;
    for member in members {
        let id = member["id"]
            .as_i64()
            .ok_or_else(|| bad("Invalid Beta wave member"))?;
        if tx.execute("INSERT INTO user_roles(user_id,role) SELECT id,'beta' FROM users WHERE id=?1 AND banned=0 AND verified=1 AND NOT EXISTS(SELECT 1 FROM user_roles WHERE user_id=?1 AND role='beta')",[id])?!=1{return Err(bad("A selected account changed; preview the wave again"));}
        notifications::notify(
            &tx,
            id,
            "beta-access",
            "You received Beta access. Enable Canna Bliss in desktop Settings to try the preview.",
            "#profile",
            &format!("beta-access:{}", digest(&input.review)),
        )?;
    }
    tx.execute(
        "DELETE FROM beta_wave_reviews WHERE hash=?1",
        [digest(&input.review)],
    )?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'beta-wave-granted',?2,?3)",
        params![owner, raw, now()],
    )?;
    tx.commit()?;
    app.live.hint("notifications");
    Ok((
        [
            ("cache-control", "private, no-store"),
            ("vary", "Cookie, Authorization"),
        ],
        axum::Json(json!({"applied":true,"granted":members.len()})),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn beta_access_waves_preview_patterns_keep_primary_roles_and_cannot_replay() {
        let (_dir, app) = fixture();
        let owner = account(&app, "wave-owner", true);
        let member = account(&app, "wave-oldest", false);
        account(&app, "wave-newest", false);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/beta-waves",
                json!({"count":2,"pattern":"random"}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/beta-waves",
                json!({"count":1,"pattern":"oldest"}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(first["proposal"]["members"][0]["username"], "wave-oldest");
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM user_roles", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let newest = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/beta-waves",
                json!({"count":1,"pattern":"newest"}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(newest["proposal"]["members"][0]["username"], "wave-newest");
        let applied = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/beta-waves",
                json!({"review":newest["review"]}),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(applied["granted"], 1);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/beta-waves",
                json!({"review":newest["review"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let db = app.db.lock().unwrap();
        assert_eq!(
            db.query_row(
                "SELECT role FROM users WHERE username='wave-newest'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "member"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM notifications WHERE kind='beta-access'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn beta_wave_aborts_atomically_when_a_previewed_account_changes() {
        let (_dir, app) = fixture();
        let owner = account(&app, "wave-owner", true);
        account(&app, "wave-one", false);
        account(&app, "wave-two", false);
        let preview = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/beta-waves",
                json!({"count":2,"pattern":"oldest"}),
                Some(&owner),
            )
            .await,
        )
        .await;
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET banned=1 WHERE username='wave-two'", [])
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/beta-waves",
                json!({"review":preview["review"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM user_roles", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
