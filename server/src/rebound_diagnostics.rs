use super::*;
use serde::Serialize;

const RETENTION: i64 = 7 * 86400;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS rebound_diagnostics(id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL UNIQUE,session TEXT NOT NULL,actor INTEGER NOT NULL,report TEXT NOT NULL,created INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS rebound_diagnostics_created ON rebound_diagnostics(created);")
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    schema: u32,
    consent: bool,
    session: String,
    actor: u32,
    guard_version: String,
    category: String,
    local: Peer,
    peers: Vec<Peer>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Peer {
    actor: u32,
    game: String,
    content: String,
    mods: String,
    assets: String,
    config: String,
    files: Vec<File>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct File {
    key: String,
    sha256: String,
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn valid_peer(peer: &Peer) -> bool {
    let mut seen = std::collections::HashSet::new();
    (1..=64).contains(&peer.actor)
        && [
            &peer.game,
            &peer.content,
            &peer.mods,
            &peer.assets,
            &peer.config,
        ]
        .iter()
        .all(|value| hex(value, 64))
        && peer.files.len() <= 32
        && peer.files.iter().all(|file| {
            hex(&file.sha256, 64)
                && seen.insert(&file.key)
                && (matches!(
                    file.key.as_str(),
                    "cards" | "maps" | "unbound" | "cr" | "classes"
                ) || file
                    .key
                    .strip_prefix("other:")
                    .is_some_and(|value| hex(value, 64)))
        })
}
fn validate(report: &Report) -> ApiResult<()> {
    let mut actors = std::collections::HashSet::new();
    if report.schema != 1
        || !report.consent
        || !hex(&report.session, 32)
        || report.actor != report.local.actor
        || !valid_peer(&report.local)
        || report.guard_version.is_empty()
        || report.guard_version.len() > 16
        || !report
            .guard_version
            .bytes()
            .all(|c| c.is_ascii_digit() || c == b'.')
        || !matches!(
            report.category.as_str(),
            "settings" | "mods" | "assets" | "game" | "waiting" | "invalid"
        )
        || report.peers.len() > 16
        || report
            .peers
            .iter()
            .any(|peer| !valid_peer(peer) || !actors.insert(peer.actor))
    {
        return Err(bad("Invalid anonymous Bliss diagnostic"));
    }
    Ok(())
}
fn cleanup(db: &Connection) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM rebound_diagnostics WHERE created<?1",
        [now() - RETENTION],
    )?;
    db.execute("DELETE FROM rebound_diagnostics WHERE id IN (SELECT id FROM rebound_diagnostics ORDER BY created DESC,id DESC LIMIT -1 OFFSET 5000)",[])?;
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
                eprintln!("Bliss diagnostic retention cleanup failed");
            }
        }
    });
}
pub async fn submit(
    State(app): State<Shared>,
    axum::Json(report): axum::Json<Report>,
) -> ApiResult<Response> {
    validate(&report)?;
    app.limits.check("rebound-diagnostics-global".into(), 60)?;
    app.limits
        .check(format!("rebound-diagnostics:{}", report.session), 6)?;
    let encoded = serde_json::to_string(&report).map_err(|_| bad("Invalid diagnostic"))?;
    if encoded.len() > 16 * 1024 {
        return Err(bad("Diagnostic exceeds size limit"));
    }
    let fingerprint = format!("{:x}", Sha256::digest(encoded.as_bytes()));
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    cleanup(&tx)?;
    let changed = tx.execute(
        "INSERT OR IGNORE INTO rebound_diagnostics VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            Uuid::new_v4().to_string(),
            fingerprint,
            report.session,
            report.actor,
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
    let reports=db.prepare("SELECT id,report,created FROM rebound_diagnostics ORDER BY created DESC,id DESC LIMIT 100")?.query_map([],|row|Ok(json!({"id":row.get::<_,String>(0)?,"report":serde_json::from_str::<Value>(&row.get::<_,String>(1)?).unwrap_or(Value::Null),"created":row.get::<_,i64>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(([("cache-control","private, no-store"),("vary","Cookie, Authorization")],axum::Json(json!({"reports":reports,"retention_days":7,"limit":100,"untrusted_client_reports":true}))).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    fn sample() -> Value {
        let peer = json!({"actor":1,"game":"a".repeat(64),"content":"b".repeat(64),"mods":"c".repeat(64),"assets":"d".repeat(64),"config":"e".repeat(64),"files":[{"key":"cr","sha256":"f".repeat(64)}]});
        json!({"schema":1,"consent":true,"session":"1".repeat(32),"actor":1,"guard_version":"0.1.3","category":"settings","local":peer,"peers":[]})
    }
    #[tokio::test]
    async fn opt_in_reports_are_anonymous_deduplicated_staff_only_and_expiring() {
        let (_dir, app) = fixture();
        let owner = account(&app, "diagnostic-owner", true);
        let member = account(&app, "diagnostic-member", false);
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/rebound/diagnostics",
                sample(),
                None,
            )
            .await,
        )
        .await;
        assert_eq!(first["accepted"], true);
        let again = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/rebound/diagnostics",
                sample(),
                None,
            )
            .await,
        )
        .await;
        assert_eq!(again["deduplicated"], true);
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/rebound-diagnostics",
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
                "/api/v1/admin/rebound-diagnostics",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let rows = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/rebound-diagnostics",
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(rows["reports"].as_array().unwrap().len(), 1);
        let db = app.db.lock().unwrap();
        let raw: String = db
            .query_row("SELECT report FROM rebound_diagnostics", [], |r| r.get(0))
            .unwrap();
        assert!(
            !raw.contains("diagnostic-member")
                && !raw.contains("authorization")
                && !raw.contains("username")
        );
        db.execute(
            "UPDATE rebound_diagnostics SET created=?1",
            [now() - RETENTION - 1],
        )
        .unwrap();
        cleanup(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM rebound_diagnostics", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn missing_consent_private_fields_and_invalid_hashes_are_refused_before_writes() {
        let (_dir, app) = fixture();
        for field in [
            "username",
            "steam_id",
            "path",
            "ip",
            "account_id",
            "setting_values",
        ] {
            let mut input = sample();
            input[field] = json!("private");
            assert!(
                !call(
                    app.clone(),
                    "POST",
                    "/api/v1/rebound/diagnostics",
                    input,
                    None
                )
                .await
                .status()
                .is_success()
            );
        }
        for input in [
            {
                let mut v = sample();
                v["consent"] = json!(false);
                v
            },
            {
                let mut v = sample();
                v["local"]["files"][0]["key"] = json!("C:/Users/name/config.cfg");
                v
            },
            {
                let mut v = sample();
                v["local"]["content"] = json!("bad");
                v
            },
        ] {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/rebound/diagnostics",
                    input,
                    None
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM rebound_diagnostics", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn anonymous_uploads_have_a_room_rate_limit() {
        let (_dir, app) = fixture();
        for _ in 0..6 {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/rebound/diagnostics",
                    sample(),
                    None
                )
                .await
                .status(),
                StatusCode::OK
            );
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/rebound/diagnostics",
                sample(),
                None
            )
            .await
            .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
