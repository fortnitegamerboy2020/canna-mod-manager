//! Local-only recovery. Snapshot immutable archives and configs; never restore saves implicitly.
use crate::{model::InstalledGame, modpacks::Modpack};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[derive(Clone, Serialize, Deserialize)]
pub struct Policy {
    pub directory: PathBuf,
    pub budget_gb: u64,
    pub automatic: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            directory: crate::modpacks::directory()
                .parent()
                .unwrap()
                .join("recovery"),
            budget_gb: 5,
            automatic: true,
        }
    }
}
fn state() -> PathBuf {
    crate::modpacks::directory()
        .parent()
        .unwrap()
        .join("play-lab")
}
impl Policy {
    pub fn load() -> Self {
        fs::read(state().join("policy.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<()> {
        anyhow::ensure!((1..=1000).contains(&self.budget_gb), "Choose 1–1000 GB");
        anyhow::ensure!(
            self.directory.is_absolute()
                && !self
                    .directory
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "Choose an absolute backup folder"
        );
        fs::create_dir_all(state())?;
        atomic(&state().join("policy.json"), &serde_json::to_vec(self)?)?;
        Ok(())
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::runtime::no_links(path)?;
    let pending = path.with_extension("pending");
    crate::runtime::no_links(&pending)?;
    let mut file = fs::File::create(&pending)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(pending, path)?;
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub target_key: String,
    #[serde(default)]
    pub steam_version: Option<(String, String)>,
    pub id: String,
    pub game: u32,
    pub created: u64,
    pub working: bool,
    pub pack: Modpack,
    pub configs: Vec<(String, String)>,
    pub bytes: u64,
}
fn time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn target_key(game: &InstalledGame) -> String {
    if game.app_id == u32::MAX {
        format!(
            "minecraft-{}",
            game.path.file_name().unwrap_or_default().to_string_lossy()
        )
    } else {
        game.app_id.to_string()
    }
}
fn config_root(game: &InstalledGame) -> PathBuf {
    if game.app_id == u32::MAX {
        game.path.join("config")
    } else if let Some(addons) = crate::model::source_addons(game.app_id) {
        game.path
            .join(Path::new(addons).parent().unwrap())
            .join("cfg")
    } else {
        game.path.join("BepInEx/config")
    }
}
fn game_store(policy: &Policy, game: u32) -> PathBuf {
    policy.directory.join("snapshots").join(game.to_string())
}
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 100 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn size(root: &Path) -> Result<u64> {
    crate::runtime::no_links(root)?;
    if !root.exists() {
        return Ok(0);
    }
    let mut bytes = 0u64;
    for e in fs::read_dir(root)? {
        let p = e?.path();
        crate::runtime::no_links(&p)?;
        let m = fs::symlink_metadata(&p)?;
        bytes = bytes
            .checked_add(if m.is_dir() {
                size(&p)?
            } else if m.is_file() {
                m.len()
            } else {
                bail!("Unexpected backup entry")
            })
            .context("Backup size overflow")?;
    }
    Ok(bytes)
}
fn copy_verified(source: &Path, target: &Path, expected: &str) -> Result<u64> {
    crate::runtime::no_links(source)?;
    anyhow::ensure!(
        fs::metadata(source)?.len() <= 128 * 1024 * 1024,
        "Archive exceeds recovery limits"
    );
    let mut input = fs::File::open(source)?;
    let mut output = fs::File::create(target)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut size = 0;
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
        output.write_all(&buffer[..n])?;
        size += n as u64;
    }
    anyhow::ensure!(
        format!("{:x}", digest.finalize()) == expected.to_lowercase(),
        "Recovery archive digest mismatch"
    );
    output.sync_all()?;
    Ok(size)
}
fn config_files(root: &Path, base: &Path, result: &mut Vec<PathBuf>) -> Result<()> {
    crate::runtime::no_links(root)?;
    if !root.exists() {
        return Ok(());
    }
    for e in fs::read_dir(root)? {
        let p = e?.path();
        crate::runtime::no_links(&p)?;
        let m = fs::symlink_metadata(&p)?;
        if m.is_dir() {
            config_files(&p, base, result)?
        } else if m.is_file()
            && p.extension().is_some_and(|e| {
                e.eq_ignore_ascii_case("cfg")
                    || e.eq_ignore_ascii_case("toml")
                    || e.eq_ignore_ascii_case("json")
            })
        {
            anyhow::ensure!(
                result.len() < 1000 && m.len() <= 1024 * 1024,
                "Configuration backup exceeds limits"
            );
            result.push(p.strip_prefix(base)?.to_path_buf());
        }
    }
    Ok(())
}
pub fn list(policy: &Policy, game: u32) -> Result<Vec<Snapshot>> {
    let root = game_store(policy, game);
    crate::runtime::no_links(&root)?;
    let mut items = vec![];
    if !root.exists() {
        return Ok(items);
    }
    for e in fs::read_dir(&root)? {
        let path = e?.path();
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with("-pending"))
        {
            continue;
        }
        crate::runtime::no_links(&path)?;
        if !path.is_dir() {
            continue;
        }
        let file = path.join("snapshot.json");
        crate::runtime::no_links(&file)?;
        if file.is_file() {
            anyhow::ensure!(
                fs::metadata(&file)?.len() <= 2 * 1024 * 1024,
                "Recovery metadata too large"
            );
            let snapshot: Snapshot = serde_json::from_slice(&fs::read(file)?)?;
            anyhow::ensure!(
                snapshot.game == game
                    && safe_id(&snapshot.id)
                    && path.file_name() == Some(std::ffi::OsStr::new(&snapshot.id)),
                "Invalid recovery metadata"
            );
            snapshot.pack.validate()?;
            items.push(snapshot);
        }
    }
    items.sort_by_key(|s| std::cmp::Reverse(s.created));
    Ok(items)
}
fn archive(_pack: &Modpack, item: &crate::model::ModInfo) -> PathBuf {
    if !item.local_file.is_empty() {
        crate::modpacks::local_directory().join(&item.local_file)
    } else {
        let ext = Path::new(&item.file)
            .extension()
            .unwrap_or_default()
            .to_string_lossy();
        crate::modpacks::directory()
            .parent()
            .unwrap()
            .join("downloads")
            .join(format!(
                "{}.{}",
                item.sha256.to_lowercase(),
                ext.to_lowercase()
            ))
    }
}
pub fn capture(
    policy: &Policy,
    game: &InstalledGame,
    pack: &Modpack,
    working: bool,
) -> Result<Snapshot> {
    let _guard = LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Recovery is busy"))?;
    capture_in(policy, game, pack, working)
}
fn capture_in(
    policy: &Policy,
    game: &InstalledGame,
    pack: &Modpack,
    working: bool,
) -> Result<Snapshot> {
    pack.validate()?;
    anyhow::ensure!(
        game.app_id == pack.game.app_id,
        "Recovery belongs to another game"
    );
    crate::runtime::ensure_closed(game)?;
    anyhow::ensure!(
        (1..=1000).contains(&policy.budget_gb) && policy.directory.is_absolute(),
        "Invalid recovery policy"
    );
    crate::runtime::no_links(&policy.directory)?;
    fs::create_dir_all(&policy.directory)?;
    let marker = policy.directory.join(".canna-recovery-owner");
    crate::runtime::no_links(&marker)?;
    if marker.exists() {
        anyhow::ensure!(
            fs::read(&marker)? == b"CannaRecovery-v1",
            "Backup folder ownership marker is invalid"
        );
    } else {
        anyhow::ensure!(
            fs::read_dir(&policy.directory)?.next().is_none(),
            "Choose an empty dedicated Canna backup folder"
        );
        atomic(&marker, b"CannaRecovery-v1")?;
    }
    let root = game_store(policy, game.app_id);
    fs::create_dir_all(&root)?;
    let id = format!(
        "snapshot-{}-{}",
        time(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos()
    );
    let stage = root.join(format!("{id}-pending"));
    fs::create_dir(&stage)?;
    let result = (|| -> Result<Snapshot> {
        fs::create_dir(stage.join("archives"))?;
        fs::create_dir(stage.join("configs"))?;
        let mut bytes = 0;
        let mut stored = std::collections::BTreeSet::new();
        for item in pack.mods.iter().filter(|m| m.enabled) {
            anyhow::ensure!(
                !item.sha256.is_empty(),
                "{} lacks an archive digest",
                item.name
            );
            let path = archive(pack, item);
            let filename = path.file_name().context("Invalid archive name")?;
            if stored.insert(filename.to_os_string()) {
                bytes+=copy_verified(&path,&stage.join("archives").join(filename),&item.sha256).with_context(||format!("Recovery archive missing for {}; re-download the pinned release before replacing this setup",item.name))?;
            }
        }
        let config_root = config_root(game);
        let mut configs = vec![];
        let mut paths = vec![];
        if config_root.exists() {
            config_files(&config_root, &config_root, &mut paths)?;
        }
        for (index, path) in paths.iter().enumerate() {
            let data = fs::read(config_root.join(path))?;
            bytes += data.len() as u64;
            let filename = format!("{index}.cfg");
            fs::write(stage.join("configs").join(&filename), &data)?;
            configs.push((
                path.to_string_lossy().replace('\\', "/"),
                format!("{:x}", Sha256::digest(&data)),
            ));
        }
        let snapshot = Snapshot {
            target_key: target_key(game),
            steam_version: crate::steam::installed_version(game).map(|v| (v.branch, v.build)),
            id: id.clone(),
            game: game.app_id,
            created: time(),
            working,
            pack: pack.clone(),
            configs,
            bytes,
        };
        anyhow::ensure!(
            size(&policy.directory)? <= policy.budget_gb * 1024 * 1024 * 1024,
            "Backup budget reached; remove an older snapshot or increase the budget"
        );
        atomic(
            &stage.join("snapshot.json"),
            &serde_json::to_vec_pretty(&snapshot)?,
        )?;
        fs::rename(&stage, root.join(&id))?;
        Ok(snapshot)
    })();
    if result.is_err() && stage.exists() {
        crate::runtime::no_links(&stage)?;
        fs::remove_dir_all(&stage)?;
    }
    if result.is_ok() {
        let snapshots = list(policy, game.app_id)?;
        for old in snapshots.iter().skip(5).filter(|s| !s.working) {
            remove_in(policy, old)?;
        }
    }
    result
}
fn remove_in(policy: &Policy, snapshot: &Snapshot) -> Result<()> {
    anyhow::ensure!(safe_id(&snapshot.id), "Invalid snapshot ID");
    let root = game_store(policy, snapshot.game).join(&snapshot.id);
    crate::runtime::no_links(&root)?;
    let resolved = fs::canonicalize(&root)?;
    let allowed = fs::canonicalize(policy.directory.join("snapshots"))?;
    anyhow::ensure!(
        resolved.starts_with(&allowed) && resolved != allowed,
        "Snapshot escapes backup directory"
    );
    // Validate every descendant as well: a junction must never redirect deletion.
    size(&resolved)?;
    fs::remove_dir_all(resolved)?;
    Ok(())
}
pub fn remove(policy: &Policy, snapshot: &Snapshot) -> Result<()> {
    let _guard = LOCK.lock().map_err(|_| anyhow::anyhow!("Recovery busy"))?;
    anyhow::ensure!(
        !snapshot.working,
        "Retain the last working snapshot; mark another one working first"
    );
    remove_in(policy, snapshot)
}
pub fn mark_working(policy: &Policy, snapshot: &Snapshot) -> Result<()> {
    let _guard = LOCK.lock().map_err(|_| anyhow::anyhow!("Recovery busy"))?;
    for mut item in list(policy, snapshot.game)? {
        if item.target_key != snapshot.target_key {
            continue;
        }
        item.working = item.id == snapshot.id;
        atomic(
            &game_store(policy, item.game)
                .join(&item.id)
                .join("snapshot.json"),
            &serde_json::to_vec(&item)?,
        )?;
    }
    Ok(())
}
pub fn remember_applied(game: &InstalledGame, pack: &Modpack) -> Result<()> {
    fs::create_dir_all(state())?;
    atomic(
        &state().join(format!("applied-{}.json", target_key(game))),
        &serde_json::to_vec(pack)?,
    )?;
    Ok(())
}
pub fn last_applied(game: &InstalledGame) -> Result<Option<Modpack>> {
    let path = state().join(format!("applied-{}.json", target_key(game)));
    last_applied_at(&path, game.app_id)
}
fn last_applied_at(path: &Path, app_id: u32) -> Result<Option<Modpack>> {
    crate::runtime::no_links(path)?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= 2 * 1024 * 1024,
        "Applied pack metadata exceeds limits or is not a file"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Applied pack metadata grew beyond limits"
    );
    let pack: Modpack = serde_json::from_slice(&bytes)?;
    pack.validate()?;
    anyhow::ensure!(
        pack.game.app_id == app_id,
        "Applied pack belongs to another game"
    );
    Ok(Some(pack))
}
pub fn before_change(game: &InstalledGame, next: &Modpack) -> Result<()> {
    let policy = Policy::load();
    if !policy.automatic {
        return Ok(());
    }
    let file = state().join(format!("applied-{}.json", target_key(game)));
    crate::runtime::no_links(&file)?;
    if !file.exists() {
        return Ok(());
    }
    anyhow::ensure!(
        fs::metadata(&file)?.len() <= 2 * 1024 * 1024,
        "Applied pack metadata exceeds limits"
    );
    let pack: Modpack = serde_json::from_slice(&fs::read(file)?)?;
    if serde_json::to_value(&pack)? == serde_json::to_value(next)? {
        return Ok(());
    }
    capture(&policy, game, &pack, false)?;
    Ok(())
}
pub fn restore(
    policy: &Policy,
    game: &InstalledGame,
    snapshot: &Snapshot,
    token: &str,
    options: crate::runtime::InstallOptions,
    progress: &dyn Fn(&str),
) -> Result<Modpack> {
    let _guard = LOCK.lock().map_err(|_| anyhow::anyhow!("Recovery busy"))?;
    crate::runtime::ensure_closed(game)?;
    anyhow::ensure!(
        snapshot.game == game.app_id
            && (snapshot.target_key.is_empty() && game.app_id != u32::MAX
                || snapshot.target_key == target_key(game))
            && safe_id(&snapshot.id),
        "Snapshot belongs to another game"
    );
    snapshot.pack.validate()?;
    let translate = options.translate(game, &snapshot.pack)?;
    if let (Some(expected), Some(current)) = (
        &snapshot.steam_version,
        crate::steam::installed_version(game),
    ) {
        anyhow::ensure!(
            *expected == (current.branch, current.build),
            "The game's branch/build changed; review the old setup in a separate test copy"
        );
    }
    let root = game_store(policy, game.app_id).join(&snapshot.id);
    crate::runtime::no_links(&root)?;
    // Preflight every archive and config before changing any game files.
    let mut restored = snapshot.pack.clone();
    let mut configs = vec![];
    fs::create_dir_all(crate::modpacks::local_directory())?;
    for item in restored.mods.iter_mut().filter(|m| m.enabled) {
        let extension = Path::new(&item.file)
            .extension()
            .context("Missing archive extension")?
            .to_string_lossy()
            .to_lowercase();
        let name = format!("{}.{extension}", item.sha256.to_lowercase());
        let temp = crate::modpacks::local_directory().join(format!("{name}.pending"));
        crate::runtime::no_links(&temp)?;
        copy_verified(&root.join("archives").join(&name), &temp, &item.sha256)?;
        let destination = crate::modpacks::local_directory().join(&name);
        crate::runtime::no_links(&destination)?;
        fs::rename(temp, destination)?;
        item.local_file = name;
    }
    for (index, (path, hash)) in snapshot.configs.iter().enumerate() {
        anyhow::ensure!(
            crate::repository::valid_path(path),
            "Invalid recovery config path"
        );
        let source = root.join("configs").join(format!("{index}.cfg"));
        crate::runtime::no_links(&source)?;
        anyhow::ensure!(
            fs::metadata(&source)?.len() <= 1024 * 1024,
            "Config exceeds limits"
        );
        let bytes = fs::read(source)?;
        anyhow::ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == *hash,
            "Recovery config digest mismatch"
        );
        let target = config_root(game).join(path);
        crate::runtime::no_links(&target)?;
        configs.push((target, bytes));
    }
    // Preflight against the config that recovery will install, without changing
    // the active config or executing any selected mod.
    let previous_config_hashes = if translate {
        Some(crate::ducttape::configuration_hashes(game)?)
    } else {
        None
    };
    let effective_configs = if translate {
        let mut effective = crate::ducttape::current_configs(game)?
            .into_iter()
            .filter(|(path, _)| {
                !path.extension().is_some_and(|e| {
                    e.eq_ignore_ascii_case("cfg")
                        || e.eq_ignore_ascii_case("toml")
                        || e.eq_ignore_ascii_case("json")
                })
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut names = std::collections::BTreeSet::new();
        for (target, bytes) in &configs {
            let relative = target.strip_prefix(config_root(game))?.to_owned();
            anyhow::ensure!(
                names.insert(relative.to_string_lossy().to_lowercase()),
                "Duplicate snapshot config path"
            );
            effective.insert(relative, bytes.clone());
        }
        Some(effective.into_iter().collect::<Vec<_>>())
    } else {
        None
    };
    let prepared = crate::runtime::prepare_install_with_configs(
        game,
        &restored,
        token,
        options,
        effective_configs.as_deref(),
        progress,
    )?;
    let applied_pack = prepared.effective_pack().clone();
    // Preserve the current applied setup before rollback; the closed-game transaction handles activation.
    let applied = state().join(format!("applied-{}.json", target_key(game)));
    if applied.exists() {
        let current: Modpack = serde_json::from_slice(&fs::read(applied)?)?;
        capture_in(policy, game, &current, false)?;
    }
    let config_root = config_root(game);
    let staged = game.path.join(".canna-config-recovery-stage");
    let previous = game.path.join(".canna-config-recovery-previous");
    crate::runtime::no_links(&staged)?;
    crate::runtime::no_links(&previous)?;
    anyhow::ensure!(
        !staged.exists() && !previous.exists(),
        "A previous config recovery needs inspection"
    );
    fs::create_dir(&staged)?;
    let prepare = (|| -> Result<()> {
        preserve_other_configs(&config_root, &config_root, &staged)?;
        let mut names = std::collections::BTreeSet::new();
        for (target, bytes) in &configs {
            let relative = target.strip_prefix(&config_root)?;
            anyhow::ensure!(
                names.insert(relative.to_string_lossy().to_lowercase()),
                "Duplicate snapshot config path"
            );
            let target = staged.join(relative);
            fs::create_dir_all(target.parent().unwrap())?;
            fs::write(target, bytes)?;
        }
        Ok(())
    })();
    if let Err(error) = prepare {
        remove_recovery_tree(&game.path, &staged)?;
        return Err(error);
    }
    let closed_and_unchanged =
        crate::runtime::ensure_closed(game).and_then(|()| prepared.verify_game(game));
    if let Err(error) = closed_and_unchanged {
        remove_recovery_tree(&game.path, &staged)?;
        return Err(error);
    }
    if let Some(expected) = previous_config_hashes {
        let unchanged =
            crate::ducttape::configuration_hashes(game).map(|current| current == expected);
        if !matches!(unchanged, Ok(true)) {
            remove_recovery_tree(&game.path, &staged)?;
            bail!("ROUNDS config changed during recovery preflight; retry");
        }
    }
    let had_config = config_root.exists();
    if had_config && let Err(error) = fs::rename(&config_root, &previous) {
        remove_recovery_tree(&game.path, &staged)?;
        return Err(error.into());
    }
    fs::create_dir_all(config_root.parent().unwrap())?;
    if let Err(error) = fs::rename(&staged, &config_root) {
        if had_config {
            fs::rename(&previous, &config_root)?;
        }
        remove_recovery_tree(&game.path, &staged)?;
        return Err(error.into());
    }
    let prepared_restore = prepared
        .verify_inputs(game)
        .and_then(|()| prepared.cache_downloads(progress))
        .and_then(|()| crate::runtime::restore_vanilla(game).map(|_| ()));
    if let Err(error) = prepared_restore {
        fs::rename(&config_root, &staged)?;
        if had_config {
            fs::rename(&previous, &config_root)?;
        }
        remove_recovery_tree(&game.path, &staged)?;
        return Err(error);
    }
    restored.save()?;
    remember_applied(game, &applied_pack)?;
    if previous.exists() {
        remove_recovery_tree(&game.path, &previous)?;
    }
    Ok(restored)
}
fn remove_recovery_tree(game_root: &Path, target: &Path) -> Result<()> {
    let root = game_root.canonicalize()?;
    let resolved = target.canonicalize()?;
    anyhow::ensure!(
        resolved.starts_with(&root)
            && resolved != root
            && target
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".canna-config-recovery-")),
        "Recovery cleanup escaped its game folder"
    );
    size(&resolved)?;
    fs::remove_dir_all(resolved)?;
    Ok(())
}
fn preserve_other_configs(path: &Path, base: &Path, stage: &Path) -> Result<()> {
    crate::runtime::no_links(path)?;
    if !path.exists() {
        return Ok(());
    }
    anyhow::ensure!(
        size(path)? <= 256 * 1024 * 1024,
        "Config recovery folder exceeds limits"
    );
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        crate::runtime::no_links(&path)?;
        if path.is_dir() {
            preserve_other_configs(&path, base, stage)?;
        } else if path.is_file()
            && !path.extension().is_some_and(|e| {
                e.eq_ignore_ascii_case("cfg")
                    || e.eq_ignore_ascii_case("toml")
                    || e.eq_ignore_ascii_case("json")
            })
        {
            let target = stage.join(path.strip_prefix(base)?);
            fs::create_dir_all(target.parent().unwrap())?;
            fs::copy(path, target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn last_applied_pack_rejects_other_games_and_unbounded_or_invalid_metadata() {
        fixture(|root, game, pack, _| {
            let file = root.join("applied-test.json");
            assert!(last_applied_at(&file, game.app_id).unwrap().is_none());
            fs::write(&file, serde_json::to_vec(&pack).unwrap()).unwrap();
            assert_eq!(
                last_applied_at(&file, game.app_id).unwrap().unwrap().id,
                pack.id
            );
            assert!(last_applied_at(&file, 1557740).is_err());
            fs::write(&file, b"invalid").unwrap();
            assert!(last_applied_at(&file, game.app_id).is_err());
            fs::write(&file, vec![b' '; 2 * 1024 * 1024 + 1]).unwrap();
            assert!(last_applied_at(&file, game.app_id).is_err());
            assert!(last_applied_at(root, game.app_id).is_err());
        });
    }
    fn fixture(run: impl FnOnce(&Path, InstalledGame, Modpack, Policy)) {
        let root = std::env::temp_dir().join(format!(
            "canna-workflow-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        crate::modpacks::with_test_root(root.join("state"), || {
            let game_path = root.join("game");
            for folder in [
                "BepInEx/core",
                "BepInEx/config",
                "BepInEx/plugins/Canna",
                "saves",
            ] {
                fs::create_dir_all(game_path.join(folder)).unwrap();
            }
            for file in [
                "BepInEx/core/BepInEx.dll",
                "winhttp.dll",
                "doorstop_config.ini",
            ] {
                fs::write(
                    game_path.join(file),
                    if file == "doorstop_config.ini" {
                        b"[UnityDoorstop]\nenabled=false\n".as_slice()
                    } else {
                        b"fixture".as_slice()
                    },
                )
                .unwrap();
            }
            fs::write(
                game_path.join("BepInEx/config/example.cfg"),
                b"Enabled = true\n",
            )
            .unwrap();
            fs::write(
                game_path.join("saves/world.bin"),
                b"world remains untouched",
            )
            .unwrap();
            let hash = format!("{:x}", Sha256::digest(b"original plugin"));
            fs::create_dir_all(crate::modpacks::local_directory()).unwrap();
            fs::write(
                crate::modpacks::local_directory().join(format!("{hash}.dll")),
                b"original plugin",
            )
            .unwrap();
            let item = crate::model::ModInfo {
                provenance: serde_json::Value::Null,
                enabled: true,
                name: "Example".into(),
                version: "1".into(),
                content_type: String::new(),
                description: String::new(),
                file: "Mods/example.dll".into(),
                sha256: hash.clone(),
                local_file: format!("{hash}.dll"),
                dependencies: vec![],
            };
            let pack = Modpack::create(
                "Fixture".into(),
                String::new(),
                &crate::model::bopl(),
                crate::cache::Source {
                    owner: "canna".into(),
                    repository: "server".into(),
                    branch: "main".into(),
                    catalog_folder: String::new(),
                },
                vec![item],
            );
            let game = InstalledGame {
                app_id: 1686940,
                name: "Fixture".into(),
                path: game_path,
                loader: "BepInEx".into(),
                plugins: 1,
                icon: None,
            };
            let policy = Policy {
                directory: root.join("backups"),
                budget_gb: 1,
                automatic: true,
            };
            run(&root, game, pack, policy);
        });
        let absolute = root.canonicalize().unwrap();
        assert!(absolute.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        fs::remove_dir_all(absolute).unwrap();
    }
    #[test]
    fn recovery_restores_exact_archives_configs_and_preserves_worlds() {
        fixture(|_, game, pack, policy| {
            let snapshot = capture(&policy, &game, &pack, true).unwrap();
            assert_eq!(list(&policy, game.app_id).unwrap().len(), 1);
            fs::write(
                game.path.join("BepInEx/config/example.cfg"),
                b"Enabled = false\n",
            )
            .unwrap();
            let restored =
                restore(&policy, &game, &snapshot, "", Default::default(), &|_| {}).unwrap();
            assert_eq!(restored.mods[0].sha256, pack.mods[0].sha256);
            assert_eq!(
                fs::read(game.path.join("BepInEx/config/example.cfg")).unwrap(),
                b"Enabled = true\n"
            );
            assert!(!game.path.join("BepInEx/plugins/Canna").exists());
            assert_eq!(
                fs::read(crate::modpacks::local_directory().join(&restored.mods[0].local_file))
                    .unwrap(),
                b"original plugin"
            );
            assert_eq!(
                fs::read(game.path.join("saves/world.bin")).unwrap(),
                b"world remains untouched"
            );
            assert!(remove(&policy, &snapshot).is_err());
        });
    }
    #[test]
    fn corrupt_snapshot_fails_before_touching_the_active_setup() {
        fixture(|_, game, pack, policy| {
            let snapshot = capture(&policy, &game, &pack, false).unwrap();
            let archive = game_store(&policy, game.app_id)
                .join(&snapshot.id)
                .join("archives")
                .join(&pack.mods[0].local_file);
            fs::write(archive, b"corrupt").unwrap();
            fs::write(
                game.path.join("BepInEx/plugins/Canna/active.dll"),
                b"active",
            )
            .unwrap();
            assert!(restore(&policy, &game, &snapshot, "", Default::default(), &|_| {}).is_err());
            assert_eq!(
                fs::read(game.path.join("BepInEx/plugins/Canna/active.dll")).unwrap(),
                b"active"
            );
            assert_eq!(
                fs::read(game.path.join("BepInEx/config/example.cfg")).unwrap(),
                b"Enabled = true\n"
            );
        });
    }
    #[test]
    fn rebound_snapshot_requires_opt_in_before_recovery_changes() {
        fixture(|_, mut game, mut pack, policy| {
            game.app_id = 1557740;
            pack.game.app_id = 1557740;
            pack.game.name = "ROUNDS".into();
            pack.game.folder = "rounds".into();
            pack.mods[0].provenance = serde_json::json!({"compatibility_profile":"rounds-public-1.1.2","required_game_branch":"public"});
            let snapshot = capture(&policy, &game, &pack, false).unwrap();
            let config = game.path.join("BepInEx/config/example.cfg");
            fs::write(&config, b"current config remains").unwrap();
            let active = game.path.join("BepInEx/plugins/Canna/active.dll");
            fs::write(&active, b"current plugin remains").unwrap();
            let error = restore(&policy, &game, &snapshot, "", Default::default(), &|_| {})
                .err()
                .unwrap();
            assert!(error.to_string().contains("Enable Canna Bliss"));
            assert_eq!(fs::read(&config).unwrap(), b"current config remains");
            assert_eq!(fs::read(&active).unwrap(), b"current plugin remains");
            assert!(!game.path.join(".canna-config-recovery-stage").exists());
            assert!(!game.path.join(".canna-config-recovery-previous").exists());
        });
    }
    #[cfg(canna_ducttape_preview)]
    #[test]
    fn rebound_restore_preflight_failure_preserves_active_configs_and_plugins() {
        fixture(|root, mut game, mut pack, policy| {
            let destination = root.join("steamapps/common/ROUNDS");
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::rename(&game.path, &destination).unwrap();
            game.path = destination;
            game.app_id = 1557740;
            pack.game.app_id = 1557740;
            pack.game.name = "ROUNDS".into();
            pack.game.folder = "rounds".into();
            fs::write(
                root.join("steamapps/appmanifest_1557740.acf"),
                r#""AppState" { "buildid" "999999999" }"#,
            )
            .unwrap();
            let snapshot = capture(&policy, &game, &pack, false).unwrap();
            let config = game.path.join("BepInEx/config/example.cfg");
            fs::write(&config, b"current config remains").unwrap();
            let active = game.path.join("BepInEx/plugins/Canna/active.dll");
            fs::write(&active, b"current plugin remains").unwrap();
            let result = restore(
                &policy,
                &game,
                &snapshot,
                "",
                crate::runtime::InstallOptions {
                    rebound_enabled: true,
                },
                &|_| {},
            );
            assert!(
                result.is_err(),
                "Missing public game assemblies must block preflight"
            );
            assert_eq!(fs::read(&config).unwrap(), b"current config remains");
            assert_eq!(fs::read(&active).unwrap(), b"current plugin remains");
            assert!(!game.path.join(".canna-config-recovery-stage").exists());
            assert!(!game.path.join(".canna-config-recovery-previous").exists());
        });
    }
    #[test]
    fn failed_capture_cleans_staging_and_budget_validation_is_explicit() {
        fixture(|_, game, mut pack, mut policy| {
            pack.mods[0].local_file = format!("{}.dll", "b".repeat(64));
            pack.mods[0].sha256 = "b".repeat(64);
            assert!(capture(&policy, &game, &pack, false).is_err());
            assert!(list(&policy, game.app_id).unwrap().is_empty());
            assert_eq!(
                fs::read_dir(game_store(&policy, game.app_id))
                    .unwrap()
                    .count(),
                0
            );
            policy.budget_gb = 0;
            assert!(policy.save().is_err());
            policy.budget_gb = 1;
            policy.directory = policy.directory.join("..").join("outside");
            assert!(policy.save().is_err());
        });
    }
    #[test]
    fn config_recovery_validates_paths_before_installation() {
        fixture(|_, game, pack, policy| {
            let mut snapshot = capture(&policy, &game, &pack, false).unwrap();
            snapshot.configs[0].0 = "../../outside.cfg".into();
            assert!(restore(&policy, &game, &snapshot, "", Default::default(), &|_| {}).is_err());
            assert_eq!(
                fs::read(game.path.join("saves/world.bin")).unwrap(),
                b"world remains untouched"
            );
        });
    }
    #[test]
    fn snapshot_ids_cannot_escape() {
        for id in ["../outside", "", "a/b", "a\\b", ".."] {
            assert!(!safe_id(id));
        }
    }
    #[test]
    fn verified_copy_rejects_corruption() {
        let root = std::env::temp_dir().join(format!("canna-recovery-test-{}", time()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("input"), b"corrupt").unwrap();
        assert!(copy_verified(&root.join("input"), &root.join("output"), &"a".repeat(64)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
