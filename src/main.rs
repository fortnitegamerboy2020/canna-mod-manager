#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod cache;
mod console;
mod model;
mod modpacks;
mod pack_ui;
mod repository;
mod runtime;
mod steam;
mod ui_helpers;
mod updater;

use eframe::egui::{self, Color32, RichText};
use model::{GameInfo, InstalledGame, Scan, Settings};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, Sender},
};

const GREEN: Color32 = Color32::from_rgb(163, 220, 144);
const MUTED: Color32 = Color32::from_rgb(143, 157, 149);
include!(concat!(env!("OUT_DIR"), "/canna_token.rs"));
include!(concat!(env!("OUT_DIR"), "/canna_update_token.rs"));
enum Event {
    Update(Result<Option<updater::Ready>, String>),
    ConsoleData(Vec<(u32, console::Snapshot)>),
    Launched(u32, bool, std::time::SystemTime),
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
    update_status: String,
    pending_update: Option<updater::Ready>,
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
    syncing: bool,
    scan_status: String,
    repo_status: String,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    screenshot: Option<std::path::PathBuf>,
    screenshot_requested: bool,
    ready_frames: usize,
}
impl Canna {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_context(&cc.egui_ctx, true)
    }
    fn new_with_context(ctx: &egui::Context, start_jobs: bool) -> Self {
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
        style.spacing.button_padding = egui::vec2(16.0, 10.0);
        ctx.set_style(style);
        let (tx, rx) = mpsc::channel();
        let settings = Settings::load();
        let configured = !settings.owner.is_empty();
        let mut app = Self {
            update_status: String::new(),
            pending_update: None,
            console_page: std::env::args().any(|arg| arg == "--console"),
            console: console::Console::new(),
            console_polling: false,
            last_console_poll: std::time::Instant::now() - std::time::Duration::from_secs(2),
            launch_watch: None,
            game_details: std::env::args().any(|arg| arg == "--game-details"),
            runtime_busy: false,
            runtime_enabled: start_jobs,
            runtime_status: String::new(),
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
            token: std::env::var("CANNA_GITHUB_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty())
                .unwrap_or_else(|| EMBEDDED_GITHUB_TOKEN.to_owned()),
            settings_open: false,
            query: String::new(),
            supported_only: false,
            games: vec![],
            catalog: vec![model::bopl()],
            libraries: vec![],
            warnings: vec![],
            textures: BTreeMap::new(),
            repository_textures: BTreeMap::new(),
            active_source: None,
            cached_at: None,
            selected: 1686940,
            scanning: false,
            syncing: false,
            scan_status: String::new(),
            repo_status: "Connect your family's GitHub repository in Settings".into(),
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
        if start_jobs {
            app.update_status = "Checking for Canna updates…".into();
            let tx = app.tx.clone();
            let repaint = ctx.clone();
            let token = std::env::var("CANNA_UPDATE_TOKEN")
                .unwrap_or_else(|_| EMBEDDED_UPDATE_TOKEN.to_owned());
            std::thread::spawn(move || {
                let result =
                    updater::check(&token, env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string());
                let _ = tx.send(Event::Update(result));
                repaint.request_repaint();
            });
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
        self.repo_status = "Reading GitHub catalog…".into();
        let s = self.settings.clone();
        let source = cache::Source::from_settings(&s);
        if self.active_source.as_ref() != Some(&source) {
            self.catalog = vec![model::bopl()];
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
                Event::Launched(id, modded, requested) => {
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
                    for g in &scan.games {
                        if !self.textures.contains_key(&g.app_id)
                            && let Some(b) = &g.icon
                        {
                            self.texture(ctx, g.app_id, b, false)
                        }
                    }
                    self.scan_status = format!(
                        "{} Unity games · {} Steam libraries · {} other entries excluded",
                        scan.games.len(),
                        scan.libraries.len(),
                        scan.excluded
                    );
                    self.games = scan.games;
                    self.libraries = scan.libraries;
                    self.warnings.extend(scan.warnings);
                    self.console.record(&self.scan_status, &self.token);
                }
                Event::Cached(source, result) => {
                    if self.active_source.as_ref() != Some(&source) {
                        continue;
                    }
                    match result {
                        Ok(Some(data)) => {
                            self.cached_at = data.cached_at;
                            self.repo_status = format!(
                                "Cached catalog · last synced {} · checking GitHub…",
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
        self.catalog = data.games;
        if !self.catalog.iter().any(|game| game.app_id == 1686940) {
            self.catalog.insert(0, model::bopl());
        }
        self.warnings.extend(data.warnings);
    }
    fn settings_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Your private library").open(&mut open).resizable(false).default_width(500.0).show(ctx,|ui| {
            ui.label(RichText::new("GITHUB REPOSITORY").color(GREEN).strong());
            ui.label("Read-only access. Add games and mods directly on GitHub.");
            ui.label("Owner / organization"); ui.text_edit_singleline(&mut self.settings.owner);
            ui.label("Repository"); ui.text_edit_singleline(&mut self.settings.repository);
            ui.label("Branch"); ui.text_edit_singleline(&mut self.settings.branch);
            ui.label("Catalog folder inside repository (optional)");
            ui.add(egui::TextEdit::singleline(&mut self.settings.catalog_folder).hint_text("Leave empty for root, or use games"));
            let folder = if self.settings.catalog_folder.is_empty() { String::new() } else { format!("{}/", self.settings.catalog_folder) };
            ui.label(RichText::new(format!("Reads {folder}catalog.json → {folder}bopl-battle/game.json → Mods/")).small().color(MUTED));
            ui.label("Private repository token");
            ui.add(egui::TextEdit::singleline(&mut self.token).password(true).hint_text("Fine-grained token with Contents: read"));
            ui.label(RichText::new("Family builds can include a default token. Changes here last for this session; CANNA_GITHUB_TOKEN overrides the build default.").small().color(MUTED));
            ui.separator(); ui.label("Steam location override (optional)");
            ui.add(egui::TextEdit::singleline(&mut self.settings.steam_path).hint_text("e.g. D:\\Steam — leave empty for auto-detection"));
            if ui.add_enabled(!self.scanning && !self.syncing, egui::Button::new("Save, scan & connect")).clicked() {
                match self.settings.save() {Ok(())=>{self.scan(ctx);self.sync(ctx);},Err(e)=>self.repo_status=format!("Could not save settings: {e}")}
            }
        });
        self.settings_open = open;
    }
    fn art(&self, ui: &mut egui::Ui, id: u32, size: egui::Vec2) {
        if let Some(t) = self
            .repository_textures
            .get(&id)
            .or_else(|| self.textures.get(&id))
        {
            ui.add(
                egui::Image::new(t)
                    .fit_to_exact_size(size)
                    .corner_radius(ui_helpers::SURFACE_RADIUS),
            );
        } else {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                ui_helpers::SURFACE_RADIUS,
                Color32::from_rgb(44, 66, 45),
            );
            ui.painter().circle_filled(
                rect.center(),
                size.x.min(size.y) * 0.22,
                Color32::from_rgb(116, 163, 99),
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
    fn queue_game_launch(&mut self, game: &InstalledGame, info: &GameInfo, modded: bool) {
        let pack = modpacks::Modpack::create(
            format!("{} current setup", game.name),
            String::new(),
            info,
            cache::Source::from_settings(&self.settings),
            vec![],
        );
        // Game launch preserves the currently installed plugins.
        if modded {
            self.pack_ui
                .runtime_requests
                .push_back(pack_ui::RuntimeAction::Setup(pack.clone()));
        }
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
        let id = match &request {
            pack_ui::RuntimeAction::Setup(p)
            | pack_ui::RuntimeAction::Install(p)
            | pack_ui::RuntimeAction::Launch(p, _) => p.game.app_id,
            pack_ui::RuntimeAction::LaunchCurrent(id) => *id,
        };
        let Some(game) = self.games.iter().find(|g| g.app_id == id).cloned() else {
            self.runtime_status = "Install this game through Steam first.".into();
            return;
        };
        self.runtime_busy = true;
        self.runtime_status = "Preparing game…".into();
        self.pack_ui.set_runtime_status(&self.runtime_status);
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let token = self.token.clone();
        std::thread::spawn(move || {
            let progress = |message: &str| {
                let _ = tx.send(Event::RuntimeProgress(message.into()));
                ctx.request_repaint();
            };
            let result = (|| -> anyhow::Result<String> {
                match request {
                    pack_ui::RuntimeAction::Setup(pack) => {
                        runtime::setup(&game, &pack, &token)?;
                        Ok("BepInEx is ready. Choose mods for your pack.".into())
                    }
                    pack_ui::RuntimeAction::Install(pack) => {
                        runtime::install_pack(&game, &pack, &token, &progress)?;
                        Ok(format!(
                            "Installed {} mods from {}",
                            pack.mods.len(),
                            pack.name
                        ))
                    }
                    pack_ui::RuntimeAction::Launch(pack, modded) => {
                        if modded {
                            runtime::install_pack(&game, &pack, &token, &progress)?;
                        }
                        let requested = std::time::SystemTime::now();
                        runtime::launch(&game, modded)?;
                        let _ = tx.send(Event::Launched(game.app_id, modded, requested));
                        Ok(if modded {
                            "Steam launch requested (modded)."
                        } else {
                            "Steam launch requested (vanilla)."
                        }
                        .into())
                    }
                    pack_ui::RuntimeAction::LaunchCurrent(_) => {
                        let requested = std::time::SystemTime::now();
                        runtime::launch(&game, true)?;
                        let _ = tx.send(Event::Launched(game.app_id, true, requested));
                        Ok("Steam launch requested (current modded setup).".into())
                    }
                }
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Runtime(result));
            ctx.request_repaint();
        });
    }
    fn render(&mut self, ctx: &egui::Context) {
        self.events(ctx);
        if self.pending_update.is_some()
            && !self.runtime_busy
            && !self.pack_ui.editing()
            && !self.settings_open
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
        self.run_runtime(ctx);
        if let Some(id) = self.pack_ui.console_game.take() {
            self.console_page = true;
            self.console.game_id = id;
        }
        if self.runtime_enabled && (self.console_page || self.launch_watch.is_some()) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
            if !self.console_polling && self.last_console_poll.elapsed().as_secs() >= 1 {
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
        if !self.runtime_status.is_empty() {
            egui::TopBottomPanel::bottom("runtime_status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if self.runtime_busy {
                        ui.spinner();
                    }
                    ui.label(&self.runtime_status);
                    if ui.button("Open Console").clicked() {
                        self.console_page = true;
                    }
                });
            });
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
        egui::SidePanel::left("navigation")
            .exact_width(218.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .corner_radius(ui_helpers::SURFACE_RADIUS)
                    .fill(Color32::from_rgb(23, 32, 27))
                    .inner_margin(22),
            )
            .show(ctx, |ui| {
                ui.add_space(14.0);
                ui.label(RichText::new("canna").size(36.0).color(GREEN).strong());
                ui.label(RichText::new("MOD MANAGER").size(11.0).color(MUTED));
                ui.add_space(40.0);
                ui.label(RichText::new("YOUR SPACE").small().color(MUTED));
                if ui
                    .add(
                        egui::Button::new("Game library")
                            .selected(!self.modpacks_page && !self.console_page)
                            .min_size(egui::vec2(170.0, 44.0)),
                    )
                    .clicked()
                {
                    self.modpacks_page = false;
                    self.game_details = false;
                    self.console_page = false;
                }
                if ui
                    .add(
                        egui::Button::new("Modpacks")
                            .selected(self.modpacks_page && !self.console_page)
                            .min_size(egui::vec2(170.0, 44.0)),
                    )
                    .clicked()
                {
                    self.modpacks_page = true;
                    self.console_page = false;
                }
                if ui
                    .add(
                        egui::Button::new("Console")
                            .selected(self.console_page)
                            .min_size(egui::vec2(170.0, 44.0)),
                    )
                    .clicked()
                {
                    self.console_page = true;
                    self.last_console_poll =
                        std::time::Instant::now() - std::time::Duration::from_secs(2);
                }
                if ui.button("Repository settings").clicked() {
                    self.settings_open = true;
                }
                ui.add_space(28.0);
                ui.label(RichText::new("Made for the family.").color(GREEN));
                ui.label(
                    RichText::new("A little greener.\nA little more chaotic.")
                        .small()
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("CANNA  /  v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(MUTED),
                    );
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .corner_radius(ui_helpers::SURFACE_RADIUS)
                    .fill(Color32::from_rgb(23, 32, 27))
                    .inner_margin(12),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.scanning || self.syncing {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.scan_status).small().color(MUTED));
                    ui.separator();
                    ui.label(RichText::new(&self.repo_status).small().color(MUTED));
                    ui.separator();
                    ui.label(RichText::new(&self.update_status).small().color(MUTED));
                });
            });
        if !self.modpacks_page && !self.game_details && !self.console_page {
            egui::SidePanel::right("detail").exact_width(310.0).resizable(false).frame(egui::Frame::new().corner_radius(ui_helpers::SURFACE_RADIUS).fill(Color32::from_rgb(23,29,26)).inner_margin(22)).show(ctx,|ui| {
            egui::ScrollArea::vertical().show(ui,|ui| {
                let installed=self.games.iter().find(|g|g.app_id==self.selected);
                let info=self.catalog.iter().find(|g|g.app_id==self.selected);
                self.art(ui,self.selected,egui::vec2(266.0,150.0));ui.add_space(10.0);
                ui.heading(info.map(|g|g.name.as_str()).or_else(|| installed.map(|g|g.name.as_str())).unwrap_or("Choose a game"));
                ui.label(RichText::new(if installed.is_some(){"INSTALLED"}else{"NOT INSTALLED"}).color(GREEN).small());
                if let Some(g)=installed {
                    ui.add_space(12.0);ui.label(&g.loader);
                    ui.label(RichText::new("Detected from files; launch health is unverified.").small().color(MUTED));
                    ui.label(format!("{} local plugin DLLs",g.plugins));
                    ui.label(RichText::new(g.path.to_string_lossy()).small().color(MUTED));
                    if ui.button("Open game folder").clicked()
                        && let Err(e)=std::process::Command::new("explorer.exe").arg(&g.path).spawn() {self.warnings.push(format!("Cannot open game folder: {e}"));}
                    if ui.button("View game & launch options").clicked() { self.game_details=true; }
                }
                ui.add_space(16.0);ui.separator();ui.label(RichText::new("FAMILY MODS").color(GREEN).small().strong());
                if let Some(info)=info {
                    if ui.add(egui::Button::new(RichText::new("+ Create modpack").color(Color32::from_rgb(19,35,22)).strong()).fill(GREEN)).clicked() {
                        self.pack_ui.start_new(info, self.active_source.as_ref()); self.modpacks_page = true;
                    }
                    ui.label(RichText::new(&info.description).color(MUTED));
                    if !info.mod_folder_status.is_empty() {ui.label(RichText::new(&info.mod_folder_status).small().color(MUTED));}
                    if info.mods.is_empty() {ui.add_space(12.0);ui.label("No mods in the catalog yet.");}
                    for m in &info.mods {egui::Frame::new().corner_radius(ui_helpers::SURFACE_RADIUS).fill(Color32::from_rgb(33,43,36)).inner_margin(12).corner_radius(ui_helpers::SURFACE_RADIUS).show(ui,|ui| {ui.strong(&m.name);ui.label(RichText::new(format!("v{}",m.version)).color(GREEN));ui.label(&m.description);ui.label(RichText::new(&m.file).small().color(MUTED));});}
                } else {
                    ui.label("This game isn't in your family's catalog yet.");
                    if let Some(game) = installed && ui.button("+ Create modpack").clicked() {
                        let game = GameInfo { app_id: game.app_id, name: game.name.clone(), folder: format!("steam-{}", game.app_id), description: String::new(), icon: String::new(), mods: vec![], mod_folder_status: String::new() };
                        self.pack_ui.start_new(&game, self.active_source.as_ref()); self.modpacks_page = true;
                    }
                }
                ui.add_space(18.0);ui.label(RichText::new("Create a modpack to set up BepInEx, add mods and launch vanilla or modded.").small().color(MUTED));
            });
        });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new().corner_radius(ui_helpers::SURFACE_RADIUS)
                    .fill(Color32::from_rgb(18, 24, 22))
                    .inner_margin(28),
            )
            .show(ctx, |ui| {
                if self.console_page {if self.console.show(ui,&self.games){self.last_console_poll=std::time::Instant::now()-std::time::Duration::from_secs(2);}return;}
                if self.modpacks_page {
                    let artwork = self.textures.iter().chain(self.repository_textures.iter()).map(|(&id, texture)| (id, texture.clone())).collect();
                    let mut pack_games = self.catalog.clone();
                    for game in &self.games {
                        if !pack_games.iter().any(|g| g.app_id == game.app_id) {
                            pack_games.push(GameInfo { app_id: game.app_id, name: game.name.clone(), folder: format!("steam-{}", game.app_id), description: String::new(), icon: String::new(), mods: vec![], mod_folder_status: String::new() });
                        }
                    }
                    if let Some(source) = self.pack_ui.show(ui, &pack_games, self.active_source.as_ref(), self.selected, self.syncing || self.scanning || self.runtime_busy, &artwork) {
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
                    if ui.button("‹ Back to library").clicked() { self.game_details = false; }
                    if let Some(game) = self.games.iter().find(|g| g.app_id == self.selected).cloned() {
                        ui.add_space(20.0); self.art(ui, game.app_id, egui::vec2(400.0,225.0));
                        ui.heading(&game.name); ui.label(&game.loader); ui.label(format!("{} plugin DLLs detected", game.plugins));
                        ui.label(game.path.display().to_string());
                        let info = self.catalog.iter().find(|g| g.app_id == game.app_id).cloned().unwrap_or_else(|| GameInfo { app_id:game.app_id, name:game.name.clone(), folder:format!("steam-{}",game.app_id),description:String::new(),icon:String::new(),mods:vec![],mod_folder_status:String::new() });
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("+ Create modpack").clicked() { self.pack_ui.start_new(&info,self.active_source.as_ref()); self.modpacks_page=true; }
                            if ui.button("Launch vanilla").clicked() { self.queue_game_launch(&game, &info, false); }
                            if ui.button("Launch modded").clicked() { self.queue_game_launch(&game, &info, true); }
                        });
                        ui.add_space(16.0); ui.heading("Family mods");
                        if info.mods.is_empty() { ui.label("No mods published for this game yet. You can still create a pack and set up BepInEx."); }
                        for item in &info.mods { ui.label(format!("{} · {}",item.name,item.version)); }
                    } else { ui.heading("Game is not installed"); }
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
                ui.label(RichText::new("Unity games · BepInEx framework · Your family catalog").color(MUTED));
                ui.add_space(16.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!self.scanning, egui::Button::new("Rescan Steam"))
                        .clicked()
                    {
                        self.scan(ctx)
                    }
                    if ui
                        .add_enabled(!self.syncing, egui::Button::new("Sync repository"))
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
                let mut rows: Vec<(u32, String, bool, bool, String)> = self
                    .games
                    .iter()
                    .map(|g| {
                        (
                            g.app_id,
                            g.name.clone(),
                            true,
                            self.catalog.iter().any(|c| c.app_id == g.app_id),
                            g.loader.clone(),
                        )
                    })
                    .collect();
                if !self.games.iter().any(|g| g.app_id == 1686940) {
                    rows.insert(
                        0,
                        (
                            1686940,
                            "Bopl Battle".into(),
                            false,
                            true,
                            "Install this game in Steam to get started".into(),
                        ),
                    );
                }
                rows.retain(|r| {
                    r.1.to_lowercase().contains(&query) && (!self.supported_only || r.3)
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.games.is_empty() && !self.scanning {
                        ui.label(
                            RichText::new(
                                "No installed Unity games found. Check your Steam location in Settings.",
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
                            .fill(if selected {
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
                                    self.art(ui, id, egui::vec2(68.0, 76.0));
                                    ui.vertical(|ui| {
                                        ui.set_max_width((ui.available_width() - 250.0).max(120.0));
                                        ui.label(RichText::new(name).size(19.0).strong());
                                        ui.label(RichText::new(loader).small().color(MUTED));
                                        ui.label(
                                            RichText::new(if installed {
                                                if supported {
                                                    "UNITY  /  FAMILY CATALOG"
                                                } else {
                                                    "UNITY DETECTED  /  NO CATALOG ENTRY"
                                                }
                                            } else {
                                                "FIRST SUPPORTED GAME"
                                            })
                                            .size(10.0)
                                            .color(GREEN),
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
                                            let create = ui.add(egui::Button::new(RichText::new("Create modpack").color(Color32::from_rgb(19,35,22)).strong()).fill(GREEN).corner_radius(ui_helpers::CONTROL_RADIUS));
                                            #[cfg(test)]
                                            self.card_create_rects.insert(id, create.rect);
                                            if create.clicked() {
                                                self.selected = id;
                                                self.pack_ui.start_new(&pack_game, self.active_source.as_ref());
                                                self.modpacks_page = true;
                                            }
                                        },
                                    );
                                });
                            });
                        crate::ui_helpers::context_menu(&card.response,|ui| {
                            if ui.button("View game").clicked() {self.selected=id;self.game_details=true;ui.close();}
                            if ui.button("Create modpack").clicked() {self.selected=id;self.pack_ui.start_new(&pack_game,self.active_source.as_ref());self.modpacks_page=true;ui.close();}
                            if let Some(game)=self.games.iter().find(|g|g.app_id==id).cloned() {
                                ui.separator();
                                if ui.add_enabled(!self.runtime_busy,egui::Button::new("Launch vanilla")).clicked() {self.queue_game_launch(&game,&pack_game,false);ui.close();}
                                if ui.add_enabled(!self.runtime_busy,egui::Button::new("Launch modded")).clicked() {self.queue_game_launch(&game,&pack_game,true);ui.close();}
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
        self.render(ctx);
    }
}
fn main() -> eframe::Result {
    #[cfg(debug_assertions)]
    if std::env::args().any(|a| a == "--updater-smoke-test") {
        let ready = updater::check(EMBEDDED_UPDATE_TOKEN, "0.1.0")
            .expect("Live updater download failed")
            .expect("No newer release for smoke test");
        updater::apply(&ready).expect("Update handoff failed");
        return Ok(());
    }
    if std::env::args().any(|a| a == "--scan") {
        let settings = Settings::load();
        let scan = steam::scan(&settings.steam_path);
        println!(
            "Unity games: {} | Other Steam entries excluded: {}",
            scan.games.len(),
            scan.excluded
        );
        for p in scan.libraries {
            println!("Library: {}", p.display());
        }
        for g in scan.games {
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([1080.0, 680.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Canna Mod Manager",
        options,
        Box::new(|cc| Ok(Box::new(Canna::new(cc)))),
    )
}
#[cfg(test)]
mod ui_tests {
    use super::*;
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
