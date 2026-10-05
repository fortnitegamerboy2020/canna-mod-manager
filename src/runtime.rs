use crate::{
    model::{InstalledGame, Settings},
    modpacks::Modpack,
    repository,
};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const BOPL_FRAMEWORK_URL: &str =
    "https://github.com/BepInEx/BepInEx/releases/download/v5.4.23.5/BepInEx_win_x64_5.4.23.5.zip";

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("Canna-Mod-Manager/0.1")
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn settings(pack: &Modpack) -> Settings {
    Settings {
        owner: pack.repository.owner.clone(),
        repository: pack.repository.repository.clone(),
        branch: pack.repository.branch.clone(),
        catalog_folder: pack.repository.catalog_folder.clone(),
        steam_path: String::new(),
    }
}
fn repo_path(pack: &Modpack, file: &str) -> String {
    [
        pack.repository.catalog_folder.trim_matches('/'),
        &pack.game.folder,
        file,
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("/")
}
fn no_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    bail!("Linked game or staging paths are unsupported")
                }
            }
            if metadata.file_type().is_symlink() {
                bail!("Linked paths are unsupported")
            }
        }
    }
    Ok(())
}
fn remove_managed(game_root: &Path, target: &Path) -> Result<()> {
    no_links(target)?;
    let root = fs::canonicalize(game_root)?;
    let resolved = fs::canonicalize(target)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    if !resolved.starts_with(&root)
        || resolved == root
        || !(name == "Canna.previous" || name.starts_with(".canna-stage-"))
    {
        bail!("Refusing to remove a path outside Canna's game staging folders")
    }
    fn check_tree(path: &Path) -> Result<()> {
        no_links(path)?;
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                check_tree(&entry?.path())?;
            }
        }
        Ok(())
    }
    check_tree(target)?;
    fs::remove_dir_all(target)?;
    Ok(())
}
pub fn game_running(game: &InstalledGame) -> Result<bool> {
    no_links(&game.path)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
            "$r=$env:CANNA_GAME_ROOT; $p=@(Get-Process | Where-Object { $_.Path -and [System.IO.Path]::GetDirectoryName($_.Path) -eq $r }); if ($p.Count) { exit 2 }"])
            .env("CANNA_GAME_ROOT", &game.path).creation_flags(0x08000000).output()?;
        match output.status.code() {
            Some(0) => Ok(false),
            Some(2) => Ok(true),
            _ => bail!("Could not determine game process status"),
        }
    }
    #[cfg(not(windows))]
    Ok(false)
}
pub fn ensure_closed(game: &InstalledGame) -> Result<()> {
    if game_running(game)? {
        bail!("Close the game before changing its mod setup")
    }
    Ok(())
}
pub(crate) fn archive_files(bytes: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    if archive.len() > 2000 {
        bail!("Archive has too many files")
    }
    let mut result = Vec::new();
    let mut total = 0u64;
    let mut names = std::collections::BTreeSet::new();
    for i in 0..archive.len() {
        let item = archive.by_index(i)?;
        let path = item.enclosed_name().context("Unsafe archive path")?;
        if item
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("Archive links are unsupported")
        }
        if item.is_dir() {
            continue;
        }
        if path.components().any(|part| match part {
            std::path::Component::Normal(s) => {
                let value = s.to_string_lossy();
                value.contains([':', '\\']) || value.ends_with(['.', ' '])
            }
            _ => true,
        }) {
            bail!("Unsafe archive filename")
        }
        if !names.insert(path.to_string_lossy().to_lowercase()) {
            bail!("Duplicate archive path")
        }
        total += item.size();
        if total > 256 * 1024 * 1024 {
            bail!("Expanded archive exceeds 256 MiB")
        }
        let mut data = Vec::new();
        item.take(256 * 1024 * 1024 + 1).read_to_end(&mut data)?;
        if data.len() > 256 * 1024 * 1024 {
            bail!("Archive entry exceeds limit")
        }
        result.push((path, data));
    }
    Ok(result)
}
fn write_new(root: &Path, entries: &[(PathBuf, Vec<u8>)]) -> Result<()> {
    for (path, _) in entries {
        let dest = root.join(path);
        no_links(&dest)?;
        if dest.exists() {
            bail!("Existing file would be overwritten: {}", path.display())
        }
    }
    let mut written = Vec::new();
    let result: Result<()> = (|| {
        for (path, bytes) in entries {
            let dest = root.join(path);
            fs::create_dir_all(dest.parent().unwrap())?;
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&dest)?;
            written.push(dest);
            file.write_all(bytes)?;
        }
        Ok(())
    })();
    if result.is_err() {
        for path in written {
            let _ = fs::remove_file(path);
        }
    }
    result
}
pub fn setup(game: &InstalledGame, pack: &Modpack, token: &str) -> Result<()> {
    ensure_closed(game)?;
    if game.path.join("BepInEx/core/BepInEx.dll").is_file()
        && game.path.join("doorstop_config.ini").is_file()
        && (game.path.join("winhttp.dll").is_file() || game.path.join("version.dll").is_file())
    {
        return Ok(());
    }
    if game.path.join("BepInEx/core/BepInEx.Core.dll").is_file() {
        bail!("BepInEx 6 detected; automatic BepInEx 5 setup is unsupported")
    }
    if !game
        .path
        .join("BoplBattle_Data/Managed/Assembly-CSharp.dll")
        .is_file()
        && game.app_id == 1686940
    {
        bail!("Bopl Battle Mono files were not found")
    }
    let api = client()?;
    let bytes = repository::fetch_optional(
        &api,
        &settings(pack),
        token,
        &repo_path(pack, "Framework/BepInEx.zip"),
        32 * 1024 * 1024,
    )?;
    let bytes = match bytes {
        Some(bytes) => bytes,
        None if game.app_id == 1686940 => {
            let download = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()?;
            let response = download
                .get(BOPL_FRAMEWORK_URL)
                .send()?
                .error_for_status()?;
            let mut bytes = Vec::new();
            response
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 32 * 1024 * 1024 {
                bail!("Framework archive exceeds limit")
            }
            bytes
        }
        None => bail!(
            "Upload a compatible BepInEx 5 package as {}/Framework/BepInEx.zip first",
            pack.game.folder
        ),
    };
    let entries = archive_files(&bytes)?;
    for required in [
        "BepInEx/core/BepInEx.dll",
        "winhttp.dll",
        "doorstop_config.ini",
    ] {
        if !entries.iter().any(|(path, _)| path == Path::new(required)) {
            bail!("Framework archive is missing {required}; use the Windows BepInEx 5 ZIP")
        }
    }
    if entries.iter().any(|(p, _)| {
        !(p.starts_with("BepInEx")
            || [
                "winhttp.dll",
                "doorstop_config.ini",
                ".doorstop_version",
                "changelog.txt",
            ]
            .iter()
            .any(|allowed| p == Path::new(allowed)))
    }) {
        bail!("Unexpected files in framework archive")
    }
    // Never overwrite another loader or partial installation.
    ensure_closed(game)?;
    write_new(&game.path, &entries)?;
    fs::create_dir_all(game.path.join("BepInEx/plugins"))?;
    fs::create_dir_all(game.path.join("BepInEx/config"))?;
    Ok(())
}
pub fn install_pack(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    progress: &dyn Fn(&str),
) -> Result<()> {
    pack.validate()?;
    progress("Checking BepInEx…");
    setup(game, pack, token)?;
    let api = client()?;
    let stage = game.path.join(format!(
        ".canna-stage-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    fs::create_dir(&stage)?;
    let result = (|| -> Result<()> {
        for (index, item) in pack.mods.iter().enumerate() {
            if !item.enabled {
                continue;
            }
            progress(&format!(
                "Preparing mod {}/{}: {}",
                index + 1,
                pack.mods.len(),
                item.name
            ));
            let bytes = if !item.local_file.is_empty() {
                fs::read(crate::modpacks::local_directory().join(&item.local_file))
                    .context("Local mod is missing; import the .canna.zip again")?
            } else {
                repository::fetch_optional(
                    &api,
                    &settings(pack),
                    token,
                    &repo_path(pack, &item.file),
                    32 * 1024 * 1024,
                )?
                .context("Mod file not found in repository")?
            };
            if !item.sha256.is_empty()
                && format!("{:x}", Sha256::digest(&bytes)) != item.sha256.to_lowercase()
            {
                bail!("Checksum mismatch for {}", item.name)
            }
            let target = stage.join(index.to_string());
            if item.file.to_lowercase().ends_with(".dll") {
                fs::create_dir_all(&target)?;
                fs::write(
                    target.join(Path::new(&item.file).file_name().unwrap()),
                    bytes,
                )?;
            } else if item.file.to_lowercase().ends_with(".zip") {
                let mut plugins = Vec::new();
                for (path, data) in archive_files(&bytes)? {
                    let path = if let Ok(p) = path.strip_prefix("BepInEx/plugins") {
                        p.to_owned()
                    } else if let Ok(p) = path.strip_prefix("plugins") {
                        p.to_owned()
                    } else {
                        path
                    };
                    if path.starts_with("BepInEx")
                        || ["manifest.json", "README.md", "icon.png", "LICENSE"]
                            .iter()
                            .any(|p| path == Path::new(p))
                    {
                        continue;
                    }
                    plugins.push((path, data));
                }
                if !plugins
                    .iter()
                    .any(|(p, _)| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")))
                {
                    bail!("{} contains no plugin DLLs", item.name)
                }
                write_new(&target, &plugins)?;
            } else {
                bail!("{} must be a plugin DLL or ZIP", item.name)
            }
        }
        ensure_closed(game)?;
        progress("Activating selected pack…");
        let active = game.path.join("BepInEx/plugins/Canna");
        no_links(&active)?;
        let previous = game.path.join("BepInEx/plugins/Canna.previous");
        if previous.exists() {
            bail!("A previous Canna activation needs recovery")
        }
        if active.exists() {
            fs::rename(&active, &previous)?;
        }
        if let Err(error) = fs::rename(&stage, &active) {
            if previous.exists() {
                let _ = fs::rename(&previous, &active);
            }
            return Err(error.into());
        }
        if previous.exists() {
            remove_managed(&game.path, &previous)?;
        }
        Ok(())
    })();
    if stage.exists() {
        let _ = remove_managed(&game.path, &stage);
    }
    result
}
pub fn set_mode(root: &Path, modded: bool) -> Result<()> {
    let path = root.join("doorstop_config.ini");
    no_links(&path)?;
    if !path.exists() {
        if modded {
            bail!("Set up BepInEx first")
        } else {
            return Ok(());
        }
    }
    let original = fs::read_to_string(&path)?;
    let mut general = false;
    let mut changed = false;
    let updated = original
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                general = trimmed.eq_ignore_ascii_case("[General]")
                    || trimmed.eq_ignore_ascii_case("[UnityDoorstop]");
            }
            if general
                && trimmed
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("enabled"))
            {
                changed = true;
                format!("enabled={modded}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    if !changed {
        bail!("Doorstop configuration has no General.enabled setting")
    }
    let backup = root.join("doorstop_config.canna-original.ini");
    if !backup.exists() {
        fs::write(backup, original)?;
    }
    fs::write(path, updated)?;
    Ok(())
}
pub fn launch(game: &InstalledGame, modded: bool) -> Result<()> {
    ensure_closed(game)?;
    set_mode(&game.path, modded)?;
    Command::new("explorer.exe")
        .arg(format!("steam://rungameid/{}", game.app_id))
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_zip_traversal() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("../outside.dll", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"bad").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(archive_files(&bytes).is_err());
    }
    #[test]
    #[ignore = "Authenticated repository downloads and installation in a temporary fixture; never launches a game"]
    fn family_framework_and_mod_install() {
        let root = std::env::temp_dir().join(format!("canna-install-{}", std::process::id()));
        fs::create_dir_all(root.join("BoplBattle_Data/Managed")).unwrap();
        fs::write(
            root.join("BoplBattle_Data/Managed/Assembly-CSharp.dll"),
            b"fixture",
        )
        .unwrap();
        let settings = Settings::load();
        let token = crate::EMBEDDED_GITHUB_TOKEN;
        let data = repository::sync(&settings, token).unwrap();
        let info = data.games.iter().find(|g| g.app_id == 1686940).unwrap();
        let pack = Modpack::create(
            "Drill integration fixture".into(),
            String::new(),
            info,
            crate::cache::Source::from_settings(&settings),
            info.mods
                .iter()
                .filter(|m| {
                    matches!(
                        m.name.as_str(),
                        "Drill Through Ball" | "Canna Procedural Maps"
                    )
                })
                .cloned()
                .collect(),
        );
        assert_eq!(pack.mods.len(), 2);
        let game = InstalledGame {
            app_id: 1686940,
            name: "Bopl Battle fixture".into(),
            path: root.clone(),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        install_pack(&game, &pack, token, &|_| {}).unwrap();
        assert!(root.join("BepInEx/core/0Harmony.dll").is_file());
        assert!(
            root.join("BepInEx/plugins/Canna/0/CannaDrillThroughBall/Canna.DrillThroughBall.dll")
                .is_file()
        );
        assert!(
            root.join("BepInEx/plugins/Canna/1/CannaProceduralMaps/Canna.ProceduralMaps.dll")
                .is_file()
        );
        fs::write(
            root.join("BepInEx/plugins/UnmanagedFixture.dll"),
            b"unmanaged",
        )
        .unwrap();
        set_mode(&root, false).unwrap();
        assert!(
            fs::read_to_string(root.join("doorstop_config.ini"))
                .unwrap()
                .contains("enabled=false")
        );
        set_mode(&root, true).unwrap();
        assert!(
            fs::read_to_string(root.join("doorstop_config.ini"))
                .unwrap()
                .contains("enabled=true")
        );
        install_pack(&game, &pack, token, &|_| {}).unwrap();
        assert!(!root.join("BepInEx/plugins/Canna.previous").exists());
        let mut disabled = pack;
        for (index, item) in disabled.mods.iter_mut().enumerate() {
            item.enabled = false;
            item.file = format!("Mods/nonexistent-disabled-file-{index}.zip");
        }
        install_pack(&game, &disabled, token, &|_| {}).unwrap();
        assert_eq!(
            fs::read_dir(root.join("BepInEx/plugins/Canna"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fs::read(root.join("BepInEx/plugins/UnmanagedFixture.dll")).unwrap(),
            b"unmanaged"
        );
        assert_eq!(
            fs::read(root.join("BoplBattle_Data/Managed/Assembly-CSharp.dll")).unwrap(),
            b"fixture"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn mode_preserves_settings_and_switches() {
        let root = std::env::temp_dir().join(format!("canna-mode-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("doorstop_config.ini"), "[General]\nenabled=true\ntargetAssembly=BepInEx/core/BepInEx.Preloader.dll\n[Other]\nenabled=true\n").unwrap();
        set_mode(&root, false).unwrap();
        let data = fs::read_to_string(root.join("doorstop_config.ini")).unwrap();
        assert!(data.contains("enabled=false"));
        assert!(data.contains("[Other]\r\nenabled=true"));
        set_mode(&root, true).unwrap();
        assert!(
            !fs::read_to_string(root.join("doorstop_config.ini"))
                .unwrap()
                .contains("enabled=false")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn overwrite_is_rejected_before_any_writes() {
        let root = std::env::temp_dir().join(format!("canna-write-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("existing"), b"original").unwrap();
        assert!(
            write_new(
                &root,
                &[("new".into(), vec![1]), ("existing".into(), vec![2])]
            )
            .is_err()
        );
        assert!(!root.join("new").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
