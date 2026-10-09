use super::*;
use tokio::io::AsyncWriteExt;
const REPORT_LIMIT: u64 = 16 * 1024 * 1024;
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS mod_scans(mod_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE,hash TEXT NOT NULL,status TEXT NOT NULL,report TEXT NOT NULL,started INTEGER NOT NULL);
 CREATE TABLE IF NOT EXISTS mod_scan_decisions(mod_id TEXT NOT NULL REFERENCES mods(id) ON DELETE CASCADE,hash TEXT NOT NULL,finding_id TEXT NOT NULL,decision TEXT NOT NULL,PRIMARY KEY(mod_id,hash,finding_id));
 INSERT OR IGNORE INTO mod_scan_decisions SELECT s.mod_id,s.hash,json_extract(f.value,'$.id'),f.value FROM mod_scans s JOIN json_each(s.report,'$.findings') f WHERE json_type(f.value,'$.id')='text' AND json_type(f.value,'$.reviewer')='integer' AND json_type(f.value,'$.reviewed')='integer';
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
pub fn download_state(db: &Connection, id: &str) -> ApiResult<Value> {
    let name: String = db
        .query_row("SELECT name FROM mods WHERE id=?1", [id], |r| r.get(0))
        .optional()?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "Mod no longer available"))?;
    if security::approved(db, id).is_ok() {
        return Ok(json!({"id":id,"name":name,"state":"ready","message":"Ready to download"}));
    }
    let mut todo = vec![id.to_owned()];
    let mut seen = std::collections::BTreeSet::new();
    let (mut waiting, mut review, mut failed, mut denied) = (0, 0, 0, 0);
    while let Some(next) = todo.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        if seen.len() > 128 {
            return Err(bad("Dependency graph exceeds limits"));
        }
        let scan: Option<(String, String)> = db
            .query_row(
                "SELECT status,report FROM mod_scans WHERE mod_id=?1",
                [&next],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match scan {
            None => waiting += 1,
            Some((state, report)) => match state.as_str() {
                "queued" => waiting += 1,
                "failed" => failed += 1,
                "rejected" => denied += 1,
                "complete" => {
                    let data: Value = serde_json::from_str(&report).unwrap_or(Value::Null);
                    let allowed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM mods m WHERE m.id=?1 AND NOT EXISTS(SELECT 1 FROM mod_reviews r WHERE r.mod_id=m.id AND r.approved=0))",[&next],|r|r.get(0))?;
                    if !allowed
                        || data["findings"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .any(|f| f["accepted"] != true)
                    {
                        review += 1;
                    }
                }
                _ => waiting += 1,
            },
        }
        let details = external::details(db, &next)?;
        todo.extend(
            details["dependency_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    let (state, message) = if denied > 0 {
        ("denied", "A mod or dependency was denied by review policy.")
    } else if failed > 0 {
        (
            "failed",
            "A mod or dependency scan failed. Staff can retry it.",
        )
    } else if review > 0 {
        (
            "needs_review",
            "Analysis found items requiring staff review, or manual approval is required.",
        )
    } else {
        (
            "waiting",
            "Waiting for mod or dependency analysis. This download will continue when approved.",
        )
    };
    Ok(
        json!({"id":id,"name":name,"state":state,"message":message,"waiting":waiting,"needs_review":review,"failed":failed}),
    )
}
pub async fn status(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    Ok(axum::Json(download_state(&app.db.lock().unwrap(), &id)?))
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
    let scan: Option<(String, String, String, String)> = db
        .query_row(
            "SELECT s.status,s.report,s.hash,m.sha256 FROM mod_scans s JOIN mods m ON m.id=s.mod_id WHERE s.mod_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((status, text, scan_hash, archive_hash)) = scan else {
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
    if scan_hash != archive_hash {
        return Err(bad(
            "Source analysis does not match this archive; run analysis again",
        ));
    }
    let report: Value = serde_json::from_str(&text).map_err(|_| bad("Source report is invalid"))?;
    let findings = report["findings"].as_array().ok_or(bad(
        "Source report findings are invalid; run analysis again",
    ))?;
    if findings
        .iter()
        .any(|f| !f.is_object() || f["accepted"] != true)
    {
        return Err(bad(
            "Resolve review findings in the review workspace before approval",
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
    tokio::fs::create_dir(&job).await.map_err(|_| bad("Could not create the isolated analysis job; staff should check worker spool permissions"))?;
    // The spool supplies the job's worker group. Explicit file group assignment
    // avoids setgid chmod, which systemd RestrictSUIDSGID deliberately rejects.
    mode(&job, 0o770).map_err(|_| bad("Could not set isolated analysis job permissions"))?;
    let file = tokio::fs::File::open(app.files.join(format!("{id}.zip")))
        .await
        .map_err(|_| bad("The stored archive is unavailable for analysis"))?;
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
            let mut report: Value =
                serde_json::from_str(&text).map_err(|_| bad("Invalid worker report"))?;
            if report["status"] != "complete" {
                return Err(bad(
                    "The analysis worker failed; staff should inspect its logs before retrying",
                ));
            }
            let mut db = app.db.lock().unwrap();
            let tx = db.transaction()?;
            let stored: Option<String> = tx
                .query_row("SELECT sha256 FROM mods WHERE id=?1", [&id], |r| r.get(0))
                .optional()?;
            if stored.as_deref() != Some(&hash) {
                return Err(bad("Mod changed or was removed during analysis"));
            }
            let previous: Option<(String, String)> = tx
                .query_row(
                    "SELECT hash,report FROM mod_scans WHERE mod_id=?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let mut decisions: Vec<Value> = tx
                .prepare("SELECT decision FROM mod_scan_decisions WHERE mod_id=?1 AND hash=?2")?
                .query_map(params![id, hash], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok().and_then(|r| serde_json::from_str(&r).ok()))
                .collect();
            if let Some((previous_hash, previous)) = previous
                && previous_hash == hash
            {
                let previous: Value = serde_json::from_str(&previous).unwrap_or(Value::Null);
                decisions.extend(
                    previous["findings"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|f| !decisions.iter().any(|d| d["id"] == f["id"]))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            }
            preserve_decisions(&mut report, &json!({"findings":decisions}), true);
            tx.execute(
                "UPDATE mod_scans SET status='complete',report=?1 WHERE mod_id=?2 AND hash=?3",
                params![report.to_string(), id, hash],
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
fn preserve_decisions(report: &mut Value, previous: &Value, same_hash: bool) {
    for finding in report["findings"].as_array_mut().into_iter().flatten() {
        if !finding.is_object() {
            continue;
        }
        for field in ["accepted", "reason", "reviewer", "reviewed"] {
            finding.as_object_mut().unwrap().remove(field);
        }
        if !same_hash {
            continue;
        }
        if let Some(old) = previous["findings"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|old| {
                old["reviewer"].is_number()
                    && old["reviewed"].is_number()
                    && [
                        "id",
                        "rule",
                        "file",
                        "line",
                        "evidence",
                        "severity",
                        "title",
                        "context",
                        "locations",
                        "trace",
                        "operation",
                        "path_classification",
                        "binary",
                    ]
                    .into_iter()
                    .all(|field| old[field] == finding[field])
            })
        {
            for field in ["accepted", "reason", "reviewer", "reviewed"] {
                finding[field] = old[field].clone();
            }
        }
    }
}
fn apply_policy(db: &Connection, id: &str, report: &Value) -> ApiResult<()> {
    let (actor, name): (i64, String) =
        db.query_row("SELECT user_id,name FROM mods WHERE id=?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let findings: Vec<_> = report["findings"]
        .as_array()
        .ok_or(bad("Invalid findings"))?
        .iter()
        .filter(|f| f["accepted"] != true)
        .collect();
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
        db.execute("INSERT INTO mod_scans VALUES(?1,?2,'queued','{}',?3) ON CONFLICT(mod_id) DO UPDATE SET report=CASE WHEN mod_scans.hash=excluded.hash THEN mod_scans.report ELSE '{}' END,hash=excluded.hash,status='queued',started=excluded.started",params![id,hash,now()])?;
        db.execute("INSERT INTO audit(actor,action,target,created) SELECT user_id,'scan-started',?1,?2 FROM mods WHERE id=?3",params![json!({"mod":id,"automatic":true}).to_string(),now(),id])?;
        hash
    };
    app.live.hint("library");
    let job = PathBuf::from(root).join(Uuid::new_v4().to_string());
    tokio::spawn(async move {
        if let Err(error) = run(app.clone(), id.clone(), hash.clone(), job.clone()).await {
            // Static API messages only; never return submitted worker text or paths.
            eprintln!("Analysis job {job:?} failed for mod {id}: {}", error.1);
            let _ = record_failure(&app.db.lock().unwrap(), &id, &hash, error.1);
            let _=app.db.lock().unwrap().execute("INSERT INTO audit(actor,action,target,created) SELECT user_id,'scan-failed',?1,?2 FROM mods WHERE id=?3",params![json!({"mod":id,"reason":"Analysis unavailable, interrupted or over limits","automatic":true}).to_string(),now(),id]);
        }
        app.live.hint("library");
        let _ = tokio::fs::remove_dir_all(job).await;
        app.review_wake.notify_one();
    });
    Ok(())
}
fn record_failure(db: &Connection, id: &str, hash: &str, error: &str) -> rusqlite::Result<()> {
    let previous: Option<String> = db
        .query_row(
            "SELECT report FROM mod_scans WHERE mod_id=?1 AND hash=?2 AND status='queued'",
            params![id, hash],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        let mut report: Value = serde_json::from_str(&previous)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"files":[],"findings":[]}));
        report["status"] = json!("failed");
        report["error"] = json!(error);
        report["retained_previous_report"] = json!(
            report["files"]
                .as_array()
                .is_some_and(|files| !files.is_empty())
        );
        db.execute("UPDATE mod_scans SET status='failed',report=?1 WHERE mod_id=?2 AND hash=?3 AND status='queued'",params![report.to_string(),id,hash])?;
    }
    Ok(())
}
fn next_waiting_scan(db: &Connection) -> rusqlite::Result<Option<String>> {
    // Scan shared dependencies first, then FIFO so newer imports cannot starve older ones.
    db.query_row("SELECT m.id FROM mods m WHERE NOT EXISTS(SELECT 1 FROM mod_scans s WHERE s.mod_id=m.id) ORDER BY EXISTS(SELECT 1 FROM mod_details d JOIN json_each(d.data,'$.dependency_ids') dep WHERE dep.value=m.id) DESC,m.rowid ASC LIMIT 1",[],|r|r.get(0)).optional()
}
pub fn start(app: Shared) {
    if std::env::var_os("CANNA_REVIEW_JOBS").is_none() {
        return;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = interval.tick() => {},
                _ = app.review_wake.notified() => {},
            }
            // Fill the bounded spool immediately; finishing a scan wakes us again.
            for _ in 0..2 {
                let id = {
                    let db = app.db.lock().unwrap();
                    next_waiting_scan(&db).unwrap_or(None)
                };
                let Some(id) = id else { break };
                if let Err(error) = queue(app.clone(), id.clone(), false).await {
                    if error.0 == StatusCode::TOO_MANY_REQUESTS {
                        break;
                    }
                    // An unavailable archive must not stall every later import.
                    let db = app.db.lock().unwrap();
                    let _ = mark_unavailable(&db, &id);
                    app.live.hint("library");
                }
            }
        }
    });
}
fn mark_unavailable(db: &Connection, id: &str) -> rusqlite::Result<()> {
    let report = json!({"status":"failed","error":"Could not prepare the archive for analysis. Retry analysis after checking its source and server logs.","files":[],"findings":[]});
    if db.execute(
        "INSERT OR IGNORE INTO mod_scans SELECT id,sha256,'failed',?1,?2 FROM mods WHERE id=?3",
        params![report.to_string(), now(), id],
    )? > 0
    {
        db.execute("INSERT INTO audit(actor,action,target,created) SELECT user_id,'scan-prepare-failed',?1,?2 FROM mods WHERE id=?3",params![json!({"mod":id,"automatic":true,"reason":"Archive unavailable for analysis"}).to_string(),now(),id])?;
    }
    Ok(())
}
pub async fn report(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    staff(&app, &headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    let db = app.db.lock().unwrap();
    let (name, app_id): (String, i64) = db
        .query_row("SELECT name,app_id FROM mods WHERE id=?1", [&id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "Mod not found"))?;
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT status,report,hash FROM mod_scans WHERE mod_id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    drop(db);
    let mut result = if let Some((status, text, hash)) = row {
        if text.len() as u64 > REPORT_LIMIT {
            return Err(bad("Source analysis report exceeds limits"));
        }
        let mut v: Value = serde_json::from_str(&text).map_err(|_| bad("Invalid report"))?;
        if !v.is_object() {
            return Err(bad("Invalid report"));
        }
        v["status"] = json!(status);
        v["sha256"] = json!(hash);
        v
    } else {
        json!({"status":"pending","files":[],"findings":[]})
    };
    result["mod_name"] = json!(name);
    result["app_id"] = json!(app_id);
    result["game_name"] = json!(crate::admin_tools::review_game_name(app_id));
    // Derived for legacy reports too. This value is never persisted or used by
    // the approval policy, and no supplied worker overview is trusted.
    result["review_overview"] = crate::review_guide::overview(&result);
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
#[derive(Deserialize)]
pub struct FindingDecision {
    id: String,
    accepted: bool,
    reason: String,
}
#[derive(Deserialize)]
pub struct BatchDecision {
    sha256: String,
    findings: Vec<FindingDecision>,
}
fn record_decisions(
    app: &App,
    actor: i64,
    id: &str,
    hash: &str,
    choices: &[FindingDecision],
) -> ApiResult<()> {
    Uuid::parse_str(id).map_err(|_| bad("Invalid mod ID"))?;
    if choices.is_empty() || choices.len() > 1500 {
        return Err(bad("Review between 1 and 1500 findings at a time"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for choice in choices {
        if choice.reason.trim().len() < 5 || choice.reason.len() > 500 {
            return Err(bad("Record a review reason between 5 and 500 bytes"));
        }
        if !seen.insert(&choice.id) {
            return Err(bad("Duplicate finding decision"));
        }
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let (text, current_hash): (String, String) = tx
        .query_row(
            "SELECT report,hash FROM mod_scans WHERE mod_id=?1 AND status='complete'",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(bad("Completed analysis not found"))?;
    if current_hash != hash {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Mod analysis changed; reload",
        ));
    }
    let mut report: Value = serde_json::from_str(&text).map_err(|_| bad("Invalid report"))?;
    let findings = report["findings"]
        .as_array_mut()
        .ok_or(bad("Invalid findings"))?;
    let reviewed = now();
    for choice in choices {
        let finding = findings
            .iter_mut()
            .find(|f| f["id"] == choice.id)
            .ok_or(bad("Finding not found"))?;
        finding["accepted"] = json!(choice.accepted);
        finding["reason"] = json!(choice.reason.trim());
        finding["reviewer"] = json!(actor);
        finding["reviewed"] = json!(reviewed);
        tx.execute("INSERT INTO mod_scan_decisions VALUES(?1,?2,?3,?4) ON CONFLICT(mod_id,hash,finding_id) DO UPDATE SET decision=excluded.decision",params![id,hash,choice.id,finding.to_string()])?;
        tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'scan-finding-decision',?2,?3)",params![actor,json!({"mod":id,"finding":choice.id,"accepted":choice.accepted,"reason":choice.reason.trim(),"sha256":hash}).to_string(),reviewed])?;
    }
    tx.execute(
        "UPDATE mod_scans SET report=?1 WHERE mod_id=?2",
        params![report.to_string(), id],
    )?;
    let unresolved = report["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|finding| finding["accepted"] != true)
        .count();
    let reason = if unresolved == 0 {
        "Findings resolved; awaiting staff approval".to_owned()
    } else {
        format!("{unresolved} findings require review")
    };
    tx.execute(
        "UPDATE mod_submissions SET reason=?1 WHERE id=?2 AND status='pending'",
        params![reason, id],
    )?;
    if choices.iter().any(|choice| !choice.accepted) {
        tx.execute("UPDATE mod_reviews SET approved=0 WHERE mod_id=?1", [id])?;
    }
    tx.commit()?;
    app.live.hint("library");
    Ok(())
}
pub async fn decision(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((id, fid)): Path<(String, String)>,
    axum::Json(input): axum::Json<Decision>,
) -> ApiResult<axum::Json<Value>> {
    let actor = staff(&app, &headers)?;
    record_decisions(
        &app,
        actor,
        &id,
        &input.sha256,
        &[FindingDecision {
            id: fid,
            accepted: input.accepted,
            reason: input.reason,
        }],
    )?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn decisions(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(input): axum::Json<BatchDecision>,
) -> ApiResult<axum::Json<Value>> {
    let actor = staff(&app, &headers)?;
    record_decisions(&app, actor, &id, &input.sha256, &input.findings)?;
    Ok(axum::Json(
        json!({"ok":true,"reviewed":input.findings.len()}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_rescan_retains_evidence_and_does_not_overwrite_new_hash() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE mods(id TEXT PRIMARY KEY); INSERT INTO mods VALUES('fixture');",
        )
        .unwrap();
        initialize(&db).unwrap();
        let previous = json!({"files":[{"name":"source.cs","text":"File.WriteAllBytes(path,bytes);"}],"findings":[{"id":"old","accepted":false}],"status":"complete"});
        db.execute(
            "INSERT INTO mod_scans VALUES('fixture','hash','queued',?1,0)",
            [previous.to_string()],
        )
        .unwrap();
        record_failure(&db, "fixture", "hash", "Worker failed").unwrap();
        let (status, raw): (String, String) = db
            .query_row("SELECT status,report FROM mod_scans", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        let report: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(status, "failed");
        assert_eq!(report["findings"], previous["findings"]);
        assert_eq!(report["files"], previous["files"]);
        assert_eq!(report["retained_previous_report"], true);
        db.execute("UPDATE mod_scans SET hash='new-hash',status='queued'", [])
            .unwrap();
        record_failure(&db, "fixture", "hash", "Late failure").unwrap();
        assert_eq!(
            db.query_row("SELECT status FROM mod_scans", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "queued"
        );
    }
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn report_overview_is_staff_only_derived_and_does_not_persist_or_approve() {
        let (_dir, app) = fixture();
        let member = account(&app, "guide-member", false);
        let owner = account(&app, "guide-owner", true);
        let id = Uuid::new_v4().to_string();
        let stored = json!({"status":"complete","sha256":"worker-supplied-hash","coverage_complete":false,
            "review_overview":{"approved":true,"suggestions":[]},"inventory":[{"name":"archive/plugin.dll","size":40}],
            "files":[{"name":"decompiled/plugin/Plugin.cs","kind":"decompiled","text":"class Plugin { void Awake() { Process.Start(\"helper\"); } }"}],
            "findings":[{"id":"process","rule":"commands","file":"decompiled/plugin/Plugin.cs","line":1}]}).to_string();
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1686940,'Guide fixture','1','','actual-hash',100)",
                [&id],
            )
            .unwrap();
            db.execute("INSERT INTO mod_reviews VALUES(?1,0)", [&id])
                .unwrap();
            db.execute(
                "INSERT INTO mod_scans VALUES(?1,'actual-hash','complete',?2,0)",
                params![id, stored],
            )
            .unwrap();
        }
        let url = format!("/api/v1/mods/{id}/analysis");
        assert_eq!(
            call(app.clone(), "GET", &url, Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(app.clone(), "GET", &url, Value::Null, Some(&member))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let result = value(call(app.clone(), "GET", &url, Value::Null, Some(&owner)).await).await;
        assert_eq!(result["sha256"], "actual-hash");
        assert_eq!(result["review_overview"]["version"], "canna-review-guide-1");
        assert!(result["review_overview"].get("approved").is_none());
        assert_eq!(result["review_overview"]["coverage"]["state"], "incomplete");
        assert_eq!(result["findings"][0]["accepted"], Value::Null);
        let db = app.db.lock().unwrap();
        let persisted: String = db
            .query_row(
                "SELECT report FROM mod_scans WHERE mod_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(persisted, stored);
        let approved: bool = db
            .query_row(
                "SELECT approved FROM mod_reviews WHERE mod_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!approved);
        assert!(require_review(&db, &id).is_err());
        let decisions: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM mod_scan_decisions WHERE mod_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decisions, 0);
    }

    #[tokio::test]
    async fn report_overview_handles_pending_and_rejects_invalid_or_oversized_stored_reports() {
        let (_dir, app) = fixture();
        let owner = account(&app, "guide-invalid-owner", true);
        let id = Uuid::new_v4().to_string();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO mods VALUES(?1,1,1686940,'Guide pending','1','','hash',100)",
                [&id],
            )
            .unwrap();
        let url = format!("/api/v1/mods/{id}/analysis");
        let pending = value(call(app.clone(), "GET", &url, Value::Null, Some(&owner)).await).await;
        assert_eq!(pending["review_overview"]["analysis_status"], "pending");
        assert_eq!(
            pending["review_overview"]["suggestions"][0]["id"],
            "analysis-status"
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO mod_scans VALUES(?1,'hash','complete','null',0)",
                [&id],
            )
            .unwrap();
        for raw in [
            "null".to_string(),
            "[]".to_string(),
            "\"string\"".to_string(),
            "{malformed".to_string(),
            format!("{{\"note\":\"{}\"}}", "x".repeat(REPORT_LIMIT as usize)),
        ] {
            app.db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE mod_scans SET report=?1 WHERE mod_id=?2",
                    params![raw, id],
                )
                .unwrap();
            assert_eq!(
                call(app.clone(), "GET", &url, Value::Null, Some(&owner))
                    .await
                    .status(),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[tokio::test]
    async fn approval_rejects_stale_analysis_without_changing_review_or_audit() {
        let (_dir, app) = fixture();
        let owner = account(&app, "stale-analysis-owner", true);
        let id = Uuid::new_v4().to_string();
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1686940,'Stale analysis','1','','current-hash',1)",
                [&id],
            )
            .unwrap();
            db.execute("INSERT INTO mod_reviews VALUES(?1,0)", [&id])
                .unwrap();
            db.execute(
                "INSERT INTO mod_scans VALUES(?1,'previous-hash','complete','{\"findings\":[]}',0)",
                [&id],
            )
            .unwrap();
        }
        let path = format!("/api/v1/mods/{id}/approve");
        assert_eq!(
            call(app.clone(), "POST", &path, json!({}), Some(&owner))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        {
            let db = app.db.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT approved FROM mod_reviews WHERE mod_id=?1",
                    [&id],
                    |r| { r.get::<_, i64>(0) }
                )
                .unwrap(),
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM audit WHERE action='approve-mod' AND target=?1",
                    [&id],
                    |r| { r.get::<_, i64>(0) }
                )
                .unwrap(),
                0
            );
            db.execute(
                "UPDATE mod_scans SET hash='current-hash' WHERE mod_id=?1",
                [&id],
            )
            .unwrap();
        }
        assert_eq!(
            call(app.clone(), "POST", &path, json!({}), Some(&owner))
                .await
                .status(),
            StatusCode::OK
        );
        assert!(security::approved(&app.db.lock().unwrap(), &id).is_ok());
    }
    #[tokio::test]
    async fn approval_rejects_malformed_complete_reports() {
        let (_dir, app) = fixture();
        let owner = account(&app, "malformed-analysis-owner", true);
        let id = Uuid::new_v4().to_string();
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1686940,'Malformed analysis','1','','current-hash',1)",
                [&id],
            )
            .unwrap();
            db.execute("INSERT INTO mod_reviews VALUES(?1,0)", [&id])
                .unwrap();
            db.execute(
                "INSERT INTO mod_scans VALUES(?1,'current-hash','complete','{}',0)",
                [&id],
            )
            .unwrap();
        }
        let path = format!("/api/v1/mods/{id}/approve");
        for report in [
            "{}",
            "{\"findings\":null}",
            "{\"findings\":{}}",
            "{\"findings\":\"accepted\"}",
            "{\"findings\":[true]}",
            "{\"findings\":[{\"accepted\":\"true\"}]}",
            "invalid JSON",
        ] {
            app.db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE mod_scans SET report=?1 WHERE mod_id=?2",
                    params![report, id],
                )
                .unwrap();
            assert_eq!(
                call(app.clone(), "POST", &path, json!({}), Some(&owner))
                    .await
                    .status(),
                StatusCode::BAD_REQUEST,
                "Report {report} must not be treated as a clean completed scan"
            );
            let db = app.db.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT approved FROM mod_reviews WHERE mod_id=?1",
                    [&id],
                    |r| { r.get::<_, i64>(0) }
                )
                .unwrap(),
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM audit WHERE action='approve-mod' AND target=?1",
                    [&id],
                    |r| { r.get::<_, i64>(0) }
                )
                .unwrap(),
                0
            );
        }
    }
    #[tokio::test]
    async fn batch_reviews_require_staff_exact_hash_and_atomic_valid_decisions() {
        let (_dir, app) = fixture();
        let owner = account(&app, "batch-owner", true);
        let member = account(&app, "batch-member", false);
        let id = external::store(
            &app,
            1,
            1557740,
            "Batch fixture",
            "1",
            "",
            "batch-fixture",
            &json!({}),
            b"PK\x05\x06test",
        )
        .await
        .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO mod_scans VALUES(?1,'exact-hash','complete',?2,0)",
                params![id, json!({"findings":[{"id":"a"},{"id":"b"}]}).to_string()],
            )
            .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_reviews SET approved=0 WHERE mod_id=?1", [&id])
            .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_submissions SET status='pending',reason='2 findings require review',resolved=NULL WHERE id=?1",
                [&id],
            )
            .unwrap();
        let path = format!("/api/v1/mods/{id}/analysis-decisions");
        let choices = json!([{"id":"a","accepted":true,"reason":"Expected loader behavior"},{"id":"b","accepted":true,"reason":"Reviewed coverage limitation"}]);
        let body = json!({"sha256":"exact-hash","findings":choices});
        assert_eq!(
            call(app.clone(), "POST", &path, body.clone(), None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(app.clone(), "POST", &path, body.clone(), Some(&member))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &path,
                json!({"sha256":"different-hash","findings":choices}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        for invalid in [
            json!([]),
            json!([{"id":"a","accepted":true,"reason":"short"},{"id":"missing","accepted":true,"reason":"Unknown finding"}]),
            json!([{"id":"a","accepted":true,"reason":"valid reason"},{"id":"b","accepted":true,"reason":"x"}]),
            json!([{"id":"a","accepted":true,"reason":"valid reason"},{"id":"a","accepted":false,"reason":"duplicate reason"}]),
        ] {
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    &path,
                    json!({"sha256":"exact-hash","findings":invalid}),
                    Some(&owner)
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST
            );
        }
        {
            let db = app.db.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM mod_scan_decisions WHERE mod_id=?1",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM audit WHERE action='scan-finding-decision'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
        let result = value(call(app.clone(), "POST", &path, body, Some(&owner)).await).await;
        assert_eq!(result["reviewed"], 2);
        {
            let db = app.db.lock().unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM mod_scan_decisions WHERE mod_id=?1 AND hash='exact-hash'",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                2
            );
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM audit WHERE action='scan-finding-decision' AND actor=1",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                2
            );
            assert_eq!(
                db.query_row(
                    "SELECT approved FROM mod_reviews WHERE mod_id=?1",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT reason FROM mod_submissions WHERE id=?1",
                    [&id],
                    |r| { r.get::<_, String>(0) }
                )
                .unwrap(),
                "Findings resolved; awaiting staff approval"
            );
            let text: String = db
                .query_row("SELECT report FROM mod_scans WHERE mod_id=?1", [&id], |r| {
                    r.get(0)
                })
                .unwrap();
            let report: Value = serde_json::from_str(&text).unwrap();
            assert!(
                report["findings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|f| f["accepted"] == true
                        && f["reviewer"] == 1
                        && f["reviewed"].is_number())
            );
            db.execute("UPDATE mod_reviews SET approved=1 WHERE mod_id=?1", [&id])
                .unwrap();
        }
        assert_eq!(call(app.clone(),"POST",&path,json!({"sha256":"exact-hash","findings":[{"id":"a","accepted":false,"reason":"Needs another review"}]}),Some(&owner)).await.status(),StatusCode::OK);
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT approved FROM mod_reviews WHERE mod_id=?1",
                    [&id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    #[test]
    fn rescan_preserves_only_identical_exact_hash_review_decisions() {
        let finding = json!({"id":"one","rule":"packing-review","file":"plugin.dll","line":null,"evidence":"Generic","severity":"high","title":"Heuristic"});
        let mut old = finding.clone();
        old["accepted"] = json!(true);
        old["reason"] = json!("Reviewed false positive");
        old["reviewer"] = json!(1);
        old["reviewed"] = json!(10);
        let previous = json!({"findings":[old]});
        let mut report = json!({"findings":[finding.clone()]});
        preserve_decisions(&mut report, &previous, true);
        assert_eq!(report["findings"][0]["accepted"], true);
        preserve_decisions(&mut report, &previous, false);
        assert!(report["findings"][0]["accepted"].is_null());
        report["findings"][0]["evidence"] = json!("Different detection");
        preserve_decisions(&mut report, &previous, true);
        assert!(report["findings"][0]["accepted"].is_null());
        for field in [
            "context",
            "locations",
            "trace",
            "operation",
            "path_classification",
            "binary",
        ] {
            let mut report = json!({"findings":[finding.clone()]});
            report["findings"][0][field] = json!(["New output path or related operation"]);
            preserve_decisions(&mut report, &previous, true);
            assert!(report["findings"][0]["accepted"].is_null());
        }
    }
    #[tokio::test]
    async fn interrupted_rescan_keeps_hash_bound_decisions_for_the_retry() {
        let (_dir, app) = fixture();
        account(&app, "decision-owner", true);
        let id = external::store(
            &app,
            1,
            1557740,
            "Decision fixture",
            "1",
            "",
            "decision-test",
            &json!({}),
            b"PK\x05\x06test",
        )
        .await
        .unwrap();
        let db = app.db.lock().unwrap();
        let report = json!({"findings":[{"id":"finding","accepted":true,"reviewer":1,"reviewed":10,"reason":"Exact false positive"}]});
        db.execute(
            "INSERT INTO mod_scans VALUES(?1,'unchanged-hash','queued',?2,0)",
            params![id, report.to_string()],
        )
        .unwrap();
        initialize(&db).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM mod_scans WHERE mod_id=?1",
                [&id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM mod_scan_decisions WHERE mod_id=?1 AND hash='unchanged-hash'",
                [&id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        initialize(&db).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM mod_scan_decisions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn member_download_status_distinguishes_analysis_review_failure_and_ready() {
        let (_dir, app) = fixture();
        let member = account(&app, "status-member", false);
        let id = external::store(
            &app,
            1,
            1557740,
            "Status fixture",
            "1",
            "",
            "status-test",
            &json!({"provider":"thunderstore"}),
            b"PK\x05\x06test",
        )
        .await
        .unwrap();
        let path = format!("/api/v1/mods/{id}/status");
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            value(call(app.clone(), "GET", &path, Value::Null, Some(&member)).await).await["state"],
            "waiting"
        );
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO mod_scans SELECT ?1,sha256,'complete',?2,0 FROM mods WHERE id=?1",
                params![
                    id,
                    json!({"findings":[{"evidence":"private code","accepted":false}]}).to_string()
                ],
            )
            .unwrap();
        }
        let status = value(call(app.clone(), "GET", &path, Value::Null, Some(&member)).await).await;
        assert_eq!(status["state"], "needs_review");
        assert!(!status.to_string().contains("private code"));
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE mod_scans SET status='failed' WHERE mod_id=?1",
                [&id],
            )
            .unwrap();
        }
        assert_eq!(
            value(call(app.clone(), "GET", &path, Value::Null, Some(&member)).await).await["state"],
            "failed"
        );
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE mod_scans SET status='complete',report='{\"findings\":[]}' WHERE mod_id=?1",
                [&id],
            )
            .unwrap();
            db.execute("UPDATE mod_reviews SET approved=1 WHERE mod_id=?1", [&id])
                .unwrap();
        }
        assert_eq!(
            value(call(app, "GET", &path, Value::Null, Some(&member)).await).await["state"],
            "ready"
        );
    }
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
    #[test]
    fn queue_scans_dependencies_first_then_oldest_imports() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE mods(id TEXT PRIMARY KEY); CREATE TABLE mod_details(mod_id TEXT,data TEXT); INSERT INTO mods VALUES('old-parent'),('shared-dependency'),('new-import'); INSERT INTO mod_details VALUES('old-parent','{\"dependency_ids\":[\"shared-dependency\"]}');").unwrap();
        initialize(&db).unwrap();
        assert_eq!(
            next_waiting_scan(&db).unwrap().as_deref(),
            Some("shared-dependency")
        );
        db.execute_batch(
            "INSERT INTO mod_scans VALUES('shared-dependency','hash','complete','{}',0);",
        )
        .unwrap();
        assert_eq!(
            next_waiting_scan(&db).unwrap().as_deref(),
            Some("old-parent")
        );
        db.execute_batch("INSERT INTO mod_scans VALUES('old-parent','hash','queued','{}',0);")
            .unwrap();
        assert_eq!(
            next_waiting_scan(&db).unwrap().as_deref(),
            Some("new-import")
        );
    }
    #[tokio::test]
    async fn unavailable_archive_does_not_starve_later_imports_or_erase_reviews() {
        let (_dir, app) = fixture();
        account(&app, "queue-owner", true);
        let first = external::store(
            &app,
            1,
            1557740,
            "Missing",
            "1",
            "",
            "queue-missing",
            &json!({}),
            b"PK\x05\x06missing",
        )
        .await
        .unwrap();
        let second = external::store(
            &app,
            1,
            1557740,
            "Next",
            "1",
            "",
            "queue-next",
            &json!({}),
            b"PK\x05\x06next",
        )
        .await
        .unwrap();
        let db = app.db.lock().unwrap();
        mark_unavailable(&db, &first).unwrap();
        mark_unavailable(&db, &first).unwrap();
        assert_eq!(next_waiting_scan(&db).unwrap(), Some(second.clone()));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM audit WHERE action='scan-prepare-failed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        db.execute(
            "INSERT INTO mod_scans VALUES(?1,'hash','complete','{}',0)",
            [&second],
        )
        .unwrap();
        mark_unavailable(&db, &second).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT status FROM mod_scans WHERE mod_id=?1",
                [second],
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
