use super::*;
use tokio::io::AsyncWriteExt;
const REPORT_LIMIT: u64 = 16 * 1024 * 1024;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS mod_scans(mod_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE,hash TEXT NOT NULL,status TEXT NOT NULL,report TEXT NOT NULL,started INTEGER NOT NULL);
 UPDATE mod_scans SET status='failed',report='{\"status\":\"failed\",\"error\":\"Analysis interrupted; run again\",\"files\":[],\"findings\":[]}' WHERE status='queued';")
}
fn staff(app: &App, headers: &HeaderMap) -> ApiResult<i64> {
    let (actor, admin) = app.auth(headers)?;
    if !admin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Administrator permission required",
        ));
    }
    Ok(actor)
}
pub fn require_review(db: &Connection, id: &str) -> ApiResult<()> {
    let scan: Option<(String, String)> = db
        .query_row(
            "SELECT status,report FROM mod_scans WHERE mod_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((status, text)) = scan else {
        if std::env::var_os("CANNA_REVIEW_JOBS").is_some() {
            return Err(bad("Automatic source analysis is pending"));
        }
        return Ok(());
    };
    if status != "complete" {
        return Err(bad(
            "Source analysis is pending or failed; inspect the review workspace",
        ));
    }
    let report: Value = serde_json::from_str(&text).map_err(|_| bad("Source report is invalid"))?;
    if report["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|f| f["accepted"] != true)
    {
        return Err(bad(
            "Resolve suspicious-code findings in the review workspace before approval",
        ));
    }
    Ok(())
}
pub async fn page(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    staff(&app, &headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    Ok(Html(include_str!("../web/review.html")).into_response())
}
// Explicit Unix modes let the isolated worker read only the job handed to it.
#[cfg(unix)]
fn mode(path: &std::path::Path, value: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(value))
}
#[cfg(not(unix))]
fn mode(_path: &std::path::Path, _value: u32) -> std::io::Result<()> {
    Ok(())
}
async fn run(app: Shared, id: String, hash: String, job: PathBuf) -> ApiResult<()> {
    tokio::fs::create_dir(&job).await?;
    mode(&job, 0o2770)?;
    let file = tokio::fs::File::open(app.files.join(format!("{id}.zip"))).await?;
    let mut output = tokio::fs::File::create(job.join("input.zip")).await?;
    mode(&job.join("input.zip"), 0o660)?;
    let stream = crypto::read(file, Zeroizing::new(*app.upload_key), id.clone());
    tokio::pin!(stream);
    let mut total = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total += chunk.len();
        if total > UPLOAD_LIMIT {
            return Err(bad("Archive exceeds scan limit"));
        }
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    drop(output);
    tokio::fs::write(job.join("ready"), b"ready").await?;
    mode(&job.join("ready"), 0o660)?;
    let result = job.join("result.json");
    let started = tokio::time::Instant::now();
    loop {
        if let Ok(meta) = tokio::fs::metadata(&result).await {
            if meta.len() > REPORT_LIMIT {
                return Err(bad("Source analysis report exceeds limits"));
            }
            let text = tokio::fs::read_to_string(&result).await?;
            let report: Value =
                serde_json::from_str(&text).map_err(|_| bad("Invalid worker report"))?;
            if report["status"] != "complete" {
                return Err(bad("Source analysis failed; run again"));
            }
            let mut db = app.db.lock().unwrap();
            let tx = db.transaction()?;
            let stored: Option<String> = tx
                .query_row("SELECT sha256 FROM mods WHERE id=?1", [&id], |r| r.get(0))
                .optional()?;
            if stored.as_deref() != Some(&hash) {
                return Err(bad("Mod changed or was removed during analysis"));
            }
            tx.execute(
                "UPDATE mod_scans SET status='complete',report=?1 WHERE mod_id=?2 AND hash=?3",
                params![text, id, hash],
            )?;
            if report["findings"].as_array().is_some_and(|f| !f.is_empty()) {
                tx.execute("INSERT INTO mod_reviews VALUES(?1,0) ON CONFLICT(mod_id) DO UPDATE SET approved=0",[&id])?;
            }
            let approved: bool = tx.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM mod_reviews WHERE mod_id=?1 AND approved=0)",
                [&id],
                |r| r.get(0),
            )?;
            if approved && require_review(&tx, &id).is_ok() {
                notifications::accepted(&tx, &id)?;
            }
            tx.commit()?;
            app.live.hint("library");
            app.live.hint("notifications");
            return Ok(());
        }
        if started.elapsed().as_secs() > 900 {
            return Err(bad("Source analysis timed out; run again"));
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
async fn queue(app: Shared, id: String, force: bool) -> ApiResult<()> {
    let root = std::env::var("CANNA_REVIEW_JOBS")
        .map_err(|_| bad("Source analysis worker is not configured"))?;
    let hash = {
        let db = app.db.lock().unwrap();
        let hash: String = db
            .query_row("SELECT sha256 FROM mods WHERE id=?1", [&id], |r| r.get(0))
            .optional()?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "Mod not found"))?;
        let existing: Option<String> = db
            .query_row("SELECT status FROM mod_scans WHERE mod_id=?1", [&id], |r| {
                r.get(0)
            })
            .optional()?;
        if existing.as_deref() == Some("queued") || (existing.is_some() && !force) {
            return Ok(());
        }
        let active: i64 = db.query_row(
            "SELECT COUNT(*) FROM mod_scans WHERE status='queued'",
            [],
            |r| r.get(0),
        )?;
        if active >= 2 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "Analysis queue is full; try shortly",
            ));
        }
        db.execute("INSERT INTO mod_scans VALUES(?1,?2,'queued','{}',?3) ON CONFLICT(mod_id) DO UPDATE SET hash=excluded.hash,status='queued',report='{}',started=excluded.started",params![id,hash,now()])?;
        hash
    };
    let job = PathBuf::from(root).join(Uuid::new_v4().to_string());
    tokio::spawn(async move {
        if run(app.clone(), id.clone(), hash, job.clone())
            .await
            .is_err()
        {
            let _=app.db.lock().unwrap().execute("UPDATE mod_scans SET status='failed',report=?1 WHERE mod_id=?2",params![json!({"status":"failed","error":"Analysis unavailable, interrupted or over limits. Run again; it has not passed inspection.","files":[],"findings":[]}).to_string(),id]);
        }
        let _ = tokio::fs::remove_dir_all(job).await;
    });
    Ok(())
}
pub fn start(app: Shared) {
    if std::env::var_os("CANNA_REVIEW_JOBS").is_none() {
        return;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let id: Option<String> = {
                let db = app.db.lock().unwrap();
                db.query_row("SELECT id FROM mods WHERE NOT EXISTS(SELECT 1 FROM mod_scans WHERE mod_id=mods.id) ORDER BY rowid DESC LIMIT 1",[],|r|r.get(0)).optional().unwrap_or(None)
            };
            if let Some(id) = id {
                let _ = queue(app.clone(), id, false).await;
            }
        }
    });
}
pub async fn report(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    staff(&app, &headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    let db = app.db.lock().unwrap();
    let name: String = db
        .query_row("SELECT name FROM mods WHERE id=?1", [&id], |r| r.get(0))
        .optional()?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "Mod not found"))?;
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT status,report,hash FROM mod_scans WHERE mod_id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let mut result = if let Some((status, text, hash)) = row {
        let mut v: Value = serde_json::from_str(&text).map_err(|_| bad("Invalid report"))?;
        v["status"] = json!(status);
        v["sha256"] = json!(hash);
        v
    } else {
        json!({"status":"pending","files":[],"findings":[]})
    };
    result["mod_name"] = json!(name);
    Ok(axum::Json(result))
}
#[derive(Deserialize)]
pub struct Start {
    #[serde(default)]
    force: bool,
}
pub async fn analyze(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<Start>,
) -> ApiResult<axum::Json<Value>> {
    let actor = staff(&app, &headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    app.limits.check(format!("analysis:{actor}"), 5)?;
    queue(app.clone(), id.clone(), input.force).await?;
    app.db.lock().unwrap().execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'analyze-mod',?2,?3)",
        params![actor, id, now()],
    )?;
    Ok(axum::Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct Decision {
    accepted: bool,
    reason: String,
    sha256: String,
}
pub async fn decision(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((id, fid)): Path<(String, String)>,
    axum::Json(input): axum::Json<Decision>,
) -> ApiResult<axum::Json<Value>> {
    let actor = staff(&app, &headers)?;
    if input.reason.trim().len() < 5 || input.reason.len() > 500 {
        return Err(bad("Record a review reason between 5 and 500 bytes"));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let (text, hash): (String, String) = tx
        .query_row(
            "SELECT report,hash FROM mod_scans WHERE mod_id=?1 AND status='complete'",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(bad("Completed analysis not found"))?;
    if hash != input.sha256 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Mod analysis changed; reload",
        ));
    }
    let mut report: Value = serde_json::from_str(&text).map_err(|_| bad("Invalid report"))?;
    let finding = report["findings"]
        .as_array_mut()
        .and_then(|a| a.iter_mut().find(|f| f["id"] == fid))
        .ok_or(bad("Finding not found"))?;
    finding["accepted"] = json!(input.accepted);
    finding["reason"] = json!(input.reason.trim());
    finding["reviewer"] = json!(actor);
    finding["reviewed"] = json!(now());
    tx.execute(
        "UPDATE mod_scans SET report=?1 WHERE mod_id=?2",
        params![report.to_string(), id],
    )?;
    if !input.accepted {
        tx.execute("UPDATE mod_reviews SET approved=0 WHERE mod_id=?1", [&id])?;
    }
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'scan-finding-decision',?2,?3)",
        params![
            actor,
            json!({"mod":id,"finding":fid,"accepted":input.accepted,"reason":input.reason})
                .to_string(),
            now()
        ],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn analysis_is_staff_only_and_unresolved_findings_block_downloads() {
        let (_dir, app) = fixture();
        let member = account(&app, "member", false);
        let owner = account(&app, "owner", true);
        let id = Uuid::new_v4().to_string();
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1686940,'Fixture','1','','sha',100)",
                [&id],
            )
            .unwrap();
            db.execute("INSERT INTO mod_reviews VALUES(?1,1)", [&id])
                .unwrap();
            db.execute("INSERT INTO mod_scans VALUES(?1,'sha','complete',?2,0)",params![id,json!({"status":"complete","files":[],"findings":[{"id":"finding","title":"Starting commands"}]}).to_string()]).unwrap();
            assert!(require_review(&db, &id).is_err());
        }
        for path in [
            format!("/review/mods/{id}"),
            format!("/api/v1/mods/{id}/analysis"),
            "/review.js".into(),
        ] {
            assert_eq!(
                call(app.clone(), "GET", &path, Value::Null, None)
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                call(app.clone(), "GET", &path, Value::Null, Some(&member))
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        let path = format!("/api/v1/mods/{id}/analysis/finding");
        let body = json!({"accepted":true,"reason":"Reviewed benign launch helper","sha256":"old"});
        assert_eq!(
            call(app.clone(), "POST", &path, body.clone(), Some(&owner))
                .await
                .status(),
            StatusCode::CONFLICT
        );
        let mut body = body;
        body["sha256"] = json!("sha");
        assert_eq!(
            call(app.clone(), "POST", &path, body.clone(), Some(&member))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert!(
            call(app.clone(), "POST", &path, body, Some(&owner))
                .await
                .status()
                .is_success()
        );
        assert!(require_review(&app.db.lock().unwrap(), &id).is_ok());
        let report = value(
            call(
                app.clone(),
                "GET",
                &format!("/api/v1/mods/{id}/analysis"),
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(report["findings"][0]["accepted"], true);
        assert!(
            call(
                app.clone(),
                "POST",
                &path,
                json!({"accepted":false,"reason":"Needs another look","sha256":"sha"}),
                Some(&owner)
            )
            .await
            .status()
            .is_success()
        );
        assert!(security::approved(&app.db.lock().unwrap(), &id).is_err());
    }
}
