#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod account;
mod cache;
mod chrome;
mod console;
mod credentials;
mod dependencies;
mod diagnostics;
mod ducttape;
mod game_compat;
#[path = "../server/src/game_profiles.rs"]
mod game_profiles;
mod minecraft;
mod minecraft_auth;
mod model;
mod modpacks;
mod owned_game;
mod pack_configs;
mod pack_import;
mod pack_ui;
mod pack_updates;
mod play_backup;
mod play_config;
mod play_lab;
#[path = "../server/src/play_manifest.rs"]
mod play_manifest;
mod play_metrics;
mod provider_browser;
mod rebound_support;
mod repository;
mod runtime;
mod runtime_cache;
mod runtime_health;
mod shared_packs;
mod skin_catalog;
mod skins;
mod source_addons;
mod steam;
mod ui_helpers;
mod unity_restore;
mod updater;
mod website;

use eframe::egui::{self, Color32, RichText};
use model::{GameInfo, InstalledGame, Scan, Settings};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
};

const GREEN: Color32 = Color32::from_rgb(163, 220, 144);
const MUTED: Color32 = Color32::from_rgb(143, 157, 149);
fn rebound_settings(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    busy: bool,
    authorized: bool,
) -> egui::Response {
    ui.strong("ROUNDS COMPATIBILITY");
    let response = ui.add_enabled(
        authorized && !busy,
        egui::Checkbox::new(enabled, "Canna Bliss for ROUNDS (preview)"),
    );
    if authorized {
        ui.label("Opt in for public ROUNDS 1.1.2. Apply, Setup and Launch modded check every enabled plugin and prepare supported dependency ports without changing saved selections. Disable the original DuctTape/preloader package first.");
        ui.label("Unsupported calls or dependencies block preparation. Read Console review warnings; preparation does not verify every gameplay path or full multiplayer matches.");
        ui.hyperlink_to(
            "Preview setup, source credits and distribution notices",
            "https://cannamods.vip/help",
        );
    } else {
        ui.label("Canna Bliss is a Beta feature. Sign in with a Beta account to enable it. Support is downloaded after the server verifies access.");
    }
    response
}
#[cfg(test)]
const EMBEDDED_GITHUB_TOKEN: &str = "";

enum Event {
    Update(Result<Option<updater::Ready>, String>),
    ConsoleData(Vec<(u32, console::Snapshot)>),
    Launched(u32, bool, std::time::SystemTime, owned_game::OwnedGame),
    PackPrepared(Box<modpacks::Modpack>),
    RuntimeProgress(String),
    Runtime(Result<String, String>),
    Scanned(Scan),
    Cached(
        cache::Source,
        Result<Option<repository::RepositoryData>, String>,
    ),
    Synced(cache::Source, Result<repository::RepositoryData, String>),
}
struct Canna {
    diagnostics: diagnostics::Reporter,
    account: account::Account,
    rebound_access: rebound_support::Access,
    website: website::Website,
    skins: skins::Skins,
    minecraft: minecraft::Minecraft,
    minecraft_page: bool,
    chrome: chrome::Chrome,
    update_status: String,
    pending_update: Option<updater::Ready>,
    owned_games: BTreeMap<u32, owned_game::OwnedGame>,
    discover_page: bool,
    discover: pack_ui::DiscoverState,
    provider_browser: provider_browser::Browser,
    console_page: bool,
    console: console::Console,
    console_polling: bool,
    last_console_poll: std::time::Instant,
    launch_watch: Option<console::LaunchWatch>,
    game_details: bool,
    runtime_busy: bool,
    runtime_enabled: bool,
    runtime_status: String,
    #[cfg(test)]
    card_create_rects: BTreeMap<u32, egui::Rect>,
    #[cfg(test)]
    card_view_rects: BTreeMap<u32, egui::Rect>,
    modpacks_page: bool,
    pack_ui: pack_ui::PackUi,
    settings: Settings,
    token: String,
    settings_open: bool,
    query: String,
    supported_only: bool,
    games: Vec<InstalledGame>,
    catalog: Vec<GameInfo>,
    libraries: Vec<std::path::PathBuf>,
    warnings: Vec<String>,
    textures: BTreeMap<u32, egui::TextureHandle>,
    repository_textures: BTreeMap<u32, egui::TextureHandle>,
    active_source: Option<cache::Source>,
    cached_at: Option<u64>,
    selected: u32,
    scanning: bool,
    last_steam_scan: std::time::Instant,
    steam_was_focused: bool,
    syncing: bool,
    scan_status: String,
    repo_status: String,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    screenshot: Option<std::path::PathBuf>,
    screenshot_requested: bool,
    ready_frames: usize,
}
fn library_rows(
    catalog: &[GameInfo],
    installed: &[InstalledGame],
) -> Vec<(u32, String, bool, bool, String)> {
    let mut rows: Vec<_> = installed
        .iter()
        .filter(|g| model::supported_game(g.app_id))
        .map(|g| {
            (
                g.app_id,
                g.name.clone(),
                true,
                catalog.iter().any(|c| c.app_id == g.app_id),
                g.loader.clone(),
            )
        })
        .collect();
    for game in catalog {
        if game.app_id != u32::MAX
            && game.app_id != 0
            && model::supported_game(game.app_id)
            && !rows.iter().any(|row| row.0 == game.app_id)
        {
            rows.push((
                game.app_id,
                game.name.clone(),
                false,
                true,
                "Not Installed".into(),
            ));
        }
    }
    rows.sort_by_key(|row| (!row.2, row.1.to_lowercase()));
    rows
}
impl Canna {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_context(&cc.egui_ctx, true)
    }
    fn new_with_context(ctx: &egui::Context, start_jobs: bool) -> Self {
        let provider_preview = std::env::var_os("CANNA_SCREENSHOT").is_some()
            && std::env::args().any(|a| a == "--provider-browser-preview");
        let account_preview = std::env::var_os("CANNA_SCREENSHOT").is_some()
            && std::env::args().any(|a| {
                matches!(
                    a.as_str(),
                    "--account-preview" | "--account-code-preview" | "--account-profile-preview"
                )
            });
        let start_jobs = start_jobs && !provider_preview && !account_preview;
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        ui_helpers::apply_corner_radii(&mut style.visuals);
        style.visuals.panel_fill = Color32::from_rgb(18, 24, 22);
        style.visuals.window_fill = Color32::from_rgb(26, 34, 30);
        style.visuals.override_text_color = Some(Color32::from_rgb(227, 237, 229));
        style.visuals.extreme_bg_color = Color32::from_rgb(35, 46, 39);
        style.visuals.text_edit_bg_color = Some(Color32::from_rgb(35, 46, 39));
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(39, 51, 43);
        style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(57, 78, 60);
        style.visuals.selection.bg_fill = Color32::from_rgb(58, 87, 54);
        style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, GREEN);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(35, 45, 39);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(54, 74, 56);
        style.spacing.item_spacing = egui::vec2(12.0, 12.0);
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.button_padding = egui::vec2(16.0, 10.0);
        ctx.set_style(style);
        let (tx, rx) = mpsc::channel();
        let mut settings = Settings::load();
        settings.owner = "canna".into();
        settings.repository = "server".into();
        settings.branch = "main".into();
        settings.catalog_folder.clear();
        let configured = !settings.owner.is_empty();
        let mut app = Self {
            diagnostics: diagnostics::Reporter::new(settings.anonymous_reports),
            account: Default::default(),
            rebound_access: Default::default(),
            website: website::Website::default(),
            skins: {
                let mut skins = skins::Skins::default();
                if std::env::args().any(|a| a == "--skins") {
                    skins.open_browser();
                }
                if let Some(query) = std::env::args()
                    .find_map(|a| a.strip_prefix("--skin-search=").map(str::to_owned))
                {
                    skins.open_browser();
                    skins.search_for(query);
                }
                skins
            },
            minecraft: Default::default(),
            minecraft_page: false,
            chrome: chrome::Chrome::new(ctx),
            owned_games: BTreeMap::new(),
            discover_page: std::env::args().any(|arg| arg == "--discover"),
            discover: Default::default(),
            provider_browser: Default::default(),
            update_status: if cfg!(canna_rebound_local_preview) {
                "Canna Bliss local preview; automatic updates disabled".into()
            } else {
                String::new()
            },
            pending_update: None,
            console_page: std::env::args().any(|arg| arg == "--console"),
            console: console::Console::new(),
            console_polling: false,
            last_console_poll: std::time::Instant::now() - std::time::Duration::from_secs(2),
            launch_watch: None,
            game_details: std::env::args().any(|arg| arg == "--game-details"),
            runtime_busy: false,
            runtime_enabled: start_jobs,
            runtime_status: if cfg!(canna_rebound_local_preview) {
                "Canna Bliss local preview · enable the ROUNDS preview in Settings".into()
            } else {
                String::new()
            },
            #[cfg(test)]
            card_create_rects: BTreeMap::new(),
            #[cfg(test)]
            card_view_rects: BTreeMap::new(),
            modpacks_page: std::env::args().any(|arg| {
                matches!(
                    arg.as_str(),
                    "--modpacks"
                        | "--new-pack"
                        | "--create-menu"
                        | "--new-group"
                        | "--pack-details"
                )
            }),
            pack_ui: pack_ui::PackUi::new(),
            settings,
            token: website::session(),
            settings_open: false,
            query: String::new(),
            supported_only: false,
            games: vec![],
            catalog: model::supported_catalog(),
            libraries: vec![],
            warnings: vec![],
            textures: BTreeMap::new(),
            repository_textures: BTreeMap::new(),
            active_source: None,
            cached_at: None,
            selected: 1686940,
            scanning: false,
            last_steam_scan: std::time::Instant::now(),
            steam_was_focused: true,
            syncing: false,
            scan_status: String::new(),
            repo_status: "Log in to connect your Canna account".into(),
            tx,
            rx,
            screenshot: std::env::var_os("CANNA_SCREENSHOT").map(Into::into),
            screenshot_requested: false,
            ready_frames: 0,
        };
        if std::env::args().any(|arg| arg == "--new-pack") {
            app.pack_ui.start_new(&model::bopl(), None);
        }
        if std::env::args().any(|arg| arg == "--create-menu") {
            app.pack_ui.open_creation_menu();
        }
        if std::env::args().any(|arg| arg == "--new-group") {
            app.pack_ui.open_group_dialog();
        }
        if std::env::args().any(|arg| arg == "--pack-details") {
            app.pack_ui.open_first_pack();
        }
        if app.screenshot.is_some() && std::env::args().any(|a| a == "--discover-game-preview") {
            app.discover_page = true;
            app.discover.game = 1686940;
            app.provider_browser.mode = 1;
            app.catalog[0].mods = vec![model::ModInfo {
                provenance: serde_json::Value::Null,
                content_type: String::new(),
                enabled: true,
                name: "Preview mod".into(),
                version: "1.0".into(),
                description: "Visual fixture only".into(),
                file: "Mods/preview.zip".into(),
                sha256: String::new(),
                local_file: String::new(),
                dependencies: vec![],
            }];
        }
        if account_preview {
            let profile = std::env::args().any(|a| a == "--account-profile-preview");
            app.token = if profile {
                "ui-fixture".into()
            } else {
                String::new()
            };
            app.account.preview(profile);
            if std::env::args().any(|a| a == "--account-code-preview") {
                app.website.preview_connection();
            }
        }
        if provider_preview {
            app.token = "ui-fixture".into();
            app.discover_page = true;
            app.provider_browser.preview_fixture();
        }
        if start_jobs {
            if !cfg!(canna_rebound_local_preview) {
                app.update_status = "Checking for Canna updates…".into();
                let tx = app.tx.clone();
                let repaint = ctx.clone();
                std::thread::spawn(move || {
                    let result =
                        updater::check(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string());
                    let _ = tx.send(Event::Update(result));
                    repaint.request_repaint();
                });
            }
            app.scan(ctx);
            if configured {
                app.sync(ctx);
            }
        }
        app
    }
    fn scan(&mut self, ctx: &egui::Context) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.last_steam_scan = std::time::Instant::now();
        self.scan_status = "Scanning Steam libraries…".into();
        let path = self.settings.steam_path.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Scanned(steam::scan(&path)));
            ctx.request_repaint();
        });
    }
    fn sync(&mut self, ctx: &egui::Context) {
        if self.syncing {
            return;
        }
        self.syncing = true;
        self.repo_status = "Reading Canna server catalog…".into();
        let s = self.settings.clone();
        let source = cache::Source::from_settings(&s);
        if self.active_source.as_ref() != Some(&source) {
            self.catalog = model::supported_catalog();
            self.repository_textures.clear();
            self.cached_at = None;
        }
        self.active_source = Some(source.clone());
        let token = self.token.clone();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let cached = cache::load(&source).map_err(|e| e.to_string());
            let _ = tx.send(Event::Cached(source.clone(), cached));
            ctx.request_repaint();
            let r = repository::sync(&s, &token)
                .map(|mut data| {
                    if let Err(error) = cache::save(&source, &data) {
                        data.warnings
                            .push(format!("Could not save offline catalog: {error}"));
                    }
                    data
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(Event::Synced(source, r));
            ctx.request_repaint();
        });
    }
    fn texture(&mut self, ctx: &egui::Context, id: u32, bytes: &[u8], repository: bool) {
        // Bound decoded dimensions as well as downloaded file size.
        let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()
        else {
            return;
        };
        let mut reader = reader;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        if let Ok(img) = reader.decode() {
            let img = img.thumbnail(600, 900).to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let textures = if repository {
                &mut self.repository_textures
            } else {
                &mut self.textures
            };
            textures.insert(
                id,
                ctx.load_texture(
                    format!("game-{id}"),
                    egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()),
                    egui::TextureOptions::LINEAR,
                ),
            );
        }
    }
    fn events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Update(result) => {
                    if cfg!(canna_rebound_local_preview) {
                        self.pending_update = None;
                        self.update_status =
                            "Canna Bliss local preview; automatic updates disabled".into();
                        continue;
                    }
                    match result {
                        Ok(Some(ready)) => {
                            self.update_status = format!(
                                "Canna {} downloaded; restarting when current work finishes",
                                ready.version
                            );
                            self.pending_update = Some(ready);
                        }
                        Ok(None) => {
                            self.update_status =
                                format!("Canna {} is up to date", env!("CARGO_PKG_VERSION"))
                        }
                        Err(error) => self.update_status = format!("Update check: {error}"),
                    }
                    self.console.record(&self.update_status, &self.token);
                }
                Event::Launched(id, modded, requested, owned) => {
                    self.console.record(
                        &format!(
                            "Retained game process {}. Stop instance is available.",
                            owned.pid
                        ),
                        &self.token,
                    );
                    self.owned_games.insert(id, owned);
                    self.console.game_id = id;
                    self.launch_watch = Some(console::LaunchWatch::new(id, modded, requested));
                    self.last_console_poll =
                        std::time::Instant::now() - std::time::Duration::from_secs(2);
                }
                Event::ConsoleData(data) => {
                    self.console_polling = false;
                    for (id, snapshot) in data {
                        if let Some(watch) = &mut self.launch_watch
                            && watch.game_id == id
                        {
                            let (status, done) = watch.update(&snapshot);
                            if self.diagnostics.is_enabled() {
                                let code = if status.contains("process status unavailable") {
                                    Some("process_status_unavailable")
                                } else if status.contains("process was not detected") {
                                    Some("process_not_detected")
                                } else if watch.modded {
                                    diagnostics::startup_code(&snapshot, watch.requested)
                                } else {
                                    None
                                };
                                if let Some(code) = code {
                                    let loader_present =
                                        self.games.iter().find(|g| g.app_id == id).is_some_and(
                                            |g| g.path.join("BepInEx/core/BepInEx.dll").is_file(),
                                        );
                                    self.diagnostics.submit(diagnostics::startup_report(
                                        id,
                                        code,
                                        self.settings.rebound_enabled,
                                        loader_present,
                                    ));
                                }
                            }
                            if self.runtime_status != status {
                                self.console.record(&status, &self.token);
                                self.runtime_status = status;
                                self.pack_ui.set_runtime_status(&self.runtime_status);
                            }
                            if done {
                                self.launch_watch = None;
                            }
                        }
                        self.console.update(id, snapshot);
                    }
                }
                Event::PackPrepared(pack) => self.pack_ui.observe_prepared_pack(*pack),
                Event::RuntimeProgress(message) => {
                    self.console.record(&message, &self.token);
                    self.runtime_status = message;
                    self.pack_ui.set_runtime_status(&self.runtime_status);
                }
                Event::Runtime(result) => {
                    self.runtime_busy = false;
                    if result.is_err() {
                        self.pack_ui.runtime_requests.clear();
                    }
                    self.runtime_status = match result {
                        Ok(message) => message,
                        Err(error) => format!("Could not complete mod setup: {error}"),
                    };
                    self.pack_ui.set_runtime_status(&self.runtime_status);
                    self.console.record(&self.runtime_status, &self.token);
                    self.scan(ctx);
                }
                Event::Scanned(scan) => {
                    self.scanning = false;
                    let changed = self.games.len() != scan.games.len()
                        || scan.games.iter().any(|g| {
                            self.games
                                .iter()
                                .find(|old| old.app_id == g.app_id)
                                .is_none_or(|old| {
                                    old.path != g.path
                                        || old.loader != g.loader
                                        || old.plugins != g.plugins
                                })
                        });
                    for g in &scan.games {
                        if !self.textures.contains_key(&g.app_id)
                            && let Some(b) = &g.icon
                        {
                            self.texture(ctx, g.app_id, b, false)
                        }
                    }
                    self.scan_status = format!(
                        "{} Supported games · {} Steam libraries · {} other entries excluded",
                        scan.games.len(),
                        scan.libraries.len(),
                        scan.excluded
                    );
                    self.games = scan.games;
                    self.pack_ui.lab_games = self.games.clone();
                    self.libraries = scan.libraries;
                    for warning in scan.warnings {
                        if !self.warnings.contains(&warning) {
                            self.warnings.push(warning);
                        }
                    }
                    if changed {
                        self.console.record(&self.scan_status, &self.token);
                    }
                }
                Event::Cached(source, result) => {
                    if self.active_source.as_ref() != Some(&source) {
                        continue;
                    }
                    match result {
                        Ok(Some(data)) => {
                            self.cached_at = data.cached_at;
                            self.repo_status = format!(
                                "Cached catalog · last synced {} · checking server…",
                                cache::age_label(data.cached_at.unwrap_or(0))
                            );
                            self.apply_catalog(ctx, data);
                        }
                        Ok(None) => {}
                        Err(error) => self
                            .warnings
                            .push(format!("Offline catalog unavailable: {error}")),
                    }
                }
                Event::Synced(source, result) => {
                    if self.active_source.as_ref() != Some(&source) {
                        continue;
                    }
                    self.syncing = false;
                    match result {
                        Ok(data) => {
                            self.cached_at = None;
                            self.repo_status = format!(
                                "Connected · {}/{} · {} games",
                                source.owner,
                                source.repository,
                                data.games.len()
                            );
                            self.apply_catalog(ctx, data);
                        }
                        Err(e) => {
                            self.repo_status = if let Some(saved_at) = self.cached_at {
                                format!(
                                    "Offline catalog · synced {} · {e}",
                                    cache::age_label(saved_at)
                                )
                            } else {
                                format!("Repository unavailable · {e}")
                            }
                        }
                    }
                }
            }
        }
    }
    fn apply_catalog(&mut self, ctx: &egui::Context, data: repository::RepositoryData) {
        self.repository_textures.clear();
        for (id, bytes) in data.icons {
            self.texture(ctx, id, &bytes, true);
        }
        self.catalog = data
            .games
            .into_iter()
            .filter(|g| model::supported_game(g.app_id))
            .collect();
        for game in model::supported_catalog() {
            if !self.catalog.iter().any(|entry| entry.app_id == game.app_id) {
                self.catalog.push(game);
            }
        }
        self.warnings.extend(data.warnings);
    }
    fn settings_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Canna settings")
            .open(&mut open)
            .default_width(500.0)
            .vscroll(true)
            .max_height((ctx.content_rect().height() - 80.0).max(240.0))
            .show(ctx, |ui| {
                ui.strong("CANNA SERVER");
                if ui.checkbox(&mut self.settings.low_end,"Low-end PC mode").changed(){let _=self.settings.save();}
                ui.label("Low-end mode reduces background scans and console refresh frequency; it does not change game graphics or memory settings.");
                ui.label("cannamods.vip · private community library");
                ui.label(if self.token.is_empty() {
                    "Not connected"
                } else {
                    "Account connected · signed in until you log out"
                });
                if ui
                    .add_enabled(
                        !self.website.connecting(),
                        egui::Button::new("Sign in & connect account"),
                    )
                    .clicked()
                {
                    self.account.open(ctx, &self.token);
                }
                if !self.token.is_empty() && ui.button("Manage logged-in devices").clicked() {
                    ctx.open_url(egui::OpenUrl::new_tab("https://cannamods.vip/?devices=1"));
                }
                if !self.website.account_status.is_empty() {
                    ui.label(&self.website.account_status);
                }
                if self.website.connecting() && ui.button("Cancel connection").clicked() {
                    self.website.cancel_sign_in();
                }
                if ui
                    .add_enabled(
                        !self.token.is_empty(),
                        egui::Button::new("Disconnect account"),
                    )
                    .clicked()
                {
                    website::disconnect();
                    self.token.clear();
                    self.catalog = model::supported_catalog();
                    self.repository_textures.clear();
                    self.repo_status = "Account disconnected".into();
                }
                ui.separator();
                ui.strong("OPTIONAL DIAGNOSTICS");
                if ui.checkbox(&mut self.settings.anonymous_reports, "Send anonymous launcher error reports").changed() {
                    match self.settings.save() {
                        Ok(()) => self.diagnostics.enable(self.settings.anonymous_reports),
                        Err(error) => {
                            self.settings.anonymous_reports = !self.settings.anonymous_reports;
                            self.runtime_status = format!("Could not save reporting preference: {error}");
                        }
                    }
                }
                ui.label("Off by default. Reports contain error categories, setup stage, game ID, app version, mod count and loader/Bliss flags. The same setting enables anonymous Bliss compatibility hashes. No raw logs, personal paths, usernames, tokens or modpack names are uploaded. Reports expire after seven days.");
                ui.label(self.diagnostics.status());
                ui.separator();
                if self.rebound_access.needs_check() {
                    self.rebound_access.refresh(ctx);
                }
                if rebound_settings(ui, &mut self.settings.rebound_enabled, self.runtime_busy, self.rebound_access.allowed).changed()
                    && let Err(error) = self.settings.save() {
                    self.settings.rebound_enabled = !self.settings.rebound_enabled;
                    self.runtime_status = format!("Could not save Bliss preference: {error}");
                }
                if !self.rebound_access.status.is_empty() {
                    ui.label(&self.rebound_access.status);
                }
                if ui.add_enabled(!self.rebound_access.checking(), egui::Button::new("Recheck Beta access")).clicked() {
                    self.rebound_access.refresh(ctx);
                }
                ui.separator();
                ui.label("Steam location override (optional)");
                ui.text_edit_singleline(&mut self.settings.steam_path);
                if ui.button("Save, scan & sync server").clicked() {
                    let _ = self.settings.save();
                    self.scan(ctx);
                    self.sync(ctx);
                }
            });
        self.settings_open = open && !self.account.open;
    }
    fn art(&self, ui: &mut egui::Ui, id: u32, size: egui::Vec2) {
        self.art_tinted(ui, id, size, Color32::WHITE);
    }
    fn art_tinted(&self, ui: &mut egui::Ui, id: u32, size: egui::Vec2, tint: Color32) {
        let texture = if id == u32::MAX {
            Some(self.chrome.minecraft_banner())
        } else {
            self.repository_textures
                .get(&id)
                .or_else(|| self.textures.get(&id))
        };
        if let Some(t) = texture {
            let src = t.size_vec2();
            let ratio = size.x / size.y;
            let source_ratio = src.x / src.y;
            let uv = if source_ratio > ratio {
                let width = ratio / source_ratio;
                egui::Rect::from_min_max(
                    egui::pos2((1.0 - width) * 0.5, 0.0),
                    egui::pos2((1.0 + width) * 0.5, 1.0),
                )
            } else {
                let height = source_ratio / ratio;
                egui::Rect::from_min_max(
                    egui::pos2(0.0, (1.0 - height) * 0.5),
                    egui::pos2(1.0, (1.0 + height) * 0.5),
                )
            };
            ui.add(
                egui::Image::new(t)
                    .tint(tint)
                    .uv(uv)
                    .maintain_aspect_ratio(false)
                    .fit_to_exact_size(size)
                    .corner_radius(ui_helpers::SURFACE_RADIUS),
            );
        } else {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                ui_helpers::SURFACE_RADIUS,
                if tint == Color32::WHITE {
                    Color32::from_rgb(44, 66, 45)
                } else {
                    Color32::from_gray(35)
                },
            );
            ui.painter().circle_filled(
                rect.center(),
                size.x.min(size.y) * 0.22,
                if tint == Color32::WHITE {
                    Color32::from_rgb(116, 163, 99)
                } else {
                    Color32::from_gray(65)
                },
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if id == 1686940 { "B" } else { "C" },
                egui::FontId::proportional(34.0),
                Color32::from_rgb(21, 35, 23),
            );
        }
    }
}
impl Canna {
    fn queue_vanilla_cleanup(&mut self, id: u32) {
        if self.games.iter().any(|game| {
            game.app_id == id && id != u32::MAX
        }) && !self.pack_ui.runtime_requests.iter().any(|request| {
            matches!(request, pack_ui::RuntimeAction::RestoreVanilla(queued) if *queued == id)
        }) {
            self.pack_ui.runtime_requests.push_front(pack_ui::RuntimeAction::RestoreVanilla(id));
        }
    }
    fn stop_game(&mut self, id: u32) {
        let result = self.owned_games.get(&id).map(|game| game.stop());
        self.runtime_status = match result {
            Some(Ok(())) => {
                self.launch_watch = None;
                if self.owned_games.get(&id).is_some_and(|game| game.running()) {
                    // TerminateProcess is asynchronous. Retain ownership so the
                    // normal exit poll schedules cleanup after the process closes.
                    "Stopping game. Waiting for its process to close…".into()
                } else {
                    self.owned_games.remove(&id);
                    self.queue_vanilla_cleanup(id);
                    if id != u32::MAX && model::source_addons(id).is_none() {
                        "Game stopped. Restoring vanilla files…"
                    } else {
                        "Game stopped."
                    }
                    .into()
                }
            }
            Some(Err(error)) => format!("Could not stop game: {error}"),
            None => "No running game owned by Canna.".into(),
        };
        self.pack_ui.set_runtime_status(&self.runtime_status);
        self.console.record(&self.runtime_status, &self.token);
    }
    fn open_discover(&mut self) {
        if self.discover_page && !self.website.open && !self.skins.open && !self.minecraft_page {
            self.discover.game = 0;
            self.discover.query.clear();
        }
        self.website.open = false;
        self.skins.open = false;
        self.minecraft_page = false;
        self.console_page = false;
        self.discover_page = true;
    }
    fn discover_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.provider_browser.mode, 0, "Browse providers");
            ui.selectable_value(
                &mut self.provider_browser.mode,
                1,
                "Community mods & modpacks",
            );
            ui.selectable_value(&mut self.provider_browser.mode, 2, "Subscriptions");
            if !self.token.is_empty()
                && ui
                    .add_enabled(!self.syncing, egui::Button::new("Refresh library"))
                    .clicked()
            {
                self.sync(ui.ctx());
            }
        });
        if self.provider_browser.mode != 1 {
            self.provider_browser.observe_installations(&self.games);
            if self.token.is_empty() && ui.button("Sign in & connect account").clicked() {
                self.account.open(ui.ctx(), &self.token);
            }
            self.provider_browser.show(
                ui,
                &self.catalog,
                self.active_source.as_ref(),
                &mut self.pack_ui,
                &mut self.discover.target,
            );
            return;
        }

        ui.horizontal_wrapped(|ui| {
            if self.token.is_empty()
                || self.repo_status.contains("expired")
                || self.repo_status.contains("revoked")
            {
                ui.heading("Connect to Canna");
                ui.label(
                    "Sign in and approve this app on the website to browse your community's mods.",
                );
                if ui
                    .add_enabled(
                        !self.website.connecting(),
                        egui::Button::new("Sign in & connect account"),
                    )
                    .clicked()
                {
                    self.account.open(ui.ctx(), &self.token);
                }
                if !self.website.account_status.is_empty() {
                    ui.label(&self.website.account_status);
                }
            } else if ui
                .add_enabled(!self.syncing, egui::Button::new("Refresh library"))
                .clicked()
            {
                self.sync(ui.ctx());
            }
            if ui.button("Add mod from external site").clicked() {
                let game = if self.discover.game == u32::MAX {
                    "minecraft".into()
                } else {
                    self.discover.game.to_string()
                };
                ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                    "https://cannamods.vip/?import=1&game={game}"
                )));
            }
        });
        if self.discover.game == 0 {
            ui.heading("Discover games");
            ui.label("Choose a game to browse its mods.");
            ui.add_space(12.0);
            let mut games: Vec<_> = self
                .catalog
                .iter()
                .filter(|g| model::supported_game(g.app_id))
                .cloned()
                .collect();
            if !games.iter().any(|g| g.app_id == u32::MAX) {
                games.insert(
                    0,
                    GameInfo {
                        app_id: u32::MAX,
                        name: "Minecraft".into(),
                        folder: "minecraft".into(),
                        description: String::new(),
                        icon: String::new(),
                        mods: vec![],
                        mod_folder_status: String::new(),
                    },
                );
            }
            games.sort_by_key(|g| g.name.to_lowercase());
            egui::ScrollArea::vertical().show(ui, |ui| {
                let columns = ((ui.available_width() / 220.0) as usize).max(1);
                egui::Grid::new("discover_game_grid")
                    .num_columns(columns)
                    .spacing(egui::vec2(12.0, 12.0))
                    .show(ui, |ui| {
                        for (index, game) in games.into_iter().enumerate() {
                            let frame = egui::Frame::new()
                                .fill(Color32::from_rgb(29, 39, 33))
                                .corner_radius(ui_helpers::SURFACE_RADIUS)
                                .inner_margin(12)
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.set_width(184.0);
                                        self.art(ui, game.app_id, egui::vec2(184.0, 103.5));
                                        ui.add_space(6.0);
                                        ui.label(
                                            egui::RichText::new(&game.name).size(18.0).strong(),
                                        );
                                        ui.label(if self.token.is_empty() {
                                            "Connect to browse".into()
                                        } else {
                                            format!("{} mods", game.mods.len())
                                        });
                                    });
                                });
                            if frame
                                .response
                                .interact(egui::Sense::click())
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.discover.game = game.app_id;
                                self.discover.query.clear();
                                self.discover.kind.clear();
                            }
                            if (index + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        } else {
            ui.horizontal(|ui| {
                if self
                    .chrome
                    .nav(ui, 8, "Back to Discover games", false)
                    .clicked()
                {
                    self.discover.game = 0;
                    self.discover.query.clear();
                }
                if let Some(game) = self.catalog.iter().find(|g| g.app_id == self.discover.game) {
                    ui.heading(&game.name);
                }
            });
            if self.discover.game == 0 {
                return;
            }
            self.pack_ui.discover(
                ui,
                &self.catalog,
                self.active_source.as_ref(),
                &mut self.discover,
                self.syncing || self.runtime_busy,
            );
        }
    }
    fn queue_game_launch(&mut self, game: &InstalledGame, info: &GameInfo, modded: bool) {
        let pack = modpacks::Modpack::create(
            format!("{} current setup", game.name),
            String::new(),
            info,
            cache::Source::from_settings(&self.settings),
            vec![],
        );
        self.pack_ui.runtime_requests.push_back(if modded {
            pack_ui::RuntimeAction::LaunchCurrent(game.app_id)
        } else {
            pack_ui::RuntimeAction::Launch(pack, false)
        });
    }
    fn run_runtime(&mut self, ctx: &egui::Context) {
        if self.runtime_busy || !self.runtime_enabled {
            return;
        }
        let Some(request) = self.pack_ui.runtime_requests.pop_front() else {
            return;
        };
        if let pack_ui::RuntimeAction::Stop(id) = request {
            self.stop_game(id);
            return;
        }
        let id = match &request {
            pack_ui::RuntimeAction::Stop(_) => unreachable!(),
            pack_ui::RuntimeAction::Setup(p)
            | pack_ui::RuntimeAction::Install(p)
            | pack_ui::RuntimeAction::Launch(p, _) => p.game.app_id,
            pack_ui::RuntimeAction::LaunchCurrent(id)
            | pack_ui::RuntimeAction::RestoreVanilla(id) => *id,
        };
        let Some(game) = self.games.iter().find(|g| g.app_id == id).cloned() else {
            self.runtime_status = "Install this game through Steam first.".into();
            return;
        };
        let diagnostic_operation = match &request {
            pack_ui::RuntimeAction::Install(_) | pack_ui::RuntimeAction::Setup(_) => "prepare",
            pack_ui::RuntimeAction::Launch(_, true) => "modded_launch",
            pack_ui::RuntimeAction::Launch(_, false) => "vanilla_launch",
            pack_ui::RuntimeAction::LaunchCurrent(_) => "current_launch",
            pack_ui::RuntimeAction::RestoreVanilla(_) => "restore",
            pack_ui::RuntimeAction::Stop(_) => unreachable!(),
        };
        let diagnostic_mod_count = match &request {
            pack_ui::RuntimeAction::Install(p)
            | pack_ui::RuntimeAction::Setup(p)
            | pack_ui::RuntimeAction::Launch(p, _) => p.mods.iter().filter(|m| m.enabled).count(),
            _ => 0,
        };
        self.runtime_busy = true;
        self.runtime_status = if matches!(request, pack_ui::RuntimeAction::RestoreVanilla(_)) {
            "Restoring vanilla files…"
        } else {
            "Preparing game…"
        }
        .into();
        self.pack_ui.set_runtime_status(&self.runtime_status);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let token = self.token.clone();
        let options = runtime::InstallOptions {
            rebound_enabled: self.settings.rebound_enabled,
        };
        let diagnostic_reporter = self.diagnostics.clone();
        std::thread::spawn(move || {
            let diagnostic_stage = std::sync::Mutex::new(if diagnostic_operation == "restore" {
                "restore"
            } else {
                "prepare"
            });
            let progress = |message: &str| {
                if let Ok(mut stage) = diagnostic_stage.lock() {
                    *stage = diagnostics::phase(message);
                }
                let _ = tx.send(Event::RuntimeProgress(message.into()));
                ctx.request_repaint();
            };
            let result = (|| -> anyhow::Result<String> {
                match request {
                    pack_ui::RuntimeAction::Stop(_) => unreachable!(),
                    pack_ui::RuntimeAction::RestoreVanilla(_) => runtime::restore_vanilla(&game),
                    pack_ui::RuntimeAction::Setup(_) => Ok(
                        "Pack selections saved. Launch modded prepares and activates them.".into(),
                    ),
                    pack_ui::RuntimeAction::Install(pack) => {
                        let pack = pack_updates::refresh(&pack, &token, &progress)?;
                        let prepared =
                            runtime::prepare_install(&game, &pack, &token, options, &progress)?;
                        prepared.cache_downloads(&progress)?;
                        pack.save()?;
                        let _ = tx.send(Event::PackPrepared(Box::new(pack)));
                        Ok("Downloads prepared. Launch modded activates the selected mods.".into())
                    }
                    pack_ui::RuntimeAction::Launch(pack, modded) => {
                        if modded {
                            let pack = pack_updates::refresh(&pack, &token, &progress)?;
                            let prepared =
                                runtime::prepare_install(&game, &pack, &token, options, &progress)?;
                            pack.save()?;
                            let _ = tx.send(Event::PackPrepared(Box::new(pack)));
                            let applied = prepared.effective_pack().clone();
                            play_backup::before_change(&game, &applied)?;
                            runtime::install_prepared(&game, prepared, &token, &progress)
                                .inspect_err(|_| {
                                    if runtime::ensure_closed(&game).is_ok() {
                                        let _ = runtime::restore_vanilla(&game);
                                    }
                                })?;
                            play_backup::remember_applied(&game, &applied)?;
                        }
                        let requested = std::time::SystemTime::now();
                        progress("Launching through Steam…");
                        let owned = runtime::launch(&game, modded).inspect_err(|_| {
                            if modded && runtime::ensure_closed(&game).is_ok() {
                                let _ = runtime::restore_vanilla(&game);
                            }
                        })?;
                        let _ = tx.send(Event::Launched(game.app_id, modded, requested, owned));
                        Ok(if modded {
                            "Steam launch requested (modded)."
                        } else {
                            "Steam launch requested (vanilla)."
                        }
                        .into())
                    }
                    pack_ui::RuntimeAction::LaunchCurrent(_) => {
                        if let Some(last) = play_backup::last_applied(&game)? {
                            let pack = modpacks::load_all()
                                .0
                                .into_iter()
                                .find(|pack| pack.id == last.id)
                                .unwrap_or(last);
                            let pack = pack_updates::refresh(&pack, &token, &progress)?;
                            let prepared =
                                runtime::prepare_install(&game, &pack, &token, options, &progress)?;
                            pack.save()?;
                            let _ = tx.send(Event::PackPrepared(Box::new(pack)));
                            let applied = prepared.effective_pack().clone();
                            play_backup::before_change(&game, &applied)?;
                            runtime::install_prepared(&game, prepared, &token, &progress)
                                .inspect_err(|_| {
                                    if runtime::ensure_closed(&game).is_ok() {
                                        let _ = runtime::restore_vanilla(&game);
                                    }
                                })?;
                            play_backup::remember_applied(&game, &applied)?;
                        }
                        let requested = std::time::SystemTime::now();
                        progress("Launching current setup through Steam…");
                        let owned = runtime::launch_current(&game, &token, options, &progress)
                            .inspect_err(|_| {
                                if runtime::ensure_closed(&game).is_ok() {
                                    let _ = runtime::restore_vanilla(&game);
                                }
                            })?;
                        let _ = tx.send(Event::Launched(game.app_id, true, requested, owned));
                        Ok("Steam launch requested (current modded setup).".into())
                    }
                }
            })()
            .map_err(|e| format!("{e:#}"));
            if let Err(error) = &result {
                let stage = diagnostic_stage.lock().map(|v| *v).unwrap_or("prepare");
                diagnostic_reporter.submit(diagnostics::report(
                    id,
                    diagnostic_operation,
                    stage,
                    error,
                    diagnostic_mod_count,
                    options.rebound_enabled,
                    game.path.join("BepInEx/core/BepInEx.dll").is_file(),
                ));
            }
            let _ = tx.send(Event::Runtime(result));
            ctx.request_repaint();
        });
    }
    fn render(&mut self, ctx: &egui::Context) {
        for message in std::mem::take(&mut self.pack_ui.runtime_progress) {
            self.console.record(&message, &self.token);
        }
        self.pack_ui.runtime_options = runtime::InstallOptions {
            rebound_enabled: self.settings.rebound_enabled,
        };
        self.pack_ui.lab_memory = self
            .owned_games
            .iter()
            .filter_map(|(id, game)| game.memory().map(|m| (*id, m)))
            .collect();
        ctx.layer_painter(egui::LayerId::background()).rect_filled(
            ctx.viewport_rect(),
            0,
            chrome::CANVAS,
        );
        self.events(ctx);
        let exited: Vec<_> = self
            .owned_games
            .iter()
            .filter_map(|(id, game)| (!game.running()).then_some(*id))
            .collect();
        for id in exited {
            self.owned_games.remove(&id);
            self.queue_vanilla_cleanup(id);
        }
        self.pack_ui.owned_games = self.owned_games.keys().copied().collect();
        if !self.owned_games.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
        if let Some(id) = self.pack_ui.discover_pack.take() {
            self.provider_browser
                .select_game(self.pack_ui.pack_game(&id).unwrap_or(0));
            self.discover.game = self.pack_ui.pack_game(&id).unwrap_or(0);
            self.discover.target = Some(id);
            self.discover_page = true;
            self.console_page = false;
        }
        if !cfg!(canna_rebound_local_preview)
            && self.pending_update.is_some()
            && !self.runtime_busy
            && !self.provider_browser.busy()
            && !self.website.busy()
            && !self.minecraft.busy()
            && !self.skins.busy()
            && self.owned_games.is_empty()
            && !self.pack_ui.editing()
            && !self.settings_open
            && !self.account.open
            && !self.account.busy()
            && self.pack_ui.runtime_requests.is_empty()
        {
            let ready = self.pending_update.take().unwrap();
            match updater::apply(&ready) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(error) => {
                    self.update_status = format!("Could not apply update: {error}");
                    self.console.record(&self.update_status, &self.token);
                }
            }
        }
        if self.pack_ui.discover_return {
            self.pack_ui.discover_return = false;
            self.discover_page = false;
            self.modpacks_page = true;
        }
        while let Some(index) = self
            .pack_ui
            .runtime_requests
            .iter()
            .position(|request| matches!(request, pack_ui::RuntimeAction::Stop(_)))
        {
            if let Some(pack_ui::RuntimeAction::Stop(id)) =
                self.pack_ui.runtime_requests.remove(index)
            {
                self.stop_game(id);
            }
        }
        if let Some(pack) = self.pack_ui.health_pack.take() {
            let id = pack.game.app_id;
            if let Some(game) = self.games.iter().find(|g| g.app_id == id) {
                self.console
                    .record(&runtime_health::report(game, Some(&pack)), &self.token);
                self.console_page = true;
                self.console.game_id = id;
            }
        }
        self.run_runtime(ctx);
        if let Some(id) = self.pack_ui.console_game.take() {
            self.console_page = true;
            self.console.game_id = id;
        }
        if self.runtime_enabled && (self.console_page || self.launch_watch.is_some()) {
            let interval = if self.settings.low_end && self.launch_watch.is_none() {
                5
            } else {
                1
            };
            ctx.request_repaint_after(std::time::Duration::from_secs(interval));
            if !self.console_polling && self.last_console_poll.elapsed().as_secs() >= interval {
                let mut ids = std::collections::BTreeSet::new();
                if self.console_page {
                    ids.insert(self.console.game_id);
                }
                if let Some(watch) = &self.launch_watch {
                    ids.insert(watch.game_id);
                }
                let games: Vec<_> = self
                    .games
                    .iter()
                    .filter(|g| ids.contains(&g.app_id))
                    .cloned()
                    .collect();
                self.console_polling = true;
                self.last_console_poll = std::time::Instant::now();
                let tx = self.tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let data = games
                        .iter()
                        .map(|game| (game.app_id, console::collect(game)))
                        .collect();
                    let _ = tx.send(Event::ConsoleData(data));
                    ctx.request_repaint();
                });
            }
        }
        if let Some(path) = &self.screenshot {
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    match image::save_buffer(
                        path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => {
                            eprintln!("Cannot save screenshot: {e}");
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
            }
            if !self.scanning
                && !self.syncing
                && !self.skins.busy()
                && !self.screenshot_requested
                && (!self.console_page || self.games.is_empty() || self.console.has_snapshot())
            {
                self.ready_frames += 1;
                if self.ready_frames >= 3 {
                    self.screenshot_requested = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
            }
            ctx.request_repaint();
        }
        let notices = [
            self.repo_status.as_str(),
            self.runtime_status.as_str(),
            self.update_status.as_str(),
        ];
        let notice = notices.iter().find(|s| {
            let s = s.to_lowercase();
            s.contains("failed")
                || s.contains("unavailable")
                || s.contains("expired")
                || s.contains("error")
        });
        if chrome::title_bar(
            ctx,
            if self.token.is_empty() {
                "Log in"
            } else {
                "Account"
            },
        )
        .account_clicked
        {
            self.settings_open = false;
            self.account.open(ctx, &self.token);
        }
        let notice = notice
            .copied()
            .map(|s| (9, s))
            .or_else(|| self.warnings.first().map(|s| (10, s.as_str())));
        if let Some((icon, message)) = notice {
            egui::Area::new("status_notice".into())
                .anchor(egui::Align2::RIGHT_TOP, [-146.0, 3.0])
                .show(ctx, |ui| {
                    if self
                        .chrome
                        .nav(ui, icon, "View status", false)
                        .on_hover_text(message)
                        .clicked()
                    {
                        self.settings_open = true;
                    }
                });
        }
        egui::SidePanel::left("navigation")
            .exact_width(80.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .corner_radius(0)
                    .fill(Color32::from_rgb(23, 32, 27))
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(12.0, 12.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.add_space(6.0);
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Image::new(self.chrome.logo())
                            .fit_to_exact_size(egui::vec2(44.0, 44.0)),
                    )
                    .on_hover_text("canna · Made for the family.");
                });
                ui.add_space(24.0);
                egui::ScrollArea::vertical()
                    .id_salt("navigation-items")
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - 84.0).max(80.0))
                    .show(ui, |ui| {
                        if self
                            .chrome
                            .nav(
                                ui,
                                0,
                                "Game library",
                                !self.website.open
                                    && !self.skins.open
                                    && !self.modpacks_page
                                    && !self.console_page
                                    && !self.discover_page
                                    && !self.minecraft_page,
                            )
                            .clicked()
                        {
                            self.account.hide();
                            self.website.open = false;
                            self.skins.open = false;
                            self.minecraft_page = false;
                            self.discover_page = false;
                            self.modpacks_page = false;
                            self.game_details = false;
                            self.console_page = false;
                        }
                        if chrome::Chrome::packs(
                            ui,
                            !self.website.open
                                && !self.skins.open
                                && self.modpacks_page
                                && !self.console_page
                                && !self.discover_page
                                && !self.minecraft_page,
                        )
                        .clicked()
                        {
                            self.account.hide();
                            self.website.open = false;
                            self.skins.open = false;
                            self.minecraft_page = false;
                            self.discover_page = false;
                            self.modpacks_page = true;
                            self.console_page = false;
                        }
                        if self
                            .chrome
                            .nav(
                                ui,
                                1,
                                "Discover",
                                self.discover_page && !self.website.open && !self.skins.open,
                            )
                            .clicked()
                        {
                            self.account.hide();
                            self.open_discover();
                        }
                        if self
                            .chrome
                            .nav(
                                ui,
                                2,
                                "Console",
                                self.console_page && !self.website.open && !self.skins.open,
                            )
                            .clicked()
                        {
                            self.account.hide();
                            self.website.open = false;
                            self.skins.open = false;
                            self.minecraft_page = false;
                            self.discover_page = false;
                            self.console_page = true;
                            self.last_console_poll =
                                std::time::Instant::now() - std::time::Duration::from_secs(2);
                        }
                        if self
                            .chrome
                            .nav(ui, 5, "Minecraft", self.minecraft_page)
                            .clicked()
                        {
                            self.account.hide();
                            self.minecraft_page = true;
                            self.website.open = false;
                            self.skins.open = false;
                            self.discover_page = false;
                            self.console_page = false;
                        }
                        if self.chrome.nav(ui, 4, "Skins", self.skins.open).clicked() {
                            self.account.hide();
                            self.website.open = false;
                            self.skins.open = false;
                            self.minecraft_page = false;
                            self.skins.open_browser();
                        }
                        if self
                            .chrome
                            .nav(ui, 7, "Downloads", self.website.open)
                            .clicked()
                        {
                            self.account.hide();
                            self.minecraft_page = false;
                            self.website.open = true;
                            self.skins.open = false;
                        }
                        if self
                            .chrome
                            .nav(ui, 3, "Settings", self.settings_open)
                            .clicked()
                        {
                            self.account.hide();
                            self.settings_open = true;
                        }
                        for id in self.owned_games.keys().copied().collect::<Vec<_>>() {
                            let name = self
                                .games
                                .iter()
                                .find(|g| g.app_id == id)
                                .map(|g| g.name.as_str())
                                .unwrap_or("game");
                            if chrome::Chrome::close_game(ui, name).clicked() {
                                self.account.hide();
                                self.stop_game(id);
                            }
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(env!("CARGO_PKG_VERSION"))
                            .size(12.0)
                            .color(MUTED),
                    );
                    if self.chrome.nav(ui, 6, "Website", false).clicked() {
                        self.account.hide();
                        ctx.open_url(egui::OpenUrl::new_tab("https://cannamods.vip"));
                    }
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .corner_radius(0)
                    .fill(Color32::from_rgb(23, 32, 27))
                    .inner_margin(12),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.scanning || self.syncing {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.scan_status).size(13.0).color(MUTED));
                    ui.separator();
                    ui.label(
                        RichText::new(if self.token.is_empty() {
                            "Account not connected"
                        } else if self.syncing {
                            "Loading library…"
                        } else {
                            "Canna server library"
                        })
                        .size(13.0)
                        .color(MUTED),
                    );
                    ui.separator();
                    ui.label(RichText::new(&self.update_status).size(13.0).color(MUTED));
                });
            });
        if !self.account.open
            && !self.website.open
            && !self.skins.open
            && !self.modpacks_page
            && !self.game_details
            && !self.console_page
            && !self.discover_page
            && !self.minecraft_page
        {
            egui::SidePanel::right("detail")
                .exact_width(310.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .corner_radius(ui_helpers::SURFACE_RADIUS)
                        .fill(Color32::from_rgb(23, 29, 26))
                        .inner_margin(22),
                )
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let installed = self.games.iter().find(|g| g.app_id == self.selected);
                        let info = self.catalog.iter().find(|g| g.app_id == self.selected);
                        self.art(ui, self.selected, egui::vec2(266.0, 150.0));
                        ui.add_space(10.0);
                        ui.heading(
                            info.map(|g| g.name.as_str())
                                .or_else(|| installed.map(|g| g.name.as_str()))
                                .unwrap_or("Choose a game"),
                        );
                        ui.label(
                            RichText::new(if installed.is_some() {
                                "INSTALLED"
                            } else {
                                "NOT INSTALLED"
                            })
                            .color(GREEN)
                            .small(),
                        );
                        if let Some(g) = installed {
                            ui.add_space(12.0);
                            ui.label(&g.loader);
                            ui.label(
                                RichText::new("Detected from files; launch health is unverified.")
                                    .small()
                                    .color(MUTED),
                            );
                            if model::source_addons(g.app_id).is_some() { ui.label("Modded launches use -insecure practice mode. Vanilla disables Canna addons; external modifications remain your responsibility."); } else { ui.label(format!("{} local plugin DLLs", g.plugins)); }
                            ui.label(RichText::new(g.path.to_string_lossy()).small().color(MUTED));
                            if ui.button("Open game folder").clicked()
                                && let Err(e) = std::process::Command::new("explorer.exe")
                                    .arg(&g.path)
                                    .spawn()
                            {
                                self.warnings.push(format!("Cannot open game folder: {e}"));
                            }
                            if ui.button("View game & launch options").clicked() {
                                self.game_details = true;
                            }
                        }
                        ui.add_space(16.0);
                        ui.separator();
                        ui.label(RichText::new("FAMILY MODS").color(GREEN).small().strong());
                        if let Some(info) = info {
                            if ui
                                .add_enabled(installed.is_some(),
                                    egui::Button::new(
                                        RichText::new("+ Create modpack")
                                            .color(Color32::from_rgb(19, 35, 22))
                                            .strong(),
                                    )
                                    .fill(GREEN),
                                )
                                .clicked()
                            {
                                self.pack_ui.start_new(info, self.active_source.as_ref());
                                self.modpacks_page = true;
                            }
                            ui.label(RichText::new(&info.description).color(MUTED));
                            if !info.mod_folder_status.is_empty() {
                                ui.label(
                                    RichText::new(&info.mod_folder_status).small().color(MUTED),
                                );
                            }
                            if info.mods.is_empty() {
                                ui.add_space(12.0);
                                ui.label("No mods in the catalog yet.");
                            }
                            for m in &info.mods {
                                egui::Frame::new()
                                    .corner_radius(ui_helpers::SURFACE_RADIUS)
                                    .fill(Color32::from_rgb(33, 43, 36))
                                    .inner_margin(12)
                                    .corner_radius(ui_helpers::SURFACE_RADIUS)
                                    .show(ui, |ui| {
                                        ui.strong(&m.name);
                                        ui_helpers::mod_credits(ui,m);
                                        ui.label(
                                            RichText::new(format!("v{}", m.version)).color(GREEN),
                                        );
                                        ui.label(&m.description);
                                        ui.label(RichText::new(&m.file).small().color(MUTED));
                                    });
                            }
                        } else {
                            ui.label("This game isn't in your family's catalog yet.");
                            if let Some(game) = installed
                                && ui.button("+ Create modpack").clicked()
                            {
                                let game = GameInfo {
                                    app_id: game.app_id,
                                    name: game.name.clone(),
                                    folder: format!("steam-{}", game.app_id),
                                    description: String::new(),
                                    icon: String::new(),
                                    mods: vec![],
                                    mod_folder_status: String::new(),
                                };
                                self.pack_ui.start_new(&game, self.active_source.as_ref());
                                self.modpacks_page = true;
                            }
                        }
                        ui.add_space(18.0);
                        ui.label(
                            RichText::new(
                                "Create a modpack to add mods and launch vanilla or modded.",
                            )
                            .small()
                            .color(MUTED),
                        );
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().corner_radius(ui_helpers::SURFACE_RADIUS)
                    .fill(Color32::from_rgb(18, 24, 22))
                    .inner_margin(20),
            )
            .show(ctx, |ui| {
                if self.account.open {
                    if self.account.show(ui, &self.token, &mut self.website) {
                        website::disconnect();
                        self.token.clear();
                        self.catalog = model::supported_catalog();
                        self.repository_textures.clear();
                        self.repo_status = "Account logged out".into();
                    }
                    return;
                }
                if self.website.open { if ui_helpers::responsive_page(ui,"downloads-page",|ui|self.website.show(ui)) { self.pack_ui = pack_ui::PackUi::new(); } return; }
                if self.skins.open { self.skins.show(ui); return; }
                if self.minecraft_page { self.minecraft.open=false;self.minecraft.library(ui);return; }
                if self.discover_page { ui_helpers::responsive_page(ui,"discover-page",|ui|self.discover_ui(ui)); return; }
                if self.console_page {if self.console.show(ui,&self.games){self.last_console_poll=std::time::Instant::now()-std::time::Duration::from_secs(2);}return;}
                if self.modpacks_page {
                    let artwork = self.textures.iter().chain(self.repository_textures.iter()).map(|(&id, texture)| (id, texture.clone())).collect();
                    let mut pack_games = self.catalog.clone();
                    for game in &self.games {
                        if model::supported_game(game.app_id) && !pack_games.iter().any(|g| g.app_id == game.app_id) {
                            pack_games.push(GameInfo { app_id: game.app_id, name: game.name.clone(), folder: format!("steam-{}", game.app_id), description: String::new(), icon: String::new(), mods: vec![], mod_folder_status: String::new() });
                        }
                    }
                    if let Some(source) = ui_helpers::responsive_page(ui,"modpacks-page",|ui|self.pack_ui.show(ui, &pack_games, self.active_source.as_ref(), self.selected, self.syncing || self.scanning || self.runtime_busy, &artwork)) {
                        self.settings.owner = source.owner;
                        self.settings.repository = source.repository;
                        self.settings.branch = source.branch;
                        self.settings.catalog_folder = source.catalog_folder;
                        match self.settings.save() {
                            Ok(()) => self.sync(ctx),
                            Err(error) => self.repo_status = format!("Could not save repository: {error}"),
                        }
                    }
                    return;
                }
                if self.game_details {
                    egui::ScrollArea::both().id_salt("game-details-page").show(ui, |ui| {
                    if ui.button("‹ Back to library").clicked() { self.game_details = false; }
                    if let Some(game) = self.games.iter().find(|g| g.app_id == self.selected).cloned() {
                        ui.add_space(20.0); self.art(ui, game.app_id, egui::vec2(400.0,225.0));
                        ui.heading(&game.name); ui.label(&game.loader); ui.label(format!("{} plugin DLLs detected", game.plugins));
                        ui.label(game.path.display().to_string());
                        let info = self.catalog.iter().find(|g| g.app_id == game.app_id).cloned().unwrap_or_else(|| GameInfo { app_id:game.app_id, name:game.name.clone(), folder:format!("steam-{}",game.app_id),description:String::new(),icon:String::new(),mods:vec![],mod_folder_status:String::new() });
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("+ Create modpack").clicked() { self.pack_ui.start_new(&info,self.active_source.as_ref()); self.modpacks_page=true; }
                            if self.owned_games.contains_key(&game.app_id) && ui.button("Stop instance").clicked() { self.stop_game(game.app_id); }
                            if ui.button("Launch vanilla").clicked() { self.queue_game_launch(&game, &info, false); }
                            if ui.button("Launch modded").clicked() { self.queue_game_launch(&game, &info, true); }
                            if model::source_addons(game.app_id).is_none() && ui.add_enabled(!self.runtime_busy,egui::Button::new("Restore vanilla files")).clicked() {self.queue_vanilla_cleanup(game.app_id);}
                        });
                        ui.add_space(16.0); ui.heading("Family mods");
                        if info.mods.is_empty() { ui.label("No mods published for this game yet. You can still create a pack and import local mods."); }
                        for item in &info.mods { ui.label(format!("{} · {}",item.name,item.version)); }
                    } else if let Some(info)=self.catalog.iter().find(|g|g.app_id==self.selected).cloned() {
                        ui.add_space(20.0);self.art_tinted(ui,info.app_id,egui::vec2(400.0,225.0),Color32::from_gray(95));
                        ui.heading(&info.name);ui.label(RichText::new("Not Installed").color(MUTED));ui.label(&info.description);
                        ui.label("Install this game in Steam, then use Rescan Steam to enable its modpacks and launch controls.");
                        ui.horizontal(|ui| {ui.add_enabled(false,egui::Button::new("Create modpack"));ui.add_enabled(false,egui::Button::new("Launch vanilla"));ui.add_enabled(false,egui::Button::new("Launch modded"));});
                    } else { ui.heading("Game is not installed"); }
                    });
                    return;
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("GOOD TIMES, GROWN TOGETHER")
                                .small()
                                .color(GREEN),
                        );
                        ui.label(RichText::new("Your game library").size(30.0).strong());
                    });
                });
                ui.label(RichText::new("Supported games · Unity plugins / Source addons · Your family catalog").color(MUTED));
                ui.add_space(16.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!self.scanning, egui::Button::new("Rescan Steam"))
                        .clicked()
                    {
                        self.scan(ctx)
                    }
                    if ui
                        .add_enabled(!self.syncing, egui::Button::new("Sync server"))
                        .clicked()
                    {
                        self.sync(ctx)
                    }
                });
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search your games…")
                        .desired_width(f32::INFINITY),
                );
                ui.checkbox(&mut self.supported_only, "Only games in the family catalog");
                ui.add_space(12.0);
                let query = self.query.to_lowercase();
                let mut rows = library_rows(&self.catalog, &self.games);
                rows.retain(|r| {
                    r.1.to_lowercase().contains(&query) && (!self.supported_only || r.3)
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::new().fill(Color32::from_rgb(29,39,33)).corner_radius(ui_helpers::SURFACE_RADIUS).inner_margin(18).show(ui,|ui| {
                        ui.horizontal(|ui| {
                            self.art(ui,u32::MAX,egui::vec2(68.0,76.0)); ui.vertical(|ui| {ui.heading("Minecraft");ui.label("Java Edition · managed instances");});
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                                if ui.button("View").clicked(){self.minecraft_page=true;self.minecraft.open=false;}
                                if ui.button("Create instance").clicked(){self.minecraft.creating=true;}
                            });
                        });
                    });ui.add_space(16.0);
                    if self.games.is_empty() && !self.scanning {
                        ui.label(
                            RichText::new(
                                "No installed supported games found. Check your Steam location in Settings.",
                            )
                            .color(MUTED),
                        );
                        ui.add_space(12.0);
                    }
                    if rows.is_empty() {
                        ui.label("No games match this search.");
                    }
                    for (id, name, installed, supported, loader) in rows {
                        let selected = self.selected == id;
                        let pack_game = self.catalog.iter().find(|game| game.app_id == id).cloned().unwrap_or_else(|| GameInfo {
                            app_id: id, name: name.clone(), folder: format!("steam-{id}"), description: String::new(), icon: String::new(), mods: vec![], mod_folder_status: String::new(),
                        });
                        let card=egui::Frame::new().corner_radius(ui_helpers::SURFACE_RADIUS)
                            .fill(if !installed { Color32::from_gray(25) } else if selected {
                                Color32::from_rgb(36, 52, 38)
                            } else {
                                Color32::from_rgb(27, 35, 30)
                            })
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                if selected {
                                    Color32::from_rgb(94, 136, 80)
                                } else {
                                    Color32::from_rgb(43, 54, 46)
                                },
                            ))
                            .inner_margin(16)
                            .corner_radius(ui_helpers::SURFACE_RADIUS)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    self.art_tinted(ui, id, egui::vec2(68.0, 76.0), if installed { Color32::WHITE } else { Color32::from_gray(95) });
                                    ui.vertical(|ui| {
                                        ui.set_max_width((ui.available_width() - 250.0).max(120.0));
                                        ui.label(RichText::new(name).size(19.0).strong().color(if installed {Color32::WHITE} else {MUTED}));
                                        ui.label(RichText::new(loader).small().color(MUTED));
                                        ui.label(
                                            RichText::new(if installed {
                                                if supported {
                                                    model::framework_label(id)
                                                } else {
                                                    "UNITY DETECTED  /  NO CATALOG ENTRY"
                                                }
                                            } else {
                                                "NOT INSTALLED"
                                            })
                                            .size(10.0)
                                            .color(if installed {GREEN} else {MUTED}),
                                        );
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let view=ui.button("View");
                                            #[cfg(test)]
                                            self.card_view_rects.insert(id,view.rect);
                                            if view.clicked() {
                                                self.selected = id;
                                                self.game_details = true;
                                            }
                                            let create = ui.add_enabled(installed, egui::Button::new(RichText::new("Create modpack").color(Color32::from_rgb(19,35,22)).strong()).fill(GREEN).corner_radius(ui_helpers::CONTROL_RADIUS));
                                            #[cfg(test)]
                                            self.card_create_rects.insert(id, create.rect);
                                            if create.clicked() {
                                                self.selected = id;
                                                self.pack_ui.start_new(&pack_game, self.active_source.as_ref());
                                                self.website.open = false; self.skins.open = false;
                    self.minecraft_page=false;
                    self.discover_page = false;
                    self.modpacks_page = true;
                                            }
                                        },
                                    );
                                });
                            });
                        crate::ui_helpers::context_menu(&card.response,|ui| {
                            if ui.button("View game").clicked() {self.selected=id;self.game_details=true;ui.close();}
                            if ui.add_enabled(installed, egui::Button::new("Create modpack")).clicked() {self.selected=id;self.pack_ui.start_new(&pack_game,self.active_source.as_ref());self.modpacks_page=true;ui.close();}
                            if let Some(game)=self.games.iter().find(|g|g.app_id==id).cloned() {
                                ui.separator();
                                if ui.add_enabled(!self.runtime_busy,egui::Button::new("Launch vanilla")).clicked() {self.queue_game_launch(&game,&pack_game,false);ui.close();}
                                if ui.add_enabled(!self.runtime_busy,egui::Button::new("Launch modded")).clicked() {self.queue_game_launch(&game,&pack_game,true);ui.close();}
                                if model::source_addons(game.app_id).is_none() && ui.add_enabled(!self.runtime_busy,egui::Button::new("Restore vanilla files")).clicked() {self.queue_vanilla_cleanup(game.app_id);ui.close();}
                                ui.separator();
                                if ui.button("Open game folder").clicked() {if let Err(error)=std::process::Command::new("explorer.exe").arg(&game.path).spawn(){self.warnings.push(error.to_string());}ui.close();}
                                if ui.button("Copy game folder").clicked() {ui.ctx().copy_text(game.path.display().to_string());ui.close();}
                            }
                        });
                        ui.add_space(10.0);
                    }
                    if !self.warnings.is_empty() {
                        egui::CollapsingHeader::new(format!(
                            "Diagnostics ({})",
                            self.warnings.len()
                        ))
                        .show(ui, |ui| {
                            for w in &self.warnings {
                                ui.label(w);
                            }
                            for p in &self.libraries {
                                ui.label(p.to_string_lossy());
                            }
                        });
                    }
                });
            });
        if self.settings_open {
            self.settings_ui(ctx)
        }
    }
}
impl eframe::App for Canna {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if !self.scanning
            && (self.last_steam_scan.elapsed()
                >= std::time::Duration::from_secs(if self.settings.low_end { 120 } else { 30 })
                || focused
                    && !self.steam_was_focused
                    && self.last_steam_scan.elapsed() >= std::time::Duration::from_secs(3))
        {
            self.scan(ctx);
        }
        self.steam_was_focused = focused;
        ctx.request_repaint_after(std::time::Duration::from_secs(30));
        if self.screenshot.is_none() && self.website.update(ctx) {
            self.pack_ui = pack_ui::PackUi::new();
            self.token = website::session();
            self.sync(ctx);
        }
        if self.account.update(ctx, &self.token) {
            self.pack_ui = pack_ui::PackUi::new();
            self.token = website::session();
            self.sync(ctx);
        }
        self.rebound_access.observe(ctx, &self.token);
        if self.provider_browser.update(ctx, &self.token) {
            self.website.refresh_downloads();
            self.sync(ctx);
        }
        if self.website.discover_requested {
            self.website.discover_requested = false;
            self.website.open = false;
            self.discover_page = true;
            self.modpacks_page = false;
            self.console_page = false;
            self.minecraft_page = false;
            self.skins.open = false;
            self.provider_browser.mode = 0;
        }
        self.skins.update(ctx);
        self.render(ctx);
        if !self.account.open {
            self.minecraft.ui(ctx);
            if self.pack_ui.mod_details_window(ctx) {
                self.website.refresh_downloads();
            }
            if self.discover_page {
                self.provider_browser.modal(ctx);
            }
        }
        chrome::resize_handles(ctx);
        if self.minecraft.discover_requested {
            self.minecraft.discover_requested = false;
            self.minecraft_page = false;
            self.discover_page = true;
            self.discover.game = u32::MAX;
            self.provider_browser.select_game(u32::MAX);
        }
    }
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(chrome::CANVAS).to_array()
    }
}
fn restore_vanilla_cli_id(args: &[String]) -> anyhow::Result<Option<u32>> {
    let Some(index) = args.iter().position(|arg| arg == "--restore-vanilla") else {
        return Ok(None);
    };
    anyhow::ensure!(
        args.iter()
            .filter(|arg| *arg == "--restore-vanilla")
            .count()
            == 1,
        "Specify --restore-vanilla once with a Steam app ID"
    );
    let value = args
        .get(index + 1)
        .ok_or_else(|| anyhow::anyhow!("Use --restore-vanilla followed by a Steam app ID"))?;
    anyhow::ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "Invalid Steam app ID"
    );
    let id: u32 = value.parse()?;
    anyhow::ensure!(id > 0 && id != u32::MAX, "Invalid Steam app ID");
    Ok(Some(id))
}
fn main() -> eframe::Result {
    match restore_vanilla_cli_id(&std::env::args().collect::<Vec<_>>()) {
        Ok(Some(id)) => {
            let settings = Settings::load();
            let result = steam::scan(&settings.steam_path)
                .games
                .into_iter()
                .find(|game| game.app_id == id)
                .ok_or_else(|| {
                    anyhow::anyhow!("Install this supported Unity game through Steam first")
                })
                .and_then(|game| runtime::restore_vanilla(&game));
            match result {
                Ok(message) => {
                    println!("{message}");
                    return Ok(());
                }
                Err(error) => {
                    eprintln!("Vanilla restore: {error:#}");
                    std::process::exit(1);
                }
            }
        }
        Ok(None) => (),
        Err(error) => {
            eprintln!("Vanilla restore: {error:#}");
            std::process::exit(1);
        }
    }
    #[cfg(debug_assertions)]
    if std::env::args().any(|a| a == "--provider-smoke-test") {
        match provider_browser::live_check() {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("Native provider check: {e}");
                std::process::exit(1);
            }
        }
    }
    #[cfg(all(debug_assertions, not(canna_rebound_local_preview)))]
    if std::env::args().any(|a| a == "--updater-smoke-test") {
        let ready = updater::check("0.1.0")
            .expect("Live updater download failed")
            .expect("No newer release for smoke test");
        updater::apply(&ready).expect("Update handoff failed");
        return Ok(());
    }
    if std::env::args().any(|a| a == "--scan") {
        let settings = Settings::load();
        let scan = steam::scan(&settings.steam_path);
        println!(
            "Supported games: {} | Other Steam entries excluded: {}",
            scan.games.len(),
            scan.excluded
        );
        for p in scan.libraries {
            println!("Library: {}", p.display());
        }
        for g in scan.games {
            if let Some(v) = steam::installed_version(&g) {
                println!(
                    "Steam version: {} | branch={} | build={}",
                    g.name, v.branch, v.build
                );
            }
            println!(
                "{} | {} | {} | {} plugins | {}",
                g.app_id,
                g.name,
                g.loader,
                g.plugins,
                g.path.display()
            );
        }
        for w in scan.warnings {
            eprintln!("Warning: {w}");
        }
        return Ok(());
    }
    let uri = std::env::args().find(|a| a.starts_with("canna:"));
    let ticket = uri.as_deref().and_then(|raw| website::parse_uri(raw).ok());
    if uri.is_some() && ticket.is_none() {
        return Ok(());
    }
    let website_receiver = if std::env::var_os("CANNA_SCREENSHOT").is_some() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        receiver
    } else {
        let _ = website::register_protocol();
        let Some(receiver) = website::instance(ticket) else {
            return Ok(());
        };
        receiver
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("assets/canna-logo.png"))
                    .expect("Bundled Canna app icon"),
            )
            .with_decorations(false)
            .with_resizable(true)
            .with_inner_size(
                if std::env::var_os("CANNA_SCREENSHOT").is_some()
                    && std::env::args().any(|a| a == "--small-preview")
                {
                    [840.0, 560.0]
                } else {
                    [1240.0, 820.0]
                },
            )
            .with_min_inner_size([840.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        if cfg!(canna_rebound_local_preview) {
            "Canna Mod Manager · Canna Bliss local preview"
        } else {
            "Canna Mod Manager"
        },
        options,
        Box::new(move |cc| {
            let mut app = Canna::new(cc);
            app.website.set_receiver(website_receiver);
            Ok(Box::new(app))
        }),
    )
}
#[cfg(test)]
mod ui_tests {
    use super::*;
    #[test]
    fn vanilla_cli_rejects_ambiguous_or_invalid_targets_before_game_access() {
        let parse = |values: &[&str]| {
            restore_vanilla_cli_id(&values.iter().map(|v| (*v).into()).collect::<Vec<_>>())
        };
        assert_eq!(parse(&["canna", "--scan"]).unwrap(), None);
        assert_eq!(
            parse(&["canna", "--restore-vanilla", "1557740"]).unwrap(),
            Some(1557740)
        );
        for args in [
            vec!["canna", "--restore-vanilla"],
            vec!["canna", "--restore-vanilla", "0"],
            vec!["canna", "--restore-vanilla", "4294967295"],
            vec!["canna", "--restore-vanilla", "../ROUNDS"],
            vec![
                "canna",
                "--restore-vanilla",
                "1557740",
                "--restore-vanilla",
                "1686940",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
    }
    #[test]
    fn owned_unity_exit_cleanup_precedes_launch_requests_and_deduplicates() {
        let ctx = egui::Context::default();
        let mut app = Canna::new_with_context(&ctx, false);
        let game = |id| InstalledGame {
            app_id: id,
            name: "Cleanup dispatch fixture".into(),
            path: Default::default(),
            loader: "Fixture".into(),
            plugins: 0,
            icon: None,
        };
        app.games = vec![game(1557740), game(550), game(u32::MAX)];
        app.pack_ui
            .runtime_requests
            .push_back(pack_ui::RuntimeAction::LaunchCurrent(1557740));
        for id in [1557740, 1557740, 550, u32::MAX, 42] {
            app.queue_vanilla_cleanup(id);
        }
        assert_eq!(app.pack_ui.runtime_requests.len(), 3);
        assert!(matches!(
            app.pack_ui.runtime_requests.pop_front(),
            Some(pack_ui::RuntimeAction::RestoreVanilla(550))
        ));
        assert!(matches!(
            app.pack_ui.runtime_requests.pop_front(),
            Some(pack_ui::RuntimeAction::RestoreVanilla(1557740))
        ));
        assert!(matches!(
            app.pack_ui.runtime_requests.pop_front(),
            Some(pack_ui::RuntimeAction::LaunchCurrent(1557740))
        ));
    }
    #[test]
    fn rebound_checkbox_requires_verified_beta_and_idle_state() {
        for authorized in [false, true] {
            for busy in [false, true] {
                for size in [egui::vec2(1240.0, 820.0), egui::vec2(840.0, 650.0)] {
                    let ctx = egui::Context::default();
                    let mut enabled = false;
                    let rect = std::cell::Cell::new(egui::Rect::NOTHING);
                    let input = |events| egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    };
                    let mut draw = |ctx: &egui::Context| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let response = rebound_settings(ui, &mut enabled, busy, authorized);
                            assert_eq!(response.enabled(), authorized && !busy);
                            rect.set(response.rect);
                        });
                    };
                    let _ = ctx.run(input(vec![]), &mut draw);
                    let pos = rect.get().center();
                    let _ = ctx.run(
                        input(vec![
                            egui::Event::PointerMoved(pos),
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]),
                        &mut draw,
                    );
                    let _ = ctx.run(
                        input(vec![egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        }]),
                        &mut draw,
                    );
                    assert_eq!(enabled, authorized && !busy);
                }
            }
        }
    }
    fn unavailable_update_fixture() -> updater::Ready {
        updater::Ready {
            version: "99.0.0".into(),
            file: std::path::PathBuf::from("missing-update-fixture.exe"),
            hash: "0".repeat(64),
        }
    }
    #[test]
    fn update_events_respect_preview_isolation_without_changing_stable_behavior() {
        let ctx = egui::Context::default();
        let mut app = Canna::new_with_context(&ctx, false);
        app.tx
            .send(Event::Update(Ok(Some(unavailable_update_fixture()))))
            .unwrap();
        app.events(&ctx);
        if cfg!(canna_rebound_local_preview) {
            assert!(app.pending_update.is_none());
            assert_eq!(
                app.update_status,
                "Canna Bliss local preview; automatic updates disabled"
            );
            app.tx
                .send(Event::Update(Err("late failed stable update".into())))
                .unwrap();
            app.events(&ctx);
            assert_eq!(
                app.update_status,
                "Canna Bliss local preview; automatic updates disabled"
            );
        } else {
            assert_eq!(app.pending_update.as_ref().unwrap().version, "99.0.0");
            assert!(app.update_status.contains("99.0.0 downloaded"));
        }
    }
    #[test]
    fn anonymous_reporting_is_visible_in_settings_without_login_or_beta() {
        for size in [egui::vec2(1280.0, 900.0), egui::vec2(840.0, 650.0)] {
            let ctx = egui::Context::default();
            let mut app = Canna::new_with_context(&ctx, false);
            app.token.clear();
            app.settings_open = true;
            app.settings.anonymous_reports = false;
            app.diagnostics.enable(false);
            let mut found = false;
            for _ in 0..3 {
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.settings_ui(ctx),
                );
                found=output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Send anonymous launcher error reports" && t.pos.y>=s.clip_rect.min.y && t.pos.y+18.0<=s.clip_rect.max.y));
            }
            assert!(found, "Reporting preference must be visible at {size:?}");
            assert!(!app.diagnostics.is_enabled());
            assert!(!app.rebound_access.allowed);
        }
    }
    #[cfg(canna_rebound_local_preview)]
    #[test]
    fn preview_does_not_apply_even_an_injected_pending_stable_update() {
        let ctx = egui::Context::default();
        let mut app = Canna::new_with_context(&ctx, false);
        assert_eq!(
            app.update_status,
            "Canna Bliss local preview; automatic updates disabled"
        );
        app.pending_update = Some(unavailable_update_fixture());
        for size in [egui::vec2(1240.0, 820.0), egui::vec2(840.0, 650.0)] {
            let result = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| app.render(ctx),
            );
            assert!(
                app.pending_update.is_some(),
                "Pending stable update must never be taken for application"
            );
            assert_eq!(
                app.update_status,
                "Canna Bliss local preview; automatic updates disabled"
            );
            assert!(result.viewport_output.values().all(|viewport| {
                !viewport
                    .commands
                    .iter()
                    .any(|command| matches!(command, egui::ViewportCommand::Close))
            }));
        }
    }
    #[test]
    fn unsupported_catalog_and_installed_games_do_not_create_library_rows() {
        let mut catalog = model::supported_catalog();
        let mut fake = catalog[0].clone();
        fake.app_id = 42;
        fake.name = "Unsupported game".into();
        catalog.push(fake);
        let installed = vec![InstalledGame {
            app_id: 42,
            name: "Unsupported game".into(),
            path: Default::default(),
            loader: "Unity".into(),
            plugins: 0,
            icon: None,
        }];
        let rows = library_rows(&catalog, &installed);
        assert!(rows.iter().all(|r| model::supported_game(r.0)));
        assert!(rows.iter().any(|r| r.0 == 550 && !r.2));
    }
    #[test]
    fn discover_remembers_game_across_pages_and_repeated_click_goes_home() {
        let ctx = egui::Context::default();
        let mut app = Canna::new_with_context(&ctx, false);
        app.discover.game = 1686940;
        app.discover_page = false;
        app.modpacks_page = true;
        app.open_discover();
        assert_eq!(app.discover.game, 1686940);
        app.website.open = true;
        app.open_discover();
        assert_eq!(app.discover.game, 1686940);
        app.open_discover();
        assert_eq!(app.discover.game, 0);
    }
    #[test]
    fn missing_source_game_is_visible_but_cannot_create_or_launch() {
        let ctx = egui::Context::default();
        let mut app = Canna::new_with_context(&ctx, false);
        app.modpacks_page = false;
        app.selected = 550;
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 1400.0),
            )),
            events,
            ..Default::default()
        };
        for _ in 0..3 {
            let _ = ctx.run(input(vec![]), |ctx| app.render(ctx));
        }
        assert!(
            app.card_view_rects.contains_key(&550),
            "Uninstalled Left 4 Dead 2 must have a Library card"
        );
        let missing = library_rows(&app.catalog, &app.games)
            .into_iter()
            .find(|row| row.0 == 550)
            .unwrap();
        assert!(!missing.2);
        assert_eq!(missing.4, "Not Installed");
        let pos = app.card_create_rects[&550].center();
        let _ = ctx.run(
            input(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]),
            |ctx| app.render(ctx),
        );
        let _ = ctx.run(
            input(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| app.render(ctx),
        );
        assert!(!app.modpacks_page);
        assert!(app.pack_ui.draft_game_id().is_none());
        assert!(!app.runtime_busy);
        app.game_details = true;
        let _ = ctx.run(input(vec![]), |ctx| app.render(ctx));
        assert!(!app.runtime_busy);
        app.games.push(InstalledGame {
            app_id: 550,
            name: "Left 4 Dead 2".into(),
            path: "fixture".into(),
            loader: "Source VPK addons".into(),
            plugins: 0,
            icon: None,
        });
        assert!(
            library_rows(&app.catalog, &app.games)
                .into_iter()
                .find(|r| r.0 == 550)
                .unwrap()
                .2
        );
    }
    #[test]
    fn game_card_create_receives_pointer_click_and_opens_editor() {
        card_action(false);
    }
    #[test]
    fn game_card_view_opens_detail_page() {
        card_action(true);
    }
    fn card_action(view: bool) {
        let ctx = egui::Context::default();
        let mut app = Canna::new_with_context(&ctx, false);
        app.modpacks_page = false;
        let mut rounds = model::bopl();
        rounds.app_id = 1557740;
        rounds.name = "ROUNDS".into();
        rounds.folder = "rounds".into();
        app.catalog.push(rounds);
        app.games.push(InstalledGame {
            app_id: 1557740,
            name: "ROUNDS".into(),
            path: std::path::PathBuf::new(),
            loader: "BepInEx detected".into(),
            plugins: 0,
            icon: None,
        });
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 820.0),
            )),
            events,
            ..Default::default()
        };
        for _ in 0..3 {
            let _ = ctx.run(input(vec![]), |ctx| app.render(ctx));
        }
        let pos = if view {
            app.card_view_rects[&1557740].center()
        } else {
            app.card_create_rects[&1557740].center()
        };
        let _ = ctx.run(
            input(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]),
            |ctx| app.render(ctx),
        );
        let _ = ctx.run(
            input(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| app.render(ctx),
        );
        if view {
            assert!(app.game_details);
            assert!(!app.modpacks_page);
            assert_eq!(app.selected, 1557740);
            return;
        }
        assert!(app.modpacks_page, "Card button must switch to modpacks");
        assert_eq!(app.selected, 1557740);
        assert_eq!(app.pack_ui.draft_game_id(), Some(1557740));
        let _ = ctx.run(input(vec![]), |ctx| app.render(ctx));
        assert_eq!(app.pack_ui.draft_game_id(), Some(1557740));
    }
}
