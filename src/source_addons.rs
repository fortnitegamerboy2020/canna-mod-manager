use crate::{model::InstalledGame, modpacks::Modpack, runtime};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

fn addons(game: &InstalledGame) -> Result<PathBuf> {
    let path = game
        .path
        .join(crate::model::source_addons(game.app_id).context("Unsupported Source game")?);
    runtime::no_links(&path)?;
    Ok(path)
}
fn store(game: &InstalledGame) -> PathBuf {
    crate::runtime_cache::root(game).join("source")
}
pub fn setup(game: &InstalledGame) -> Result<()> {
    runtime::ensure_closed(game)?;
    let path = addons(game)?;
    if !path.parent().unwrap().join("gameinfo.txt").is_file() {
        bail!("Source game content is missing; verify the installation in Steam");
    }
    fs::create_dir_all(path)?;
    Ok(())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn plugin_vdf(name: &str) -> Vec<u8> {
    format!(
        "\"Plugin\" {{ \"file\" \"addons/{}\" }}\n",
        name.trim_end_matches(".dll")
    )
    .into_bytes()
}
fn check_plugin(bytes: &[u8]) -> Result<()> {
    // L4D2's Windows server plugin ABI is 32-bit x86. Reject executables,
    // incompatible architectures and malformed PE headers before installation.
    anyhow::ensure!(
        bytes.len() >= 64 && bytes.len() <= 8 * 1024 * 1024 && &bytes[..2] == b"MZ",
        "Invalid Source plugin DLL"
    );
    let offset = u32::from_le_bytes(bytes[60..64].try_into()?) as usize;
    let header = bytes
        .get(offset..offset.saturating_add(24))
        .context("Truncated Source plugin PE header")?;
    anyhow::ensure!(
        &header[..4] == b"PE\0\0"
            && header[4..6] == [0x4c, 1]
            && u16::from_le_bytes(header[22..24].try_into()?) & 0x2000 != 0,
        "Source plugin must be a 32-bit x86 DLL"
    );
    Ok(())
}
fn native_library(game: &InstalledGame, content: &[(PathBuf, Vec<u8>)]) -> Result<Option<Vec<u8>>> {
    let Some((_, data)) = content
        .iter()
        .find(|(path, _)| path == std::path::Path::new("manifest.json"))
    else {
        return Ok(None);
    };
    anyhow::ensure!(data.len() <= 8192, "Source plugin manifest exceeds limits");
    let manifest: serde_json::Value = serde_json::from_slice(data)?;
    anyhow::ensure!(
        manifest["format"] == "canna-source-plugin-v1"
            && manifest["game"] == game.app_id
            && game.app_id == 550
            && manifest["library"] == "plugin.dll",
        "Unsupported Source plugin manifest"
    );
    let bytes = &content
        .iter()
        .find(|(path, _)| path == std::path::Path::new("plugin.dll"))
        .context("Source plugin DLL is missing")?
        .1;
    anyhow::ensure!(
        manifest["sha256"].as_str() == Some(hash(bytes).as_str()),
        "Source plugin DLL checksum mismatch"
    );
    check_plugin(bytes)?;
    Ok(Some(bytes.clone()))
}
/// Only self-contained VPKs. Split archives require all matching segments and are not supported.
fn check_vpk(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 12 || bytes[..4] != [0x34, 0x12, 0xaa, 0x55] {
        bail!("Invalid VPK header");
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into()?);
    let header = match version {
        1 => 12,
        2 => 28,
        _ => bail!("Unsupported VPK version"),
    };
    let tree = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
    if tree == 0 || header + tree > bytes.len() {
        bail!("Truncated VPK directory");
    }
    // Parse directory entries to reject split VPK references and malformed directory trees.
    let directory = &bytes[header..header + tree];
    let mut pos = 0;
    fn text<'a>(data: &'a [u8], pos: &mut usize) -> Result<&'a [u8]> {
        let start = *pos;
        let len = data
            .get(start..)
            .context("Truncated VPK tree")?
            .iter()
            .position(|v| *v == 0)
            .context("Unterminated VPK name")?;
        *pos += len + 1;
        Ok(&data[start..start + len])
    }
    loop {
        if text(directory, &mut pos)?.is_empty() {
            break;
        }
        loop {
            if text(directory, &mut pos)?.is_empty() {
                break;
            }
            loop {
                if text(directory, &mut pos)?.is_empty() {
                    break;
                }
                let entry = directory
                    .get(pos..pos + 18)
                    .context("Truncated VPK entry")?;
                let preload = u16::from_le_bytes(entry[4..6].try_into()?) as usize;
                if u16::from_le_bytes(entry[6..8].try_into()?) != 0x7fff {
                    bail!("Split VPKs are not supported; use a self-contained addon");
                }
                if entry[16..18] != [0xff, 0xff] {
                    bail!("Invalid VPK entry terminator");
                }
                let offset = u32::from_le_bytes(entry[8..12].try_into()?) as usize;
                let length = u32::from_le_bytes(entry[12..16].try_into()?) as usize;
                if offset
                    .checked_add(length)
                    .is_none_or(|end| end > bytes.len() - header - tree)
                {
                    bail!("VPK data is truncated");
                }
                pos += 18 + preload;
                if pos > directory.len() {
                    bail!("VPK preload data is truncated");
                }
            }
        }
    }
    Ok(())
}
fn entries(game: &InstalledGame) -> Result<Vec<(String, Vec<u8>)>> {
    let root = if store(game).exists() {
        store(game)
    } else {
        game.path.join(".canna-source")
    };
    runtime::no_links(&root)?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(root)? {
        let item = item?;
        let path = item.path();
        runtime::no_links(&path)?;
        let name = item.file_name().to_string_lossy().into_owned();
        let extension = if name.ends_with(".vpk") {
            "vpk"
        } else if name.ends_with(".dll") && game.app_id == 550 {
            "dll"
        } else {
            bail!("Unexpected file in Canna Source store");
        };
        if !name.starts_with("canna-") || !item.file_type()?.is_file() {
            bail!("Unexpected file in Canna Source store");
        }
        anyhow::ensure!(
            entries.len() < 1000 && item.metadata()?.len() <= 32 * 1024 * 1024,
            "Source store exceeds its limits"
        );
        let data = fs::read(path)?;
        if name != format!("canna-{}.{}", hash(&data), extension) {
            bail!("Canna addon checksum mismatch");
        }
        if extension == "dll" {
            check_plugin(&data)?;
        }
        entries.push((name, data));
    }
    Ok(entries)
}
pub fn set_mode(game: &InstalledGame, enabled: bool) -> Result<()> {
    let target = addons(game)?;
    let files = entries(game)?;
    // Preflight all files before changing any. Never overwrite an unrelated addon.
    for (name, data) in &files {
        let path = target.join(name);
        runtime::no_links(&path)?;
        if path.exists() && hash(&fs::read(&path)?) != hash(data) {
            bail!("Addon changed outside Canna: {name}; restore or move it before switching packs");
        }
        if name.ends_with(".dll") {
            let vdf = target.join(name.replace(".dll", ".vdf"));
            runtime::no_links(&vdf)?;
            if vdf.exists() && fs::read(&vdf)? != plugin_vdf(name) {
                bail!("Plugin registration changed outside Canna: {name}");
            }
        }
    }
    fs::create_dir_all(&target)?;
    for (name, data) in files {
        let path = target.join(&name);
        let registration = name
            .ends_with(".dll")
            .then(|| target.join(name.replace(".dll", ".vdf")));
        if enabled {
            if !path.exists() {
                fs::write(path, data)?;
            }
            if let Some(vdf) = registration {
                fs::write(vdf, plugin_vdf(&name))?;
            }
        } else {
            if let Some(vdf) = registration
                && vdf.exists()
            {
                fs::remove_file(vdf)?;
            }
            if path.exists() {
                fs::remove_file(path)?;
            }
        }
    }
    if !enabled {
        migrate_store(game)?;
    }
    Ok(())
}
fn migrate_store(game: &InstalledGame) -> Result<()> {
    let legacy = game.path.join(".canna-source");
    runtime::no_links(&legacy)?;
    if !legacy.exists() {
        return Ok(());
    }
    anyhow::ensure!(
        !store(game).exists(),
        "Both old and current Source caches exist; preserve them for recovery"
    );
    let files = entries(game)?;
    let root = store(game);
    runtime::no_links(&root)?;
    fs::create_dir_all(&root)?;
    for (name, data) in &files {
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join(name))?;
        std::io::Write::write_all(&mut out, data)?;
        out.sync_all()?;
    }
    for (name, data) in &files {
        anyhow::ensure!(
            fs::read(root.join(name))? == *data && fs::read(legacy.join(name))? == *data,
            "Source cache changed during migration; both copies preserved"
        );
    }
    for (name, _) in files {
        runtime::no_links(&legacy.join(&name))?;
        fs::remove_file(legacy.join(name))?;
    }
    fs::remove_dir(legacy)?;
    Ok(())
}
pub(crate) fn prepare_files(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    progress: &dyn Fn(&str),
) -> Result<Vec<(String, Vec<u8>)>> {
    pack.validate()?;
    runtime::ensure_closed(game)?;
    anyhow::ensure!(
        addons(game)?
            .parent()
            .unwrap()
            .join("gameinfo.txt")
            .is_file(),
        "Source game content is missing; verify it in Steam"
    );
    let api = runtime::client()?;
    let mut files = Vec::new();
    let mut downloaded = 0usize;
    for item in pack.mods.iter().filter(|item| item.enabled) {
        anyhow::ensure!(
            item.provenance["external_only"] != true,
            "{} is an original-site download, not a Canna modpack addon",
            item.name
        );
        progress(&format!("Preparing Source addon: {}", item.name));
        let data = if !item.local_file.is_empty() {
            fs::read(crate::modpacks::local_directory().join(&item.local_file))?
        } else {
            crate::repository::fetch_optional(
                &api,
                &runtime::settings(pack),
                token,
                &runtime::repo_path(pack, &item.file),
                32 * 1024 * 1024,
            )?
            .context("Addon not available on the Canna server")?
        };
        downloaded = downloaded
            .checked_add(data.len())
            .context("Source pack size overflow")?;
        anyhow::ensure!(
            downloaded <= 256 * 1024 * 1024,
            "Source pack exceeds the preparation budget"
        );
        if item.sha256.is_empty() || hash(&data) != item.sha256.to_lowercase() {
            bail!("Addon checksum mismatch: {}", item.name);
        }
        let content = if item.file.to_lowercase().ends_with(".vpk") {
            vec![(PathBuf::from("addon.vpk"), data.clone())]
        } else if item.file.to_lowercase().ends_with(".zip") {
            runtime::archive_files(&data)?
        } else {
            bail!("Source packs accept VPK files or ZIPs containing VPKs");
        };
        let native = native_library(game, &content)?;
        if let Some(bytes) = &native {
            files.push((format!("canna-{}.dll", hash(bytes)), bytes.clone()));
        }
        let mut count = 0;
        for (path, bytes) in content {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("vpk"))
            {
                check_vpk(&bytes)?;
                files.push((format!("canna-{}.vpk", hash(&bytes)), bytes));
                count += 1;
            } else if path == std::path::Path::new("plugin.dll") && native.is_some() {
                // The verified library is installed under a content-derived name;
                // registrations are generated locally, never accepted from archives.
            } else if !matches!(
                path.file_name().and_then(|n| n.to_str()),
                Some("README.md" | "LICENSE" | "manifest.json" | "icon.png")
            ) {
                bail!(
                    "Source addon archive contains unsupported file: {}",
                    path.display()
                );
            }
        }
        if count == 0 {
            bail!("{} contains no VPK addons", item.name);
        }
        let _ = crate::website::remember_mod(pack, item, &data, false);
    }
    Ok(files)
}
#[cfg(test)]
pub fn install(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    progress: &dyn Fn(&str),
) -> Result<()> {
    let files = prepare_files(game, pack, token, progress)?;
    install_files(game, files, progress)
}
pub(crate) fn install_files(
    game: &InstalledGame,
    files: Vec<(String, Vec<u8>)>,
    progress: &dyn Fn(&str),
) -> Result<()> {
    setup(game)?;
    let root = store(game);
    let stage = crate::runtime_cache::root(game).join("source-stage");
    let previous = crate::runtime_cache::root(game).join("source-previous");
    runtime::no_links(&stage)?;
    fs::create_dir_all(crate::runtime_cache::root(game))?;
    for path in [&root, &stage, &previous] {
        runtime::no_links(path)?;
    }
    if stage.exists() || previous.exists() {
        bail!("Previous Source pack operation needs recovery; leave its files intact");
    }
    fs::create_dir(&stage)?;
    for (name, data) in files {
        fs::write(stage.join(name), data)?;
    }
    runtime::ensure_closed(game)?;
    if let Err(error) = set_mode(game, false) {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    if root.exists() {
        fs::rename(&root, &previous)?;
    }
    if let Err(error) = fs::rename(&stage, &root) {
        if previous.exists() {
            fs::rename(&previous, &root)?;
        }
        return Err(error.into());
    }
    if previous.exists() {
        fs::remove_dir_all(previous)?;
    }
    progress(
        "Source pack saved. Launch modded uses -insecure practice mode; vanilla disables Canna addons.",
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_source_store_moves_outside_game_on_vanilla_cleanup() {
        let game = fixture();
        let data = b"fixture cache bytes";
        let name = format!("canna-{}.vpk", hash(data));
        let legacy = game.path.join(".canna-source");
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join(&name), data).unwrap();
        fs::write(addons(&game).unwrap().join(&name), data).unwrap();
        fs::write(addons(&game).unwrap().join("manual.vpk"), b"manual addon").unwrap();
        set_mode(&game, false).unwrap();
        assert!(!legacy.exists());
        assert!(!addons(&game).unwrap().join(&name).exists());
        assert_eq!(fs::read(store(&game).join(&name)).unwrap(), data);
        assert_eq!(
            fs::read(addons(&game).unwrap().join("manual.vpk")).unwrap(),
            b"manual addon"
        );
        assert!(!store(&game).starts_with(&game.path));
        fs::remove_dir_all(crate::runtime_cache::root(&game)).unwrap();
        fs::remove_dir_all(game.path).unwrap();
    }
    fn fixture() -> InstalledGame {
        let root = std::env::temp_dir().join(format!(
            "canna-source-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("left4dead2/addons")).unwrap();
        fs::write(root.join("left4dead2/gameinfo.txt"), "fixture").unwrap();
        InstalledGame {
            app_id: 550,
            name: "Left 4 Dead 2".into(),
            path: root,
            loader: String::new(),
            plugins: 0,
            icon: None,
        }
    }
    fn test_dll() -> Vec<u8> {
        let mut bytes = vec![0; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        bytes[68..70].copy_from_slice(&0x14cu16.to_le_bytes());
        bytes[86..88].copy_from_slice(&0x2000u16.to_le_bytes());
        bytes
    }
    #[test]
    fn plugin_manifest_checks_game_digest_architecture_and_missing_library() {
        let mut game = fixture();
        let bytes = test_dll();
        let manifest = serde_json::json!({"format":"canna-source-plugin-v1","game":550,"library":"plugin.dll","sha256":hash(&bytes)});
        let mut content = vec![
            (
                PathBuf::from("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            ),
            (PathBuf::from("plugin.dll"), bytes),
        ];
        assert!(native_library(&game, &content).unwrap().is_some());
        game.app_id = 500;
        assert!(native_library(&game, &content).is_err());
        game.app_id = 550;
        content[1].1[68] = 0x64;
        assert!(native_library(&game, &content).is_err());
        assert!(check_plugin(&content[1].1).is_err());
        content.pop();
        assert!(native_library(&game, &content).is_err());
        fs::remove_dir_all(game.path).unwrap();
    }
    #[test]
    fn plugin_switching_manages_registration_and_preserves_unrelated_plugins() {
        let game = fixture();
        let bytes = test_dll();
        fs::create_dir_all(store(&game)).unwrap();
        let name = format!("canna-{}.dll", hash(&bytes));
        fs::write(store(&game).join(&name), &bytes).unwrap();
        let target = addons(&game).unwrap();
        let vdf = target.join(name.replace(".dll", ".vdf"));
        fs::write(target.join("other.vdf"), "keep").unwrap();
        set_mode(&game, true).unwrap();
        assert_eq!(fs::read(&vdf).unwrap(), plugin_vdf(&name));
        fs::write(&vdf, "modified").unwrap();
        assert!(set_mode(&game, false).is_err());
        assert!(target.join(&name).exists());
        fs::write(&vdf, plugin_vdf(&name)).unwrap();
        set_mode(&game, false).unwrap();
        assert!(!target.join(&name).exists());
        assert!(!vdf.exists());
        assert_eq!(fs::read(target.join("other.vdf")).unwrap(), b"keep");
        fs::remove_dir_all(game.path).unwrap();
    }
    #[test]
    fn switching_preserves_other_addons_and_rejects_changed_files() {
        let game = fixture();
        let data = [0x34, 0x12, 0xaa, 0x55, 1, 0, 0, 0, 1, 0, 0, 0, 0];
        check_vpk(&data).unwrap();
        fs::create_dir_all(store(&game)).unwrap();
        let name = format!("canna-{}.vpk", hash(&data));
        fs::write(store(&game).join(&name), data).unwrap();
        let target = addons(&game).unwrap();
        fs::write(target.join("unmanaged.vpk"), "keep").unwrap();
        set_mode(&game, true).unwrap();
        assert!(target.join(&name).exists());
        set_mode(&game, false).unwrap();
        assert!(!target.join(&name).exists());
        assert_eq!(fs::read(target.join("unmanaged.vpk")).unwrap(), b"keep");
        fs::write(target.join(&name), "changed").unwrap();
        assert!(set_mode(&game, false).is_err());
        assert!(target.join(&name).exists());
        fs::remove_dir_all(game.path).unwrap();
    }
    #[test]
    fn local_vpk_pack_install_and_bundle_round_trip() {
        let game = fixture();
        let source = game.path.join("practice.vpk");
        let mut data = vec![0x34, 0x12, 0xaa, 0x55, 1, 0, 0, 0, 1, 0, 0, 0, 0];
        data.extend_from_slice(game.path.to_string_lossy().as_bytes());
        fs::write(&source, &data).unwrap();
        let item = crate::modpacks::add_local(&source).unwrap();
        let info = crate::model::supported_catalog()
            .into_iter()
            .find(|g| g.app_id == 550)
            .unwrap();
        let pack = Modpack::create(
            "Practice".into(),
            String::new(),
            &info,
            crate::cache::Source {
                owner: "canna".into(),
                repository: "server".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            },
            vec![item.clone()],
        );
        pack.validate().unwrap();
        install(&game, &pack, "", &|_| {}).unwrap();
        assert_eq!(entries(&game).unwrap().len(), 1);
        set_mode(&game, true).unwrap();
        assert!(
            addons(&game)
                .unwrap()
                .join(format!("canna-{}.vpk", hash(&data)))
                .exists()
        );
        let mut empty = pack.clone();
        empty.mods.clear();
        install(&game, &empty, "", &|_| {}).unwrap();
        assert!(entries(&game).unwrap().is_empty());
        assert!(
            !addons(&game)
                .unwrap()
                .join(format!("canna-{}.vpk", hash(&data)))
                .exists()
        );
        let bundle = game.path.join("pack.canna.zip");
        pack.export(&bundle).unwrap();
        let archive = runtime::archive_files(&fs::read(bundle).unwrap()).unwrap();
        assert!(
            archive
                .iter()
                .any(|(path, bytes)| path.to_string_lossy().ends_with(".vpk") && bytes == &data)
        );
        fs::remove_file(crate::modpacks::local_directory().join(item.local_file)).unwrap();
        fs::remove_dir_all(game.path).unwrap();
    }
    #[test]
    #[ignore = "Requires scripts/Build-AutoHop.ps1; temporary game fixture only"]
    fn native_autohop_package_install_and_vanilla_switch() {
        let game = fixture();
        let item = crate::modpacks::add_local(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("server/staging-autohop/Canna-Auto-Hop.zip"),
        )
        .unwrap();
        let info = crate::model::supported_catalog()
            .into_iter()
            .find(|g| g.app_id == 550)
            .unwrap();
        let pack = Modpack::create(
            "Auto-Hop".into(),
            String::new(),
            &info,
            crate::cache::Source {
                owner: "canna".into(),
                repository: "server".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            },
            vec![item.clone()],
        );
        install(&game, &pack, "", &|_| {}).unwrap();
        assert_eq!(entries(&game).unwrap().len(), 2);
        set_mode(&game, true).unwrap();
        assert_eq!(fs::read_dir(addons(&game).unwrap()).unwrap().count(), 3);
        set_mode(&game, false).unwrap();
        assert_eq!(fs::read_dir(addons(&game).unwrap()).unwrap().count(), 0);
        fs::remove_file(crate::modpacks::local_directory().join(item.local_file)).unwrap();
        fs::remove_dir_all(game.path).unwrap();
    }
    #[test]
    fn invalid_and_split_vpks_rejected() {
        assert!(check_vpk(b"not a vpk").is_err());
        let mut data = vec![0x34, 0x12, 0xaa, 0x55, 1, 0, 0, 0, 0, 0, 0, 0];
        let mut tree = b"txt\0 \0example\0".to_vec();
        tree.extend_from_slice(&[0; 18]);
        tree.extend_from_slice(&[0, 0, 0]);
        data[8..12].copy_from_slice(&(tree.len() as u32).to_le_bytes());
        data.extend(tree);
        assert!(check_vpk(&data).is_err());
    }
    #[test]
    #[ignore = "Requires scripts/Prepare-SourceCatalog.py; temporary game fixtures only"]
    fn curated_source_archives_install_without_touching_real_games() {
        for name in [
            "L4D2-Practice-Script",
            "L4dAutoConfig",
            "L4dRemovedMainMenuMusic",
        ] {
            let mut game = fixture();
            if name != "L4D2-Practice-Script" {
                game.app_id = 500;
                fs::create_dir_all(game.path.join("left4dead/addons")).unwrap();
                fs::write(game.path.join("left4dead/gameinfo.txt"), "fixture").unwrap();
            }
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("server/staging-source")
                .join(format!("{name}.zip"));
            let data = fs::read(&path).unwrap();
            let archive = runtime::archive_files(&data).unwrap();
            assert!(
                archive
                    .iter()
                    .any(|(p, _)| p.to_string_lossy() == "LICENSE")
            );
            let vpk = &archive
                .iter()
                .find(|(p, _)| p.to_string_lossy() == "addon.vpk")
                .unwrap()
                .1;
            check_vpk(vpk).unwrap();
            let item = crate::modpacks::add_local(&path).unwrap();
            let info = crate::model::supported_catalog()
                .into_iter()
                .find(|g| g.app_id == game.app_id)
                .unwrap();
            let mut pack = Modpack::create(
                "Curated fixture".into(),
                String::new(),
                &info,
                crate::cache::Source {
                    owner: "canna".into(),
                    repository: "server".into(),
                    branch: "main".into(),
                    catalog_folder: String::new(),
                },
                vec![item.clone()],
            );
            pack.validate().unwrap();
            install(&game, &pack, "", &|_| {}).unwrap();
            set_mode(&game, true).unwrap();
            assert_eq!(
                fs::read(
                    addons(&game)
                        .unwrap()
                        .join(format!("canna-{}.vpk", hash(vpk)))
                )
                .unwrap(),
                *vpk
            );
            set_mode(&game, false).unwrap();
            pack.mods[0].provenance = serde_json::json!({"external_only":true});
            assert!(pack.validate().is_err());
            assert!(install(&game, &pack, "", &|_| {}).is_err());
            fs::remove_file(crate::modpacks::local_directory().join(item.local_file)).unwrap();
            fs::remove_dir_all(game.path).unwrap();
        }
    }
}
