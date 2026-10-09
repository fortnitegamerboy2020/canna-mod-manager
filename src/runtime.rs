use crate::{
    model::{InstalledGame, Settings},
    modpacks::Modpack,
    repository,
};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const REBOUND_PROFILE: &str = "rounds-public-1.1.2";
const REBOUND_MANIFEST: &str = "DuctTapePlusPlus/compatibility-manifest.json";

pub(crate) fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("Canna-Mod-Manager/0.1")
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
pub(crate) fn settings(pack: &Modpack) -> Settings {
    Settings {
        owner: pack.repository.owner.clone(),
        repository: pack.repository.repository.clone(),
        branch: pack.repository.branch.clone(),
        catalog_folder: pack.repository.catalog_folder.clone(),
        steam_path: String::new(),
        low_end: false,
        rebound_enabled: false,
        anonymous_reports: false,
    }
}
pub(crate) fn repo_path(pack: &Modpack, file: &str) -> String {
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
pub(crate) fn no_links(path: &Path) -> Result<()> {
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
        if game.app_id == u32::MAX {
            let output=Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command","$r=$env:CANNA_GAME_ROOT; $p=@(Get-CimInstance Win32_Process -Filter \"name='java.exe' OR name='javaw.exe'\" | Where-Object { $_.CommandLine -and $_.CommandLine.IndexOf($r,[StringComparison]::OrdinalIgnoreCase) -ge 0 }); if ($p.Count) { exit 2 }"]).env("CANNA_GAME_ROOT",game.path.to_string_lossy().replace('/',"\\")).creation_flags(0x08000000).output()?;
            return match output.status.code() {
                Some(0) => Ok(false),
                Some(2) => Ok(true),
                _ => bail!("Could not determine Minecraft process status"),
            };
        }
        let output = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
            "$r=$env:CANNA_GAME_ROOT; $p=@(Get-Process | Where-Object { $_.Path -and [System.IO.Path]::GetDirectoryName($_.Path) -eq $r }); if ($p.Count) { exit 2 }"])
            .env("CANNA_GAME_ROOT", game.path.to_string_lossy().replace('/' , "\\")).creation_flags(0x08000000).output()?;
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
fn pe_machine(data: &[u8]) -> Result<u16> {
    anyhow::ensure!(data.get(..2) == Some(b"MZ"), "Not a Windows executable");
    let offset =
        u32::from_le_bytes(data.get(0x3c..0x40).context("Truncated PE")?.try_into()?) as usize;
    anyhow::ensure!(
        data.get(offset..offset + 4) == Some(b"PE\0\0"),
        "Invalid PE header"
    );
    Ok(u16::from_le_bytes(
        data.get(offset + 4..offset + 6)
            .context("Truncated PE machine")?
            .try_into()?,
    ))
}
fn framework_entries(bytes: &[u8], il2cpp: bool, app_id: u32) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let raw = archive_files(bytes)?;
    let prefix = raw
        .iter()
        .find_map(|(p, _)| {
            let text = p.to_string_lossy();
            text.find("BepInEx/core/").map(|i| text[..i].to_owned())
        })
        .context("No BepInEx core in loader archive")?;
    let mut entries = Vec::new();
    for (path, data) in raw {
        let text = path.to_string_lossy();
        let Some(relative) = text.strip_prefix(&prefix) else {
            continue;
        };
        let path = PathBuf::from(relative);
        if [
            "README.md",
            "manifest.json",
            "icon.png",
            "LICENSE",
            "LICENSE.txt",
        ]
        .iter()
        .any(|v| path == Path::new(v))
        {
            continue;
        }
        anyhow::ensure!(
            path.starts_with("BepInEx")
                || (app_id == 1557740 && path == Path::new("corlibs/mscorlib.dll"))
                || [
                    "winhttp.dll",
                    "version.dll",
                    "doorstop_config.ini",
                    ".doorstop_version",
                    "changelog.txt"
                ]
                .iter()
                .any(|v| path == Path::new(v)),
            "Unsupported loader archive file: {}",
            path.display()
        );
        entries.push((path, data));
    }
    let core = if il2cpp {
        "BepInEx/core/BepInEx.Unity.IL2CPP.dll"
    } else {
        "BepInEx/core/BepInEx.dll"
    };
    anyhow::ensure!(
        entries.iter().any(|(p, _)| p == Path::new(core)),
        "Loader does not match this game's Mono/IL2CPP runtime"
    );
    anyhow::ensure!(
        entries
            .iter()
            .any(|(p, _)| p == Path::new("doorstop_config.ini")),
        "Missing Doorstop configuration"
    );
    anyhow::ensure!(
        entries
            .iter()
            .any(|(p, _)| p == Path::new("winhttp.dll") || p == Path::new("version.dll")),
        "Missing Windows loader proxy"
    );
    Ok(entries)
}
#[cfg(test)]
pub fn setup(game: &InstalledGame, pack: &Modpack, token: &str) -> Result<()> {
    setup_with_framework(game, pack, token, None)
}
#[cfg(test)]
pub fn setup_with_options(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    options: InstallOptions,
    progress: &dyn Fn(&str),
) -> Result<()> {
    if !options.translate(game, pack)? {
        return setup(game, pack, token);
    }
    let prepared = prepare_install(game, pack, token, options, progress)?;
    prepared.verify_inputs(game)?;
    prepared.verify_authorization(token)?;
    setup_with_framework(game, &prepared.pack, token, prepared.framework.as_deref())
}
fn setup_with_framework(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    prepared: Option<&[u8]>,
) -> Result<()> {
    crate::game_compat::check_pack(game, pack)?;
    if crate::model::source_addons(game.app_id).is_some() {
        return crate::source_addons::setup(game);
    }
    ensure_closed(game)?;
    crate::unity_restore::resume(game, false)?;
    let il2cpp = game.path.join("GameAssembly.dll").is_file();
    let core = if il2cpp {
        "BepInEx/core/BepInEx.Unity.IL2CPP.dll"
    } else {
        "BepInEx/core/BepInEx.dll"
    };
    if game.path.join(core).is_file()
        && game.path.join("doorstop_config.ini").is_file()
        && (game.path.join("winhttp.dll").is_file() || game.path.join("version.dll").is_file())
    {
        return Ok(());
    }
    if game.path.join("BepInEx/core/BepInEx.Core.dll").is_file()
        || game.path.join("BepInEx/core/BepInEx.dll").is_file()
    {
        bail!(
            "Existing BepInEx runtime does not match this game's Mono/IL2CPP profile; preserve it and select a compatible loader"
        );
    }
    if let Some(profile) = crate::game_profiles::by_id(game.app_id) {
        anyhow::ensure!(
            game.path.join(&profile.data_folder).is_dir(),
            "This game's Unity data directory was not found"
        );
    }
    let api = client()?;
    let bytes = if let Some(bytes) = prepared {
        bytes.to_vec()
    } else {
        repository::fetch_optional(
            &api,
            &settings(pack),
            token,
            &repo_path(pack, "Framework/BepInEx.zip"),
            128 * 1024 * 1024,
        )?
        .ok_or_else(|| {
            anyhow::anyhow!("The Canna server does not have a compatible framework for this game")
        })?
    };
    let mut entries = framework_entries(&bytes, il2cpp, game.app_id)?;
    // A new bootstrap must be inert for direct Steam launches until Launch modded.
    // Existing manual loader settings are handled by the early return above.
    for (path, data) in &mut entries {
        if path == Path::new("doorstop_config.ini") {
            anyhow::ensure!(
                data.len() <= 256 * 1024,
                "Doorstop configuration exceeds its safety limit"
            );
            *data = doorstop_text(std::str::from_utf8(data)?, false)?.into_bytes();
        }
    }
    if let Some(profile) = crate::game_profiles::by_id(game.app_id) {
        let exe = profile
            .executables
            .iter()
            .map(|name| game.path.join(name))
            .find(|p| p.is_file())
            .context("The game's Windows executable was not found")?;
        let machine = pe_machine(&fs::read(exe)?)?;
        for (path, data) in &entries {
            if path == Path::new("winhttp.dll") || path == Path::new("version.dll") {
                anyhow::ensure!(
                    pe_machine(data)? == machine,
                    "Loader architecture does not match the game executable"
                );
            }
        }
    }
    // Never overwrite another loader or partial installation.
    ensure_closed(game)?;
    let framework = crate::model::ModInfo {
        provenance: serde_json::Value::Null,
        content_type: String::new(),
        enabled: true,
        name: "BepInEx".into(),
        version: "framework".into(),
        description: String::new(),
        file: "Framework/BepInEx.zip".into(),
        sha256: String::new(),
        local_file: String::new(),
        dependencies: Vec::new(),
    };
    let _ = crate::website::remember_mod(pack, &framework, &bytes, true);
    crate::unity_restore::preflight_framework(game)?;
    write_new(&game.path, &entries)?;
    crate::unity_restore::record_framework(game, &entries)?;
    fs::create_dir_all(game.path.join("BepInEx/plugins"))?;
    fs::create_dir_all(game.path.join("BepInEx/config"))?;
    Ok(())
}
#[derive(Default)]
pub(crate) struct PluginEntries {
    pub plugins: Vec<(PathBuf, Vec<u8>)>,
    pub patchers: Vec<(PathBuf, Vec<u8>)>,
    pub configs: Vec<(PathBuf, Vec<u8>)>,
}
fn plugin_entries(bytes: &[u8]) -> Result<PluginEntries> {
    let mut entries = PluginEntries::default();
    for (path, data) in archive_files(bytes)? {
        if [
            "manifest.json",
            "README.md",
            "icon.png",
            "LICENSE",
            "LICENSE.txt",
        ]
        .iter()
        .any(|p| path == Path::new(p))
        {
            continue;
        }
        if path.starts_with("BepInEx/core")
            || path.starts_with("BepInEx/monomod")
            || path.starts_with("monomod")
            || path.to_string_lossy().ends_with(".mm.dll")
        {
            bail!(
                "This package modifies the loader or game assemblies; a dedicated installer is required"
            );
        }
        if let Ok(relative) = path
            .strip_prefix("BepInEx/config")
            .or_else(|_| path.strip_prefix("config"))
        {
            entries.configs.push((relative.to_owned(), data));
        } else if let Ok(relative) = path
            .strip_prefix("BepInEx/patchers")
            .or_else(|_| path.strip_prefix("patchers"))
        {
            entries.patchers.push((relative.to_owned(), data));
        } else {
            let path = path
                .strip_prefix("BepInEx/plugins")
                .or_else(|_| path.strip_prefix("plugins"))
                .map(Path::to_owned)
                .unwrap_or(path);
            anyhow::ensure!(
                !path.starts_with("BepInEx"),
                "Unsupported BepInEx package route"
            );
            entries.plugins.push((path, data));
        }
    }
    Ok(entries)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct InstallOptions {
    pub rebound_enabled: bool,
}
impl InstallOptions {
    pub(crate) fn translate(self, game: &InstalledGame, pack: &Modpack) -> Result<bool> {
        let translate = self.rebound_enabled
            && game.app_id == 1557740
            && crate::steam::installed_version(game).is_some_and(|v| {
                v.branch
                    .eq_ignore_ascii_case(crate::game_compat::PUBLIC_BRANCH)
            });
        // This marker can require preflight; it never authorizes an override.
        if game.app_id == 1557740
            && pack.mods.iter().any(|m| {
                m.enabled
                    && m.provenance["compatibility_profile"].as_str() == Some("rounds-public-1.1.2")
            })
            && !translate
        {
            bail!(
                "This setup was prepared with Canna Bliss. Enable Canna Bliss for ROUNDS (preview) in Settings and select the supported public ROUNDS version before applying or restoring it."
            )
        }
        Ok(translate)
    }
}
pub struct PreparedInstall {
    pack: Modpack,
    files: Option<PluginEntries>,
    source_files: Option<Vec<(String, Vec<u8>)>>,
    history: Vec<(crate::model::ModInfo, Vec<u8>)>,
    framework: Option<Vec<u8>>,
    game_sha256: Option<String>,
    config_sha256: Option<Vec<(PathBuf, String)>>,
    rebound_support_sha256: Option<String>,
    imported_configs: Vec<crate::pack_configs::Config>,
    imported_config_source: Option<Vec<(PathBuf, String)>>,
}
impl PreparedInstall {
    pub fn effective_pack(&self) -> &Modpack {
        &self.pack
    }
    pub fn cache_downloads(&self, progress: &dyn Fn(&str)) -> Result<()> {
        for (item, bytes) in &self.history {
            crate::website::remember_mod(&self.pack, item, bytes, false)?;
        }
        progress("Pack prepared in Canna's download cache. Files activate only on Launch modded.");
        Ok(())
    }
    pub(crate) fn verify_game(&self, game: &InstalledGame) -> Result<()> {
        if let Some(expected) = &self.game_sha256 {
            anyhow::ensure!(
                crate::ducttape::game_hash(game)? == *expected,
                "ROUNDS changed after compatibility preflight; retry after Steam finishes updating"
            );
        }
        Ok(())
    }
    fn verify_authorization(&self, token: &str) -> Result<()> {
        if let Some(expected) = &self.rebound_support_sha256 {
            crate::rebound_support::verify_current(token, expected)?;
        }
        Ok(())
    }
    pub(crate) fn verify_inputs(&self, game: &InstalledGame) -> Result<()> {
        self.verify_game(game)?;
        if let Some(expected) = &self.config_sha256 {
            anyhow::ensure!(
                crate::ducttape::configuration_hashes(game)? == *expected,
                "ROUNDS config changed after compatibility preflight; retry"
            );
        }
        Ok(())
    }
}
pub fn prepare_install(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    options: InstallOptions,
    progress: &dyn Fn(&str),
) -> Result<PreparedInstall> {
    prepare_install_with_configs(game, pack, token, options, None, progress)
}
pub(crate) fn prepare_install_with_configs(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    options: InstallOptions,
    configs: Option<&[(PathBuf, Vec<u8>)]>,
    progress: &dyn Fn(&str),
) -> Result<PreparedInstall> {
    pack.validate()?;
    if pack.game.app_id != game.app_id {
        bail!("Modpack belongs to a different game");
    }
    if game.app_id == u32::MAX || crate::model::source_addons(game.app_id).is_some() {
        return Ok(PreparedInstall {
            pack: pack.clone(),
            files: None,
            source_files: if crate::model::source_addons(game.app_id).is_some() {
                Some(crate::source_addons::prepare_files(
                    game, pack, token, progress,
                )?)
            } else {
                None
            },
            history: vec![],
            framework: None,
            game_sha256: None,
            config_sha256: None,
            rebound_support_sha256: None,
            imported_configs: vec![],
            imported_config_source: None,
        });
    }
    let translate = options.translate(game, pack)?;
    if !translate {
        crate::game_compat::check_pack(game, pack)?;
    }
    ensure_closed(game)?;
    // Preparation never restores loader/plugin files into the game. Missing
    // compatibility references are staged in the external preview workspace.
    let support = if translate {
        progress("Verifying Canna Bliss Beta access…");
        Some(crate::rebound_support::authorized_bundle(token)?)
    } else {
        None
    };
    let api = client()?;
    let mut prepared = PreparedInstall {
        pack: pack.clone(),
        files: Some(PluginEntries::default()),
        source_files: None,
        history: vec![],
        framework: None,
        game_sha256: None,
        config_sha256: None,
        rebound_support_sha256: support
            .as_ref()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes))),
        imported_configs: if configs.is_none() {
            crate::pack_configs::pending(game, pack)?
        } else {
            vec![]
        },
        imported_config_source: None,
    };
    let imported_effective = if !prepared.imported_configs.is_empty() {
        let current = crate::ducttape::current_configs(game)?;
        prepared.imported_config_source =
            Some(crate::ducttape::configuration_fingerprint(&current)?);
        Some(crate::pack_configs::merge(
            current,
            &prepared.imported_configs,
        ))
    } else {
        None
    };
    {
        let files = prepared.files.as_mut().unwrap();
        let mut archive_bytes = 0usize;
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
                    128 * 1024 * 1024,
                )?
                .context("Mod file not found in repository")?
            };
            if translate {
                archive_bytes = archive_bytes
                    .checked_add(bytes.len())
                    .context("Compatibility archive size overflow")?;
                anyhow::ensure!(
                    archive_bytes <= 256 * 1024 * 1024,
                    "Compatibility source archives exceed preview limits"
                );
            }
            if !item.sha256.is_empty()
                && format!("{:x}", Sha256::digest(&bytes)) != item.sha256.to_lowercase()
            {
                bail!("Checksum mismatch for {}", item.name)
            }
            let target = PathBuf::from(index.to_string());
            if item.file.to_lowercase().ends_with(".dll") {
                files.plugins.push((
                    target.join(Path::new(&item.file).file_name().unwrap()),
                    bytes.clone(),
                ));
            } else if item.file.to_lowercase().ends_with(".zip") {
                let entries = plugin_entries(&bytes)?;
                let has_binary = entries
                    .plugins
                    .iter()
                    .chain(entries.patchers.iter())
                    .any(|(p, _)| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")));
                anyhow::ensure!(
                    has_binary
                        || !entries.configs.is_empty()
                        || (!item.dependencies.is_empty() && entries.plugins.is_empty()),
                    "{} contains no supported plugins, patchers or configuration",
                    item.name
                );
                files.plugins.extend(
                    entries
                        .plugins
                        .into_iter()
                        .map(|(path, bytes)| (target.join(path), bytes)),
                );
                files.patchers.extend(
                    entries
                        .patchers
                        .into_iter()
                        .map(|(path, bytes)| (target.join(path), bytes)),
                );
                files.configs.extend(entries.configs);
            } else {
                bail!("{} must be a plugin DLL or ZIP", item.name)
            }
            prepared.history.push((item.clone(), bytes));
        }
    }
    if translate {
        if !game.path.join("BepInEx/core/BepInEx.dll").is_file() {
            progress("Preparing reviewed BepInEx references outside the game…");
            let bytes = repository::fetch_optional(
                &api,
                &settings(pack),
                token,
                &repo_path(pack, "Framework/BepInEx.zip"),
                128 * 1024 * 1024,
            )?
            .context("A reviewed ROUNDS loader is required for compatibility preflight")?;
            framework_entries(&bytes, false, game.app_id)?;
            prepared.framework = Some(bytes);
        }
        let mut pinned_sources = pack.clone();
        for (item, bytes) in &prepared.history {
            if let Some(source) = pinned_sources
                .mods
                .iter_mut()
                .find(|source| source.file == item.file)
            {
                source.sha256 = format!("{:x}", Sha256::digest(bytes));
            }
        }
        let resolved = crate::ducttape::resolve(
            game,
            &pinned_sources,
            prepared.files.take().unwrap(),
            support.as_deref().unwrap(),
            prepared.framework.as_deref(),
            configs.or(imported_effective.as_deref()),
            progress,
        )?;
        prepared.pack = resolved.pack;
        prepared.files = Some(resolved.files);
        prepared.game_sha256 = Some(resolved.game_sha256);
        prepared.config_sha256 = Some(resolved.config_sha256);
        bind_rebound_launch(&mut prepared)?;
    }
    Ok(prepared)
}

fn rebound_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn bind_rebound_launch(prepared: &mut PreparedInstall) -> Result<()> {
    let support = prepared
        .rebound_support_sha256
        .as_ref()
        .context("Missing authorized Bliss support checksum")?;
    let game = prepared
        .game_sha256
        .as_ref()
        .context("Missing Bliss game checksum")?;
    let manifests: Vec<_> = prepared
        .files
        .as_ref()
        .context("Missing prepared Bliss files")?
        .plugins
        .iter()
        .filter(|(path, _)| path == Path::new(REBOUND_MANIFEST))
        .collect();
    anyhow::ensure!(
        manifests.len() == 1,
        "Missing or ambiguous prepared Bliss manifest"
    );
    let marked: Vec<_> = prepared
        .pack
        .mods
        .iter_mut()
        .filter(|item| {
            item.enabled
                && item.provenance["compatibility_profile"].as_str() == Some(REBOUND_PROFILE)
        })
        .collect();
    anyhow::ensure!(
        marked.len() == 1 && rebound_hash(support) && rebound_hash(game),
        "Invalid prepared Bliss launch binding"
    );
    let item = marked.into_iter().next().unwrap();
    item.provenance["rebound_support_sha256"] = support.clone().into();
    item.provenance["rebound_game_sha256"] = game.clone().into();
    item.provenance["rebound_manifest_sha256"] =
        format!("{:x}", Sha256::digest(&manifests[0].1)).into();
    Ok(())
}

fn current_launch_rebound_expectation(
    game: &InstalledGame,
    modded: bool,
    options: InstallOptions,
    applied: &dyn Fn() -> Result<Option<Modpack>>,
) -> Result<Option<String>> {
    if !modded || game.app_id != 1557740 {
        return Ok(None);
    }
    let managed = game.path.join("BepInEx/plugins/Canna");
    no_links(&managed)?;
    match fs::symlink_metadata(&managed) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Could not inspect the installed Bliss setup"),
        Ok(metadata) => anyhow::ensure!(
            metadata.is_dir(),
            "Invalid installed managed setup; reapply the pack"
        ),
    }
    let active = managed.join("DuctTapePlusPlus");
    no_links(&active)?;
    let has_manifest_folder = match fs::symlink_metadata(&active) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).context("Could not inspect the installed Bliss setup"),
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_dir(),
                "Invalid installed Bliss setup; reapply the pack"
            );
            true
        }
    };
    let pack = match applied() {
        Ok(pack) => pack,
        // History alone must not add a launch restriction to an ordinary setup.
        // Installed Bliss evidence still requires readable, bound metadata.
        Err(_) if !has_manifest_folder => return Ok(None),
        Err(error) => {
            return Err(error).context("Installed Bliss metadata is unreadable; reapply the pack");
        }
    };
    let has_pack_marker = pack.as_ref().is_some_and(|pack| {
        pack.mods.iter().any(|item| {
            item.enabled
                && item.provenance["compatibility_profile"].as_str() == Some(REBOUND_PROFILE)
        })
    });
    if !has_manifest_folder && !has_pack_marker {
        return Ok(None);
    }
    let pack = pack.context(
        "This installed Bliss setup needs a fresh binding; reapply the pack before launching",
    )?;
    pack.validate()?;
    anyhow::ensure!(
        pack.game.app_id == game.app_id,
        "Bliss setup metadata belongs to another game; reapply the pack"
    );
    let marked: Vec<_> = pack
        .mods
        .iter()
        .filter(|item| {
            item.enabled
                && item.provenance["compatibility_profile"].as_str() == Some(REBOUND_PROFILE)
        })
        .collect();
    anyhow::ensure!(
        marked.len() == 1,
        "This installed Bliss setup needs a fresh binding; reapply the pack before launching"
    );
    anyhow::ensure!(
        options.translate(game, &pack)?,
        "Enable Canna Bliss and select the supported public ROUNDS branch before launching this setup"
    );
    let binding = &marked[0].provenance;
    let pinned = |key: &str| -> Result<&str> {
        binding[key]
            .as_str()
            .filter(|hash| rebound_hash(hash))
            .context("This installed Bliss setup needs a fresh binding; reapply the pack before launching")
    };
    let support = pinned("rebound_support_sha256")?;
    let game_hash = pinned("rebound_game_sha256")?;
    let manifest_hash = pinned("rebound_manifest_sha256")?;
    let manifest_path = game
        .path
        .join("BepInEx/plugins/Canna")
        .join(REBOUND_MANIFEST);
    no_links(&manifest_path)?;
    let metadata = fs::metadata(&manifest_path)
        .context("Installed Bliss manifest is missing; reapply the pack")?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= 4 * 1024 * 1024,
        "Invalid installed Bliss manifest; reapply the pack"
    );
    let mut bytes = Vec::new();
    fs::File::open(&manifest_path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 4 * 1024 * 1024 && format!("{:x}", Sha256::digest(&bytes)) == manifest_hash,
        "Installed Bliss manifest changed; reapply the pack before launching"
    );
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .context("Invalid installed Bliss manifest; reapply the pack")?;
    anyhow::ensure!(
        manifest["profile"].as_str() == Some(REBOUND_PROFILE)
            && manifest["protocol"].as_str() == Some("canna.ducttape++/1")
            && manifest["game_sha256"].as_str() == Some(game_hash),
        "Installed Bliss manifest does not match its launch binding; reapply the pack"
    );
    anyhow::ensure!(
        crate::ducttape::game_hash(game)? == game_hash,
        "ROUNDS changed since this Bliss setup was prepared; reapply the pack before launching"
    );
    Ok(Some(support.into()))
}

fn launch_current_with<T>(
    game: &InstalledGame,
    modded: bool,
    options: InstallOptions,
    applied: &dyn Fn() -> Result<Option<Modpack>>,
    verify: &dyn Fn(&str) -> Result<()>,
    launch: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if let Some(expected) = current_launch_rebound_expectation(game, modded, options, applied)? {
        verify(&expected)?;
        anyhow::ensure!(
            current_launch_rebound_expectation(game, modded, options, applied)? == Some(expected),
            "Bliss setup changed while checking access; retry or reapply the pack"
        );
    }
    launch()
}

pub fn launch_current(
    game: &InstalledGame,
    token: &str,
    options: InstallOptions,
    progress: &dyn Fn(&str),
) -> Result<crate::owned_game::OwnedGame> {
    launch_current_with(
        game,
        true,
        options,
        &|| crate::play_backup::last_applied(game),
        &|expected| {
            progress("Verifying current Canna Bliss Beta access…");
            crate::rebound_support::verify_current_read_only(token, expected)
        },
        || launch(game, true),
    )
}
pub fn install_prepared(
    game: &InstalledGame,
    prepared: PreparedInstall,
    token: &str,
    progress: &dyn Fn(&str),
) -> Result<()> {
    if let Some(expected) = &prepared.imported_config_source {
        anyhow::ensure!(
            crate::ducttape::configuration_hashes(game)? == *expected,
            "Config changed since imported settings were prepared; retry"
        );
    }
    let configs = prepared.imported_configs.clone();
    crate::pack_configs::activate(game, &configs, || {
        install_prepared_inner(game, prepared, token, progress)
    })
}
fn install_prepared_inner(
    game: &InstalledGame,
    prepared: PreparedInstall,
    token: &str,
    progress: &dyn Fn(&str),
) -> Result<()> {
    let pack = &prepared.pack;
    if game.app_id == u32::MAX {
        return crate::minecraft::restore_play_pack(game, pack);
    }
    if crate::model::source_addons(game.app_id).is_some() {
        return crate::source_addons::install_files(
            game,
            prepared
                .source_files
                .context("Source preparation is missing")?,
            progress,
        );
    }
    pack.validate()?;
    ensure_closed(game)?;
    prepared.verify_inputs(game)?;
    prepared.verify_authorization(token)?;
    progress("Checking BepInEx…");
    setup_with_framework(game, pack, token, prepared.framework.as_deref())?;
    let stage = game.path.join(format!(
        ".canna-stage-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    no_links(&stage)?;
    fs::create_dir(&stage)?;
    fs::create_dir(stage.join("plugins"))?;
    fs::create_dir(stage.join("patchers"))?;
    let result = (|| -> Result<()> {
        let files = prepared
            .files
            .as_ref()
            .context("Missing prepared plugin files")?;
        write_new(&stage.join("plugins"), &files.plugins)?;
        write_new(&stage.join("patchers"), &files.patchers)?;
        ensure_closed(game)?;
        if let Some(expected) = &prepared.game_sha256 {
            anyhow::ensure!(
                crate::ducttape::game_hash(game)? == *expected,
                "ROUNDS changed before compatibility activation; retry"
            );
        }
        if let Some(expected) = &prepared.config_sha256 {
            anyhow::ensure!(
                crate::ducttape::configuration_hashes(game)? == *expected,
                "ROUNDS config changed before compatibility activation; retry"
            );
        }
        prepared.verify_authorization(token)?;
        progress("Activating selected pack…");
        activate_stage(&game.path, &stage, files.configs.clone())
    })();
    if stage.exists() {
        let _ = remove_managed(&game.path, &stage);
    }
    if result.is_ok() {
        for (item, bytes) in &prepared.history {
            if let Err(error) = crate::website::remember_mod(pack, item, bytes, false) {
                progress(&format!("Couldn't save download history: {error}"));
            }
        }
    }
    result
}
#[cfg(test)]
pub fn install_pack(
    game: &InstalledGame,
    pack: &Modpack,
    token: &str,
    progress: &dyn Fn(&str),
) -> Result<()> {
    install_prepared(
        game,
        prepare_install(game, pack, token, InstallOptions::default(), progress)?,
        token,
        progress,
    )
}
fn activate_stage(root: &Path, stage: &Path, configs: Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
    let routes = [
        ("plugins", root.join("BepInEx/plugins/Canna")),
        ("patchers", root.join("BepInEx/patchers/Canna")),
    ];
    for (_, active) in &routes {
        no_links(active)?;
        anyhow::ensure!(
            !active.with_file_name("Canna.previous").exists(),
            "A previous Canna activation needs recovery"
        );
        fs::create_dir_all(active.parent().unwrap())?;
    }
    let config_root = root.join("BepInEx/config");
    let mut new_configs = Vec::new();
    for (path, data) in configs {
        let destination = config_root.join(&path);
        no_links(&destination)?;
        if destination.exists() {
            anyhow::ensure!(
                destination.is_file(),
                "Config path is not a file: {}",
                path.display()
            );
        } else if let Some((_, existing)) = new_configs.iter().find(|(p, _)| p == &path) {
            anyhow::ensure!(
                existing == &data,
                "Conflicting mod config defaults: {}",
                path.display()
            );
        } else {
            new_configs.push((path, data));
        }
    }
    write_new(&config_root, &new_configs)?;
    let mut backups = Vec::new();
    let mut promoted = Vec::new();
    let activation = (|| -> Result<()> {
        for (_, active) in &routes {
            if active.exists() {
                fs::rename(active, active.with_file_name("Canna.previous"))?;
                backups.push(active.clone());
            }
        }
        for (route, active) in &routes {
            fs::rename(stage.join(route), active)?;
            promoted.push((*route, active.clone()));
        }
        Ok(())
    })();
    if let Err(error) = activation {
        for (route, active) in promoted.into_iter().rev() {
            fs::rename(active, stage.join(route))?;
        }
        for active in backups.into_iter().rev() {
            fs::rename(active.with_file_name("Canna.previous"), active)?;
        }
        for (path, _) in &new_configs {
            fs::remove_file(config_root.join(path))?;
        }
        return Err(error);
    }
    for active in backups {
        remove_managed(root, &active.with_file_name("Canna.previous"))?;
    }
    Ok(())
}
pub(crate) fn doorstop_text(original: &str, modded: bool) -> Result<String> {
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
    Ok(updated)
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
    anyhow::ensure!(
        fs::metadata(&path)?.len() <= 256 * 1024,
        "Doorstop configuration exceeds its safety limit"
    );
    let original = fs::read_to_string(&path)?;
    let updated = doorstop_text(&original, modded)?;
    let backup = root.join("doorstop_config.canna-original.ini");
    no_links(&backup)?;
    if !backup.exists() {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(backup)?
            .write_all(original.as_bytes())?;
    }
    crate::unity_restore::replace_file(&path, updated.as_bytes())?;
    Ok(())
}
pub fn restore_vanilla(game: &InstalledGame) -> Result<String> {
    if crate::model::source_addons(game.app_id).is_some() {
        ensure_closed(game)?;
        crate::source_addons::set_mode(game, false)?;
        return Ok("Vanilla files restored: Canna Source addons disabled.".into());
    }
    anyhow::ensure!(
        crate::model::framework(game.app_id) == "bepinex" && game.app_id != u32::MAX,
        "Restore vanilla files is available for supported Unity games"
    );
    anyhow::ensure!(
        crate::model::supported_catalog()
            .iter()
            .any(|entry| entry.app_id == game.app_id),
        "This game is not a supported Unity game"
    );
    crate::unity_restore::restore(game)
}
pub fn launch(game: &InstalledGame, modded: bool) -> Result<crate::owned_game::OwnedGame> {
    ensure_closed(game)?;
    if crate::model::source_addons(game.app_id).is_some() {
        if modded {
            crate::game_compat::check_current(game)?;
        }
        crate::source_addons::set_mode(game, modded)?;
    } else if modded {
        crate::unity_restore::resume(game, true)?;
        crate::game_compat::check_current(game)?;
        ensure_loader_ready(&game.path)?;
        set_mode(&game.path, true)?;
    } else {
        restore_vanilla(game)?;
    }
    let earliest = crate::owned_game::OwnedGame::now();
    if crate::model::source_addons(game.app_id).is_some() {
        crate::steam::launch_source(game.app_id, modded)?;
    } else {
        Command::new("explorer.exe")
            .arg(format!("steam://rungameid/{}", game.app_id))
            .spawn()?;
    }
    let waiting = std::time::Instant::now();
    while waiting.elapsed() < Duration::from_secs(45) {
        use std::os::windows::process::CommandExt;
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Get-Process | Where-Object { $_.Path -and $_.Name -notmatch 'crash|helper|uninstall|setup|launcher' -and [IO.Path]::GetDirectoryName($_.Path) -eq $env:CANNA_GAME_ROOT } | ForEach-Object { $_.Id }"])
            .env("CANNA_GAME_ROOT", game.path.to_string_lossy().replace('/' , "\\"))
            .creation_flags(0x08000000).output()?;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Ok(pid) = line.trim().parse()
                && let Ok(owned) = crate::owned_game::OwnedGame::capture(pid, &game.path, earliest)
            {
                return Ok(owned);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    bail!(
        "Steam launch requested, but no game process could be retained. Check Steam or the Console."
    )
}

fn ensure_loader_ready(root: &Path) -> Result<()> {
    let il2cpp = root.join("GameAssembly.dll").is_file();
    let core = root.join(if il2cpp {
        "BepInEx/core/BepInEx.Unity.IL2CPP.dll"
    } else {
        "BepInEx/core/BepInEx.dll"
    });
    no_links(&core)?;
    anyhow::ensure!(
        core.is_file(),
        "The loader bootstrap exists but the matching BepInEx core is missing. Open Setup health, then repair the framework with the game closed; manual files are preserved."
    );
    anyhow::ensure!(
        ["winhttp.dll", "version.dll"]
            .iter()
            .any(|p| root.join(p).is_file()),
        "The BepInEx bootstrap DLL is missing. Launch modded from a saved modpack to prepare its framework."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_and_download_cache_leave_parked_loader_and_game_files_inactive() {
        let root = temporary_preparation_root("launch-only");
        crate::modpacks::with_test_root(root.clone(), || {
            let game_root = root.join("game");
            fs::create_dir_all(&game_root).unwrap();
            let (game, mut pack) = preparation_fixture(&game_root);
            let input = root.join("fixture.dll");
            fs::write(&input, b"fixture bytes never executed").unwrap();
            pack.mods = vec![crate::modpacks::add_local(&input).unwrap()];
            fs::write(
                game_root.join("doorstop_config.ini"),
                b"[UnityDoorstop]\nenabled=false\n",
            )
            .unwrap();
            fs::create_dir_all(game_root.join("BepInEx/plugins/Canna")).unwrap();
            fs::write(
                game_root.join("BepInEx/plugins/Canna/old.dll"),
                b"old fixture",
            )
            .unwrap();
            crate::unity_restore::restore(&game).unwrap();
            let before = fs::read(game_root.join(".canna-runtime/state.json")).unwrap();
            let before_ini = fs::read(game_root.join("doorstop_config.ini")).unwrap();
            let prepared =
                prepare_install(&game, &pack, "", InstallOptions::default(), &|_| {}).unwrap();
            prepared.cache_downloads(&|_| {}).unwrap();
            assert_eq!(
                fs::read(game_root.join(".canna-runtime/state.json")).unwrap(),
                before
            );
            assert!(!game_root.join("BepInEx/plugins/Canna").exists());
            assert!(!game_root.join("winhttp.dll").exists());
            assert_eq!(
                fs::read(game_root.join("doorstop_config.ini")).unwrap(),
                before_ini
            );
            fs::remove_dir_all(crate::runtime_cache::root(&game)).unwrap();
        });
        fs::remove_dir_all(root).unwrap();
    }
    fn preparation_fixture(root: &Path) -> (InstalledGame, Modpack) {
        let game = InstalledGame {
            app_id: 1686940,
            name: "Temporary Bopl fixture".into(),
            path: root.to_owned(),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        let info = crate::model::bopl();
        let source = crate::cache::Source {
            owner: "fixture".into(),
            repository: "fixture".into(),
            branch: "main".into(),
            catalog_folder: String::new(),
        };
        (
            game,
            Modpack::create(
                "Fixture selections".into(),
                String::new(),
                &info,
                source,
                vec![],
            ),
        )
    }
    fn temporary_preparation_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "canna-preparation-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }
    fn bound_current_fixture(name: &str) -> (PathBuf, InstalledGame, PreparedInstall) {
        let root = temporary_preparation_root(name);
        let game_root = root.join("steamapps/common/ROUNDS");
        fs::create_dir_all(game_root.join("ROUNDS_Data/Managed")).unwrap();
        fs::create_dir_all(game_root.join("BepInEx/plugins/Canna/DuctTapePlusPlus")).unwrap();
        fs::write(
            root.join("steamapps/appmanifest_1557740.acf"),
            r#""AppState" { "buildid" "21020021" "UserConfig" { "BetaKey" "public" } }"#,
        )
        .unwrap();
        fs::write(
            game_root.join("ROUNDS_Data/Managed/Assembly-CSharp.dll"),
            b"harmless native fixture bytes",
        )
        .unwrap();
        fs::write(
            game_root.join("doorstop_config.ini"),
            b"[General]\nenabled=false\n",
        )
        .unwrap();
        let (mut game, mut pack) = preparation_fixture(&game_root);
        game.app_id = 1557740;
        pack.game.app_id = game.app_id;
        pack.game.name = "ROUNDS".into();
        pack.game.folder = "rounds".into();
        pack.mods = vec![serde_json::from_value(serde_json::json!({
            "name":"Resolved Bliss fixture", "version":REBOUND_PROFILE,
            "file":"Mods/fixture.zip", "sha256":"a".repeat(64),
            "provenance":{"compatibility_profile":REBOUND_PROFILE,"required_game_branch":"public"}
        })).unwrap()];
        let game_hash = crate::ducttape::game_hash(&game).unwrap();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "protocol":"canna.ducttape++/1", "profile":REBOUND_PROFILE,
            "game_sha256":game_hash, "assemblies":[], "files":[], "digest":"b".repeat(64)
        }))
        .unwrap();
        fs::write(
            game.path
                .join("BepInEx/plugins/Canna")
                .join(REBOUND_MANIFEST),
            &manifest,
        )
        .unwrap();
        let mut prepared = PreparedInstall {
            pack,
            files: Some(PluginEntries {
                plugins: vec![(REBOUND_MANIFEST.into(), manifest)],
                ..PluginEntries::default()
            }),
            source_files: None,
            history: vec![],
            framework: None,
            game_sha256: Some(game_hash),
            config_sha256: None,
            rebound_support_sha256: Some("c".repeat(64)),
            imported_configs: vec![],
            imported_config_source: None,
        };
        bind_rebound_launch(&mut prepared).unwrap();
        (root, game, prepared)
    }
    #[test]
    fn current_launch_rebound_binds_authorized_support_without_changing_payload_bytes() {
        let (root, game, mut prepared) = bound_current_fixture("launch-binding");
        let bytes = prepared.files.as_ref().unwrap().plugins.clone();
        bind_rebound_launch(&mut prepared).unwrap();
        assert_eq!(prepared.files.as_ref().unwrap().plugins, bytes);
        assert_eq!(
            prepared.pack.mods[0].provenance["rebound_support_sha256"],
            "c".repeat(64)
        );
        assert_eq!(
            prepared.pack.mods[0].provenance["rebound_game_sha256"],
            crate::ducttape::game_hash(&game).unwrap()
        );
        assert_eq!(
            prepared.pack.mods[0].provenance["rebound_manifest_sha256"],
            format!("{:x}", Sha256::digest(&bytes[0].1))
        );
        let saved = serde_json::to_vec(&prepared.pack).unwrap();
        let restored: Modpack = serde_json::from_slice(&saved).unwrap();
        assert_eq!(
            current_launch_rebound_expectation(
                &game,
                true,
                InstallOptions {
                    rebound_enabled: true
                },
                &|| Ok(Some(restored.clone()))
            )
            .unwrap(),
            Some("c".repeat(64))
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn current_launch_rebound_checks_access_before_dispatch_and_never_writes_game_files() {
        let (root, game, prepared) = bound_current_fixture("launch-fresh-auth");
        let marker = game
            .path
            .join("BepInEx/plugins/Canna")
            .join(REBOUND_MANIFEST);
        let before_manifest = fs::read(&marker).unwrap();
        let before_loader = fs::read(game.path.join("doorstop_config.ini")).unwrap();
        for allowed in [false, true, false] {
            let requests = std::cell::Cell::new(0);
            let launches = std::cell::Cell::new(0);
            let result = launch_current_with(
                &game,
                true,
                InstallOptions {
                    rebound_enabled: true,
                },
                &|| Ok(Some(prepared.pack.clone())),
                &|expected| {
                    assert_eq!(expected, "c".repeat(64));
                    requests.set(requests.get() + 1);
                    if allowed {
                        Ok(())
                    } else {
                        bail!("Beta access revoked in fixture")
                    }
                },
                || {
                    launches.set(launches.get() + 1);
                    Ok(())
                },
            );
            assert_eq!(requests.get(), 1);
            assert_eq!(result.is_ok(), allowed);
            assert_eq!(launches.get(), u32::from(allowed));
            assert_eq!(fs::read(&marker).unwrap(), before_manifest);
            assert_eq!(
                fs::read(game.path.join("doorstop_config.ini")).unwrap(),
                before_loader
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn current_launch_rebound_rechecks_installed_inputs_after_the_server_reply() {
        for change_game in [false, true] {
            let (root, game, prepared) = bound_current_fixture("launch-input-race");
            let launches = std::cell::Cell::new(0);
            let result = launch_current_with(
                &game,
                true,
                InstallOptions {
                    rebound_enabled: true,
                },
                &|| Ok(Some(prepared.pack.clone())),
                &|_| {
                    let changed = if change_game {
                        game.path.join("ROUNDS_Data/Managed/Assembly-CSharp.dll")
                    } else {
                        game.path
                            .join("BepInEx/plugins/Canna")
                            .join(REBOUND_MANIFEST)
                    };
                    fs::write(changed, b"changed while server was replying")?;
                    Ok(())
                },
                || {
                    launches.set(launches.get() + 1);
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert_eq!(launches.get(), 0);
            assert_eq!(
                fs::read(game.path.join("doorstop_config.ini")).unwrap(),
                b"[General]\nenabled=false\n"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn current_launch_rebound_manifest_game_and_metadata_changes_fail_before_http_or_dispatch() {
        for mutation in 0..8 {
            let (root, game, mut prepared) =
                bound_current_fixture(&format!("launch-invalid-{mutation}"));
            let marker = game
                .path
                .join("BepInEx/plugins/Canna")
                .join(REBOUND_MANIFEST);
            match mutation {
                0 => fs::write(&marker, b"altered manifest").unwrap(),
                1 => fs::remove_file(&marker).unwrap(),
                2 => fs::write(
                    game.path.join("ROUNDS_Data/Managed/Assembly-CSharp.dll"),
                    b"changed game",
                )
                .unwrap(),
                3 => {
                    prepared.pack.mods[0]
                        .provenance
                        .as_object_mut()
                        .unwrap()
                        .remove("rebound_support_sha256");
                }
                4 => {
                    prepared.pack.mods[0].provenance["rebound_manifest_sha256"] =
                        "not-a-hash".into()
                }
                5 => prepared.pack.mods.push(prepared.pack.mods[0].clone()),
                6 => prepared.pack.game.app_id = 1686940,
                _ => fs::remove_dir_all(game.path.join("BepInEx/plugins/Canna/DuctTapePlusPlus"))
                    .unwrap(),
            }
            let requests = std::cell::Cell::new(0);
            let launches = std::cell::Cell::new(0);
            let loader = fs::read(game.path.join("doorstop_config.ini")).unwrap();
            let result = launch_current_with(
                &game,
                true,
                InstallOptions {
                    rebound_enabled: true,
                },
                &|| Ok(Some(prepared.pack.clone())),
                &|_| {
                    requests.set(requests.get() + 1);
                    Ok(())
                },
                || {
                    launches.set(launches.get() + 1);
                    Ok(())
                },
            );
            assert!(result.is_err(), "mutation {mutation} accepted");
            assert_eq!(requests.get(), 0, "mutation {mutation} reached server");
            assert_eq!(launches.get(), 0, "mutation {mutation} dispatched process");
            assert_eq!(
                fs::read(game.path.join("doorstop_config.ini")).unwrap(),
                loader
            );
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn current_launch_rebound_missing_binding_disabled_opt_in_or_old_branch_requires_reapply() {
        for mutation in 0..4 {
            let (root, game, prepared) =
                bound_current_fixture(&format!("launch-policy-{mutation}"));
            let mut options = InstallOptions {
                rebound_enabled: true,
            };
            let mut pack = Some(prepared.pack);
            match mutation {
                0 => pack = None,
                1 => options.rebound_enabled = false,
                2 => fs::write(
                    root.join("steamapps/appmanifest_1557740.acf"),
                    r#""AppState" { "UserConfig" { "BetaKey" "old-rounds-for-mods" } }"#,
                )
                .unwrap(),
                _ => pack.as_mut().unwrap().mods[0].enabled = false,
            }
            let result: Result<()> = launch_current_with(
                &game,
                true,
                options,
                &|| Ok(pack.clone()),
                &|_| panic!("invalid setup reached server"),
                || panic!("invalid setup dispatched game"),
            );
            assert!(result.is_err());
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn current_launch_rebound_vanilla_other_games_and_inactive_trees_never_check_access() {
        let (root, game, prepared) = bound_current_fixture("launch-scope");
        let mut other = game.clone();
        other.app_id = 1686940;
        for (candidate, modded) in [(&game, false), (&other, true)] {
            let launched = launch_current_with(
                candidate,
                modded,
                InstallOptions::default(),
                &|| panic!("unrelated launch loaded Bliss metadata"),
                &|_| panic!("unrelated launch reached server"),
                || Ok("allowed"),
            )
            .unwrap();
            assert_eq!(launched, "allowed");
        }
        fs::rename(
            game.path.join("BepInEx/plugins/Canna"),
            root.join("fixture-parked"),
        )
        .unwrap();
        assert!(
            launch_current_with(
                &game,
                true,
                InstallOptions::default(),
                &|| panic!("inactive tree loaded metadata"),
                &|_| panic!("inactive tree reached server"),
                || Ok(())
            )
            .is_ok()
        );
        fs::create_dir(game.path.join("BepInEx/plugins/Canna")).unwrap();
        let mut ordinary = prepared.pack;
        ordinary.mods.clear();
        assert!(
            launch_current_with(
                &game,
                true,
                InstallOptions::default(),
                &|| Ok(Some(ordinary.clone())),
                &|_| panic!("ordinary ROUNDS reached server"),
                || Ok(())
            )
            .is_ok()
        );
        assert!(
            launch_current_with(
                &game,
                true,
                InstallOptions::default(),
                &|| Ok(None),
                &|_| panic!("manual ROUNDS reached server"),
                || Ok(())
            )
            .is_ok()
        );
        assert!(
            launch_current_with(
                &game,
                true,
                InstallOptions::default(),
                &|| bail!("unrelated corrupt history"),
                &|_| panic!("ordinary corrupt history reached server"),
                || Ok(())
            )
            .is_ok()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rebound_opt_in_does_not_change_ordinary_or_other_game_preparation() {
        let root = temporary_preparation_root("opt-in");
        crate::modpacks::with_test_root(root.clone(), || {
            let game_root = root.join("steamapps/common/ROUNDS");
            fs::create_dir_all(game_root.join("BepInEx/core")).unwrap();
            fs::write(
                game_root.join("BepInEx/core/BepInEx.dll"),
                b"scratch references",
            )
            .unwrap();
            fs::write(
                root.join("steamapps/appmanifest_1557740.acf"),
                r#""AppState" { "buildid" "999999999" }"#,
            )
            .unwrap();
            let (mut game, mut pack) = preparation_fixture(&game_root);
            game.app_id = 1557740;
            pack.game.app_id = 1557740;
            pack.game.name = "ROUNDS".into();
            pack.game.folder = "rounds".into();
            let input = root.join("future.dll");
            fs::write(&input, b"ordinary future public mod bytes").unwrap();
            pack.mods = vec![crate::modpacks::add_local(&input).unwrap()];
            let before = serde_json::to_vec(&pack).unwrap();
            let ordinary =
                prepare_install(&game, &pack, "", InstallOptions::default(), &|_| {}).unwrap();
            assert!(ordinary.game_sha256.is_none());
            assert_eq!(
                serde_json::to_vec(ordinary.effective_pack()).unwrap(),
                before
            );
            assert_eq!(
                ordinary.files.unwrap().plugins[0].1,
                b"ordinary future public mod bytes"
            );
            let enabled = InstallOptions {
                rebound_enabled: true,
            };
            assert!(enabled.translate(&game, &pack).unwrap());
            let error = prepare_install(&game, &pack, "", enabled, &|_| {})
                .err()
                .unwrap();
            assert!(error.to_string().contains("Beta account"));
            assert!(setup_with_options(&game, &pack, "", enabled, &|_| {}).is_err());
            assert!(!game_root.join("BepInEx/plugins").exists());
            assert!(!game_root.join("winhttp.dll").exists());
            assert_eq!(
                fs::read(game_root.join("BepInEx/core/BepInEx.dll")).unwrap(),
                b"scratch references"
            );
            game.app_id = 1686940;
            pack.game.app_id = 1686940;
            let other_game = prepare_install(&game, &pack, "", enabled, &|_| {}).unwrap();
            assert!(other_game.game_sha256.is_none());
            assert_eq!(
                other_game.files.unwrap().plugins[0].1,
                b"ordinary future public mod bytes"
            );
            assert_eq!(
                fs::read(&input).unwrap(),
                b"ordinary future public mod bytes"
            );
        });
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rebound_off_preserves_official_ducttape_branch_exception() {
        let root = temporary_preparation_root("official-ducttape");
        let game_root = root.join("steamapps/common/ROUNDS");
        fs::create_dir_all(&game_root).unwrap();
        fs::write(
            root.join("steamapps/appmanifest_1557740.acf"),
            r#""AppState" { "buildid" "999999999" }"#,
        )
        .unwrap();
        let (mut game, mut pack) = preparation_fixture(&game_root);
        game.app_id = 1557740;
        pack.game.app_id = 1557740;
        pack.game.folder = "rounds".into();
        pack.mods = serde_json::from_value(serde_json::json!([
            {"name":"DuctTape","version":"1","file":"Mods/ducttape.zip","provenance":{"provider":"thunderstore","id":"kieron_exe-DuctTape","source_url":"https://thunderstore.io/c/rounds/p/kieron_exe/DuctTape/"}},
            {"name":"UnboundLib","version":"3.2.14","file":"Mods/unbound.zip"}
        ])).unwrap();
        assert!(!InstallOptions::default().translate(&game, &pack).unwrap());
        crate::game_compat::check_pack(&game, &pack).unwrap();
        assert_eq!(crate::game_compat::required_branch(&pack), Some("public"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_later_archive_fails_preparation_before_loader_or_game_changes() {
        let root = temporary_preparation_root("invalid");
        crate::modpacks::with_test_root(root.clone(), || {
            let game_root = root.join("game");
            fs::create_dir(&game_root).unwrap();
            let (game, mut pack) = preparation_fixture(&game_root);
            let plugin = root.join("valid.dll");
            fs::write(&plugin, b"original DLL fixture").unwrap();
            let invalid = root.join("invalid.zip");
            fs::write(&invalid, b"invalid archive fixture").unwrap();
            pack.mods = vec![
                crate::modpacks::add_local(&plugin).unwrap(),
                crate::modpacks::add_local(&invalid).unwrap(),
            ];
            pack.save().unwrap();
            let saved = crate::modpacks::directory().join(format!("{}.canna.json", pack.id));
            let before = fs::read(&saved).unwrap();
            let error = prepare_install(&game, &pack, "", InstallOptions::default(), &|_| {})
                .err()
                .unwrap();
            assert!(error.to_string().contains("archive"));
            assert_eq!(fs::read_dir(&game_root).unwrap().count(), 0);
            assert_eq!(fs::read(&saved).unwrap(), before);
            assert_eq!(fs::read(&plugin).unwrap(), b"original DLL fixture");
            assert_eq!(fs::read(&invalid).unwrap(), b"invalid archive fixture");
        });
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preparation_honors_disabled_mods_and_preserves_source_selection() {
        let root = temporary_preparation_root("disabled");
        crate::modpacks::with_test_root(root.clone(), || {
            let game_root = root.join("game");
            fs::create_dir(&game_root).unwrap();
            let (game, mut pack) = preparation_fixture(&game_root);
            let plugin = root.join("legacy.dll");
            fs::write(&plugin, b"selected original bytes").unwrap();
            let invalid = root.join("disabled.zip");
            fs::write(&invalid, b"invalid but disabled").unwrap();
            pack.mods = vec![
                crate::modpacks::add_local(&plugin).unwrap(),
                crate::modpacks::add_local(&invalid).unwrap(),
            ];
            pack.mods[0].dependencies = vec!["Replacement supplies this".into()];
            pack.mods[1].enabled = false;
            fs::remove_file(crate::modpacks::local_directory().join(&pack.mods[1].local_file))
                .unwrap();
            let before = serde_json::to_vec(&pack).unwrap();
            let prepared =
                prepare_install(&game, &pack, "", InstallOptions::default(), &|_| {}).unwrap();
            assert_eq!(prepared.history.len(), 1);
            assert_eq!(prepared.files.as_ref().unwrap().plugins.len(), 1);
            assert_eq!(
                prepared.files.as_ref().unwrap().plugins[0].1,
                b"selected original bytes"
            );
            assert_eq!(
                serde_json::to_vec(prepared.effective_pack()).unwrap(),
                before
            );
            assert_eq!(serde_json::to_vec(&pack).unwrap(), before);
            assert_eq!(fs::read_dir(&game_root).unwrap().count(), 0);
        });
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_game_or_config_rejects_prepared_translation_before_setup() {
        for changed_game in [true, false] {
            let root = temporary_preparation_root(if changed_game {
                "game-changed"
            } else {
                "config-changed"
            });
            let (mut game, mut pack) = preparation_fixture(&root);
            game.app_id = 1557740;
            pack.game.app_id = 1557740;
            pack.game.name = "ROUNDS".into();
            pack.game.folder = "rounds".into();
            fs::create_dir_all(root.join("ROUNDS_Data/Managed")).unwrap();
            fs::write(
                root.join("ROUNDS_Data/Managed/Assembly-CSharp.dll"),
                b"current game fixture",
            )
            .unwrap();
            if !changed_game {
                fs::create_dir_all(root.join("BepInEx/config")).unwrap();
                fs::write(root.join("BepInEx/config/gameplay.cfg"), b"new settings").unwrap();
            }
            let prepared = PreparedInstall {
                pack,
                files: Some(PluginEntries::default()),
                source_files: None,
                history: vec![],
                framework: None,
                game_sha256: Some(if changed_game {
                    "0".repeat(64)
                } else {
                    crate::ducttape::game_hash(&game).unwrap()
                }),
                config_sha256: Some(vec![]),
                rebound_support_sha256: None,
                imported_configs: vec![],
                imported_config_source: None,
            };
            let error = install_prepared(&game, prepared, "", &|_| {}).unwrap_err();
            assert!(error.to_string().contains(if changed_game {
                "changed after compatibility preflight"
            } else {
                "config changed"
            }));
            assert!(!root.join("BepInEx/core").exists());
            assert!(!root.join("BepInEx/plugins").exists());
            assert!(!root.join("winhttp.dll").exists());
            assert!(!fs::read_dir(&root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".canna-stage-")
            }));
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn prepared_rounds_config_snapshot_ignores_loader_settings_but_guards_gameplay() {
        let root = temporary_preparation_root("config-scope");
        let (mut game, mut pack) = preparation_fixture(&root);
        game.app_id = 1557740;
        pack.game.app_id = 1557740;
        fs::create_dir_all(root.join("ROUNDS_Data/Managed")).unwrap();
        fs::write(
            root.join("ROUNDS_Data/Managed/Assembly-CSharp.dll"),
            b"unchanged game",
        )
        .unwrap();
        fs::create_dir_all(root.join("BepInEx/config")).unwrap();
        for (name, bytes) in [
            ("BepInEx.cfg", b"loader logging = true".as_slice()),
            ("cosmicrounds.cfg", b"Gameplay = chosen".as_slice()),
            ("hollowpurple.cfg", b"Cards = configured".as_slice()),
            ("UnboundLib.cfg", b"".as_slice()),
        ] {
            fs::write(root.join("BepInEx/config").join(name), bytes).unwrap();
        }
        let snapshot = crate::ducttape::current_configs(&game).unwrap();
        let prepared = PreparedInstall {
            pack,
            files: Some(PluginEntries::default()),
            source_files: None,
            history: vec![],
            framework: None,
            game_sha256: Some(crate::ducttape::game_hash(&game).unwrap()),
            config_sha256: Some(crate::ducttape::configuration_fingerprint(&snapshot).unwrap()),
            rebound_support_sha256: None,
            imported_configs: vec![],
            imported_config_source: None,
        };
        // This is the setup failure's actual snapshot/verification sequence.
        prepared.verify_inputs(&game).unwrap();
        fs::write(
            root.join("BepInEx/config/BepInEx.cfg"),
            b"loader logging = false",
        )
        .unwrap();
        prepared.verify_inputs(&game).unwrap();
        let gameplay = root.join("BepInEx/config/cosmicrounds.cfg");
        fs::write(&gameplay, b"Gameplay = changed externally").unwrap();
        assert!(
            prepared
                .verify_inputs(&game)
                .unwrap_err()
                .to_string()
                .contains("config changed")
        );
        fs::write(&gameplay, b"Gameplay = chosen").unwrap();
        let added = root.join("BepInEx/config/new-gameplay.cfg");
        fs::write(&added, b"new external setting").unwrap();
        assert!(prepared.verify_inputs(&game).is_err());
        fs::remove_file(added).unwrap();
        fs::remove_file(root.join("BepInEx/config/UnboundLib.cfg")).unwrap();
        assert!(prepared.verify_inputs(&game).is_err());
        fs::write(root.join("BepInEx/config/UnboundLib.cfg"), b"").unwrap();
        prepared.verify_inputs(&game).unwrap();
        fs::write(
            root.join("ROUNDS_Data/Managed/Assembly-CSharp.dll"),
            b"changed game",
        )
        .unwrap();
        assert!(
            prepared
                .verify_inputs(&game)
                .unwrap_err()
                .to_string()
                .contains("ROUNDS changed")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn prepared_rounds_recovery_override_matches_only_after_its_config_swap() {
        let root = temporary_preparation_root("config-recovery-scope");
        let (mut game, mut pack) = preparation_fixture(&root);
        game.app_id = 1557740;
        pack.game.app_id = 1557740;
        let active = root.join("BepInEx/config");
        fs::create_dir_all(active.join("nested")).unwrap();
        fs::write(active.join("BepInEx.cfg"), b"current loader settings").unwrap();
        fs::write(active.join("gameplay.cfg"), b"current gameplay settings").unwrap();
        let recovered = vec![
            (
                PathBuf::from("nested/extra.json"),
                b"{\"enabled\":true}".to_vec(),
            ),
            (
                PathBuf::from("gameplay.cfg"),
                b"snapshot gameplay settings".to_vec(),
            ),
            (
                PathBuf::from("BepInEx.cfg"),
                b"snapshot loader settings".to_vec(),
            ),
        ];
        let prepared = PreparedInstall {
            pack,
            files: Some(PluginEntries::default()),
            source_files: None,
            history: vec![],
            framework: None,
            game_sha256: None,
            config_sha256: Some(crate::ducttape::configuration_fingerprint(&recovered).unwrap()),
            rebound_support_sha256: None,
            imported_configs: vec![],
            imported_config_source: None,
        };
        assert!(prepared.verify_inputs(&game).is_err());
        // Recovery preflights the requested snapshot before replacing the active config tree.
        fs::rename(&active, root.join("config.previous")).unwrap();
        fs::create_dir_all(active.join("nested")).unwrap();
        for (path, bytes) in &recovered {
            fs::write(active.join(path), bytes).unwrap();
        }
        prepared.verify_inputs(&game).unwrap();
        fs::write(active.join("BepInEx.cfg"), b"new logging option").unwrap();
        prepared.verify_inputs(&game).unwrap();
        fs::write(active.join("nested/extra.json"), b"{\"enabled\":false}").unwrap();
        assert!(prepared.verify_inputs(&game).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn thunderstore_plugins_patchers_and_config_keep_their_routes() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for path in [
            "BepInEx/plugins/mod/plugin.dll",
            "BepInEx/patchers/patch.dll",
            "BepInEx/config/mod.cfg",
            "manifest.json",
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"fixture").unwrap();
        }
        let entries = plugin_entries(&zip.finish().unwrap().into_inner()).unwrap();
        assert_eq!(entries.plugins[0].0, Path::new("mod/plugin.dll"));
        assert_eq!(entries.patchers[0].0, Path::new("patch.dll"));
        assert_eq!(entries.configs[0].0, Path::new("mod.cfg"));
    }
    #[test]
    fn thunderstore_loader_roots_and_runtime_types_are_checked() {
        use std::io::Write;
        let zip = |core: &str| {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for path in [
                format!("BepInExPack/{core}"),
                "BepInExPack/winhttp.dll".into(),
                "BepInExPack/doorstop_config.ini".into(),
                "manifest.json".into(),
            ] {
                writer
                    .start_file(path, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(b"fixture").unwrap();
            }
            writer.finish().unwrap().into_inner()
        };
        let mono = zip("BepInEx/core/BepInEx.dll");
        assert!(framework_entries(&mono, false, 1686940).is_ok());
        let il2cpp = zip("BepInEx/core/BepInEx.Unity.IL2CPP.dll");
        assert!(
            framework_entries(&mono, false, 1966720)
                .unwrap()
                .iter()
                .any(|(p, _)| p == Path::new("winhttp.dll"))
        );
        assert!(framework_entries(&mono, true, 1966720).is_err());
        assert!(framework_entries(&il2cpp, true, 945360).is_ok());
        assert!(framework_entries(&il2cpp, false, 945360).is_err());
        assert!(pe_machine(b"not a PE").is_err());
    }
    #[test]
    #[ignore = "Downloads official loader for in-memory validation only"]
    fn live_generic_bepinex_pack_for_bopl() {
        let client = reqwest::blocking::Client::builder()
            .user_agent("Canna (https://cannamods.vip)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap();
        let metadata: serde_json::Value = client
            .get("https://thunderstore.io/api/experimental/package/BepInEx/BepInExPack/")
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .unwrap();
        let link = metadata["latest"]["download_url"].as_str().unwrap();
        assert!(link.starts_with("https://thunderstore.io/package/download/BepInEx/BepInExPack/"));
        let bytes = client
            .get(link)
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .unwrap();
        assert!(bytes.len() < 4 * 1024 * 1024);
        let entries = framework_entries(&bytes, false, 1686940).unwrap();
        let proxy = entries
            .iter()
            .find(|(p, _)| p == Path::new("winhttp.dll"))
            .unwrap();
        assert_eq!(pe_machine(&proxy.1).unwrap(), 0x8664);
        println!(
            "Official generic BepInExPack {}: Bopl Mono layout and x64 proxy verified",
            metadata["latest"]["version_number"]
        );
    }
    #[test]
    #[ignore = "Starts Bopl Battle through Steam and terminates only the retained launch process"]
    fn steam_launch_retains_and_stops_bopl() {
        let game = InstalledGame {
            app_id: 1686940,
            name: "Bopl Battle".into(),
            path: "D:/SteamLibrary/steamapps/common/Bopl Battle".into(),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        let owned = launch(&game, true).unwrap();
        assert!(owned.running());
        owned.stop().unwrap();
        for _ in 0..100 {
            if !owned.running() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("Owned game failed to exit");
    }
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
    #[ignore = "Downloads family catalog into a temporary game fixture; never launches the game"]
    fn family_catalog_visuals_and_dependencies_install() {
        let root =
            std::env::temp_dir().join(format!("canna-catalog-install-{}", std::process::id()));
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
        let mut mods = info.mods.clone();
        for item in &mut mods {
            item.enabled = true;
        }
        let pack = Modpack::create(
            "Full catalog fixture".into(),
            String::new(),
            info,
            crate::cache::Source::from_settings(&settings),
            mods,
        );
        pack.validate().unwrap();
        let game = InstalledGame {
            app_id: 1686940,
            name: "Fixture".into(),
            path: root.clone(),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        install_pack(&game, &pack, token, &|_| {}).unwrap();
        fn files(path: &Path, result: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    files(&path, result);
                } else {
                    result.push(path);
                }
            }
        }
        let mut installed = vec![];
        files(&root.join("BepInEx/plugins/Canna"), &mut installed);
        for required in [
            "Canna.SharedColors.dll",
            "Canna.FriendsTrajectories.dll",
            "FourthAbilitySlot.dll",
            "FourthAbilitySlotStableRepair.dll",
        ] {
            assert!(
                installed.iter().any(|p| p.file_name().unwrap() == required),
                "Missing {required}"
            );
        }
        assert!(
            installed
                .iter()
                .filter(|p| p.extension().is_some_and(|e| e == "png")
                    && p.to_string_lossy().contains("Assets"))
                .count()
                >= 22
        );
        assert!(
            !installed
                .iter()
                .any(|p| p.file_name().unwrap() == "Canna.CatalogAudit.dll")
        );
        assert!(pack.mods.len() >= 24);
        fs::remove_dir_all(root).unwrap();
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
    fn new_framework_is_receipted_and_inert_until_modded_launch() {
        let root = temporary_preparation_root("vanilla-bootstrap");
        crate::modpacks::with_test_root(root.join("test-state"), || {
            fs::create_dir(root.join("Rounds_Data")).unwrap();
            let mut pe = vec![0u8; 256];
            pe[..2].copy_from_slice(b"MZ");
            pe[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
            pe[0x80..0x84].copy_from_slice(b"PE\0\0");
            pe[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
            fs::write(root.join("Rounds.exe"), &pe).unwrap();
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (name, data) in [
            ("BepInEx/core/BepInEx.dll", b"fixture core".as_slice()),
            ("winhttp.dll", pe.as_slice()),
            ("doorstop_config.ini", b"[UnityDoorstop]\nenabled=true\ntargetAssembly=BepInEx\\core\\BepInEx.Preloader.dll\n".as_slice()),
            ("BepInEx/config/BepInEx.cfg", b"Value = default".as_slice()),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(data).unwrap();
        }
            let framework = writer.finish().unwrap().into_inner();
            let (mut game, mut pack) = preparation_fixture(&root);
            game.app_id = 1557740;
            pack.game.app_id = 1557740;
            pack.game.name = "ROUNDS".into();
            pack.game.folder = "rounds".into();
            fs::create_dir_all(root.join("ROUNDS_Data/Managed")).unwrap();
            fs::write(
                root.join("ROUNDS_Data/Managed/Assembly-CSharp.dll"),
                b"fixture game assembly",
            )
            .unwrap();
            fs::create_dir_all(root.join("BepInEx/config")).unwrap();
            fs::write(
                root.join("BepInEx/config/gameplay.cfg"),
                b"saved gameplay settings",
            )
            .unwrap();
            let prepared = |loader: Option<Vec<u8>>| PreparedInstall {
                pack: pack.clone(),
                files: Some(PluginEntries {
                    plugins: vec![(
                        "fixture-mod.dll".into(),
                        b"fixture plugin, never executed".to_vec(),
                    )],
                    ..PluginEntries::default()
                }),
                source_files: None,
                history: vec![],
                framework: loader,
                game_sha256: Some(crate::ducttape::game_hash(&game).unwrap()),
                config_sha256: Some(
                    crate::ducttape::configuration_fingerprint(
                        &crate::ducttape::current_configs(&game).unwrap(),
                    )
                    .unwrap(),
                ),
                rebound_support_sha256: None,
                imported_configs: vec![],
                imported_config_source: None,
            };
            install_prepared(&game, prepared(Some(framework.clone())), "", &|_| {}).unwrap();
            assert!(
                fs::read_to_string(root.join("doorstop_config.ini"))
                    .unwrap()
                    .contains("enabled=false")
            );
            fs::write(
                root.join("BepInEx/config/BepInEx.cfg"),
                b"Value = user setting",
            )
            .unwrap();
            set_mode(&root, true).unwrap();
            install_prepared(&game, prepared(None), "", &|_| {}).unwrap();
            assert!(
                fs::read_to_string(root.join("doorstop_config.ini"))
                    .unwrap()
                    .contains("enabled=true")
            );
            restore_vanilla(&game).unwrap();
            assert!(!root.join("winhttp.dll").exists());
            assert_eq!(fs::read(root.join("Rounds.exe")).unwrap(), pe);
            install_prepared(&game, prepared(None), "", &|_| {}).unwrap();
            assert!(root.join("winhttp.dll").is_file());
            assert!(
                fs::read_to_string(root.join("doorstop_config.ini"))
                    .unwrap()
                    .contains("enabled=false")
            );
            assert_eq!(
                fs::read(root.join("BepInEx/config/BepInEx.cfg")).unwrap(),
                b"Value = user setting"
            );
            assert_eq!(
                fs::read(root.join("BepInEx/config/gameplay.cfg")).unwrap(),
                b"saved gameplay settings"
            );
            assert!(root.join("BepInEx/plugins/Canna/fixture-mod.dll").is_file());
        });
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn activation_preserves_user_config_and_rolls_back_both_routes() {
        let root = std::env::temp_dir().join(format!("canna-activate-{}", std::process::id()));
        fs::create_dir_all(root.join("BepInEx/plugins/Canna")).unwrap();
        fs::create_dir_all(root.join("BepInEx/patchers/Canna")).unwrap();
        fs::create_dir_all(root.join("BepInEx/config")).unwrap();
        fs::write(root.join("BepInEx/plugins/Canna/old.dll"), b"old").unwrap();
        fs::write(root.join("BepInEx/patchers/Canna/old.dll"), b"old").unwrap();
        fs::write(root.join("BepInEx/config/custom.cfg"), b"user").unwrap();
        let stage = root.join("stage");
        fs::create_dir_all(stage.join("plugins")).unwrap();
        fs::write(stage.join("plugins/new.dll"), b"new").unwrap();
        // Missing patcher stage forces failure after the first promotion.
        assert!(
            activate_stage(&root, &stage, vec![("new.cfg".into(), b"default".to_vec())]).is_err()
        );
        assert!(root.join("BepInEx/plugins/Canna/old.dll").is_file());
        assert!(root.join("BepInEx/patchers/Canna/old.dll").is_file());
        assert!(!root.join("BepInEx/config/new.cfg").exists());
        fs::create_dir_all(stage.join("patchers")).unwrap();
        fs::write(stage.join("patchers/new.dll"), b"new").unwrap();
        activate_stage(
            &root,
            &stage,
            vec![
                ("custom.cfg".into(), b"default".to_vec()),
                ("new.cfg".into(), b"default".to_vec()),
            ],
        )
        .unwrap();
        assert_eq!(
            fs::read(root.join("BepInEx/config/custom.cfg")).unwrap(),
            b"user"
        );
        assert!(root.join("BepInEx/plugins/Canna/new.dll").is_file());
        assert!(root.join("BepInEx/patchers/Canna/new.dll").is_file());
        assert!(!root.join("BepInEx/plugins/Canna.previous").exists());
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

#[cfg(test)]
#[path = "runtime_live_tests.rs"]
mod live_tests;
