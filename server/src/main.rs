use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use rand::{RngCore, rngs::OsRng};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;
mod catalog;
mod community;
mod crypto;
mod curseforge;
mod devices;
mod email;
mod external;
mod handoff;
mod live;
mod profiles;
mod sections;
mod security;
mod twofactor;
mod updates;
use uuid::Uuid;

const UPLOAD_LIMIT: usize = 128 * 1024 * 1024;
const STORAGE_LIMIT: i64 = 30 * 1024 * 1024 * 1024;
type Shared = Arc<App>;
type ApiResult<T> = Result<T, ApiError>;
struct App {
    db: Mutex<Connection>,
    files: PathBuf,
    auth_gate: Arc<Semaphore>,
    upload_gate: Semaphore,
    limits: security::Limits,
    upload_key: Zeroizing<[u8; 32]>,
    mail: email::Mailer,
    dummy_hash: String,
    live: live::Live,
}
#[derive(Debug)]
struct ApiError(StatusCode, &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, axum::Json(json!({"error":self.1}))).into_response()
    }
}
impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        eprintln!("Database operation failed: {error}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database operation failed",
        )
    }
}
impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        eprintln!("File operation failed: {error}");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "File operation failed")
    }
}
fn bad(message: &'static str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message)
}
fn denied() -> ApiError {
    ApiError(StatusCode::UNAUTHORIZED, "Sign in to your family account")
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn token() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
impl App {
    fn open(
        database: &std::path::Path,
        files: PathBuf,
        key: &str,
        upload_key: [u8; 32],
        mail: email::Mailer,
    ) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&files)?;
        anyhow::ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Database key must contain 32 random bytes in hexadecimal"
        );
        let db = Connection::open(database)?;
        db.execute_batch(&format!(
            "PRAGMA key = \"x'{key}'\"; PRAGMA cipher_memory_security=ON;"
        ))?;
        let cipher: String = db.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
        anyhow::ensure!(!cipher.is_empty(), "SQLCipher encryption is required");
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, username TEXT UNIQUE COLLATE NOCASE NOT NULL, password TEXT NOT NULL, admin INTEGER NOT NULL DEFAULT 0, invites_remaining INTEGER NOT NULL DEFAULT 1, email TEXT UNIQUE COLLATE NOCASE, verified INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS invites(hash TEXT PRIMARY KEY, admin INTEGER NOT NULL DEFAULT 0, expires INTEGER NOT NULL, issued_by INTEGER REFERENCES users(id), wave TEXT);
            CREATE TABLE IF NOT EXISTS sessions(hash TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), expires INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS mods(id TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), app_id INTEGER NOT NULL, name TEXT NOT NULL, version TEXT NOT NULL, description TEXT NOT NULL, sha256 TEXT NOT NULL, size INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS packs(id TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), name TEXT NOT NULL, manifest TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS codes(challenge TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), hash TEXT NOT NULL, kind TEXT NOT NULL, expires INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS secrets(name TEXT PRIMARY KEY,value TEXT NOT NULL);")?;
        let columns: Vec<String> = db
            .prepare("PRAGMA table_info(users)")?
            .query_map([], |r| r.get(1))?
            .collect::<Result<_, _>>()?;
        if !columns.iter().any(|c| c == "email") {
            db.execute_batch("ALTER TABLE users ADD COLUMN email TEXT; CREATE UNIQUE INDEX users_email_unique ON users(email COLLATE NOCASE);")?;
        }
        if !columns.iter().any(|c| c == "verified") {
            db.execute_batch("ALTER TABLE users ADD COLUMN verified INTEGER NOT NULL DEFAULT 0;")?;
        }
        if !columns.iter().any(|c| c == "role") {
            db.execute_batch("ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'member'; UPDATE users SET role='owner' WHERE admin=1;")?;
        }
        if !columns.iter().any(|c| c == "banned") {
            db.execute_batch("ALTER TABLE users ADD COLUMN banned INTEGER NOT NULL DEFAULT 0;")?;
        }
        db.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS single_owner ON users(role) WHERE role='owner';
            CREATE TABLE IF NOT EXISTS audit(id INTEGER PRIMARY KEY, actor INTEGER NOT NULL REFERENCES users(id), action TEXT NOT NULL, target TEXT NOT NULL, created INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS topics(id TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), app_id INTEGER NOT NULL, category TEXT NOT NULL, title TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, locked INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL, updated INTEGER NOT NULL, mod_id TEXT REFERENCES mods(id) ON DELETE SET NULL);
            CREATE TABLE IF NOT EXISTS posts(id TEXT PRIMARY KEY, topic_id TEXT NOT NULL REFERENCES topics(id) ON DELETE CASCADE, user_id INTEGER NOT NULL REFERENCES users(id), body TEXT NOT NULL, created INTEGER NOT NULL);")?;
        sections::initialize(&db)?;
        external::initialize(&db)?;
        catalog::initialize(&db)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS mod_reviews(mod_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE, approved INTEGER NOT NULL DEFAULT 0);")?;
        handoff::initialize(&db)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS profiles(user_id INTEGER PRIMARY KEY REFERENCES users(id), status TEXT NOT NULL DEFAULT '', bio TEXT NOT NULL DEFAULT '', avatar TEXT);
            CREATE TABLE IF NOT EXISTS profile_comments(id TEXT PRIMARY KEY,target INTEGER NOT NULL REFERENCES users(id),author INTEGER NOT NULL REFERENCES users(id),body TEXT NOT NULL,created INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS ratings(target INTEGER NOT NULL REFERENCES users(id),voter INTEGER NOT NULL REFERENCES users(id),stars INTEGER NOT NULL CHECK(stars BETWEEN 1 AND 5),PRIMARY KEY(target,voter),CHECK(target!=voter));")?;
        let mfa_exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='trusted_devices')",
            [],
            |r| r.get(0),
        )?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS trusted_devices(hash TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES users(id),expires INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS login_codes(challenge TEXT PRIMARY KEY,user_id INTEGER NOT NULL REFERENCES users(id),hash TEXT NOT NULL,binding TEXT NOT NULL,expires INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0);")?;
        if !mfa_exists {
            db.execute("DELETE FROM sessions", [])?;
        }
        devices::initialize(&db)?;
        Ok(Self {
            db: Mutex::new(db),
            live: live::Live::new(),
            files,
            auth_gate: Arc::new(Semaphore::new(2)),
            upload_gate: Semaphore::new(1),
            limits: security::Limits::default(),
            upload_key: Zeroizing::new(upload_key),
            mail,
            dummy_hash: Argon2::new(
                argon2::Algorithm::Argon2id,
                argon2::Version::V0x13,
                argon2::Params::new(65536, 3, 1, None).unwrap(),
            )
            .hash_password(
                b"dummy-login-password-not-an-account",
                &SaltString::generate(&mut OsRng),
            )
            .map_err(|_| anyhow::anyhow!("Password setup failed"))?
            .to_string(),
        })
    }
    fn auth(&self, headers: &HeaderMap) -> ApiResult<(i64, bool)> {
        let raw = auth_token(headers).ok_or_else(denied)?;
        if raw.len() != 64 {
            return Err(denied());
        }
        let db = self.db.lock().unwrap();
        let result = db.query_row("SELECT u.id,u.role IN ('admin','owner') FROM users u JOIN sessions s ON u.id=s.user_id WHERE s.hash=?1 AND (s.expires=-1 OR s.expires>?2) AND u.verified=1 AND u.banned=0", params![digest(raw), now()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.ok_or_else(denied)?;
        db.execute(
            "UPDATE session_devices SET last_seen=?1 WHERE hash=?2 AND last_seen<?3",
            params![now(), digest(raw), now() - 30],
        )?;
        if let Some(trust) = devices::trust(headers) {
            db.execute("UPDATE session_devices SET trust_hash=?1 WHERE hash=?2 AND EXISTS(SELECT 1 FROM trusted_devices WHERE hash=?1 AND user_id=?3)",params![trust,digest(raw),result.0])?;
        }
        Ok(result)
    }
    fn session(&self, id: i64, headers: &HeaderMap) -> ApiResult<axum::Json<Value>> {
        let raw = token();
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "DELETE FROM sessions WHERE expires<>-1 AND expires<=?1",
            [now()],
        )?;
        devices::record(&tx, &raw, id, &devices::name(headers), "browser")?;
        if let Some(trust) = devices::trust(headers) {
            tx.execute(
                "UPDATE session_devices SET trust_hash=?1 WHERE hash=?2",
                params![trust, digest(&raw)],
            )?;
        }
        tx.commit()?;
        Ok(axum::Json(json!({"token":raw,"expires_in":null})))
    }
}
fn auth_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .or_else(|| {
            headers
                .get("cookie")
                .and_then(|h| h.to_str().ok())
                .and_then(|cookies| {
                    cookies
                        .split(';')
                        .find_map(|c| c.trim().strip_prefix("canna_session="))
                })
        })
}
fn session_response(session: axum::Json<Value>) -> Response {
    let raw = session.0["token"].as_str().unwrap();
    (
        [(
            "set-cookie",
            format!(
                "canna_session={raw}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
                devices::COOKIE_AGE
            ),
        )],
        session,
    )
        .into_response()
}
async fn origin_check(request: Request, next: Next) -> Response {
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) && (request.headers().contains_key("origin")
        || (request.headers().contains_key("cookie")
            && !request.headers().contains_key("authorization")))
    {
        let origin = request
            .headers()
            .get("origin")
            .and_then(|v| v.to_str().ok());
        if !matches!(
            origin,
            Some("https://cannamods.vip" | "https://api.cannamods.vip")
        ) {
            return ApiError(StatusCode::FORBIDDEN, "Invalid request origin").into_response();
        }
    }
    next.run(request).await
}
fn valid_credentials(username: &str, password: &str) -> bool {
    (3..=32).contains(&username.len())
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && (12..=256).contains(&password.len())
}
#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    invite: String,
    #[serde(default)]
    email: String,
}
async fn login(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Credentials>,
) -> ApiResult<Response> {
    if !valid_credentials(&input.username, &input.password) {
        return Err(denied());
    }
    app.limits.check(
        format!("account:{}", digest(&input.username.to_ascii_lowercase())),
        20,
    )?;
    let entry: Option<(i64, String)> = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id,password FROM users WHERE username=?1 AND verified=1 AND banned=0",
            [&input.username],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (id, hash) = entry.unwrap_or((0, app.dummy_hash.clone()));
    let permit = app
        .auth_gate
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError(StatusCode::TOO_MANY_REQUESTS, "Please try again shortly"))?;
    let matches = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let password = Zeroizing::new(input.password);
        PasswordHash::new(&hash).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
    })
    .await
    .map_err(|_| denied())?;
    if !matches || id == 0 {
        return Err(denied());
    }
    twofactor::start(app, id, &headers).await
}
async fn logout(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    let raw = auth_token(&headers).ok_or_else(denied)?;
    app.db
        .lock()
        .unwrap()
        .execute("DELETE FROM sessions WHERE hash=?1", [digest(raw)])?;
    Ok((
        [(
            "set-cookie",
            "canna_session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0",
        )],
        StatusCode::NO_CONTENT,
    )
        .into_response())
}
async fn me(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (id, admin) = app.auth(&headers)?;
    let (name, remaining): (String, u32) = app.db.lock().unwrap().query_row(
        "SELECT username,invites_remaining FROM users WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let role = community::role(&app, id)?;
    Ok(axum::Json(
        json!({"id":id,"username":name,"admin":admin,"role":role,"invites_remaining":remaining,"can_invite":role!="admin" && (role=="owner" || remaining>0),"can_publish_guides":role!="member"}),
    ))
}
async fn invite(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    let (issuer, _) = app.auth(&headers)?;
    let role = community::role(&app, issuer)?;
    if role == "admin" {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Admins cannot issue invitations",
        ));
    }
    let admin = role == "owner";
    let raw = token();
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("DELETE FROM invites WHERE expires<=?1", [now()])?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM invites", [], |r| r.get(0))?;
    if count >= 200 {
        return Err(bad("There are already 200 pending invitations"));
    }
    if !admin && tx.execute("UPDATE users SET invites_remaining=invites_remaining-1 WHERE id=?1 AND invites_remaining>0", [issuer])? != 1 {
        return Err(ApiError(StatusCode::FORBIDDEN,"You have already used your friend invitation"));
    }
    tx.execute(
        "INSERT INTO invites(hash,admin,expires,issued_by) VALUES(?1,0,?2,?3)",
        params![digest(&raw), now() + 7 * 86400, issuer],
    )?;
    tx.commit()?;
    Ok(axum::Json(json!({"invite":raw,"expires_in":7*86400})))
}
#[derive(Deserialize)]
struct Wave {
    count: u32,
}
async fn invite_wave(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Wave>,
) -> ApiResult<axum::Json<Value>> {
    let (issuer, _) = app.auth(&headers)?;
    if community::role(&app, issuer)? != "owner" {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Only the owner can create invite waves",
        ));
    }
    if !(1..=50).contains(&input.count) {
        return Err(bad("Choose between 1 and 50 invitations"));
    }
    let wave = Uuid::new_v4().to_string();
    let expires = now() + 7 * 86400;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    tx.execute("DELETE FROM invites WHERE expires<=?1", [now()])?;
    let pending: u32 = tx.query_row("SELECT COUNT(*) FROM invites", [], |r| r.get(0))?;
    if pending + input.count > 200 {
        return Err(bad("Maximum 200 pending invitations"));
    }
    let mut codes = Vec::new();
    for _ in 0..input.count {
        let raw = token();
        tx.execute(
            "INSERT INTO invites(hash,admin,expires,issued_by,wave) VALUES(?1,0,?2,?3,?4)",
            params![digest(&raw), expires, issuer, wave],
        )?;
        codes.push(raw);
    }
    tx.commit()?;
    Ok(axum::Json(
        json!({"wave":wave,"invites":codes,"expires_in":7*86400}),
    ))
}
async fn revoke_wave(
    State(app): State<Shared>,
    Path(wave): Path<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    if community::role(&app, app.auth(&headers)?.0)? != "owner" {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Only the owner can revoke invite waves",
        ));
    }
    let removed = app
        .db
        .lock()
        .unwrap()
        .execute("DELETE FROM invites WHERE wave=?1", [wave])?;
    Ok(axum::Json(json!({"revoked":removed})))
}
#[derive(Deserialize)]
struct UploadInfo {
    app_id: u32,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
}
async fn upload(
    State(app): State<Shared>,
    Query(info): Query<UploadInfo>,
    request: Request,
) -> ApiResult<axum::Json<Value>> {
    let (owner, _) = app.auth(request.headers())?;
    if info.app_id == 0
        || info.name.trim().is_empty()
        || info.name.len() > 100
        || info.version.is_empty()
        || info.version.len() > 40
        || info.description.len() > 4000
    {
        return Err(bad("Invalid mod metadata"));
    }
    let _permit = app.upload_gate.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "Another upload is in progress; try again shortly",
        )
    })?;
    let used: i64 =
        app.db
            .lock()
            .unwrap()
            .query_row("SELECT COALESCE(SUM(size),0) FROM mods", [], |r| r.get(0))?;
    if used + UPLOAD_LIMIT as i64 > STORAGE_LIMIT {
        return Err(bad("Upload storage is full"));
    }
    security::quota(&app.db.lock().unwrap(), owner, UPLOAD_LIMIT as i64)?;
    let id = Uuid::new_v4().to_string();
    let partial = app.files.join(format!("{id}.partial"));
    let path = app.files.join(format!("{id}.zip"));
    let result: ApiResult<(usize, String)> = async {
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .await?;
        let mut file = crypto::Writer::new(file, &app.upload_key, id.clone()).await?;
        let mut stream = request.into_body().into_data_stream();
        let mut size = 0usize;
        let mut hash = Sha256::new();
        let mut magic = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| bad("Upload interrupted"))?;
            size = size
                .checked_add(chunk.len())
                .ok_or_else(|| bad("Upload too large"))?;
            if size > UPLOAD_LIMIT {
                return Err(ApiError(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "ZIP uploads are limited to 128 MiB",
                ));
            }
            for byte in chunk.iter().take(4usize.saturating_sub(magic.len())) {
                magic.push(*byte);
            }
            hash.update(&chunk);
            file.write(&chunk).await?;
        }
        if magic != b"PK\x03\x04" && magic != b"PK\x05\x06" {
            return Err(bad("Upload a ZIP file containing your mod"));
        }
        file.finish().await?;
        tokio::fs::rename(&partial, &path).await?;
        Ok((size, hex::encode(hash.finalize())))
    }
    .await;
    let (size, hash) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(error);
        }
    };
    let inserted: rusqlite::Result<()> = (|| {
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO mods VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                owner,
                info.app_id,
                info.name,
                info.version,
                info.description,
                hash,
                size as i64
            ],
        )?;
        tx.execute("INSERT INTO mod_reviews(mod_id) VALUES(?1)", [&id])?;
        tx.commit()
    })();
    if let Err(error) = inserted {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error.into());
    }
    Ok(axum::Json(
        json!({"id":id,"sha256":hash,"size":size,"review_status":"pending"}),
    ))
}
async fn mods(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt = db.prepare("SELECT m.id,m.app_id,m.name,m.version,m.description,m.sha256,m.size,u.username FROM mods m JOIN users u ON m.user_id=u.id ORDER BY m.name")?;
    let entries = stmt.query_map([], |r|Ok(json!({"id":r.get::<_,String>(0)?,"app_id":r.get::<_,u32>(1)?,"name":r.get::<_,String>(2)?,"version":r.get::<_,String>(3)?,"description":r.get::<_,String>(4)?,"sha256":r.get::<_,String>(5)?,"size":r.get::<_,i64>(6)?,"author":r.get::<_,String>(7)?})))?.collect::<Result<Vec<_>,_>>()?;
    let entries: Vec<Value> = entries
        .into_iter()
        .map(|mut m| {
            let details =
                external::details(&db, m["id"].as_str().unwrap()).unwrap_or_else(|_| json!({}));
            m["details"] = details;
            m["review_status"] = json!(
                if security::approved(&db, m["id"].as_str().unwrap()).is_ok() {
                    "approved"
                } else {
                    "pending"
                }
            );
            m
        })
        .collect();
    Ok(axum::Json(json!(entries)))
}
async fn download(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    app.auth(&headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    let size: Option<i64> = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT size FROM mods WHERE id=?1", [&id], |r| r.get(0))
        .optional()?;
    let size = size.ok_or(ApiError(StatusCode::NOT_FOUND, "Mod not found"))?;
    security::approved(&app.db.lock().unwrap(), &id)?;
    let file = tokio::fs::File::open(app.files.join(format!("{id}.zip"))).await?;
    Ok((
        [
            ("content-type", "application/zip".to_owned()),
            ("content-length", size.to_string()),
            (
                "content-disposition",
                format!(
                    "attachment; filename=\"{}\"",
                    external::filename(&app, &id)?
                ),
            ),
        ],
        Body::from_stream(crypto::read(file, Zeroizing::new(*app.upload_key), id)),
    )
        .into_response())
}
async fn delete_mod(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let (owner, admin) = app.auth(&headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    let changed = app.db.lock().unwrap().execute(
        "DELETE FROM mods WHERE id=?1 AND (user_id=?2 OR ?3)",
        params![id, owner, admin],
    )?;
    if changed == 0 {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Mod not found or not owned by you",
        ));
    }
    tokio::fs::remove_file(app.files.join(format!("{id}.zip"))).await?;
    Ok(StatusCode::NO_CONTENT)
}
fn check_pack(manifest: &Value) -> ApiResult<&str> {
    if manifest["format"] != "canna_modpack" || manifest["schema_version"] != 1 {
        return Err(bad("Choose a Canna .canna.json modpack export"));
    }
    let name = manifest["name"]
        .as_str()
        .filter(|v| !v.trim().is_empty() && v.len() <= 100)
        .ok_or_else(|| bad("Invalid modpack name"))?;
    let mods = manifest["mods"]
        .as_array()
        .filter(|m| m.len() <= 200)
        .ok_or_else(|| bad("Invalid mods list"))?;
    if mods
        .iter()
        .any(|m| m["local_file"].as_str().is_some_and(|s| !s.is_empty()))
    {
        return Err(bad(
            "This pack contains local files. Share repository mods until bundled file sharing is implemented.",
        ));
    }
    if manifest["game"]["app_id"].as_u64().unwrap_or_default() == 0 {
        return Err(bad("Modpack must specify a Steam game"));
    }
    Ok(name)
}
async fn share(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(manifest): axum::Json<Value>,
) -> ApiResult<axum::Json<Value>> {
    let (owner, _) = app.auth(&headers)?;
    let name = check_pack(&manifest)?;
    let id = Uuid::new_v4().to_string();
    let db = app.db.lock().unwrap();
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM packs WHERE user_id=?1",
        [owner],
        |r| r.get(0),
    )?;
    if count >= 200 {
        return Err(bad("You can share up to 200 packs"));
    }
    db.execute(
        "INSERT INTO packs VALUES(?1,?2,?3,?4)",
        params![id, owner, name, manifest.to_string()],
    )?;
    Ok(axum::Json(
        json!({"id":id,"url":format!("https://cannamods.vip/packs/{id}")}),
    ))
}
async fn packs(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt = db.prepare(
        "SELECT p.id,p.name,u.username,p.manifest FROM packs p JOIN users u ON p.user_id=u.id ORDER BY p.name",
    )?;
    let entries = stmt.query_map([], |r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"author":r.get::<_,String>(2)?,"game":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or(Value::Null)["game"].clone()})))?.collect::<Result<Vec<_>,_>>()?;
    Ok(axum::Json(json!(entries)))
}
async fn pack(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    app.auth(&headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid pack ID"))?;
    let manifest: String = app
        .db
        .lock()
        .unwrap()
        .query_row("SELECT manifest FROM packs WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "Modpack not found"))?;
    Ok((
        [
            ("content-type", "application/json"),
            (
                "content-disposition",
                "attachment; filename=\"Shared-Modpack.canna.json\"",
            ),
        ],
        manifest,
    )
        .into_response())
}
async fn delete_pack(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    let (owner, admin) = app.auth(&headers)?;
    let changed = app.db.lock().unwrap().execute(
        "DELETE FROM packs WHERE id=?1 AND (user_id=?2 OR ?3)",
        params![id, owner, admin],
    )?;
    if changed == 0 {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Modpack not found or not owned by you",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}
fn asset(source: &'static str) -> Response {
    (
        [
            ("content-type", "text/javascript; charset=utf-8"),
            ("cache-control", "no-store"),
            ("vary", "Cookie, Authorization"),
        ],
        source,
    )
        .into_response()
}
async fn community_page(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let source = if app.auth(&headers).is_ok() {
        include_str!("../web/index.html")
    } else {
        include_str!("../web/login.html")
    };
    (
        [
            ("cache-control", "no-store"),
            ("vary", "Cookie, Authorization"),
        ],
        Html(source),
    )
        .into_response()
}
async fn connect_page(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let source = if app.auth(&headers).is_ok() {
        include_str!("../web/connect.html")
    } else {
        include_str!("../web/login.html")
    };
    (
        [
            ("cache-control", "no-store"),
            ("vary", "Cookie, Authorization"),
        ],
        Html(source),
    )
        .into_response()
}
async fn connect_script(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    Ok(asset(include_str!("../web/connect.js")))
}
async fn app_script(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    Ok(asset(include_str!("../web/app.js")))
}
async fn community_script(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    Ok(asset(include_str!("../web/community.js")))
}
async fn profiles_script(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    Ok(asset(include_str!("../web/profiles.js")))
}
async fn sections_script(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    Ok(asset(include_str!("../web/sections.js")))
}
async fn live_script(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    Ok(asset(include_str!("../web/live.js")))
}
async fn library_script(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<Response> {
    app.auth(&headers)?;
    Ok(asset(include_str!("../web/library.js")))
}
fn router(app: Shared) -> Router {
    Router::new()
        .route("/", get(community_page))
        .route(
            "/help",
            get(|| async {
                (
                    [("cache-control", "no-store")],
                    Html(include_str!("../web/help.html")),
                )
            }),
        )
        .route(
            "/help.css",
            get(|| async {
                (
                    [
                        ("content-type", "text/css; charset=utf-8"),
                        ("cache-control", "no-store"),
                    ],
                    include_str!("../web/help.css"),
                )
            }),
        )
        .route("/packs/{id}", get(community_page))
        .route("/connect", get(connect_page))
        .route("/connect.js", get(connect_script))
        .route(
            "/login.js",
            get(|| async { asset(include_str!("../web/login.js")) }),
        )
        .route("/app.js", get(app_script))
        .route("/live.js", get(live_script))
        .route("/library.js", get(library_script))
        .route("/api/v1/events", get(live::events))
        .route("/updates/latest", get(updates::latest))
        .route("/updates/{version}", get(updates::binary))
        .route(
            "/forum.css",
            get(|| async {
                (
                    [("content-type", "text/css; charset=utf-8")],
                    include_str!("../web/forum.css"),
                )
            }),
        )
        .route("/community.js", get(community_script))
        .route("/profiles.js", get(profiles_script))
        .route("/sections.js", get(sections_script))
        .route(
            "/health",
            get(|| async {
                axum::Json(
                    json!({"service":"canna","version":env!("CARGO_PKG_VERSION"),"status":"ok"}),
                )
            }),
        )
        .route("/api/v1/register", post(email::register))
        .route("/api/v1/verify-email", post(email::verify))
        .route("/api/v1/resend-verification", post(email::resend))
        .route("/api/v1/forgot-password", post(email::forgot))
        .route("/api/v1/reset-password", post(email::reset))
        .route(
            "/api/v1/auth-status",
            get(|State(app): State<Shared>| async move {
                axum::Json(json!({"email_ready":app.mail.ready()}))
            }),
        )
        .route("/api/v1/login", post(login))
        .route("/api/v1/login/verify", post(twofactor::verify))
        .route(
            "/api/v1/trusted-devices",
            axum::routing::delete(twofactor::revoke),
        )
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/me", get(me))
        .route("/api/v1/devices", get(devices::list))
        .route(
            "/api/v1/devices/logout-others",
            post(devices::revoke_others),
        )
        .route(
            "/api/v1/devices/{id}",
            post(devices::rename).delete(devices::revoke),
        )
        .route("/api/v1/profiles", get(profiles::directory))
        .route("/api/v1/profiles/me", post(profiles::update))
        .route(
            "/api/v1/profiles/me/avatar",
            post(profiles::upload_avatar).layer(DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route("/api/v1/profiles/{id}", get(profiles::profile))
        .route("/api/v1/profiles/{id}/avatar", get(profiles::avatar))
        .route("/api/v1/profiles/{id}/comments", post(profiles::comment))
        .route("/api/v1/profiles/{id}/rating", post(profiles::rate))
        .route(
            "/api/v1/profile-comments/{id}",
            axum::routing::delete(profiles::delete_comment),
        )
        .route("/api/v1/sections", get(sections::list))
        .route("/api/v1/admin/sections/review", post(sections::review))
        .route("/api/v1/admin/sections/apply", post(sections::apply))
        .route("/api/v1/admin/users", get(community::users))
        .route("/api/v1/admin/users/{id}/ban", post(community::ban))
        .route("/api/v1/admin/users/{id}/role", post(community::set_role))
        .route(
            "/api/v1/admin/transfer-owner",
            post(community::transfer_owner),
        )
        .route("/api/v1/admin/audit", get(community::audit))
        .route("/api/v1/mods/{id}/source", get(community::source))
        .route(
            "/api/v1/topics",
            get(community::topics).post(community::new_topic),
        )
        .route(
            "/api/v1/topics/{id}",
            get(community::topic).delete(community::delete_topic),
        )
        .route("/api/v1/topics/{id}/reply", post(community::reply))
        .route("/api/v1/topics/{id}/moderate", post(community::moderate))
        .route(
            "/api/v1/posts/{id}",
            axum::routing::delete(community::delete_post),
        )
        .route("/api/v1/invites", post(invite))
        .route("/api/v1/invite-waves", post(invite_wave))
        .route(
            "/api/v1/invite-waves/{id}",
            axum::routing::delete(revoke_wave),
        )
        .route(
            "/api/v1/mods",
            get(mods)
                .post(upload)
                .layer(DefaultBodyLimit::max(UPLOAD_LIMIT)),
        )
        .route("/api/v1/catalog", get(catalog::list))
        .route("/api/v1/catalog/file", get(catalog::file))
        .route("/api/v1/mods/external/preview", post(external::preview))
        .route("/api/v1/mods/external/import", post(external::import))
        .route("/api/v1/desktop/connect", post(handoff::connect))
        .route("/api/v1/desktop/start", post(handoff::pair_start))
        .route("/api/v1/desktop/approve", post(handoff::pair_approve))
        .route("/api/v1/desktop/poll", post(handoff::pair_poll))
        .route("/api/v1/desktop/claim", post(handoff::connect_claim))
        .route("/api/v1/download-tickets", post(handoff::create))
        .route("/api/v1/download-tickets/claim", post(handoff::claim))
        .route("/api/v1/download-tickets/transfer", post(handoff::transfer))
        .route("/api/v1/download-tickets/complete", post(handoff::complete))
        .route("/api/v1/download-tickets/{id}", get(handoff::status))
        .route("/api/v1/mods/{id}/approve", post(security::approve))
        .route("/api/v1/mods/{id}", get(download).delete(delete_mod))
        .route("/api/v1/packs", get(packs).post(share))
        .route("/api/v1/packs/{id}", get(pack).delete(delete_pack))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            security::protect,
        ))
        .layer(middleware::from_fn(origin_check))
        .layer(middleware::from_fn_with_state(app.clone(), live::publish))
        .with_state(app)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state =
        PathBuf::from(std::env::var("CANNA_STATE").unwrap_or_else(|_| "/var/lib/canna".into()));
    let files = PathBuf::from(
        std::env::var("CANNA_UPLOADS").unwrap_or_else(|_| "/mnt/canna/uploads".into()),
    );
    std::fs::create_dir_all(&state)?;
    let key = credential("database.key")?;
    let upload_key: [u8; 32] = hex::decode(credential("uploads.key")?.trim())?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid uploads key"))?;
    let database = state.join("accounts.sqlite");
    if std::env::args().any(|a| a == "--encrypt-existing") {
        encrypt_existing(&database, key.trim())?;
    }
    let app = Arc::new(App::open(
        &database,
        files,
        key.trim(),
        upload_key,
        email::Mailer::configured()?,
    )?);
    if let Some(path) = std::env::args()
        .skip_while(|a| a != "--import-catalog")
        .nth(1)
    {
        external::catalog(&app, std::path::Path::new(&path)).await?;
        let assets = std::path::Path::new(&path).with_file_name("assets-import.json");
        if assets.exists() {
            catalog::migrate(&app, &assets).await?;
        }
        return Ok(());
    }
    // Import the pre-account bootstrap invitation into encrypted storage.
    let bootstrap = state.join("first-admin-invite.txt");
    if bootstrap.exists() {
        let raw = Zeroizing::new(std::fs::read_to_string(&bootstrap)?);
        app.db.lock().unwrap().execute(
            "INSERT OR REPLACE INTO secrets VALUES('first-admin-invite',?1)",
            [raw.trim()],
        )?;
        std::fs::remove_file(&bootstrap)?;
    }
    {
        let db = app.db.lock().unwrap();
        let users: i64 = db.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        let invites: i64 = db.query_row(
            "SELECT COUNT(*) FROM invites WHERE admin=1 AND expires>?1",
            [now()],
            |r| r.get(0),
        )?;
        if users == 0 && invites == 0 {
            let raw = token();
            db.execute(
                "INSERT INTO invites(hash,admin,expires) VALUES(?1,1,?2)",
                params![digest(&raw), now() + 7 * 86400],
            )?;
            db.execute(
                "INSERT OR REPLACE INTO secrets VALUES('first-admin-invite',?1)",
                [raw],
            )?;
        }
    }
    if std::env::args().any(|a| a == "--audit-catalog") {
        catalog::audit(&app).await?;
        return Ok(());
    }
    if std::env::args().any(|a| a == "--check-storage") {
        let integrity: String =
            app.db
                .lock()
                .unwrap()
                .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        anyhow::ensure!(integrity == "ok", "Database integrity check failed");
        println!("Encrypted storage verified");
        return Ok(());
    }
    if std::env::args().any(|a| a == "--print-first-invite") {
        let raw: String = app.db.lock().unwrap().query_row(
            "SELECT value FROM secrets WHERE name='first-admin-invite'",
            [],
            |r| r.get(0),
        )?;
        println!("{raw}");
        return Ok(());
    }
    let address = std::env::var("CANNA_LISTEN").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Canna server listening on {address}");
    axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}
fn credential(name: &str) -> anyhow::Result<Zeroizing<String>> {
    let directory =
        std::env::var("CREDENTIALS_DIRECTORY").unwrap_or_else(|_| "/etc/canna/keys".into());
    Ok(Zeroizing::new(std::fs::read_to_string(
        PathBuf::from(directory).join(name),
    )?))
}
fn encrypt_existing(path: &std::path::Path, key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid database key"
    );
    let bytes = std::fs::read(path)?;
    if !bytes.starts_with(b"SQLite format 3\0") {
        return Ok(());
    }
    let db = Connection::open(path)?;
    // This migration was requested before accounts or uploads existed. Fail
    // rather than discard or transform a populated production database.
    for table in ["users", "mods", "packs"] {
        let count: i64 =
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
        anyhow::ensure!(
            count == 0,
            "Populated storage requires a separate audited migration"
        );
    }
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
    let encrypted = path.with_extension("encrypted");
    anyhow::ensure!(
        !encrypted.exists(),
        "An encrypted migration already exists; inspect it before retrying"
    );
    db.execute(
        "ATTACH DATABASE ?1 AS encrypted KEY ?2",
        params![encrypted.to_string_lossy(), format!("x'{key}'")],
    )?;
    db.execute_batch("SELECT sqlcipher_export('encrypted'); DETACH DATABASE encrypted;")?;
    drop(db);
    // Validate the key and complete schema before replacing the original.
    let check = Connection::open(&encrypted)?;
    check.execute_batch(&format!("PRAGMA key=\"x'{key}'\";"))?;
    check.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    drop(check);
    std::fs::rename(encrypted, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    async fn page_text(response: Response) -> String {
        String::from_utf8(
            axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn catalog_includes_unity_and_minecraft_but_requires_membership() {
        let (_dir, app) = fixture();
        let token = account(&app, "catalog-reader", false);
        {
            let db = app.db.lock().unwrap();
            let user: i64 = db
                .query_row(
                    "SELECT id FROM users WHERE username IS NOT NULL LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            for (id, game, name) in [
                ("catalog-unity", 1686940, "Unity mod"),
                ("catalog-minecraft", 0, "Minecraft shader"),
            ] {
                db.execute(
                    "INSERT INTO mods VALUES(?1,?2,?3,?4,'1','Test catalog','hash',1)",
                    params![id, user, game, name],
                )
                .unwrap();
            }
            db.execute("INSERT INTO mod_details(mod_id,origin,data) VALUES('catalog-minecraft','test:shader',?1)",[r#"{"game":"Minecraft","content_type":"shader"}"#]).unwrap();
        }
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/catalog", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = call(app, "GET", "/api/v1/catalog", Value::Null, Some(&token)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let text = page_text(response).await;
        let data: Value = serde_json::from_str(&text).unwrap();
        let games = data["games"].as_array().unwrap();
        assert_eq!(games.len(), 2);
        let mc = games.iter().find(|g| g["app_id"] == u32::MAX).unwrap();
        assert_eq!(mc["name"], "Minecraft");
        assert_eq!(mc["mods"][0]["content_type"], "shader");
    }
    #[tokio::test]
    async fn help_is_public_without_exposing_member_content() {
        let (_dir, app) = fixture();
        let member = account(&app, "help-reader", false);
        for session in [None, Some(member.as_str())] {
            let response = call(app.clone(), "GET", "/help", Value::Null, session).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let text = page_text(response).await;
            for expected in [
                "What is Canna?",
                "Current features",
                "Frequently asked questions",
                "Minecraft API access approval",
            ] {
                assert!(text.contains(expected));
            }
            for private in [
                "id=\"space\"",
                "id=\"forumview\"",
                "/app.js",
                "/community.js",
                "help-reader",
            ] {
                assert!(!text.contains(private));
            }
            let home = page_text(call(app.clone(), "GET", "/", Value::Null, session).await).await;
            assert!(home.contains("href=\"/help\">Help / FAQ"));
        }
        assert_eq!(
            call(app.clone(), "GET", "/help.css", Value::Null, None)
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            call(app, "GET", "/api/v1/catalog", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[tokio::test]
    async fn community_markup_and_scripts_are_only_served_to_valid_members() {
        let (_dir, app) = fixture();
        let member = account(&app, "member", false);
        for path in ["/", "/packs/example"] {
            let response = call(app.clone(), "GET", path, Value::Null, None).await;
            assert_eq!(response.headers()["cache-control"], "no-store");
            let text = page_text(response).await;
            assert!(text.contains("id=\"auth\""));
            for private in [
                "id=\"space\"",
                "id=\"moderation\"",
                "id=\"forumview\"",
                "id=\"upload\"",
                "/community.js",
                "/app.js",
            ] {
                assert!(!text.contains(private), "Anonymous page contains {private}");
            }
            let response = call(app.clone(), "GET", path, Value::Null, Some(&member)).await;
            assert_eq!(response.headers()["cache-control"], "no-store");
            let text = page_text(response).await;
            assert!(text.contains("id=\"forumview\""));
            assert!(!text.contains("id=\"auth\""));
        }
        let anonymous =
            page_text(call(app.clone(), "GET", "/connect", Value::Null, None).await).await;
        assert!(anonymous.contains("id=\"auth\""));
        assert!(!anonymous.contains("connectionform"));
        let connected =
            page_text(call(app.clone(), "GET", "/connect", Value::Null, Some(&member)).await).await;
        assert!(connected.contains("connectionform"));
        assert!(!connected.contains("forumview"));
        for path in [
            "/app.js",
            "/community.js",
            "/profiles.js",
            "/sections.js",
            "/live.js",
            "/connect.js",
        ] {
            assert_eq!(
                call(app.clone(), "GET", path, Value::Null, None)
                    .await
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            let response = call(app.clone(), "GET", path, Value::Null, Some(&member)).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
        assert_eq!(
            call(app.clone(), "GET", "/login.js", Value::Null, None)
                .await
                .status(),
            StatusCode::OK
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET expires=0", [])
            .unwrap();
        let text = page_text(call(app.clone(), "GET", "/", Value::Null, Some(&member)).await).await;
        assert!(text.contains("id=\"auth\""));
        assert!(!text.contains("id=\"space\""));
        assert_eq!(
            call(app, "GET", "/app.js", Value::Null, Some(&member))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    pub(super) fn fixture() -> (tempfile::TempDir, Shared) {
        let dir = tempfile::tempdir().unwrap();
        let app = Arc::new(
            App::open(
                &dir.path().join("db.sqlite"),
                dir.path().join("files"),
                &"23".repeat(32),
                [23; 32],
                email::Mailer::Test(Mutex::new(Vec::new())),
            )
            .unwrap(),
        );
        (dir, app)
    }
    pub(super) async fn call(
        app: Shared,
        method: &str,
        path: &str,
        body: Value,
        auth: Option<&str>,
    ) -> Response {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(auth) = auth {
            request = request.header("authorization", format!("Bearer {auth}"));
        }
        router(app)
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }
    pub(super) async fn value(response: Response) -> Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap()
    }
    #[tokio::test]
    async fn private_content_requires_authentication() {
        let (_dir, app) = fixture();
        for path in [
            "/api/v1/mods",
            "/api/v1/packs",
            "/api/v1/me",
            "/api/v1/packs/../../etc/passwd",
        ] {
            assert_ne!(
                call(app.clone(), "GET", path, Value::Null, None)
                    .await
                    .status(),
                StatusCode::OK
            );
        }
    }
    #[test]
    fn database_requires_key_and_hides_identity() {
        let (dir, app) = fixture();
        app.db.lock().unwrap().execute("INSERT INTO users(username,password,email) VALUES('sensitive-family-name','password-hash','private@example.com')", []).unwrap();
        app.db
            .lock()
            .unwrap()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        let bytes = std::fs::read(dir.path().join("db.sqlite")).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3"));
        for needle in [b"sensitive-family-name".as_slice(), b"private@example.com"] {
            assert!(!bytes.windows(needle.len()).any(|w| w == needle));
        }
        let plain = Connection::open(dir.path().join("db.sqlite")).unwrap();
        assert!(
            plain
                .query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
                .is_err()
        );
        plain
            .execute_batch(&format!("PRAGMA key=\"x'{}'\"", "ff".repeat(32)))
            .unwrap();
        assert!(
            plain
                .query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
                .is_err()
        );
    }
    #[test]
    fn pre_account_migration_preserves_invites_and_refuses_populated_storage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE users(id INTEGER); CREATE TABLE mods(id TEXT); CREATE TABLE packs(id TEXT); CREATE TABLE invites(hash TEXT);
            INSERT INTO invites VALUES('existing-invitation-hash');").unwrap();
        drop(db);
        let key = "23".repeat(32);
        encrypt_existing(&path, &key).unwrap();
        let db = Connection::open(&path).unwrap();
        db.execute_batch(&format!("PRAGMA key=\"x'{key}'\";"))
            .unwrap();
        assert_eq!(
            db.query_row("SELECT hash FROM invites", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "existing-invitation-hash"
        );
        assert!(
            !std::fs::read(&path)
                .unwrap()
                .starts_with(b"SQLite format 3")
        );
        let populated = dir.path().join("populated.sqlite");
        let db = Connection::open(&populated).unwrap();
        db.execute_batch("CREATE TABLE users(id INTEGER); CREATE TABLE mods(id TEXT); CREATE TABLE packs(id TEXT); INSERT INTO users VALUES(1);").unwrap();
        drop(db);
        assert!(encrypt_existing(&populated, &key).is_err());
        assert!(
            std::fs::read(&populated)
                .unwrap()
                .starts_with(b"SQLite format 3")
        );
    }
    #[tokio::test]
    async fn cookie_mutations_require_same_origin() {
        let (_dir, app) = fixture();
        let auth = account(&app, "family", true);
        for (origin, expected) in [
            ("https://evil.example", StatusCode::FORBIDDEN),
            ("https://cannamods.vip", StatusCode::NO_CONTENT),
        ] {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/logout")
                .header("cookie", format!("canna_session={auth}"))
                .header("origin", origin)
                .body(Body::empty())
                .unwrap();
            assert_eq!(
                router(app.clone()).oneshot(request).await.unwrap().status(),
                expected
            );
        }
    }
    #[tokio::test]
    async fn invites_are_single_use_and_logout_revokes_access() {
        let (_dir, app) = fixture();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO invites(hash,admin,expires) VALUES(?1,1,?2)",
                params![digest("fixture"), now() + 60],
            )
            .unwrap();
        let input = json!({"username":"family","password":"testing-password-123","email":"family@example.com","invite":"fixture"});
        let response = call(app.clone(), "POST", "/api/v1/register", input.clone(), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let pending = value(response).await;
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/login", input.clone(), None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let code = match &app.mail {
            email::Mailer::Test(outbox) => outbox.lock().unwrap()[0].1.clone(),
            _ => panic!("Test mail required"),
        };
        let response = call(
            app.clone(),
            "POST",
            "/api/v1/verify-email",
            json!({"challenge":pending["challenge"],"code":code}),
            None,
        )
        .await;
        assert!(
            response.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .contains("HttpOnly; Secure; SameSite=Strict")
        );
        let session = value(response).await;
        let auth = session["token"].as_str().unwrap();
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/login",
                json!({"username":"family","password":"wrong-password-123"}),
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let login = call(app.clone(), "POST", "/api/v1/login", input.clone(), None).await;
        assert_eq!(login.status(), StatusCode::OK);
        let login = value(login).await;
        assert_eq!(login["two_factor_required"], true);
        assert!(login.get("token").is_none());
        assert_eq!(
            value(call(app.clone(), "GET", "/api/v1/me", Value::Null, Some(auth)).await).await["admin"],
            true
        );
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/register", input, None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/logout",
                Value::Null,
                Some(auth)
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(app, "GET", "/api/v1/me", Value::Null, Some(auth))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    #[test]
    fn shared_packs_reject_unbundled_local_files() {
        let manifest = json!({"format":"canna_modpack","schema_version":1,"name":"Family","game":{"app_id":1686940},"mods":[{"local_file":"private.dll"}]});
        assert!(check_pack(&manifest).is_err());
    }
    pub(super) fn account(app: &App, name: &str, admin: bool) -> String {
        let id = {
            let db = app.db.lock().unwrap();
            db.execute(
                "INSERT INTO users(username,password,admin,verified,role) VALUES(?1,'fixture',?2,1,?3)",
                params![name, admin, if admin {"owner"} else {"member"}],
            )
            .unwrap();
            db.last_insert_rowid()
        };
        app.session(id, &HeaderMap::new()).unwrap().0["token"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    #[tokio::test]
    async fn member_invites_are_limited_and_admin_waves_can_be_revoked() {
        let (_dir, app) = fixture();
        let member = account(&app, "member", false);
        let admin = account(&app, "admin", true);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/invites",
                json!({}),
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
                "/api/v1/invites",
                json!({}),
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
                "/api/v1/invite-waves",
                json!({"count":3}),
                Some(&member)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let wave = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/invite-waves",
                json!({"count":3}),
                Some(&admin),
            )
            .await,
        )
        .await;
        assert_eq!(wave["invites"].as_array().unwrap().len(), 3);
        let response = call(
            app.clone(),
            "DELETE",
            &format!("/api/v1/invite-waves/{}", wave["wave"].as_str().unwrap()),
            Value::Null,
            Some(&admin),
        )
        .await;
        assert_eq!(value(response).await["revoked"], 3);
        let registration = json!({"username":"newfriend","password":"testing-password-123","invite":wave["invites"][0]});
        assert_eq!(
            call(app, "POST", "/api/v1/register", registration, None)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn uploads_round_trip_and_only_owners_can_delete() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", false);
        let other = account(&app, "other", false);
        let bytes = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/v1/mods?app_id=1686940&name=Fixture&version=1.0")
            .header("authorization", format!("Bearer {owner}"))
            .body(Body::from(bytes.as_slice()))
            .unwrap();
        let response = router(app.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let item = value(response).await;
        assert_eq!(item["sha256"], hex::encode(Sha256::digest(bytes)));
        let path = format!("/api/v1/mods/{}", item["id"].as_str().unwrap());
        let response = call(app.clone(), "GET", &path, Value::Null, Some(&other)).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let catalog = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/catalog",
                Value::Null,
                Some(&other),
            )
            .await,
        )
        .await;
        assert!(catalog["games"].as_array().unwrap().is_empty());
        assert_eq!(
            call(
                app.clone(),
                "GET",
                &format!(
                    "/api/v1/catalog/file?path=Mods/{}.zip",
                    item["id"].as_str().unwrap()
                ),
                Value::Null,
                Some(&other)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets",
                json!({"id":item["id"],"kind":"mods"}),
                Some(&other)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let approve_path = format!("{path}/approve");
        assert_eq!(
            call(app.clone(), "POST", &approve_path, json!({}), Some(&other))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let admin = account(&app, "reviewer", true);
        assert_eq!(
            call(app.clone(), "POST", &approve_path, json!({}), Some(&admin))
                .await
                .status(),
            StatusCode::OK
        );
        let response = call(app.clone(), "GET", &path, Value::Null, Some(&other)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        assert_eq!(
            call(app.clone(), "DELETE", &path, Value::Null, Some(&other))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(app.clone(), "DELETE", &path, Value::Null, Some(&owner))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(std::fs::read_dir(&app.files).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn shared_manifest_round_trip_is_private_and_owner_controlled() {
        let (_dir, app) = fixture();
        let owner = account(&app, "owner", false);
        let other = account(&app, "other", false);
        let manifest = json!({"format":"canna_modpack","schema_version":1,"name":"Family","game":{"app_id":1686940},"mods":[]});
        let result = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/packs",
                manifest.clone(),
                Some(&owner),
            )
            .await,
        )
        .await;
        let path = format!("/api/v1/packs/{}", result["id"].as_str().unwrap());
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            value(call(app.clone(), "GET", &path, Value::Null, Some(&other)).await).await,
            manifest
        );
        assert_eq!(
            call(app.clone(), "DELETE", &path, Value::Null, Some(&other))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(app, "DELETE", &path, Value::Null, Some(&owner))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }
}
