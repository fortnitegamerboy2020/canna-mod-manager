//! Opt-in, install-time ROUNDS translation. Mod code is never executed here.
use crate::{model::InstalledGame, modpacks::Modpack, runtime::PluginEntries};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const SUPPORT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ducttape-support.zip"));
const PROTOCOL: &str = "canna.ducttape++/1";
const PROFILE: &str = "rounds-public-1.1.2";
const MANIFEST: &str = "DuctTapePlusPlus/compatibility-manifest.json";
const HELPER: &str = "helper/Canna.DuctTapePlusPlus.Translate.exe";
const MAX_FILES: usize = 4000;
const MAX_BYTES: usize = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct Translation {
    file: String,
    before_sha256: String,
    after_sha256: String,
    changes: Vec<String>,
}
#[derive(Deserialize)]
struct Replacement {
    identity: String,
    sha256: String,
    file: String,
    reason: String,
}
#[derive(Deserialize)]
struct Report {
    // Process status is supplied by native code, never trusted from report JSON.
    #[serde(skip)]
    exited_successfully: bool,
    ok: bool,
    required: bool,
    profile: String,
    protocol: String,
    game_sha256: String,
    manifest_sha256: String,
    fingerprint: String,
    translated: Vec<Translation>,
    replacements: Vec<Replacement>,
    warnings: Vec<String>,
    errors: Vec<String>,
}
#[derive(Deserialize, serde::Serialize)]
struct AssemblyRow {
    identity: String,
    sha256: String,
}
#[derive(Deserialize, serde::Serialize)]
struct FileRow {
    root: String,
    path: String,
    sha256: String,
}
#[derive(Deserialize, serde::Serialize)]
struct Manifest {
    protocol: String,
    profile: String,
    game_sha256: String,
    digest: String,
    assemblies: Vec<AssemblyRow>,
    files: Vec<FileRow>,
}
fn atom(value: &str) -> bool {
    !value.is_empty() && value.len() <= 2048 && !value.contains(['\r', '\n', '\t', '\0'])
}
fn lower_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
#[derive(Deserialize)]
struct PayloadIndex {
    protocol: String,
    payloads: Vec<Payload>,
}
#[derive(Deserialize)]
struct Payload {
    assembly: String,
    file: String,
    sha256: String,
}
type ReplacementRegistry = BTreeMap<String, (String, std::collections::BTreeSet<String>)>;
fn file_hashes(files: &[(PathBuf, Vec<u8>)]) -> Result<BTreeMap<PathBuf, String>> {
    let mut result = BTreeMap::new();
    let mut names = std::collections::BTreeSet::new();
    for (path, bytes) in files {
        let relative = path.to_string_lossy().replace('\\', "/");
        relative_path(&relative)?;
        ensure!(
            names.insert(relative.to_lowercase()),
            "Duplicate compatibility file path"
        );
        result.insert(path.clone(), digest(bytes));
    }
    Ok(result)
}
fn replacement_registry(support: &[(PathBuf, Vec<u8>)]) -> Result<ReplacementRegistry> {
    let find = |name: &str| -> Result<&[u8]> {
        Ok(&support
            .iter()
            .find(|(p, _)| p == Path::new(name))
            .with_context(|| format!("Missing pinned replacement registry: {name}"))?
            .1)
    };
    let index: PayloadIndex = serde_json::from_slice(find("payloads/index.json")?)?;
    ensure!(
        index.protocol == PROTOCOL && index.payloads.len() <= 32,
        "Invalid pinned replacement index"
    );
    let mut result = BTreeMap::new();
    for payload in index.payloads {
        ensure!(
            atom(&payload.assembly)
                && lower_hash(&payload.sha256)
                && payload.file == format!("{}.dll", payload.sha256),
            "Invalid pinned replacement entry"
        );
        ensure!(
            digest(find(&format!("payloads/{}", payload.file))?) == payload.sha256,
            "Pinned dependency checksum mismatch"
        );
        let mut hashes = std::collections::BTreeSet::new();
        hashes.insert(payload.sha256.clone());
        ensure!(
            result
                .insert(payload.assembly, (payload.sha256, hashes))
                .is_none(),
            "Duplicate pinned replacement assembly"
        );
    }
    let old = std::str::from_utf8(find("helper/old-libraries.tsv")?)?;
    ensure!(
        old.len() <= 1024 * 1024,
        "Pinned old library registry exceeds limits"
    );
    for line in old.lines().filter(|line| !line.is_empty()) {
        let fields = line.split('\t').collect::<Vec<_>>();
        ensure!(
            fields.len() >= 2 && lower_hash(fields[1]),
            "Invalid pinned old library checksum"
        );
        let name = Path::new(fields[0])
            .file_stem()
            .and_then(|v| v.to_str())
            .context("Invalid pinned old library name")?;
        if let Some((_, hashes)) = result.get_mut(name) {
            hashes.insert(fields[1].to_owned());
        }
    }
    for (assembly, filename) in [
        ("rounds-port.Runtime", "rounds-port.Runtime.dll"),
        (
            "Canna.DuctTapePlusPlus.NetworkGuard",
            "Canna.DuctTapePlusPlus.NetworkGuard.dll",
        ),
    ] {
        let hash = digest(find(&format!("runtime/{filename}"))?);
        let hashes = std::collections::BTreeSet::from([hash.clone()]);
        ensure!(
            result.insert(assembly.into(), (hash, hashes)).is_none(),
            "Duplicate pinned compatibility runtime identity"
        );
    }
    Ok(result)
}
fn ordinal(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}
fn canonical_manifest(manifest: &Manifest) -> String {
    let mut canonical = format!(
        "{}\n{}\n{}\n",
        manifest.protocol, manifest.profile, manifest.game_sha256
    );
    let mut assemblies = manifest.assemblies.iter().collect::<Vec<_>>();
    assemblies.sort_by(|a, b| ordinal(&a.identity, &b.identity));
    for row in assemblies {
        canonical.push_str(&format!("assembly\t{}\t{}\n", row.identity, row.sha256));
    }
    let mut files = manifest.files.iter().collect::<Vec<_>>();
    files.sort_by(|a, b| {
        ordinal(
            &format!("{}/{}", a.root, a.path),
            &format!("{}/{}", b.root, b.path),
        )
    });
    for row in files {
        canonical.push_str(&format!(
            "file\t{}/{}\t{}\n",
            row.root, row.path, row.sha256
        ));
    }
    canonical
}
fn validate_manifest(
    bytes: &[u8],
    game: &str,
    fingerprint: &str,
    output: &[(PathBuf, Vec<u8>)],
    configs: &[(PathBuf, Vec<u8>)],
) -> Result<Manifest> {
    file_hashes(output)?;
    file_hashes(configs)?;
    let manifest: Manifest =
        serde_json::from_slice(bytes).context("Invalid compatibility manifest JSON")?;
    ensure!(
        manifest.protocol == PROTOCOL
            && manifest.profile == PROFILE
            && manifest.game_sha256 == game
            && lower_hash(&manifest.game_sha256)
            && lower_hash(&manifest.digest)
            && manifest.assemblies.len() <= 4096
            && manifest.files.len() <= 16384,
        "Invalid compatibility manifest header or limits"
    );
    let mut identities = std::collections::BTreeSet::new();
    let mut simple = std::collections::BTreeSet::new();
    let mut by_hash = BTreeMap::new();
    for row in &manifest.assemblies {
        ensure!(
            atom(&row.identity)
                && lower_hash(&row.sha256)
                && identities.insert(row.identity.clone())
                && simple.insert(row.identity.split(',').next().unwrap().to_lowercase())
                && by_hash
                    .insert(row.sha256.clone(), row.identity.clone())
                    .is_none(),
            "Invalid or duplicate managed assembly identity"
        );
    }
    let mut file_keys = std::collections::BTreeSet::new();
    for row in &manifest.files {
        ensure!(
            atom(&row.path)
                && lower_hash(&row.sha256)
                && matches!(row.root.as_str(), "plugins" | "patchers" | "config")
                && !row.path.starts_with('/')
                && !row.path.contains('\\')
                && row
                    .path
                    .split('/')
                    .all(|p| !p.is_empty() && p != "." && p != "..")
                && file_keys.insert(format!("{}/{}", row.root, row.path).to_lowercase()),
            "Invalid or duplicate prepared content key"
        );
    }
    ensure!(
        digest(canonical_manifest(&manifest).as_bytes()) == manifest.digest
            && manifest.digest == fingerprint,
        "Compatibility manifest canonical digest mismatch"
    );
    let dlls = output
        .iter()
        .filter(|(p, _)| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")))
        .collect::<Vec<_>>();
    ensure!(
        dlls.len() == manifest.assemblies.len(),
        "Compatibility manifest assembly count mismatch"
    );
    let mut owners = BTreeMap::<PathBuf, Vec<String>>::new();
    let mut seen_hashes = std::collections::BTreeSet::new();
    for (path, bytes) in dlls {
        let hash = digest(bytes);
        let identity = by_hash
            .get(&hash)
            .context("Compatibility manifest does not bind an output DLL")?;
        ensure!(seen_hashes.insert(hash), "Duplicate output DLL content");
        owners
            .entry(path.parent().unwrap_or(Path::new("")).to_owned())
            .or_default()
            .push(identity.clone());
    }
    let mut actual = BTreeMap::new();
    for (path, bytes) in output {
        if path == Path::new(MANIFEST)
            || path == Path::new(".canna-ducttape-staging")
            || path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        {
            continue;
        }
        let mut owner = None;
        for directory in path.parent().unwrap_or(Path::new("")).ancestors() {
            if let Some(identities) = owners.get(directory) {
                let mut names = identities.clone();
                names.sort_by(|a, b| ordinal(a, b));
                let name = if names.len() == 1 {
                    names.remove(0)
                } else {
                    format!("assemblies-{}", digest(names.join("\n").as_bytes()))
                };
                owner = Some(format!(
                    "{}/{}",
                    name,
                    path.strip_prefix(directory)?
                        .to_string_lossy()
                        .replace('\\', "/")
                ));
                break;
            }
        }
        let key = format!(
            "plugins/{}",
            owner.unwrap_or_else(|| format!(
                "unowned/{}",
                path.to_string_lossy().replace('\\', "/")
            ))
        );
        ensure!(
            actual.insert(key, digest(bytes)).is_none(),
            "Duplicate canonical asset key"
        );
    }
    for (path, bytes) in configs {
        if path.to_string_lossy().eq_ignore_ascii_case("BepInEx.cfg") {
            continue;
        }
        ensure!(
            actual
                .insert(
                    format!("config/{}", path.to_string_lossy().replace('\\', "/")),
                    digest(bytes)
                )
                .is_none(),
            "Duplicate canonical config key"
        );
    }
    let declared = manifest
        .files
        .iter()
        .map(|r| (format!("{}/{}", r.root, r.path), r.sha256.clone()))
        .collect::<BTreeMap<_, _>>();
    ensure!(
        actual == declared,
        "Compatibility manifest does not bind the prepared assets and effective configs"
    );
    Ok(manifest)
}
pub(crate) struct Resolved {
    pub files: PluginEntries,
    pub pack: Modpack,
    pub game_sha256: String,
    pub config_sha256: Vec<(PathBuf, String)>,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn relative_path(value: &str) -> Result<PathBuf> {
    ensure!(
        atom(value)
            && !value.starts_with('/')
            && !value.contains(['\\', ':'])
            && !value.ends_with(['.', ' ']),
        "Invalid compatibility output path"
    );
    let path = PathBuf::from(value);
    ensure!(path.components().all(|part| matches!(part, std::path::Component::Normal(name) if !name.to_string_lossy().ends_with(['.', ' ']))), "Invalid compatibility output path");
    Ok(path)
}
pub(crate) fn game_hash(game: &InstalledGame) -> Result<String> {
    let path = game.path.join("ROUNDS_Data/Managed/Assembly-CSharp.dll");
    crate::runtime::no_links(&path)?;
    ensure!(
        fs::metadata(&path)?.is_file() && fs::metadata(&path)?.len() <= 64 * 1024 * 1024,
        "Invalid ROUNDS game assembly"
    );
    Ok(digest(&fs::read(path)?))
}
pub(crate) fn current_configs(game: &InstalledGame) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    read_tree(&game.path.join("BepInEx/config"))
}
pub(crate) fn configuration_hashes(game: &InstalledGame) -> Result<Vec<(PathBuf, String)>> {
    Ok(current_configs(game)?
        .into_iter()
        .filter(|(path, _)| !path.to_string_lossy().eq_ignore_ascii_case("BepInEx.cfg"))
        .map(|(path, bytes)| (path, digest(&bytes)))
        .collect())
}
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Result<Self> {
        let base = std::env::temp_dir();
        crate::runtime::no_links(&base)?;
        let name = format!(
            "canna-ducttape-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        );
        let root = base.join(name);
        fs::create_dir(&root)?;
        crate::runtime::no_links(&root)?;
        Ok(Self(root))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        fn safe_tree(root: &Path) -> Result<()> {
            let mut pending = vec![root.to_owned()];
            let mut entries = 0usize;
            while let Some(path) = pending.pop() {
                crate::runtime::no_links(&path)?;
                let metadata = fs::symlink_metadata(&path)?;
                ensure!(
                    metadata.is_file() || metadata.is_dir(),
                    "Unexpected staging file type"
                );
                if metadata.is_dir() {
                    for entry in fs::read_dir(path)? {
                        entries += 1;
                        ensure!(
                            entries <= 100_000,
                            "Staging cleanup exceeds traversal limits"
                        );
                        pending.push(entry?.path());
                    }
                }
            }
            Ok(())
        }
        if let (Ok(base), Ok(root)) = (
            fs::canonicalize(std::env::temp_dir()),
            fs::canonicalize(&self.0),
        ) && root.starts_with(&base)
            && root != base
            && root
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("canna-ducttape-"))
            && safe_tree(&root).is_ok()
        {
            let _ = fs::remove_dir_all(root);
        }
    }
}
fn write_files(root: &Path, entries: &[(PathBuf, Vec<u8>)]) -> Result<()> {
    for (relative, bytes) in entries {
        let path = relative_path(&relative.to_string_lossy().replace('\\', "/"))?;
        let destination = root.join(path);
        crate::runtime::no_links(&destination)?;
        fs::create_dir_all(destination.parent().context("Missing staging parent")?)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        file.write_all(bytes)?;
    }
    Ok(())
}
fn read_tree(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    fn visit(
        root: &Path,
        path: &Path,
        files: &mut Vec<(PathBuf, Vec<u8>)>,
        total: &mut usize,
    ) -> Result<()> {
        crate::runtime::no_links(path)?;
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            crate::runtime::no_links(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                visit(root, &path, files, total)?;
            } else {
                ensure!(metadata.is_file(), "Unexpected compatibility output file");
                let relative = path.strip_prefix(root)?.to_owned();
                relative_path(&relative.to_string_lossy().replace('\\', "/"))?;
                ensure!(
                    files.len() < MAX_FILES && metadata.len() <= MAX_BYTES as u64,
                    "Compatibility output exceeds limits"
                );
                *total = total
                    .checked_add(metadata.len() as usize)
                    .context("Compatibility size overflow")?;
                ensure!(*total <= MAX_BYTES, "Compatibility output exceeds limits");
                files.push((relative, fs::read(path)?));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    if root.exists() {
        visit(root, root, &mut files, &mut 0)?;
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}
fn run_helper(helper: &Path, request: &Path, report: &Path) -> Result<bool> {
    let mut command = Command::new(helper);
    command
        .args(["--request"])
        .arg(request)
        .arg("--report")
        .arg(report)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .context("Could not start the bundled Canna Rebound preflight helper")?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success() || report.is_file(),
                "Canna Rebound helper failed before producing a report"
            );
            return Ok(status.success());
        }
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Canna Rebound preflight timed out; the game was not changed");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn validate_report(
    report: &Report,
    game: &str,
    input: &[(PathBuf, Vec<u8>)],
    output: &[(PathBuf, Vec<u8>)],
    support: &[(PathBuf, Vec<u8>)],
    configs: &[(PathBuf, Vec<u8>)],
) -> Result<()> {
    ensure!(
        report.errors.len() <= 256 && report.errors.iter().all(|e| e.len() <= 2000),
        "Compatibility error report exceeds limits"
    );
    ensure!(
        report.ok && report.errors.is_empty(),
        "Canna Rebound cannot install this pack: {}",
        report.errors.join("; ")
    );
    ensure!(
        report.exited_successfully,
        "Canna Rebound helper exited unsuccessfully; the game was not changed"
    );
    ensure!(
        report.protocol == PROTOCOL && report.profile == PROFILE && report.game_sha256 == game,
        "Compatibility report does not match this ROUNDS build"
    );
    ensure!(
        valid_hash(&report.manifest_sha256) && valid_hash(&report.fingerprint),
        "Invalid compatibility manifest digest"
    );
    ensure!(
        report.translated.len() <= MAX_FILES
            && report.replacements.len() <= 256
            && report.warnings.len() <= 256,
        "Compatibility report exceeds limits"
    );
    ensure!(
        report.warnings.iter().all(|e| e.len() <= 2000),
        "Compatibility warning report exceeds limits"
    );
    let original = file_hashes(input)?;
    let final_hashes = file_hashes(output)?;
    let bundled = file_hashes(support)?;
    let manifest = output
        .iter()
        .find(|(p, _)| p == Path::new(MANIFEST))
        .context("Missing compatibility manifest")?;
    ensure!(
        digest(&manifest.1) == report.manifest_sha256,
        "Compatibility manifest checksum mismatch"
    );
    let manifest_data = validate_manifest(&manifest.1, game, &report.fingerprint, output, configs)?;
    let mut declared = BTreeMap::new();
    let mut reported_paths = std::collections::BTreeSet::new();
    for translated in &report.translated {
        let path = relative_path(&translated.file)?;
        ensure!(
            valid_hash(&translated.before_sha256)
                && valid_hash(&translated.after_sha256)
                && translated.changes.len() <= 1000
                && translated.changes.iter().all(|change| change.len() <= 2000)
                && reported_paths.insert(translated.file.to_lowercase()),
            "Invalid translation metadata"
        );
        ensure!(
            original
                .get(&path)
                .is_some_and(|h| h == &translated.before_sha256)
                || bundled.values().any(|h| h == &translated.before_sha256),
            "Translation input checksum mismatch for {}",
            translated.file
        );
        ensure!(
            final_hashes.get(&path) == Some(&translated.after_sha256),
            "Translation output checksum mismatch for {}",
            translated.file
        );
        ensure!(
            declared
                .insert(path, translated.after_sha256.clone())
                .is_none(),
            "Duplicate translation output"
        );
    }
    for replacement in &report.replacements {
        let path = relative_path(&replacement.file)?;
        ensure!(
            !replacement.identity.is_empty()
                && replacement.identity.len() <= 200
                && replacement.reason.len() <= 2000
                && valid_hash(&replacement.sha256)
                && reported_paths.insert(replacement.file.to_lowercase()),
            "Invalid replacement metadata"
        );
        ensure!(
            final_hashes.get(&path) == Some(&replacement.sha256)
                && bundled.values().any(|h| h == &replacement.sha256),
            "Replacement checksum mismatch for {}",
            replacement.identity
        );
        ensure!(manifest_data.assemblies.iter().any(|row|row.identity == replacement.identity && row.sha256 == replacement.sha256), "Replacement identity is not bound to the manifest");
        declared.insert(path, replacement.sha256.clone());
    }
    for (path, hash) in &final_hashes {
        if path == Path::new(MANIFEST) {
            continue;
        }
        ensure!(
            original.get(path) == Some(hash)
                || declared.get(path) == Some(hash)
                || bundled.values().any(|h| h == hash),
            "Unaccounted compatibility output: {}",
            path.display()
        );
    }
    // A removed input needs its exact trusted old SHA and the matching pinned
    // replacement assembly. A different replacement cannot justify its removal.
    let registry = if original.keys().any(|path| !final_hashes.contains_key(path)) {
        replacement_registry(support)?
    } else {
        BTreeMap::new()
    };
    for (path, hash) in &original {
        if !final_hashes.contains_key(path) {
            ensure!(
                registry
                    .iter()
                    .any(|(assembly, (pinned, known))| known.contains(hash)
                        && report
                            .replacements
                            .iter()
                            .any(|replacement| replacement.sha256 == *pinned
                                && replacement.identity.split(',').next()
                                    == Some(assembly.as_str()))),
                "Unsupported removed dependency: {}",
                path.display()
            );
        }
    }
    Ok(())
}
fn cache_resolved(pack: &Modpack, files: &PluginEntries) -> Result<Modpack> {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (route, entries) in [
        ("plugins", &files.plugins),
        ("patchers", &files.patchers),
        ("config", &files.configs),
    ] {
        for (path, bytes) in entries {
            archive.start_file(
                format!(
                    "BepInEx/{route}/{}",
                    path.to_string_lossy().replace('\\', "/")
                ),
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )?;
            archive.write_all(bytes)?;
        }
    }
    let bytes = archive.finish()?.into_inner();
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "Resolved preview archive exceeds cache limits"
    );
    let hash = digest(&bytes);
    let name = format!("{hash}.zip");
    let directory = crate::modpacks::local_directory();
    crate::runtime::no_links(&directory)?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(&name);
    crate::runtime::no_links(&path)?;
    if path.exists() {
        ensure!(
            digest(&fs::read(&path)?) == hash,
            "Resolved archive cache checksum mismatch"
        );
    } else {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    let mut resolved = pack.clone();
    resolved.mods = vec![crate::model::ModInfo {
        enabled: true,
        name: "Canna Rebound resolved public ROUNDS pack".into(),
        version: PROFILE.into(),
        content_type: "mod".into(),
        description: format!(
            "Prepared from {} without changing its saved selections.",
            pack.name
        ),
        file: format!("Mods/{name}"),
        sha256: hash,
        local_file: name,
        dependencies: Vec::new(),
        provenance: serde_json::json!({"required_game_branch":"public","compatibility_profile":PROFILE,"source_archives":pack.mods.iter().filter(|m|m.enabled).map(|m|serde_json::json!({"file":m.file,"sha256":m.sha256})).collect::<Vec<_>>() }),
    }];
    resolved.validate()?;
    Ok(resolved)
}
pub(crate) fn resolve(
    game: &InstalledGame,
    pack: &Modpack,
    mut files: PluginEntries,
    framework: Option<&[u8]>,
    config_override: Option<&[(PathBuf, Vec<u8>)]>,
    progress: &dyn Fn(&str),
) -> Result<Resolved> {
    let input_count = files.plugins.len() + files.patchers.len() + files.configs.len();
    let input_size = files
        .plugins
        .iter()
        .chain(&files.patchers)
        .chain(&files.configs)
        .try_fold(0usize, |n, (_, b)| {
            n.checked_add(b.len())
                .context("Compatibility input size overflow")
        })?;
    ensure!(
        input_count <= MAX_FILES && input_size <= MAX_BYTES,
        "Compatibility inputs exceed preview limits"
    );
    ensure!(
        cfg!(canna_ducttape_preview) && !SUPPORT.is_empty(),
        "Canna Rebound support is not included in this build"
    );
    ensure!(
        files.patchers.is_empty(),
        "Canna Rebound preview cannot combine preloader patchers. Disable the existing DuctTape/preloader package in this pack before retrying."
    );
    let enabled: Vec<_> = pack.mods.iter().filter(|m| m.enabled).collect();
    ensure!(
        !(enabled.iter().any(|m| m.name == "HollowPurple")
            && enabled.iter().any(|m| m.name == "HollowPurple Fixed")),
        "Disable original HollowPurple before using HollowPurple Fixed; their plugin identities conflict"
    );
    for (index, item) in pack
        .mods
        .iter()
        .enumerate()
        .filter(|(_, item)| item.enabled)
    {
        if crate::game_compat::rounds_requirement(item)
            && item.provenance["source_url"].as_str().is_some_and(|s| {
                s.trim_end_matches('/') == "https://thunderstore.io/c/rounds/p/flofl/HollowPurple"
            })
        {
            ensure!(
                files
                    .plugins
                    .iter()
                    .any(|(p, b)| p.starts_with(index.to_string())
                        && matches!(
                            digest(b).as_str(),
                            "4cdeaaea4a0b296f1e609b39535058de0afdfd29e5467cd18ab48156aa7fef32"
                                | "7a92423326722a445b29b5cf6d8e93ba38add6db1c6b8b8bc7be5a578eebb9e3"
                        )),
                "Original HollowPurple's legacy branch guard cannot be overridden for an unknown source hash"
            );
        }
    }
    progress("Preparing Canna Rebound support (preview)…");
    let workspace = Workspace::new()?;
    let bundle = crate::runtime::archive_files(SUPPORT)?;
    write_files(&workspace.0, &bundle)?;
    let helper = workspace.0.join(HELPER);
    let expected = bundle
        .iter()
        .find(|(p, _)| p == Path::new(HELPER))
        .context("Bundled compatibility helper missing")?;
    ensure!(
        digest(&fs::read(&helper)?) == digest(&expected.1),
        "Bundled compatibility helper checksum mismatch"
    );
    let plugins = workspace.0.join("plugins");
    fs::create_dir(&plugins)?;
    write_files(&plugins, &files.plugins)?;
    fs::write(plugins.join(".canna-ducttape-staging"), PROTOCOL)?;
    let config = workspace.0.join("config");
    fs::create_dir(&config)?;
    let existing = if let Some(configs) = config_override {
        file_hashes(configs)?;
        configs.to_vec()
    } else {
        current_configs(game)?
    };
    let config_sha256 = existing
        .iter()
        .map(|(p, b)| (p.clone(), digest(b)))
        .collect::<Vec<_>>();
    let mut effective_configs = existing.into_iter().collect::<BTreeMap<_, _>>();
    let mut defaults = BTreeMap::new();
    for (path, bytes) in &files.configs {
        if effective_configs.contains_key(path) {
            continue;
        }
        if let Some(previous) = defaults.insert(path.clone(), bytes.clone()) {
            ensure!(
                previous == *bytes,
                "Conflicting selected config defaults: {}",
                path.display()
            );
        }
    }
    effective_configs.extend(defaults);
    let effective_configs = effective_configs.into_iter().collect::<Vec<_>>();
    write_files(&config, &effective_configs)?;
    let core = if let Some(bytes) = framework {
        let core = workspace.0.join("core");
        fs::create_dir(&core)?;
        let entries = crate::runtime::archive_files(bytes)?
            .into_iter()
            .filter_map(|(p, b)| {
                let text = p.to_string_lossy().replace('\\', "/");
                let (_, relative) = text.split_once("BepInEx/core/")?;
                Some((PathBuf::from(relative), b))
            })
            .collect::<Vec<_>>();
        write_files(&core, &entries)?;
        core
    } else {
        game.path.join("BepInEx/core")
    };
    let source_hash = game_hash(game)?;
    let declared_dependencies = enabled
        .iter()
        .flat_map(|item| {
            item.dependencies.iter().cloned().chain(
                item.provenance["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned)),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(
        declared_dependencies.len() <= 4096 && declared_dependencies.iter().all(|name| atom(name)),
        "Declared dependency metadata exceeds preview limits"
    );
    let request = workspace.0.join("request.json");
    let report_path = workspace.0.join("report.json");
    fs::write(
        &request,
        serde_json::to_vec(
            &serde_json::json!({"game_root":game.path,"plugins":plugins,"core":core,"config":config,"declared_dependencies":declared_dependencies}),
        )?,
    )?;
    progress("Checking and translating every selected ROUNDS plugin…");
    let exited_successfully = run_helper(&helper, &request, &report_path)?;
    crate::runtime::no_links(&report_path)?;
    ensure!(
        fs::metadata(&report_path)?.len() <= 4 * 1024 * 1024,
        "Compatibility report is oversized"
    );
    let mut report: Report = serde_json::from_slice(&fs::read(&report_path)?)
        .context("Invalid Canna Rebound preflight report")?;
    report.exited_successfully = exited_successfully;
    let output = read_tree(&plugins)?
        .into_iter()
        .filter(|(p, _)| p != Path::new(".canna-ducttape-staging"))
        .collect::<Vec<_>>();
    validate_report(
        &report,
        &source_hash,
        &files.plugins,
        &output,
        &bundle,
        &effective_configs,
    )?;
    ensure!(
        game_hash(game)? == source_hash,
        "ROUNDS changed during compatibility preflight; retry after Steam finishes updating"
    );
    for warning in &report.warnings {
        progress(&format!("Canna Rebound preview: {warning}"));
    }
    if !report.required {
        progress("Canna Rebound preview: selected plugins passed the public ROUNDS API preflight.");
    }
    files.plugins = output;
    let resolved = cache_resolved(pack, &files)?;
    crate::game_compat::check_pack(game, &resolved)?;
    Ok(Resolved {
        files,
        pack: resolved,
        game_sha256: source_hash,
        config_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "LegacyGameplay, Version=1.0.0.0, Culture=neutral, PublicKeyToken=null";
    type Files = Vec<(PathBuf, Vec<u8>)>;
    #[cfg(canna_ducttape_preview)]
    #[test]
    fn bundled_registry_pins_all_dependencies_and_known_legacy_inputs() {
        let entries = crate::runtime::archive_files(SUPPORT).unwrap();
        let registry = replacement_registry(&entries).unwrap();
        for (assembly, original) in [
            (
                "UnboundLib",
                "ffdb9d0b6482477cbc94d4866023ac84d4956f6a517fcf40510ad4f52a815b29",
            ),
            (
                "MMHOOK_Assembly-CSharp",
                "febd1c9c7b7e264638bf83078341e707c45a11971d92ebb51b52fac5f1af2b8d",
            ),
            (
                "RoundsWithFriends",
                "d1c6440919412e75ebade7181e509fbd04a34b35ed269ef85b8ead6eb0cfde69",
            ),
        ] {
            assert!(registry[assembly].1.contains(original));
        }
        assert!(registry.contains_key("ModdingUtils"));
        assert!(registry.contains_key("RarityLib"));
        assert!(registry.contains_key("Octokit"));
    }
    #[cfg(canna_ducttape_preview)]
    #[test]
    #[ignore = "Requires explicit prerecorded DLL and isolated public ROUNDS reference copy; never installs or launches a game"]
    fn prerecorded_dependency_free_legacy_roundtrip() {
        let fixture_path = |name: &str| {
            let path = fs::canonicalize(
                std::env::var_os(name).expect("Explicit prerecorded fixture path required"),
            )
            .unwrap();
            let target = fs::canonicalize("target").unwrap();
            assert!(
                path.starts_with(target),
                "Fixture must be inside workspace target, never the installed game"
            );
            path
        };
        let input_path = fixture_path("CANNA_DUCTTAPE_FIXTURE_DLL");
        let game_copy = fixture_path("CANNA_DUCTTAPE_FIXTURE_GAME");
        let original = fs::read(&input_path).unwrap();
        let original_hash = digest(&original);
        let workspace = Workspace::new().unwrap();
        crate::modpacks::with_test_root(workspace.0.clone(), || {
            let game_root = workspace.0.join("steamapps/common/ROUNDS");
            fs::create_dir_all(&game_root).unwrap();
            write_files(
                &game_root.join("ROUNDS_Data/Managed"),
                &read_tree(&game_copy.join("ROUNDS_Data/Managed")).unwrap(),
            )
            .unwrap();
            write_files(
                &game_root.join("BepInEx/core"),
                &read_tree(&game_copy.join("BepInEx/core")).unwrap(),
            )
            .unwrap();
            fs::write(
                workspace.0.join("steamapps/appmanifest_1557740.acf"),
                "\"AppState\" { \"buildid\" \"fixture\" }",
            )
            .unwrap();
            let game = InstalledGame {
                app_id: 1557740,
                name: "Pristine temporary ROUNDS references".into(),
                path: game_root.clone(),
                loader: String::new(),
                plugins: 0,
                icon: None,
            };
            let source = crate::cache::Source {
                owner: "fixture".into(),
                repository: "fixture".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            };
            let info: crate::model::GameInfo = serde_json::from_value(
                serde_json::json!({"app_id":1557740,"name":"ROUNDS","folder":"rounds"}),
            )
            .unwrap();
            let mut item: crate::model::ModInfo = serde_json::from_value(serde_json::json!({"name":"Dependency-free legacy fixture","version":"1.0","file":"Mods/DependencyFreeLegacy.dll","sha256":original_hash})).unwrap();
            item.local_file = crate::modpacks::add_local(&input_path).unwrap().local_file;
            let pack = Modpack::create(
                "Custom fixture name".into(),
                String::new(),
                &info,
                source,
                vec![item],
            );
            let before = serde_json::to_vec(&pack).unwrap();
            let off =
                crate::runtime::prepare_install(&game, &pack, "", Default::default(), &|_| {})
                    .unwrap();
            assert_eq!(serde_json::to_vec(off.effective_pack()).unwrap(), before);
            fs::create_dir_all(game_root.join("BepInEx/config")).unwrap();
            fs::write(
                game_root.join("BepInEx/config/fixture.cfg"),
                b"Choice = current\n",
            )
            .unwrap();
            let snapshot_configs = vec![(
                PathBuf::from("fixture.cfg"),
                b"Choice = snapshot\n".to_vec(),
            )];
            let enabled = crate::runtime::prepare_install_with_configs(
                &game,
                &pack,
                "",
                crate::runtime::InstallOptions {
                    rebound_enabled: true,
                },
                Some(&snapshot_configs),
                &|_| {},
            )
            .unwrap();
            assert_eq!(
                enabled.effective_pack().mods[0].provenance["compatibility_profile"],
                PROFILE
            );
            let resolved_bytes = fs::read(
                crate::modpacks::local_directory()
                    .join(&enabled.effective_pack().mods[0].local_file),
            )
            .unwrap();
            let enabled_files = crate::runtime::archive_files(&resolved_bytes).unwrap();
            let manifest: Manifest = serde_json::from_slice(
                &enabled_files
                    .iter()
                    .find(|(path, _)| path == &Path::new("BepInEx/plugins").join(MANIFEST))
                    .unwrap()
                    .1,
            )
            .unwrap();
            assert!(manifest.files.iter().any(|file| file.root == "config"
                && file.path == "fixture.cfg"
                && file.sha256 == digest(b"Choice = snapshot\n")));
            assert_eq!(
                fs::read(game_root.join("BepInEx/config/fixture.cfg")).unwrap(),
                b"Choice = current\n"
            );
            assert!(!game_root.join("BepInEx/plugins").exists());
            assert_eq!(serde_json::to_vec(&pack).unwrap(), before);
            let prepared = resolve(
                &game,
                &pack,
                PluginEntries {
                    plugins: vec![("0/DependencyFreeLegacy.dll".into(), original.clone())],
                    patchers: vec![],
                    configs: vec![],
                },
                None,
                None,
                &|_| {},
            )
            .unwrap();
            assert_ne!(
                digest(
                    &prepared
                        .files
                        .plugins
                        .iter()
                        .find(|(path, _)| path == Path::new("0/DependencyFreeLegacy.dll"))
                        .unwrap()
                        .1
                ),
                original_hash
            );
            assert!(prepared.files.plugins.iter().any(|(path, _)| path
                == Path::new("DuctTapePlusPlus/Canna.DuctTapePlusPlus.NetworkGuard.dll")));
            assert_eq!(serde_json::to_vec(&pack).unwrap(), before);
            assert!(!game_root.join("BepInEx/plugins").exists());
            assert_eq!(fs::read(&input_path).unwrap(), original);
            let translated_hash = digest(
                &prepared
                    .files
                    .plugins
                    .iter()
                    .find(|(path, _)| path == Path::new("0/DependencyFreeLegacy.dll"))
                    .unwrap()
                    .1,
            );
            let restore_files = PluginEntries {
                plugins: prepared
                    .files
                    .plugins
                    .into_iter()
                    .map(|(path, bytes)| (PathBuf::from("0").join(path), bytes))
                    .collect(),
                patchers: vec![],
                configs: vec![],
            };
            let restored =
                resolve(&game, &prepared.pack, restore_files, None, None, &|_| {}).unwrap();
            assert_eq!(
                restored
                    .files
                    .plugins
                    .iter()
                    .filter(|(path, _)| path
                        .file_name()
                        .is_some_and(|name| name == "Canna.DuctTapePlusPlus.NetworkGuard.dll"))
                    .count(),
                1
            );
            assert_eq!(
                digest(
                    &restored
                        .files
                        .plugins
                        .iter()
                        .find(|(path, _)| path == Path::new("0/0/DependencyFreeLegacy.dll"))
                        .unwrap()
                        .1
                ),
                translated_hash
            );
            assert!(!game_root.join("BepInEx/plugins").exists());
        });
    }
    #[cfg(canna_ducttape_preview)]
    #[test]
    fn bundled_helper_failure_remains_blocking_and_cannot_supply_native_exit_status() {
        let workspace = Workspace::new().unwrap();
        let support = workspace.0.join("support");
        write_files(&support, &crate::runtime::archive_files(SUPPORT).unwrap()).unwrap();
        let game = workspace.0.join("fake-game");
        let plugins = workspace.0.join("plugins");
        let core = workspace.0.join("core");
        for path in [&game, &plugins, &core] {
            fs::create_dir(path).unwrap();
        }
        fs::write(plugins.join(".canna-ducttape-staging"), PROTOCOL).unwrap();
        for name in ["BepInEx.dll", "0Harmony.dll"] {
            fs::copy(support.join("helper").join(name), core.join(name)).unwrap();
        }
        let request = workspace.0.join("request.json");
        fs::write(&request, serde_json::to_vec(&serde_json::json!({"game_root":game,"plugins":plugins,"core":core,"declared_dependencies":[]})).unwrap()).unwrap();
        let report = workspace.0.join("report.json");
        assert!(!run_helper(&support.join(HELPER), &request, &report).unwrap());
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert_eq!(value["ok"], false);
        assert!(!value["errors"].as_array().unwrap().is_empty());
        value["exited_successfully"] = true.into();
        let decoded: Report = serde_json::from_value(value).unwrap();
        assert!(!decoded.exited_successfully);
        assert_eq!(fs::read_dir(game).unwrap().count(), 0);
        assert_eq!(
            fs::read(plugins.join(".canna-ducttape-staging")).unwrap(),
            PROTOCOL.as_bytes()
        );
    }
    fn fixture() -> (Report, Files, Files, Files) {
        let input = vec![
            ("0/gameplay.dll".into(), b"original fixture".to_vec()),
            ("0/assets/map.bundle".into(), b"asset fixture".to_vec()),
        ];
        let output = vec![
            ("0/gameplay.dll".into(), b"translated fixture".to_vec()),
            input[1].clone(),
        ];
        let config = vec![
            ("gameplay.cfg".into(), b"effective user settings".to_vec()),
            ("BepInEx.cfg".into(), b"system settings".to_vec()),
        ];
        let mut manifest = Manifest {
            protocol: PROTOCOL.into(),
            profile: PROFILE.into(),
            game_sha256: digest(b"game fixture"),
            digest: String::new(),
            assemblies: vec![AssemblyRow {
                identity: ID.into(),
                sha256: digest(&output[0].1),
            }],
            files: vec![
                FileRow {
                    root: "config".into(),
                    path: "gameplay.cfg".into(),
                    sha256: digest(&config[0].1),
                },
                FileRow {
                    root: "plugins".into(),
                    path: format!("{ID}/assets/map.bundle"),
                    sha256: digest(&output[1].1),
                },
            ],
        };
        manifest.digest = digest(canonical_manifest(&manifest).as_bytes());
        let raw = serde_json::to_vec(&manifest).unwrap();
        let report = Report {
            exited_successfully: true,
            ok: true,
            required: true,
            profile: PROFILE.into(),
            protocol: PROTOCOL.into(),
            game_sha256: manifest.game_sha256,
            manifest_sha256: digest(&raw),
            fingerprint: manifest.digest,
            translated: vec![Translation {
                file: "0/gameplay.dll".into(),
                before_sha256: digest(&input[0].1),
                after_sha256: digest(&output[0].1),
                changes: vec!["Current game API call translation".into()],
            }],
            replacements: vec![],
            warnings: vec![],
            errors: vec![],
        };
        let mut output = output;
        output.push((MANIFEST.into(), raw));
        (report, input, output, config)
    }
    fn rewrite_manifest(
        report: &mut Report,
        output: &mut [(PathBuf, Vec<u8>)],
        change: impl FnOnce(&mut Manifest),
        recalculate: bool,
    ) {
        let entry = output
            .iter_mut()
            .find(|(p, _)| p == Path::new(MANIFEST))
            .unwrap();
        let mut manifest: Manifest = serde_json::from_slice(&entry.1).unwrap();
        change(&mut manifest);
        if recalculate {
            manifest.digest = digest(canonical_manifest(&manifest).as_bytes());
            report.fingerprint = manifest.digest.clone();
        }
        entry.1 = serde_json::to_vec(&manifest).unwrap();
        report.manifest_sha256 = digest(&entry.1);
    }
    #[test]
    fn dependency_free_translation_binds_game_assets_and_effective_config() {
        let (report, input, output, config) = fixture();
        validate_report(&report, &report.game_sha256, &input, &output, &[], &config).unwrap();
        let mut renamed = output.clone();
        for (path, _) in &mut renamed {
            if let Ok(relative) = path.strip_prefix("0") {
                *path = PathBuf::from("arbitrary-manager-folder").join(relative);
            }
        }
        validate_manifest(
            &output.last().unwrap().1,
            &report.game_sha256,
            &report.fingerprint,
            &renamed,
            &config,
        )
        .unwrap();
        let mut changed = config.clone();
        changed[0].1.push(1);
        assert!(
            validate_report(&report, &report.game_sha256, &input, &output, &[], &changed).is_err()
        );
    }
    #[test]
    fn canonical_digest_rejects_tampered_output_even_with_updated_raw_report_hash() {
        let (mut report, input, mut output, config) = fixture();
        output[0].1 = b"tampered prepared output".to_vec();
        report.translated[0].after_sha256 = digest(&output[0].1);
        let altered = report.translated[0].after_sha256.clone();
        rewrite_manifest(
            &mut report,
            &mut output,
            |manifest| manifest.assemblies[0].sha256 = altered,
            false,
        );
        let error = validate_report(&report, &report.game_sha256, &input, &output, &[], &config)
            .unwrap_err();
        assert!(error.to_string().contains("canonical digest"));
    }
    #[test]
    fn canonical_manifest_order_is_ordinal_and_independent_of_json_order() {
        let (mut report, input, mut output, config) = fixture();
        let raw_before = report.manifest_sha256.clone();
        rewrite_manifest(
            &mut report,
            &mut output,
            |manifest| manifest.files.reverse(),
            false,
        );
        assert_ne!(report.manifest_sha256, raw_before);
        validate_report(&report, &report.game_sha256, &input, &output, &[], &config).unwrap();
        assert_eq!(ordinal("\u{10000}", "\u{e000}"), std::cmp::Ordering::Less);
        let manifest: Manifest = serde_json::from_slice(&output.last().unwrap().1).unwrap();
        let canonical = canonical_manifest(&manifest);
        assert!(canonical.starts_with(&format!(
            "{PROTOCOL}\n{PROFILE}\n{}\nassembly\t{ID}\t",
            report.game_sha256
        )));
        assert!(canonical.ends_with('\n'));
        assert!(
            canonical.find("file\tconfig/").unwrap() < canonical.find("file\tplugins/").unwrap()
        );
    }
    #[test]
    fn duplicate_identities_paths_and_malformed_manifest_headers_are_rejected() {
        for change in 0..6 {
            let (mut report, input, mut output, config) = fixture();
            rewrite_manifest(
                &mut report,
                &mut output,
                |manifest| match change {
                    0 => manifest.assemblies.push(AssemblyRow {
                        identity: ID.to_lowercase(),
                        sha256: digest(b"other library"),
                    }),
                    1 => manifest.assemblies.push(AssemblyRow {
                        identity: ID.into(),
                        sha256: digest(b"other library"),
                    }),
                    2 => manifest.files.push(FileRow {
                        root: "config".into(),
                        path: "GAMEPLAY.CFG".into(),
                        sha256: digest(b"other config"),
                    }),
                    3 => manifest.files[0].path = "../outside.cfg".into(),
                    4 => manifest.protocol = "different protocol".into(),
                    _ => manifest.game_sha256 = digest(b"other game"),
                },
                true,
            );
            assert!(
                validate_report(&report, &report.game_sha256, &input, &output, &[], &config)
                    .is_err(),
                "case {change}"
            );
        }
        let (report, input, mut output, config) = fixture();
        output.push(("0/GAMEPLAY.dll".into(), output[0].1.clone()));
        assert!(
            validate_report(&report, &report.game_sha256, &input, &output, &[], &config)
                .unwrap_err()
                .to_string()
                .contains("Duplicate compatibility file path")
        );
    }
    #[test]
    fn residual_errors_oversized_diagnostics_and_duplicate_translation_paths_block() {
        let (mut report, input, output, config) = fixture();
        report.exited_successfully = false;
        assert!(
            validate_report(&report, &report.game_sha256, &input, &output, &[], &config)
                .unwrap_err()
                .to_string()
                .contains("exited unsuccessfully")
        );
        report.exited_successfully = true;
        report.errors.push("Unchecked legacy API reference".into());
        assert!(
            validate_report(&report, &report.game_sha256, &input, &output, &[], &config)
                .unwrap_err()
                .to_string()
                .contains("Unchecked legacy API")
        );
        report.errors = vec!["error".into(); 257];
        assert!(
            validate_report(&report, &report.game_sha256, &input, &output, &[], &config)
                .unwrap_err()
                .to_string()
                .contains("exceeds limits")
        );
        report.errors.clear();
        report.translated[0].changes.push("x".repeat(2001));
        assert!(
            validate_report(&report, &report.game_sha256, &input, &output, &[], &config).is_err()
        );
        report.translated[0].changes.clear();
        report.translated.push(Translation {
            file: "0/GAMEPLAY.dll".into(),
            before_sha256: digest(&input[0].1),
            after_sha256: digest(&output[0].1),
            changes: vec![],
        });
        assert!(
            validate_report(&report, &report.game_sha256, &input, &output, &[], &config).is_err()
        );
    }
    #[test]
    fn every_removed_dependency_requires_its_own_exact_pinned_replacement() {
        let (mut report, mut input, mut output, config) = fixture();
        let old = b"known old RarityLib fixture".to_vec();
        input.push(("1/renamed-library.dll".into(), old.clone()));
        let pinned = b"pinned RarityLib fixture".to_vec();
        let pinned_hash = digest(&pinned);
        let replacement_id = "RarityLib, Version=2.0.0.0, Culture=neutral, PublicKeyToken=null";
        output.push((
            "DuctTapePlusPlus/Libraries/RarityLib.dll".into(),
            pinned.clone(),
        ));
        report.replacements.push(Replacement {
            file: "DuctTapePlusPlus/Libraries/RarityLib.dll".into(),
            identity: replacement_id.into(),
            sha256: pinned_hash.clone(),
            reason: "Pinned modern dependency".into(),
        });
        rewrite_manifest(
            &mut report,
            &mut output,
            |manifest| {
                manifest.assemblies.push(AssemblyRow {
                    identity: replacement_id.into(),
                    sha256: pinned_hash.clone(),
                })
            },
            true,
        );
        let mut support = vec![("payloads/index.json".into(), serde_json::to_vec(&serde_json::json!({"protocol":PROTOCOL,"payloads":[{"assembly":"RarityLib","file":format!("{pinned_hash}.dll"),"sha256":pinned_hash}]})).unwrap()), (format!("payloads/{pinned_hash}.dll").into(), pinned), ("helper/old-libraries.tsv".into(), format!("RarityLib.dll\t{}\tFixture\n", digest(&old)).into_bytes())];
        support.push((
            "runtime/rounds-port.Runtime.dll".into(),
            b"pinned runtime fixture".to_vec(),
        ));
        support.push((
            "runtime/Canna.DuctTapePlusPlus.NetworkGuard.dll".into(),
            b"pinned network guard fixture".to_vec(),
        ));
        validate_report(
            &report,
            &report.game_sha256,
            &input,
            &output,
            &support,
            &config,
        )
        .unwrap();
        input.last_mut().unwrap().1 = b"unknown old library fixture".to_vec();
        assert!(
            validate_report(
                &report,
                &report.game_sha256,
                &input,
                &output,
                &support,
                &config
            )
            .unwrap_err()
            .to_string()
            .contains("Unsupported removed dependency")
        );
        input.last_mut().unwrap().1 = old;
        let mut mismatched = support.clone();
        mismatched[0].1 = serde_json::to_vec(&serde_json::json!({"protocol":PROTOCOL,"payloads":[{"assembly":"UnboundLib","file":format!("{pinned_hash}.dll"),"sha256":pinned_hash}]})).unwrap();
        mismatched[2].1 = format!(
            "UnboundLib.dll\t{}\tFixture\n",
            digest(&input.last().unwrap().1)
        )
        .into_bytes();
        assert!(
            validate_report(
                &report,
                &report.game_sha256,
                &input,
                &output,
                &mismatched,
                &config
            )
            .is_err()
        );
    }
    #[test]
    fn resolved_cache_preserves_saved_pack_choices_and_hashes_the_exact_archive() {
        let workspace = Workspace::new().unwrap();
        crate::modpacks::with_test_root(workspace.0.clone(), || {
            let (report, _, output, config) = fixture();
            let game = crate::model::GameInfo {
                app_id: 1557740,
                name: "ROUNDS".into(),
                folder: "rounds".into(),
                description: String::new(),
                icon: String::new(),
                mods: vec![],
                mod_folder_status: String::new(),
            };
            let source = crate::cache::Source {
                owner: "fixture".into(),
                repository: "fixture".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            };
            let item = crate::model::ModInfo {
                provenance: serde_json::json!({"required_game_branch":"old-rounds-for-mods"}),
                enabled: false,
                name: "Old dependency".into(),
                version: "1".into(),
                content_type: String::new(),
                description: String::new(),
                file: "Mods/disabled.dll".into(),
                sha256: digest(b"disabled"),
                local_file: String::new(),
                dependencies: vec!["Replacement supplied elsewhere".into()],
            };
            let pack = Modpack::create(
                "Any custom name".into(),
                String::new(),
                &game,
                source,
                vec![item],
            );
            pack.save().unwrap();
            let saved_path = crate::modpacks::directory().join(format!("{}.canna.json", pack.id));
            let before = fs::read(&saved_path).unwrap();
            let files = PluginEntries {
                plugins: output,
                patchers: vec![],
                configs: config,
            };
            let resolved = cache_resolved(&pack, &files).unwrap();
            assert_eq!(fs::read(&saved_path).unwrap(), before);
            assert!(!pack.mods[0].enabled);
            assert_eq!(resolved.mods.len(), 1);
            assert_eq!(
                resolved.mods[0].provenance["compatibility_profile"],
                PROFILE
            );
            let bytes =
                fs::read(crate::modpacks::local_directory().join(&resolved.mods[0].local_file))
                    .unwrap();
            assert_eq!(digest(&bytes), resolved.mods[0].sha256);
            let entries = crate::runtime::archive_files(&bytes).unwrap();
            assert!(entries.iter().any(|(path, raw)| path
                == &PathBuf::from("BepInEx/plugins").join(MANIFEST)
                && digest(raw) == report.manifest_sha256));
        });
    }
}
