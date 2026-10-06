use super::*;
use futures_util::TryStreamExt;
use std::io::Read;

pub fn role(app: &App, id: i64) -> ApiResult<String> {
    Ok(app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT role FROM users WHERE id=?1", [id], |r| r.get(0))?)
}
fn moderator(app: &App, headers: &HeaderMap) -> ApiResult<(i64, String)> {
    let (id, allowed) = app.auth(headers)?;
    if !allowed {
        return Err(ApiError(StatusCode::FORBIDDEN, "Moderator access required"));
    }
    Ok((id, role(app, id)?))
}
pub(super) fn owner(app: &App, headers: &HeaderMap) -> ApiResult<i64> {
    let (id, role) = moderator(app, headers)?;
    if role != "owner" {
        return Err(ApiError(StatusCode::FORBIDDEN, "Owner access required"));
    }
    Ok(id)
}
fn record(db: &Connection, actor: i64, action: &str, target: &str) -> ApiResult<()> {
    db.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,?2,?3,?4)",
        params![actor, action, target, now()],
    )?;
    Ok(())
}
pub async fn users(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(page): Query<lists::Page>,
) -> ApiResult<axum::Json<Value>> {
    moderator(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let total: i64 = db.query_row(
        "SELECT COUNT(*) FROM users WHERE instr(lower(username),lower(?1))>0",
        [page.term()],
        |r| r.get(0),
    )?;
    let mut statement = db.prepare("SELECT id,username,role,banned,verified,invites_remaining FROM users WHERE instr(lower(username),lower(?1))>0 ORDER BY username,id LIMIT ?2 OFFSET ?3")?;
    let users = statement.query_map(params![page.term(),page.limit(500),page.offset()], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"username":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"banned":r.get::<_,bool>(3)?,"verified":r.get::<_,bool>(4)?,"invites_remaining":r.get::<_,i64>(5)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(page.response(users, total)))
}
#[derive(Deserialize)]
pub struct Ban {
    banned: bool,
}
pub async fn ban(
    State(app): State<Shared>,
    Path(target): Path<i64>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Ban>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, actor_role) = moderator(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let target_role: String = tx
        .query_row("SELECT role FROM users WHERE id=?1", [target], |r| r.get(0))
        .optional()?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "Member not found"))?;
    if target == actor
        || target_role == "owner"
        || (actor_role == "admin" && target_role == "admin")
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "You cannot moderate this account",
        ));
    }
    tx.execute(
        "UPDATE users SET banned=?1 WHERE id=?2",
        params![input.banned, target],
    )?;
    if input.banned {
        tx.execute("DELETE FROM sessions WHERE user_id=?1", [target])?;
        tx.execute("DELETE FROM codes WHERE user_id=?1", [target])?;
        tx.execute("DELETE FROM trusted_devices WHERE user_id=?1", [target])?;
        tx.execute("DELETE FROM login_codes WHERE user_id=?1", [target])?;
        tx.execute("DELETE FROM invites WHERE issued_by=?1", [target])?;
    }
    record(
        &tx,
        actor,
        if input.banned { "ban" } else { "unban" },
        &target.to_string(),
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct RoleChange {
    role: String,
}
pub async fn set_role(
    State(app): State<Shared>,
    Path(target): Path<i64>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<RoleChange>,
) -> ApiResult<axum::Json<Value>> {
    let actor = owner(&app, &headers)?;
    if !matches!(input.role.as_str(), "member" | "vip" | "admin") {
        return Err(bad(
            "Choose member, vip or admin; ownership uses the transfer action",
        ));
    }
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute("UPDATE users SET role=?1,admin=?2 WHERE id=?3 AND role!='owner' AND verified=1 AND banned=0",params![input.role,input.role=="admin",target])? != 1 { return Err(bad("Choose an active verified member other than the owner")); }
    tx.execute("DELETE FROM sessions WHERE user_id=?1", [target])?;
    tx.execute("DELETE FROM trusted_devices WHERE user_id=?1", [target])?;
    tx.execute("DELETE FROM login_codes WHERE user_id=?1", [target])?;
    record(
        &tx,
        actor,
        &format!("role:{}", input.role),
        &target.to_string(),
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct Transfer {
    user_id: i64,
}
pub async fn transfer_owner(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Transfer>,
) -> ApiResult<axum::Json<Value>> {
    let actor = owner(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let active: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND id!=?2 AND verified=1 AND banned=0)",
        params![input.user_id, actor],
        |r| r.get(0),
    )?;
    if !active {
        return Err(bad("Choose another active verified member"));
    }
    tx.execute("UPDATE users SET role='admin',admin=1 WHERE id=?1", [actor])?;
    tx.execute(
        "UPDATE users SET role='owner',admin=1 WHERE id=?1",
        [input.user_id],
    )?;
    tx.execute(
        "DELETE FROM sessions WHERE user_id IN (?1,?2)",
        params![actor, input.user_id],
    )?;
    tx.execute(
        "DELETE FROM trusted_devices WHERE user_id IN (?1,?2)",
        params![actor, input.user_id],
    )?;
    tx.execute(
        "DELETE FROM login_codes WHERE user_id IN (?1,?2)",
        params![actor, input.user_id],
    )?;
    record(&tx, actor, "transfer-owner", &input.user_id.to_string())?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn audit(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(page): Query<lists::Page>,
) -> ApiResult<axum::Json<Value>> {
    moderator(&app, &headers)?;
    let db = app.db.lock().unwrap();
    let total:i64=db.query_row("SELECT COUNT(*) FROM audit a JOIN users u ON u.id=a.actor WHERE instr(lower(a.action || a.target || u.username),lower(?1))>0",[page.term()],|r|r.get(0))?;
    let mut stmt=db.prepare("SELECT a.id,u.username,a.action,a.target,a.created FROM audit a JOIN users u ON u.id=a.actor WHERE instr(lower(a.action || a.target || u.username),lower(?1))>0 ORDER BY a.id DESC LIMIT ?2 OFFSET ?3")?;
    let items=stmt.query_map(params![page.term(),page.limit(200),page.offset()],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"actor":r.get::<_,String>(1)?,"action":r.get::<_,String>(2)?,"target":r.get::<_,String>(3)?,"created":r.get::<_,i64>(4)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(page.response(items, total)))
}
pub async fn source(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = moderator(&app, &headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    let _permit = app.upload_gate.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Another archive operation is in progress",
        )
    })?;
    let size: Option<i64> = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT size FROM mods WHERE id=?1", [&id], |r| r.get(0))
        .optional()?;
    let size = size.ok_or(ApiError(StatusCode::NOT_FOUND, "Mod not found"))?;
    if size > 32 * 1024 * 1024 {
        return Err(bad("Source inspection is limited to archives up to 32 MiB"));
    }
    let file = tokio::fs::File::open(app.files.join(format!("{id}.zip"))).await?;
    let chunks: Vec<_> = crypto::read(file, Zeroizing::new(*app.upload_key), id.clone())
        .try_collect()
        .await?;
    let bytes = Zeroizing::new(chunks.concat());
    if bytes.len() > UPLOAD_LIMIT {
        return Err(bad("Archive too large"));
    }
    let files = tokio::task::spawn_blocking(move || read_source(&bytes))
        .await
        .map_err(|_| bad("Archive inspection failed"))??;
    record(&app.db.lock().unwrap(), actor, "inspect-source", &id)?;
    Ok(axum::Json(
        json!({"files":files,"note":"Only source files included by the uploader are available. DLL-only archives do not contain original source."}),
    ))
}
fn read_source(bytes: &[u8]) -> ApiResult<Vec<Value>> {
    // Bound central-directory allocation before asking the ZIP parser to read it.
    // Large ZIP64 archives remain downloadable but are outside source-preview scope.
    let end = bytes
        .windows(4)
        .rposition(|w| w == b"PK\x05\x06")
        .ok_or_else(|| bad("ZIP directory missing"))?;
    let directory = bytes
        .get(end..end + 22)
        .ok_or_else(|| bad("ZIP directory truncated"))?;
    let count = u16::from_le_bytes([directory[10], directory[11]]);
    let length = u32::from_le_bytes(directory[12..16].try_into().unwrap());
    if count > 2000 || length > 4 * 1024 * 1024 {
        return Err(bad("ZIP directory exceeds source inspection limits"));
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| bad("This upload is not a readable ZIP archive"))?;
    if zip.len() > 2000 {
        return Err(bad("Too many archive entries"));
    }
    let mut files = Vec::new();
    let mut total = 0;
    for i in 0..zip.len() {
        let file = zip
            .by_index(i)
            .map_err(|_| bad("Unsupported or encrypted ZIP entry"))?;
        if file.enclosed_name().is_none() || file.is_dir() {
            continue;
        }
        let name = file.name().to_owned();
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if !matches!(
            ext.as_str(),
            "cs" | "rs" | "js" | "ts" | "cpp" | "h" | "lua" | "shader" | "py"
        ) {
            continue;
        }
        if file.size() > 1024 * 1024 || files.len() >= 100 {
            return Err(bad("Source inspection limit exceeded"));
        }
        let mut text = String::new();
        file.take(1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|_| bad("Source file must be UTF-8 text"))?;
        total += text.len();
        if text.len() > 1024 * 1024 || total > 2 * 1024 * 1024 {
            return Err(bad("Source inspection limit exceeded"));
        }
        files.push(json!({"name":name,"text":text}));
    }
    Ok(files)
}
#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    offset: u32,
    #[serde(default)]
    category: String,
}
pub async fn topics(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt=db.prepare("SELECT t.id,t.title,t.category,t.app_id,u.username,t.pinned,t.locked,t.updated,t.mod_id,(SELECT count(*) FROM posts p WHERE p.topic_id=t.id) FROM topics t JOIN users u ON u.id=t.user_id WHERE (?2='' OR t.category=?2) ORDER BY t.pinned DESC,t.updated DESC LIMIT 50 OFFSET ?1")?;
    let items=stmt.query_map(params![page.offset.min(100000),page.category],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"category":r.get::<_,String>(2)?,"app_id":r.get::<_,u32>(3)?,"author":r.get::<_,String>(4)?,"pinned":r.get::<_,bool>(5)?,"locked":r.get::<_,bool>(6)?,"updated":r.get::<_,i64>(7)?,"mod_id":r.get::<_,Option<String>>(8)?,"posts":r.get::<_,i64>(9)?})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(items)))
}
#[derive(Deserialize)]
pub struct TopicInput {
    title: String,
    body: String,
    category: String,
    app_id: u32,
    #[serde(default)]
    mod_id: Option<String>,
}
fn valid_body(body: &str) -> ApiResult<()> {
    if body.trim().is_empty() || body.len() > 10000 {
        Err(bad("Write between 1 and 10000 bytes"))
    } else {
        Ok(())
    }
}
pub async fn new_topic(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<TopicInput>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = app.auth(&headers)?;
    valid_body(&input.body)?;
    if !(3..=140).contains(&input.title.trim().len()) || input.app_id == 0 {
        return Err(bad(
            "Choose a category, Steam game ID and a 3–140 byte title",
        ));
    }
    if let Some(id) = &input.mod_id {
        Uuid::parse_str(id).map_err(|_| bad("Invalid linked mod"))?;
    }
    let id = Uuid::new_v4().to_string();
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    security::content_quota(&tx, actor)?;
    let section: Option<(bool, bool)> = tx
        .query_row(
            "SELECT active,vip_only FROM forum_sections WHERE id=?1",
            [&input.category],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (active, vip_only) = section.ok_or(bad("Choose an existing section"))?;
    if !active {
        return Err(bad("This section is closed to new discussions"));
    }
    let member: bool = tx.query_row(
        "SELECT role='member' FROM users WHERE id=?1",
        [actor],
        |r| r.get(0),
    )?;
    if vip_only && member {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "VIP or moderator access is required to post in this section",
        ));
    }
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM topics WHERE user_id=?1",
        [actor],
        |r| r.get(0),
    )?;
    if count >= 500 {
        return Err(bad("Topic limit reached"));
    }
    tx.execute("INSERT INTO topics(id,user_id,app_id,category,title,created,updated,mod_id) VALUES(?1,?2,?3,?4,?5,?6,?6,?7)",params![id,actor,input.app_id,input.category,input.title.trim(),now(),input.mod_id])?;
    tx.execute(
        "INSERT INTO posts VALUES(?1,?2,?3,?4,?5)",
        params![Uuid::new_v4().to_string(), id, actor, input.body, now()],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"id":id})))
}
pub async fn topic(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let header:Option<Value>=db.query_row("SELECT title,locked,pinned,mod_id FROM topics WHERE id=?1",[&id],|r|Ok(json!({"title":r.get::<_,String>(0)?,"locked":r.get::<_,bool>(1)?,"pinned":r.get::<_,bool>(2)?,"mod_id":r.get::<_,Option<String>>(3)?}))).optional()?;
    let mut header = header.ok_or(ApiError(StatusCode::NOT_FOUND, "Discussion not found"))?;
    let mut stmt=db.prepare("SELECT p.id,p.user_id,u.username,u.role,p.body,p.created,pr.avatar FROM posts p JOIN users u ON u.id=p.user_id LEFT JOIN profiles pr ON pr.user_id=u.id WHERE p.topic_id=?1 ORDER BY p.created,p.rowid LIMIT 200")?;
    let posts=stmt.query_map([&id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"user_id":r.get::<_,i64>(1)?,"author":r.get::<_,String>(2)?,"role":r.get::<_,String>(3)?,"body":r.get::<_,String>(4)?,"created":r.get::<_,i64>(5)?,"avatar":r.get::<_,Option<String>>(6)?.is_some()})))?.collect::<Result<Vec<_>,_>>()?;
    header["posts"] = json!(posts);
    Ok(axum::Json(header))
}
#[derive(Deserialize)]
pub struct Reply {
    body: String,
}
pub async fn reply(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Reply>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, moderator) = app.auth(&headers)?;
    valid_body(&input.body)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    security::content_quota(&tx, actor)?;
    let locked: Option<bool> = tx
        .query_row("SELECT locked FROM topics WHERE id=?1", [&id], |r| r.get(0))
        .optional()?;
    if locked.ok_or(ApiError(StatusCode::NOT_FOUND, "Discussion not found"))? && !moderator {
        return Err(ApiError(StatusCode::FORBIDDEN, "Discussion is locked"));
    }
    let count: i64 = tx.query_row("SELECT count(*) FROM posts WHERE topic_id=?1", [&id], |r| {
        r.get(0)
    })?;
    if count >= 200 {
        return Err(bad("Discussion is full; start a continuation"));
    }
    tx.execute(
        "INSERT INTO posts VALUES(?1,?2,?3,?4,?5)",
        params![Uuid::new_v4().to_string(), id, actor, input.body, now()],
    )?;
    tx.execute(
        "UPDATE topics SET updated=?1 WHERE id=?2",
        params![now(), id],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
#[derive(Deserialize)]
pub struct Moderate {
    locked: bool,
    pinned: bool,
}
pub async fn moderate(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Moderate>,
) -> ApiResult<axum::Json<Value>> {
    let (actor, _) = moderator(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute(
        "UPDATE topics SET locked=?1,pinned=?2 WHERE id=?3",
        params![input.locked, input.pinned, id],
    )? != 1
    {
        return Err(ApiError(StatusCode::NOT_FOUND, "Discussion not found"));
    }
    record(
        &tx,
        actor,
        &format!("topic:locked={},pinned={}", input.locked, input.pinned),
        &id,
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"ok":true})))
}
pub async fn delete_topic(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let (actor, _) = moderator(&app, &headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute("DELETE FROM topics WHERE id=?1", [&id])? != 1 {
        return Err(ApiError(StatusCode::NOT_FOUND, "Discussion not found"));
    }
    record(&tx, actor, "delete-topic", &id)?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn delete_post(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let (actor, moderator) = app.auth(&headers)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    if tx.execute(
        "DELETE FROM posts WHERE id=?1 AND (user_id=?2 OR ?3)",
        params![id, actor, moderator],
    )? != 1
    {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "You cannot remove this post",
        ));
    }
    record(&tx, actor, "delete-post", &id)?;
    tx.commit()?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn section_pages_filter_topics_before_pagination() {
        let (_dir, app) = fixture();
        let member = account(&app, "section-reader", false);
        for (category, title) in [("help", "Help topic"), ("discussion", "Discussion topic")] {
            let response=call(app.clone(),"POST","/api/v1/topics",json!({"title":title,"body":"A useful forum question","category":category,"app_id":1686940}),Some(&member)).await;
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = call(
            app.clone(),
            "GET",
            "/api/v1/topics?category=help",
            Value::Null,
            Some(&member),
        )
        .await;
        let rows = value(response).await;
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["category"], "help");
        assert_eq!(rows[0]["title"], "Help topic");
        let rows = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/topics?category=help&offset=1",
                Value::Null,
                Some(&member),
            )
            .await,
        )
        .await;
        assert!(rows.as_array().unwrap().is_empty());
        assert_eq!(
            call(
                app,
                "GET",
                "/api/v1/topics?category=help",
                Value::Null,
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn ownership_transfer_is_atomic_and_revokes_both_sessions() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let member = account(&app, "member", false);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/transfer-owner",
                json!({"user_id":2}),
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
                "/api/v1/admin/transfer-owner",
                json!({"user_id":999}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(role(&app, 1).unwrap(), "owner");
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/transfer-owner",
                json!({"user_id":2}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(role(&app, 1).unwrap(), "admin");
        assert_eq!(role(&app, 2).unwrap(), "owner");
        for auth in [&owner, &member] {
            assert_eq!(
                call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(auth))
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT count(*) FROM users WHERE role='owner'", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn roles_bans_and_forum_moderation_enforce_hierarchy() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", true);
        let admin = account(&app, "moderator", false);
        let member = account(&app, "member", false);
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE users SET role='admin',admin=1 WHERE username='moderator'",
                [],
            )
            .unwrap();
        let id: i64 = app
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM users WHERE username='member'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/admin/users",
                Value::Null,
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        for path in ["/api/v1/invites", "/api/v1/invite-waves"] {
            assert_eq!(
                call(app.clone(), "POST", path, json!({"count":1}), Some(&admin))
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/admin/users/{id}/role"),
                json!({"role":"owner"}),
                Some(&admin)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/admin/users/1/ban",
                json!({"banned":true}),
                Some(&admin)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let input = json!({"title":"Unity help","body":"How do I patch a method?","category":"guides","app_id":1686940});
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/topics",
                input.clone(),
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
                &format!("/api/v1/admin/users/{id}/role"),
                json!({"role":"vip"}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&member))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let vip = app.session(id, &HeaderMap::new()).unwrap().0["token"]
            .as_str()
            .unwrap()
            .to_owned();
        let response = call(app.clone(), "POST", "/api/v1/topics", input, Some(&vip)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let topic = value(response).await["id"].as_str().unwrap().to_owned();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/topics/{topic}/moderate"),
                json!({"locked":true,"pinned":true}),
                Some(&vip)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/topics/{topic}/moderate"),
                json!({"locked":true,"pinned":true}),
                Some(&admin)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/topics/{topic}/reply"),
                json!({"body":"reply"}),
                Some(&vip)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &format!("/api/v1/admin/users/{id}/ban"),
                json!({"banned":true}),
                Some(&admin)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(&vip))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/topics",
                Value::Null,
                Some(&vip)
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[test]
    fn source_reader_returns_text_and_rejects_unsafe_paths() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("Plugin.cs", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"class Plugin {}").unwrap();
        zip.start_file("../private.cs", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"hidden").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let files = read_source(&bytes).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["text"], "class Plugin {}");
    }
}
