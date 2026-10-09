//! Reversible removal of Canna-owned Unity runtime files from the loader search paths.
//! Unknown or changed loader files remain in place, with Doorstop disabled.
use crate::{model::InstalledGame, runtime};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const STATE: &str = ".canna-runtime/state.json";
const MAX_STATE: u64 = 2 * 1024 * 1024;
const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_TREE: u64 = 512 * 1024 * 1024;
const MANAGED: [&str; 4] = [
    "BepInEx/plugins/Canna",
    "BepInEx/patchers/Canna",
    "BepInEx/plugins/Canna.previous",
    "BepInEx/patchers/Canna.previous",
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileReceipt {
    path: String,
    sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Parked {
    source: String,
    #[serde(default)]
    external: bool,
    parked: String,
    directory: bool,
    sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    app_id: u32,
    owned_loader: Vec<FileReceipt>,
    inactive: Vec<Parked>,
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.chars().any(|c| {
                    c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                })
        })
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn loader_path(path: &str, app_id: u32) -> bool {
    // Configs are user state; plugin/patcher namespaces are handled as complete trees.
    safe_relative(path)
        && (path.starts_with("BepInEx/core/")
            || path.starts_with("BepInEx/unhollowed/")
            || path.starts_with("BepInEx/interop/")
            || [
                "winhttp.dll",
                "version.dll",
                ".doorstop_version",
                "changelog.txt",
            ]
            .contains(&path)
            || app_id == 1557740 && path == "corlibs/mscorlib.dll")
}
fn checked_path(root: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(safe_relative(relative), "Unsafe Canna runtime receipt path");
    let path = root.join(relative);
    runtime::no_links(&path)?;
    Ok(path)
}
fn parked_path(game: &InstalledGame, row: &Parked) -> Result<PathBuf> {
    let external = checked_path(&crate::runtime_cache::root(game), &row.parked)?;
    let legacy = checked_path(&game.path, &row.parked)?;
    // A write-ahead migration can be interrupted before its move. Both locations
    // remain bounded and content is verified before any restoration.
    Ok(if row.external {
        if external.exists() || !legacy.exists() {
            external
        } else {
            legacy
        }
    } else {
        legacy
    })
}
fn content_hash(path: &Path, directory: bool) -> Result<String> {
    if directory {
        tree_hash(path)
    } else {
        Ok(digest_file(path)?.0)
    }
}
fn copy_tree(
    source: &Path,
    destination: &Path,
    depth: usize,
    count: &mut usize,
    bytes: &mut u64,
) -> Result<()> {
    *count += 1;
    ensure!(
        depth <= 64 && *count <= 20_000,
        "Runtime copy exceeded its tree limit"
    );
    runtime::no_links(source)?;
    runtime::no_links(destination)?;
    if source.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &destination.join(entry.file_name()),
                depth + 1,
                count,
                bytes,
            )?;
        }
    } else {
        let mut input = fs::File::open(source)?.take(MAX_FILE + 1);
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let size = std::io::copy(&mut input, &mut output)?;
        *bytes += size;
        ensure!(
            size <= MAX_FILE && *bytes <= MAX_TREE,
            "Runtime copy exceeded its limit"
        );
        output.sync_all()?;
    }
    Ok(())
}
fn remove_verified_tree(path: &Path, root: &Path) -> Result<()> {
    runtime::no_links(path)?;
    ensure!(
        path.canonicalize()?.starts_with(root),
        "Runtime cleanup escaped its verified source"
    );
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            remove_verified_tree(&entry?.path(), root)?;
        }
        fs::remove_dir(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn copy_relocate(source: &Path, destination: &Path, directory: bool, expected: &str) -> Result<()> {
    ensure!(
        !destination.exists(),
        "Recovery destination already exists; files preserved"
    );
    ensure!(
        content_hash(source, directory)? == expected,
        "Runtime source changed; files preserved"
    );
    copy_tree(source, destination, 0, &mut 0, &mut 0)?;
    ensure!(
        content_hash(destination, directory)? == expected
            && content_hash(source, directory)? == expected,
        "Runtime changed during relocation; both copies preserved"
    );
    let canonical = source.canonicalize()?;
    remove_verified_tree(source, &canonical)
}
fn relocate(source: &Path, destination: &Path, directory: bool, expected: &str) -> Result<()> {
    runtime::no_links(source)?;
    runtime::no_links(destination)?;
    ensure!(
        !destination.exists(),
        "Recovery destination already exists; files preserved"
    );
    ensure!(
        content_hash(source, directory)? == expected,
        "Runtime source changed; files preserved"
    );
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.raw_os_error() == Some(if cfg!(windows) { 17 } else { 18 }) => {
            copy_relocate(source, destination, directory, expected)
        }
        Err(error) => Err(error.into()),
    }
}
fn migrate_legacy(game: &InstalledGame, state: &mut State) -> Result<()> {
    for index in 0..state.inactive.len() {
        let row = state.inactive[index].clone();
        let legacy = checked_path(&game.path, &row.parked)?;
        if !legacy.exists() {
            continue;
        }
        ensure!(
            content_hash(&legacy, row.directory)? == row.sha256,
            "Legacy parked runtime changed; recovery files preserved"
        );
        let external = checked_path(&crate::runtime_cache::root(game), &row.parked)?;
        state.version = 2;
        state.inactive[index].external = true;
        save(game, state)?;
        fs::create_dir_all(external.parent().context("Missing cache parent")?)?;
        if external.exists() {
            ensure!(
                content_hash(&external, row.directory)? == row.sha256,
                "Runtime recovery conflict; both copies preserved"
            );
            let canonical = legacy.canonicalize()?;
            remove_verified_tree(&legacy, &canonical)?;
        } else {
            relocate(&legacy, &external, row.directory, &row.sha256)?;
        }
    }
    // Older releases retained displaced generations after removing their active
    // receipt. Move those recovery-only bytes as well, without activating them.
    let legacy = checked_path(&game.path, ".canna-runtime/parked")?;
    if legacy.exists() {
        tree_hash(&legacy)?; // Bound and reject links before walking or copying.
        let external = checked_path(&crate::runtime_cache::root(game), ".canna-runtime/parked")?;
        fs::create_dir_all(&external)?;
        for entry in fs::read_dir(&legacy)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            ensure!(
                !name.is_empty()
                    && name.len() <= 64
                    && name.bytes().all(|b| b.is_ascii_digit() || b == b'-'),
                "Unexpected legacy recovery batch; files preserved"
            );
            merge_recovery(&entry.path(), &external.join(name))?;
        }
        fs::remove_dir(legacy)?;
    }
    Ok(())
}
fn merge_recovery(source: &Path, target: &Path) -> Result<()> {
    runtime::no_links(source)?;
    runtime::no_links(target)?;
    let directory = source.is_dir();
    let hash = content_hash(source, directory)?;
    if !target.exists() {
        return relocate(source, target, directory, &hash);
    }
    ensure!(
        directory == target.is_dir(),
        "Recovery conflict; both copies preserved"
    );
    if directory {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            ensure!(
                safe_relative(&name),
                "Unsafe recovery name; files preserved"
            );
            merge_recovery(&entry.path(), &target.join(name))?;
        }
        fs::remove_dir(source)?;
    } else {
        ensure!(
            digest_file(target)?.0 == hash,
            "Recovery copies differ; both files preserved"
        );
        fs::remove_file(source)?;
    }
    Ok(())
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    runtime::no_links(path)?;
    let metadata = fs::metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "Canna runtime file exceeds its safety limit"
    );
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut data)?;
    ensure!(
        data.len() as u64 <= limit,
        "Canna runtime file grew beyond its safety limit"
    );
    Ok(data)
}
fn digest_file(path: &Path) -> Result<(String, u64)> {
    runtime::no_links(path)?;
    let metadata = fs::metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_FILE,
        "Unsupported or oversized runtime file"
    );
    let mut stream = fs::File::open(path)?.take(MAX_FILE + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut length = 0u64;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        length += count as u64;
        ensure!(
            length <= MAX_FILE,
            "Runtime file grew beyond its safety limit"
        );
        hash.update(&buffer[..count]);
    }
    Ok((format!("{:x}", hash.finalize()), length))
}
fn tree_hash(root: &Path) -> Result<String> {
    fn visit(
        root: &Path,
        path: &Path,
        depth: usize,
        rows: &mut Vec<String>,
        bytes: &mut u64,
    ) -> Result<()> {
        ensure!(
            depth <= 64 && rows.len() < 20_000,
            "Managed runtime tree exceeds its safety limit"
        );
        runtime::no_links(path)?;
        let metadata = fs::symlink_metadata(path)?;
        let name = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if metadata.is_dir() {
            rows.push(format!("d\t{name}\n"));
            for item in fs::read_dir(path)? {
                visit(root, &item?.path(), depth + 1, rows, bytes)?;
            }
        } else {
            ensure!(metadata.is_file(), "Unsupported file in Canna runtime tree");
            let (hash, count) = digest_file(path)?;
            *bytes += count;
            ensure!(
                *bytes <= MAX_TREE,
                "Managed runtime tree exceeds its byte limit"
            );
            rows.push(format!("f\t{name}\t{hash}\n"));
        }
        Ok(())
    }
    let mut rows = Vec::new();
    visit(root, root, 0, &mut rows, &mut 0)?;
    rows.sort();
    Ok(format!("{:x}", Sha256::digest(rows.concat().as_bytes())))
}
fn validate(state: &State, app_id: u32) -> Result<()> {
    ensure!(
        matches!(state.version, 1 | 2) && state.app_id == app_id,
        "Canna runtime receipt belongs to another game or version"
    );
    ensure!(
        state.owned_loader.len() <= 4000 && state.inactive.len() <= 4004,
        "Too many Canna runtime receipt entries"
    );
    let mut owned = BTreeSet::new();
    for row in &state.owned_loader {
        ensure!(
            loader_path(&row.path, app_id)
                && hash_valid(&row.sha256)
                && owned.insert(row.path.to_ascii_lowercase()),
            "Invalid or duplicate Canna loader receipt"
        );
    }
    let mut sources = BTreeSet::new();
    let mut destinations = BTreeSet::new();
    for row in &state.inactive {
        let prefix = row
            .parked
            .strip_prefix(".canna-runtime/parked/")
            .context("Invalid parked runtime path")?;
        let (batch, source) = prefix
            .split_once('/')
            .context("Invalid parked runtime batch")?;
        ensure!(
            batch.len() <= 64
                && !batch.is_empty()
                && batch.bytes().all(|c| c.is_ascii_digit() || c == b'-')
                && source == row.source
                && safe_relative(&row.parked)
                && hash_valid(&row.sha256)
                && sources.insert(row.source.to_ascii_lowercase())
                && destinations.insert(row.parked.to_ascii_lowercase()),
            "Invalid parked runtime receipt"
        );
        if row.directory {
            ensure!(
                MANAGED.contains(&row.source.as_str()),
                "Unowned managed runtime directory"
            );
        } else {
            ensure!(
                state
                    .owned_loader
                    .iter()
                    .any(|owned| owned.path == row.source && owned.sha256 == row.sha256),
                "Parked runtime is not bound to an installation receipt"
            );
        }
    }
    Ok(())
}
fn load(game: &InstalledGame) -> Result<State> {
    let path = checked_path(&game.path, STATE)?;
    if !path.exists() {
        return Ok(State {
            version: 2,
            app_id: game.app_id,
            owned_loader: vec![],
            inactive: vec![],
        });
    }
    let state: State = serde_json::from_slice(&read_bounded(&path, MAX_STATE)?)
        .context("Canna runtime receipt is damaged; files were preserved")?;
    validate(&state, game.app_id)?;
    for row in &state.inactive {
        parked_path(game, row)?;
        checked_path(&game.path, &row.source)?;
    }
    Ok(state)
}
fn save(game: &InstalledGame, state: &State) -> Result<()> {
    validate(state, game.app_id)?;
    let path = checked_path(&game.path, STATE)?;
    let data = serde_json::to_vec_pretty(state)?;
    ensure!(
        data.len() as u64 <= MAX_STATE,
        "Canna runtime receipt exceeds its byte limit"
    );
    let parent = path.parent().context("Missing receipt parent")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!("state-{}.pending", batch()));
    runtime::no_links(&temp)?;
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&data)?;
        file.sync_all()?;
        runtime::no_links(&path)?;
        fs::rename(&temp, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
pub(crate) fn replace_file(path: &Path, data: &[u8]) -> Result<()> {
    runtime::no_links(path)?;
    let temp = path.with_file_name(format!(".canna-ini-{}.pending", batch()));
    runtime::no_links(&temp)?;
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(data)?;
        file.sync_all()?;
        runtime::no_links(path)?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
fn batch() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
pub fn preflight_framework(game: &InstalledGame) -> Result<()> {
    load(game).map(|_| ())
}
pub fn record_framework(game: &InstalledGame, files: &[(PathBuf, Vec<u8>)]) -> Result<()> {
    let mut state = load(game)?;
    for (path, data) in files {
        let name = path.to_string_lossy().replace('\\', "/");
        if !loader_path(&name, game.app_id) {
            continue;
        }
        let actual = checked_path(&game.path, &name)?;
        let hash = format!("{:x}", Sha256::digest(data));
        ensure!(
            digest_file(&actual)?.0 == hash,
            "Installed loader changed before its receipt was saved"
        );
        state.owned_loader.retain(|row| row.path != name);
        state.inactive.retain(|row| row.source != name);
        state.owned_loader.push(FileReceipt {
            path: name,
            sha256: hash,
        });
    }
    save(game, &state)
}
pub fn has_parked_managed(game: &InstalledGame) -> Result<bool> {
    Ok(load(game)?
        .inactive
        .iter()
        .any(|row| row.directory && MANAGED[..2].contains(&row.source.as_str())))
}

type IniChange = (PathBuf, Vec<u8>, Vec<u8>);
fn disabled_ini(root: &Path, removable: &BTreeSet<String>) -> Result<Option<IniChange>> {
    let current = checked_path(root, "doorstop_config.ini")?;
    let backup = checked_path(root, "doorstop_config.canna-original.ini")?;
    if !current.exists() {
        // A legacy proxy without its switch cannot safely be labelled vanilla.
        for proxy in ["winhttp.dll", "version.dll"] {
            ensure!(
                !checked_path(root, proxy)?.exists() || removable.contains(proxy),
                "Loader proxy has no Doorstop configuration; files were preserved"
            );
        }
        return Ok(None);
    }
    let original = read_bounded(&current, 256 * 1024)?;
    let text = std::str::from_utf8(&original)
        .context("Doorstop configuration is not UTF-8; files were preserved")?;
    let mut disabled = runtime::doorstop_text(text, false)?;
    if backup.exists() {
        let data = read_bounded(&backup, 256 * 1024)?;
        let backup_text = std::str::from_utf8(&data)
            .context("Original Doorstop backup is not UTF-8; files were preserved")?;
        // Restore known original settings only when there are no unrelated manual changes.
        let baseline = runtime::doorstop_text(backup_text, false)?;
        if baseline == disabled {
            disabled = baseline;
        }
    }
    Ok(Some((current, original, disabled.into_bytes())))
}
fn rollback_moves(moved: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (source, parked) in moved.iter().rev() {
        runtime::no_links(source)?;
        runtime::no_links(parked)?;
        ensure!(
            !source.exists(),
            "A file appeared during rollback; both copies were preserved"
        );
        let directory = parked.is_dir();
        let hash = content_hash(parked, directory)?;
        relocate(parked, source, directory, &hash)
            .context("Could not roll back runtime relocation; recovery files remain parked")?;
    }
    Ok(())
}
pub fn restore(game: &InstalledGame) -> Result<String> {
    runtime::ensure_closed(game)?;
    let mut state = load(game)?;
    migrate_legacy(game, &mut state)?;
    let previous_state = state.clone();
    let mut planned = Vec::new();
    let mut preserved = 0usize;
    let id = batch();
    for source in MANAGED {
        let path = checked_path(&game.path, source)?;
        if !path.exists() {
            continue;
        }
        ensure!(
            path.is_dir(),
            "Canna runtime directory is an unexpected file; files were preserved"
        );
        planned.push(Parked {
            source: source.into(),
            external: true,
            parked: format!(".canna-runtime/parked/{id}/{source}"),
            directory: true,
            sha256: tree_hash(&path)?,
        });
    }
    for receipt in &state.owned_loader {
        let path = checked_path(&game.path, &receipt.path)?;
        if !path.exists() {
            continue;
        }
        if digest_file(&path)?.0 != receipt.sha256 {
            preserved += 1;
            continue;
        }
        planned.push(Parked {
            source: receipt.path.clone(),
            external: true,
            parked: format!(".canna-runtime/parked/{id}/{}", receipt.path),
            directory: false,
            sha256: receipt.sha256.clone(),
        });
    }
    let removable = planned.iter().map(|row| row.source.clone()).collect();
    let ini = disabled_ini(&game.path, &removable)?;
    // Validate every destination before moving anything. Keep all displaced generations as recovery files.
    for row in &planned {
        ensure!(
            !parked_path(game, row)?.exists(),
            "Runtime recovery destination already exists"
        );
    }
    runtime::ensure_closed(game)?;
    // Write-ahead receipt: after an interrupted move each source is either still active
    // or exists at its exact parked path. Subsequent restore/resume reconciles both.
    for row in &planned {
        state.inactive.retain(|old| old.source != row.source);
        state.inactive.push(row.clone());
    }
    if !planned.is_empty() {
        save(game, &state)?;
    }
    let mut moved = Vec::new();
    let mut ini_written = false;
    let result = (|| -> Result<()> {
        for row in &planned {
            let source = checked_path(&game.path, &row.source)?;
            let parked = parked_path(game, row)?;
            ensure!(
                (if row.directory {
                    tree_hash(&source)?
                } else {
                    digest_file(&source)?.0
                }) == row.sha256,
                "Runtime files changed during cleanup; retry when idle"
            );
            fs::create_dir_all(parked.parent().context("Missing parked parent")?)?;
            relocate(&source, &parked, row.directory, &row.sha256)?;
            moved.push((source, parked));
        }
        if let Some((path, original, disabled)) = &ini {
            ensure!(
                read_bounded(path, 256 * 1024)? == *original,
                "Doorstop configuration changed during cleanup"
            );
            replace_file(path, disabled)?;
            ini_written = true;
        }
        save(game, &state)?;
        Ok(())
    })();
    if let Err(error) = result {
        let rollback = rollback_moves(&moved);
        if ini_written
            && let Some((path, original, disabled)) = &ini
            && read_bounded(path, 256 * 1024).ok().as_ref() == Some(disabled)
        {
            replace_file(path, original)
                .context("Could not restore the original Doorstop configuration")?;
        }
        rollback?;
        if !planned.is_empty() {
            save(game, &previous_state)?;
        }
        return Err(error);
    }
    Ok(format!(
        "Vanilla files restored: {} Canna runtime item(s) parked for reuse; Doorstop disabled. Manual files and {preserved} changed loader file(s) preserved.",
        planned.len()
    ))
}

pub fn resume(game: &InstalledGame, managed: bool) -> Result<()> {
    runtime::ensure_closed(game)?;
    let mut state = load(game)?;
    migrate_legacy(game, &mut state)?;
    let mut planned = Vec::new();
    let mut consumed = BTreeSet::new();
    for row in &state.inactive {
        if row.directory && !managed {
            continue;
        }
        // Interrupted previous generations are recovery-only; never activate them as current mods.
        if row.directory && row.source.ends_with(".previous") {
            continue;
        }
        let source = checked_path(&game.path, &row.source)?;
        if source.exists() {
            // Applying another pack wins over a parked old pack; leave its recovery copy intact.
            if row.directory {
                consumed.insert(row.source.clone());
                continue;
            }
            ensure!(
                digest_file(&source)?.0 == row.sha256,
                "A manual loader now occupies a parked Canna path; files were preserved"
            );
            consumed.insert(row.source.clone());
            continue;
        }
        let parked = parked_path(game, row)?;
        if row.directory {
            let manifest = parked.join("DuctTapePlusPlus/compatibility-manifest.json");
            runtime::no_links(&manifest)?;
            ensure!(
                !manifest.exists(),
                "Reapply the Bliss modpack to verify current Beta access before launching modded"
            );
            ensure!(
                tree_hash(&parked)? == row.sha256,
                "Parked Canna plugins changed; reapply the modpack instead"
            );
        } else {
            ensure!(
                digest_file(&parked)?.0 == row.sha256,
                "Parked Canna loader changed; recovery files were preserved"
            );
        }
        planned.push(row.clone());
        consumed.insert(row.source.clone());
    }
    runtime::ensure_closed(game)?;
    let mut moved = Vec::new();
    let result = (|| -> Result<()> {
        for row in &planned {
            let source = checked_path(&game.path, &row.source)?;
            let parked = parked_path(game, row)?;
            ensure!(
                !source.exists(),
                "Runtime destination changed during restoration"
            );
            ensure!(
                (if row.directory {
                    tree_hash(&parked)?
                } else {
                    digest_file(&parked)?.0
                }) == row.sha256,
                "Parked runtime changed during restoration; files were preserved"
            );
            fs::create_dir_all(source.parent().context("Missing runtime parent")?)?;
            relocate(&parked, &source, row.directory, &row.sha256)?;
            moved.push((parked, source));
        }
        state.inactive.retain(|row| !consumed.contains(&row.source));
        if !consumed.is_empty() {
            save(game, &state)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        rollback_moves(&moved)?;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_volume_copy_verifies_before_removal_and_rejects_changed_or_occupied_paths() {
        let f = Fixture::new();
        f.put("copy-source/a/mod.dll", b"fixture plugin");
        let source = f.0.path.join("copy-source");
        let target = crate::runtime_cache::root(&f.0).join("copied");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let hash = tree_hash(&source).unwrap();
        assert!(copy_relocate(&source, &target, true, &"0".repeat(64)).is_err());
        assert!(source.exists());
        assert!(!target.exists());
        copy_relocate(&source, &target, true, &hash).unwrap();
        assert!(!source.exists());
        assert_eq!(
            fs::read(target.join("a/mod.dll")).unwrap(),
            b"fixture plugin"
        );
        f.put("copy-source/a/mod.dll", b"new fixture");
        assert!(copy_relocate(&source, &target, true, &tree_hash(&source).unwrap()).is_err());
        assert_eq!(fs::read(source.join("a/mod.dll")).unwrap(), b"new fixture");
    }
    #[test]
    fn legacy_parked_receipts_migrate_outside_game_without_activating_plugins() {
        let f = Fixture::new();
        f.ini();
        f.put("BepInEx/plugins/Canna/mod.dll", b"legacy fixture");
        let source = "BepInEx/plugins/Canna";
        let relative = format!(".canna-runtime/parked/{}/{source}", batch());
        let row = Parked {
            source: source.into(),
            external: false,
            parked: relative.clone(),
            directory: true,
            sha256: tree_hash(&f.0.path.join(source)).unwrap(),
        };
        let destination = f.0.path.join(&relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::rename(f.0.path.join(source), &destination).unwrap();
        let state = State {
            version: 1,
            app_id: f.0.app_id,
            owned_loader: vec![],
            inactive: vec![row],
        };
        save(&f.0, &state).unwrap();
        restore(&f.0).unwrap();
        let migrated = load(&f.0).unwrap();
        assert!(migrated.inactive[0].external);
        assert!(!destination.exists());
        assert!(!f.0.path.join(source).exists());
        assert!(
            parked_path(&f.0, &migrated.inactive[0])
                .unwrap()
                .starts_with(crate::runtime_cache::root(&f.0))
        );
        resume(&f.0, true).unwrap();
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"legacy fixture");
    }
    struct Fixture(InstalledGame);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("canna-vanilla-{}", batch()));
            fs::create_dir(&path).unwrap();
            Self(InstalledGame {
                app_id: 1557740,
                name: "Temporary ROUNDS fixture".into(),
                path,
                loader: String::new(),
                plugins: 0,
                icon: None,
            })
        }
        fn put(&self, path: &str, data: &[u8]) {
            let target = self.0.path.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }
        fn read(&self, path: &str) -> Vec<u8> {
            fs::read(self.0.path.join(path)).unwrap()
        }
        fn ini(&self) {
            self.put("doorstop_config.ini", b"[UnityDoorstop]\nenabled=true\ntargetAssembly=BepInEx\\core\\BepInEx.Preloader.dll\n[Other]\nenabled=true\n");
        }
        fn framework(&self) -> Vec<(PathBuf, Vec<u8>)> {
            let files: Vec<(PathBuf, Vec<u8>)> = vec![
                ("winhttp.dll".into(), b"owned proxy".to_vec()),
                ("BepInEx/core/BepInEx.dll".into(), b"owned core".to_vec()),
                ("corlibs/mscorlib.dll".into(), b"owned corlib".to_vec()),
                (
                    "BepInEx/config/BepInEx.cfg".into(),
                    b"User = saved".to_vec(),
                ),
            ];
            for (path, bytes) in &files {
                self.put(&path.to_string_lossy(), bytes);
            }
            record_framework(&self.0, &files).unwrap();
            files
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Test fixtures are uniquely created in TEMP; production cleanup never recursively deletes trees.
            let _ = fs::remove_dir_all(&self.0.path);
            let _ = fs::remove_dir_all(crate::runtime_cache::root(&self.0));
        }
    }

    #[test]
    fn legacy_restore_parks_only_canna_trees_and_disables_loader_without_launching() {
        let f = Fixture::new();
        f.ini();
        f.put("winhttp.dll", b"unreceipted manual proxy");
        f.put("BepInEx/core/BepInEx.dll", b"unreceipted core");
        f.put("BepInEx/plugins/Canna/mod.dll", b"Canna mod");
        f.put("BepInEx/patchers/Canna/rebound.dll", b"Canna patcher");
        f.put("BepInEx/plugins/Manual/manual.dll", b"manual mod");
        f.put("BepInEx/config/user.cfg", b"User = custom");
        f.put(
            "ROUNDS_Data/Managed/Assembly-CSharp.dll",
            b"original game assembly",
        );
        let report = runtime::restore_vanilla(&f.0).unwrap();
        assert!(report.contains("2 Canna runtime"));
        assert!(!f.0.path.join("BepInEx/plugins/Canna").exists());
        assert!(!f.0.path.join("BepInEx/patchers/Canna").exists());
        assert_eq!(f.read("winhttp.dll"), b"unreceipted manual proxy");
        assert_eq!(f.read("BepInEx/plugins/Manual/manual.dll"), b"manual mod");
        assert_eq!(f.read("BepInEx/config/user.cfg"), b"User = custom");
        assert_eq!(
            f.read("ROUNDS_Data/Managed/Assembly-CSharp.dll"),
            b"original game assembly"
        );
        assert!(
            String::from_utf8(f.read("doorstop_config.ini"))
                .unwrap()
                .contains("[UnityDoorstop]\r\nenabled=false")
        );
        assert!(
            String::from_utf8(f.read("doorstop_config.ini"))
                .unwrap()
                .contains("[Other]\r\nenabled=true")
        );
        let state = load(&f.0).unwrap();
        assert_eq!(state.inactive.len(), 2);
        for row in &state.inactive {
            assert!(parked_path(&f.0, row).unwrap().is_dir());
        }
        assert!(has_parked_managed(&f.0).unwrap());
    }
    #[test]
    fn receipts_park_loader_and_corlib_and_resume_without_changing_configs() {
        let f = Fixture::new();
        f.ini();
        f.framework();
        f.put("BepInEx/plugins/Canna/mod.dll", b"pack one");
        restore(&f.0).unwrap();
        assert!(!f.0.path.join("winhttp.dll").exists());
        assert!(!f.0.path.join("corlibs/mscorlib.dll").exists());
        assert_eq!(f.read("BepInEx/config/BepInEx.cfg"), b"User = saved");
        resume(&f.0, false).unwrap();
        assert_eq!(f.read("winhttp.dll"), b"owned proxy");
        assert_eq!(f.read("corlibs/mscorlib.dll"), b"owned corlib");
        assert!(!f.0.path.join("BepInEx/plugins/Canna").exists());
        resume(&f.0, true).unwrap();
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"pack one");
        assert!(!has_parked_managed(&f.0).unwrap());
        assert!(
            String::from_utf8(f.read("doorstop_config.ini"))
                .unwrap()
                .contains("enabled=false")
        );
    }
    #[test]
    fn changed_owned_loader_and_manual_settings_are_preserved() {
        let f = Fixture::new();
        f.ini();
        f.framework();
        f.put(
            "doorstop_config.canna-original.ini",
            b"[UnityDoorstop]\nenabled=false\ntargetAssembly=old.dll\n",
        );
        f.put("winhttp.dll", b"manual replacement proxy");
        let report = restore(&f.0).unwrap();
        assert!(report.contains("1 changed loader"));
        assert_eq!(f.read("winhttp.dll"), b"manual replacement proxy");
        let ini = String::from_utf8(f.read("doorstop_config.ini")).unwrap();
        assert!(ini.contains("targetAssembly=BepInEx\\core\\BepInEx.Preloader.dll"));
        assert_eq!(
            f.read("doorstop_config.canna-original.ini"),
            b"[UnityDoorstop]\nenabled=false\ntargetAssembly=old.dll\n"
        );
    }
    #[test]
    fn repeated_restore_is_idempotent_and_new_pack_wins_over_old_recovery() {
        let f = Fixture::new();
        f.ini();
        f.put("BepInEx/plugins/Canna/mod.dll", b"old pack");
        restore(&f.0).unwrap();
        let old = load(&f.0).unwrap().inactive[0].parked.clone();
        let state = f.read(STATE);
        restore(&f.0).unwrap();
        assert_eq!(f.read(STATE), state);
        f.put("BepInEx/plugins/Canna/mod.dll", b"new pack");
        resume(&f.0, true).unwrap();
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"new pack");
        assert_eq!(
            fs::read(crate::runtime_cache::root(&f.0).join(old).join("mod.dll")).unwrap(),
            b"old pack"
        );
    }
    #[test]
    fn interrupted_previous_trees_are_preserved_but_never_resumed_as_current() {
        let f = Fixture::new();
        f.ini();
        f.put(
            "BepInEx/plugins/Canna.previous/old.dll",
            b"previous generation",
        );
        f.put(
            "BepInEx/patchers/Canna.previous/old.dll",
            b"previous patcher",
        );
        restore(&f.0).unwrap();
        resume(&f.0, true).unwrap();
        assert!(!f.0.path.join("BepInEx/plugins/Canna.previous").exists());
        let state = load(&f.0).unwrap();
        assert_eq!(state.inactive.len(), 2);
        assert_eq!(
            fs::read(
                parked_path(&f.0, &state.inactive[0])
                    .unwrap()
                    .join("old.dll")
            )
            .unwrap(),
            b"previous generation"
        );
    }
    #[test]
    fn parked_rebound_cannot_bypass_fresh_beta_preparation() {
        let f = Fixture::new();
        f.ini();
        f.put(
            "BepInEx/plugins/Canna/DuctTapePlusPlus/compatibility-manifest.json",
            b"{}",
        );
        restore(&f.0).unwrap();
        assert!(
            resume(&f.0, true)
                .unwrap_err()
                .to_string()
                .contains("Beta access")
        );
        assert!(!f.0.path.join("BepInEx/plugins/Canna").exists());
    }
    #[test]
    fn tampered_recovery_and_manual_destination_are_not_overwritten() {
        let f = Fixture::new();
        f.ini();
        f.framework();
        restore(&f.0).unwrap();
        let state = load(&f.0).unwrap();
        let row = state
            .inactive
            .iter()
            .find(|row| row.source == "winhttp.dll")
            .unwrap();
        fs::write(parked_path(&f.0, row).unwrap(), b"tampered recovery").unwrap();
        assert!(resume(&f.0, false).is_err());
        assert!(!f.0.path.join("BepInEx/core/BepInEx.dll").exists());
        f.put("winhttp.dll", b"manual new proxy");
        assert!(resume(&f.0, false).is_err());
        assert_eq!(f.read("winhttp.dll"), b"manual new proxy");
    }
    #[test]
    fn malicious_receipt_paths_and_oversized_receipts_do_not_touch_files() {
        let f = Fixture::new();
        f.ini();
        f.put("BepInEx/plugins/Canna/mod.dll", b"untouched");
        let before = f.read("doorstop_config.ini");
        let mut state = load(&f.0).unwrap();
        state.owned_loader.push(FileReceipt {
            path: "../outside.dll".into(),
            sha256: "0".repeat(64),
        });
        f.put(STATE, &serde_json::to_vec(&state).unwrap());
        assert!(restore(&f.0).is_err());
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"untouched");
        assert_eq!(f.read("doorstop_config.ini"), before);
        f.put(STATE, &vec![b' '; MAX_STATE as usize + 1]);
        assert!(restore(&f.0).is_err());
        assert_eq!(f.read("doorstop_config.ini"), before);
    }
    #[test]
    fn receipt_for_game_assembly_or_other_game_is_rejected() {
        let f = Fixture::new();
        f.ini();
        let mut state = load(&f.0).unwrap();
        state.owned_loader.push(FileReceipt {
            path: "ROUNDS_Data/Managed/Assembly-CSharp.dll".into(),
            sha256: "0".repeat(64),
        });
        assert!(validate(&state, f.0.app_id).is_err());
        state.owned_loader.clear();
        state.app_id = 1686940;
        f.put(STATE, &serde_json::to_vec(&state).unwrap());
        assert!(restore(&f.0).is_err());
    }
    #[test]
    fn damaged_ini_and_unknown_proxy_without_ini_fail_before_relocation() {
        let f = Fixture::new();
        f.put("BepInEx/plugins/Canna/mod.dll", b"untouched");
        f.put("winhttp.dll", b"manual proxy");
        assert!(restore(&f.0).is_err());
        f.put(
            "doorstop_config.ini",
            b"[UnityDoorstop]\ntargetAssembly=x\n",
        );
        assert!(restore(&f.0).is_err());
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"untouched");
    }
    #[test]
    fn receipted_proxy_can_be_parked_when_its_ini_was_removed() {
        let f = Fixture::new();
        f.framework();
        restore(&f.0).unwrap();
        assert!(!f.0.path.join("winhttp.dll").exists());
    }
    #[test]
    fn interrupted_write_ahead_relocation_can_resume_or_finish_cleanup() {
        let f = Fixture::new();
        f.ini();
        f.framework();
        f.put("BepInEx/plugins/Canna/mod.dll", b"current pack");
        let mut state = load(&f.0).unwrap();
        let id = batch();
        for source in ["winhttp.dll", "BepInEx/plugins/Canna"] {
            let directory = source.starts_with("BepInEx/plugins");
            let path = f.0.path.join(source);
            state.inactive.push(Parked {
                source: source.into(),
                external: true,
                parked: format!(".canna-runtime/parked/{id}/{source}"),
                directory,
                sha256: if directory {
                    tree_hash(&path).unwrap()
                } else {
                    digest_file(&path).unwrap().0
                },
            });
        }
        save(&f.0, &state).unwrap();
        let row = &state.inactive[0];
        let parked = parked_path(&f.0, row).unwrap();
        fs::create_dir_all(parked.parent().unwrap()).unwrap();
        fs::rename(f.0.path.join(&row.source), &parked).unwrap();
        // Simulate a PC restart between the first and second rename: one source is parked,
        // one remains active, and both are recoverable using the persisted receipt.
        resume(&f.0, true).unwrap();
        assert_eq!(f.read("winhttp.dll"), b"owned proxy");
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"current pack");
        assert!(load(&f.0).unwrap().inactive.is_empty());
        restore(&f.0).unwrap();
        assert!(!f.0.path.join("winhttp.dll").exists());
        assert!(has_parked_managed(&f.0).unwrap());
    }
    #[cfg(windows)]
    #[test]
    fn ini_replace_failure_rolls_back_relocations_without_truncating_the_original() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        f.ini();
        f.put("BepInEx/plugins/Canna/mod.dll", b"preserved pack");
        let original = f.read("doorstop_config.ini");
        // Permit read-only preflight, but deny replacement of this fixture INI.
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(f.0.path.join("doorstop_config.ini"))
            .unwrap();
        assert!(restore(&f.0).is_err());
        assert_eq!(f.read("doorstop_config.ini"), original);
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"preserved pack");
        assert!(load(&f.0).unwrap().inactive.is_empty());
        drop(lock);
        restore(&f.0).unwrap();
        assert!(!f.0.path.join("BepInEx/plugins/Canna").exists());
    }
    #[cfg(windows)]
    #[test]
    fn directly_started_fixture_process_blocks_vanilla_restore_without_mutation() {
        use std::os::windows::process::CommandExt;
        struct Dummy(std::process::Child);
        impl Drop for Dummy {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let f = Fixture::new();
        f.ini();
        f.put("BepInEx/plugins/Canna/mod.dll", b"untouched");
        let command = std::env::var_os("COMSPEC").unwrap();
        let dummy = f.0.path.join("canna-fixture-process.exe");
        fs::copy(command, &dummy).unwrap();
        // Only a temporary copy of cmd.exe is launched. It holds a piped stdin open;
        // no Steam/game process is launched or stopped by this integration fixture.
        let mut process = Dummy(
            std::process::Command::new(&dummy)
                .args(["/D", "/Q", "/K"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x08000000)
                .spawn()
                .unwrap(),
        );
        assert!(process.0.try_wait().unwrap().is_none());
        let original = f.read("doorstop_config.ini");
        assert!(
            runtime::restore_vanilla(&f.0)
                .unwrap_err()
                .to_string()
                .contains("Close the game")
        );
        assert_eq!(f.read("doorstop_config.ini"), original);
        assert_eq!(f.read("BepInEx/plugins/Canna/mod.dll"), b"untouched");
        assert!(!f.0.path.join(STATE).exists());
        drop(process);
        runtime::restore_vanilla(&f.0).unwrap();
        assert!(!f.0.path.join("BepInEx/plugins/Canna").exists());
    }
    #[test]
    fn linked_managed_tree_or_state_is_rejected_before_disabling_loader() {
        let f = Fixture::new();
        f.ini();
        let target = f.0.path.join("manual");
        fs::create_dir(&target).unwrap();
        fs::create_dir_all(f.0.path.join("BepInEx/plugins")).unwrap();
        let link = f.0.path.join("BepInEx/plugins/Canna");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let status = std::process::Command::new("cmd.exe")
                .args(["/C", "mklink", "/J"])
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(target.to_string_lossy().replace('/', "\\"))
                .creation_flags(0x08000000)
                .status()
                .unwrap();
            assert!(status.success());
        }
        #[cfg(not(windows))]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let before = f.read("doorstop_config.ini");
        assert!(restore(&f.0).is_err());
        assert_eq!(f.read("doorstop_config.ini"), before);
        // Remove only the fixture link itself; never recurse into its target.
        #[cfg(windows)]
        fs::remove_dir(&link).unwrap();
        #[cfg(not(windows))]
        fs::remove_file(&link).unwrap();
        let state_link = f.0.path.join(".canna-runtime");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            assert!(
                std::process::Command::new("cmd.exe")
                    .args(["/C", "mklink", "/J"])
                    .arg(state_link.to_string_lossy().replace('/', "\\"))
                    .arg(target.to_string_lossy().replace('/', "\\"))
                    .creation_flags(0x08000000)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        #[cfg(not(windows))]
        std::os::unix::fs::symlink(&target, &state_link).unwrap();
        assert!(restore(&f.0).is_err());
        assert_eq!(f.read("doorstop_config.ini"), before);
        #[cfg(windows)]
        fs::remove_dir(&state_link).unwrap();
        #[cfg(not(windows))]
        fs::remove_file(&state_link).unwrap();
    }
}
