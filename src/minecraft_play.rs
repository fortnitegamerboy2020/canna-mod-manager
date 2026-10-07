//! Minecraft instance adapters; invoked only after an explicit local Play Lab action.
use super::*;
use sha2::Sha256;
fn read_instance(path: &Path) -> Result<Instance> {
    crate::runtime::no_links(path)?;
    anyhow::ensure!(
        path.canonicalize()?
            .starts_with(root().join("instances").canonicalize()?),
        "Choose a Canna-owned Minecraft instance"
    );
    let bytes = std::fs::read(path.join("instance.json"))?;
    anyhow::ensure!(bytes.len() <= 8192, "Instance metadata exceeds limits");
    let value: Instance = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        safe_component(&value.id) && dir(&value).canonicalize()? == path.canonicalize()?,
        "Instance identity mismatch"
    );
    Ok(value)
}
pub fn play_setup(
    instance: &Instance,
) -> Result<(crate::modpacks::Modpack, crate::model::InstalledGame)> {
    let folder = dir(instance);
    read_instance(&folder)?;
    let mut mods = vec![];
    for (category, subdir) in [
        ("mod", "mods"),
        ("shader", "shaderpacks"),
        ("resourcepack", "resourcepacks"),
    ] {
        let path = folder.join(subdir);
        crate::runtime::no_links(&path)?;
        if !path.exists() {
            continue;
        }
        for entry in std::fs::read_dir(path)? {
            let path = entry?.path();
            crate::runtime::no_links(&path)?;
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !["jar", "zip"].contains(&ext.as_str()) {
                continue;
            }
            anyhow::ensure!(
                mods.len() < 1000 && std::fs::metadata(&path)?.len() <= 128 * 1024 * 1024,
                "Instance content exceeds limits"
            );
            let data = std::fs::read(&path)?;
            let hash = format!("{:x}", Sha256::digest(&data));
            let local = format!("{hash}.{ext}");
            std::fs::create_dir_all(crate::modpacks::local_directory())?;
            let target = crate::modpacks::local_directory().join(&local);
            crate::runtime::no_links(&target)?;
            if !target.exists() {
                std::fs::write(&target, &data)?;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            mods.push(crate::model::ModInfo{provenance:json!({"game_version":instance.version,"loader":instance.loader,"target_file":name}),enabled:true,name:name.clone(),version:instance.version.clone(),content_type:category.into(),description:"Observed local file; original author metadata is not inferred".into(),file:format!("Mods/{local}"),sha256:hash,local_file:local,dependencies:vec![]});
        }
    }
    let game_info = crate::model::GameInfo {
        app_id: u32::MAX,
        name: "Minecraft".into(),
        folder: "minecraft".into(),
        ..crate::model::bopl()
    };
    let mut pack = crate::modpacks::Modpack::create(
        instance.name.clone(),
        "Local Minecraft instance observation".into(),
        &game_info,
        crate::cache::Source {
            owner: "canna".into(),
            repository: "server".into(),
            branch: "main".into(),
            catalog_folder: String::new(),
        },
        mods,
    );
    pack.id = format!("minecraft-{}", instance.id);
    pack.validate()?;
    Ok((
        pack,
        crate::model::InstalledGame {
            app_id: u32::MAX,
            name: instance.name.clone(),
            path: folder,
            loader: format!("{} {}", instance.loader, instance.loader_version),
            plugins: 0,
            icon: None,
        },
    ))
}
pub fn play_version(path: &Path) -> Option<(String, String)> {
    let i = read_instance(path).ok()?;
    Some((i.version, format!("{} {}", i.loader, i.loader_version)))
}
pub fn create_play_candidate(
    pack: &crate::modpacks::Modpack,
    game: &crate::model::InstalledGame,
) -> Result<()> {
    let mut i = read_instance(&game.path)?;
    i.id = pack.id.clone();
    i.name = pack.name.clone();
    anyhow::ensure!(!dir(&i).exists(), "Candidate instance already exists");
    save(&i)?;
    let candidate = crate::model::InstalledGame {
        path: dir(&i),
        ..game.clone()
    };
    restore_play_pack(&candidate, pack)?;
    let configs = game.path.join("config");
    if configs.exists() {
        let mut budget = (0usize, 0u64);
        copy_test_configs(&configs, &candidate.path.join("config"), &mut budget)?;
    }
    Ok(())
}
fn copy_test_configs(source: &Path, target: &Path, budget: &mut (usize, u64)) -> Result<()> {
    crate::runtime::no_links(source)?;
    crate::runtime::no_links(target)?;
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let path = entry?.path();
        crate::runtime::no_links(&path)?;
        let output = target.join(path.file_name().context("Invalid config path")?);
        if path.is_dir() {
            copy_test_configs(&path, &output, budget)?;
        } else if path.extension().is_some_and(|e| {
            e.eq_ignore_ascii_case("json")
                || e.eq_ignore_ascii_case("toml")
                || e.eq_ignore_ascii_case("cfg")
        }) {
            let len = std::fs::metadata(&path)?.len();
            budget.0 += 1;
            budget.1 += len;
            anyhow::ensure!(
                budget.0 <= 1000 && len <= 1024 * 1024 && budget.1 <= 10 * 1024 * 1024,
                "Test config copy exceeds limits"
            );
            std::fs::copy(path, output)?;
        }
    }
    Ok(())
}
pub fn restore_play_pack(
    game: &crate::model::InstalledGame,
    pack: &crate::modpacks::Modpack,
) -> Result<()> {
    crate::runtime::ensure_closed(game)?;
    let i = read_instance(&game.path)?;
    pack.validate()?;
    anyhow::ensure!(
        pack.game.app_id == u32::MAX,
        "This is not a Minecraft setup"
    );
    let routes = ["mods", "shaderpacks", "resourcepacks"];
    let stage = game.path.join(".canna-play-stage");
    crate::runtime::no_links(&stage)?;
    anyhow::ensure!(
        !stage.exists(),
        "A previous Minecraft recovery stage needs inspection"
    );
    std::fs::create_dir(&stage)?;
    let result = (|| -> Result<()> {
        for route in routes {
            let active = game.path.join(route);
            crate::runtime::no_links(&active)?;
            crate::runtime::no_links(&active.with_extension("canna-previous"))?;
            anyhow::ensure!(
                !active.with_extension("canna-previous").exists(),
                "Previous instance recovery needs inspection"
            );
            std::fs::create_dir(stage.join(route))?;
        }
        for item in pack.mods.iter().filter(|m| m.enabled) {
            anyhow::ensure!(
                item.provenance["game_version"] == i.version
                    && item.provenance["loader"] == i.loader,
                "Snapshot's Minecraft version or loader differs; use a separate matching instance"
            );
            let route = match item.content_type.as_str() {
                "shader" => "shaderpacks",
                "resourcepack" => "resourcepacks",
                _ => "mods",
            };
            let name = item.provenance["target_file"]
                .as_str()
                .context("Minecraft content filename is missing")?;
            anyhow::ensure!(safe_component(name), "Invalid Minecraft filename");
            let source = crate::modpacks::local_directory().join(&item.local_file);
            crate::runtime::no_links(&source)?;
            anyhow::ensure!(
                std::fs::metadata(&source)?.len() <= 128 * 1024 * 1024,
                "Content exceeds limits"
            );
            let bytes = std::fs::read(source)?;
            anyhow::ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == item.sha256,
                "Minecraft recovery digest mismatch"
            );
            std::fs::write(stage.join(route).join(name), bytes)?;
        }
        let mut backed = vec![];
        let mut promoted = vec![];
        let activate = (|| -> Result<()> {
            for route in routes {
                let active = game.path.join(route);
                if active.exists() {
                    std::fs::rename(&active, active.with_extension("canna-previous"))?;
                    backed.push(route);
                }
                std::fs::rename(stage.join(route), &active)?;
                promoted.push(route);
            }
            Ok(())
        })();
        if let Err(error) = activate {
            for route in promoted.into_iter().rev() {
                std::fs::rename(game.path.join(route), stage.join(route))?;
            }
            for route in backed.into_iter().rev() {
                std::fs::rename(
                    game.path.join(route).with_extension("canna-previous"),
                    game.path.join(route),
                )?;
            }
            return Err(error);
        }
        for route in backed {
            let previous = game.path.join(route).with_extension("canna-previous");
            check_tree(&previous)?;
            std::fs::remove_dir_all(previous)?;
        }
        Ok(())
    })();
    if stage.exists() {
        check_tree(&stage)?;
        let absolute = stage.canonicalize()?;
        anyhow::ensure!(
            absolute.starts_with(game.path.canonicalize()?),
            "Recovery stage escapes instance"
        );
        std::fs::remove_dir_all(absolute)?;
    }
    result
}
fn check_tree(path: &Path) -> Result<()> {
    crate::runtime::no_links(path)?;
    for e in std::fs::read_dir(path)? {
        let p = e?.path();
        crate::runtime::no_links(&p)?;
        if p.is_dir() {
            check_tree(&p)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidates_restore_exact_files_and_reject_version_changes() {
        let temporary =
            std::env::temp_dir().join(format!("canna-minecraft-lab-{}", std::process::id()));
        std::fs::create_dir_all(&temporary).unwrap();
        crate::modpacks::with_test_root(temporary.clone(), || {
            let instance = Instance {
                id: "fixture".into(),
                name: "Fixture".into(),
                version: "1.21.1".into(),
                loader: "fabric".into(),
                loader_version: "0.16".into(),
                java: String::new(),
                memory: 2048,
            };
            save(&instance).unwrap();
            let folder = dir(&instance);
            std::fs::create_dir_all(folder.join("mods")).unwrap();
            std::fs::create_dir_all(folder.join("saves/world")).unwrap();
            std::fs::write(folder.join("mods/example+fabric.jar"), b"fixture jar").unwrap();
            std::fs::write(folder.join("saves/world/level.dat"), b"world").unwrap();
            std::fs::create_dir_all(folder.join("config")).unwrap();
            std::fs::write(folder.join("config/test.json"), b"{}").unwrap();
            let (pack, game) = play_setup(&instance).unwrap();
            assert_eq!(pack.mods.len(), 1);
            assert_eq!(
                crate::play_lab::manifest(&pack, Some(&game)).build,
                "1.21.1"
            );
            let mut candidate = pack.clone();
            candidate.id = "candidate".into();
            candidate.name = "Candidate".into();
            create_play_candidate(&candidate, &game).unwrap();
            assert_eq!(
                std::fs::read(root().join("instances/candidate/mods/example+fabric.jar")).unwrap(),
                b"fixture jar"
            );
            assert!(!root().join("instances/candidate/saves").exists());
            assert_eq!(
                std::fs::read(root().join("instances/candidate/config/test.json")).unwrap(),
                b"{}"
            );
            std::fs::write(folder.join("mods/example+fabric.jar"), b"changed").unwrap();
            restore_play_pack(&game, &pack).unwrap();
            assert_eq!(
                std::fs::read(folder.join("mods/example+fabric.jar")).unwrap(),
                b"fixture jar"
            );
            assert_eq!(
                std::fs::read(folder.join("saves/world/level.dat")).unwrap(),
                b"world"
            );
            let mut changed = instance.clone();
            changed.version = "1.22".into();
            save(&changed).unwrap();
            assert!(restore_play_pack(&game, &pack).is_err());
            assert!(!folder.join(".canna-play-stage").exists());
        });
        let resolved = temporary.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
