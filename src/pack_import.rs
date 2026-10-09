//! Content detection and reviewed exact-version imports for r2modman/Thunderstore exports.
use crate::{
    model::{GameInfo, ModInfo},
    modpacks::Modpack,
    pack_configs::Config,
};
use anyhow::{Context, Result, bail, ensure};
use eframe::egui;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub name: String,
    pub version: String,
    pub enabled: bool,
}
#[derive(Clone)]
pub struct Profile {
    pub name: String,
    pub description: String,
    pub kind: &'static str,
    pub mods: Vec<Pin>,
    pub configs: Vec<Config>,
    pub notes: Vec<String>,
    local: Option<(Vec<u8>, String)>,
}
enum Detected {
    Canna(PathBuf),
    External(Profile),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct R2Profile {
    profile_name: String,
    mods: Vec<R2Mod>,
}
#[derive(Deserialize)]
struct R2Mod {
    name: String,
    version: R2Version,
    #[serde(default = "enabled")]
    enabled: bool,
}
fn enabled() -> bool {
    true
}
#[derive(Deserialize)]
struct R2Version {
    major: u32,
    minor: u32,
    patch: u32,
}

fn package_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('-').collect();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 100
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}
fn version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 10
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u32>().is_ok()
        })
}
fn dependency(value: &str) -> Result<Pin> {
    let (name, ver) = value
        .rsplit_once('-')
        .context("Thunderstore dependencies must be Owner-Package-1.2.3")?;
    ensure!(
        package_name(name) && version(ver),
        "Invalid Thunderstore dependency: {value}"
    );
    Ok(Pin {
        name: name.into(),
        version: ver.into(),
        enabled: true,
    })
}
fn validate(profile: &Profile) -> Result<()> {
    ensure!(
        !profile.name.trim().is_empty()
            && profile.name.len() <= 100
            && profile.description.len() <= 2000,
        "Imported name/description exceeds Canna's limits"
    );
    ensure!(profile.mods.len() <= 1000, "Profile exceeds 1000 packages");
    let mut seen = BTreeSet::new();
    for pin in &profile.mods {
        ensure!(
            package_name(&pin.name)
                && version(&pin.version)
                && seen.insert(pin.name.to_lowercase()),
            "Invalid, duplicate or conflicting package pin: {}",
            pin.name
        );
    }
    crate::pack_configs::validate(&profile.configs)
}
fn detect(path: &Path) -> Result<Detected> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "Pack archive exceeds 128 MiB"
    );
    if !bytes.starts_with(b"PK") {
        ensure!(
            bytes.len() <= 2 * 1024 * 1024,
            "Pack manifest exceeds 2 MiB"
        );
        let pack: Modpack = serde_json::from_slice(&bytes)
            .context("Choose a Canna JSON, .r2z profile or Thunderstore ZIP")?;
        pack.validate()?;
        return Ok(Detected::Canna(path.into()));
    }
    let entries = crate::runtime::archive_files(&bytes)?;
    let r2 = entries.iter().find(|(p, _)| p == Path::new("export.r2x"));
    let manifest = entries
        .iter()
        .find(|(p, _)| p == Path::new("manifest.json"));
    if let Some((_, data)) = manifest {
        ensure!(data.len() <= 2 * 1024 * 1024, "Pack manifest exceeds 2 MiB");
        let value: Value = serde_json::from_slice(data).context("Invalid manifest.json")?;
        if value["format"] == "canna_modpack" {
            ensure!(
                r2.is_none(),
                "Archive contains conflicting Canna and r2modman manifests"
            );
            return Ok(Detected::Canna(path.into()));
        }
    }
    let mut profile = if let Some((_, data)) = r2 {
        ensure!(
            manifest.is_none(),
            "Archive contains conflicting profile manifests"
        );
        ensure!(
            data.len() <= 2 * 1024 * 1024,
            "r2modman manifest exceeds 2 MiB"
        );
        let options = serde_saphyr::options! { strict_booleans:true, reject_unsupported_tags:true, emit_comments:false, duplicate_keys:serde_saphyr::options::DuplicateKeyPolicy::Error, budget:serde_saphyr::budget! { max_depth:16, max_documents:1, max_events:50000, max_aliases:0, max_anchors:0, max_total_scalar_bytes:2*1024*1024 }, merge_keys:serde_saphyr::options::MergeKeyPolicy::Error };
        let parsed: R2Profile =
            serde_saphyr::from_str_with_options(std::str::from_utf8(data)?, options)
                .context("Invalid export.r2x")?;
        Profile {
            name: parsed.profile_name,
            description: "Imported r2modman profile".into(),
            kind: "r2modman / Thunderstore profile",
            mods: parsed
                .mods
                .into_iter()
                .map(|m| Pin {
                    name: m.name,
                    version: format!(
                        "{}.{}.{}",
                        m.version.major, m.version.minor, m.version.patch
                    ),
                    enabled: m.enabled,
                })
                .collect(),
            configs: vec![],
            notes: vec![],
            local: None,
        }
    } else if let Some((_, data)) = manifest {
        #[derive(Deserialize)]
        struct Manifest {
            name: String,
            version_number: String,
            #[serde(default)]
            description: String,
            dependencies: Vec<String>,
        }
        let parsed: Manifest =
            serde_json::from_slice(data).context("Unrecognized Thunderstore manifest")?;
        ensure!(
            version(&parsed.version_number),
            "Invalid Thunderstore package version"
        );
        let payload = entries.iter().any(|(p, _)| {
            p.extension().is_some_and(|e| {
                matches!(
                    e.to_string_lossy().to_ascii_lowercase().as_str(),
                    "dll" | "bundle" | "assetbundle"
                )
            })
        });
        if payload {
            ensure!(
                bytes.len() <= 32 * 1024 * 1024 && parsed.dependencies.len() <= 32,
                "Local packages support up to 32 MiB and 32 declared dependencies"
            );
        }
        ensure!(
            !entries.iter().any(|(p, _)| p
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
                .contains("bepinex/core/")
                || matches!(
                    p.to_string_lossy().to_ascii_lowercase().as_str(),
                    "winhttp.dll" | "doorstop_config.ini" | "version.dll"
                )),
            "Loader distributions are managed separately; import a profile or modpack instead"
        );
        Profile {
            name: parsed.name,
            description: parsed.description,
            kind: "Thunderstore package / modpack",
            mods: parsed
                .dependencies
                .iter()
                .map(|s| dependency(s))
                .collect::<Result<_>>()?,
            configs: vec![],
            notes: vec![],
            local: payload.then_some((bytes, parsed.version_number)),
        }
    } else {
        bail!(
            "No export.r2x or manifest.json found. Export the profile from r2modman Settings first."
        );
    };
    for (path, data) in &entries {
        let normalized = path.to_string_lossy().replace('\\', "/");
        let lower = normalized.to_ascii_lowercase();
        let relative = if lower.starts_with("bepinex/config/") {
            Some(&normalized[15..])
        } else if lower.starts_with("config/") {
            Some(&normalized[7..])
        } else {
            None
        };
        if let Some(relative) = relative {
            ensure!(
                crate::pack_configs::safe_path(relative),
                "Unsupported/unsafe config: {normalized}"
            );
            profile.configs.push(Config {
                path: relative.into(),
                contents: std::str::from_utf8(data)
                    .with_context(|| format!("Config must be UTF-8 text: {normalized}"))?
                    .into(),
            });
        } else if !matches!(
            lower.as_str(),
            "export.r2x"
                | "manifest.json"
                | "mods.yml"
                | "readme.md"
                | "icon.png"
                | "license"
                | "license.txt"
                | "license.md"
        ) && profile.local.is_none()
        {
            ensure!(
                !path.extension().is_some_and(|e| matches!(
                    e.to_string_lossy().to_ascii_lowercase().as_str(),
                    "dll" | "exe" | "bat" | "cmd" | "ps1" | "vbs" | "jar" | "scr" | "com" | "lnk"
                )),
                "Profile contains executable payload {normalized}; r2modman exports should contain selections and configs only"
            );
            ensure!(
                profile.notes.len() < 100,
                "Too many unsupported profile files"
            );
            profile
                .notes
                .push(format!("Not imported outside BepInEx/config: {normalized}"));
        }
    }
    validate(&profile)?;
    Ok(Detected::External(profile))
}

#[derive(Deserialize)]
struct Record {
    app_id: u32,
    item: ModInfo,
}
fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn record_id(item: &ModInfo) -> Option<&str> {
    item.file
        .strip_prefix("Mods/")?
        .strip_suffix(".zip")
        .filter(|id| valid_id(id))
}

/// Explicit exported versions/disabled selections override transitive choices by provider identity.
fn assemble(
    profile: &Profile,
    game: &GameInfo,
    roots: &BTreeMap<String, String>,
    records: &BTreeMap<String, Record>,
) -> Result<Modpack> {
    validate(profile)?;
    let mut selected = Vec::new();
    let mut projects = BTreeSet::new();
    let mut queue = VecDeque::new();
    for pin in &profile.mods {
        let id = roots
            .get(&pin.name)
            .context("Missing reviewed package pin")?;
        let record = records
            .get(id)
            .context("Missing exact approved package metadata")?;
        ensure!(
            record.item.provenance["provider"] == "thunderstore"
                && record.item.provenance["id"].as_str() == Some(&pin.name)
                && record.item.version == pin.version,
            "Server returned a different package/version for {}",
            pin.name
        );
        projects.insert(pin.name.clone());
        if record.item.provenance["framework_root"].is_string() {
            ensure!(
                crate::game_profiles::loader(&pin.name).is_some(),
                "Unsupported loader package: {}",
                pin.name
            );
            continue;
        }
        ensure!(
            record.app_id == game.app_id,
            "{} belongs to another game; choose the correct game",
            pin.name
        );
        let mut item = record.item.clone();
        item.enabled = pin.enabled;
        if item.enabled {
            queue.push_back(id.clone());
        }
        selected.push(item);
    }
    while let Some(id) = queue.pop_front() {
        let parent = records
            .get(&id)
            .context("Missing approved dependency metadata")?;
        for id in parent.item.provenance["dependency_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let record = records
                .get(id)
                .context("Required dependency is not approved/available")?;
            if record.item.provenance["framework_root"].is_string() {
                continue;
            }
            ensure!(
                record.app_id == game.app_id,
                "Required dependency belongs to another game"
            );
            let identity = record.item.provenance["id"]
                .as_str()
                .context("Dependency has no project identity")?;
            if !projects.insert(identity.to_owned()) {
                continue;
            }
            ensure!(selected.len() < 1000, "Dependency graph exceeds 1000 mods");
            selected.push(record.item.clone());
            queue.push_back(id.to_owned());
        }
    }
    if let Some((bytes, ver)) = &profile.local {
        use sha2::Digest;
        let hash = format!("{:x}", sha2::Sha256::digest(bytes));
        let dependencies = profile
            .mods
            .iter()
            .filter_map(|pin| roots.get(&pin.name))
            .filter_map(|id| records.get(id))
            .filter(|r| !r.item.provenance["framework_root"].is_string())
            .map(|r| r.item.name.clone())
            .collect();
        selected.insert(
            0,
            ModInfo {
                enabled: true,
                name: profile.name.clone(),
                version: ver.clone(),
                description: profile.description.clone(),
                file: format!("Mods/{hash}.zip"),
                sha256: hash.clone(),
                local_file: format!("{hash}.zip"),
                dependencies,
                content_type: "mod".into(),
                provenance: Value::Null,
            },
        );
    }
    let mut pack = Modpack::create(
        profile.name.clone(),
        profile.description.clone(),
        game,
        crate::cache::Source {
            owner: "canna".into(),
            repository: "server".into(),
            branch: "main".into(),
            catalog_folder: String::new(),
        },
        selected,
    );
    pack.auto_update = false;
    pack.imported_configs = profile.configs.clone();
    pack.validate()?;
    ensure!(
        serde_json::to_vec(&pack)?.len() <= 2 * 1024 * 1024,
        "Imported pack metadata exceeds 2 MiB; export a smaller profile"
    );
    Ok(pack)
}

enum Event {
    Detected(Result<Detected, String>),
    Progress(String),
    Root(String, String),
    Finished(Result<Outcome, String>),
}
enum Outcome {
    Ready(Box<Modpack>),
    Pending(Vec<String>, bool),
}

fn resolve(
    profile: &Profile,
    game: &GameInfo,
    token: &str,
    cancel: &AtomicBool,
    progress: &dyn Fn(String),
    known_roots: &BTreeMap<String, String>,
    root_ready: &dyn Fn(String, String),
) -> Result<Outcome> {
    ensure!(
        !token.is_empty() || profile.mods.is_empty(),
        "Connect your Canna account in Settings first"
    );
    let check = || -> Result<()> {
        ensure!(!cancel.load(Ordering::Relaxed), "Import cancelled");
        Ok(())
    };
    let client = crate::provider_browser::client()?;
    let community = &crate::game_profiles::by_id(game.app_id)
        .context("Choose a supported Thunderstore BepInEx game")?
        .community;
    let request = |method, path: &str, body| {
        crate::provider_browser::request(
            &client,
            token,
            method,
            crate::provider_browser::api(path),
            body,
        )
    };
    let mut roots = BTreeMap::new();
    for (i, pin) in profile.mods.iter().enumerate() {
        check()?;
        if let Some(id) = known_roots.get(&pin.name) {
            ensure!(valid_id(id), "Invalid retained import ID");
            roots.insert(pin.name.clone(), id.clone());
            continue;
        }
        progress(format!(
            "Retrieving {}/{}: {} {}",
            i + 1,
            profile.mods.len(),
            pin.name,
            pin.version
        ));
        let (owner, name) = pin.name.split_once('-').unwrap();
        let data = request(reqwest::Method::POST,"mods/external/import",Some(json!({"url":format!("https://thunderstore.io/c/{community}/p/{owner}/{name}/"),"version":pin.version}))).with_context(||format!("{} {}",pin.name,pin.version))?;
        let id = data["id"]
            .as_str()
            .filter(|id| valid_id(id))
            .context("Invalid import ID")?;
        roots.insert(pin.name.clone(), id.to_owned());
        root_ready(pin.name.clone(), id.to_owned());
    }
    let mut records = BTreeMap::new();
    let mut queue: VecDeque<_> = roots.values().cloned().collect();
    let mut visited = BTreeSet::new();
    let mut pending = Vec::new();
    let mut retry = true;
    while let Some(id) = queue.pop_front() {
        check()?;
        if !visited.insert(id.clone()) {
            continue;
        }
        ensure!(
            visited.len() <= 2000,
            "Dependency graph exceeds import limits"
        );
        let status = request(reqwest::Method::GET, &format!("mods/{id}/status"), None)?;
        if status["state"] != "ready" {
            retry &= matches!(status["state"].as_str(), Some("waiting" | "needs_review"));
            pending.push(format!(
                "{}: {}",
                status["name"].as_str().unwrap_or(&id),
                status["message"].as_str().unwrap_or("Waiting for review")
            ));
            continue;
        }
        let data = request(reqwest::Method::GET, &format!("mods/{id}/manifest"), None)?;
        let record: Record = serde_json::from_value(data)?;
        ensure!(
            record_id(&record.item) == Some(id.as_str())
                && record.item.sha256.len() == 64
                && record.item.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid approved archive metadata"
        );
        for dependency in record.item.provenance["dependency_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            ensure!(valid_id(dependency), "Invalid dependency ID");
            queue.push_back(dependency.to_owned());
        }
        records.insert(id, record);
    }
    if !pending.is_empty() {
        return Ok(Outcome::Pending(pending, retry));
    }
    check()?;
    let pack = assemble(profile, game, &roots, &records)?;
    // Verify/cache the complete reviewed selection, including disabled mods, without activating anything.
    for (i, item) in pack
        .mods
        .iter()
        .enumerate()
        .filter(|(_, m)| m.local_file.is_empty())
    {
        check()?;
        progress(format!(
            "Caching {}/{}: {}",
            i + 1,
            pack.mods.len(),
            item.name
        ));
        let bytes = crate::repository::fetch_optional(
            &client,
            &crate::runtime::settings(&pack),
            token,
            &format!("{}/{}", pack.game.folder, item.file),
            128 * 1024 * 1024,
        )?
        .context("Reviewed archive is unavailable")?;
        use sha2::Digest;
        ensure!(
            format!("{:x}", sha2::Sha256::digest(&bytes)) == item.sha256.to_ascii_lowercase(),
            "Checksum mismatch for {}",
            item.name
        );
        check()?;
        crate::website::remember_mod(&pack, item, &bytes, false)?;
    }
    check()?;
    Ok(Outcome::Ready(Box::new(pack)))
}

#[derive(Default)]
pub struct Importer {
    rx: Option<Receiver<Event>>,
    cancel: Arc<AtomicBool>,
    session: String,
    profile: Option<Profile>,
    game: u32,
    status: String,
    waiting: Vec<String>,
    open: bool,
    roots: BTreeMap<String, String>,
    auto_continue: bool,
    last_poll: Option<std::time::Instant>,
}
impl Importer {
    pub fn start(&mut self, path: PathBuf, game: u32, ctx: &egui::Context) {
        self.cancel.store(true, Ordering::Relaxed);
        *self = Self {
            game,
            open: true,
            ..Default::default()
        };
        self.status = "Inspecting pack archive…".into();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Detected(detect(&path).map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
    }
    fn import(&mut self, game: GameInfo, ctx: &egui::Context) {
        let Some(profile) = self.profile.clone() else {
            return;
        };
        let session = crate::website::session();
        if self.session != session {
            self.roots.clear();
        }
        self.session = session;
        let token = self.session.clone();
        let roots = self.roots.clone();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        self.status = "Preparing reviewed imports…".into();
        self.waiting.clear();
        std::thread::spawn(move || {
            let progress = |s| {
                let _ = tx.send(Event::Progress(s));
                ctx.request_repaint();
            };
            let root_ready = |name, id| {
                let _ = tx.send(Event::Root(name, id));
            };
            let result = resolve(
                &profile,
                &game,
                &token,
                &cancel,
                &progress,
                &roots,
                &root_ready,
            )
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Finished(result));
            ctx.request_repaint();
        });
    }
    pub fn show(&mut self, ctx: &egui::Context, catalog: &[GameInfo]) -> Option<Modpack> {
        let mut completed = None;
        while let Some(event) = self.rx.as_ref().and_then(|r| r.try_recv().ok()) {
            match event {
                Event::Progress(status) => self.status = status,
                Event::Root(name, id) => {
                    self.roots.insert(name, id);
                }
                Event::Detected(result) => {
                    self.rx = None;
                    if !self.open || self.cancel.load(Ordering::Relaxed) {
                        continue;
                    }
                    match result {
                        Ok(Detected::Canna(path)) => match Modpack::import(&path) {
                            Ok(pack) => {
                                completed = Some(pack);
                                self.open = false;
                            }
                            Err(e) => self.status = format!("{e:#}"),
                        },
                        Ok(Detected::External(profile)) => {
                            self.status.clear();
                            self.profile = Some(profile);
                        }
                        Err(e) => self.status = e,
                    }
                }
                Event::Finished(result) => {
                    self.rx = None;
                    if self.cancel.load(Ordering::Relaxed)
                        || self.session != crate::website::session()
                    {
                        self.roots.clear();
                        self.auto_continue = false;
                        self.status="Import stopped because the account changed or it was cancelled. Retry when signed in.".into();
                        continue;
                    }
                    match result {
                        Ok(Outcome::Ready(pack)) => {
                            let saved = (|| -> Result<()> {
                                if let Some((bytes, _)) =
                                    self.profile.as_ref().and_then(|p| p.local.as_ref())
                                {
                                    let item =
                                        pack.mods.first().context("Missing local package")?;
                                    let destination =
                                        crate::modpacks::local_directory().join(&item.local_file);
                                    crate::runtime::no_links(&destination)?;
                                    std::fs::create_dir_all(destination.parent().unwrap())?;
                                    std::fs::write(destination, bytes)?;
                                }
                                pack.save()
                            })();
                            match saved {
                                Ok(()) => {
                                    completed = Some(*pack);
                                    self.open = false;
                                }
                                Err(e) => {
                                    self.status = format!("Could not save imported pack: {e:#}")
                                }
                            }
                        }
                        Ok(Outcome::Pending(waiting, retry)) => {
                            self.status=if retry {"Waiting for analysis/review. Import continues automatically when approved; keep this window open."} else {"A file was denied or its scan failed. Resolve its review status, then retry."}.into();
                            self.waiting = waiting;
                            self.auto_continue = retry;
                            self.last_poll = Some(std::time::Instant::now());
                        }
                        Err(e) => {
                            self.status = e;
                            self.auto_continue = false;
                        }
                    }
                }
            }
        }
        if !self.open {
            return completed;
        }
        if self.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        if self.auto_continue
            && self.rx.is_none()
            && self
                .last_poll
                .is_some_and(|last| last.elapsed() >= std::time::Duration::from_secs(5))
        {
            if self.session != crate::website::session() {
                self.auto_continue = false;
                self.roots.clear();
                self.status = "Account changed; retry after signing in.".into();
            } else if let Some(game) = catalog.iter().find(|g| g.app_id == self.game).cloned() {
                self.last_poll = Some(std::time::Instant::now());
                self.import(game, ctx);
            }
        }
        if self.auto_continue {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        let mut start = None;
        let mut close = false;
        let modal=egui::Modal::new(egui::Id::new("external-pack-import")).show(ctx,|ui|{
            ui.set_max_width((ctx.content_rect().width()-60.0).clamp(240.0,640.0));
            egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-80.0).max(120.0)).show(ui,|ui| {
            ui.heading("Import modpack");
            if let Some(profile)=&self.profile {
                ui.strong(&profile.name); ui.label(format!("{} · {} packages · {} config files",profile.kind,profile.mods.len(),profile.configs.len()));
                ui.label("Exports do not identify their game. Check the target before importing.");
                ui.add_enabled_ui(self.rx.is_none() && self.roots.is_empty(),|ui|egui::ComboBox::from_id_salt("import-pack-game").selected_text(catalog.iter().find(|g|g.app_id==self.game).map(|g|g.name.as_str()).unwrap_or("Choose game…")).show_ui(ui,|ui|{
                    for game in catalog.iter().filter(|g|crate::game_profiles::by_id(g.app_id).is_some()) {ui.selectable_value(&mut self.game,game.app_id,&game.name);}
                }));
                ui.label("Exact mod versions and disabled selections are preserved. Auto updates start off. Canna manages the compatible BepInEx loader separately.");
                if !profile.configs.is_empty() {ui.label("Imported settings apply on the first modded launch (or when switching back to this pack), with backups. Later edits survive relaunch. Configs are included in Canna file exports.");}
                if profile.local.is_some() {ui.label("This ZIP includes a local plugin package. Its bundled bytes are retained as a local import; dependency downloads still pass server review.");}
                egui::ScrollArea::vertical().id_salt("import-pack-entries").max_height(220.0).show(ui,|ui|{
                    for pin in &profile.mods {ui.label(format!("{} · {}{}",pin.name,pin.version,if pin.enabled {""} else {" · disabled"}));}
                    for note in &profile.notes {ui.label(note);}
                    for message in &self.waiting {ui.label(message);}
                });
                if ui.add_enabled(self.rx.is_none() && catalog.iter().any(|g|g.app_id==self.game),egui::Button::new(if self.waiting.is_empty(){"Import and prepare downloads"}else{"Retry reviewed imports"})).clicked(){start=catalog.iter().find(|g|g.app_id==self.game).cloned();}
            }
            if self.rx.is_some() {ui.spinner();}
            if !self.status.is_empty() {ui.label(&self.status);}
            if ui.button(if self.rx.is_some(){"Cancel import"}else{"Close"}).clicked(){close=true;}
            });
        });
        #[cfg(test)]
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("import-modal-rect"), modal.response.rect)
        });
        if close || modal.should_close() {
            self.cancel.store(true, Ordering::Relaxed);
            self.open = false;
        }
        if let Some(game) = start {
            self.import(game, ctx);
        }
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, data) in entries {
            zip.start_file(*path, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    fn inspect(entries: &[(&str, &[u8])]) -> Result<Profile> {
        let path = std::env::temp_dir().join(format!(
            "canna-pack-detect-{}-{}.r2z",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, archive(entries)).unwrap();
        let result = detect(&path);
        std::fs::remove_file(path).unwrap();
        match result? {
            Detected::External(profile) => Ok(profile),
            _ => bail!("Not an external profile"),
        }
    }
    const R2:&[u8]=b"profileName: 'Family Night'\nmods:\n  - name: BepInEx-BepInExPack\n    version: { major: 5, minor: 4, patch: 2100 }\n    enabled: true\n  - name: Team-Root\n    version:\n      major: 1\n      minor: 2\n      patch: 3\n    enabled: false\n";
    #[test]
    fn reads_real_export_schema_pins_disabled_mods_and_configs() {
        let profile = inspect(&[
            ("export.r2x", R2),
            ("config/Root.cfg", b"[General]\nEnabled = false"),
            ("BepInEx/config/nested/settings.json", b"{}"),
            ("other/settings.cfg", b"ignored"),
        ])
        .unwrap();
        assert_eq!(profile.name, "Family Night");
        assert_eq!(profile.mods[0].version, "5.4.2100");
        assert!(!profile.mods[1].enabled);
        assert_eq!(profile.configs[0].path, "Root.cfg");
        assert_eq!(profile.configs[1].path, "nested/settings.json");
        assert_eq!(profile.notes.len(), 1);
        assert!(profile.local.is_none());
    }
    #[test]
    fn rejects_conflicting_pins_yaml_aliases_duplicate_keys_and_unsafe_archives() {
        for yaml in [
            "profileName: x\nprofileName: y\nmods: []",
            "profileName: &a x\nmods: *a",
            "profileName: x\nmods: [{name: Team-Root, version: {major: 1, minor: 0, patch: 0}}, {name: Team-Root, version: {major: 2, minor: 0, patch: 0}}]",
            "profileName: x\nmods: [{name: Team-Root, version: {major: 1, minor: 0, patch: 0}, enabled: no}]",
            "profileName: !include secret\nmods: []",
        ] {
            assert!(
                inspect(&[("export.r2x", yaml.as_bytes())]).is_err(),
                "{yaml}"
            );
        }
        for path in [
            "../evil.cfg",
            "config/CON.cfg",
            "config/evil.dll",
            "config/evil.cfg.exe",
            "BepInEx/plugins/local.dll",
        ] {
            assert!(
                inspect(&[("export.r2x", R2), (path, b"unsafe")]).is_err(),
                "{path}"
            );
        }
        assert!(
            inspect(&[
                ("export.r2x", R2),
                ("config/a.cfg", b"1"),
                ("config/A.cfg", b"2")
            ])
            .is_err()
        );
        assert!(inspect(&[("export.r2x", R2), ("manifest.json", b"{}")]).is_err());
    }
    #[test]
    fn detects_thunderstore_modpacks_and_bundled_local_plugins() {
        let manifest =
            br#"{"name":"GameNight","version_number":"1.0.0","dependencies":["Team-Root-1.2.3"]}"#;
        let profile = inspect(&[
            ("manifest.json", manifest),
            ("BepInEx/config/a.cfg", b"true"),
        ])
        .unwrap();
        assert_eq!(profile.mods[0].name, "Team-Root");
        assert!(profile.local.is_none());
        let local = inspect(&[
            ("manifest.json", manifest),
            ("plugins/Root.dll", b"MZfake fixture"),
        ])
        .unwrap();
        assert!(local.local.is_some());
        assert!(
            inspect(&[
                ("manifest.json", manifest),
                ("BepInEx/core/BepInEx.dll", b"loader")
            ])
            .is_err()
        );
        assert!(
            inspect(&[(
                "manifest.json",
                br#"{"name":"x","version_number":"1.0.0","dependencies":["http://evil"]}"#
            )])
            .is_err()
        );
    }
    fn rec(id: &str, project: &str, title: &str, ver: &str, deps: &[&str]) -> Record {
        Record{app_id:1686940,item:serde_json::from_value(json!({"name":title,"version":ver,"file":format!("Mods/{id}.zip"),"sha256":"a".repeat(64),"dependencies":deps.iter().map(|_|"Dependency").collect::<Vec<_>>(),"provenance":{"provider":"thunderstore","id":project,"dependency_ids":deps}})).unwrap()}
    }
    const ROOT: &str = "11111111-1111-4111-8111-111111111111";
    const OLD: &str = "22222222-2222-4222-8222-222222222222";
    const NEW: &str = "33333333-3333-4333-8333-333333333333";
    #[test]
    fn preserves_explicit_old_versions_and_disabled_dependencies_without_duplicates() {
        let profile = Profile {
            name: "Pinned".into(),
            description: String::new(),
            kind: "r2modman",
            mods: vec![
                Pin {
                    name: "Team-Root".into(),
                    version: "1.2.3".into(),
                    enabled: true,
                },
                Pin {
                    name: "Team-Dependency".into(),
                    version: "1.0.0".into(),
                    enabled: false,
                },
            ],
            configs: vec![],
            notes: vec![],
            local: None,
        };
        let roots = BTreeMap::from([
            ("Team-Root".into(), ROOT.into()),
            ("Team-Dependency".into(), OLD.into()),
        ]);
        let records = BTreeMap::from([
            (ROOT.into(), rec(ROOT, "Team-Root", "Root", "1.2.3", &[NEW])),
            (
                OLD.into(),
                rec(OLD, "Team-Dependency", "Dependency", "1.0.0", &[]),
            ),
            (
                NEW.into(),
                rec(NEW, "Team-Dependency", "Dependency", "2.0.0", &[]),
            ),
        ]);
        let pack = assemble(&profile, &crate::model::bopl(), &roots, &records).unwrap();
        assert_eq!(pack.mods.len(), 2);
        assert_eq!(pack.mods[1].version, "1.0.0");
        assert!(!pack.mods[1].enabled);
        assert!(!pack.auto_update);
        let mut game = crate::model::bopl();
        game.mods = records.values().map(|r| r.item.clone()).collect();
        let (again, count) = crate::pack_updates::select(&pack, &[game]).unwrap();
        assert_eq!(count, 0);
        assert_eq!(again.mods.len(), 2);
        assert_eq!(again.mods[1].version, "1.0.0");
        let mut wrong = crate::model::bopl();
        wrong.app_id = 1557740;
        assert!(assemble(&profile, &wrong, &roots, &records).is_err());
    }
    #[test]
    fn adds_unlisted_dependencies_and_rejects_wrong_or_missing_release_metadata() {
        let profile = Profile {
            name: "Profile".into(),
            description: String::new(),
            kind: "r2",
            mods: vec![Pin {
                name: "Team-Root".into(),
                version: "1.2.3".into(),
                enabled: true,
            }],
            configs: vec![],
            notes: vec![],
            local: None,
        };
        let roots = BTreeMap::from([("Team-Root".into(), ROOT.into())]);
        let mut records = BTreeMap::from([
            (ROOT.into(), rec(ROOT, "Team-Root", "Root", "1.2.3", &[NEW])),
            (
                NEW.into(),
                rec(NEW, "Team-Dependency", "Dependency", "2.0.0", &[ROOT]),
            ),
        ]);
        assert_eq!(
            assemble(&profile, &crate::model::bopl(), &roots, &records)
                .unwrap()
                .mods
                .len(),
            2
        );
        records.get_mut(ROOT).unwrap().item.version = "9.9.9".into();
        assert!(assemble(&profile, &crate::model::bopl(), &roots, &records).is_err());
        records.get_mut(ROOT).unwrap().item.version = "1.2.3".into();
        records.remove(NEW);
        assert!(assemble(&profile, &crate::model::bopl(), &roots, &records).is_err());
    }
    #[test]
    fn canna_config_export_roundtrip_is_content_detected_and_never_overwrites_ids() {
        let root = std::env::temp_dir().join(format!(
            "canna-import-roundtrip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        crate::modpacks::with_test_root(root.join("state"), || {
            let mut pack = Modpack::create(
                "Config roundtrip".into(),
                String::new(),
                &crate::model::bopl(),
                crate::cache::Source {
                    owner: "canna".into(),
                    repository: "server".into(),
                    branch: "main".into(),
                    catalog_folder: String::new(),
                },
                vec![],
            );
            pack.imported_configs.push(Config {
                path: "a.cfg".into(),
                contents: "[A]\nx = 42".into(),
            });
            let zip = root.join("profile.zip");
            pack.export(&zip).unwrap();
            let renamed = root.join("profile.r2z");
            std::fs::rename(&zip, &renamed).unwrap();
            assert!(matches!(detect(&renamed).unwrap(), Detected::Canna(_)));
            let imported = Modpack::import(&renamed).unwrap();
            assert_ne!(imported.id, pack.id);
            assert_eq!(imported.imported_configs, pack.imported_configs);
            assert!(!root.join("game").exists());
        });
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancelled_resolution_does_not_contact_server_or_save_partial_pack() {
        let profile = Profile {
            name: "Cancelled".into(),
            description: String::new(),
            kind: "r2",
            mods: vec![Pin {
                name: "Team-Root".into(),
                version: "1.0.0".into(),
                enabled: true,
            }],
            configs: vec![],
            notes: vec![],
            local: None,
        };
        let cancel = AtomicBool::new(true);
        let error = resolve(
            &profile,
            &crate::model::bopl(),
            "fixture",
            &cancel,
            &|_| {},
            &BTreeMap::new(),
            &|_, _| {},
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("cancelled"));
    }
    #[test]
    fn closing_during_detection_does_not_import_a_canna_pack() {
        let root = std::env::temp_dir().join(format!(
            "canna-cancel-detection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        crate::modpacks::with_test_root(root.join("state"), || {
            let pack = Modpack::create(
                "Cancelled detection".into(),
                String::new(),
                &crate::model::bopl(),
                crate::cache::Source {
                    owner: "canna".into(),
                    repository: "server".into(),
                    branch: "main".into(),
                    catalog_folder: String::new(),
                },
                vec![],
            );
            let path = root.join("pack.json");
            pack.export(&path).unwrap();
            let (tx, rx) = mpsc::channel();
            tx.send(Event::Detected(Ok(Detected::Canna(path)))).unwrap();
            let mut importer = Importer {
                rx: Some(rx),
                open: false,
                ..Default::default()
            };
            assert!(importer.show(&egui::Context::default(), &[]).is_none());
            let (saved, errors) = crate::modpacks::load_all();
            assert!(saved.is_empty() && errors.is_empty());
        });
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preview_stays_inside_compact_windows_and_escape_cancels() {
        let ctx = egui::Context::default();
        let mut profile =
            inspect(&[("export.r2x", R2), ("config/test.cfg", b"[Main]\nValue=1")]).unwrap();
        profile.mods.extend((0..80).map(|n| Pin {
            name: format!("Team-Mod{n}"),
            version: "1.0.0".into(),
            enabled: true,
        }));
        for size in [egui::vec2(640.0, 480.0), egui::vec2(390.0, 720.0)] {
            let mut importer = Importer {
                profile: Some(profile.clone()),
                game: 1686940,
                open: true,
                ..Default::default()
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    importer.show(ctx, &[crate::model::bopl()]);
                },
            );
            let rect = ctx
                .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::new("import-modal-rect")))
                .unwrap();
            assert!(
                rect.width() <= size.x && rect.height() <= size.y,
                "{size:?} {rect:?}"
            );
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events: vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    ..Default::default()
                },
                |ctx| {
                    importer.show(ctx, &[crate::model::bopl()]);
                },
            );
            assert!(!importer.open);
            assert!(importer.cancel.load(Ordering::Relaxed));
        }
    }
    #[test]
    #[ignore = "Imports a supplied profile through the authenticated server into isolated temporary Canna storage; never launches a game"]
    fn live_reviewed_profile_import() {
        let path = PathBuf::from(
            std::env::var_os("CANNA_IMPORT_PROFILE").expect("Set CANNA_IMPORT_PROFILE"),
        );
        let game_id: u32 = std::env::var("CANNA_IMPORT_GAME").unwrap().parse().unwrap();
        let profile = match detect(&path).unwrap() {
            Detected::External(p) => p,
            _ => panic!("Supply an external profile"),
        };
        let template = crate::game_profiles::by_id(game_id).unwrap();
        let game = GameInfo {
            app_id: game_id,
            name: template.name.clone(),
            folder: template.folder.clone(),
            description: String::new(),
            icon: String::new(),
            mods: vec![],
            mod_folder_status: String::new(),
        };
        let root = std::env::temp_dir().join(format!(
            "canna-live-pack-import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let token = crate::website::session();
        assert!(!token.is_empty(), "Connect Canna first");
        crate::modpacks::with_test_root(root.join("state"), || {
            let outcome = resolve(
                &profile,
                &game,
                &token,
                &AtomicBool::new(false),
                &|message| println!("{message}"),
                &BTreeMap::new(),
                &|_, _| {},
            )
            .unwrap();
            let pack = match outcome {
                Outcome::Ready(pack) => pack,
                Outcome::Pending(waiting, _) => panic!("Review pending: {waiting:?}"),
            };
            assert!(!pack.auto_update);
            assert_eq!(pack.imported_configs, profile.configs);
            pack.save().unwrap();
            for pin in profile
                .mods
                .iter()
                .filter(|p| crate::game_profiles::loader(&p.name).is_none())
            {
                let item = pack
                    .mods
                    .iter()
                    .find(|m| m.provenance["id"].as_str() == Some(pin.name.as_str()))
                    .unwrap();
                assert_eq!(item.version, pin.version);
                assert_eq!(item.enabled, pin.enabled);
            }
            assert!(!root.join("game").exists());
            println!(
                "Verified {} exact mod selections and {} imported configs; no game activation",
                pack.mods.len(),
                pack.imported_configs.len()
            );
        });
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
