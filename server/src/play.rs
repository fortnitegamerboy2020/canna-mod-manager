use super::*;
use crate::play_manifest::{Manifest, compare, redact};

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS play_rooms(id TEXT PRIMARY KEY,owner INTEGER NOT NULL REFERENCES users(id),code_hash TEXT NOT NULL,manifest TEXT NOT NULL,expires INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS play_members(room TEXT NOT NULL REFERENCES play_rooms(id) ON DELETE CASCADE,user INTEGER NOT NULL REFERENCES users(id),alias TEXT NOT NULL,manifest TEXT NOT NULL,seen INTEGER NOT NULL,PRIMARY KEY(room,user));
    CREATE TABLE IF NOT EXISTS play_reports(id TEXT PRIMARY KEY,actor INTEGER NOT NULL REFERENCES users(id),fingerprint TEXT NOT NULL,manifest TEXT NOT NULL,outcome TEXT NOT NULL,note TEXT NOT NULL,created INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS play_reports_fingerprint ON play_reports(fingerprint,created);
    CREATE TABLE IF NOT EXISTS play_channels(id TEXT PRIMARY KEY,owner INTEGER NOT NULL REFERENCES users(id),name TEXT NOT NULL,created INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS play_releases(id TEXT PRIMARY KEY,channel TEXT NOT NULL REFERENCES play_channels(id) ON DELETE CASCADE,manifest TEXT NOT NULL,fingerprint TEXT NOT NULL,stage TEXT NOT NULL,created INTEGER NOT NULL);
    CREATE INDEX IF NOT EXISTS play_releases_channel ON play_releases(channel,created);")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    action: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    alias: String,
    #[serde(default)]
    manifest: Option<Manifest>,
    #[serde(default)]
    outcome: String,
    #[serde(default)]
    note: String,
}
fn manifest(input: &Input) -> ApiResult<&Manifest> {
    let value = input
        .manifest
        .as_ref()
        .ok_or_else(|| bad("A play manifest is required"))?;
    value.validate().map_err(bad)?;
    Ok(value)
}
fn clean_name(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || " -_".contains(c))
}
fn summary(raw: &str) -> Value {
    match serde_json::from_str::<Manifest>(raw) {
        Ok(m) => {
            json!({"game":m.game,"branch":m.branch,"build":m.build,"loader":m.loader,"manager":m.manager,"mod_count":m.mods.len()})
        }
        Err(_) => Value::Null,
    }
}
fn cleanup(db: &Connection) -> rusqlite::Result<()> {
    db.execute("DELETE FROM play_rooms WHERE expires<?1", [now()])?;
    db.execute(
        "DELETE FROM play_reports WHERE created<?1",
        [now() - 30 * 86400],
    )?;
    Ok(())
}
pub fn start_cleanup(app: Shared) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Ok(db) = app.db.lock()
                && let Err(error) = cleanup(&db)
            {
                eprintln!("Play Lab retention cleanup failed: {error}");
            }
        }
    });
}
fn member(db: &Connection, id: &str, user: i64) -> ApiResult<()> {
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM play_members m JOIN play_rooms r ON r.id=m.room WHERE m.room=?1 AND m.user=?2 AND r.expires>?3)",params![id,user,now()],|r|r.get::<_,bool>(0))? {
        return Err(ApiError(StatusCode::FORBIDDEN,"Join this lobby first"));
    }
    Ok(())
}
fn room(db: &Connection, id: &str, user: i64) -> ApiResult<Value> {
    member(db, id, user)?;
    let (owner, raw, expires): (i64, String, i64) = db.query_row(
        "SELECT owner,manifest,expires FROM play_rooms WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let expected: Manifest =
        serde_json::from_str(&raw).map_err(|_| bad("Stored manifest is invalid"))?;
    let rows = db
        .prepare(
            "SELECT user,alias,manifest,seen FROM play_members WHERE room=?1 ORDER BY seen DESC",
        )?
        .query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let members=rows.into_iter().map(|(member,alias,raw,seen)| {
        let checks=serde_json::from_str::<Manifest>(&raw).map(|m|compare(&expected,&m)).unwrap_or_else(|_|vec![play_manifest::Check{level:"blocked".into(),message:"Stored readiness is invalid".into()}]);
        let ready=seen>now()-60 && !checks.iter().any(|c|c.level=="blocked"||c.level=="unknown");
        let total=checks.len();let checks:Vec<_>=checks.into_iter().take(50).collect();
        json!({"alias":alias,"you":member==user,"host":member==owner,"ready":ready,"active":seen>now()-60,"checks":checks,"check_count":total})
    }).collect::<Vec<_>>();
    Ok(json!({"id":id,"host":owner==user,"manifest":expected,"expires":expires,"members":members}))
}
pub async fn action(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Input>,
) -> ApiResult<axum::Json<Value>> {
    let (user, admin) = app.auth(&headers)?;
    app.limits.check(format!("play:{user}"), 120)?;
    if input.id.len() > 64 || input.code.len() > 64 || input.note.len() > 2000 {
        return Err(bad("Play request exceeds limits"));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    cleanup(&tx)?;
    let value = match input.action.as_str() {
        "read-report" => {
            let raw: String = tx
                .query_row(
                    "SELECT manifest FROM play_reports WHERE id=?1",
                    [&input.id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| bad("Report not found"))?;
            json!({"manifest":serde_json::from_str::<Value>(&raw).map_err(|_|bad("Stored report is invalid"))?})
        }
        "read-release" => {
            let raw: String = tx
                .query_row(
                    "SELECT manifest FROM play_releases WHERE id=?1",
                    [&input.id],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| bad("Release not found"))?;
            json!({"manifest":serde_json::from_str::<Value>(&raw).map_err(|_|bad("Stored release is invalid"))?})
        }
        "create" => {
            let m = manifest(&input)?;
            if !clean_name(&input.alias) {
                return Err(bad(
                    "Choose a lobby display name of 1-40 letters, numbers or spaces",
                ));
            }
            let (own,total):(i64,i64)=tx.query_row("SELECT (SELECT COUNT(*) FROM play_rooms WHERE owner=?1),(SELECT COUNT(*) FROM play_rooms)",[user],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if own >= 4 || total >= 200 {
                return Err(bad("Close an old lobby before creating another"));
            }
            let id = Uuid::new_v4().to_string();
            let code = token();
            let raw = serde_json::to_string(m).unwrap();
            tx.execute(
                "INSERT INTO play_rooms VALUES(?1,?2,?3,?4,?5)",
                params![id, user, digest(&code), raw, now() + 86400],
            )?;
            tx.execute(
                "INSERT INTO play_members VALUES(?1,?2,?3,?4,?5)",
                params![id, user, input.alias, raw, now()],
            )?;
            let mut result = room(&tx, &id, user)?;
            result["invite"] = json!(format!("{id}:{code}"));
            result
        }
        "join" => {
            let m = manifest(&input)?;
            if !clean_name(&input.alias) {
                return Err(bad("Choose a lobby display name"));
            }
            app.limits.check(format!("play-join:{user}"), 12)?;
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM play_rooms WHERE id=?1 AND code_hash=?2 AND expires>?3)",params![input.id,digest(&input.code),now()],|r|r.get(0))?;
            if !valid {
                return Err(ApiError(
                    StatusCode::FORBIDDEN,
                    "Lobby invitation is invalid or expired",
                ));
            }
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM play_members WHERE room=?1",
                [&input.id],
                |r| r.get(0),
            )?;
            if count >= 32 {
                member(&tx, &input.id, user)?;
            }
            tx.execute("INSERT INTO play_members VALUES(?1,?2,?3,?4,?5) ON CONFLICT(room,user) DO UPDATE SET alias=excluded.alias,manifest=excluded.manifest,seen=excluded.seen",params![input.id,user,input.alias,serde_json::to_string(m).unwrap(),now()])?;
            room(&tx, &input.id, user)?
        }
        "update" => {
            let m = manifest(&input)?;
            member(&tx, &input.id, user)?;
            tx.execute(
                "UPDATE play_members SET manifest=?1,seen=?2 WHERE room=?3 AND user=?4",
                params![serde_json::to_string(m).unwrap(), now(), input.id, user],
            )?;
            room(&tx, &input.id, user)?
        }
        "read" => room(&tx, &input.id, user)?,
        "leave" => {
            member(&tx, &input.id, user)?;
            tx.execute(
                "DELETE FROM play_members WHERE room=?1 AND user=?2",
                params![input.id, user],
            )?;
            json!({"left":true})
        }
        "close" => {
            let affected = tx.execute(
                "DELETE FROM play_rooms WHERE id=?1 AND owner=?2",
                params![input.id, user],
            )?;
            if affected == 0 {
                return Err(ApiError(
                    StatusCode::FORBIDDEN,
                    "Only the host can close this lobby",
                ));
            }
            json!({"closed":true})
        }
        "report" => {
            let m = manifest(&input)?;
            if !["worked", "failed"].contains(&input.outcome.as_str()) {
                return Err(bad("Choose worked or failed"));
            }
            // One account cannot inflate confidence with repeated reports for the same fingerprint.
            let fp = m.fingerprint().map_err(bad)?;
            let id = Uuid::new_v4().to_string();
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM play_reports WHERE actor=?1 AND created>?2",
                params![user, now() - 86400],
                |r| r.get(0),
            )?;
            let total: i64 = tx.query_row("SELECT COUNT(*) FROM play_reports", [], |r| r.get(0))?;
            if count >= 20 || total >= 10000 {
                return Err(bad("Daily compatibility report limit reached"));
            }
            tx.execute(
                "DELETE FROM play_reports WHERE actor=?1 AND fingerprint=?2",
                params![user, fp],
            )?;
            tx.execute(
                "INSERT INTO play_reports VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    id,
                    user,
                    fp,
                    serde_json::to_string(m).unwrap(),
                    input.outcome,
                    redact(&input.note),
                    now()
                ],
            )?;
            json!({"id":id,"fingerprint":fp})
        }
        "remove-report" => {
            tx.execute(
                "DELETE FROM play_reports WHERE id=?1 AND (actor=?2 OR ?3)",
                params![input.id, user, admin],
            )?;
            json!({"removed":true})
        }
        "issue" => {
            let m = manifest(&input)?;
            if input.note.trim().len() < 10 {
                return Err(bad("Describe the problem in at least 10 characters"));
            }
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM support_tickets WHERE user_id=?1 AND created>?2",
                params![user, now() - 86400],
                |r| r.get(0),
            )?;
            if count >= 10 {
                return Err(bad("Daily support limit reached"));
            }
            let id = Uuid::new_v4().to_string();
            let body = format!(
                "Play Lab report\n{}\nManifest: {}",
                redact(&input.note),
                serde_json::to_string(m).unwrap()
            );
            tx.execute("INSERT INTO support_tickets(id,secret,user_id,client,subject,category,created,updated) VALUES(?1,?2,?3,'play-lab','Play Lab setup issue','bug',?4,?4)",params![id,digest(&token()),user,now()])?;
            tx.execute(
                "INSERT INTO support_messages VALUES(?1,?2,0,?3,?4)",
                params![Uuid::new_v4().to_string(), id, body, now()],
            )?;
            json!({"ticket":id})
        }
        "publish" => {
            let m = manifest(&input)?;
            let channel = if input.id.is_empty() {
                if !clean_name(&input.alias) {
                    return Err(bad("Choose a release channel name"));
                }
                let count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM play_channels WHERE owner=?1",
                    [user],
                    |r| r.get(0),
                )?;
                let total: i64 =
                    tx.query_row("SELECT COUNT(*) FROM play_channels", [], |r| r.get(0))?;
                if count >= 20 || total >= 500 {
                    return Err(bad("Release channel limit reached"));
                }
                let id = Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO play_channels VALUES(?1,?2,?3,?4)",
                    params![id, user, input.alias, now()],
                )?;
                id
            } else {
                input.id.clone()
            };
            let owned: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM play_channels WHERE id=?1 AND owner=?2)",
                params![channel, user],
                |r| r.get(0),
            )?;
            if !owned {
                return Err(ApiError(
                    StatusCode::FORBIDDEN,
                    "Only this channel's owner can publish",
                ));
            }
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM play_releases WHERE channel=?1",
                [&channel],
                |r| r.get(0),
            )?;
            let total: i64 =
                tx.query_row("SELECT COUNT(*) FROM play_releases", [], |r| r.get(0))?;
            if count >= 30 || total >= 4000 {
                return Err(bad("Remove an old experimental release first"));
            }
            let id = Uuid::new_v4().to_string();
            let fp = m.fingerprint().map_err(bad)?;
            tx.execute(
                "INSERT INTO play_releases VALUES(?1,?2,?3,?4,'experimental',?5)",
                params![id, channel, serde_json::to_string(m).unwrap(), fp, now()],
            )?;
            json!({"channel":channel,"release":id,"stage":"experimental"})
        }
        "promote" | "remove-release" => {
            let channel:String=tx.query_row("SELECT r.channel FROM play_releases r JOIN play_channels c ON c.id=r.channel WHERE r.id=?1 AND c.owner=?2",params![input.id,user],|r|r.get(0)).optional()?.ok_or(ApiError(StatusCode::FORBIDDEN,"Only the release owner can change it"))?;
            if input.action == "promote" {
                let raw: String = tx.query_row(
                    "SELECT manifest FROM play_releases WHERE id=?1",
                    [&input.id],
                    |r| r.get(0),
                )?;
                let m: Manifest = serde_json::from_str(&raw).map_err(|_| bad("Invalid release"))?;
                if m.preflight()
                    .iter()
                    .any(|c| c.level == "blocked" || c.level == "unknown")
                {
                    return Err(bad("Resolve manifest blockers before promotion"));
                }
                let fp = m.fingerprint().map_err(bad)?;
                let tested:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM play_reports WHERE actor=?1 AND fingerprint=?2 AND outcome='worked')",params![user,fp],|r|r.get(0))?;
                if !tested {
                    return Err(bad(
                        "Record a successful session for this exact release first",
                    ));
                }
                tx.execute(
                    "UPDATE play_releases SET stage='archived' WHERE channel=?1 AND stage='stable'",
                    [&channel],
                )?;
                tx.execute(
                    "UPDATE play_releases SET stage='stable' WHERE id=?1",
                    [&input.id],
                )?;
            } else {
                tx.execute("DELETE FROM play_releases WHERE id=?1", [&input.id])?;
            }
            json!({"changed":true})
        }
        _ => return Err(bad("Unknown Play Lab action")),
    };
    if [
        "create",
        "join",
        "leave",
        "close",
        "publish",
        "promote",
        "remove-release",
        "remove-report",
        "issue",
    ]
    .contains(&input.action.as_str())
    {
        let target = value
            .get("id")
            .or_else(|| value.get("release"))
            .or_else(|| value.get("ticket"))
            .and_then(Value::as_str)
            .unwrap_or(&input.id);
        tx.execute(
            "INSERT INTO audit(actor,action,target,created) VALUES(?1,?2,?3,?4)",
            params![user, format!("play_{}", input.action), target, now()],
        )?;
    }
    tx.commit()?;
    Ok(axum::Json(value))
}
#[derive(Deserialize)]
pub struct Filter {
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    channel: String,
    #[serde(default)]
    page: usize,
}
pub async fn list(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<Filter>,
) -> ApiResult<axum::Json<Value>> {
    let (user, admin) = app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    cleanup(&db)?;
    if q.fingerprint.len() > 64 || q.channel.len() > 64 {
        return Err(bad("Invalid Play Lab filter"));
    }
    let offset = q.page.clamp(1, 10000).saturating_sub(1) * 50;
    let rooms=db.prepare("SELECT r.id,r.owner=?1,r.expires FROM play_rooms r JOIN play_members m ON m.room=r.id WHERE m.user=?1 ORDER BY r.expires DESC")?.query_map([user],|r|Ok(json!({"id":r.get::<_,String>(0)?,"host":r.get::<_,bool>(1)?,"expires":r.get::<_,i64>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
    let reports=db.prepare("SELECT id,fingerprint,manifest,outcome,note,created,actor=?1 FROM play_reports WHERE (?2='' OR fingerprint=?2) ORDER BY created DESC LIMIT 50 OFFSET ?3")?.query_map(params![user,q.fingerprint,offset as i64],|r|Ok(json!({"id":r.get::<_,String>(0)?,"fingerprint":r.get::<_,String>(1)?,"manifest":summary(&r.get::<_,String>(2)?),"outcome":r.get::<_,String>(3)?,"note":r.get::<_,String>(4)?,"created":r.get::<_,i64>(5)?,"can_remove":r.get::<_,bool>(6)?||admin})))?.collect::<Result<Vec<_>,_>>()?;
    let total: i64 = db.query_row(
        "SELECT COUNT(*) FROM play_reports WHERE (?1='' OR fingerprint=?1)",
        [&q.fingerprint],
        |r| r.get(0),
    )?;
    let channels=db.prepare("SELECT id,name,owner=?1 FROM play_channels ORDER BY created DESC LIMIT 50 OFFSET ?2")?.query_map(params![user,offset as i64],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"you":r.get::<_,bool>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
    let releases = if q.channel.is_empty() {
        vec![]
    } else {
        db.prepare("SELECT r.id,r.manifest,r.fingerprint,r.stage,r.created,c.owner=?2 FROM play_releases r JOIN play_channels c ON c.id=r.channel WHERE r.channel=?1 ORDER BY r.created DESC LIMIT 30")?.query_map(params![q.channel,user],|r|Ok(json!({"id":r.get::<_,String>(0)?,"manifest":summary(&r.get::<_,String>(1)?),"fingerprint":r.get::<_,String>(2)?,"stage":r.get::<_,String>(3)?,"created":r.get::<_,i64>(4)?,"you":r.get::<_,bool>(5)?})))?.collect::<Result<Vec<_>,_>>()?
    };
    let channel_total: i64 =
        db.query_row("SELECT COUNT(*) FROM play_channels", [], |r| r.get(0))?;
    Ok(axum::Json(
        json!({"channel_total":channel_total,"rooms":rooms,"reports":reports,"channels":channels,"releases":releases,"total":total,"page":q.page.max(1),"retention_days":30}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    fn manifest() -> Value {
        json!({"game":550,"branch":"public","build":"123","loader":"Source VPK addons","manager":"test","mods":[{"name":"Hop","version":"1","sha256":"a".repeat(64),"dependencies":[]}],"shared_configs":{}})
    }
    async fn action(app: Shared, auth: &str, input: Value) -> Value {
        let response = call(app, "POST", "/api/v1/play/action", input, Some(auth)).await;
        assert_eq!(response.status(), StatusCode::OK);
        value(response).await
    }
    #[tokio::test]
    async fn lobbies_enforce_membership_expiry_and_private_payloads() {
        let (_dir, app) = fixture();
        let host = account(&app, "host", false);
        let guest = account(&app, "guest", false);
        let outsider = account(&app, "outsider", false);
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/play", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let room = action(
            app.clone(),
            &host,
            json!({"action":"create","alias":"Leaf","manifest":manifest()}),
        )
        .await;
        let id = room["id"].as_str().unwrap();
        let invitation = room["invite"].as_str().unwrap();
        let code = invitation.split_once(':').unwrap().1;
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/play/action",
                json!({"action":"read","id":id}),
                Some(&outsider)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(call(app.clone(),"POST","/api/v1/play/action",json!({"action":"join","id":id,"code":"wrong","alias":"Friend","manifest":manifest()}),Some(&guest)).await.status(),StatusCode::FORBIDDEN);
        let joined = action(
            app.clone(),
            &guest,
            json!({"action":"join","id":id,"code":code,"alias":"Friend","manifest":manifest()}),
        )
        .await;
        assert!(
            joined["members"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["ready"] == true)
        );
        let text = joined.to_string();
        for forbidden in [
            "username",
            "device",
            "ip_address",
            "user_id",
            "code_hash",
            "invite",
        ] {
            assert!(!text.contains(forbidden));
        }
        let mut mismatch = manifest();
        mismatch["build"] = "124".into();
        let changed = action(
            app.clone(),
            &guest,
            json!({"action":"update","id":id,"manifest":mismatch}),
        )
        .await;
        assert!(
            changed["members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["you"] == true && m["ready"] == false)
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE play_members SET seen=?1 WHERE room=?2",
                params![now() - 61, id],
            )
            .unwrap();
        let stale = action(app.clone(), &host, json!({"action":"read","id":id})).await;
        assert!(
            stale["members"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["ready"] == false)
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/play/action",
                json!({"action":"close","id":id}),
                Some(&guest)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let mut private = manifest();
        private["device"] = "private".into();
        assert_ne!(
            call(
                app.clone(),
                "POST",
                "/api/v1/play/action",
                json!({"action":"update","id":id,"manifest":private}),
                Some(&host)
            )
            .await
            .status(),
            StatusCode::OK
        );
        action(app.clone(), &host, json!({"action":"close","id":id})).await;
        assert_eq!(
            call(
                app,
                "POST",
                "/api/v1/play/action",
                json!({"action":"read","id":id}),
                Some(&guest)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn reports_are_anonymous_deduplicated_and_removable() {
        let (_dir, app) = fixture();
        let author = account(&app, "private_author", false);
        let other = account(&app, "other", false);
        for outcome in ["failed", "worked"] {
            action(app.clone(),&author,json!({"action":"report","manifest":manifest(),"outcome":outcome,"note":"Issue at C:/Users/Private with IP 192.168.1.2\nWorks in local session"})).await;
        }
        let data = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/play",
                Value::Null,
                Some(&other),
            )
            .await,
        )
        .await;
        assert_eq!(data["total"], 1);
        let text = data.to_string();
        for secret in ["private_author", "C:/", "192.168.1.2", "actor"] {
            assert!(!text.contains(secret));
        }
        assert_eq!(data["reports"][0]["can_remove"], false);
        let id = data["reports"][0]["id"].as_str().unwrap();
        action(
            app.clone(),
            &other,
            json!({"action":"remove-report","id":id}),
        )
        .await;
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM play_reports", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        action(
            app.clone(),
            &author,
            json!({"action":"remove-report","id":id}),
        )
        .await;
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM play_reports", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn stable_release_requires_owner_and_exact_successful_session() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner_of_pack", false);
        let other = account(&app, "other", false);
        let release = action(
            app.clone(),
            &owner,
            json!({"action":"publish","alias":"Game night","manifest":manifest()}),
        )
        .await;
        let id = release["release"].as_str().unwrap();
        let channel = release["channel"].as_str().unwrap();
        for auth in [&owner, &other] {
            assert_ne!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/play/action",
                    json!({"action":"promote","id":id}),
                    Some(auth)
                )
                .await
                .status(),
                StatusCode::OK
            );
        }
        action(app.clone(),&owner,json!({"action":"report","manifest":manifest(),"outcome":"worked","note":"Local keyboard session passed"})).await;
        action(app.clone(), &owner, json!({"action":"promote","id":id})).await;
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/play/action",
                json!({"action":"publish","id":channel,"manifest":manifest()}),
                Some(&other)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let data = value(
            call(
                app.clone(),
                "GET",
                &format!("/api/v1/play?channel={channel}"),
                Value::Null,
                Some(&other),
            )
            .await,
        )
        .await;
        assert_eq!(data["releases"][0]["stage"], "stable");
        assert!(!data.to_string().contains("owner_of_pack"));
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE play_reports SET created=?1", [now() - 31 * 86400])
            .unwrap();
        let data = value(call(app, "GET", "/api/v1/play", Value::Null, Some(&owner)).await).await;
        assert_eq!(data["total"], 0);
    }
    #[tokio::test]
    async fn issue_reports_create_private_tickets_without_raw_logs() {
        let (_dir, app) = fixture();
        let user = account(&app, "reporter", false);
        let result=action(app.clone(),&user,json!({"action":"issue","manifest":manifest(),"note":"Missing dependency during local launch\nAuthorization: secret"})).await;
        let id = result["ticket"].as_str().unwrap();
        let body: String = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT body FROM support_messages WHERE ticket=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(body.contains("Missing dependency"));
        assert!(!body.contains("Authorization"));
    }
    #[tokio::test]
    async fn thousand_record_lists_are_paged_and_return_compact_private_metadata() {
        let (_dir, app) = fixture();
        let auth = account(&app, "private_report_author", false);
        let user: i64 = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM users WHERE username='private_report_author'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let raw = manifest().to_string();
        {
            let mut db = app.db.lock().unwrap();
            let tx = db.transaction().unwrap();
            for i in 0..1001 {
                tx.execute("INSERT INTO play_reports VALUES(?1,?2,?3,?4,'worked','Fixture observation',?5)",params![format!("fixture-{i}"),user,"fingerprint",raw,now()+i]).unwrap();
            }
            tx.commit().unwrap();
        }
        let data = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/play?page=20",
                Value::Null,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(data["total"], 1001);
        assert_eq!(data["reports"].as_array().unwrap().len(), 50);
        assert!(data["reports"][0]["manifest"]["mods"].is_null());
        assert!(data.to_string().len() < 65536);
        assert!(!data.to_string().contains("private_report_author"));
        let id = data["reports"][0]["id"].as_str().unwrap();
        let detail = action(app.clone(), &auth, json!({"action":"read-report","id":id})).await;
        assert!(detail["manifest"]["mods"].is_array());
        let tail = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/play?page=21",
                Value::Null,
                Some(&auth),
            )
            .await,
        )
        .await;
        assert_eq!(tail["reports"].as_array().unwrap().len(), 1);
        assert_eq!(
            call(app, "GET", "/api/v1/play", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
