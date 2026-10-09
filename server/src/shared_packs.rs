use super::*;

const REBOUND_PROFILE: &str = "rounds-public-1.1.2";

// Exact reviewed provider releases supplied by the current Bliss payload.
// Names alone never qualify: a different author or version must be installed.
fn rebound_dependency(
    details: &Value,
    parent: &Value,
    parent_version: &str,
    parent_hash: &str,
) -> Option<String> {
    if details["provider"] != "thunderstore"
        || !details["source_url"]
            .as_str()?
            .starts_with("https://thunderstore.io/c/rounds/p/")
    {
        return None;
    }
    let alias = format!(
        "{}-{}",
        details["id"].as_str()?,
        details["release_id"].as_str()?
    );
    let (author, project) = details["id"].as_str()?.split_once('-')?;
    if details["source_url"].as_str()?
        != format!("https://thunderstore.io/c/rounds/p/{author}/{project}/")
    {
        return None;
    }
    let supported: Value =
        serde_json::from_str(include_str!("fixtures/rebound-dependencies.json")).ok()?;
    let supplied = supported
        .as_array()?
        .iter()
        .any(|entry| entry.as_str() == Some(&alias));
    // The exact archived CR release is replaced by the reviewed curated adapter.
    // These four metadata-only legacy patches are absent from its validated hard
    // dependency closure. The exception cannot apply to another CR archive.
    let retired = parent["provider"] == "thunderstore"
        && parent["id"] == "XAngelMoonX-CR"
        && parent["source_url"] == "https://thunderstore.io/c/rounds/p/XAngelMoonX/CR/"
        && parent_version == "2.7.0"
        && parent_hash == "db059e5c38fb365cba8f019320f40e0d9510938fa2983442e82a58b9a8ee5ea7"
        && matches!(
            alias.as_str(),
            "Root-CardThemeLib-1.1.7"
                | "Root-GravityPatch-0.0.0"
                | "TeamDK-ZeroGBulletPatch-1.1.0"
                | "willuwontu-StopShootingYoureDead-0.0.0"
        );
    (supplied || retired).then_some(alias)
}

fn requires_rebound(manifest: &Value) -> bool {
    manifest["mods"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|item| {
            item["enabled"] == true
                && item["provenance"]["compatibility_profile"] == REBOUND_PROFILE
        })
}

fn authorize_publish(db: &Connection, manifest: &Value, user: i64) -> ApiResult<()> {
    if requires_rebound(manifest) && !admin_settings::has_rebound(db, user)? {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Sharing this pack requires Canna Bliss Beta access",
        ));
    }
    Ok(())
}

pub fn initialize(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS pack_meta(id TEXT PRIMARY KEY REFERENCES packs(id) ON DELETE CASCADE,revision INTEGER NOT NULL,updated INTEGER NOT NULL);
      INSERT OR IGNORE INTO pack_meta SELECT id,1,0 FROM packs;
      CREATE TABLE IF NOT EXISTS pack_transfers(receipt TEXT PRIMARY KEY,manifest TEXT NOT NULL,expires INTEGER NOT NULL);")
}

fn canonical(db: &Connection, input: &Value) -> ApiResult<Value> {
    let name = check_pack(input)?;
    let game = input["game"]["app_id"]
        .as_u64()
        .filter(|id| *id <= u32::MAX as u64)
        .ok_or_else(|| bad("Invalid game"))? as u32;
    if !game_profiles::supports_game(game) {
        return Err(bad("This game has no Canna installation profile"));
    }
    let description = input["description"].as_str().unwrap_or("");
    if description.len() > 2000 {
        return Err(bad("Pack description is too long"));
    }
    let (game_name, folder, framework) = match game {
        u32::MAX => ("Minecraft", "minecraft", "minecraft"),
        1686940 => ("Bopl Battle", "bopl-battle", "bepinex"),
        1557740 => ("ROUNDS", "rounds", "bepinex"),
        550 => ("Left 4 Dead 2", "left-4-dead-2", "source-vpk"),
        500 => ("Left 4 Dead", "left-4-dead", "source-vpk"),
        _ => {
            let p = game_profiles::by_id(game).ok_or_else(|| bad("Unsupported game"))?;
            (p.name.as_str(), p.folder.as_str(), "bepinex")
        }
    };
    let mut mods = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let enabled_files: std::collections::HashSet<&str> = input["mods"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["enabled"].as_bool().unwrap_or(true))
        .filter_map(|item| item["file"].as_str())
        .collect();
    for item in input["mods"].as_array().unwrap() {
        let id = item["file"]
            .as_str()
            .and_then(|s| s.strip_prefix("Mods/"))
            .and_then(|s| s.strip_suffix(".zip"))
            .ok_or_else(|| bad("Use server library mods in shared packs"))?;
        Uuid::parse_str(id).map_err(|_| bad("Invalid server mod reference"))?;
        if !seen.insert(id) {
            return Err(bad("Duplicate mod in pack"));
        }
        let row: Option<(u32, String, String, String, String)> = db
            .query_row(
                "SELECT app_id,name,version,description,sha256 FROM mods WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (appid, title, version, desc, hash) =
            row.ok_or_else(|| bad("A referenced mod was removed from the server"))?;
        if appid != game && !(game == u32::MAX && appid == 0) {
            return Err(bad("Mod belongs to a different game"));
        }
        if item["sha256"].as_str() != Some(&hash) {
            return Err(bad("Mod archive pin does not match the server"));
        }
        let d = external::details(db, id)?;
        let mut provenance = json!({});
        for key in [
            "provider",
            "source_url",
            "authors",
            "author_links",
            "icon_url",
            "game_versions",
            "loaders",
            "install_notes",
        ] {
            if !d[key].is_null() {
                provenance[key] = d[key].clone();
            }
        }
        let deps = if let Some(ids) = d["dependency_ids"].as_array() {
            let mut names = Vec::new();
            let mut supplied = Vec::new();
            for dep in ids.iter().filter_map(Value::as_str) {
                let dependency = external::details(db, dep)?;
                if dependency["framework_root"].is_string() {
                    continue;
                }
                let (dep_game, dep_name, dep_version): (u32, String, String) = db
                    .query_row(
                        "SELECT app_id,name,version FROM mods WHERE id=?1",
                        [dep],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .map_err(|_| bad("Required dependency was removed"))?;
                if dep_game != game && !(game == u32::MAX && dep_game == 0) {
                    return Err(bad("Required dependency belongs to a different game"));
                }
                if item["enabled"].as_bool().unwrap_or(true)
                    && !enabled_files.contains(format!("Mods/{dep}.zip").as_str())
                {
                    if game == 1557740
                        && dependency["release_id"] == dep_version
                        && let Some(alias) = rebound_dependency(&dependency, &d, &version, &hash)
                    {
                        supplied.push(alias);
                        continue;
                    }
                    return Err(bad(
                        "Add and enable the exact required dependency versions before sharing",
                    ));
                }
                names.push(dep_name);
            }
            if !supplied.is_empty() {
                provenance["compatibility_profile"] = json!(REBOUND_PROFILE);
                provenance["required_game_branch"] = json!("public");
                provenance["rebound_supplied_dependencies"] = json!(supplied);
                // Keep exact provider aliases for the translator's dependency closure.
                provenance["dependencies"] = d["dependencies"].clone();
            }
            names
        } else {
            item["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        if deps.len() > 32 || deps.iter().any(|s| s.len() > 200 || s.is_empty()) {
            return Err(bad("Invalid dependencies"));
        }
        mods.push(json!({"name":title,"version":version,"description":desc,"file":format!("Mods/{id}.zip"),"sha256":hash,"enabled":item["enabled"].as_bool().unwrap_or(true),"local_file":"","dependencies":deps,"content_type":d["content_type"].as_str().or(d["project_type"].as_str()).unwrap_or("mod"),"provenance":provenance}));
    }
    for item in mods.iter().filter(|m| m["enabled"] == true) {
        for dep in item["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
        {
            if !mods
                .iter()
                .any(|m| m["name"] == dep && m["enabled"] == true)
            {
                return Err(bad(
                    "Add and enable all required dependencies before sharing",
                ));
            }
        }
    }
    let manifest = json!({"format":"canna_modpack","schema_version":1,"id":"shared-pack","name":name,"description":description,"group":"","theme":input["theme"].as_u64().unwrap_or(0).min(3),"game":{"app_id":game,"name":game_name,"folder":folder,"framework":framework},"repository":{"owner":"canna","repository":"server","branch":"main","catalog_folder":""},"mods":mods});
    if manifest.to_string().len() > 1536 * 1024 {
        return Err(bad(
            "Shared pack metadata exceeds 1.5 MiB. Split it into smaller packs",
        ));
    }
    Ok(manifest)
}
fn raw(db: &Connection, id: &str) -> ApiResult<(i64, String, i64, i64, Value)> {
    Uuid::parse_str(id).map_err(|_| bad("Invalid pack ID"))?;
    let row:Option<(i64,String,i64,i64,String)>=db.query_row("SELECT p.user_id,u.username,m.revision,m.updated,p.manifest FROM packs p JOIN users u ON u.id=p.user_id JOIN pack_meta m ON m.id=p.id WHERE p.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let (owner, author, revision, updated, data) =
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "Shared pack was removed"))?;
    let mut manifest = canonical(
        db,
        &serde_json::from_str::<Value>(&data).map_err(|_| bad("Invalid stored pack"))?,
    )?;
    manifest["id"] = json!(id);
    manifest["shared"] = json!({"id":id,"revision":revision});
    Ok((owner, author, revision, updated, manifest))
}
fn available(db: &Connection, manifest: &Value) -> bool {
    manifest["mods"].as_array().unwrap().iter().all(|m| {
        security::approved(
            db,
            m["file"]
                .as_str()
                .unwrap()
                .trim_start_matches("Mods/")
                .trim_end_matches(".zip"),
        )
        .is_ok()
    })
}
fn information(db: &Connection, id: &str, user: i64) -> ApiResult<Value> {
    let (owner, author, revision, updated, manifest) = raw(db, id)?;
    Ok(
        json!({"id":id,"url":format!("https://cannamods.vip/packs/{id}"),"author":author,"revision":revision,"updated":updated,"can_update":owner==user,"ready":available(db,&manifest),"requires_rebound":requires_rebound(&manifest),"manifest":manifest}),
    )
}
pub fn download_manifest(db: &Connection, id: &str) -> ApiResult<String> {
    let (_, _, _, _, manifest) = raw(db, id)?;
    if !available(db, &manifest) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "One or more pack mods need approval or are unavailable",
        ));
    }
    Ok(manifest.to_string())
}
pub async fn publish(
    State(app): State<Shared>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Value>,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    app.limits.check(format!("pack-publish:{user}"), 20)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let mut manifest = canonical(&tx, &input)?;
    authorize_publish(&tx, &manifest, user)?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM packs WHERE user_id=?1", [user], |r| {
        r.get(0)
    })?;
    if count >= 200 {
        return Err(bad("You can share up to 200 packs"));
    }
    let id = Uuid::new_v4().to_string();
    manifest["id"] = json!(id);
    tx.execute(
        "INSERT INTO packs VALUES(?1,?2,?3,?4)",
        params![id, user, manifest["name"].as_str(), manifest.to_string()],
    )?;
    tx.execute("INSERT INTO pack_meta VALUES(?1,1,?2)", params![id, now()])?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'pack-published',?2,?3)",
        params![user, id, now()],
    )?;
    let result = information(&tx, &id, user)?;
    tx.commit()?;
    Ok(axum::Json(result))
}
pub async fn info(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    Ok(axum::Json(information(&app.db.lock().unwrap(), &id, user)?))
}
pub async fn local_mod(
    State(app): State<Shared>,
    Path(hash): Path<String>,
    headers: HeaderMap,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad("Invalid archive hash"));
    }
    let id: Option<String> = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT id FROM mods WHERE user_id=?1 AND sha256=?2 ORDER BY rowid DESC LIMIT 1",
            params![user, hash.to_lowercase()],
            |r| r.get(0),
        )
        .optional()?;
    Ok(axum::Json(
        json!({"id":id.ok_or(ApiError(StatusCode::NOT_FOUND,"No matching upload"))?,"sha256":hash.to_lowercase()}),
    ))
}
pub async fn update(
    State(app): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<Value>,
) -> ApiResult<axum::Json<Value>> {
    let (user, _) = app.auth(&headers)?;
    app.limits.check(format!("pack-publish:{user}"), 20)?;
    let mut db = app.db.lock().unwrap();
    let tx = db.transaction()?;
    let (owner, _, revision, _, old) = raw(&tx, &id)?;
    if owner != user {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "Only the pack creator can publish updates",
        ));
    }
    if input["expected_revision"].as_i64() != Some(revision) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "Pack changed on another device. Check updates before publishing",
        ));
    }
    let mut manifest = canonical(&tx, &input["manifest"])?;
    authorize_publish(&tx, &manifest, user)?;
    if manifest["game"]["app_id"] != old["game"]["app_id"] {
        return Err(bad("Create a new shared pack to change games"));
    }
    manifest["id"] = json!(id);
    tx.execute(
        "UPDATE packs SET name=?1,manifest=?2 WHERE id=?3",
        params![manifest["name"].as_str(), manifest.to_string(), id],
    )?;
    tx.execute(
        "UPDATE pack_meta SET revision=revision+1,updated=?1 WHERE id=?2",
        params![now(), id],
    )?;
    tx.execute(
        "INSERT INTO audit(actor,action,target,created) VALUES(?1,'pack-updated',?2,?3)",
        params![user, id, now()],
    )?;
    let result = information(&tx, &id, user)?;
    tx.commit()?;
    Ok(axum::Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{account, call, fixture, value};
    fn manifest() -> Value {
        json!({"format":"canna_modpack","schema_version":1,"name":"Family pack","description":"Game night","game":{"app_id":1686940},"mods":[],"token":"must-not-share","repository":{"token":"must-not-share"},"device_name":"must-not-share"})
    }
    #[test]
    fn exact_user_cr_archive_can_share_its_complete_provider_dependency_list() {
        let (_dir, app) = fixture();
        let _owner = account(&app, "cr-pack-fixture", false);
        let db = app.db.lock().unwrap();
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/rounds-dependencies.json")).unwrap();
        let id = Uuid::new_v4().to_string();
        let hash = "db059e5c38fb365cba8f019320f40e0d9510938fa2983442e82a58b9a8ee5ea7";
        db.execute(
            "INSERT INTO mods VALUES(?1,1,1557740,'CR','2.7.0','Original',?2,1)",
            params![id, hash],
        )
        .unwrap();
        let mut ids = Vec::new();
        for alias in fixture["packages"]["XAngelMoonX-CR"]["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
        {
            let (project, _) = alias.rsplit_once('-').unwrap();
            if project.starts_with("BepInEx-") {
                continue;
            }
            let version = fixture["packages"][project]["version"].as_str().unwrap();
            let dep = Uuid::new_v4().to_string();
            let (author, name) = project.split_once('-').unwrap();
            db.execute(
                "INSERT INTO mods VALUES(?1,1,1557740,?2,?3,'Original',?4,1)",
                params![dep, name, version, "a".repeat(64)],
            )
            .unwrap();
            db.execute("INSERT INTO mod_details VALUES(?1,?2,?3)",params![dep,format!("fixture:{project}"),json!({"provider":"thunderstore","id":project,"release_id":version,"source_url":format!("https://thunderstore.io/c/rounds/p/{author}/{name}/")}).to_string()]).unwrap();
            ids.push(dep);
        }
        assert_eq!(ids.len(), 13);
        db.execute("INSERT INTO mod_details VALUES(?1,'fixture:actual-cr',?2)",params![id,json!({"provider":"thunderstore","id":"XAngelMoonX-CR","source_url":"https://thunderstore.io/c/rounds/p/XAngelMoonX/CR/","dependency_ids":ids,"dependencies":fixture["packages"]["XAngelMoonX-CR"]["dependencies"]}).to_string()]).unwrap();
        let mut pack = manifest();
        pack["game"]["app_id"] = json!(1557740);
        pack["mods"] = json!([{"file":format!("Mods/{id}.zip"),"sha256":hash,"enabled":true}]);
        let result = canonical(&db, &pack).unwrap();
        assert_eq!(
            result["mods"][0]["provenance"]["rebound_supplied_dependencies"]
                .as_array()
                .unwrap()
                .len(),
            13
        );
        assert!(requires_rebound(&result));
        // An altered archive with identical branding cannot use retired-patch exceptions.
        let unknown_hash = "b".repeat(64);
        db.execute(
            "UPDATE mods SET sha256=?1 WHERE id=?2",
            params![unknown_hash, id],
        )
        .unwrap();
        pack["mods"][0]["sha256"] = json!(unknown_hash);
        assert!(canonical(&db, &pack).is_err());
    }
    #[tokio::test]
    async fn beta_can_share_exact_rebound_dependencies_without_shipping_support() {
        let (_dir, app) = fixture();
        let owner = account(&app, "rebound-pack-owner", false);
        let id = Uuid::new_v4().to_string();
        let dep = Uuid::new_v4().to_string();
        let hash = "a".repeat(64);
        let data = json!({"provider":"thunderstore","id":"willis81808-UnboundLib","release_id":"3.2.14","source_url":"https://thunderstore.io/c/rounds/p/willis81808/UnboundLib/"});
        {
            let db = app.db.lock().unwrap();
            for (mod_id, name) in [(&id, "CR"), (&dep, "UnboundLib")] {
                db.execute(
                    "INSERT INTO mods VALUES(?1,1,1557740,?2,?3,'Original',?4,1)",
                    params![
                        mod_id,
                        name,
                        if name == "UnboundLib" {
                            "3.2.14"
                        } else {
                            "2.7.0"
                        },
                        hash
                    ],
                )
                .unwrap();
                db.execute(
                    "INSERT INTO mod_reviews(mod_id,approved) VALUES(?1,1)",
                    [mod_id],
                )
                .unwrap();
            }
            db.execute("INSERT INTO mod_details VALUES(?1,'test:cr',?2)",params![id,json!({"dependency_ids":[dep],"dependencies":["willis81808-UnboundLib-3.2.14"]}).to_string()]).unwrap();
            db.execute(
                "INSERT INTO mod_details VALUES(?1,'test:unbound',?2)",
                params![dep, data.to_string()],
            )
            .unwrap();
        }
        let mut pack = manifest();
        pack["game"]["app_id"] = json!(1557740);
        pack["mods"] = json!([{"file":format!("Mods/{id}.zip"),"sha256":hash,"enabled":true}]);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/packs",
                pack.clone(),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        app.db
            .lock()
            .unwrap()
            .execute("INSERT INTO user_roles VALUES(1,'beta')", [])
            .unwrap();
        let info = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/packs",
                pack.clone(),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(info["ready"], true);
        assert_eq!(info["requires_rebound"], true);
        let m = &info["manifest"];
        assert_eq!(m["mods"].as_array().unwrap().len(), 1);
        assert_eq!(
            m["mods"][0]["provenance"]["compatibility_profile"],
            REBOUND_PROFILE
        );
        assert_eq!(
            m["mods"][0]["provenance"]["dependencies"],
            json!(["willis81808-UnboundLib-3.2.14"])
        );
        assert_eq!(m["mods"][0]["dependencies"], json!([]));
        let reread = value(
            call(
                app.clone(),
                "GET",
                &format!("/api/v1/packs/{}/info", info["id"].as_str().unwrap()),
                Value::Null,
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(reread["manifest"], *m);
        for (key, bad_value) in [
            ("release_id", "9.9.9"),
            ("provider", "local"),
            ("id", "Other-UnboundLib"),
            (
                "source_url",
                "https://thunderstore.io/c/bopl-battle/p/willis81808/UnboundLib/",
            ),
        ] {
            let mut forged = data.clone();
            forged[key] = json!(bad_value);
            app.db
                .lock()
                .unwrap()
                .execute(
                    "UPDATE mod_details SET data=?1 WHERE mod_id=?2",
                    params![forged.to_string(), dep],
                )
                .unwrap();
            assert_eq!(
                call(
                    app.clone(),
                    "POST",
                    "/api/v1/packs",
                    pack.clone(),
                    Some(&owner)
                )
                .await
                .status(),
                StatusCode::BAD_REQUEST,
                "{key}"
            );
        }
        assert!(
            app.db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM packs", [], |r| r.get::<_, i64>(0))
                .unwrap()
                == 1
        );
    }
    #[tokio::test]
    async fn links_have_private_metadata_owner_revisions_and_atomic_transfers() {
        let (_dir, app) = fixture();
        let owner = account(&app, "pack-creator", false);
        let other = account(&app, "pack-friend", true);
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/packs",
                manifest(),
                Some(&owner),
            )
            .await,
        )
        .await;
        let id = first["id"].as_str().unwrap();
        let path = format!("/api/v1/packs/{id}");
        assert_eq!(first["revision"], 1);
        assert_eq!(
            call(app.clone(), "GET", "/shared-packs.js", Value::Null, None)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(first["ready"], true);
        assert!(!first.to_string().contains("must-not-share"));
        assert_eq!(
            call(
                app.clone(),
                "GET",
                &format!("{path}/info"),
                Value::Null,
                None
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let info = value(
            call(
                app.clone(),
                "GET",
                &format!("{path}/info"),
                Value::Null,
                Some(&other),
            )
            .await,
        )
        .await;
        assert_eq!(info["author"], "pack-creator");
        assert_eq!(info["can_update"], false);
        let ticket = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets",
                json!({"kind":"packs","id":id}),
                Some(&other),
            )
            .await,
        )
        .await;
        let claim = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets/claim",
                json!({"ticket":ticket["ticket"]}),
                None,
            )
            .await,
        )
        .await;
        let mut second = manifest();
        second["name"] = json!("Updated family pack");
        let body = json!({"expected_revision":1,"manifest":second});
        assert_eq!(
            call(app.clone(), "POST", &path, body.clone(), Some(&other))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        let updated =
            value(call(app.clone(), "POST", &path, body.clone(), Some(&owner)).await).await;
        assert_eq!(updated["revision"], 2);
        assert_eq!(updated["id"], id);
        assert_eq!(
            call(app.clone(), "POST", &path, body, Some(&owner))
                .await
                .status(),
            StatusCode::CONFLICT
        );
        let transfer = call(
            app.clone(),
            "POST",
            "/api/v1/download-tickets/transfer",
            json!({"receipt":claim["receipt"]}),
            None,
        )
        .await;
        assert_eq!(transfer.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(transfer.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            claim["sha256"].as_str().unwrap()
        );
        let downloaded: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(downloaded["shared"]["revision"], 1);
        assert_eq!(downloaded["name"], "Family pack");
        let latest = value(call(app.clone(), "GET", &path, Value::Null, Some(&other)).await).await;
        assert_eq!(latest["shared"]["revision"], 2);
        let mut wrong = manifest();
        wrong["game"]["app_id"] = json!(550);
        assert_eq!(
            call(
                app.clone(),
                "POST",
                &path,
                json!({"expected_revision":2,"manifest":wrong}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(app.clone(), "DELETE", &path, Value::Null, Some(&owner))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            call(
                app,
                "GET",
                &format!("{path}/info"),
                Value::Null,
                Some(&other)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn referenced_mods_must_match_game_hash_dependencies_and_current_approval() {
        let (_dir, app) = fixture();
        let owner = account(&app, "pack-owner", false);
        let id = Uuid::new_v4().to_string();
        let hash = "a".repeat(64);
        let other = account(&app, "different-uploader", false);
        {
            let db = app.db.lock().unwrap();
            db.execute("INSERT INTO mods VALUES(?1,1,1686940,'Reviewed mod','1','Original description',?2,1)",params![id,hash]).unwrap();
            db.execute(
                "INSERT INTO mod_reviews(mod_id,approved) VALUES(?1,0)",
                [&id],
            )
            .unwrap();
        }
        assert_eq!(
            call(
                app.clone(),
                "GET",
                &format!("/api/v1/packs/local-mod/{hash}"),
                Value::Null,
                Some(&other)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            value(
                call(
                    app.clone(),
                    "GET",
                    &format!("/api/v1/packs/local-mod/{hash}"),
                    Value::Null,
                    Some(&owner)
                )
                .await
            )
            .await["id"],
            id
        );
        let mut pack = manifest();
        pack["mods"] = json!([{"file":format!("Mods/{id}.zip"),"sha256":hash,"name":"stolen credit","enabled":true,"dependencies":[]}]);
        let first = value(
            call(
                app.clone(),
                "POST",
                "/api/v1/packs",
                pack.clone(),
                Some(&owner),
            )
            .await,
        )
        .await;
        assert_eq!(first["ready"], false);
        assert_eq!(first["manifest"]["mods"][0]["name"], "Reviewed mod");
        let pid = first["id"].as_str().unwrap();
        let path = format!("/api/v1/packs/{pid}");
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, Some(&owner))
                .await
                .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(
                app.clone(),
                "POST",
                "/api/v1/download-tickets",
                json!({"kind":"packs","id":pid}),
                Some(&owner)
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let mut bad_hash = pack.clone();
        bad_hash["mods"][0]["sha256"] = json!("b".repeat(64));
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/packs", bad_hash, Some(&owner))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        let mut wrong = pack.clone();
        wrong["game"]["app_id"] = json!(550);
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/packs", wrong, Some(&owner))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        pack["mods"][0]["dependencies"] = json!(["Missing dependency"]);
        assert_eq!(
            call(app.clone(), "POST", "/api/v1/packs", pack, Some(&owner))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_reviews SET approved=1 WHERE mod_id=?1", [&id])
            .unwrap();
        assert_eq!(
            call(app.clone(), "GET", &path, Value::Null, Some(&owner))
                .await
                .status(),
            StatusCode::OK
        );
        app.db
            .lock()
            .unwrap()
            .execute("UPDATE mod_reviews SET approved=0 WHERE mod_id=?1", [&id])
            .unwrap();
        assert_eq!(
            call(app, "GET", &path, Value::Null, Some(&owner))
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
}
