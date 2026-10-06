use super::*;
use reqwest::Url;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS mod_details(mod_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE,origin TEXT UNIQUE NOT NULL,data TEXT NOT NULL);")
}
#[derive(Deserialize)]
pub struct Link {
    pub url: String,
    #[serde(default)]
    pub version: String,
}
#[derive(Clone, serde::Serialize, Deserialize)]
pub struct Release {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub dependencies: Value,
    #[serde(skip_serializing)]
    pub download: String,
    #[serde(skip_serializing)]
    pub hash: String,
    #[serde(skip_serializing)]
    pub algorithm: String,
}
#[derive(serde::Serialize)]
pub struct Project {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_url: String,
    pub game: String,
    pub authors: String,
    pub license: String,
    pub versions: Vec<Release>,
}
fn slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn link(raw: &str) -> ApiResult<(String, Vec<String>)> {
    let url = Url::parse(raw).map_err(|_| bad("Paste a complete HTTPS project link"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(bad("Use an HTTPS Modrinth or CurseForge project link"));
    }
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.");
    let parts: Vec<String> = url
        .path_segments()
        .unwrap()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if !parts.iter().all(|s| slug(s)) {
        return Err(bad("Invalid project link"));
    }
    if host == "modrinth.com"
        && parts.len() >= 2
        && matches!(
            parts[0].as_str(),
            "mod" | "resourcepack" | "shader" | "datapack"
        )
    {
        return Ok(("modrinth".into(), parts));
    }
    if host == "thunderstore.io" && parts.len() >= 5 && parts[0] == "c" && parts[2] == "p" {
        return Ok(("thunderstore".into(), parts));
    }
    if host == "curseforge.com"
        && parts.len() >= 3
        && matches!(parts[1].as_str(), "mc-mods" | "mods" | "addons")
    {
        return Ok(("curseforge".into(), parts));
    }
    Err(bad(
        "Use a mod project link from thunderstore.io, modrinth.com or curseforge.com",
    ))
}
fn client() -> ApiResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("Canna/0.3.7 (https://cannamods.vip)")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|_| bad("Could not connect to the provider"))
}
async fn metadata(url: &str, cf: bool) -> ApiResult<Value> {
    let mut response = if cf {
        curseforge::get(url).await?
    } else {
        client()?
            .get(url)
            .send()
            .await
            .map_err(|_| bad("Provider unavailable; try again shortly"))?
    };
    if !response.status().is_success() {
        return Err(bad(
            "Provider could not find this project or authorize the request",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| bad("Provider response interrupted"))?
    {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err(bad("Provider response too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| bad("Invalid provider response"))
}
fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}
pub fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(160)
        .collect();
    cleaned.trim_start_matches('.').to_owned()
}
pub async fn resolve(raw: &str) -> ApiResult<Project> {
    let (provider, parts) = link(raw)?;
    if provider == "thunderstore" {
        if steam_id(&parts[1]).is_none() {
            return Err(bad(
                "This Thunderstore community is not a supported Steam/Unity game",
            ));
        }
        let p = metadata(
            &format!(
                "https://thunderstore.io/api/experimental/package/{}/{}/",
                parts[3], parts[4]
            ),
            false,
        )
        .await?;
        if !p["community_listings"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v["community"] == parts[1]))
        {
            return Err(bad("This mod is not listed for the selected game"));
        }
        if p["is_deprecated"] == true || p["latest"]["is_active"] != true {
            return Err(bad("This mod is deprecated or unavailable"));
        }
        let latest = &p["latest"];
        let version = text(latest, "version_number");
        let game = match parts[1].as_str() {
            "bopl-battle" => "Bopl Battle",
            "rounds" => "ROUNDS",
            "riskofrain2" => "Risk of Rain 2",
            "lethal-company" => "Lethal Company",
            "content-warning" => "Content Warning",
            other => other,
        };
        Ok(Project {
            provider,
            id: format!("{}-{}", parts[3], parts[4]),
            name: text(&p, "name"),
            description: text(latest, "description"),
            source_url: format!(
                "https://thunderstore.io/c/{}/p/{}/{}/",
                parts[1], parts[3], parts[4]
            ),
            game: game.into(),
            authors: text(&p, "owner"),
            license: "See license in mod archive and original project".into(),
            versions: vec![Release {
                id: version.clone(),
                name: version.clone(),
                filename: format!("{}-{}-{version}.zip", parts[3], parts[4]),
                loaders: vec!["BepInEx".into()],
                game_versions: vec!["Check author compatibility notes".into()],
                dependencies: latest["dependencies"].clone(),
                download: text(latest, "download_url"),
                hash: String::new(),
                algorithm: "https".into(),
            }],
        })
    } else if provider == "modrinth" {
        let project = metadata(
            &format!("https://api.modrinth.com/v2/project/{}", parts[1]),
            false,
        )
        .await?;
        let id = text(&project, "id");
        if !slug(&id) {
            return Err(bad("Invalid provider project"));
        }
        let kind = text(&project, "project_type");
        if !matches!(
            kind.as_str(),
            "mod" | "resourcepack" | "shader" | "datapack"
        ) {
            return Err(bad(
                "Choose a mod, data pack, shader or resource pack project",
            ));
        }
        let versions = metadata(
            &format!("https://api.modrinth.com/v2/project/{id}/version"),
            false,
        )
        .await?;
        let team = text(&project, "team");
        let authors = if slug(&team) {
            let members = metadata(
                &format!("https://api.modrinth.com/v2/team/{team}/members"),
                false,
            )
            .await?;
            members
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m["user"]["username"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        Ok(Project {
            provider,
            id,
            name: text(&project, "title"),
            description: text(&project, "description"),
            source_url: format!("https://modrinth.com/{}/{}", parts[0], parts[1]),
            game: "Minecraft".into(),
            authors,
            license: text(&project["license"], "id"),
            versions: versions
                .as_array()
                .into_iter()
                .flatten()
                .take(80)
                .filter_map(|v| {
                    let files = v["files"].as_array()?;
                    let f = files
                        .iter()
                        .find(|f| f["primary"] == true)
                        .or_else(|| files.first())?;
                    Some(Release {
                        id: text(v, "id"),
                        name: text(v, "version_number"),
                        filename: text(f, "filename"),
                        loaders: strings(&v["loaders"]),
                        game_versions: strings(&v["game_versions"]),
                        dependencies: v["dependencies"].clone(),
                        download: text(f, "url"),
                        hash: text(&f["hashes"], "sha512"),
                        algorithm: "sha512".into(),
                    })
                })
                .collect(),
        })
    } else {
        if steam_id(&parts[0]).is_none() {
            return Err(bad(
                "This external project is not for a supported Steam/Unity game",
            ));
        }
        let games = metadata("https://api.curseforge.com/v1/games?pageSize=50", true).await?;
        let game = games["data"]
            .as_array()
            .and_then(|a| a.iter().find(|g| g["slug"] == parts[0]))
            .ok_or_else(|| bad("This CurseForge game is not supported by its API"))?;
        let game_id = game["id"]
            .as_u64()
            .ok_or_else(|| bad("Invalid provider game"))?;
        let found = metadata(
            &format!(
                "https://api.curseforge.com/v1/mods/search?gameId={game_id}&slug={}",
                parts[2]
            ),
            true,
        )
        .await?;
        let project = found["data"]
            .as_array()
            .and_then(|a| a.iter().find(|m| m["slug"] == parts[2]))
            .ok_or_else(|| bad("CurseForge project not found"))?;
        if project["allowModDistribution"] == false {
            return Err(bad(
                "This author disabled third-party downloads. Use the original project page.",
            ));
        }
        let id = project["id"]
            .as_u64()
            .ok_or_else(|| bad("Invalid provider project"))?
            .to_string();
        let files = metadata(
            &format!("https://api.curseforge.com/v1/mods/{id}/files?pageSize=50"),
            true,
        )
        .await?;
        let releases = files["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|f| f["isAvailable"] == true)
            .map(|f| {
                let hash = f["hashes"]
                    .as_array()
                    .and_then(|a| a.iter().find(|h| h["algo"] == 1))
                    .map(|h| text(h, "value"))
                    .unwrap_or_default();
                Release {
                    id: f["id"].as_u64().unwrap_or_default().to_string(),
                    name: text(f, "displayName"),
                    filename: text(f, "fileName"),
                    loaders: strings(&f["gameVersions"])
                        .into_iter()
                        .filter(|v| matches!(v.as_str(), "Forge" | "Fabric" | "NeoForge" | "Quilt"))
                        .collect(),
                    game_versions: strings(&f["gameVersions"]),
                    dependencies: f["dependencies"].clone(),
                    download: text(f, "downloadUrl"),
                    hash,
                    algorithm: "sha1".into(),
                }
            })
            .collect();
        Ok(Project {
            provider,
            id,
            name: text(project, "name"),
            description: text(project, "summary"),
            source_url: format!(
                "https://www.curseforge.com/{}/{}/{}",
                parts[0], parts[1], parts[2]
            ),
            game: text(game, "name"),
            authors: project["authors"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|m| text(m, "name"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default(),
            license: "See original project".into(),
            versions: releases,
        })
    }
}
fn parts_type(url: &str) -> String {
    link(url)
        .ok()
        .map(|(_, p)| p[0].clone())
        .unwrap_or_else(|| "mod".into())
}
pub async fn preview(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Link>,
) -> ApiResult<axum::Json<Project>> {
    app.auth(&headers)?;
    let _permit = app
        .upload_gate
        .try_acquire()
        .map_err(|_| bad("Another import is in progress"))?;
    Ok(axum::Json(resolve(&input.url).await?))
}
fn artifact_url(raw: &str, provider: &str) -> ApiResult<Url> {
    let u = Url::parse(raw)
        .map_err(|_| bad("The author has not made this file available for third-party download"))?;
    let allowed = match provider {
        "modrinth" => u.host_str() == Some("cdn.modrinth.com") && u.path().starts_with("/data/"),
        "curseforge" => {
            matches!(
                u.host_str(),
                Some("edge.forgecdn.net" | "mediafilez.forgecdn.net")
            ) && u.path().starts_with("/files/")
        }
        "thunderstore" => {
            (u.host_str() == Some("thunderstore.io") && u.path().starts_with("/package/download/"))
                || (matches!(
                    u.host_str(),
                    Some("ccdn.thunderstore.io" | "gcdn.thunderstore.io")
                ) && u.path().starts_with("/live/repository/packages/"))
        }
        _ => false,
    };
    if !allowed
        || u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.query().is_some()
    {
        return Err(bad("Provider returned an unsupported download address"));
    }
    Ok(u)
}
pub async fn import(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Link>,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    let _permit = app
        .upload_gate
        .try_acquire()
        .map_err(|_| bad("Another import is in progress"))?;
    let project = resolve(&input.url).await?;
    let release = project
        .versions
        .iter()
        .find(|v| v.id == input.version)
        .ok_or_else(|| bad("Choose a version from the preview"))?;
    let origin = format!("{}:{}:{}", project.provider, project.id, release.id);
    if let Some(id) = existing(&app, &origin)? {
        return Ok(axum::Json(json!({"id":id,"existing":true})));
    }
    let url = artifact_url(&release.download, &project.provider)?;
    let mut response = if project.provider == "curseforge" {
        curseforge::get(url.as_str()).await?
    } else {
        client()?
            .get(url)
            .send()
            .await
            .map_err(|_| bad("Mod download failed"))?
    };
    if project.provider == "thunderstore" && response.status().is_redirection() {
        let target = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| bad("Invalid provider redirect"))?;
        response = client()?
            .get(artifact_url(target, "thunderstore")?)
            .send()
            .await
            .map_err(|_| bad("Mod download failed"))?;
    }
    if !response.status().is_success() {
        return Err(bad(
            "Provider refused the download. Check API access and author permissions.",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| bad("Mod download interrupted"))?
    {
        if bytes.len() + chunk.len() > UPLOAD_LIMIT {
            return Err(bad("Mods are limited to 128 MiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if release.hash.is_empty() && project.provider != "thunderstore" {
        return Err(bad("Provider did not supply a file checksum"));
    }
    let actual = if release.algorithm == "sha512" {
        hex::encode(sha2::Sha512::digest(&bytes))
    } else {
        hex::encode(sha1::Sha1::digest(&bytes))
    };
    if project.provider != "thunderstore" && !actual.eq_ignore_ascii_case(&release.hash) {
        return Err(bad("Provider file checksum mismatch"));
    }
    let mut details = serde_json::to_value(&project).unwrap();
    details.as_object_mut().unwrap().remove("versions");
    details["filename"] = json!(safe_filename(&release.filename));
    details["loaders"] = json!(release.loaders);
    details["game_versions"] = json!(release.game_versions);
    details["dependencies"] = release.dependencies.clone();
    details["content_type"] = json!(if project.provider == "modrinth" {
        parts_type(&input.url)
    } else {
        "mod".to_owned()
    });
    let (_, parts) = link(&input.url)?;
    let game = steam_id(
        &parts[if project.provider == "thunderstore" {
            1
        } else {
            0
        }],
    )
    .ok_or_else(|| bad("Unsupported Steam/Unity game"))?;
    let id = store(
        &app,
        user,
        game,
        &project.name,
        &release.name,
        &project.description,
        &origin,
        &details,
        &bytes,
    )
    .await?;
    Ok(axum::Json(json!({"id":id,"existing":false})))
}
fn steam_id(slug: &str) -> Option<u32> {
    match slug {
        "bopl-battle" => Some(1686940),
        "rounds" => Some(1557740),
        "valheim" => Some(892970),
        "kerbal-space-program" => Some(220200),
        "cities-skylines" => Some(255710),
        "risk-of-rain-2" => Some(632360),
        "riskofrain2" => Some(632360),
        "lethal-company" => Some(1966720),
        "content-warning" => Some(2881650),
        "minecraft" | "mod" | "resourcepack" | "datapack" | "shader" => Some(0),
        _ => None,
    }
}
fn existing(app: &App, origin: &str) -> ApiResult<Option<String>> {
    Ok(app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT mod_id FROM mod_details WHERE origin=?1",
            [origin],
            |r| r.get(0),
        )
        .optional()?)
}
#[allow(clippy::too_many_arguments)]
pub async fn store(
    app: &App,
    user: i64,
    game: u32,
    name: &str,
    version: &str,
    description: &str,
    origin: &str,
    details: &Value,
    bytes: &[u8],
) -> ApiResult<String> {
    if bytes.len() > UPLOAD_LIMIT
        || !(bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06"))
    {
        return Err(bad(
            "Only ZIP or JAR mod archives up to 128 MiB are supported",
        ));
    }
    let used: i64 =
        app.db
            .lock()
            .unwrap()
            .query_row("SELECT COALESCE(SUM(size),0) FROM mods", [], |r| r.get(0))?;
    if used + bytes.len() as i64 > STORAGE_LIMIT {
        return Err(bad("Mod storage is full"));
    }
    security::quota(&app.db.lock().unwrap(), user, bytes.len() as i64)?;
    let id = Uuid::new_v4().to_string();
    let path = app.files.join(format!("{id}.zip"));
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await?;
    let result: ApiResult<()> = async {
        let mut writer = crypto::Writer::new(file, &app.upload_key, id.clone()).await?;
        writer.write(bytes).await?;
        writer.finish().await?;
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO mods VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                user,
                game,
                name,
                version,
                description,
                hex::encode(Sha256::digest(bytes)),
                bytes.len() as i64
            ],
        )?;
        tx.execute(
            "INSERT INTO mod_details VALUES(?1,?2,?3)",
            params![id, origin, details.to_string()],
        )?;
        tx.execute("INSERT INTO mod_reviews(mod_id) VALUES(?1)", [&id])?;
        tx.commit()?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(path).await;
        return Err(e);
    }
    Ok(id)
}
pub fn details(db: &Connection, id: &str) -> ApiResult<Value> {
    let raw: Option<String> = db
        .query_row("SELECT data FROM mod_details WHERE mod_id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({})))
}
pub fn filename(app: &App, id: &str) -> ApiResult<String> {
    let d = details(&app.db.lock().unwrap(), id)?;
    Ok(d["filename"]
        .as_str()
        .map(safe_filename)
        .unwrap_or_else(|| format!("{id}.zip")))
}
pub async fn catalog(app: &App, manifest: &std::path::Path) -> anyhow::Result<()> {
    let input: Value = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let user: i64 =
        app.db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM users WHERE role='owner'", [], |r| r.get(0))?;
    let root = manifest.parent().unwrap().canonicalize()?;
    for m in input
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Expected a catalog array"))?
    {
        let path = root
            .join(m["local_file"].as_str().unwrap_or_default())
            .canonicalize()?;
        anyhow::ensure!(
            path.starts_with(&root),
            "Catalog file outside staging directory"
        );
        let origin = text(m, "origin");
        if existing(app, &origin)
            .map_err(|_| anyhow::anyhow!("Catalog database error"))?
            .is_some()
        {
            continue;
        }
        let bytes = std::fs::read(path)?;
        anyhow::ensure!(
            hex::encode(Sha256::digest(&bytes)) == text(m, "sha256"),
            "Catalog checksum mismatch"
        );
        let imported = store(
            app,
            user,
            m["app_id"].as_u64().unwrap_or_default() as u32,
            &text(m, "name"),
            &text(m, "version"),
            &text(m, "description"),
            &origin,
            &m["details"],
            &bytes,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Catalog import failed: {}", e.1))?;
        app.db.lock().unwrap().execute(
            "UPDATE mod_reviews SET approved=1 WHERE mod_id=?1",
            [&imported],
        )?;
        println!("Imported {} {}", text(m, "name"), text(m, "version"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Live Thunderstore API and archive verification in isolated temporary encrypted storage"]
    async fn thunderstore_live_import_round_trip() {
        import_round_trip(
            "https://thunderstore.io/c/bopl-battle/p/Antimality/InfiniteBlackHoles/",
            "Bopl Battle",
        )
        .await;
    }
    #[tokio::test]
    #[ignore = "Live Modrinth API and encrypted database import in isolated storage"]
    async fn modrinth_live_import_round_trip() {
        import_round_trip("https://modrinth.com/mod/sodium", "Minecraft").await;
    }
    async fn import_round_trip(url: &str, game: &str) {
        use crate::tests::{account, call, fixture, value};
        let (_dir, app) = fixture();
        let member = account(&app, "importer", false);
        let preview = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/mods/external/preview",
                json!({"url":url}),
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(preview["game"], game);
        let request = json!({"url":url,"version":preview["versions"][0]["id"]});
        let imported = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/mods/external/import",
                request.clone(),
                Some(&member),
            )
            .await,
        )
        .await;
        let id = imported["id"]
            .as_str()
            .expect("Import did not return an ID");
        let raw = std::fs::read(app.files.join(format!("{id}.zip"))).unwrap();
        assert!(!raw.starts_with(b"PK"));
        let repeat = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/mods/external/import",
                request,
                Some(&member),
            )
            .await,
        )
        .await;
        assert_eq!(repeat["id"], id);
        assert_eq!(repeat["existing"], true);
        let response = call(
            app.clone(),
            "GET",
            &format!("/api/v1/mods/{id}"),
            Value::Null,
            Some(&member),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let owner = account(&app, "reviewer", true);
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
        let response = call(
            app.clone(),
            "GET",
            &format!("/api/v1/mods/{id}"),
            Value::Null,
            Some(&member),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), UPLOAD_LIMIT)
            .await
            .unwrap();
        assert!(bytes.starts_with(b"PK"));
        assert_eq!(
            value(call(app, "GET", "/api/v1/mods", Value::Null, Some(&member)).await).await[0]["sha256"],
            hex::encode(Sha256::digest(bytes))
        );
    }
    #[test]
    fn provider_urls_and_downloads_are_strict() {
        assert!(link("https://modrinth.com/mod/sodium").is_ok());
        assert!(link("https://www.curseforge.com/minecraft/mc-mods/sodium").is_ok());
        for u in [
            "https://modrinth.com.evil/mod/sodium",
            "https://user@modrinth.com/mod/sodium",
            "http://modrinth.com/mod/sodium",
            "https://127.0.0.1/mod/x",
        ] {
            assert!(link(u).is_err());
        }
        assert!(artifact_url("https://cdn.modrinth.com/data/a/file.jar", "modrinth").is_ok());
        assert!(artifact_url("https://cdn.modrinth.com.evil/data/a", "modrinth").is_err());
        assert!(
            artifact_url(
                "https://edge.forgecdn.net/files/a.jar?api-key=x",
                "curseforge"
            )
            .is_err()
        );
        assert!(!safe_filename("../../evil\".jar").contains('/'));
    }
}
