use super::*;
use tokio::io::AsyncWriteExt;
const REPORT_LIMIT: u64 = 16 * 1024 * 1024;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS mod_scans(mod_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE,hash TEXT NOT NULL,status TEXT NOT NULL,report TEXT NOT NULL,started INTEGER NOT NULL);
 DELETE FROM mod_scans WHERE status='queued';")
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
fn manual_upload(db: &Connection, id: &str) -> ApiResult<bool> {
    Ok(db.query_row("SELECT NOT EXISTS(SELECT 1 FROM mod_details WHERE mod_id=?1 AND COALESCE(json_extract(data,'$.provider'),'uploaded')!='uploaded')",[id],|r|r.get(0))?)
}
pub fn enforce_manual_uploads(db: &Connection) -> rusqlite::Result<()> {
    let transaction = db.unchecked_transaction()?;
    let db = &transaction;
    if db.execute(
        "INSERT OR IGNORE INTO notification_meta VALUES('manual-upload-review-v1')",
        [],
    )? == 0
    {
        return Ok(());
    }
    let ids=db.prepare("SELECT m.id,m.user_id,m.name FROM mods m WHERE NOT EXISTS(SELECT 1 FROM mod_details d WHERE d.mod_id=m.id AND COALESCE(json_extract(d.data,'$.provider'),'uploaded')!='uploaded') AND NOT EXISTS(SELECT 1 FROM audit a WHERE a.action='approve-mod' AND a.target=m.id) AND NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=m.id AND s.status='rejected') AND NOT EXISTS(SELECT 1 FROM mod_submissions s WHERE s.id=m.id AND s.status='denied')")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
    for (id, user, name) in ids {
        db.execute(
            "INSERT INTO mod_reviews VALUES(?1,0) ON CONFLICT(mod_id) DO UPDATE SET approved=0",
            [&id],
        )?;
        db.execute("INSERT OR IGNORE INTO mod_submissions(id,user_id,name,version,created) SELECT id,user_id,name,version,?1 FROM mods WHERE id=?2",params![now(),id])?;
        db.execute("UPDATE mod_submissions SET status='pending',reason='Manual staff approval required',resolved=NULL WHERE id=?1",[&id])?;
        db.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'manual-upload-review-required',?2,?3)",params![user,json!({"mod":id,"name":name}).to_string(),now()])?;
    }
    transaction.commit()
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
fn worker_file(path: &std::path::Path, job: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::os::unix::fs::chown(path, None, Some(std::fs::metadata(job)?.gid()))?;
    }
    #[cfg(not(unix))]
    let _ = job;
    mode(path, 0o660)
}
async fn run(app: Shared, id: String, hash: String, job: PathBuf) -> ApiResult<()> {
    tokio::fs::create_dir(&job).await?;
    // The spool supplies the job's worker group. Explicit file group assignment
    // avoids setgid chmod, which systemd RestrictSUIDSGID deliberately rejects.
    mode(&job, 0o770)?;
    let file = tokio::fs::File::open(app.files.join(format!("{id}.zip"))).await?;
    let mut output = tokio::fs::File::create(job.join("input.zip")).await?;
    worker_file(&job.join("input.zip"), &job)?;
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
    worker_file(&job.join("ready"), &job)?;
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
            apply_policy(&tx, &id, &report)?;
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
pub fn requeue_uncertain_denials(db: &mut Connection) -> anyhow::Result<usize> {
    let tx = db.transaction()?;
    let rows=tx.prepare("SELECT s.mod_id,m.user_id,s.report FROM mod_scans s JOIN mods m ON m.id=s.mod_id JOIN mod_submissions sub ON sub.id=m.id WHERE s.status='rejected' AND sub.reason LIKE 'Rejected by scan policy:%' AND json_extract(s.report,'$.version')='canna-static-4'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
    let mut count = 0;
    for (id, actor, raw) in rows {
        let report: Value = serde_json::from_str(&raw)?;
        let uncertain = report["findings"].as_array().is_some_and(|findings| {
            findings.iter().any(|f| {
                f["rule"] == "packer-heuristic"
                    || (f["rule"] == "packer-marker" && f["evidence"] == "MPRESS")
                    || (f["rule"] == "packer-signature"
                        && f["evidence"].as_str().is_some_and(|e| e.contains("(Heur)")))
            })
        });
        if !uncertain {
            continue;
        }
        // Reanalysis grants no download approval, even for an owner import.
        tx.execute("DELETE FROM mod_scans WHERE mod_id=?1", [&id])?;
        tx.execute("UPDATE mod_reviews SET approved=0 WHERE mod_id=?1", [&id])?;
        tx.execute("UPDATE mod_submissions SET status='pending',reason='Scanner correction: reanalysis required; uncertain packing evidence is reviewed by staff',resolved=NULL WHERE id=?1",[&id])?;
        tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'scanner-correction-requeue',?2,?3)",params![actor,id,now()])?;
        notifications::notify(
            &tx,
            actor,
            "mod-review",
            "Your mod was returned to review for corrected packing analysis.",
            "/submissions",
            &format!("scanner-correction-5:{id}"),
        )?;
        count += 1;
    }
    tx.commit()?;
    Ok(count)
}
fn apply_policy(db: &Connection, id: &str, report: &Value) -> ApiResult<()> {
    let (actor, name): (i64, String) =
        db.query_row("SELECT user_id,name FROM mods WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let findings = report["findings"]
        .as_array()
        .ok_or(bad("Invalid findings"))?;
    let blocked: Vec<_> = findings
        .iter()
        .filter(|f| {
            f["rule"]
                .as_str()
                .is_some_and(|rule| rule.starts_with("packer-") || rule == "signature")
        })
        .collect();
    let (action, reason) = if !blocked.is_empty() {
        let reason = format!(
            "Rejected by scan policy: {}",
            blocked
                .iter()
                .take(8)
                .map(|f| format!(
                    "{}: {} ({})",
                    f["title"].as_str().unwrap_or("Detection"),
                    f["evidence"].as_str().unwrap_or(""),
                    f["file"].as_str().unwrap_or("archive")
                ))
                .collect::<Vec<_>>()
                .join("; ")
        );
        db.execute(
            "UPDATE mod_scans SET status='rejected' WHERE mod_id=?1",
            [id],
        )?;
        db.execute(
            "UPDATE mod_submissions SET status='denied',reason=?1,resolved=?2 WHERE id=?3",
            params![reason, now(), id],
        )?;
        notifications::notify(
            db,
            actor,
            "mod-denied",
            &format!("Your mod {name} was denied: {reason}"),
            "/submissions",
            &format!("scan-denied:{id}"),
        )?;
        ("auto-deny-mod", reason)
    } else if findings.is_empty() && manual_upload(db, id)? {
        let verified: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM audit WHERE action='approve-mod' AND target=?1)",
            [id],
            |r| r.get(0),
        )?;
        (
            if verified {
                "manual-approval-retained"
            } else {
                "scan-needs-review"
            },
            if verified {
                "Previously verified manual upload; completed rescan has no findings".to_owned()
            } else {
                "Manually uploaded mods require staff approval even when the scan has no findings"
                    .to_owned()
            },
        )
    } else if findings.is_empty() {
        (
            "auto-approve-mod",
            "Completed scan with no findings".to_owned(),
        )
    } else {
        (
            "scan-needs-review",
            format!("{} findings require review", findings.len()),
        )
    };
    db.execute("INSERT INTO mod_reviews VALUES(?1,?2) ON CONFLICT(mod_id) DO UPDATE SET approved=excluded.approved",params![id,matches!(action,"auto-approve-mod"|"manual-approval-retained")])?;
    if action == "scan-needs-review" {
        db.execute(
            "UPDATE mod_submissions SET status='pending',reason=?1,resolved=NULL WHERE id=?2",
            params![reason, id],
        )?;
    }
    db.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,?2,?3,?4)",
        params![
            actor,
            action,
            json!({"mod":id,"name":name,"reason":reason,"automatic":true}).to_string(),
            now()
        ],
    )?;
    if action == "auto-approve-mod" {
        notifications::accepted(db, id)?;
    }
    Ok(())
}
async fn queue(app: Shared, id: String, force: bool) -> ApiResult<()> {
    let root = std::env::var("CANNA_REVIEW_JOBS")
        .map_err(|_| bad("Source analysis worker is not configured"))?;
    if !app.files.join(format!("{id}.zip")).is_file() {
        provider_cache::ensure(&app, &id).await?;
    }
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
        db.execute("INSERT INTO audit(actor,action,target,created) SELECT user_id,'scan-started',?1,?2 FROM mods WHERE id=?3",params![json!({"mod":id,"automatic":true}).to_string(),now(),id])?;
        hash
    };
    let job = PathBuf::from(root).join(Uuid::new_v4().to_string());
    tokio::spawn(async move {
        if run(app.clone(), id.clone(), hash, job.clone())
            .await
            .is_err()
        {
            let _=app.db.lock().unwrap().execute("UPDATE mod_scans SET status='failed',report=?1 WHERE mod_id=?2",params![json!({"status":"failed","error":"Analysis unavailable, interrupted or over limits. Run again; it has not passed inspection.","files":[],"findings":[]}).to_string(),id]);
            let _=app.db.lock().unwrap().execute("INSERT INTO audit(actor,action,target,created) SELECT user_id,'scan-failed',?1,?2 FROM mods WHERE id=?3",params![json!({"mod":id,"reason":"Analysis unavailable, interrupted or over limits","automatic":true}).to_string(),now(),id]);
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
    #[test]
    fn corrected_scanner_requeues_only_automatic_uncertain_denials() {
        let (_dir, app) = fixture();
        account(&app, "scanner-submit", false);
        let mut db = app.db.lock().unwrap();
        for (id, reason, evidence) in [
            (
                "uncertain",
                "Rejected by scan policy: Generic",
                "(Heur)Packer: Generic",
            ),
            ("confirmed", "Rejected by scan policy: UPX", "Packer: UPX"),
            ("staff", "Staff denied this upload", "(Heur)Packer: Generic"),
        ] {
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1686940,?1,'1','','hash',1)",
                [id],
            )
            .unwrap();
            db.execute("INSERT INTO mod_reviews VALUES(?1,0)", [id])
                .unwrap();
            db.execute("INSERT INTO mod_submissions(id,user_id,name,version,status,reason,created,resolved) VALUES(?1,1,?1,'1','denied',?2,0,1) ON CONFLICT(id) DO UPDATE SET status='denied',reason=excluded.reason,resolved=1",params![id,reason]).unwrap();
            let report = json!({"version":"canna-static-4","findings":[{"rule":"packer-signature","evidence":evidence}]});
            db.execute(
                "INSERT INTO mod_scans VALUES(?1,'hash','rejected',?2,0)",
                params![id, report.to_string()],
            )
            .unwrap();
        }
        assert_eq!(requeue_uncertain_denials(&mut db).unwrap(), 1);
        assert_eq!(requeue_uncertain_denials(&mut db).unwrap(), 0);
        assert_eq!(
            db.query_row("SELECT count(*) FROM mod_scans", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row(
                "SELECT status FROM mod_submissions WHERE id='uncertain'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
        assert!(security::approved(&db, "uncertain").is_err());
    }
    #[test]
    fn scan_policy_auto_approves_reviews_or_quarantines() {
        let (_dir, app) = fixture();
        account(&app, "submitter", false);
        let db = app.db.lock().unwrap();
        for (id, findings, expected) in [
            ("clean", json!([]), "accepted"),
            (
                "review",
                json!([{"rule":"packing-review","evidence":"(Heur)Packer: Generic"}]),
                "pending",
            ),
            (
                "packed",
                json!([{"rule":"packer-signature","title":"UPX","evidence":"UPX signature","file":"a.dll"}]),
                "denied",
            ),
            ("malware", json!([{"rule":"signature"}]), "denied"),
        ] {
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1686940,?1,'1','','hash',1)",
                [id],
            )
            .unwrap();
            if id == "clean" {
                db.execute("INSERT INTO mod_details VALUES(?1,'modrinth:fixture:1','{\"provider\":\"modrinth\"}')",[id]).unwrap();
            }
            db.execute(
                "INSERT INTO mod_scans VALUES(?1,'hash','complete','{}',0)",
                [id],
            )
            .unwrap();
            apply_policy(&db, id, &json!({"findings":findings})).unwrap();
            let status: String = db
                .query_row(
                    "SELECT status FROM mod_submissions WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status, expected);
            let approved: bool = db
                .query_row(
                    "SELECT approved FROM mod_reviews WHERE mod_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(approved, expected == "accepted");
            let status: String = db
                .query_row("SELECT status FROM mod_scans WHERE mod_id=?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(
                status,
                if expected == "denied" {
                    "rejected"
                } else {
                    "complete"
                }
            );
        }
        assert_eq!(db.query_row("SELECT COUNT(*) FROM audit WHERE action IN ('auto-approve-mod','auto-deny-mod','scan-needs-review')",[],|r|r.get::<_,i64>(0)).unwrap(),4);
    }
    #[test]
    fn interrupted_jobs_retry_without_erasing_completed_reviews() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE mods(id TEXT PRIMARY KEY); INSERT INTO mods VALUES('pending'),('reviewed');")
            .unwrap();
        initialize(&db).unwrap();
        db.execute_batch("INSERT INTO mod_scans VALUES('pending','hash','queued','{}',0); INSERT INTO mod_scans VALUES('reviewed','hash','complete','{}',0);").unwrap();
        initialize(&db).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM mod_scans WHERE mod_id='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT status FROM mod_scans WHERE mod_id='reviewed'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "complete"
        );
    }
    #[tokio::test]
    async fn even_owner_uploads_require_manual_approval_after_clean_scan() {
        use tower::ServiceExt;
        let (_dir, app) = fixture();
        let owner = account(&app, "upload-owner", true);
        let bytes = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/v1/mods?app_id=550&name=Manual&version=1&provider=modrinth")
            .header("authorization", format!("Bearer {owner}"))
            .body(Body::from(bytes.as_slice()))
            .unwrap();
        let item = value(router(app.clone()).oneshot(request).await.unwrap()).await;
        assert_eq!(item["review_status"], "pending");
        let id = item["id"].as_str().unwrap();
        {
            let db = app.db.lock().unwrap();
            assert!(manual_upload(&db, id).unwrap());
            db.execute(
                "INSERT INTO mod_scans VALUES(?1,?2,'complete','{\"findings\":[]}',0)",
                params![id, item["sha256"].as_str().unwrap()],
            )
            .unwrap();
            apply_policy(&db, id, &json!({"findings":[]})).unwrap();
            assert!(security::approved(&db, id).is_err());
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/mods/{id}/approve"),
                json!({}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        {
            let db = app.db.lock().unwrap();
            apply_policy(&db, id, &json!({"findings":[]})).unwrap();
            assert!(security::approved(&db, id).is_ok());
        }
    }
    #[tokio::test]
    async fn legacy_auto_approved_uploads_return_to_review_but_verified_uploads_remain() {
        let (_dir, app) = fixture();
        account(&app, "migration-owner", true);
        let manual = external::store(
            &app,
            1,
            550,
            "Manual",
            "1",
            "",
            "uploaded:fixture",
            &json!({}),
            b"PK\x03\x04manual",
        )
        .await
        .unwrap();
        let verified = external::store(
            &app,
            1,
            550,
            "Verified",
            "1",
            "",
            "uploaded:verified",
            &json!({"provider":"uploaded"}),
            b"PK\x03\x04verified",
        )
        .await
        .unwrap();
        let imported = external::store(
            &app,
            1,
            550,
            "Imported",
            "1",
            "",
            "modrinth:external:1",
            &json!({"provider":"modrinth"}),
            b"PK\x03\x04external",
        )
        .await
        .unwrap();
        let db = app.db.lock().unwrap();
        db.execute(
            "INSERT INTO audit(actor,action,target,created) VALUES(1,'approve-mod',?1,0)",
            [&verified],
        )
        .unwrap();
        db.execute(
            "DELETE FROM notification_meta WHERE key='manual-upload-review-v1'",
            [],
        )
        .unwrap();
        enforce_manual_uploads(&db).unwrap();
        assert!(security::approved(&db, &manual).is_err());
        assert!(security::approved(&db, &verified).is_ok());
        assert!(security::approved(&db, &imported).is_ok());
        enforce_manual_uploads(&db).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM audit WHERE action='manual-upload-review-required'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    #[cfg(unix)]
    #[test]
    fn job_permissions_preserve_worker_group_inheritance() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        mode(root.path(), 0o2770).unwrap();
        let job = root.path().join("job");
        std::fs::create_dir(&job).unwrap();
        mode(&job, 0o770).unwrap();
        assert_eq!(
            std::fs::metadata(&job).unwrap().permissions().mode() & 0o7777,
            0o770
        );
        let archive = job.join("input.zip");
        std::fs::write(&archive, b"fixture").unwrap();
        worker_file(&archive, &job).unwrap();
        assert_eq!(
            std::fs::metadata(&archive).unwrap().gid(),
            std::fs::metadata(root.path()).unwrap().gid()
        );
        assert_eq!(
            std::fs::metadata(&archive).unwrap().permissions().mode() & 0o777,
            0o660
        );
    }
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
