use super::*;
/// Exact approved archive metadata, including historical releases hidden by listings.
pub async fn manifest(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    Uuid::parse_str(&id).map_err(|_| bad("Invalid mod ID"))?;
    let db = app.db.lock().unwrap();
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM mods WHERE id=?1)",
        [&id],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(ApiError(StatusCode::NOT_FOUND, "Mod no longer available"));
    }
    security::approved(&db, &id)?;
    let (app_id, name, version, description, hash): (u32, String, String, String, String) = db
        .query_row(
            "SELECT app_id,name,version,description,sha256 FROM mods WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
    let details = external::details(&db, &id)?;
    let mut dependencies = Vec::new();
    for dependency in details["dependency_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if external::details(&db, dependency)?["framework_root"].is_string() {
            continue;
        }
        dependencies.push(db.query_row(
            "SELECT name FROM mods WHERE id=?1",
            [dependency],
            |r| r.get::<_, String>(0),
        )?);
    }
    Ok(axum::Json(
        json!({"app_id":app_id,"item":{"enabled":true,"name":name,"version":version,"description":description,"file":format!("Mods/{id}.zip"),"sha256":hash,"dependencies":dependencies,"provenance":details,"content_type":details["project_type"].as_str().or(details["content_type"].as_str()).unwrap_or("mod")}}),
    ))
}
#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}
pub fn recommendations() -> Vec<Value> {
    serde_json::from_str(include_str!("../web/source-recommendations.json"))
        .expect("Curated Source catalog")
}
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS game_assets(id TEXT PRIMARY KEY,alias TEXT UNIQUE NOT NULL,sha256 TEXT NOT NULL,size INTEGER NOT NULL,game_id INTEGER NOT NULL,kind TEXT NOT NULL);")?;
    let rows=db.prepare("SELECT d.mod_id,d.data,m.description FROM mod_details d JOIN mods m ON m.id=d.mod_id WHERE json_extract(d.data,'$.provider')='catalog'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
    for (id, raw, description) in rows {
        let Ok(mut data) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if data["authors"].as_str().is_some_and(|s| !s.is_empty()) {
            continue;
        }
        if let Some((_, author)) = description.rsplit_once("Author: ") {
            let author = author
                .split(". Canna extension:")
                .next()
                .unwrap_or(author)
                .trim()
                .trim_end_matches('.');
            if !author.is_empty() && author.len() <= 120 {
                data["authors"] = json!(author);
                db.execute(
                    "UPDATE mod_details SET data=?1 WHERE mod_id=?2",
                    params![data.to_string(), id],
                )?;
            }
        }
    }
    Ok(())
}
pub async fn list(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt =
        db.prepare("SELECT id,app_id,name,version,description,sha256 FROM mods WHERE NOT EXISTS(SELECT 1 FROM mod_reviews WHERE mod_id=mods.id AND approved=0) ORDER BY rowid DESC")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let names: Vec<String> = rows.iter().map(|r| r.2.clone()).collect();
    let mut games: std::collections::BTreeMap<u32, Value> = Default::default();
    for (id, name, folder, framework) in [
        (1686940, "Bopl Battle", "bopl-battle", "bepinex"),
        (1557740, "ROUNDS", "rounds", "bepinex"),
        (550, "Left 4 Dead 2", "left-4-dead-2", "source-vpk"),
        (500, "Left 4 Dead", "left-4-dead", "source-vpk"),
    ] {
        games.insert(id, json!({"app_id":id,"name":name,"folder":folder,"framework":framework,"icon":match id {1686940=>"icon.png",1557740=>"game.jpg",_=>""},"description":if framework=="source-vpk" {"VPK addon packs; modded launches use -insecure practice mode"} else {"Unity modpacks with BepInEx"},"mods":[],"mod_folder_status":"Server library ready"}));
    }
    for profile in game_profiles::games() {
        games.entry(profile.app_id).or_insert_with(||json!({"app_id":profile.app_id,"name":profile.name,"folder":profile.folder,"framework":profile.loader,"icon":"","description":format!("Thunderstore {} profile - preview; requires a compatible reviewed loader",profile.loader),"mods":[],"mod_folder_status":"Game profile available"}));
    }
    let mut latest = std::collections::HashSet::new();
    for (id, appid, name, version, description, hash) in rows {
        if security::approved(&db, &id).is_err() {
            continue;
        }
        let appid = if appid == 0 { u32::MAX } else { appid };
        if !game_profiles::supports_game(appid) {
            continue;
        }
        let d = external::details(&db, &id)?;
        if let Some(key) = project_key(&d)
            && !latest.insert(key)
        {
            continue;
        }
        // Loader distributions are installed through Framework, never as plugin DLLs.
        if d["provider"] == "thunderstore"
            && (d["framework_root"].is_string() || name.starts_with("BepInExPack"))
        {
            continue;
        }
        let folder = d["folder"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| match appid {
                u32::MAX => "minecraft".into(),
                1686940 => "bopl-battle".into(),
                1557740 => "rounds".into(),
                other => format!("steam-{other}"),
            });
        let game_name = d["game"].as_str().map(str::to_owned).unwrap_or_else(|| {
            if appid == u32::MAX {
                "Minecraft".into()
            } else {
                format!("Steam game {appid}")
            }
        });
        let game=games.entry(appid).or_insert_with(||json!({"app_id":appid,"name":game_name,"folder":folder,"description":"Canna community catalog","icon":"icon.png","mod_folder_status":"Server library ready","mods":[]}));
        let deps: Vec<String> = if let Some(ids) = d["dependency_ids"].as_array() {
            ids.iter()
                .filter_map(Value::as_str)
                .filter(|id| {
                    external::details(&db, id).is_ok_and(|d| !d["framework_root"].is_string())
                })
                .map(|id| {
                    db.query_row("SELECT name FROM mods WHERE id=?1", [id], |r| {
                        r.get::<_, String>(0)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|name: &String| !name.starts_with("BepInExPack"))
                .collect()
        } else {
            d["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
                .filter_map(|dep| {
                    if dep.starts_with("BepInEx-") {
                        return None;
                    }
                    if names.iter().any(|n| n == dep) {
                        return Some(dep.to_owned());
                    }
                    let candidate = dep.split('-').nth(1).unwrap_or(dep);
                    Some(candidate.to_owned())
                })
                .collect()
        };
        game["mods"].as_array_mut().unwrap().push(json!({"enabled":true,"provenance":d,"content_type":d["project_type"].as_str().or(d["content_type"].as_str()).unwrap_or("mod"),"name":name,"version":version,"description":description,"file":format!("Mods/{id}.zip"),"sha256":hash,"dependencies":deps}));
    }
    for item in recommendations() {
        if let Some(game) = games.get_mut(&(item["app_id"].as_u64().unwrap() as u32)) {
            game["mods"].as_array_mut().unwrap().push(json!({"enabled":false,"provenance":item["details"],"content_type":"mod","name":item["name"],"version":item["version"],"description":item["description"],"file":format!("Mods/{}.zip",item["id"].as_str().unwrap()),"sha256":"","dependencies":item["details"]["dependencies"]}));
        }
    }
    Ok(axum::Json(
        json!({"games":games.into_values().collect::<Vec<_>>()}),
    ))
}
pub fn project_key(d: &Value) -> Option<String> {
    let url = d["source_url"].as_str()?;
    if !matches!(
        d["provider"].as_str(),
        Some("github" | "thunderstore" | "modrinth" | "curseforge")
    ) {
        return None;
    }
    let (loader, version) = external::update_profile(d);
    Some(format!("{}|{}|{}", url, loader, version))
}
fn framework_game(path: &str) -> Option<u32> {
    match path {
        "bopl-battle/Framework/BepInEx.zip" => Some(1686940),
        "rounds/Framework/BepInEx.zip" => Some(1557740),
        _ => game_profiles::games()
            .iter()
            .find(|g| {
                path == format!(
                    "{}/Framework/{}.zip",
                    g.folder,
                    match g.loader.as_str() {
                        "gdweave" => "GDWeave",
                        "return-of-modding" => "ReturnOfModding",
                        _ => "BepInEx",
                    }
                )
            })
            .map(|g| g.app_id),
    }
}
fn reviewed_framework(db: &Connection, app_id: u32) -> ApiResult<Option<(String, i64, String)>> {
    let mut stmt = db.prepare("SELECT m.id,m.size,m.sha256 FROM mods m JOIN mod_details d ON d.mod_id=m.id WHERE (m.app_id=?1 OR EXISTS(SELECT 1 FROM mods parent JOIN mod_details pd ON pd.mod_id=parent.id JOIN json_each(pd.data,'$.dependency_ids') dep WHERE parent.app_id=?1 AND dep.value=m.id)) AND json_type(d.data,'$.framework_root')='text' ORDER BY m.rowid DESC LIMIT 128")?;
    let rows = stmt
        .query_map([app_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for row in rows {
        let details = external::details(db, &row.0)?;
        let kind = game_profiles::by_id(app_id)
            .map(|p| p.loader.as_str())
            .unwrap_or("bepinex");
        if details["framework_kind"].as_str().unwrap_or("bepinex") != kind {
            continue;
        }
        match security::approved(db, &row.0) {
            Ok(()) => return Ok(Some(row)),
            Err(error) if error.0.is_server_error() => return Err(error),
            Err(_) => continue,
        }
    }
    Ok(None)
}
pub async fn file(
    State(app): State<Shared>,
    headers: HeaderMap,
    Query(q): Query<FileQuery>,
) -> ApiResult<Response> {
    app.auth(&headers)?;
    if q.path.len() > 300
        || q.path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
        || q.path.contains(['\\', ':', '%'])
    {
        return Err(bad("Invalid catalog path"));
    }
    let (id, size, hash, artwork) = {
        let db = app.db.lock().unwrap();
        // Legacy imported asset aliases are not reviewed mod records. Prefer
        // an actual approved loader, including its complete dependency graph.
        let reviewed_loader = framework_game(&q.path)
            .map(|app_id| reviewed_framework(&db, app_id))
            .transpose()?
            .flatten();
        let asset: Option<(String, i64, String, String)> = db
            .query_row(
                "SELECT id,size,sha256,kind FROM game_assets WHERE alias=?1",
                [&q.path],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((id, size, hash)) = reviewed_loader {
            (id, size, hash, false)
        } else if let Some((id, size, hash, kind)) = asset {
            (id, size, hash, kind == "icon")
        } else {
            let filename = q
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .trim_end_matches(".zip");
            let ordinary = db.query_row("SELECT m.id,m.size,m.sha256 FROM mods m LEFT JOIN mod_details d ON d.mod_id=m.id WHERE m.id=?1 OR json_extract(d.data,'$.catalog_file')=?2",params![filename,q.path],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if let Some(row) = ordinary {
                (row.0, row.1, row.2, false)
            } else {
                let app_id = framework_game(&q.path)
                    .ok_or(ApiError(StatusCode::NOT_FOUND, "Catalog file unavailable"))?;
                let row = reviewed_framework(&db, app_id)?.ok_or(ApiError(StatusCode::NOT_FOUND,"Import a mod with its official loader dependencies, then complete its review before installing"))?;
                (row.0, row.1, row.2, false)
            }
        }
    };
    // Icons are trusted server-imported assets, not user-submitted executable mods.
    // Every other asset and mod retains the approval and dependency gates.
    if !artwork {
        security::approved(&app.db.lock().unwrap(), &id)?;
        provider_cache::ensure(&app, &id).await?;
    }
    let file = tokio::fs::File::open(app.files.join(format!("{id}.zip"))).await?;
    Ok((
        [
            ("content-type", "application/octet-stream".into()),
            ("content-length", size.to_string()),
            ("x-canna-sha256", hash),
        ],
        Body::from_stream(crypto::read(file, Zeroizing::new(*app.upload_key), id)),
    )
        .into_response())
}
pub async fn migrate(app: &App, path: &std::path::Path) -> anyhow::Result<()> {
    let root = path.parent().unwrap().canonicalize()?;
    let entries: Vec<Value> = serde_json::from_slice(&std::fs::read(path)?)?;
    for a in entries {
        let alias = a["alias"].as_str().unwrap();
        let exists: bool = app.db.lock().unwrap().query_row(
            "SELECT EXISTS(SELECT 1 FROM game_assets WHERE alias=?1)",
            [alias],
            |r| r.get(0),
        )?;
        if exists {
            continue;
        }
        let source = root
            .join(a["local_file"].as_str().unwrap())
            .canonicalize()?;
        anyhow::ensure!(source.starts_with(&root), "Asset outside staging directory");
        let bytes = std::fs::read(source)?;
        let hash = hex::encode(Sha256::digest(&bytes));
        anyhow::ensure!(a["sha256"] == hash, "Asset checksum mismatch");
        let id = Uuid::new_v4().to_string();
        let file = tokio::fs::File::create(app.files.join(format!("{id}.zip"))).await?;
        let mut writer = crypto::Writer::new(file, &app.upload_key, id.clone()).await?;
        writer.write(&bytes).await?;
        writer.finish().await?;
        app.db.lock().unwrap().execute(
            "INSERT INTO game_assets VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                id,
                alias,
                hash,
                bytes.len() as i64,
                a["game_id"].as_u64().unwrap_or_default() as u32,
                a["kind"].as_str().unwrap_or("asset")
            ],
        )?;
        println!("Imported asset {alias}");
    }
    Ok(())
}
pub async fn audit(app: &App) -> anyhow::Result<()> {
    use futures_util::TryStreamExt;
    let rows = {
        let db = app.db.lock().unwrap();
        db.prepare(
            "SELECT id,sha256,size,EXISTS(SELECT 1 FROM provider_archives p WHERE p.mod_id=mods.id AND p.evicted=1) FROM mods UNION ALL SELECT id,sha256,size,0 FROM game_assets",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let mut cached_out = 0;
    for (id, expected, size, evicted) in &rows {
        if *evicted && !app.files.join(format!("{id}.zip")).exists() {
            cached_out += 1;
            continue;
        }
        let file = tokio::fs::File::open(app.files.join(format!("{id}.zip"))).await?;
        let stream = crypto::read(file, Zeroizing::new(*app.upload_key), id.clone());
        tokio::pin!(stream);
        let mut hash = Sha256::new();
        let mut actual = 0i64;
        while let Some(bytes) = stream.try_next().await? {
            actual += bytes.len() as i64;
            hash.update(bytes);
        }
        anyhow::ensure!(
            actual == *size && hex::encode(hash.finalize()) == *expected,
            "Stored artifact checksum mismatch"
        );
    }
    let db = app.db.lock().unwrap();
    let count: i64 = db.query_row("SELECT COUNT(*) FROM mods", [], |r| r.get(0))?;
    let assets: i64 = db.query_row("SELECT COUNT(*) FROM game_assets", [], |r| r.get(0))?;
    println!(
        "Audited {count} mod records and {assets} game assets; {cached_out} provider archives intentionally expired, remaining files verified"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn exact_manifest_requires_membership_approval_and_preserves_historical_pins() {
        let (_dir, app) = fixture();
        let token = account(&app, "manifest-member", false);
        let old=external::store(&app,1,1686940,"Pinned fixture","1.0.0","","manifest:old",&json!({"provider":"thunderstore","id":"Team-Pinned","source_url":"https://thunderstore.io/c/bopl-battle/p/Team/Pinned/"}),b"PK\x03\x04fixture-old").await.unwrap();
        let new=external::store(&app,1,1686940,"Pinned fixture","2.0.0","","manifest:new",&json!({"provider":"thunderstore","id":"Team-Pinned","source_url":"https://thunderstore.io/c/bopl-battle/p/Team/Pinned/"}),b"PK\x03\x04fixture-new").await.unwrap();
        let path = format!("/api/v1/mods/{old}/manifest");
        assert_eq!(
            call(app.clone(), "GET", &path, json!({}), None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert!(
            !call(app.clone(), "GET", &path, json!({}), Some(&token))
                .await
                .status()
                .is_success()
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_reviews SET approved=1 WHERE mod_id IN (?1,?2)",
                params![old, new],
            )
            .unwrap();
        let data = value(call(app.clone(), "GET", &path, json!({}), Some(&token)).await).await;
        assert_eq!(data["item"]["version"], "1.0.0");
        assert_eq!(data["item"]["file"], format!("Mods/{old}.zip"));
        assert_eq!(data["app_id"], 1686940);
        assert_eq!(
            data["item"]["sha256"],
            hex::encode(Sha256::digest(b"PK\x03\x04fixture-old"))
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                "/api/v1/mods/00000000-0000-4000-8000-000000000000/manifest",
                json!({}),
                Some(&token)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        app.db.lock().unwrap().execute("INSERT INTO mod_scans VALUES(?1,(SELECT sha256 FROM mods WHERE id=?1),'complete',?2,0)",params![old,json!({"findings":[{"id":"unaccepted","accepted":false}]}).to_string()]).unwrap();
        assert!(
            !call(app, "GET", &path, json!({}), Some(&token))
                .await
                .status()
                .is_success()
        );
    }
    #[test]
    fn legacy_catalog_credits_original_creator_and_preserves_extension_notes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE mods(id TEXT,description TEXT); CREATE TABLE mod_details(mod_id TEXT,data TEXT); INSERT INTO mods VALUES('arrow','Example. Author: WackyModer.'),('extension','Author: Obelous. Canna extension: F9 controls.'); INSERT INTO mod_details VALUES('arrow','{\"provider\":\"catalog\"}'),('extension','{\"provider\":\"catalog\"}');").unwrap();
        super::initialize(&db).unwrap();
        for (id, author) in [("arrow", "WackyModer"), ("extension", "Obelous")] {
            let data: String = db
                .query_row("SELECT data FROM mod_details WHERE mod_id=?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&data).unwrap()["authors"],
                author
            );
        }
    }
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[tokio::test]
    async fn reviewed_loader_wins_over_legacy_asset_and_newer_unreviewed_release() {
        let (_dir, app) = fixture();
        let token = account(&app, "loader-alias-owner", true);
        let bytes = b"PK\x03\x04approved loader";
        let loader = external::store(
            &app,
            1,
            1686940,
            "BepInExPack",
            "1",
            "",
            "loader-alias:1",
            &json!({"framework_root":"BepInExPack","provider":"thunderstore"}),
            bytes,
        )
        .await
        .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_reviews SET approved=1 WHERE mod_id=?1",
                [&loader],
            )
            .unwrap();
        let newer = external::store(
            &app,
            1,
            1686940,
            "BepInExPack",
            "2",
            "",
            "loader-alias:2",
            &json!({"framework_root":"BepInExPack","provider":"thunderstore"}),
            b"PK\x03\x04unreviewed",
        )
        .await
        .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_reviews SET approved=0 WHERE mod_id=?1",
                [&newer],
            )
            .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO game_assets VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    "legacy-loader-asset",
                    "bopl-battle/Framework/BepInEx.zip",
                    "b".repeat(64),
                    12,
                    1686940,
                    "framework"
                ],
            )
            .unwrap();
        let path = "/api/v1/catalog/file?path=bopl-battle%2FFramework%2FBepInEx.zip";
        assert_eq!(
            call(app.clone(), "GET", path, Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = call(app.clone(), "GET", path, Value::Null, Some(&token)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["x-canna-sha256"],
            hex::encode(Sha256::digest(bytes))
        );
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        {
            let db = app.db.lock().unwrap();
            db.execute(
                "UPDATE mod_reviews SET approved=1 WHERE mod_id=?1",
                [&newer],
            )
            .unwrap();
            db.execute("UPDATE mod_details SET data=json_set(data,'$.dependency_ids',json('[\"missing-required-library\"]')) WHERE mod_id=?1", [&newer]).unwrap();
        }
        // A checked approval flag alone cannot bypass a blocked dependency.
        let response = call(app.clone(), "GET", path, Value::Null, Some(&token)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["x-canna-sha256"],
            hex::encode(Sha256::digest(bytes))
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_reviews SET approved=0 WHERE mod_id=?1",
                [&loader],
            )
            .unwrap();
        assert_eq!(
            call(app.clone(), "GET", path, Value::Null, Some(&token))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn shared_reviewed_loader_resolves_through_game_dependencies() {
        let (_dir, app) = fixture();
        let token = account(&app, "loader-owner", true);
        let bytes = b"PK\x03\x04shared loader";
        let loader = external::store(
            &app,
            1,
            892970,
            "BepInExPack",
            "1",
            "",
            "loader:1",
            &json!({"framework_root":"BepInExPack","provider":"thunderstore"}),
            bytes,
        )
        .await
        .unwrap();
        external::store(
            &app,
            1,
            1966720,
            "LC mod",
            "1",
            "",
            "lc:1",
            &json!({"dependency_ids":[loader]}),
            b"PK\x03\x04mod",
        )
        .await
        .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_reviews SET approved=1", [])
            .unwrap();
        let path = format!(
            "/api/v1/catalog/file?path={}/Framework/BepInEx.zip",
            game_profiles::by_id(1966720).unwrap().folder
        );
        let response = call(app.clone(), "GET", &path, json!({}), Some(&token)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_reviews SET approved=0 WHERE mod_id=?1",
                [loader],
            )
            .unwrap();
        assert_ne!(
            call(app.clone(), "GET", &path, json!({}), Some(&token))
                .await
                .status(),
            StatusCode::OK
        );
    }
    #[tokio::test]
    async fn updates_keep_previous_download_until_new_scan_is_approved() {
        let (_dir, app) = fixture();
        let token = account(&app, "update-owner", true);
        let d = json!({"provider":"modrinth","source_url":"https://modrinth.com/mod/fixture","game":"Minecraft","loaders":["fabric"],"game_versions":["1.21.1"]});
        let old = external::store(
            &app,
            1,
            0,
            "Fixture",
            "1",
            "",
            "modrinth:fixture:1",
            &d,
            b"PK\x03\x04old",
        )
        .await
        .unwrap();
        let new = external::store(
            &app,
            1,
            0,
            "Fixture",
            "2",
            "",
            "modrinth:fixture:2",
            &d,
            b"PK\x03\x04new",
        )
        .await
        .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO mod_scans SELECT ?1,sha256,'queued','{}',0 FROM mods WHERE id=?1",
                [&new],
            )
            .unwrap();
        let read = |v: Value| {
            v["games"]
                .as_array()
                .unwrap()
                .iter()
                .find(|g| g["name"] == "Minecraft")
                .unwrap()["mods"][0]["version"]
                .clone()
        };
        assert_eq!(
            read(
                value(
                    call(
                        app.clone(),
                        "GET",
                        "/api/v1/catalog",
                        Value::Null,
                        Some(&token)
                    )
                    .await
                )
                .await
            ),
            "1"
        );
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE mod_scans SET status='complete',report='{\"findings\":[]}' WHERE mod_id=?1",
                [&new],
            )
            .unwrap();
        assert_eq!(
            read(
                value(
                    call(
                        app.clone(),
                        "GET",
                        "/api/v1/catalog",
                        Value::Null,
                        Some(&token)
                    )
                    .await
                )
                .await
            ),
            "2"
        );
        assert_eq!(
            call(
                app.clone(),
                "GET",
                &format!("/api/v1/mods/{old}"),
                Value::Null,
                Some(&token)
            )
            .await
            .status(),
            StatusCode::OK
        );
    }
    #[test]
    fn workshop_recommendations_are_removed() {
        assert!(recommendations().is_empty());
    }
    #[tokio::test]
    async fn catalog_and_legacy_file_aliases_require_auth_and_preserve_pins() {
        let (dir, app) = fixture();
        let token = account(&app, "catalog-user", true);
        let user = app
            .auth(&HeaderMap::from_iter([(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            )]))
            .unwrap()
            .0;
        let bytes = b"PK\x03\x04fixture";
        external::store(&app,user,1686940,"Example","1.0","","fixture",&json!({"game":"Bopl Battle","folder":"bopl-battle","catalog_file":"bopl-battle/Mods/old.zip"}),bytes).await.unwrap();
        external::store(
            &app,
            user,
            1557740,
            "BepInExPack_ROUNDS",
            "5.4.1901",
            "",
            "loader-fixture",
            &json!({"provider":"thunderstore","game":"ROUNDS"}),
            bytes,
        )
        .await
        .unwrap();
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_reviews SET approved=1", [])
            .unwrap();
        assert_eq!(
            call(app.clone(), "GET", "/api/v1/catalog", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let catalog = value(
            call(
                app.clone(),
                "GET",
                "/api/v1/catalog",
                Value::Null,
                Some(&token),
            )
            .await,
        )
        .await;
        assert_eq!(
            catalog["games"]
                .as_array()
                .unwrap()
                .iter()
                .find(|game| game["app_id"] == 1686940)
                .unwrap()["mods"][0]["sha256"],
            hex::encode(Sha256::digest(bytes))
        );
        let games = catalog["games"].as_array().unwrap();
        assert!(
            games
                .iter()
                .any(|g| g["app_id"] == 1557740 && g["mods"].as_array().unwrap().is_empty())
        );
        assert!(
            games
                .iter()
                .any(|g| g["app_id"] == 550 && g["framework"] == "source-vpk")
        );
        let response = call(
            app.clone(),
            "GET",
            "/api/v1/catalog/file?path=bopl-battle%2FMods%2Fold.zip",
            Value::Null,
            Some(&token),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap()
                .as_ref(),
            bytes
        );
        std::fs::write(dir.path().join("icon.png"), b"image fixture").unwrap();
        std::fs::write(dir.path().join("assets-import.json"),json!([{"local_file":"icon.png","alias":"bopl-battle/icon.png","game_id":1686940,"kind":"icon","sha256":hex::encode(Sha256::digest(b"image fixture"))}]).to_string()).unwrap();
        migrate(&app, &dir.path().join("assets-import.json"))
            .await
            .unwrap();
        let member = account(&app, "artwork-member", false);
        let artwork_path = "/api/v1/catalog/file?path=rounds%2Ficon.png";
        app.db.lock().unwrap().execute("INSERT INTO game_assets SELECT id || '-rounds', 'rounds/icon.png',sha256,size,1557740,kind FROM game_assets WHERE alias='bopl-battle/icon.png'", []).unwrap();
        // Give the second alias its own encrypted blob, matching production imports.
        let id: String = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT id FROM game_assets WHERE alias='rounds/icon.png'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let file = tokio::fs::File::create(app.files.join(format!("{id}.zip")))
            .await
            .unwrap();
        let mut writer = crypto::Writer::new(file, &app.upload_key, id)
            .await
            .unwrap();
        writer.write(b"image fixture").await.unwrap();
        writer.finish().await.unwrap();
        assert_eq!(
            call(app.clone(), "GET", artwork_path, Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        for path in [
            artwork_path,
            "/api/v1/catalog/file?path=bopl-battle%2Ficon.png",
        ] {
            let response = call(app.clone(), "GET", path, Value::Null, Some(&member)).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["x-canna-sha256"],
                hex::encode(Sha256::digest(b"image fixture"))
            );
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .as_ref(),
                b"image fixture"
            );
        }
        app.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE game_assets SET kind='framework' WHERE alias='rounds/icon.png'",
                [],
            )
            .unwrap();
        assert_eq!(
            call(app.clone(), "GET", artwork_path, Value::Null, Some(&member))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        audit(&app).await.unwrap();
        assert_eq!(
            call(
                app,
                "GET",
                "/api/v1/catalog/file?path=../secret",
                Value::Null,
                Some(&token)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
}
