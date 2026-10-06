use super::*;
#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}
pub fn recommendations() -> Vec<Value> {
    serde_json::from_str(include_str!("../web/source-recommendations.json")).expect("Curated Source catalog")
}
pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS game_assets(id TEXT PRIMARY KEY,alias TEXT UNIQUE NOT NULL,sha256 TEXT NOT NULL,size INTEGER NOT NULL,game_id INTEGER NOT NULL,kind TEXT NOT NULL);")
}
pub async fn list(State(app): State<Shared>, headers: HeaderMap) -> ApiResult<axum::Json<Value>> {
    app.auth(&headers)?;
    let db = app.db.lock().unwrap();
    let mut stmt =
        db.prepare("SELECT id,app_id,name,version,description,sha256 FROM mods WHERE NOT EXISTS(SELECT 1 FROM mod_reviews WHERE mod_id=mods.id AND approved=0) ORDER BY name")?;
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
        games.insert(id, json!({"app_id":id,"name":name,"folder":folder,"framework":framework,"icon":if id==1686940 || id==1557740 {"icon.png"} else {""},"description":if framework=="source-vpk" {"VPK addon packs; modded launches use -insecure practice mode"} else {"Unity modpacks with BepInEx"},"mods":[],"mod_folder_status":"Server library ready"}));
    }
    for (id, appid, name, version, description, hash) in rows {
        if security::approved(&db, &id).is_err() {
            continue;
        }
        let appid = if appid == 0 { u32::MAX } else { appid };
        let d = external::details(&db, &id)?;
        // Loader distributions are installed through Framework, never as plugin DLLs.
        if d["provider"] == "thunderstore" && name.starts_with("BepInExPack") { continue; }
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
        if let Some(game)=games.get_mut(&(item["app_id"].as_u64().unwrap() as u32)) {
            game["mods"].as_array_mut().unwrap().push(json!({"enabled":false,"provenance":item["details"],"content_type":"mod","name":item["name"],"version":item["version"],"description":item["description"],"file":format!("Mods/{}.zip",item["id"].as_str().unwrap()),"sha256":"","dependencies":item["details"]["dependencies"]}));
        }
    }
    Ok(axum::Json(
        json!({"games":games.into_values().collect::<Vec<_>>()}),
    ))
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
    let (id, size, hash) = {
        let db = app.db.lock().unwrap();
        let asset: Option<(String, i64, String)> = db
            .query_row(
                "SELECT id,size,sha256 FROM game_assets WHERE alias=?1",
                [&q.path],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some(row) = asset {
            row
        } else {
            let filename = q
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .trim_end_matches(".zip");
            db.query_row("SELECT m.id,m.size,m.sha256 FROM mods m LEFT JOIN mod_details d ON d.mod_id=m.id WHERE m.id=?1 OR json_extract(d.data,'$.catalog_file')=?2",params![filename,q.path],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(ApiError(StatusCode::NOT_FOUND,"Catalog file unavailable"))?
        }
    };
    security::approved(&app.db.lock().unwrap(), &id)?;
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
            "SELECT id,sha256,size FROM mods UNION ALL SELECT id,sha256,size FROM game_assets",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    for (id, expected, size) in &rows {
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
    println!("Verified {count} encrypted mods and {assets} encrypted game assets");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    #[test]
    fn workshop_recommendations_preserve_credit_and_are_not_hosted_downloads() {
        let entries=recommendations();
        assert_eq!(entries.len(),6);
        for item in &entries {
            assert_eq!(item["app_id"],550);
            assert_eq!(item["details"]["external_only"],true);
            assert!(item["details"]["author_links"][0]["url"].as_str().unwrap().starts_with("https://steamcommunity.com/"));
            assert!(!item["details"]["icon_data"].as_str().unwrap().is_empty());
            assert!(!item["description"].as_str().unwrap().is_empty());
        }
        let bots=entries.iter().find(|m|m["name"]=="Left 4 Bots 2").unwrap();
        assert_eq!(bots["details"]["dependencies"],json!(["Left 4 Lib","NavFixes"]));
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
        external::store(&app,user,1557740,"BepInExPack_ROUNDS","5.4.1901","","loader-fixture",&json!({"provider":"thunderstore","game":"ROUNDS"}),bytes).await.unwrap();
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
            catalog["games"].as_array().unwrap().iter().find(|game| game["app_id"]==1686940).unwrap()["mods"][0]["sha256"],
            hex::encode(Sha256::digest(bytes))
        );
        let games=catalog["games"].as_array().unwrap();
        assert!(games.iter().any(|g|g["app_id"]==1557740 && g["mods"].as_array().unwrap().is_empty()));
        assert!(games.iter().any(|g|g["app_id"]==550 && g["framework"]=="source-vpk"));
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
