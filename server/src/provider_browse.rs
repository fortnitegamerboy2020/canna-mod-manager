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
    Ok(axum::Json(
        json!({"games":game_profiles::games().iter().map(|g|json!({"id":g.app_id,"name":g.name,"community":g.community})).collect::<Vec<_>>(),"preview":true}),
    ))
}
fn valid(q: &Search) -> ApiResult<()> {
    if !matches!(
        q.provider.as_str(),
        "thunderstore" | "modrinth" | "curseforge"
    ) || q.page == 0
        || q.page > 500
        || q.q.len() > 120
        || q.category.len() > 80
        || q.version.len() > 40
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
        .await?;
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
    let games = external::metadata("https://api.curseforge.com/v1/games?pageSize=50", true).await?;
    let game = games["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|g| g["slug"] == q.game)
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
fn normalize_thunderstore(data: &Value, community: &str) -> Vec<Value> {
    data["packages"].as_array().into_iter().flatten().filter(|p|p["is_nsfw"]!=true && p["is_deprecated"]!=true).take(24).map(|p|json!({"name":p["package_name"],"description":p["description"],"authors":p["team_name"],"author_url":format!("https://thunderstore.io/c/{community}/p/{}/",p["namespace"].as_str().unwrap_or_default()),"source_url":format!("https://thunderstore.io/c/{community}/p/{}/{}/",p["namespace"].as_str().unwrap_or_default(),p["package_name"].as_str().unwrap_or_default()),"icon_url":p["image_src"],"downloads":p["download_count"],"rating":p["rating_score"],"rating_label":"upvotes","updated":p["last_updated"]})).collect()
}
#[cfg(test)]
mod tests {
    #[test]
    fn provider_search_is_bounded_and_attribution_preserved() {
        let mut q = super::Search {
            provider: "thunderstore".into(),
            game: "rounds".into(),
            q: String::new(),
            order: "downloads".into(),
            category: String::new(),
            version: String::new(),
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
