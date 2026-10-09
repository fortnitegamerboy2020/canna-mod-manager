//! Anonymous, opt-in launcher error codes. No free-text logs or account linkage.
use super::*;
use serde::Serialize;
const RETENTION: i64 = 7 * 86400;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    schema: u8,
    consent: bool,
    session: String,
    desktop_version: String,
    platform: String,
    game_id: u32,
    operation: String,
    phase: String,
    code: String,
    http_status: Option<u16>,
    mod_count: usize,
    bliss_enabled: bool,
    loader_present: bool,
}
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS launcher_diagnostics(id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL UNIQUE,session TEXT NOT NULL,report TEXT NOT NULL,created INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS launcher_diagnostics_created ON launcher_diagnostics(created);")
}
fn validate(r: &Report) -> ApiResult<()> {
    if r.schema != 1
        || !r.consent
        || r.session.len() != 32
        || !r
            .session
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || r.desktop_version.is_empty()
        || r.desktop_version.len() > 16
        || !r
            .desktop_version
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.')
        || !matches!(r.platform.as_str(), "windows" | "linux" | "macos")
        || !game_profiles::supports_game(r.game_id)
        || !matches!(
            r.operation.as_str(),
            "prepare"
                | "modded_launch"
                | "vanilla_launch"
                | "current_launch"
                | "restore"
                | "startup"
        )
        || !matches!(
            r.phase.as_str(),
            "loader"
                | "compatibility"
                | "mod_download"
                | "catalog"
                | "restore"
                | "launch"
                | "prepare"
        )
        || !matches!(
            r.code.as_str(),
            "session_expired"
                | "access_denied"
                | "rate_limited"
                | "checksum_mismatch"
                | "unsupported_compatibility"
                | "dependency_unavailable"
                | "game_running"
                | "prepared_state_changed"
                | "network_failure"
                | "launch_failure"
                | "filesystem_failure"
                | "setup_failure"
                | "process_not_detected"
                | "process_status_unavailable"
                | "loader_initialization"
                | "plugin_load_failure"
                | "compatibility_mismatch"
        )
        || r.http_status.is_some_and(|s| !(400..=599).contains(&s))
        || r.mod_count > 1000
    {
        return Err(bad("Invalid anonymous launcher diagnostic"));
    }
    Ok(())
}
fn cleanup(db: &Connection) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM launcher_diagnostics WHERE created<?1",
        [now() - RETENTION],
    )?;
    db.execute("DELETE FROM launcher_diagnostics WHERE id IN (SELECT id FROM launcher_diagnostics ORDER BY created DESC,id DESC LIMIT -1 OFFSET 5000)",[])?;
    Ok(())
}
pub fn start_cleanup(app: Shared) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Ok(db) = app.db.lock()
                && cleanup(&db).is_err()
            {
                eprintln!("Launcher diagnostic retention cleanup failed");
            }
        }
    });
}
pub async fn submit(
    State(app): State<Shared>,
    axum::Json(report): axum::Json<Report>,
) -> ApiResult<Response> {
    validate(&report)?;
    app.limits.check("launcher-diagnostics-global".into(), 60)?;
    app.limits
        .check(format!("launcher-diagnostics:{}", report.session), 6)?;
    let encoded = serde_json::to_string(&report).map_err(|_| bad("Invalid launcher diagnostic"))?;
    let fingerprint = format!("{:x}", Sha256::digest(encoded.as_bytes()));
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    cleanup(&tx)?;
    let changed = tx.execute(
        "INSERT OR IGNORE INTO launcher_diagnostics VALUES(?1,?2,?3,?4,?5)",
        params![
            Uuid::new_v4().to_string(),
            fingerprint,
            report.session,
            encoded,
            now()
        ],
    )?;
    cleanup(&tx)?;
    tx.commit()?;
    Ok((
        [("cache-control", "no-store")],
        axum::Json(json!({"accepted":true,"deduplicated":changed==0})),
    )
        .into_response())
}
pub async fn list(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    let (_, admin) = app.auth(&headers)?;
    if !admin {
        return Err(ApiError(StatusCode::FORBIDDEN, "Staff access required"));
    }
    let db = app.db.lock().unwrap();
    cleanup(&db)?;
    let reports=db.prepare("SELECT id,report,created FROM launcher_diagnostics ORDER BY created DESC,id DESC LIMIT 100")?.query_map([],|row|Ok(json!({"id":row.get::<_,String>(0)?,"report":serde_json::from_str::<Value>(&row.get::<_,String>(1)?).unwrap_or(Value::Null),"created":row.get::<_,i64>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(([("cache-control","private, no-store"),("vary","Cookie, Authorization")],axum::Json(json!({"reports":reports,"retention_days":7,"limit":100,"untrusted_client_reports":true}))).into_response())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    fn sample() -> Value {
        json!({"schema":1,"consent":true,"session":"a".repeat(32),"desktop_version":"0.2.46","platform":"windows","game_id":1686940,"operation":"prepare","phase":"loader","code":"access_denied","http_status":403,"mod_count":14,"bliss_enabled":false,"loader_present":false})
    }
    #[tokio::test]
    async fn reports_are_anonymous_deduplicated_expiring_and_staff_only() {
        let (_dir, app) = fixture();
        let owner = account(&app, "launcher-owner", true);
        let member = account(&app, "launcher-member", false);
        for repeated in [false, true] {
            let body = value(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/launcher/diagnostics",
                    sample(),
                    None,
                )
                .await,
            )
            .await;
            assert_eq!(body["accepted"], true);
            assert_eq!(body["deduplicated"], repeated);
        }
        for token in [None, Some(member.as_str())] {
            assert_ne!(
                call(
                    app.clone(),
                    "GET",
                    "/api/v1/admin/launcher-diagnostics",
                    Value::Null,
                    token
                )
                .await
                .status(),
                StatusCode::OK
            );
        }
        let body = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/launcher-diagnostics",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(body["reports"].as_array().unwrap().len(), 1);
        let raw = body.to_string();
        for private in [
            "launcher-owner",
            "launcher-member",
            "user_id",
            "email",
            "token",
            "C:\\",
        ] {
            assert!(!raw.contains(private));
        }
        let db = app.db.lock().unwrap();
        db.execute(
            "UPDATE launcher_diagnostics SET created=?1",
            [now() - RETENTION - 1],
        )
        .unwrap();
        cleanup(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM launcher_diagnostics", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn missing_consent_private_text_invalid_enums_and_oversized_payloads_are_rejected() {
        let (_dir, app) = fixture();
        for change in 0..9 {
            let mut r = sample();
            match change {
                0 => r["consent"] = json!(false),
                1 => r["log"] = json!("private"),
                2 => r["code"] = json!("arbitrary error"),
                3 => r["session"] = json!("bad"),
                4 => r["mod_count"] = json!(1001),
                5 => r["http_status"] = json!(200),
                6 => r["game_id"] = json!(0),
                7 => r["platform"] = json!("private-machine-name"),
                _ => r["session"] = json!("a".repeat(8192)),
            };
            assert!(
                !call(app.clone(), "POST", "/api/v1/launcher/diagnostics", r, None)
                    .await
                    .status()
                    .is_success()
            );
        }
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM launcher_diagnostics", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn anonymous_reports_are_rate_limited_and_retention_has_a_hard_cap() {
        let (_dir, app) = fixture();
        for i in 0..7 {
            let response = call(
                app.clone(),
                "POST",
                "/api/v1/launcher/diagnostics",
                sample(),
                None,
            )
            .await;
            assert_eq!(
                response.status(),
                if i < 6 {
                    StatusCode::OK
                } else {
                    StatusCode::TOO_MANY_REQUESTS
                }
            );
        }
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        for i in 0..5002 {
            tx.execute(
                "INSERT INTO launcher_diagnostics VALUES(?1,?1,?2,'{}',?3)",
                params![format!("fixture-{i}"), "f".repeat(32), now()],
            )
            .unwrap();
        }
        cleanup(&tx).unwrap();
        assert_eq!(
            tx.query_row("SELECT COUNT(*) FROM launcher_diagnostics", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            5000
        );
        tx.commit().unwrap();
    }
}
