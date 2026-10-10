//! Explicit non-Unity layouts. Reviewed archives stage outside the game until launch.
use crate::{model::InstalledGame, modpacks::Modpack, runtime};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub fn kind(id: u32) -> Option<&'static str> {
    match id {
        3146520 => Some("gdweave"),
        1337520 => Some("return-of-modding"),
        _ => None,
    }
}
fn root(id: u32) -> &'static str {
    if id == 3146520 {
        "GDWeave"
    } else {
        "ReturnOfModding"
    }
}
fn proxy(id: u32) -> &'static str {
    if id == 3146520 {
        "winmm.dll"
    } else {
        "version.dll"
    }
}
fn framework(id: u32) -> &'static str {
    if id == 3146520 {
        "GDWeave.zip"
    } else {
        "ReturnOfModding.zip"
    }
}
fn safe(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    !s.is_empty()
        && s.len() <= 240
        && s.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && !s.ends_with(['.', ' '])
                && !s.chars().any(|c| c.is_control() || ":<>\"|?*".contains(c))
                && ![
                    "CON", "CONIN$", "CONOUT$", "PRN", "AUX", "NUL", "COM¹", "COM²", "COM³",
                    "LPT¹", "LPT²", "LPT³", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                    "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                    "LPT9",
                ]
                .contains(
                    &s.split('.')
                        .next()
                        .unwrap_or("")
                        .to_ascii_uppercase()
                        .as_str(),
                )
        })
        && path.is_relative()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn owned_path(id: u32, path: &Path) -> bool {
    path == Path::new(proxy(id))
        || (id == 3146520 && (path.starts_with("GDWeave/core") || path.starts_with("GDWeave/mods")))
        || (id == 1337520 && path.starts_with("ReturnOfModding/plugins"))
}
fn metadata(path: &Path) -> bool {
    matches!(
        path.to_string_lossy().to_ascii_lowercase().as_str(),
        "manifest.json" | "readme.md" | "icon.png" | "license" | "license.txt" | "changelog.md"
    )
}
pub fn valid_game(game: &InstalledGame) -> bool {
    crate::game_profiles::by_id(game.app_id)
        .is_some_and(|p| p.executables.iter().any(|e| game.path.join(e).is_file()))
        && (game.app_id != 3146520 || game.path.join("webfishing.pck").is_file())
}
fn loader_entries(id: u32, bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let files = runtime::archive_files(bytes)?;
    let mut result = vec![];
    for (path, bytes) in files {
        if metadata(&path) || path == Path::new("GDWeave/mods/README.txt") {
            continue;
        }
        let path = if id == 1337520 && path == Path::new("ReturnOfModdingPack/version.dll") {
            PathBuf::from("version.dll")
        } else {
            path
        };
        ensure!(
            safe(&path)
                && (path == Path::new(proxy(id))
                    || id == 3146520 && path.starts_with("GDWeave/core")),
            "Unsupported non-Unity loader file: {}",
            path.display()
        );
        result.push((path, bytes));
    }
    ensure!(
        result.iter().any(|(p, _)| p == Path::new(proxy(id))),
        "Loader proxy is missing"
    );
    if id == 3146520 {
        ensure!(
            result
                .iter()
                .any(|(p, _)| p == Path::new("GDWeave/core/GDWeave.dll")),
            "GDWeave core is missing"
        );
    }
    Ok(result)
}
fn mod_entries(id: u32, identity: &str, bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let files = runtime::archive_files(bytes)?;
    if id == 3146520 {
        let mut result = vec![];
        let mut manifests = BTreeSet::new();
        for (path, bytes) in files {
            if metadata(&path) {
                continue;
            }
            ensure!(
                safe(&path) && path.starts_with("GDWeave/mods"),
                "GDWeave mods must use their official GDWeave/mods/<Id> layout"
            );
            let parts: Vec<_> = path.components().collect();
            ensure!(parts.len() >= 4, "Incomplete GDWeave mod path");
            if parts.len() == 4 && path.file_name().is_some_and(|n| n == "manifest.json") {
                let v: serde_json::Value = serde_json::from_slice(&bytes)?;
                let folder = parts[2].as_os_str().to_string_lossy();
                ensure!(
                    v["Id"].as_str() == Some(&folder),
                    "GDWeave folder must match manifest Id"
                );
                for key in ["AssemblyPath", "PackPath"] {
                    if let Some(relative) = v[key].as_str() {
                        ensure!(
                            safe(Path::new(relative)),
                            "Unsafe GDWeave manifest reference"
                        );
                    }
                }
                manifests.insert(path.parent().unwrap().to_owned());
            }
            result.push((path, bytes));
        }
        ensure!(
            !result.is_empty()
                && result
                    .iter()
                    .all(|(p, _)| manifests.iter().any(|m| p.starts_with(m))),
            "Every GDWeave mod needs its Id manifest"
        );
        for (p, b) in &result {
            if p.file_name().is_some_and(|n| n == "manifest.json")
                && manifests.contains(p.parent().unwrap())
            {
                let v: serde_json::Value = serde_json::from_slice(b)?;
                for key in ["AssemblyPath", "PackPath"] {
                    if let Some(r) = v[key].as_str() {
                        ensure!(
                            result
                                .iter()
                                .any(|(f, _)| f == &p.parent().unwrap().join(r)),
                            "GDWeave manifest file is missing"
                        );
                    }
                }
            }
        }
        Ok(result)
    } else {
        ensure!(
            !identity.is_empty()
                && identity.len() <= 100
                && identity
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && identity.contains('-'),
            "A ReturnOfModding package needs a pinned Team-Mod identity"
        );
        let mut result = vec![];
        let target = PathBuf::from("ReturnOfModding/plugins").join(identity);
        for (path, bytes) in files {
            let routed = if let Ok(relative) = path.strip_prefix(&target) {
                target.join(relative)
            } else {
                ensure!(
                    !path.starts_with("ReturnOfModding")
                        && !path.starts_with("plugins")
                        && !path.starts_with("config")
                        && !path.starts_with("plugins_data"),
                    "Unsupported ReturnOfModding package layout; export a plugin package"
                );
                target.join(path)
            };
            ensure!(safe(&routed), "Unsafe ReturnOfModding path");
            result.push((routed, bytes));
        }
        ensure!(
            result.iter().any(|(p, _)| p == &target.join("main.lua"))
                && result
                    .iter()
                    .any(|(p, _)| p == &target.join("manifest.json")),
            "ReturnOfModding requires main.lua and manifest.json"
        );
        Ok(result)
    }
}
type Files = Vec<(PathBuf, Vec<u8>)>;
type History = Vec<(crate::model::ModInfo, Vec<u8>)>;
pub fn prepare(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    progress: &dyn Fn(&str),
) -> Result<(Files, History)> {
    ensure!(
        kind(game.app_id).is_some() && valid_game(game),
        "Non-Unity game executable/content is missing"
    );
    ensure!(
        pack.imported_configs.is_empty(),
        "Imported BepInEx configs cannot be applied to this loader"
    );
    runtime::ensure_closed(game)?;
    let api = runtime::client()?;
    let load = crate::repository::fetch_optional(
        &api,
        &runtime::settings(pack),
        token,
        &runtime::repo_path(pack, &format!("Framework/{}", framework(game.app_id))),
        128 * 1024 * 1024,
    )?
    .context("Import and approve this game's official loader dependency first")?;
    let mut files = loader_entries(game.app_id, &load)?;
    let exe = crate::game_profiles::by_id(game.app_id)
        .unwrap()
        .executables
        .iter()
        .map(|e| game.path.join(e))
        .find(|e| e.is_file())
        .unwrap();
    let machine = runtime::pe_machine(&fs::read(exe)?)?;
    ensure!(
        files
            .iter()
            .filter(|(p, _)| p == Path::new(proxy(game.app_id)))
            .all(|(_, b)| runtime::pe_machine(b).is_ok_and(|m| m == machine)),
        "Loader architecture differs from game executable"
    );
    let mut history = vec![];
    for item in pack.mods.iter().filter(|m| m.enabled) {
        progress(&format!("Preparing {}…", item.name));
        let bytes = if item.local_file.is_empty() {
            crate::repository::fetch_optional(
                &api,
                &runtime::settings(pack),
                token,
                &runtime::repo_path(pack, &item.file),
                128 * 1024 * 1024,
            )?
            .context("Mod archive unavailable")?
        } else {
            fs::read(crate::modpacks::local_directory().join(&item.local_file))?
        };
        ensure!(
            item.sha256.is_empty() || hash(&bytes).eq_ignore_ascii_case(&item.sha256),
            "Mod archive checksum mismatch"
        );
        let identity = item.provenance["id"].as_str().unwrap_or("");
        files.extend(mod_entries(game.app_id, identity, &bytes)?);
        history.push((item.clone(), bytes));
    }
    let mut names = BTreeSet::new();
    let mut total = 0usize;
    for (p, b) in &files {
        ensure!(
            names.insert(p.to_string_lossy().to_lowercase()),
            "Conflicting loader/mod files"
        );
        total += b.len();
    }
    ensure!(
        files.len() <= 20000 && total <= 512 * 1024 * 1024,
        "Non-Unity installation exceeds limits"
    );
    history.push((
        crate::model::ModInfo {
            name: root(game.app_id).into(),
            file: format!("Framework/{}", framework(game.app_id)),
            provenance: serde_json::Value::Null,
            enabled: true,
            version: "framework".into(),
            content_type: String::new(),
            description: String::new(),
            sha256: hash(&load),
            local_file: String::new(),
            dependencies: vec![],
        },
        load,
    ));
    Ok((files, history))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    app_id: u32,
    files: Vec<(PathBuf, String)>,
}
fn receipt(game: &InstalledGame) -> PathBuf {
    crate::runtime_cache::root(game).join("foreign-active.json")
}
pub fn restore(game: &InstalledGame) -> Result<String> {
    runtime::ensure_closed(game)?;
    let path = receipt(game);
    runtime::no_links(&path)?;
    if !path.exists() {
        ensure!(
            !game.path.join(proxy(game.app_id)).exists(),
            "A manual loader proxy is present; Canna will not remove it"
        );
        return Ok("Vanilla files already active.".into());
    }
    ensure!(
        fs::metadata(&path)?.len() <= 2 * 1024 * 1024,
        "Invalid non-Unity receipt"
    );
    let mut r: Receipt = serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        r.app_id == game.app_id && r.files.len() <= 20000,
        "Invalid non-Unity receipt"
    );
    for (p, h) in &r.files {
        ensure!(
            safe(p)
                && h.len() == 64
                && h.bytes().all(|b| b.is_ascii_hexdigit())
                && owned_path(game.app_id, p),
            "Unsafe non-Unity receipt"
        );
        runtime::no_links(&game.path.join(p))?;
    }
    // Remove only content recorded before creation. Proxy first: vanilla remains
    // inert even if a manually edited plugin must stay for inspection.
    r.files
        .sort_by_key(|(p, _)| p != Path::new(proxy(game.app_id)));
    let mut changed = false;
    for (p, h) in &r.files {
        let target = game.path.join(p);
        if target.exists() {
            if target.is_file()
                && fs::metadata(&target)?.len() <= 128 * 1024 * 1024
                && hash(&fs::read(&target)?) == *h
            {
                fs::remove_file(target)?;
            } else {
                changed = true;
            }
        }
    }
    ensure!(
        !changed,
        "Edited loader/mod files were preserved; inspect them before reapplying"
    );
    fs::remove_file(path)?;
    Ok("Canna non-Unity loader and mod files removed; user data preserved.".into())
}
pub fn install(game: &InstalledGame, files: Files) -> Result<()> {
    ensure!(kind(game.app_id).is_some(), "Unsupported non-Unity loader");
    runtime::ensure_closed(game)?;
    let mut names = BTreeSet::new();
    let mut total = 0usize;
    for (p, b) in &files {
        ensure!(
            safe(p) && owned_path(game.app_id, p),
            "Unsafe non-Unity installation path"
        );
        ensure!(
            names.insert(p.to_string_lossy().to_lowercase()),
            "Conflicting non-Unity installation paths"
        );
        total = total
            .checked_add(b.len())
            .context("Installation size overflow")?;
        ensure!(
            b.len() <= 128 * 1024 * 1024,
            "Non-Unity file exceeds limits"
        );
    }
    ensure!(
        files.len() <= 20000 && total <= 512 * 1024 * 1024,
        "Non-Unity installation exceeds limits"
    );
    restore(game)?;
    let r = Receipt {
        app_id: game.app_id,
        files: files.iter().map(|(p, b)| (p.clone(), hash(b))).collect(),
    };
    for (p, _) in &files {
        ensure!(safe(p), "Unsafe non-Unity installation path");
        runtime::no_links(&game.path.join(p))?;
        ensure!(
            !game.path.join(p).exists(),
            "A manual file occupies {}",
            p.display()
        );
    }
    let path = receipt(game);
    runtime::no_links(&path)?;
    fs::create_dir_all(path.parent().unwrap())?;
    // Write-ahead ownership is recoverable if the process stops mid-install.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?
        .write_all(&serde_json::to_vec(&r)?)?;
    let result = (|| {
        for (p, b) in files {
            let target = game.path.join(p);
            fs::create_dir_all(target.parent().unwrap())?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?
                .write_all(&b)?;
        }
        Ok(())
    })();
    if result.is_err() {
        restore(game)?;
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (p, b) in files {
            w.start_file(*p, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(b).unwrap();
        }
        w.finish().unwrap().into_inner()
    }
    #[test]
    fn gdweave_requires_exact_ids_and_manifest_files() {
        let good = zip(&[
            (
                "GDWeave/mods/Test/manifest.json",
                br#"{"Id":"Test","AssemblyPath":"Test.dll"}"#,
            ),
            ("GDWeave/mods/Test/Test.dll", b"fixture"),
        ]);
        assert_eq!(mod_entries(3146520, "", &good).unwrap().len(), 2);
        assert!(
            mod_entries(
                3146520,
                "",
                &zip(&[("GDWeave/mods/Test/manifest.json", br#"{"Id":"Wrong"}"#)])
            )
            .is_err()
        );
        assert!(
            mod_entries(
                3146520,
                "",
                &zip(&[("BepInEx/plugins/Test.dll", b"fixture")])
            )
            .is_err()
        );
        assert!(mod_entries(3146520, "", &good[..good.len() / 2]).is_err());
    }
    #[test]
    fn returns_preserves_plugin_hierarchy_without_unity_injection() {
        let b = zip(&[
            ("main.lua", b"-- inert fixture"),
            ("manifest.json", b"{}"),
            ("assets/sample.txt", b"sample"),
        ]);
        let files = mod_entries(1337520, "Team-Mod", &b).unwrap();
        assert!(
            files
                .iter()
                .all(|(p, _)| p.starts_with("ReturnOfModding/plugins/Team-Mod"))
        );
        assert!(mod_entries(1337520, "../Bad", &b).is_err());
        assert!(loader_entries(1337520, &zip(&[("winhttp.dll", b"fake")])).is_err());
    }
    #[test]
    fn install_cleanup_preserves_manual_changes_and_user_data() {
        let temp = std::env::temp_dir().join(format!(
            "canna-foreign-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp).unwrap();
        let game = InstalledGame {
            app_id: 1337520,
            path: temp.clone(),
            name: "Fixture".into(),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        let files = vec![
            (PathBuf::from("version.dll"), b"proxy fixture".to_vec()),
            (
                PathBuf::from("ReturnOfModding/plugins/Team-Mod/main.lua"),
                b"-- inert".to_vec(),
            ),
        ];
        install(&game, files.clone()).unwrap();
        assert!(
            install(
                &game,
                vec![(
                    PathBuf::from("ReturnOfModding/config/user.cfg"),
                    b"bad".to_vec()
                )]
            )
            .is_err()
        );
        assert_eq!(
            fs::read(temp.join("version.dll")).unwrap(),
            b"proxy fixture"
        );
        fs::create_dir_all(temp.join("ReturnOfModding/config")).unwrap();
        fs::write(temp.join("ReturnOfModding/config/user.cfg"), b"keep").unwrap();
        restore(&game).unwrap();
        assert!(!temp.join("version.dll").exists());
        assert!(
            !temp
                .join("ReturnOfModding/plugins/Team-Mod/main.lua")
                .exists()
        );
        assert_eq!(
            fs::read(temp.join("ReturnOfModding/config/user.cfg")).unwrap(),
            b"keep"
        );
        install(&game, files).unwrap();
        fs::write(
            temp.join("ReturnOfModding/plugins/Team-Mod/main.lua"),
            b"edited",
        )
        .unwrap();
        assert!(restore(&game).is_err());
        assert!(!temp.join("version.dll").exists());
        assert_eq!(
            fs::read(temp.join("ReturnOfModding/plugins/Team-Mod/main.lua")).unwrap(),
            b"edited"
        );
        fs::remove_dir_all(crate::runtime_cache::root(&game)).unwrap();
        fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn official_loader_archives_have_supported_layouts() {
        for (id, file) in [(3146520, "GDWeave.zip"), (1337520, "ReturnOfModding.zip")] {
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/overnight-growth/research")
                .join(file);
            if p.is_file() {
                assert!(loader_entries(id, &fs::read(p).unwrap()).is_ok());
            }
        }
    }
}
