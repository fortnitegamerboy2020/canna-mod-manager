use super::*;
use reqwest::Url;
const TTL: i64 = 7 * 86400;
static REQUESTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
#[derive(Deserialize, serde::Serialize)]
pub struct Search {
    pub provider: String,
    pub game: String,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub order: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub loader: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default = "first")]
    pub page: u32,
}
fn first() -> u32 {
    1
}
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS provider_pages(key TEXT PRIMARY KEY,data TEXT NOT NULL,fetched INTEGER NOT NULL);")
}
pub async fn games(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let (cf, error) = match curseforge_games().await {
        Ok(rows) => (rows, None),
        Err(_) => (
            Vec::new(),
            Some("CurseForge games are temporarily unavailable. Retry later."),
        ),
    };
    Ok(axum::Json(
        json!({"games":supported_provider_games(&cf),"curseforge_error":error,"preview":true}),
    ))
}
fn supported_provider_games(cf: &[Value]) -> Vec<Value> {
    let available: std::collections::HashSet<u32> = cf
        .iter()
        .filter_map(|g| external::steam_id(g["slug"].as_str()?))
        .collect();
    game_profiles::games().iter().map(|g|json!({"id":g.app_id,"name":g.name,"community":g.community,"curseforge":available.contains(&g.app_id)})).collect()
}
async fn curseforge_games() -> ApiResult<Vec<Value>> {
    static CACHE: tokio::sync::Mutex<Option<(i64, Vec<Value>)>> =
        tokio::sync::Mutex::const_new(None);
    let mut cache = CACHE.lock().await;
    if let Some((at, rows)) = &*cache
        && now() - at < 1800
    {
        return Ok(rows.clone());
    }
    let rows = external::cf_pages("https://api.curseforge.com/v1/games").await?;
    *cache = Some((now(), rows.clone()));
    Ok(rows)
}
fn valid(q: &Search) -> ApiResult<()> {
    if !matches!(
        q.provider.as_str(),
        "thunderstore" | "modrinth" | "curseforge"
    ) || q.page == 0
        || q.page > 500
        || (q.provider == "curseforge" && q.page > 416)
        || q.q.len() > 120
        || q.category.len() > 80
        || q.version.len() > 40
        || !matches!(
            q.loader.as_str(),
            "" | "fabric" | "forge" | "neoforge" | "quilt"
        )
        || !matches!(
            q.content_type.as_str(),
            "" | "mod" | "shader" | "resourcepack" | "datapack"
        )
        || !matches!(
            q.order.as_str(),
            "" | "updated" | "downloads" | "rating" | "newest"
        )
    {
        return Err(bad("Invalid provider search"));
    }
    if q.game != "minecraft" && game_profiles::by_community(&q.game).is_none() {
        return Err(bad("This game family does not yet have a Canna installer"));
    }
    if q.provider == "modrinth" && q.game != "minecraft" {
        return Err(bad("This Modrinth integration supports Minecraft"));
    }
    Ok(())
}
fn url(base: &str, parameters: &[(&str, String)]) -> String {
    let mut url = Url::parse(base).unwrap();
    for (k, v) in parameters {
        if !v.is_empty() {
            url.query_pairs_mut().append_pair(k, v);
        }
    }
    url.to_string()
}
pub async fn search(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<Search>,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    app.limits.check(format!("provider-search:{user}"), 60)?;
    valid(&q)?;
    let key = serde_json::to_string(&q).unwrap();
    let cached: Option<(String, i64)> = {
        app.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT data,fetched FROM provider_pages WHERE key=?1 AND fetched>?2",
                params![key, now() - TTL],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
    };
    if let Some((ref raw, at)) = cached
        && now() - at < 1800
    {
        let mut d: Value = serde_json::from_str(raw).map_err(|_| bad("Invalid provider cache"))?;
        d["cached"] = json!(true);
        return Ok(axum::Json(d));
    }
    let _permit = REQUESTS
        .acquire()
        .await
        .map_err(|_| bad("Search unavailable"))?;
    let result = fetch(&q).await;
    let mut data = match result {
        Ok(data) => data,
        Err(e) => {
            if let Some((raw, _)) = cached {
                let mut d: Value =
                    serde_json::from_str(&raw).map_err(|_| bad("Invalid provider cache"))?;
                d["stale"] = json!(true);
                return Ok(axum::Json(d));
            }
            return Err(e);
        }
    };
    data["has_more"] = json!(
        data["has_more"] == true && q.page < if q.provider == "curseforge" { 416 } else { 500 }
    );
    data["page"] = json!(q.page);
    data["provider"] = json!(q.provider);
    data["cache_days"] = json!(7);
    let raw = data.to_string();
    if raw.len() > 1024 * 1024 {
        return Err(bad("Provider page exceeds limit"));
    }
    let db = app.db.lock().unwrap();
    db.execute(
        "DELETE FROM provider_pages WHERE fetched<=?1",
        [now() - TTL],
    )?;
    db.execute(
        "INSERT OR REPLACE INTO provider_pages VALUES(?1,?2,?3)",
        params![key, raw, now()],
    )?;
    db.execute("DELETE FROM provider_pages WHERE key NOT IN (SELECT key FROM provider_pages ORDER BY fetched DESC LIMIT 256)",[])?;
    Ok(axum::Json(data))
}
async fn fetch(q: &Search) -> ApiResult<Value> {
    if q.provider == "thunderstore" {
        if let Some((at, rows)) = THUNDERSTORE_LISTINGS.lock().await.get(&q.game)
            && now() - at < 1800
        {
            return Ok(public_list_page(rows, q));
        }
        let ordering = match q.order.as_str() {
            "downloads" => "most-downloaded",
            "rating" => "top-rated",
            "newest" => "newest",
            _ => "last-updated",
        };
        let data = external::metadata(
            &url(
                &format!(
                    "https://thunderstore.io/api/experimental/frontend/c/{}/packages/",
                    q.game
                ),
                &[
                    ("q", q.q.clone()),
                    ("page", q.page.to_string()),
                    ("ordering", ordering.into()),
                    ("included_categories", q.category.clone()),
                    ("nsfw", "false".into()),
                    ("deprecated", "false".into()),
                ],
            ),
            false,
        )
        .await;
        let data = match data {
            Ok(data) => data,
            Err(_) => return thunderstore_public(q).await,
        };
        return Ok(
            json!({"items":normalize_thunderstore(&data,&q.game),"categories":data["categories"],"has_more":data["has_more_pages"]}),
        );
    }
    if q.provider == "modrinth" {
        let index = match q.order.as_str() {
            "downloads" => "downloads",
            "rating" => "follows",
            "newest" => "newest",
            _ => "updated",
        };
        let mut facets = vec![
            vec![
                "project_type:mod",
                "project_type:resourcepack",
                "project_type:shader",
                "project_type:datapack",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>(),
        ];
        if !q.content_type.is_empty() {
            facets[0] = vec![format!("project_type:{}", q.content_type)];
        }
        if !q.loader.is_empty() {
            facets.push(vec![format!("categories:{}", q.loader)]);
        }
        if !q.category.is_empty() {
            facets.push(vec![format!("categories:{}", q.category)]);
        }
        if !q.version.is_empty() {
            facets.push(vec![format!("versions:{}", q.version)]);
        }
        let data = external::metadata(
            &url(
                "https://api.modrinth.com/v2/search",
                &[
                    ("query", q.q.clone()),
                    ("index", index.into()),
                    ("limit", "24".into()),
                    ("offset", ((q.page - 1) * 24).to_string()),
                    ("facets", serde_json::to_string(&facets).unwrap()),
                ],
            ),
            false,
        )
        .await?;
        let items=data["hits"].as_array().into_iter().flatten().take(24).map(|v|json!({"name":v["title"],"description":v["description"],"authors":v["author"],"author_url":format!("https://modrinth.com/user/{}",v["author"].as_str().unwrap_or_default()),"source_url":format!("https://modrinth.com/{}/{}",v["project_type"].as_str().unwrap_or("mod"),v["slug"].as_str().unwrap_or_default()),"icon_url":v["icon_url"],"downloads":v["downloads"],"rating":v["follows"],"rating_label":"followers","updated":v["date_modified"]})).collect::<Vec<_>>();
        let categories = external::metadata("https://api.modrinth.com/v2/tag/category", false)
            .await?
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v["project_type"] == "mod")
            .map(|v| json!({"id":v["name"],"name":v["name"]}))
            .collect::<Vec<_>>();
        return Ok(
            json!({"items":items,"categories":categories,"has_more":data["total_hits"].as_u64().unwrap_or(0)>q.page as u64*24}),
        );
    }
    let games = curseforge_games().await?;
    let game = games
        .iter()
        .find(|g| {
            external::steam_id(g["slug"].as_str().unwrap_or("")) == external::steam_id(&q.game)
        })
        .ok_or_else(|| bad("CurseForge does not expose this game through the configured API"))?;
    let game_id = game["id"]
        .as_u64()
        .ok_or_else(|| bad("Invalid provider game"))?;
    let order = match q.order.as_str() {
        "downloads" => 6,
        "rating" => 2,
        "newest" => 11,
        _ => 3,
    };
    let data = external::metadata(
        &url(
            "https://api.curseforge.com/v1/mods/search",
            &[
                ("gameId", game_id.to_string()),
                ("searchFilter", q.q.clone()),
                ("sortField", order.to_string()),
                ("sortOrder", "desc".into()),
                ("pageSize", "24".into()),
                ("index", ((q.page - 1) * 24).to_string()),
                ("categoryId", q.category.clone()),
                ("gameVersion", q.version.clone()),
                (
                    "modLoaderType",
                    match q.loader.as_str() {
                        "forge" => "1",
                        "fabric" => "4",
                        "quilt" => "5",
                        "neoforge" => "6",
                        _ => "",
                    }
                    .into(),
                ),
                (
                    "classId",
                    if q.game == "minecraft" {
                        match q.content_type.as_str() {
                            "mod" => "6",
                            "resourcepack" => "12",
                            "shader" => "6552",
                            "datapack" => "6945",
                            _ => "",
                        }
                    } else {
                        ""
                    }
                    .into(),
                ),
            ],
        ),
        true,
    )
    .await?;
    let items=data["data"].as_array().into_iter().flatten().take(24).map(|v|json!({"name":v["name"],"description":v["summary"],"authors":v["authors"].as_array().into_iter().flatten().filter_map(|a|a["name"].as_str()).collect::<Vec<_>>().join(", "),"author_url":v["authors"][0]["url"],"source_url":v["links"]["websiteUrl"],"icon_url":v["logo"]["thumbnailUrl"],"downloads":v["downloadCount"],"rating":null,"rating_label":"popularity","updated":v["dateModified"]})).collect::<Vec<_>>();
    let categories = external::metadata(
        &format!("https://api.curseforge.com/v1/categories?gameId={game_id}"),
        true,
    )
    .await?["data"]
        .clone();
    Ok(
        json!({"items":items,"categories":categories,"has_more":data["pagination"]["totalCount"].as_u64().unwrap_or(0)>q.page as u64*24}),
    )
}
// Cache compact metadata only, without archives or full version histories.
type CommunityCache = std::collections::BTreeMap<String, (i64, std::sync::Arc<Vec<Value>>)>;
static THUNDERSTORE_LISTINGS: std::sync::LazyLock<tokio::sync::Mutex<CommunityCache>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(std::collections::BTreeMap::new()));
async fn thunderstore_public(q: &Search) -> ApiResult<Value> {
    let mut cache = THUNDERSTORE_LISTINGS.lock().await;
    let rows = if let Some((at, rows)) = cache.get(&q.game)
        && now() - at < 1800
    {
        rows.clone()
    } else {
        let data = external::metadata_limited(
            &format!("https://thunderstore.io/c/{}/api/v1/package/", q.game),
            false,
            64 * 1024 * 1024,
        )
        .await?;
        let rows = std::sync::Arc::new(normalize_public_list(&data, &q.game)?);
        if cache.len() >= 4
            && !cache.contains_key(&q.game)
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest);
        }
        cache.insert(q.game.clone(), (now(), rows.clone()));
        rows
    };
    drop(cache);
    Ok(public_list_page(&rows, q))
}
fn normalize_public_list(data: &Value, community: &str) -> ApiResult<Vec<Value>> {
    let packages = data
        .as_array()
        .ok_or_else(|| bad("Invalid Thunderstore package index"))?;
    let mut rows = Vec::new();
    for p in packages
        .iter()
        .filter(|p| p["has_nsfw_content"] != true && p["is_deprecated"] != true)
    {
        let Some(version) = p["versions"]
            .as_array()
            .and_then(|v| v.iter().find(|v| v["is_active"] != false))
        else {
            continue;
        };
        let name = p["name"].as_str().unwrap_or_default();
        let owner = p["owner"].as_str().unwrap_or_default();
        if name.is_empty() || owner.is_empty() {
            continue;
        }
        let downloads = p["versions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["downloads"].as_u64())
            .fold(0u64, u64::saturating_add);
        rows.push(json!({"name":name,"description":version["description"],"authors":owner,
            "author_url":format!("https://thunderstore.io/c/{community}/p/{owner}/"),
            "source_url":format!("https://thunderstore.io/c/{community}/p/{owner}/{name}/"),
            "icon_url":version["icon"],"downloads":downloads,"rating":p["rating_score"],
            "rating_label":"upvotes","updated":p["date_updated"],"created":p["date_created"],"categories":p["categories"]}));
    }
    if serde_json::to_vec(&rows)
        .map_err(|_| bad("Invalid Thunderstore index"))?
        .len()
        > 8 * 1024 * 1024
    {
        return Err(bad("Thunderstore listing index exceeds limit"));
    }
    Ok(rows)
}
fn public_list_page(rows: &[Value], q: &Search) -> Value {
    let categories: std::collections::BTreeSet<&str> = rows
        .iter()
        .flat_map(|v| v["categories"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    let query = q.q.to_lowercase();
    let mut matching: Vec<&Value> = rows
        .iter()
        .filter(|v| {
            (q.category.is_empty()
                || v["categories"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|c| c.as_str() == Some(q.category.as_str()))))
                && (query.is_empty()
                    || ["name", "description", "authors"].iter().any(|k| {
                        v[*k]
                            .as_str()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&query)
                    }))
        })
        .collect();
    matching.sort_by(|a, b| {
        let order = match q.order.as_str() {
            "downloads" => b["downloads"].as_u64().cmp(&a["downloads"].as_u64()),
            "rating" => b["rating"].as_u64().cmp(&a["rating"].as_u64()),
            "newest" => b["created"].as_str().cmp(&a["created"].as_str()),
            _ => b["updated"].as_str().cmp(&a["updated"].as_str()),
        };
        order.then_with(|| a["source_url"].as_str().cmp(&b["source_url"].as_str()))
    });
    let start = (q.page as usize - 1) * 24;
    json!({"items":matching.iter().skip(start).take(24).collect::<Vec<_>>(),
        "categories":categories.into_iter().map(|c| json!({"id":c,"name":c})).collect::<Vec<_>>(),
        "has_more":matching.len()>start+24})
}
fn normalize_thunderstore(data: &Value, community: &str) -> Vec<Value> {
    data["packages"].as_array().into_iter().flatten().filter(|p|p["is_nsfw"]!=true && p["is_deprecated"]!=true).take(24).map(|p|json!({"name":p["package_name"],"description":p["description"],"authors":p["team_name"],"author_url":format!("https://thunderstore.io/c/{community}/p/{}/",p["namespace"].as_str().unwrap_or_default()),"source_url":format!("https://thunderstore.io/c/{community}/p/{}/{}/",p["namespace"].as_str().unwrap_or_default(),p["package_name"].as_str().unwrap_or_default()),"icon_url":p["image_src"],"downloads":p["download_count"],"rating":p["rating_score"],"rating_label":"upvotes","updated":p["last_updated"]})).collect()
}
#[cfg(test)]
mod tests {
    use super::{Value, json};
    #[test]
    fn public_index_filters_sorts_and_pages_without_losing_attribution() {
        let packages: Vec<Value> = (0..50).map(|n| json!({
            "name":format!("Mod{n:02}"),"owner":"Author","rating_score":n,
            "categories":[if n % 2 == 0 { "Tools" } else { "Cards" }],
            "has_nsfw_content":n==49,"is_deprecated":n==48,
            "versions":[{"is_active":true,"description":"Description","downloads":n,"icon":"https://ccdn.thunderstore.io/icon.png"},{"downloads":10}]
        })).collect();
        let rows = super::normalize_public_list(&json!(packages), "rounds").unwrap();
        assert_eq!(rows.len(), 48);
        let mut q = super::Search {
            provider: "thunderstore".into(),
            game: "rounds".into(),
            q: String::new(),
            order: "downloads".into(),
            category: String::new(),
            version: String::new(),
            loader: String::new(),
            content_type: String::new(),
            page: 1,
        };
        let first = super::public_list_page(&rows, &q);
        assert_eq!(first["items"].as_array().unwrap().len(), 24);
        assert_eq!(first["items"][0]["name"], "Mod47");
        assert_eq!(first["items"][0]["downloads"], 57);
        assert_eq!(first["has_more"], true);
        assert_eq!(
            first["items"][0]["source_url"],
            "https://thunderstore.io/c/rounds/p/Author/Mod47/"
        );
        q.page = 2;
        let second = super::public_list_page(&rows, &q);
        assert_eq!(second["items"][0]["name"], "Mod23");
        assert_eq!(second["has_more"], false);
        q.page = 1;
        q.category = "Tools".into();
        q.q = "mod02".into();
        let filtered = super::public_list_page(&rows, &q);
        assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["items"][0]["name"], "Mod02");
        assert!(super::normalize_public_list(&json!({}), "rounds").is_err());
    }
    #[tokio::test]
    #[ignore = "live provider metadata only"]
    async fn live_public_thunderstore_browsing() {
        for game in ["bopl-battle", "rounds"] {
            let mut q = super::Search {
                provider: "thunderstore".into(),
                game: game.into(),
                q: String::new(),
                order: "downloads".into(),
                category: String::new(),
                version: String::new(),
                loader: String::new(),
                content_type: String::new(),
                page: 1,
            };
            let first = super::fetch(&q).await.unwrap();
            assert_eq!(first["items"].as_array().unwrap().len(), 24);
            q.page = 2;
            let second = super::fetch(&q).await.unwrap();
            assert_eq!(second["items"].as_array().unwrap().len(), 24);
            assert_ne!(
                first["items"][0]["source_url"],
                second["items"][0]["source_url"]
            );
            println!(
                "{game}: 24 listings per page, distinct pages, {} categories",
                first["categories"].as_array().unwrap().len()
            );
        }
    }
    #[tokio::test]
    async fn browsing_cached_metadata_never_imports_archives() {
        use crate::tests::{account, call, fixture, value};
        let (_dir, app) = fixture();
        let token = account(&app, "browser", false);
        let q = super::Search {
            provider: "modrinth".into(),
            game: "minecraft".into(),
            q: "fixture".into(),
            order: "downloads".into(),
            category: String::new(),
            version: String::new(),
            loader: String::new(),
            content_type: String::new(),
            page: 1,
        };
        let key = serde_json::to_string(&q).unwrap();
        app.db.lock().unwrap().execute("INSERT INTO provider_pages VALUES(?1,?2,?3)",super::params![key,serde_json::json!({"items":[{"name":"Metadata only"}],"has_more":true,"categories":[]}).to_string(),super::now()]).unwrap();
        let response = call(
            app.clone(),
            "GET",
            "/api/v1/providers/search?provider=modrinth&game=minecraft&q=fixture&order=downloads",
            serde_json::json!({}),
            Some(&token),
        )
        .await;
        assert_eq!(response.status(), super::StatusCode::OK);
        assert_eq!(value(response).await["items"][0]["name"], "Metadata only");
        assert_eq!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM mods", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(std::fs::read_dir(&app.files).unwrap().count(), 0);
    }
    #[test]
    fn provider_search_is_bounded_and_attribution_preserved() {
        let mut q = super::Search {
            provider: "thunderstore".into(),
            game: "rounds".into(),
            q: String::new(),
            order: "downloads".into(),
            category: String::new(),
            version: String::new(),
            loader: String::new(),
            content_type: String::new(),
            page: 1,
        };
        assert!(super::valid(&q).is_ok());
        q.game = "../../secret".into();
        assert!(super::valid(&q).is_err());
        q.game = "rounds".into();
        q.page = 501;
        assert!(super::valid(&q).is_err());
        let rows = super::normalize_thunderstore(
            &serde_json::json!({"packages":[{"package_name":"Mod","namespace":"Author","team_name":"Author","download_count":25,"rating_score":3},{"is_nsfw":true}]}),
            "rounds",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["downloads"], 25);
        assert_eq!(
            rows[0]["source_url"],
            "https://thunderstore.io/c/rounds/p/Author/Mod/"
        );
    }
}

#[cfg(test)]
mod supported_game_tests {
    use super::*;
    #[test]
    fn provider_games_are_registry_scoped_and_curseforge_aliases_match() {
        let rows = supported_provider_games(&[
            json!({"slug":"riskofrain2"}),
            json!({"slug":"rounds"}),
            json!({"slug":"unsupported-game"}),
        ]);
        assert_eq!(rows.len(), game_profiles::games().len());
        assert!(
            rows.iter()
                .all(|g| game_profiles::supports_game(g["id"].as_u64().unwrap() as u32))
        );
        assert!(
            rows.iter()
                .any(|g| g["id"] == 632360 && g["curseforge"] == true)
        );
        assert!(
            rows.iter()
                .any(|g| g["id"] == 1686940 && g["curseforge"] == false)
        );
        assert!(rows.iter().all(|g| g["community"] != "unsupported-game"));
    }
}
