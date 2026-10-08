use super::*;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS invitation_settings(id INTEGER PRIMARY KEY CHECK(id=1),invites_paused INTEGER NOT NULL DEFAULT 0,registrations_paused INTEGER NOT NULL DEFAULT 0,reason TEXT NOT NULL DEFAULT '',revision INTEGER NOT NULL DEFAULT 0);
        INSERT OR IGNORE INTO invitation_settings(id) VALUES(1);
        CREATE TABLE IF NOT EXISTS user_roles(user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,role TEXT NOT NULL CHECK(role IN ('beta')),PRIMARY KEY(user_id,role));")
}

pub fn roles(db: &Connection, id: i64) -> ApiResult<Vec<String>> {
    let primary: String = db.query_row("SELECT role FROM users WHERE id=?1", [id], |r| r.get(0))?;
    let mut values = vec![primary];
    let mut stmt = db.prepare("SELECT role FROM user_roles WHERE user_id=?1 ORDER BY role")?;
    values.extend(
        stmt.query_map([id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(values)
}

pub fn has_rebound(db: &Connection, id: i64) -> ApiResult<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_roles WHERE user_id=?1 AND role='beta')",
        [id],
        |r| r.get(0),
    )?)
}

pub fn check_invites(db: &Connection) -> ApiResult<()> {
    check_pause(
        db,
        "invites_paused",
        "Invitation generation is paused by the owner",
    )
}
pub fn check_registration(db: &Connection) -> ApiResult<()> {
    check_pause(
        db,
        "registrations_paused",
        "New registrations are paused by the owner",
    )
}
fn check_pause(db: &Connection, column: &str, message: &'static str) -> ApiResult<()> {
    let paused: bool = db.query_row(
        &format!("SELECT {column} FROM invitation_settings WHERE id=1"),
        [],
        |r| r.get(0),
    )?;
    if paused {
        Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, message))
    } else {
        Ok(())
    }
}
fn settings(db: &Connection) -> ApiResult<Value> {
    Ok(db.query_row("SELECT invites_paused,registrations_paused,reason,revision FROM invitation_settings WHERE id=1", [], |r| Ok(json!({"invites_paused":r.get::<_,bool>(0)?,"registrations_paused":r.get::<_,bool>(1)?,"reason":r.get::<_,String>(2)?,"revision":r.get::<_,i64>(3)?})))?)
}
pub async fn get(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    community::owner(&app, &headers)?;
    Ok(axum::Json(settings(&app.db.lock().unwrap())?))
}
#[derive(Deserialize)]
pub struct Settings {
    invites_paused: bool,
    registrations_paused: bool,
    reason: String,
    revision: i64,
}
pub async fn set(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Settings>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    let reason = input.reason.trim();
    if reason.chars().count() < 5
        || reason.chars().count() > 500
        || reason.chars().any(char::is_control)
    {
        return Err(bad("Include a reason of 5–500 characters"));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    // Recheck permission in the same transaction as the change.
    if tx.query_row("SELECT role FROM users WHERE id=?1", [actor], |r| {
        r.get::<_, String>(0)
    })? != "owner"
    {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner access required"));
    }
    if tx.execute("UPDATE invitation_settings SET invites_paused=?1,registrations_paused=?2,reason=?3,revision=revision+1 WHERE id=1 AND revision=?4", params![input.invites_paused,input.registrations_paused,reason,input.revision])? != 1 { return Err(ApiError(StatusCode::CONFLICT, "Invitation settings changed; refresh before saving")); }
    tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'invitation-settings',?2,?3)", params![actor,json!({"invites_paused":input.invites_paused,"registrations_paused":input.registrations_paused,"reason":reason}).to_string(),now()])?;
    let value = settings(&tx)?;
    tx.commit()?;
    Ok(axum::Json(value))
}
#[derive(Deserialize)]
pub struct RoleSet {
    roles: Vec<String>,
}
pub async fn set_roles(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(target): Path<i64>,
    axum::Json(input): axum::Json<RoleSet>,
) -> ApiResult<axum::Json<Value>> {
    let actor = community::owner(&app, &headers)?;
    if input.roles.len() > 1 || input.roles.iter().any(|role| role != "beta") {
        return Err(bad("Only the additional Beta role can be assigned here"));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.query_row("SELECT role FROM users WHERE id=?1", [actor], |r| {
        r.get::<_, String>(0)
    })? != "owner"
    {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner access required"));
    }
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1)",
        [target],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(ApiError(StatusCode::NOT_FOUND, "Member not found"));
    }
    let before = roles(&tx, target)?;
    tx.execute("DELETE FROM user_roles WHERE user_id=?1", [target])?;
    if !input.roles.is_empty() {
        tx.execute(
            "INSERT INTO user_roles(user_id,role) VALUES(?1,'beta')",
            [target],
        )?;
    }
    let after = roles(&tx, target)?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'member-beta-role',?2,?3)",
        params![
            actor,
            json!({"user":target,"before":before,"after":after}).to_string(),
            now()
        ],
    )?;
    tx.commit()?;
    Ok(axum::Json(
        json!({"id":target,"roles":after,"can_rebound":input.roles.contains(&"beta".to_owned())}),
    ))
}

fn rebound_account(app: &App, headers: &HeaderMap) -> ApiResult<i64> {
    let (id, _) = app.auth(headers)?;
    if !has_rebound(&app.db.lock().unwrap(), id)? {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Canna Rebound requires the additional Beta role",
        ));
    }
    Ok(id)
}
fn support_path(app: &App) -> PathBuf {
    app.files.join("rebound/support.zip")
}
const REBOUND_LIMIT: u64 = 128 * 1024 * 1024;
pub async fn rebound_manifest(
    State(app): State<Shared>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let id = rebound_account(&app, &headers)?;
    app.limits.check(format!("rebound-manifest:{id}"), 20)?;
    let path = support_path(&app);
    let metadata = tokio::fs::metadata(&path).await.map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "Rebound support is not available yet",
        )
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > REBOUND_LIMIT {
        return Err(bad("Invalid Rebound support artifact"));
    }
    let bytes = tokio::fs::read(path).await?;
    if !bytes.starts_with(b"PK\x03\x04") || bytes.len() as u64 > REBOUND_LIMIT {
        return Err(bad("Invalid Rebound support artifact"));
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    // Recheck after disk I/O so a revoked role cannot receive a manifest.
    rebound_account(&app, &headers)?;
    Ok(([("cache-control","private, no-store"),("vary","Cookie, Authorization")], axum::Json(json!({"authorized":true,"sha256":sha256,"size":bytes.len(),"profile":"rounds-public-1.1.2","download_path":"/api/v1/rebound/support"}))).into_response())
}
pub async fn rebound_support(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    let id = rebound_account(&app, &headers)?;
    app.limits.check(format!("rebound-support:{id}"), 10)?;
    let file = tokio::fs::File::open(support_path(&app))
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "Rebound support is not available yet",
            )
        })?;
    let metadata = file.metadata().await?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > REBOUND_LIMIT {
        return Err(bad("Invalid Rebound support artifact"));
    }
    rebound_account(&app, &headers)?;
    Ok((
        [
            ("content-type", "application/zip".to_owned()),
            ("content-length", metadata.len().to_string()),
            ("cache-control", "private, no-store".to_owned()),
            ("vary", "Cookie, Authorization".to_owned()),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn pauses_do_not_spend_invites_or_consume_registration_codes() {
        let (_dir, app) = fixture();
        let owner = account(&app, "pause-owner", true);
        let member = account(&app, "pause-member", false);
        let code = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/invites",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await["invite"]
            .as_str()
            .unwrap()
            .to_owned();
        let change = json!({"invites_paused":true,"registrations_paused":true,"reason":"Maintenance preview","revision":0});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/invitation-settings",
                change,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        for (path, body, session) in [
            ("/api/v1/invites", Value::Null, Some(member.as_str())),
            (
                "/api/v1/invite-waves",
                json!({"count":2}),
                Some(owner.as_str()),
            ),
            (
                "/api/v1/register",
                json!({"username":"paused-user","email":"pause@example.org","password":"long-fixture-password","invite":code}),
                None,
            ),
        ] {
            assert_eq!(
                call(app.clone(), "POST", path, body, session)
                    .await
                    .status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        let db = app.db.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT invites_remaining FROM users WHERE id=2", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM invites WHERE hash=?1",
                [digest(&code)],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
    #[tokio::test]
    async fn owner_pause_is_separate_reversible_and_revision_checked() {
        let (_dir, app) = fixture();
        let owner = account(&app, "settings-owner", true);
        let member = account(&app, "settings-member", false);
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/invitation-settings",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let change = json!({"invites_paused":true,"registrations_paused":false,"reason":"Pause for testing","revision":0});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/invitation-settings",
                change.clone(),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/invites",
                Value::Null,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(check_registration(&app.db.lock().unwrap()).is_ok());
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/invitation-settings",
                change,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(call(app.clone(),"POST","/api/v1/admin/invitation-settings",json!({"invites_paused":false,"registrations_paused":true,"reason":"Resume invitations","revision":1}),Some(&owner)).await.status(),StatusCode::OK);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/invites",
                Value::Null,
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert!(check_registration(&app.db.lock().unwrap()).is_err());
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM audit WHERE action='invitation-settings'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
    }
    #[tokio::test]
    async fn beta_is_additional_owner_only_and_rebound_revocation_is_enforced() {
        let (_dir, app) = fixture();
        let owner = account(&app, "beta-owner", true);
        let member = account(&app, "beta-member", false);
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/rebound/support-manifest",
                Value::Null,
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/rebound/support",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/users/2/roles",
                json!({"roles":["beta"]}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/users/2/roles",
                json!({"roles":["owner"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/users/2/roles",
                json!({"roles":["beta"]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        let me =
            value(call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&member)).await).await;
        assert_eq!(me["role"], "member");
        assert_eq!(me["roles"], json!(["member", "beta"]));
        assert_eq!(me["can_rebound"], true);
        assert_eq!(me["admin"], false);
        std::fs::create_dir_all(app.files.join("rebound")).unwrap();
        let artifact = b"PK\x03\x04test-support";
        std::fs::write(support_path(&app), artifact).unwrap();
        let manifest = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/rebound/support-manifest",
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(
            manifest["sha256"],
            format!("{:x}", Sha256::digest(artifact))
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/rebound/support",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/users/2/roles",
                json!({"roles":[]}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/rebound/support-manifest",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/rebound/support",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
}
