use super::*;
use reqwest::Url;

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS mod_details(mod_id TEXT PRIMARY KEY REFERENCES mods(id) ON DELETE CASCADE,origin TEXT UNIQUE NOT NULL,data TEXT NOT NULL);")
}
#[derive(Clone, Deserialize)]
pub struct Link {
    pub url: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub game_version: String,
    #[serde(default)]
    pub loader: String,
    #[serde(default)]
    pub include_optional: bool,
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
#[derive(Clone, serde::Serialize)]
pub struct Project {
    #[serde(flatten)]
    pub attribution: Value,
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
    if host == "curseforge.com" && parts.len() >= 3 && cf_content_type(&parts[1]).is_some() {
        return Ok(("curseforge".into(), parts));
    }
    Err(bad(
        "Use a mod project link from thunderstore.io, modrinth.com or curseforge.com",
    ))
}
pub(crate) fn client() -> ApiResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("Canna/0.3.7 (https://cannamods.vip)")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|_| bad("Could not connect to the provider"))
}
fn cf_content_type(category: &str) -> Option<&'static str> {
    match category {
        "mc-mods" | "mods" | "addons" => Some("mod"),
        "texture-packs" | "resource-packs" => Some("resourcepack"),
        "shaders" => Some("shader"),
        "data-packs" | "datapacks" => Some("datapack"),
        "modpacks" => Some("modpack"),
        _ => None,
    }
}
fn cf_release(file: &Value) -> Option<Release> {
    if file["isAvailable"] != true {
        return None;
    }
    let id = file["id"].as_u64().filter(|id| *id > 0)?;
    let versions = strings(&file["gameVersions"]);
    let mut loaders = Vec::new();
    let mut game_versions = Vec::new();
    for version in versions {
        let lower = version.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "forge" | "fabric" | "neoforge" | "quilt" | "liteloader" | "rift"
        ) {
            if !loaders.contains(&lower) {
                loaders.push(lower);
            }
        } else if !game_versions.contains(&version) {
            game_versions.push(version);
        }
    }
    let hash = file["hashes"]
        .as_array()
        .and_then(|a| a.iter().find(|h| h["algo"] == 1))
        .map(|h| text(h, "value"))
        .unwrap_or_default();
    Some(Release {
        id: id.to_string(),
        name: text(file, "displayName"),
        filename: text(file, "fileName"),
        loaders,
        game_versions,
        dependencies: file["dependencies"].clone(),
        download: text(file, "downloadUrl"),
        hash,
        algorithm: "sha1".into(),
    })
}
pub(crate) async fn cf_pages(url: &str) -> ApiResult<Vec<Value>> {
    let mut output = Vec::new();
    for index in (0..10000).step_by(50) {
        let separator = if url.contains('?') { '&' } else { '?' };
        let page = metadata(&format!("{url}{separator}index={index}&pageSize=50"), true).await?;
        let data = page["data"]
            .as_array()
            .ok_or_else(|| bad("Invalid CurseForge response"))?;
        if data.len() > 50 {
            return Err(bad("Invalid CurseForge pagination"));
        }
        output.extend(data.iter().cloned());
        if data.len() < 50
            || page["pagination"]["totalCount"]
                .as_u64()
                .is_some_and(|total| output.len() as u64 >= total)
        {
            break;
        }
    }
    Ok(output)
}
pub(crate) async fn metadata(url: &str, cf: bool) -> ApiResult<Value> {
    metadata_limited(url, cf, 8 * 1024 * 1024).await
}
pub(crate) async fn metadata_limited(url: &str, cf: bool, limit: usize) -> ApiResult<Value> {
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
        if bytes.len() + chunk.len() > limit {
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
type ThunderstoreFallback = Option<(String, i64, std::sync::Arc<Value>)>;
static THUNDERSTORE_FALLBACK: std::sync::LazyLock<tokio::sync::Mutex<ThunderstoreFallback>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(None));
fn compact_thunderstore_index(data: &Value) -> ApiResult<Value> {
    let packages = data
        .as_array()
        .ok_or_else(|| bad("Invalid Thunderstore package index"))?;
    let compact: Vec<Value> = packages.iter().map(|p| {
        let versions: Vec<Value> = p["versions"].as_array().into_iter().flatten().map(|v|json!({
            "version_number":v["version_number"],"description":v["description"],"icon":v["icon"],
            "dependencies":v["dependencies"],"download_url":v["download_url"],"is_active":v["is_active"]
        })).collect();
        json!({"name":p["name"],"owner":p["owner"],"is_deprecated":p["is_deprecated"],
            "categories":p["categories"],"has_nsfw_content":p["has_nsfw_content"],"versions":versions})
    }).collect();
    let compact = json!(compact);
    if serde_json::to_vec(&compact)
        .map_err(|_| bad("Invalid Thunderstore package index"))?
        .len()
        > 16 * 1024 * 1024
    {
        return Err(bad("Thunderstore dependency index exceeds limit"));
    }
    Ok(compact)
}
fn public_thunderstore_package(index: &Value, parts: &[String], version: &str) -> ApiResult<Value> {
    let p = index
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["owner"] == parts[3] && p["name"] == parts[4])
        .ok_or_else(|| bad("This mod is not listed for the selected game"))?;
    let versions = p["versions"]
        .as_array()
        .ok_or_else(|| bad("Invalid Thunderstore package versions"))?;
    if !version.is_empty() {
        return versions
            .iter()
            .find(|v| v["version_number"] == version)
            .cloned()
            .ok_or_else(|| bad("Required dependency is unavailable"));
    }
    let latest = versions
        .first()
        .ok_or_else(|| bad("This mod is unavailable"))?;
    Ok(
        json!({"name":p["name"],"owner":p["owner"],"is_deprecated":p["is_deprecated"],"latest":latest,
        "community_listings":[{"community":parts[1],"categories":p["categories"],"has_nsfw_content":p["has_nsfw_content"]}]}),
    )
}
async fn thunderstore_metadata(parts: &[String], version: &str) -> ApiResult<Value> {
    // The public community index is the same provider's official metadata, not
    // a scraper or an authorization bypass. Reuse a bounded snapshot on failures
    // instead of fetching the whole index for every transitive requirement.
    let suffix = if version.is_empty() {
        String::new()
    } else {
        format!("{version}/")
    };
    let endpoint = format!(
        "https://thunderstore.io/api/experimental/package/{}/{}/{suffix}",
        parts[3], parts[4]
    );
    {
        let cache = THUNDERSTORE_FALLBACK.lock().await;
        if let Some((community, at, index)) = &*cache
            && community == &parts[1]
            && now() - at < 300
        {
            return public_thunderstore_package(index, parts, version);
        }
    }
    if let Ok(data) = metadata(&endpoint, false).await {
        return Ok(data);
    }
    let mut cache = THUNDERSTORE_FALLBACK.lock().await;
    if !cache
        .as_ref()
        .is_some_and(|(community, at, _)| community == &parts[1] && now() - at < 300)
    {
        let data = metadata_limited(
            &format!("https://thunderstore.io/c/{}/api/v1/package/", parts[1]),
            false,
            64 * 1024 * 1024,
        )
        .await?;
        *cache = Some((
            parts[1].clone(),
            now(),
            std::sync::Arc::new(compact_thunderstore_index(&data)?),
        ));
    }
    public_thunderstore_package(&cache.as_ref().unwrap().2, parts, version)
}
pub async fn resolve(raw: &str) -> ApiResult<Project> {
    resolve_context(raw, false).await
}
async fn resolve_context(raw: &str, dependency: bool) -> ApiResult<Project> {
    let (provider, parts) = link(raw)?;
    if provider == "thunderstore" {
        if steam_id(&parts[1]).is_none() {
            return Err(bad(
                "This Thunderstore community is not a supported Steam/Unity game",
            ));
        }
        let p = thunderstore_metadata(&parts, "").await?;
        let listing = p["community_listings"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["community"] == parts[1]))
            .ok_or_else(|| bad("This mod is not listed for the selected game"))?;
        thunderstore_available(&p, dependency)?;
        let latest = &p["latest"];
        let version = text(latest, "version_number");
        let game = game_profiles::by_community(&parts[1])
            .map(|g| g.name.as_str())
            .unwrap_or_else(|| match parts[1].as_str() {
                "bopl-battle" => "Bopl Battle",
                "rounds" => "ROUNDS",
                "riskofrain2" => "Risk of Rain 2",
                "lethal-company" => "Lethal Company",
                "content-warning" => "Content Warning",
                other => other,
            });
        Ok(Project {
            attribution: json!({"icon_url":text(latest,"icon"),"deprecated":p["is_deprecated"]==true,"is_modpack":listing["categories"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(|s|s.eq_ignore_ascii_case("Modpacks")))),"author_links":[{"name":text(&p,"owner"),"url":format!("https://thunderstore.io/c/{}/p/{}/",parts[1],parts[3])}]}),
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
        let mut author_links = Vec::new();
        let authors = if slug(&team) {
            let members = metadata(
                &format!("https://api.modrinth.com/v2/team/{team}/members"),
                false,
            )
            .await?;
            for m in members.as_array().into_iter().flatten() {
                let name = text(&m["user"], "username");
                if slug(&name) {
                    author_links.push(
                        json!({"name":name,"url":format!("https://modrinth.com/user/{name}")}),
                    );
                }
            }
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
            attribution: json!({"icon_url":text(&project,"icon_url"),"author_links":author_links}),
            provider,
            id,
            name: text(&project, "title"),
            description: text(&project, "description"),
            source_url: format!(
                "https://modrinth.com/{}/{}",
                project["project_type"]
                    .as_str()
                    .filter(|v| ["mod", "resourcepack", "shader", "datapack"].contains(v))
                    .unwrap_or(&parts[0]),
                parts[1]
            ),
            game: "Minecraft".into(),
            authors,
            license: text(&project["license"], "id"),
            versions: versions
                .as_array()
                .into_iter()
                .flatten()
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
        let games = cf_pages("https://api.curseforge.com/v1/games").await?;
        let game = games
            .iter()
            .find(|g| g["slug"] == parts[0])
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
        let files = cf_pages(&format!("https://api.curseforge.com/v1/mods/{id}/files")).await?;
        let releases = files.iter().filter_map(cf_release).collect();
        Ok(Project {
            attribution: json!({"icon_url":text(&project["logo"],"thumbnailUrl"),"author_links":project["authors"].as_array().into_iter().flatten().map(|a|json!({"name":text(a,"name"),"url":text(a,"url")})).collect::<Vec<_>>()}),
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
    let result = import_background(&app, user, input).await?;
    subscriptions::subscribe(
        &app.db.lock().unwrap(),
        user,
        result["id"]
            .as_str()
            .ok_or_else(|| bad("Import did not return a mod"))?,
    )?;
    Ok(axum::Json(result))
}
pub async fn refresh_existing(app: &App, user: i64, id: &str) -> ApiResult<Option<String>> {
    let (data, origin) = {
        let db = app.db.lock().unwrap();
        (
            details(&db, id)?,
            db.query_row(
                "SELECT origin FROM mod_details WHERE mod_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )?,
        )
    };
    if data["provider"] == "github" {
        return crate::source_packages::refresh(app, user, &data, &origin).await;
    }
    let (loader, game_version) = update_profile(&data);
    let mut input = Link {
        url: data["source_url"].as_str().unwrap_or_default().into(),
        version: String::new(),
        game_version,
        loader,
        include_optional: false,
    };
    let (project, release) = selection(&input).await?;
    if origin == format!("{}:{}:{}", project.provider, project.id, release.id) {
        return Ok(None);
    }
    input.version = release.id;
    let result = import_background(app, user, input).await?;
    Ok(Some(
        result["id"]
            .as_str()
            .ok_or_else(|| bad("Update import failed"))?
            .into(),
    ))
}
pub fn update_profile(d: &Value) -> (String, String) {
    let field = |name: &str, array: &str| {
        d["update_profile"][name]
            .as_str()
            .or_else(|| {
                d[array]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_owned()
    };
    let loader = field("loader", "loaders");
    let mut version = field("game_version", "game_versions");
    if version == "Check author compatibility notes" {
        version.clear();
    }
    (loader, version)
}
pub async fn import_background(app: &App, user: i64, input: Link) -> ApiResult<Value> {
    let (project, release) = selection(&input).await?;
    let origin = format!("{}:{}:{}", project.provider, project.id, release.id);
    if let Some(id) = existing(app, &origin)? {
        let db = app.db.lock().unwrap();
        provider_cache::track(&db, &id)?;
        return Ok(
            json!({"id":id,"existing":true,"approved":security::approved(&db,&id).is_ok(),"dependencies_added":0,"dependency_count":details(&db,&id)?["dependency_ids"].as_array().map(Vec::len).unwrap_or(0)}),
        );
    }
    let (root, nodes, edges, order) = dependency_graph(input).await?;
    let mut ids = std::collections::BTreeMap::<String, String>::new();
    let mut imported = 0;
    let mut root_existing = false;
    for origin in order {
        let (input, project, release) = &nodes[&origin];
        let deps: Vec<String> = edges
            .get(&origin)
            .into_iter()
            .flatten()
            .filter_map(|key| ids.get(key).cloned())
            .collect();
        let (id, existing) = import_one(app, user, input, project, release, &deps).await?;
        if !existing {
            imported += 1;
        }
        if origin == root {
            root_existing = existing;
        }
        ids.insert(origin, id);
    }
    let id = &ids[&root];
    if community::role(app, user)? == "owner" {
        let mut db = app.db.lock().unwrap();
        let tx = db.transaction()?;
        for id in ids.values() {
            if tx.execute(
                "UPDATE mod_reviews SET approved=1 WHERE mod_id=?1 AND approved=0",
                [id],
            )? == 1
            {
                tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'owner-publish-mod',?2,?3)",params![user,id,now()])?;
            }
        }
        tx.commit()?;
    }
    let approved = security::approved(&app.db.lock().unwrap(), id).is_ok();
    let dependencies_added = imported - i32::from(!root_existing);
    Ok(
        json!({"id":id,"existing":root_existing,"approved":approved,"dependencies_added":dependencies_added,"dependency_count":ids.len().saturating_sub(1)}),
    )
}
pub(crate) async fn download_release(project: &Project, release: &Release) -> ApiResult<Vec<u8>> {
    let download = if project.provider == "curseforge" && release.download.is_empty() {
        let result = metadata(
            &format!(
                "https://api.curseforge.com/v1/mods/{}/files/{}/download-url",
                project.id, release.id
            ),
            true,
        )
        .await?;
        result["data"]
            .as_str()
            .ok_or_else(|| bad("The author has not enabled third-party downloads"))?
            .to_owned()
    } else {
        release.download.clone()
    };
    let mut url = artifact_url(&download, &project.provider)?;
    // CDN requests never receive provider credentials. Every redirect must stay
    // on the provider's explicit archive hosts and paths; automatic redirects
    // remain disabled so a provider response cannot turn this into an SSRF.
    let client = client()?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|_| bad("Mod download failed"))?;
    for hop in 0..=3 {
        if !response.status().is_redirection() {
            break;
        }
        if hop == 3 {
            return Err(bad("Provider download exceeded the redirect limit"));
        }
        let target = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| bad("Invalid provider redirect"))?;
        url = redirected_artifact(&url, target, &project.provider)?;
        response = client
            .get(url.clone())
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
    Ok(bytes)
}
fn redirected_artifact(current: &Url, location: &str, provider: &str) -> ApiResult<Url> {
    let next = current
        .join(location)
        .map_err(|_| bad("Invalid provider redirect"))?;
    artifact_url(next.as_str(), provider)
}
async fn import_one(
    app: &App,
    user: i64,
    input: &Link,
    project: &Project,
    release: &Release,
    deps: &[String],
) -> ApiResult<(String, bool)> {
    let origin = format!("{}:{}:{}", project.provider, project.id, release.id);
    if let Some(id) = existing(app, &origin)? {
        let db = app.db.lock().unwrap();
        let mut data = details(&db, &id)?;
        // Reusing a release must retain the exact dependency graph already reviewed.
        if project.provider == "thunderstore"
            && let Some(loader) = game_profiles::loader(&project.id)
        {
            data["framework_root"] = json!(loader.root);
        }
        db.execute(
            "UPDATE mod_details SET data=?1 WHERE mod_id=?2",
            params![data.to_string(), id],
        )?;
        provider_cache::track(&db, &id)?;
        return Ok((id, true));
    }
    let bytes = download_release(project, release).await?;
    let mut details = serde_json::to_value(project).unwrap();
    details.as_object_mut().unwrap().remove("versions");
    if project.provider == "thunderstore"
        && let Some(loader) = game_profiles::loader(&project.id)
    {
        details["framework_root"] = json!(loader.root);
    }
    details["filename"] = json!(safe_filename(&release.filename));
    details["release_id"] = json!(release.id);
    details["update_profile"] = json!({"loader":input.loader,"game_version":input.game_version});
    details["loaders"] = json!(release.loaders);
    details["game_versions"] = json!(release.game_versions);
    details["dependencies"] = release.dependencies.clone();
    details["dependency_ids"] = json!(deps);
    details["content_type"] = json!(if project.provider == "modrinth" {
        parts_type(&project.source_url)
    } else if project.provider == "curseforge" {
        let (_, p) = link(&input.url)?;
        cf_content_type(&p[1]).unwrap_or("mod").to_owned()
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
    if let Some(profile) = game_profiles::by_id(game) {
        details["folder"] = json!(profile.folder);
    }
    let id = store(
        app,
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
    provider_cache::track(&app.db.lock().unwrap(), &id)?;
    Ok((id, false))
}
fn version_valid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
type ImportNodes = std::collections::BTreeMap<String, (Link, Project, Release)>;
type ImportEdges = std::collections::BTreeMap<String, Vec<String>>;
fn compatible(v: &Release, input: &Link) -> bool {
    (input.game_version.is_empty() || v.game_versions.contains(&input.game_version))
        && (input.loader.is_empty()
            || v.loaders.is_empty()
            || v.loaders
                .iter()
                .any(|l| l.eq_ignore_ascii_case(&input.loader)))
}
fn mr_file(v: &Value) -> Option<Release> {
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
}
fn thunderstore_available(project: &Value, dependency: bool) -> ApiResult<()> {
    // Deprecated packages may still be active requirements of supported mods.
    // Deprecation is not a malware verdict; imported files still pass review.
    if project["latest"]["is_active"] != true {
        return Err(bad(if dependency {
            "Required dependency is unavailable"
        } else {
            "This mod is unavailable"
        }));
    }
    if project["is_deprecated"] == true && !dependency {
        return Err(bad("This mod is deprecated"));
    }
    Ok(())
}
pub(crate) async fn selection(input: &Link) -> ApiResult<(Project, Release)> {
    selection_context(input, false).await
}
async fn selection_context(input: &Link, dependency: bool) -> ApiResult<(Project, Release)> {
    let mut project = resolve_context(&input.url, dependency).await?;
    if !input.version.is_empty() && !project.versions.iter().any(|v| v.id == input.version) {
        if !version_valid(&input.version) {
            return Err(bad("Invalid dependency version"));
        }
        let version = if project.provider == "modrinth" {
            let data = metadata(
                &format!("https://api.modrinth.com/v2/version/{}", input.version),
                false,
            )
            .await?;
            if data["project_id"] != project.id {
                return Err(bad("Dependency version belongs to another project"));
            }
            mr_file(&data).ok_or_else(|| bad("Dependency file is unavailable"))?
        } else if project.provider == "thunderstore" {
            let (_, parts) = link(&input.url)?;
            let data = thunderstore_metadata(&parts, &input.version).await?;
            if data["is_active"] != true {
                return Err(bad("Required dependency is unavailable"));
            }
            Release {
                id: input.version.clone(),
                name: input.version.clone(),
                filename: format!("{}-{}.zip", project.name, input.version),
                loaders: vec![],
                game_versions: vec![],
                dependencies: data["dependencies"].clone(),
                download: text(&data, "download_url"),
                hash: String::new(),
                algorithm: String::new(),
            }
        } else {
            let data = metadata(
                &format!(
                    "https://api.curseforge.com/v1/mods/{}/files/{}",
                    project.id, input.version
                ),
                true,
            )
            .await?;
            cf_release(&data["data"])
                .ok_or_else(|| bad("Required dependency file is unavailable"))?
        };
        project.versions.push(version);
    }
    let release = project
        .versions
        .iter()
        .find(|v| {
            (input.version.is_empty() || v.id == input.version)
                && (project.provider == "thunderstore" || compatible(v, input))
        })
        .cloned()
        .ok_or_else(|| bad("No dependency file matches this Minecraft version and loader"))?;
    Ok((project, release))
}
async fn dependency_links(
    project: &Project,
    release: &Release,
    input: &Link,
) -> ApiResult<Vec<Link>> {
    let mut links = Vec::new();
    for dep in release.dependencies.as_array().into_iter().flatten() {
        let (url, version) = match project.provider.as_str() {
            "thunderstore" => {
                let dep = dep
                    .as_str()
                    .ok_or_else(|| bad("Invalid Thunderstore dependency"))?;
                let parts: Vec<&str> = dep.split('-').collect();
                if parts.len() != 3
                    || !slug(parts[0])
                    || !slug(parts[1])
                    || !version_valid(parts[2])
                {
                    return Err(bad("Invalid Thunderstore dependency"));
                }
                let (_, root) = link(&input.url)?;
                (
                    format!(
                        "https://thunderstore.io/c/{}/p/{}/{}/",
                        root[1], parts[0], parts[1]
                    ),
                    parts[2].to_owned(),
                )
            }
            "modrinth" => {
                let kind = dep["dependency_type"].as_str().unwrap_or("");
                if kind != "required" && !(input.include_optional && kind == "optional") {
                    continue;
                }
                let version = text(dep, "version_id");
                let mut id = text(dep, "project_id");
                if id.is_empty() && !version.is_empty() {
                    if !slug(&version) {
                        return Err(bad("Invalid dependency version"));
                    }
                    id = text(
                        &metadata(
                            &format!("https://api.modrinth.com/v2/version/{version}"),
                            false,
                        )
                        .await?,
                        "project_id",
                    );
                }
                if !slug(&id) {
                    return Err(bad("Required dependency project is unavailable"));
                }
                (format!("https://modrinth.com/mod/{id}"), version)
            }
            "curseforge" => {
                let kind = dep["relationType"].as_u64().unwrap_or(0);
                if kind != 3 && !(input.include_optional && kind == 2) {
                    continue;
                }
                let id = dep["modId"]
                    .as_u64()
                    .filter(|id| *id > 0)
                    .ok_or_else(|| bad("Invalid dependency project"))?;
                let data =
                    metadata(&format!("https://api.curseforge.com/v1/mods/{id}"), true).await?;
                let url = text(&data["data"]["links"], "websiteUrl");
                let (provider, _) = link(&url)?;
                if provider != "curseforge" {
                    return Err(bad("Invalid dependency project address"));
                }
                (url, String::new())
            }
            _ => return Err(bad("Unsupported dependency provider")),
        };
        links.push(Link {
            url,
            version,
            game_version: input.game_version.clone(),
            loader: input.loader.clone(),
            include_optional: input.include_optional,
        });
    }
    Ok(links)
}
fn dependency_order(root: &str, edges: &ImportEdges) -> ApiResult<Vec<String>> {
    fn visit(
        key: &str,
        edges: &ImportEdges,
        active: &mut std::collections::BTreeSet<String>,
        done: &mut std::collections::BTreeSet<String>,
        out: &mut Vec<String>,
    ) -> ApiResult<()> {
        if done.contains(key) {
            return Ok(());
        }
        if active.len() >= 32 || !active.insert(key.to_owned()) {
            return Err(bad(
                "Dependency graph contains a cycle or exceeds 32 levels",
            ));
        }
        for next in edges.get(key).into_iter().flatten() {
            visit(next, edges, active, done, out)?;
        }
        active.remove(key);
        done.insert(key.to_owned());
        out.push(key.to_owned());
        Ok(())
    }
    let mut out = Vec::new();
    visit(
        root,
        edges,
        &mut Default::default(),
        &mut Default::default(),
        &mut out,
    )?;
    Ok(out)
}
async fn dependency_graph(
    input: Link,
) -> ApiResult<(String, ImportNodes, ImportEdges, Vec<String>)> {
    dependency_graph_with(input, |input, dependency| async move {
        selection_context(&input, dependency).await
    })
    .await
}
async fn dependency_graph_with<F, Fut>(
    input: Link,
    mut select: F,
) -> ApiResult<(String, ImportNodes, ImportEdges, Vec<String>)>
where
    F: FnMut(Link, bool) -> Fut,
    Fut: std::future::Future<Output = ApiResult<(Project, Release)>>,
{
    let mut queue = std::collections::VecDeque::from([(None::<String>, input)]);
    let mut root = String::new();
    let mut game = String::new();
    let mut nodes = ImportNodes::new();
    let mut edges = ImportEdges::new();
    let mut requests = 0;
    let mut versions = std::collections::BTreeMap::new();
    let mut selections = std::collections::BTreeMap::new();
    let mut latest_dependencies = false;
    while let Some((parent, mut input)) = queue.pop_front() {
        requests += 1;
        if requests > 512 {
            return Err(bad("Dependency import exceeds the 128-project limit"));
        }
        let request_key = (
            input.url.clone(),
            input.version.clone(),
            input.game_version.clone(),
            input.loader.clone(),
            input.include_optional,
        );
        let (project, release) = if let Some(selected) = selections.get(&request_key) {
            let selected: &(Project, Release) = selected;
            selected.clone()
        } else {
            let selected = select(input.clone(), parent.is_some()).await?;
            selections.insert(request_key, selected.clone());
            selected
        };
        let key = format!("{}:{}", project.provider, project.id);
        if let Some(old) = versions.insert(key, release.id.clone())
            && old != release.id
        {
            return Err(bad(
                "Dependencies require conflicting versions of the same project",
            ));
        }
        let origin = format!("{}:{}:{}", project.provider, project.id, release.id);
        if let Some(parent) = parent {
            edges.entry(parent).or_default().push(origin.clone());
        } else {
            root = origin.clone();
            game = project.game.clone();
            // Match the ecosystem's individual-mod behavior. Provider modpacks
            // retain exact declarations; Minecraft file pins stay exact too.
            latest_dependencies =
                project.provider == "thunderstore" && project.attribution["is_modpack"] != true;
            if project.provider != "thunderstore" && input.game_version.is_empty() {
                input.game_version = release.game_versions.first().cloned().unwrap_or_default();
            }
            if project.provider != "thunderstore" && input.loader.is_empty() {
                input.loader = release.loaders.first().cloned().unwrap_or_default();
            }
        }
        if project.game != game {
            return Err(bad("A dependency belongs to a different game"));
        }
        if nodes.contains_key(&origin) {
            continue;
        }
        if nodes.len() >= 128 {
            return Err(bad("Dependency import exceeds the 128-project limit"));
        }
        for mut dependency in dependency_links(&project, &release, &input).await? {
            if latest_dependencies && project.provider == "thunderstore" {
                dependency.version.clear();
            }
            queue.push_back((Some(origin.clone()), dependency));
        }
        nodes.insert(origin, (input, project, release));
    }
    let order = dependency_order(&root, &edges)?;
    Ok((root, nodes, edges, order))
}
pub(crate) fn steam_id(slug: &str) -> Option<u32> {
    if let Some(profile) = game_profiles::by_community(slug) {
        return Some(profile.app_id);
    }
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
        let approved:bool=tx.query_row("SELECT role='owner' FROM users WHERE id=?1",[user],|r|r.get(0))?;
        tx.execute("INSERT INTO mod_reviews(mod_id,approved) VALUES(?1,?2)",params![id,approved])?;
        if approved {tx.execute("INSERT INTO audit(actor,action,target,created) VALUES(?1,'owner-publish-mod',?2,?3)",params![user,id,now()])?;}
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
    // Backfill provider attribution without changing existing release pins or dependency graphs.
    let rows = {
        let db = app.db.lock().unwrap();
        db.prepare("SELECT mod_id,data FROM mod_details WHERE json_extract(data,'$.provider') IN ('thunderstore','modrinth','curseforge')")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<Result<Vec<_>,_>>()?
    };
    for (id, raw) in rows {
        let mut details: Value = serde_json::from_str(&raw)?;
        if details["author_links"].is_array() {
            continue;
        }
        if let Ok(project) = resolve(details["source_url"].as_str().unwrap_or_default()).await {
            details["author_links"] = project.attribution["author_links"].clone();
            details["icon_url"] = project.attribution["icon_url"].clone();
            app.db.lock().unwrap().execute(
                "UPDATE mod_details SET data=?1 WHERE mod_id=?2",
                params![details.to_string(), id],
            )?;
            println!("Refreshed provider attribution");
        } else {
            println!("Provider attribution unavailable; retained existing metadata");
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_thunderstore_fallback_preserves_identity_history_and_availability() {
        let index=compact_thunderstore_index(&json!([
            {"owner":"Other","name":"Pack","versions":[{"version_number":"99.0.0","is_active":true}]},
            {"owner":"Test","name":"Pack","is_deprecated":true,"categories":["Modpacks"],
            "versions":[{"version_number":"2.0.0","is_active":true,"dependencies":["Test-Lib-1.0.0"],"download_url":"https://thunderstore.io/package/download/Test/Pack/2.0.0/","icon":"https://ccdn.thunderstore.io/icon.png","description":"original"},
                {"version_number":"1.0.0","is_active":false,"dependencies":[]}]}
        ])).unwrap();
        let (_, parts) = link(&graph_input("Pack").url).unwrap();
        let p = public_thunderstore_package(&index, &parts, "").unwrap();
        assert_eq!(p["owner"], "Test");
        assert_eq!(p["latest"]["version_number"], "2.0.0");
        assert_eq!(p["latest"]["dependencies"], json!(["Test-Lib-1.0.0"]));
        assert_eq!(p["latest"]["description"], "original");
        assert_eq!(p["community_listings"][0]["community"], "rounds");
        assert_eq!(
            p["community_listings"][0]["categories"],
            json!(["Modpacks"])
        );
        assert!(thunderstore_available(&p, false).is_err());
        assert!(thunderstore_available(&p, true).is_ok());
        assert_eq!(
            public_thunderstore_package(&index, &parts, "1.0.0").unwrap()["is_active"],
            false
        );
        assert!(public_thunderstore_package(&index, &parts, "3.0.0").is_err());
        let (_, missing) = link(&graph_input("Missing").url).unwrap();
        assert!(public_thunderstore_package(&index, &missing, "").is_err());
        assert!(compact_thunderstore_index(&json!({})).is_err());
    }
    fn graph_fixture(
        input: &Link,
        version: &str,
        deps: Value,
        modpack: bool,
    ) -> (Project, Release) {
        let (provider, parts) = link(&input.url).unwrap();
        let thunderstore = provider == "thunderstore";
        let id = if thunderstore {
            format!("{}-{}", parts[3], parts[4])
        } else {
            parts[1].clone()
        };
        let release = Release {
            id: version.into(),
            name: version.into(),
            filename: "fixture.zip".into(),
            loaders: vec![],
            game_versions: vec![],
            dependencies: deps,
            download: String::new(),
            hash: String::new(),
            algorithm: String::new(),
        };
        (
            Project {
                provider,
                id: id.clone(),
                name: id,
                description: String::new(),
                source_url: input.url.clone(),
                attribution: json!({"is_modpack":modpack}),
                game: if thunderstore { "ROUNDS" } else { "Minecraft" }.into(),
                authors: String::new(),
                license: String::new(),
                versions: vec![release.clone()],
            },
            release,
        )
    }
    fn graph_input(name: &str) -> Link {
        Link {
            url: format!("https://thunderstore.io/c/rounds/p/Test/{name}/"),
            version: "1.0.0".into(),
            game_version: String::new(),
            loader: String::new(),
            include_optional: false,
        }
    }
    #[tokio::test]
    async fn repeated_release_keeps_reviewed_dependency_pins_and_scan_decisions() {
        let (_dir, app) = crate::tests::fixture();
        crate::tests::account(&app, "repeat-owner", true);
        let input = graph_input("Repeated");
        let (project, release) = graph_fixture(&input, "1.0.0", json!([]), false);
        let id = store(
            &app,
            1,
            1557740,
            "Repeated",
            "1.0.0",
            "",
            "thunderstore:Test-Repeated:1.0.0",
            &json!({"provider":"thunderstore","dependency_ids":["reviewed-old-pin"]}),
            b"PK\x05\x06test",
        )
        .await
        .unwrap();
        {
            let db = app.db.lock().unwrap();
            db.execute("INSERT INTO mod_scans VALUES(?1,'hash','complete','{\"findings\":[{\"accepted\":true}]}',0)",[&id]).unwrap();
        }
        let (same, reused) = import_one(
            &app,
            1,
            &input,
            &project,
            &release,
            &["new-unreviewed-pin".into()],
        )
        .await
        .unwrap();
        assert!(reused);
        assert_eq!(same, id);
        let db = app.db.lock().unwrap();
        assert_eq!(
            details(&db, &id).unwrap()["dependency_ids"],
            json!(["reviewed-old-pin"])
        );
        assert!(
            db.query_row("SELECT report FROM mod_scans WHERE mod_id=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
            .contains("true")
        );
    }
    #[tokio::test]
    async fn rounds_reported_mods_resolve_recursive_latest_dependencies_once() {
        let data: Value =
            serde_json::from_str(include_str!("fixtures/rounds-dependencies.json")).unwrap();
        for name in data["roots"].as_array().unwrap() {
            let name = name.as_str().unwrap();
            let (owner, package) = name.split_once('-').unwrap();
            let mut input = graph_input(package);
            input.url = format!("https://thunderstore.io/c/rounds/p/{owner}/{package}/");
            input.version = data["packages"][name]["version"].as_str().unwrap().into();
            let mut counts = std::collections::BTreeMap::<String, usize>::new();
            let (root, nodes, edges, order) = dependency_graph_with(input.clone(), |request, dependency| {
                let (_, parts) = link(&request.url).unwrap();
                let key = format!("{}-{}", parts[3], parts[4]);
                *counts.entry(key.clone()).or_default() += 1;
                let p = &data["packages"][&key];
                assert!(p.is_object(), "Missing fixture {key}");
                thunderstore_available(&json!({"is_deprecated":p["deprecated"],"latest":{"is_active":p["active"]}}),dependency).unwrap();
                if dependency { assert!(request.version.is_empty()); } else { assert_eq!(request.version, input.version); }
                let selected = graph_fixture(&request, p["version"].as_str().unwrap(), p["dependencies"].clone(), false);
                std::future::ready(Ok(selected))
            }).await.unwrap();
            assert_eq!(nodes[&root].2.id, input.version);
            assert!(nodes.len() >= 10, "Incomplete closure for {name}");
            assert_eq!(order.len(), nodes.len());
            assert_eq!(order.last(), Some(&root));
            assert!(
                counts.values().all(|n| *n == 1),
                "Repeated metadata lookup for {name}"
            );
            assert_eq!(counts.len(), nodes.len());
            for (parent, children) in edges {
                for child in children {
                    assert!(
                        order.iter().position(|k| k == &child).unwrap()
                            < order.iter().position(|k| k == &parent).unwrap()
                    );
                }
            }
        }
    }
    #[tokio::test]
    async fn thunderstore_modpack_and_minecraft_file_pins_still_reject_real_conflicts() {
        let err = dependency_graph_with(graph_input("Pack"), |request, _| {
            let (_, parts) = link(&request.url).unwrap();
            let deps = if parts[4] == "Pack" {
                json!(["Test-Lib-1.0.0", "Test-Other-1.0.0"])
            } else if parts[4] == "Other" {
                json!(["Test-Lib-2.0.0"])
            } else {
                json!([])
            };
            let selected = graph_fixture(&request, &request.version, deps, true);
            std::future::ready(Ok(selected))
        })
        .await
        .err()
        .unwrap();
        assert_eq!(
            err.1,
            "Dependencies require conflicting versions of the same project"
        );
        let mut input = graph_input("Pack");
        input.url = "https://modrinth.com/mod/root".into();
        let result = dependency_graph_with(input, |request, _| {
            let (_, parts) = link(&request.url).unwrap();
            let deps = match parts[1].as_str() {
                "root" => json!([{"dependency_type":"required","project_id":"leaf","version_id":"v1"},{"dependency_type":"required","project_id":"other","version_id":"v1"}]),
                "other" => json!([{"dependency_type":"required","project_id":"leaf","version_id":"v2"}]), _=>json!([]),
            };
            let selected = graph_fixture(&request, &request.version, deps, false);
            std::future::ready(Ok(selected))
        }).await;
        assert_eq!(
            result.err().unwrap().1,
            "Dependencies require conflicting versions of the same project"
        );
    }
    #[tokio::test]
    async fn latest_dependency_graph_rejects_cycles_missing_files_and_cross_game_content() {
        let cycle = dependency_graph_with(graph_input("Root"), |request, _| {
            let (_, parts) = link(&request.url).unwrap();
            let deps = if parts[4] == "Root" {
                json!(["Test-Lib-1.0.0"])
            } else {
                json!(["Test-Root-1.0.0"])
            };
            std::future::ready(Ok(graph_fixture(&request, "1.0.0", deps, false)))
        })
        .await;
        assert_eq!(
            cycle.err().unwrap().1,
            "Dependency graph contains a cycle or exceeds 32 levels"
        );
        for missing in [true, false] {
            let result = dependency_graph_with(graph_input("Root"), |request, dependency| {
                let selected = if dependency && missing {
                    Err(bad("Required dependency is unavailable"))
                } else {
                    let mut pair = graph_fixture(
                        &request,
                        "1.0.0",
                        if dependency {
                            json!([])
                        } else {
                            json!(["Test-Lib-1.0.0"])
                        },
                        false,
                    );
                    if dependency {
                        pair.0.game = "Other game".into();
                    }
                    Ok(pair)
                };
                std::future::ready(selected)
            })
            .await;
            assert_eq!(
                result.err().unwrap().1,
                if missing {
                    "Required dependency is unavailable"
                } else {
                    "A dependency belongs to a different game"
                }
            );
        }
    }
    #[test]
    fn deprecated_active_requirements_are_allowed_but_unavailable_files_are_not() {
        let p = json!({"is_deprecated":true,"latest":{"is_active":true}});
        assert_eq!(
            thunderstore_available(&p, false).err().unwrap().1,
            "This mod is deprecated"
        );
        assert!(thunderstore_available(&p, true).is_ok());
        assert!(thunderstore_available(&json!({"latest":{"is_active":false}}), true).is_err());
        assert!(thunderstore_available(&json!({}), true).is_err());
    }
    #[tokio::test]
    #[ignore = "Live Thunderstore metadata only; no archives or production database writes"]
    async fn rounds_live_dependency_graphs() {
        for (owner, name) in [
            ("XAngelMoonX", "CR"),
            ("Root", "Classes_Manager_Reborn"),
            ("CrazyCoders", "RarityBundle"),
            ("Keys", "KeysCards"),
            ("willuwontu", "ItemShops"),
        ] {
            let mut input = graph_input(name);
            input.url = format!("https://thunderstore.io/c/rounds/p/{owner}/{name}/");
            input.version.clear();
            let (root, nodes, _, order) = dependency_graph(input).await.unwrap();
            assert_eq!(nodes[&root].1.game, "ROUNDS");
            assert!(nodes.len() >= 10);
            assert_eq!(order.len(), nodes.len());
            println!("{owner}/{name}: {} resolved projects", nodes.len());
        }
    }
    #[test]
    fn dependency_graph_orders_transitive_diamonds_and_rejects_cycles() {
        let edges = ImportEdges::from([
            ("root".into(), vec!["a".into(), "b".into()]),
            ("a".into(), vec!["leaf".into()]),
            ("b".into(), vec!["leaf".into()]),
        ]);
        assert_eq!(
            dependency_order("root", &edges).unwrap(),
            ["leaf", "a", "b", "root"]
        );
        let mut cycle = edges;
        cycle.insert("leaf".into(), vec!["root".into()]);
        assert!(dependency_order("root", &cycle).is_err());
        assert!(version_valid("1.2.3"));
        assert!(!version_valid("../1"));
    }
    #[tokio::test]
    async fn thunderstore_dependency_versions_and_owner_publishing_are_preserved() {
        let (_dir, app) = crate::tests::fixture();
        let owner = crate::tests::account(&app, "publisher", false);
        let member = crate::tests::account(&app, "uploader", false);
        let owner = app
            .auth(&HeaderMap::from_iter([(
                "authorization".parse().unwrap(),
                format!("Bearer {owner}").parse().unwrap(),
            )]))
            .unwrap()
            .0;
        let member = app
            .auth(&HeaderMap::from_iter([(
                "authorization".parse().unwrap(),
                format!("Bearer {member}").parse().unwrap(),
            )]))
            .unwrap()
            .0;
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE users SET role='owner' WHERE id=?1", [owner])
            .unwrap();
        let a = store(
            &app,
            owner,
            1686940,
            "Owner mod",
            "1",
            "test",
            "fixture:owner",
            &json!({}),
            b"PK\x03\x04test bytes",
        )
        .await
        .unwrap();
        let b = store(
            &app,
            member,
            1686940,
            "Member mod",
            "1",
            "test",
            "fixture:member",
            &json!({"dependency_ids":[a]}),
            b"PK\x03\x04test bytes",
        )
        .await
        .unwrap();
        assert!(security::approved(&app.db.lock().unwrap(), &a).is_ok());
        assert!(security::approved(&app.db.lock().unwrap(), &b).is_err());
        let project = Project {
            provider: "thunderstore".into(),
            id: "Test-Mod".into(),
            name: "Mod".into(),
            description: String::new(),
            source_url: String::new(),
            attribution: json!({}),
            game: "Bopl Battle".into(),
            authors: String::new(),
            license: String::new(),
            versions: vec![],
        };
        let release = Release {
            id: "1".into(),
            name: "1".into(),
            filename: String::new(),
            loaders: vec![],
            game_versions: vec![],
            dependencies: json!(["BepInEx-BepInExPack_BoplBattle-5.4.2301"]),
            download: String::new(),
            hash: String::new(),
            algorithm: String::new(),
        };
        let input = Link {
            url: "https://thunderstore.io/c/bopl-battle/p/Test/Mod/".into(),
            version: String::new(),
            game_version: String::new(),
            loader: String::new(),
            include_optional: false,
        };
        let deps = dependency_links(&project, &release, &input).await.unwrap();
        assert_eq!(deps[0].version, "5.4.2301");
    }
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
    #[ignore = "Live ROUNDS provider/archive import, repeat import and authorized download in isolated temporary encrypted storage"]
    async fn rounds_live_import_round_trip() {
        import_round_trip(
            "https://thunderstore.io/c/rounds/p/CrazyCoders/RarityBundle/",
            "ROUNDS",
        )
        .await;
    }
    #[tokio::test]
    #[ignore = "Live Modrinth API and encrypted database import in isolated storage"]
    async fn modrinth_live_import_round_trip() {
        import_round_trip("https://modrinth.com/mod/sodium", "Minecraft").await;
    }
    #[test]
    fn download_redirects_stay_on_provider_cdn_hosts_and_archive_paths() {
        let edge = Url::parse("https://edge.forgecdn.net/files/8907/912/mod.jar").unwrap();
        assert!(
            redirected_artifact(
                &edge,
                "https://mediafilez.forgecdn.net/files/8907/912/mod.jar",
                "curseforge"
            )
            .is_ok()
        );
        assert!(redirected_artifact(&edge, "/files/8907/912/mod.jar", "curseforge").is_ok());
        for target in [
            "http://mediafilez.forgecdn.net/files/a.jar",
            "https://evil.example/files/a.jar",
            "https://api.curseforge.com/v1/mods",
            "https://mediafilez.forgecdn.net/private/a.jar",
            "https://mediafilez.forgecdn.net/files/a.jar?token=secret",
            "https://cdn.modrinth.com/data/a.jar",
            "https://127.0.0.1/files/a.jar",
        ] {
            assert!(
                redirected_artifact(&edge, target, "curseforge").is_err(),
                "{target}"
            );
        }
    }
    #[tokio::test]
    #[ignore = "Live provider archives in temporary encrypted storage; no production changes or game execution"]
    async fn live_download_thunderstore_lethal_company() {
        import_round_trip(
            "https://thunderstore.io/c/lethal-company/p/notnotnotswipez/MoreCompany/",
            "Lethal Company",
        )
        .await;
    }
    #[tokio::test]
    #[ignore = "Live provider archives in temporary encrypted storage; no production changes or game execution"]
    async fn live_download_modrinth_lithium() {
        import_round_trip("https://modrinth.com/mod/lithium", "Minecraft").await;
    }
    #[tokio::test]
    #[ignore = "Live provider archives in temporary encrypted storage; no production changes or game execution"]
    async fn live_download_modrinth_fabric_api() {
        import_round_trip("https://modrinth.com/mod/fabric-api", "Minecraft").await;
    }
    #[tokio::test]
    #[ignore = "Live CurseForge server credential required; temporary encrypted storage only"]
    async fn live_download_curseforge_appleskin() {
        import_round_trip(
            "https://www.curseforge.com/minecraft/mc-mods/appleskin",
            "Minecraft",
        )
        .await;
    }
    #[tokio::test]
    #[ignore = "Live CurseForge server credential required; temporary encrypted storage only"]
    async fn live_download_curseforge_ferritecore() {
        import_round_trip(
            "https://www.curseforge.com/minecraft/mc-mods/ferritecore",
            "Minecraft",
        )
        .await;
    }
    #[tokio::test]
    #[ignore = "Live CurseForge server credential required; temporary encrypted storage only"]
    async fn live_download_curseforge_cloth_config() {
        import_round_trip(
            "https://www.curseforge.com/minecraft/mc-mods/cloth-config",
            "Minecraft",
        )
        .await;
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
        assert_eq!(preview["game"], game, "Preview failed for {url}: {preview}");
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
            .unwrap_or_else(|| panic!("Import failed for {url}: {imported}"));
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
            value(call(app, "GET", "/api/v1/mods", Value::Null, Some(&member)).await)
                .await
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["id"] == id)
                .unwrap()["sha256"],
            hex::encode(Sha256::digest(bytes))
        );
        println!(
            "Verified preview, recursive import, repeat import, review gate and downloaded SHA-256: {url}"
        );
    }
    #[test]
    fn curseforge_categories_and_loader_versions_are_preserved() {
        for (category, expected) in [
            ("mc-mods", "mod"),
            ("texture-packs", "resourcepack"),
            ("shaders", "shader"),
            ("data-packs", "datapack"),
            ("modpacks", "modpack"),
        ] {
            assert!(
                link(&format!(
                    "https://www.curseforge.com/minecraft/{category}/example"
                ))
                .is_ok()
            );
            assert_eq!(cf_content_type(category), Some(expected));
        }
        assert!(link("https://www.curseforge.com/minecraft/unknown/example").is_err());
        let file = json!({"id":123,"isAvailable":true,"fileName":"example.jar","displayName":"Example", "gameVersions":["1.21.1","NeoForge","1.21.1","Fabric"],"hashes":[{"algo":1,"value":"abc"}], "dependencies":[{"modId":100,"relationType":3}],"downloadUrl":null});
        let release = cf_release(&file).unwrap();
        assert_eq!(release.loaders, ["neoforge", "fabric"]);
        assert_eq!(release.game_versions, ["1.21.1"]);
        assert_eq!(release.dependencies[0]["relationType"], 3);
        assert!(release.download.is_empty());
        assert!(cf_release(&json!({"id":123,"isAvailable":false})).is_none());
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
