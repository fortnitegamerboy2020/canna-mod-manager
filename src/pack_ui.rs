use crate::{cache::Source, model::GameInfo, modpacks::Modpack};

use eframe::egui::{self, Color32, RichText};

use std::collections::{BTreeMap, BTreeSet};

const GREEN: Color32 = Color32::from_rgb(163, 220, 144);

const TEXT: Color32 = Color32::from_rgb(233, 240, 234);

const MUTED: Color32 = Color32::from_rgb(149, 164, 153);

const SURFACE: Color32 = Color32::from_rgb(29, 39, 33);

#[derive(Default)]
pub struct DiscoverState {
    pub query: String,

    pub kind: String,

    pub game: u32,

    pub target: Option<String>,
}

pub struct PackUi {
    pub lab_games: Vec<crate::model::InstalledGame>,
    pub runtime_options: crate::runtime::InstallOptions,
    pub runtime_progress: Vec<String>,

    pub lab_memory: BTreeMap<u32, crate::play_metrics::Memory>,

    lab: crate::play_lab::Lab,

    sharing: crate::shared_packs::Sharing,

    pub console_game: Option<u32>,

    pub discover_pack: Option<String>,

    pub discover_return: bool,

    pub owned_games: BTreeSet<u32>,

    #[cfg(test)]
    add_mods_rect: Option<egui::Rect>,

    pub runtime_requests: std::collections::VecDeque<RuntimeAction>,

    packs: Vec<Modpack>,

    draft: Option<Modpack>,

    status: String,

    query: String,

    selected: Option<String>,

    chooser: bool,

    groups: Vec<String>,

    group_dialog: bool,

    group_name: String,

    group_members: BTreeSet<String>,

    game_filter: u32,

    sort: u8,

    detail_tab: u8,

    content_query: String,

    deleted: Option<std::path::PathBuf>,

    mod_details: Option<crate::model::ModInfo>,

    mod_details_context: Option<(GameInfo, Option<Source>, Option<Modpack>)>,

    mod_download: Option<std::sync::mpsc::Receiver<Result<(), String>>>,

    mod_download_status: String,

    #[cfg(test)]
    modal_rect: Option<egui::Rect>,

    #[cfg(test)]
    discover_rects: Vec<egui::Rect>,
}

pub enum RuntimeAction {
    Stop(u32),

    Setup(Modpack),

    Install(Modpack),

    Launch(Modpack, bool),

    LaunchCurrent(u32),
}

enum Action {
    Stop(u32),

    Discover(String),

    Toggle(Modpack, String, bool),

    Delete(Modpack),

    UndoDelete,

    Install(Modpack),

    Launch(Modpack, bool),

    Local(Modpack),

    Remove(Modpack, String),

    Duplicate(Modpack),

    Import,

    Export(Modpack),

    Edit(Modpack),

    Save,

    Cancel,

    Open(String),

    New,

    Start,

    Group,

    SaveGroup,
}

impl PackUi {
    pub fn provider_target(&self, ui: &mut egui::Ui, target: &mut Option<String>) {
        egui::ComboBox::from_id_salt("provider-target-pack")
            .height(340.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .selected_text(
                self.packs
                    .iter()
                    .find(|p| Some(&p.id) == target.as_ref())
                    .map(|p| p.name.as_str())
                    .unwrap_or("Choose a modpack to add downloaded mods"),
            )
            .show_ui(ui, |ui| {
                let choices = self
                    .packs
                    .iter()
                    .map(|pack| {
                        (
                            Some(pack.id.clone()),
                            format!("{} · {}", pack.name, pack.game.name),
                        )
                    })
                    .collect::<Vec<_>>();

                crate::ui_helpers::searchable_options(ui, target, &choices);
            });
    }

    pub fn provider_add(
        &mut self,

        id: &str,

        game: &GameInfo,

        source: Option<&Source>,

        item: crate::model::ModInfo,
    ) -> anyhow::Result<String> {
        self.add_catalog_mod(id, game, source, item)?;

        Ok("Saved to modpack. Apply modpack or Launch modded to install its enabled mods.".into())
    }

    pub fn catalog_pack_matches(&self, id: &str, game: &GameInfo, source: Option<&Source>) -> bool {
        self.packs.iter().any(|pack| {
            pack.id == id
                && pack.game.app_id == game.app_id
                && pack.game.folder == game.folder
                && Some(&pack.repository) == source
        })
    }

    pub fn open_catalog_details(
        &mut self,
        game: &GameInfo,
        source: Option<&Source>,
        target: Option<&str>,
        item: crate::model::ModInfo,
    ) {
        let selected = target
            .filter(|id| self.catalog_pack_matches(id, game, source))
            .and_then(|id| self.packs.iter().find(|pack| pack.id == id))
            .cloned();
        self.mod_details = Some(item);
        self.mod_details_context = Some((game.clone(), source.cloned(), selected));
        self.mod_download_status.clear();
    }

    pub fn discover(
        &mut self,

        ui: &mut egui::Ui,

        catalog: &[GameInfo],

        source: Option<&Source>,

        state: &mut DiscoverState,

        busy: bool,
    ) {
        let DiscoverState {
            query,

            game: game_filter,

            target,

            kind,
        } = state;

        #[cfg(test)]
        self.discover_rects.clear();

        ui.add_space(6.0);

        ui.add_sized(
            [ui.available_width(), 30.0],
            egui::TextEdit::singleline(query)
                .hint_text("Search mods, games, or descriptions…")
                .desired_width(f32::INFINITY),
        );

        if *game_filter == u32::MAX {
            ui.horizontal_wrapped(|ui| {
                for (value, label) in [
                    ("", "All"),
                    ("mod", "Mods"),
                    ("shader", "Shaders"),
                    ("resourcepack", "Resource packs"),
                    ("datapack", "Data packs"),
                ] {
                    ui.selectable_value(kind, value.into(), label);
                }
            });
        }

        ui.horizontal_wrapped(|ui| {
            ui.label("Add to");

            egui::ComboBox::from_id_salt("discover_pack")
                .height(340.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .selected_text(
                    self.packs
                        .iter()
                        .find(|p| Some(&p.id) == target.as_ref())
                        .map(|p| p.name.as_str())
                        .unwrap_or("Choose a modpack"),
                )
                .show_ui(ui, |ui| {
                    let choices = self
                        .packs
                        .iter()
                        .map(|pack| {
                            (
                                Some(pack.id.clone()),
                                format!("{} · {}", pack.name, pack.game.name),
                            )
                        })
                        .collect::<Vec<_>>();

                    crate::ui_helpers::searchable_options(ui, target, &choices);
                });

            if ui.button("Open modpack").clicked() {
                self.selected = target.clone();

                self.discover_return = target.is_some();
            }
        });

        ui.label(RichText::new("Adding a mod saves your pack. Apply modpack or Launch modded to install its enabled mods.").small().color(MUTED));

        let mut addition = None;

        let mut matches = 0;

        let query = query.to_lowercase();

        egui::ScrollArea::vertical().id_salt("discover-results").auto_shrink([false,false]).max_height(ui.available_height()).show(ui, |ui| {

            ui.set_width(ui.available_width());

            for game in catalog.iter().filter(|g| *game_filter==0 || g.app_id==*game_filter) {

                for item in &game.mods {

                    if !format!("{} {} {}",game.name,item.name,item.description).to_lowercase().contains(&query) { continue; }

                    if !kind.is_empty() && item.content_type != *kind { continue; }

                    matches += 1;

                    let card = egui::Frame::new().fill(SURFACE).corner_radius(crate::ui_helpers::SURFACE_RADIUS).inner_margin(12).show(ui, |ui| {

                        ui.set_min_width(ui.available_width());

                        ui.horizontal(|ui| {

                        crate::ui_helpers::mod_art(ui,item,egui::vec2(64.0,64.0));

                        ui.vertical(|ui| {

                        ui.horizontal_wrapped(|ui| {

                            ui.label(RichText::new(&item.name).size(20.0).strong());

                            ui.label(RichText::new(format!("v{}",item.version)).color(GREEN));

                        });

                        ui.label(RichText::new(&game.name).color(GREEN));

                        crate::ui_helpers::mod_links(ui,item);

                        });

                        });

                        let preview: String = item.description.chars().take(160).collect();

                        ui.label(format!("{}{}",preview,if item.description.chars().count()>160 {"…"}else{""}));

                        if ui.button("Show more").clicked(){self.mod_details=Some(item.clone());self.mod_details_context=Some((game.clone(),source.cloned(),self.packs.iter().find(|p|Some(&p.id)==target.as_ref()).cloned()));self.mod_download_status.clear();}

                        if item.provenance["external_only"]==true {

                            ui.label("Official-site download. Steam manages Workshop subscriptions separately from Canna modpacks.");

                            if let Some(url)=item.provenance["source_url"].as_str(){ui.hyperlink_to("Subscribe on Steam Workshop",url);}

                            if !item.dependencies.is_empty(){ui.label(format!("Required: {}",item.dependencies.join(", ")));}

                            return;

                        }

                        if game.app_id==u32::MAX { if ui.button("Download from website").clicked() {ui.ctx().open_url(egui::OpenUrl::new_tab("https://cannamods.vip/?game=minecraft"));} return; }

                        let pack = self.packs.iter().find(|p|Some(&p.id)==target.as_ref());

                        let compatible = pack.is_some_and(|p|p.game.app_id==game.app_id && Some(&p.repository)==source);

                        let existing = pack.and_then(|p|p.mods.iter().find(|m|m.local_file.is_empty() && m.name==item.name));

                        let current = existing.is_some_and(|m|m.version==item.version && m.sha256==item.sha256 && m.file==item.file);

                        let label = if current {"Added"} else if existing.is_some() {"Update in modpack"} else {"+ Add to modpack"};

                        if ui.add_enabled(compatible && !current && !busy, egui::Button::new(label)).clicked() {

                            addition = target.clone().map(|id|(id,game.clone(),item.clone()));

                        }

                        if !compatible { ui.label(RichText::new("Choose a modpack for this game and connected repository.").small().color(MUTED)); }

                    });

                    #[cfg(test)]
                    self.discover_rects.push(card.response.rect);

                    #[cfg(not(test))]
                    let _ = card;

                    ui.add_space(12.0);

                }

            }

            if matches == 0 { empty_panel(ui,"Nothing here yet.","Try another search, or add mods through the community library on cannamods.vip."); }

        });

        if let Some((id, game, item)) = addition {
            match self.add_catalog_mod(&id, &game, source, item) {
                Ok(()) => {
                    self.status =
                        "Saved to modpack. Apply modpack or Launch modded when you're ready.".into()
                }

                Err(error) => self.status = format!("Could not add mod: {error}"),
            }
        }

        if !self.status.is_empty() {
            ui.label(&self.status);
        }
    }

    fn add_catalog_mod(
        &mut self,

        id: &str,

        game: &GameInfo,

        source: Option<&Source>,

        mut item: crate::model::ModInfo,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            item.provenance["external_only"] != true,
            "This addon is downloaded through its original site"
        );

        let index = self
            .packs
            .iter()
            .position(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("Modpack no longer exists"))?;

        let mut pack = self.packs[index].clone();

        anyhow::ensure!(
            pack.game.app_id == game.app_id
                && pack.game.folder == game.folder
                && Some(&pack.repository) == source,
            "Game or repository does not match this pack"
        );

        item.enabled = true;

        item.local_file.clear();

        // Add only the selected package. Declared dependency metadata must not
        // restore removed libraries or replace the user's alternative packages.

        if let Some(old) = pack
            .mods
            .iter_mut()
            .find(|m| m.local_file.is_empty() && m.name == item.name)
        {
            item.enabled = old.enabled;

            *old = item;
        } else {
            pack.mods.push(item);
        }

        pack.save()?;

        self.packs[index] = pack;

        Ok(())
    }

    pub fn pack_game(&self, id: &str) -> Option<u32> {
        self.packs
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.game.app_id)
    }

    pub fn editing(&self) -> bool {
        self.draft.is_some() || self.group_dialog || self.chooser
    }

    pub fn set_runtime_status(&mut self, status: &str) {
        self.status = status.to_owned();
    }

    #[cfg(test)]
    pub fn draft_game_id(&self) -> Option<u32> {
        self.draft.as_ref().map(|pack| pack.game.app_id)
    }

    pub fn open_creation_menu(&mut self) {
        self.chooser = true;
    }

    pub fn open_group_dialog(&mut self) {
        self.group_dialog = true;
    }

    pub fn open_first_pack(&mut self) {
        self.selected = self.packs.first().map(|pack| pack.id.clone());
    }

    pub fn mod_details_window(&mut self, ctx: &egui::Context) -> bool {
        let mut downloaded = false;

        if let Some(rx) = &self.mod_download {
            if let Ok(result) = rx.try_recv() {
                self.mod_download = None;

                self.mod_download_status = match result {
                    Ok(()) => {
                        downloaded = true;

                        "Downloaded. Find it in Your downloads.".into()
                    }

                    Err(e) => format!("Download failed: {e}"),
                };
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        let Some(item) = self.mod_details.clone() else {
            return downloaded;
        };

        let screen = ctx.content_rect();

        let width = (screen.width() - 72.0).clamp(280.0, 820.0);

        let height = (screen.height() - 120.0).clamp(300.0, 650.0);

        let mut close = false;

        let modal = egui::Modal::new(egui::Id::new("mod-details-modal"))
            .backdrop_color(Color32::from_black_alpha(185))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .fill(SURFACE)
                    .inner_margin(20),
            )
            .show(ctx, |ui| {
                ui.set_width(width);

                ui.set_height(height);

                ui.horizontal(|ui| {
                    if !crate::ui_helpers::mod_art(ui, &item, egui::vec2(96.0, 96.0)) {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(96.0, 96.0), egui::Sense::hover());

                        ui.painter()
                            .rect_filled(rect, 8, Color32::from_rgb(48, 69, 54));

                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            item.name.chars().next().unwrap_or('?'),
                            egui::FontId::proportional(32.0),
                            GREEN,
                        );
                    }

                    ui.vertical(|ui| {
                        ui.heading(&item.name);

                        ui.label(format!("Version {}", item.version));

                        crate::ui_helpers::mod_links(ui, &item);
                    });
                });

                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("mod-details-body")
                    .auto_shrink([false, true])
                    .max_height((height - 190.0).max(100.0))
                    .show(ui, |ui| {
                        ui.set_width(width);

                        ui.label(&item.description);

                        for (label, key) in
                            [("Game versions", "game_versions"), ("Loaders", "loaders")]
                        {
                            if let Some(values) = item.provenance[key].as_array() {
                                let values =
                                    values.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>();

                                if !values.is_empty() {
                                    ui.label(format!("{label}: {}", values.join(", ")));
                                }
                            }
                        }

                        if !item.dependencies.is_empty() {
                            ui.label(format!(
                                "Required dependencies: {}",
                                item.dependencies.join(", ")
                            ));
                        }

                        if let Some(notes) = item.provenance["install_notes"].as_str() {
                            ui.separator();

                            ui.label(notes);
                        }
                    });

                ui.separator();

                ui.horizontal(|ui| {
                    let can_download = self
                        .mod_details_context
                        .as_ref()
                        .is_some_and(|(_, source, _)| source.is_some())
                        && item.provenance["external_only"] != true
                        && item.sha256.len() == 64
                        && !crate::website::session().is_empty()
                        && self.mod_download.is_none();

                    if ui
                        .add_enabled(
                            can_download,
                            egui::Button::new(if self.mod_download.is_some() {
                                "Downloading..."
                            } else {
                                "Download"
                            }),
                        )
                        .clicked()
                    {
                        self.download_details(ctx, &item);
                    }

                    if let Some((game, _, _)) = &self.mod_details_context {
                        ui.hyperlink_to(
                            "Canna website",
                            format!("https://cannamods.vip/?game={}", game.app_id),
                        );
                    }

                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });

                if !self.mod_download_status.is_empty() {
                    ui.label(&self.mod_download_status);
                }
            });

        #[cfg(test)]
        {
            self.modal_rect = Some(modal.response.rect);
        }

        if close || modal.should_close() {
            self.mod_details = None;

            self.mod_details_context = None;
        }

        downloaded
    }

    fn download_details(&mut self, ctx: &egui::Context, item: &crate::model::ModInfo) {
        let Some((game, source, selected)) = self.mod_details_context.clone() else {
            return;
        };

        let Some(source) = source else {
            return;
        };

        let pack = selected
            .filter(|p| p.game.app_id == game.app_id)
            .unwrap_or_else(|| {
                Modpack::create("Downloads".into(), String::new(), &game, source, vec![])
            });

        let item = item.clone();

        let token = crate::website::session();

        let ctx = ctx.clone();

        let (tx, rx) = std::sync::mpsc::channel();

        self.mod_download = Some(rx);

        self.mod_download_status = "Downloading...".into();

        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                use sha2::{Digest, Sha256};

                let bytes = crate::repository::fetch_optional(
                    &crate::runtime::client()?,
                    &crate::runtime::settings(&pack),
                    &token,
                    &crate::runtime::repo_path(&pack, &item.file),
                    256 * 1024 * 1024,
                )?
                .ok_or_else(|| anyhow::anyhow!("Mod unavailable"))?;

                anyhow::ensure!(
                    format!("{:x}", Sha256::digest(&bytes)) == item.sha256.to_lowercase(),
                    "Mod checksum mismatch"
                );

                crate::website::remember_mod(&pack, &item, &bytes, false)?;

                Ok(())
            })()
            .map_err(|e| e.to_string());

            let _ = tx.send(result);

            ctx.request_repaint();
        });
    }

    pub fn new() -> Self {
        let (packs, warnings) = crate::modpacks::load_all();

        let mut groups = crate::modpacks::load_groups();

        for pack in &packs {
            if !pack.group.is_empty() && !groups.contains(&pack.group) {
                groups.push(pack.group.clone());
            }
        }

        Self {
            lab_games: Vec::new(),
            runtime_options: Default::default(),
            runtime_progress: vec![],

            lab_memory: BTreeMap::new(),

            lab: Default::default(),

            sharing: Default::default(),

            console_game: None,

            discover_pack: None,

            discover_return: false,

            owned_games: BTreeSet::new(),

            #[cfg(test)]
            add_mods_rect: None,

            runtime_requests: Default::default(),

            packs,

            draft: None,

            status: warnings.join("\n"),

            query: String::new(),

            selected: None,

            chooser: false,

            groups,

            group_dialog: false,

            group_name: String::new(),

            group_members: BTreeSet::new(),

            game_filter: 0,

            sort: 0,

            detail_tab: 0,

            content_query: String::new(),

            deleted: None,

            mod_details: None,

            mod_details_context: None,

            mod_download: None,

            mod_download_status: String::new(),

            #[cfg(test)]
            modal_rect: None,

            #[cfg(test)]
            discover_rects: Vec::new(),
        }
    }

    pub fn start_new(&mut self, game: &GameInfo, source: Option<&Source>) {
        self.draft = Some(Modpack::create(
            format!("{} Family Pack", game.name),
            String::new(),
            game,
            source.cloned().unwrap_or_else(empty_source),
            vec![],
        ));

        self.chooser = false;

        self.status.clear();

        self.runtime_requests
            .push_back(RuntimeAction::Setup(self.draft.as_ref().unwrap().clone()));
    }

    pub fn show(
        &mut self,

        ui: &mut egui::Ui,

        catalog: &[GameInfo],

        source: Option<&Source>,

        selected_game: u32,

        connection_busy: bool,

        artwork: &BTreeMap<u32, egui::TextureHandle>,
    ) -> Option<Source> {
        if let Some(pack) = self.sharing.poll(ui.ctx()) {
            self.upsert(pack);
        }

        let mut action = None;

        let mut connect = None;

        if let Some(pack) = self
            .selected
            .as_ref()
            .and_then(|id| self.packs.iter().find(|p| &p.id == id))
            .cloned()
        {
            if ui.button("‹  All modpacks").clicked() {
                self.selected = None;
            }

            ui.add_space(18.0);

            let header = ui.horizontal(|ui| {
                cover(ui, &pack, artwork, egui::vec2(86.0, 86.0));

                ui.vertical(|ui| {
                    ui.label(RichText::new(&pack.name).size(28.0).color(TEXT).strong());

                    ui.label(
                        RichText::new(format!(
                            "{}  /  {}  /  {} mods",
                            pack.game.name,
                            crate::model::framework_label(pack.game.app_id),
                            pack.mods.len()
                        ))
                        .color(GREEN),
                    );

                    if !pack.group.is_empty() {
                        ui.label(RichText::new(&pack.group).small().color(MUTED));
                    }
                });
            });

            crate::ui_helpers::context_menu(&header.response, |ui| {
                pack_menu(
                    ui,
                    &pack,
                    &mut action,
                    connection_busy,
                    self.owned_games.contains(&pack.game.app_id),
                )
            });

            ui.add_space(14.0);

            if let Some(branch) = crate::game_compat::required_branch(&pack) {
                ui.label(format!("Required Steam branch: {branch}"));
                if let Some(game) = self.lab_games.iter().find(|g| g.app_id == pack.game.app_id) {
                    let version = crate::steam::installed_version(game);
                    if let Err(error) = crate::game_compat::branch_status(branch, version.as_ref())
                    {
                        ui.label(
                            RichText::new(error.to_string())
                                .color(egui::Color32::from_rgb(240, 192, 120)),
                        );
                    }
                }
            }

            ui.horizontal_wrapped(|ui| {
                let add_mods = primary(ui, "+ Add Mods");

                #[cfg(test)]
                {
                    self.add_mods_rect = Some(add_mods.rect);
                }

                if add_mods.clicked() {
                    self.discover_pack = Some(pack.id.clone());
                }

                if ui
                    .add_enabled(
                        !connection_busy && !self.owned_games.contains(&pack.game.app_id),
                        egui::Button::new("Apply modpack"),
                    )
                    .on_hover_text(
                        "Install this pack's enabled mods into the game. Close the game first.",
                    )
                    .clicked()
                {
                    self.runtime_requests
                        .push_back(RuntimeAction::Install(pack.clone()));
                }

                if ui.button("Import local mod…").clicked() {
                    action = Some(Action::Local(pack.clone()));
                }

                if ui.button("Duplicate pack").clicked() {
                    action = Some(Action::Duplicate(pack.clone()));
                }

                if self.owned_games.contains(&pack.game.app_id)
                    && ui.button("Stop instance").clicked()
                {
                    self.runtime_requests
                        .push_back(RuntimeAction::Stop(pack.game.app_id));
                }

                if ui.button("Launch modded").clicked() {
                    self.runtime_requests
                        .push_back(RuntimeAction::Launch(pack.clone(), true));
                }

                if ui.button("Launch vanilla").clicked() {
                    self.runtime_requests
                        .push_back(RuntimeAction::Launch(pack.clone(), false));
                }

                if primary(ui, "Edit modpack").clicked() {
                    action = Some(Action::Edit(pack.clone()));
                }

                if ui.button("Export file…").clicked() {
                    action = Some(Action::Export(pack.clone()));
                }

                if ui.button("Console").clicked() {
                    self.console_game = Some(pack.game.app_id);
                }

                if ui
                    .add_enabled(
                        !connection_busy,
                        egui::Button::new(
                            RichText::new("Delete modpack").color(Color32::from_rgb(239, 143, 143)),
                        ),
                    )
                    .clicked()
                {
                    action = Some(Action::Delete(pack.clone()));
                }

                if !pack.repository.owner.is_empty()
                    && source != Some(&pack.repository)
                    && ui
                        .add_enabled(!connection_busy, egui::Button::new("Connect repository"))
                        .clicked()
                {
                    connect = Some(pack.repository.clone());
                }
            });

            if let Some(updated) = self.sharing.show(ui, &pack) {
                self.upsert(updated);
            }

            ui.add_space(16.0);

            ui.separator();

            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.detail_tab, 0, "Content");

                ui.selectable_value(&mut self.detail_tab, 1, "Pack details");

                ui.selectable_value(&mut self.detail_tab, 2, "Play Lab");
            });

            ui.add_space(16.0);

            if self.detail_tab == 0 {
                ui.add(
                    egui::TextEdit::singleline(&mut self.content_query)
                        .hint_text("Search included mods…")
                        .desired_width(f32::INFINITY),
                );

                ui.add_space(12.0);

                if pack.mods.is_empty() {
                    empty_panel(
                        ui,
                        "Room for a little chaos.",
                        "Use Add Mods to choose mods from your family's catalog, then Apply modpack or Launch modded.",
                    );
                }

                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("pack_content_table")
                        .num_columns(4)
                        .spacing([32.0, 18.0])
                        .striped(true)
                        .min_col_width(140.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new("PROJECT").small().color(MUTED));

                            ui.label(RichText::new("PINNED VERSION").small().color(MUTED));

                            ui.label(RichText::new("CATALOG").small().color(MUTED));

                            ui.label("ACTIONS");

                            ui.end_row();

                            for item in pack.mods.iter().filter(|m| {
                                m.name
                                    .to_lowercase()
                                    .contains(&self.content_query.to_lowercase())
                            }) {
                                let project = ui.vertical(|ui| {
                                    ui.label(RichText::new(&item.name).color(TEXT).strong());

                                    ui.label(RichText::new(&item.file).small().color(MUTED));
                                });

                                crate::ui_helpers::context_menu(&project.response, |ui| {
                                    if ui.button("Copy name").clicked() {
                                        ui.ctx().copy_text(item.name.clone());

                                        ui.close();
                                    }

                                    if ui.button("Copy file path").clicked() {
                                        ui.ctx().copy_text(item.file.clone());

                                        ui.close();
                                    }

                                    ui.separator();

                                    if ui
                                        .add_enabled(
                                            !connection_busy,
                                            egui::Button::new(if item.enabled {
                                                "Disable mod"
                                            } else {
                                                "Enable mod"
                                            }),
                                        )
                                        .clicked()
                                    {
                                        action = Some(Action::Toggle(
                                            pack.clone(),
                                            item.file.clone(),
                                            !item.enabled,
                                        ));

                                        ui.close();
                                    }

                                    if ui.button("Remove from modpack").clicked() {
                                        action =
                                            Some(Action::Remove(pack.clone(), item.file.clone()));

                                        ui.close();
                                    }
                                });

                                ui.label(RichText::new(&item.version).color(GREEN));

                                let current = source
                                    .filter(|s| *s == &pack.repository)
                                    .and_then(|_| {
                                        catalog.iter().find(|g| {
                                            g.app_id == pack.game.app_id
                                                && g.folder == pack.game.folder
                                        })
                                    })
                                    .and_then(|g| g.mods.iter().find(|m| m.file == item.file));

                                let label = if !item.local_file.is_empty() {
                                    "Local file".to_owned()
                                } else {
                                    match current {
                                        Some(m)
                                            if m.version == item.version
                                                && (item.sha256.is_empty()
                                                    || item
                                                        .sha256
                                                        .eq_ignore_ascii_case(&m.sha256)) =>
                                        {
                                            "Matches catalog".to_owned()
                                        }

                                        Some(m) => format!("Catalog: v{}", m.version),

                                        None => "Not verified".to_owned(),
                                    }
                                };

                                ui.label(RichText::new(label).color(MUTED));

                                ui.horizontal(|ui| {
                                    let mut enabled = item.enabled;

                                    if ui
                                        .add_enabled(
                                            !connection_busy,
                                            egui::Checkbox::new(&mut enabled, "Enabled"),
                                        )
                                        .changed()
                                    {
                                        action = Some(Action::Toggle(
                                            pack.clone(),
                                            item.file.clone(),
                                            enabled,
                                        ));
                                    }

                                    if ui.button("Remove").clicked() {
                                        action =
                                            Some(Action::Remove(pack.clone(), item.file.clone()));
                                    }
                                });

                                ui.end_row();
                            }
                        });
                });
            } else if self.detail_tab == 2 {
                self.lab.show_with_options(
                    ui,
                    &pack,
                    &self.lab_games,
                    catalog,
                    connection_busy,
                    self.runtime_options,
                );
                self.runtime_progress.append(&mut self.lab.runtime_progress);

                if let Some(m) = self.lab_memory.get(&pack.game.app_id) {
                    ui.label(format!(
                        "Game working set: {:.1} MiB; peak {:.1} MiB (local only)",
                        m.current as f64 / 1048576.,
                        m.peak as f64 / 1048576.
                    ));
                }

                if std::mem::take(&mut self.lab.console_requested) {
                    self.console_game = Some(pack.game.app_id);
                }

                if let Some(changed) = self.lab.changed.take() {
                    if let Some(existing) = self.packs.iter_mut().find(|p| p.id == changed.id) {
                        *existing = changed.clone();
                    } else {
                        self.packs.push(changed.clone());
                    }

                    self.selected = Some(changed.id);
                }
            } else {
                panel().show(ui, |ui| { ui.set_min_width(ui.available_width()); ui.heading("About this pack"); ui.label(if pack.description.is_empty() { "No description yet." } else { &pack.description }); ui.add_space(14.0); ui.label(RichText::new("SOURCE REPOSITORY").small().color(GREEN)); ui.label(repository_label(&pack.repository)); ui.add_space(14.0); ui.label("Export format: .canna.zip"); ui.label(RichText::new("Contains mod selections, version pins and local files. Use Apply modpack or Launch modded after importing.").small().color(MUTED)); });
            }
        } else {
            ui.label(RichText::new("YOUR FAMILY COLLECTION").small().color(GREEN));

            ui.horizontal(|ui| {
                ui.label(RichText::new("Modpacks").size(32.0).color(TEXT).strong());

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} packs", self.packs.len())).color(MUTED));
                });
            });

            ui.label(RichText::new("A setup for every kind of game night.").color(MUTED));

            ui.add_space(22.0);

            ui.horizontal(|ui| {
                let width = (ui.available_width() - 300.0).max(160.0);

                ui.add_sized(
                    [width, 40.0],
                    egui::TextEdit::singleline(&mut self.query).hint_text("Search packs or games…"),
                );

                if ui.button("+ New group").clicked() {
                    action = Some(Action::Group);
                }

                if primary(ui, "+ New modpack").clicked() {
                    action = Some(Action::New);
                }
            });

            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_id_salt("pack_sort")
                    .height(340.0)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .selected_text(["Name A–Z", "Name Z–A", "Most mods"][self.sort as usize])
                    .show_ui(ui, |ui| {
                        crate::ui_helpers::searchable_options(
                            ui,
                            &mut self.sort,
                            &[
                                (0, "Name A–Z".into()),
                                (1, "Name Z–A".into()),
                                (2, "Most mods".into()),
                            ],
                        );
                    });

                let games: BTreeMap<u32, String> = self
                    .packs
                    .iter()
                    .map(|p| (p.game.app_id, p.game.name.clone()))
                    .collect();

                egui::ComboBox::from_id_salt("pack_game_filter")
                    .height(340.0)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .selected_text(
                        games
                            .get(&self.game_filter)
                            .map(String::as_str)
                            .unwrap_or("All games"),
                    )
                    .show_ui(ui, |ui| {
                        let mut choices = vec![(0, "All games".into())];

                        choices.extend(games);

                        crate::ui_helpers::searchable_options(ui, &mut self.game_filter, &choices);
                    });

                if ui.button("Import modpack…").clicked() {
                    action = Some(Action::Import);
                }
            });

            ui.add_space(22.0);

            if self.packs.is_empty() {
                empty_panel(
                    ui,
                    "Your next game night starts here.",
                    "Create a pack for an installed game, or import one from your family.",
                );
            }

            let mut filtered: Vec<Modpack> = self
                .packs
                .iter()
                .filter(|p| {
                    (self.game_filter == 0 || p.game.app_id == self.game_filter)
                        && format!("{} {}", p.name, p.game.name)
                            .to_lowercase()
                            .contains(&self.query.to_lowercase())
                })
                .cloned()
                .collect();

            filtered.sort_by(|a, b| match self.sort {
                1 => b.name.to_lowercase().cmp(&a.name.to_lowercase()),

                2 => b.mods.len().cmp(&a.mods.len()).then(a.name.cmp(&b.name)),

                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            });

            if !self.packs.is_empty() && filtered.is_empty() {
                ui.label("No packs match your search.");
            }

            let mut groups = self.groups.clone();

            groups.insert(0, String::new());

            egui::ScrollArea::vertical().show(ui, |ui| {
                for group in groups {
                    let packs: Vec<_> = filtered.iter().filter(|p| p.group == group).collect();

                    if group.is_empty() && packs.is_empty() {
                        continue;
                    }

                    let name = if group.is_empty() {
                        "Ungrouped"
                    } else {
                        &group
                    };

                    egui::CollapsingHeader::new(
                        RichText::new(format!("{}  ·  {}", name, packs.len()))
                            .size(17.0)
                            .color(TEXT),
                    )
                    .id_salt(("pack_group", &group))
                    .default_open(true)
                    .show(ui, |ui| {
                        let columns =
                            ((ui.available_width() + 16.0) / 236.0).floor().max(1.0) as usize;

                        for row in packs.chunks(columns) {
                            ui.columns(columns, |uis| {
                                for (index, pack) in row.iter().enumerate() {
                                    let ui = &mut uis[index];

                                    let card = panel().show(ui, |ui| {
                                        let width = ui.available_width();

                                        if cover(ui, pack, artwork, egui::vec2(width, 124.0))
                                            .clicked()
                                        {
                                            action = Some(Action::Open(pack.id.clone()));
                                        }

                                        ui.add_space(6.0);

                                        ui.label(
                                            RichText::new(&pack.name)
                                                .size(17.0)
                                                .color(TEXT)
                                                .strong(),
                                        );

                                        ui.label(
                                            RichText::new(&pack.game.name).small().color(GREEN),
                                        );

                                        ui.label(
                                            RichText::new(format!(
                                                "{} mods  /  {}",
                                                pack.mods.len(),
                                                crate::model::framework_label(pack.game.app_id)
                                            ))
                                            .small()
                                            .color(MUTED),
                                        );

                                        ui.horizontal(|ui| {
                                            if ui.button("Open pack").clicked() {
                                                action = Some(Action::Open(pack.id.clone()));
                                            }

                                            ui.menu_button("•••", |ui| {
                                                pack_menu(
                                                    ui,
                                                    pack,
                                                    &mut action,
                                                    connection_busy,
                                                    self.owned_games.contains(&pack.game.app_id),
                                                );
                                            });
                                        });
                                    });

                                    crate::ui_helpers::context_menu(&card.response, |ui| {
                                        pack_menu(
                                            ui,
                                            pack,
                                            &mut action,
                                            connection_busy,
                                            self.owned_games.contains(&pack.game.app_id),
                                        )
                                    });

                                    #[cfg(test)]
                                    ui.ctx().data_mut(|data| {
                                        data.insert_temp(
                                            egui::Id::new(("pack_card", &pack.id)),
                                            card.response.rect,
                                        )
                                    });
                                }
                            });

                            ui.add_space(14.0);
                        }

                        if packs.is_empty() {
                            ui.label(
                                RichText::new("Add packs to this group from the pack editor.")
                                    .small()
                                    .color(MUTED),
                            );
                        }
                    });

                    ui.add_space(12.0);
                }
            });
        }

        if !self.status.is_empty() {
            ui.add_space(10.0);

            ui.label(RichText::new(&self.status).color(GREEN));
        }

        if self.deleted.is_some() && ui.button("Undo delete").clicked() {
            action = Some(Action::UndoDelete);
        }

        if self.chooser {
            let modal = modal("pack_start").show(ui.ctx(), |ui| {
                ui.set_width(480.0);

                dialog_header(ui, "Create a modpack", &mut action);

                ui.label(RichText::new("How would you like to start?").color(MUTED));

                ui.add_space(16.0);

                if option(
                    ui,
                    "01",
                    "Custom setup",
                    "Choose a Unity game and collect mods from your catalog.",
                )
                .clicked()
                {
                    action = Some(Action::Start);
                }

                ui.add_space(10.0);

                if option(
                    ui,
                    "02",
                    "Import a family pack",
                    "Open a .canna.zip bundle or .canna.json manifest.",
                )
                .clicked()
                {
                    action = Some(Action::Import);
                }

                ui.add_space(12.0);

                ui.label(
                    RichText::new("Built for your games. Shared with your people.")
                        .small()
                        .color(GREEN),
                );
            });

            if modal.should_close() {
                action = Some(Action::Cancel);
            }
        }

        if let Some(pack) = &mut self.draft {
            let editing = self.packs.iter().any(|p| p.id == pack.id);

            let modal = modal("pack_editor").show(ui.ctx(), |ui| {

                ui.set_width(520.0); dialog_header(ui, if editing {"Edit modpack"} else {"Create modpack"}, &mut action);

                egui::ScrollArea::vertical().max_height((ui.ctx().content_rect().height() - 230.0).max(260.0)).show(ui, |ui| {

                    ui.horizontal(|ui| {

                        cover(ui, pack, artwork, egui::vec2(108.0, 88.0));

                        ui.vertical(|ui| {ui.label(RichText::new("COVER COLOR").small().color(MUTED)); ui.horizontal(|ui| {for (id, label) in ["Forest", "Sage", "Plum", "Amber"].iter().enumerate() {if ui.add(egui::Button::new(*label).selected(pack.theme == id as u8)).clicked() {pack.theme = id as u8;}}});ui.label(RichText::new("Game artwork, with a little Canna color.").small().color(MUTED));});

                    }); ui.add_space(12.0);

                    ui.strong("Pack name"); ui.add(egui::TextEdit::singleline(&mut pack.name).desired_width(f32::INFINITY).hint_text("Family Bopl Night"));

                    ui.strong("Game");

                    let mut id = pack.game.app_id;

                    egui::ComboBox::from_id_salt("editor_game").height(340.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(480.0).selected_text(&pack.game.name).show_ui(ui, |ui| {let choices=catalog.iter().map(|game|(game.app_id,game.name.clone())).collect::<Vec<_>>();crate::ui_helpers::searchable_options(ui,&mut id,&choices);});

                    if id != pack.game.app_id && let Some(game) = catalog.iter().find(|g| g.app_id == id) {pack.game = crate::modpacks::PackGame {app_id: game.app_id, name: game.name.clone(), folder: game.folder.clone(), framework: crate::model::framework(game.app_id).into()};pack.repository = source.cloned().unwrap_or_else(empty_source);pack.mods.clear();}

                    ui.horizontal(|ui| {ui.label(RichText::new("FRAMEWORK").small().color(MUTED)); ui.label(RichText::new(crate::model::framework_label(pack.game.app_id)).color(GREEN));});

                    if crate::model::source_addons(pack.game.app_id).is_some() { ui.label("VPK packs launch in practice mode (-insecure). Vanilla removes Canna addons. Native plugins and bhop tools need their own supported setup."); }

                    ui.strong("Group"); egui::ComboBox::from_id_salt("editor_group").height(340.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(480.0).selected_text(if pack.group.is_empty() {"Ungrouped"} else {&pack.group}).show_ui(ui, |ui| {let mut choices=vec![(String::new(),"Ungrouped".into())];choices.extend(self.groups.iter().map(|group|(group.clone(),group.clone())));crate::ui_helpers::searchable_options(ui,&mut pack.group,&choices);});

                    ui.strong("Description"); ui.add(egui::TextEdit::multiline(&mut pack.description).desired_width(f32::INFINITY).desired_rows(2));

                    ui.add_space(10.0);ui.separator();ui.strong(format!("Choose mods  ·  {} selected", pack.mods.len()));

                    if source != Some(&pack.repository) && pack.mods.is_empty() && let Some(current) = source && ui.button("Use connected catalog").clicked() {pack.repository = current.clone();}

                    let game = source.filter(|s| *s == &pack.repository).and_then(|_| catalog.iter().find(|g| g.app_id == pack.game.app_id && g.folder == pack.game.folder));

                    if let Some(game) = game {

                        for item in &game.mods {

                            let mut included = pack.mods.iter().any(|m| m.file == item.file);

                            if ui.checkbox(&mut included, format!("{}  ·  v{}", item.name, item.version)).changed() {if included {pack.mods.push(item.clone());} else {pack.mods.retain(|m| m.file != item.file);}}

                            if let Some(pin) = pack.mods.iter().find(|m| m.file == item.file && m.version != item.version) {ui.label(RichText::new(format!("Keeps v{}; toggle off/on to update the pin.", pin.version)).small().color(MUTED));}

                        }

                    }

                    for item in pack.mods.clone().into_iter().filter(|m| game.is_none_or(|g| !g.mods.iter().any(|c| c.file == m.file))) {

                        let mut keep = true; if ui.checkbox(&mut keep, format!("{}  ·  v{}  ·  saved selection", item.name, item.version)).changed() {pack.mods.retain(|m| m.file != item.file);}

                    }

                    if game.is_none_or(|g| g.mods.is_empty()) {ui.add(egui::Label::new(RichText::new("No catalog mods available yet. Save your setup now and add mods once your repository is ready.").color(MUTED)).wrap());}

                });

                ui.add_space(12.0);ui.separator();

                ui.horizontal(|ui| {if ui.button("Cancel").clicked() {action = Some(Action::Cancel);}ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {if primary(ui, if editing {"Save changes"} else {"+ Create modpack"}).clicked() {action = Some(Action::Save);}});});

                if !self.status.is_empty() { ui.label(RichText::new(&self.status).color(GREEN)); }

            });

            if modal.should_close() {
                action = Some(Action::Cancel);
            }
        }

        if self.group_dialog {
            let modal = modal("create_group").show(ui.ctx(), |ui| {
                ui.set_width(480.0);

                dialog_header(ui, "Create a group", &mut action);

                ui.strong("Group name");

                ui.add(
                    egui::TextEdit::singleline(&mut self.group_name)
                        .desired_width(f32::INFINITY)
                        .hint_text("e.g. Family nights"),
                );

                ui.add_space(14.0);

                ui.label(RichText::new("ADD MODPACKS").small().color(GREEN));

                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for pack in &self.packs {
                            let mut included = self.group_members.contains(&pack.id);

                            if ui.checkbox(&mut included, &pack.name).changed() {
                                if included {
                                    self.group_members.insert(pack.id.clone());
                                } else {
                                    self.group_members.remove(&pack.id);
                                }
                            }
                        }

                        if self.packs.is_empty() {
                            ui.label("Create the group now, then add packs from their editor.");
                        }
                    });

                ui.add_space(18.0);

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        action = Some(Action::Cancel);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary(ui, "+ Create group").clicked() {
                            action = Some(Action::SaveGroup);
                        }
                    });
                });

                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });

            if modal.should_close() {
                action = Some(Action::Cancel);
            }
        }

        match action {
            Some(Action::Stop(id)) => self.runtime_requests.push_back(RuntimeAction::Stop(id)),

            Some(Action::Discover(id)) => self.discover_pack = Some(id),

            Some(Action::Toggle(mut pack, file, enabled)) => {
                match pack
                    .set_mod_enabled(&file, enabled)
                    .and_then(|()| pack.save())
                {
                    Ok(()) => {
                        self.upsert(pack);

                        self.status = "Mod state saved. Apply modpack or Launch modded to apply it with the game closed.".into();
                    }

                    Err(error) => self.status = error.to_string(),
                }
            }

            Some(Action::Delete(pack)) => match pack.delete() {
                Ok(path) => {
                    self.deleted = Some(path);

                    self.packs.retain(|p| p.id != pack.id);

                    if self.selected.as_ref() == Some(&pack.id) {
                        self.selected = None;
                    }

                    if self.draft.as_ref().is_some_and(|p| p.id == pack.id) {
                        self.draft = None;
                    }

                    self.runtime_requests.retain(|request| match request {
                        RuntimeAction::Setup(p)
                        | RuntimeAction::Install(p)
                        | RuntimeAction::Launch(p, _) => p.id != pack.id,

                        RuntimeAction::LaunchCurrent(_) | RuntimeAction::Stop(_) => true,
                    });

                    self.group_members.remove(&pack.id);

                    self.status = format!(
                        "Deleted {}. Undo is available. Installed game files are unchanged.",
                        pack.name
                    );
                }

                Err(error) => self.status = format!("Could not delete modpack: {error}"),
            },

            Some(Action::UndoDelete) => {
                if let Some(path) = self.deleted.clone() {
                    match Modpack::restore_deleted(&path) {
                        Ok(pack) => {
                            self.upsert(pack);

                            self.deleted = None;

                            self.status = "Modpack restored.".into();
                        }

                        Err(error) => self.status = error.to_string(),
                    }
                }
            }

            Some(Action::Install(pack)) => self
                .runtime_requests
                .push_back(RuntimeAction::Install(pack)),

            Some(Action::Launch(pack, modded)) => self
                .runtime_requests
                .push_back(RuntimeAction::Launch(pack, modded)),

            Some(Action::Local(mut pack)) => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Add a local plugin DLL, VPK or ZIP")
                    .add_filter("Mods", &["dll", "zip", "vpk"])
                    .pick_file()
                {
                    match crate::modpacks::add_local(&path) {
                        Ok(item) => {
                            pack.mods.retain(|m| m.file != item.file);

                            pack.mods.push(item);

                            match pack.save() {
                                Ok(()) => {
                                    self.upsert(pack);

                                    self.status="Local mod added. Use Apply modpack or Launch modded to apply it.".into();
                                }

                                Err(error) => self.status = error.to_string(),
                            }
                        }

                        Err(error) => self.status = error.to_string(),
                    }
                }
            }

            Some(Action::Remove(mut pack, file)) => {
                pack.mods.retain(|m| m.file != file);

                match pack.save() {
                    Ok(()) => {
                        self.upsert(pack);

                        self.status =
                            "Removed from pack. Apply modpack to apply the change.".into();
                    }

                    Err(error) => self.status = error.to_string(),
                }
            }

            Some(Action::Duplicate(pack)) => {
                let game = GameInfo {
                    app_id: pack.game.app_id,

                    name: pack.game.name.clone(),

                    folder: pack.game.folder.clone(),

                    description: String::new(),

                    icon: String::new(),

                    mods: vec![],

                    mod_folder_status: String::new(),
                };

                let mut copy = Modpack::create(
                    format!("{} copy", pack.name),
                    pack.description.clone(),
                    &game,
                    pack.repository.clone(),
                    pack.mods.clone(),
                );

                copy.theme = pack.theme;

                copy.group = pack.group.clone();

                self.draft = Some(copy);
            }

            Some(Action::New) => {
                self.chooser = true;

                self.status.clear();
            }

            Some(Action::Start) => {
                if let Some(game) = catalog
                    .iter()
                    .find(|g| g.app_id == selected_game)
                    .or_else(|| catalog.first())
                {
                    self.start_new(game, source);
                }
            }

            Some(Action::Open(id)) => {
                self.selected = Some(id);

                self.detail_tab = 0;

                self.content_query.clear();

                self.status.clear();
            }

            Some(Action::Edit(pack)) => {
                self.draft = Some(pack);

                self.status.clear();
            }

            Some(Action::Cancel) => {
                self.chooser = false;

                self.group_dialog = false;

                self.draft = None;

                self.status.clear();
            }

            Some(Action::Save) => {
                if let Some(pack) = &self.draft {
                    match pack.save() {
                        Ok(()) => {
                            let pack = pack.clone();

                            self.runtime_requests
                                .push_back(RuntimeAction::Setup(pack.clone()));

                            self.selected = Some(pack.id.clone());

                            self.status = format!("Saved {}", pack.name);

                            self.upsert(pack);

                            self.draft = None;
                        }

                        Err(e) => self.status = format!("Could not save pack: {e}"),
                    }
                }
            }

            Some(Action::Import) => {
                self.chooser = false;

                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Import Canna modpack")
                    .add_filter("Canna modpack", &["json", "zip"])
                    .pick_file()
                {
                    match Modpack::import(&path) {
                        Ok(pack) => {
                            self.selected = Some(pack.id.clone());

                            self.status = format!("Imported {}", pack.name);

                            self.upsert(pack);
                        }

                        Err(e) => self.status = format!("Could not import: {e}"),
                    }
                }
            }

            Some(Action::Export(pack)) => {
                let filename: String = pack
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == ' ' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();

                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Export Canna modpack")
                    .set_file_name(format!("{filename}.canna.zip"))
                    .add_filter("Canna modpack bundle", &["zip"])
                    .save_file()
                {
                    self.status = match pack.export(&path) {
                        Ok(()) => format!("Exported to {}", path.display()),

                        Err(e) => format!("Could not export: {e}"),
                    };
                }
            }

            Some(Action::Group) => {
                self.group_dialog = true;

                self.group_name.clear();

                self.group_members.clear();

                self.status.clear();
            }

            Some(Action::SaveGroup) => {
                let name = self.group_name.trim().to_owned();

                if name.is_empty() || self.groups.iter().any(|g| g.eq_ignore_ascii_case(&name)) {
                    self.status = "Choose a unique group name.".into();
                } else {
                    let mut groups = self.groups.clone();

                    groups.push(name.clone());

                    match crate::modpacks::save_groups(&groups) {
                        Ok(()) => {
                            self.groups = groups;

                            let members: Vec<_> = self
                                .packs
                                .iter()
                                .filter(|p| self.group_members.contains(&p.id))
                                .cloned()
                                .collect();

                            let mut errors = vec![];

                            for mut pack in members {
                                pack.group = name.clone();

                                match pack.save() {
                                    Ok(()) => self.upsert(pack),

                                    Err(e) => errors.push(e.to_string()),
                                }
                            }

                            self.group_dialog = false;

                            self.status = if errors.is_empty() {
                                format!("Created {name}")
                            } else {
                                format!(
                                    "Group created; some assignments failed: {}",
                                    errors.join("; ")
                                )
                            };
                        }

                        Err(e) => self.status = e.to_string(),
                    }
                }
            }

            None => {}
        }

        connect
    }

    fn upsert(&mut self, pack: Modpack) {
        if !pack.group.is_empty() && !self.groups.contains(&pack.group) {
            self.groups.push(pack.group.clone());
        }

        if let Some(old) = self.packs.iter_mut().find(|p| p.id == pack.id) {
            *old = pack;
        } else {
            self.packs.push(pack);
        }

        self.packs.sort_by_key(|p| p.name.to_lowercase());
    }
}

fn pack_menu(
    ui: &mut egui::Ui,

    pack: &Modpack,

    action: &mut Option<Action>,

    busy: bool,

    owned: bool,
) {
    if owned && ui.button("Stop instance").clicked() {
        *action = Some(Action::Stop(pack.game.app_id));

        ui.close();
    }

    for (label, next) in [
        ("Open modpack", Action::Open(pack.id.clone())),
        ("Discover mods", Action::Discover(pack.id.clone())),
        ("Edit modpack", Action::Edit(pack.clone())),
        ("Import local mod…", Action::Local(pack.clone())),
        ("Duplicate", Action::Duplicate(pack.clone())),
        ("Export…", Action::Export(pack.clone())),
    ] {
        if ui.button(label).clicked() {
            *action = Some(next);

            ui.close();
        }
    }

    ui.separator();

    for (label, next) in [
        ("Apply modpack", Action::Install(pack.clone())),
        ("Launch modded", Action::Launch(pack.clone(), true)),
        ("Launch vanilla", Action::Launch(pack.clone(), false)),
    ] {
        if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
            *action = Some(next);

            ui.close();
        }
    }

    ui.separator();

    let delete = ui.add_enabled(
        !busy,
        egui::Button::new(RichText::new("Delete modpack").color(Color32::from_rgb(239, 143, 143))),
    );

    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new(("pack_delete", &pack.id)), delete.rect));

    if delete.clicked() {
        *action = Some(Action::Delete(pack.clone()));

        ui.close();
    }
}

fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(text)
                .color(Color32::from_rgb(19, 35, 22))
                .strong(),
        )
        .fill(GREEN)
        .corner_radius(crate::ui_helpers::CONTROL_RADIUS),
    )
}

fn panel() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(54, 69, 59)))
        .inner_margin(16)
        .corner_radius(crate::ui_helpers::SURFACE_RADIUS)
}

fn modal(id: &str) -> egui::Modal {
    egui::Modal::new(egui::Id::new(id))
        .backdrop_color(Color32::from_black_alpha(190))
        .frame(panel().inner_margin(24))
}

fn dialog_header(ui: &mut egui::Ui, title: &str, action: &mut Option<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(24.0).color(TEXT).strong());

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("×").clicked() {
                *action = Some(Action::Cancel);
            }
        });
    });

    ui.add_space(10.0);

    ui.separator();

    ui.add_space(10.0);
}

fn option(ui: &mut egui::Ui, number: &str, title: &str, description: &str) -> egui::Response {
    panel()
        .fill(Color32::from_rgb(38, 51, 42))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.label(RichText::new(number).size(24.0).color(GREEN));

                ui.vertical(|ui| {
                    ui.label(RichText::new(title).color(TEXT).strong());

                    ui.label(RichText::new(description).small().color(MUTED));
                });
            });
        })
        .response
        .interact(egui::Sense::click())
}

fn empty_panel(ui: &mut egui::Ui, title: &str, description: &str) {
    panel().show(ui, |ui| {
        ui.set_min_width(ui.available_width());

        ui.add_space(16.0);

        ui.label(RichText::new(title).size(22.0).color(TEXT));

        ui.label(RichText::new(description).color(MUTED));

        ui.add_space(16.0);
    });
}

fn cover(
    ui: &mut egui::Ui,

    pack: &Modpack,

    artwork: &BTreeMap<u32, egui::TextureHandle>,

    size: egui::Vec2,
) -> egui::Response {
    let colors = [
        Color32::from_rgb(59, 95, 59),
        Color32::from_rgb(69, 96, 91),
        Color32::from_rgb(87, 66, 102),
        Color32::from_rgb(118, 85, 46),
    ];

    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    let painter = ui.painter();

    let color = colors[usize::from(pack.theme.min(3))];

    painter.rect_filled(rect, crate::ui_helpers::SURFACE_RADIUS, color);

    if let Some(texture) = artwork.get(&pack.game.app_id) {
        let src = texture.size_vec2();

        let ratio = size.x / size.y;

        let src_ratio = src.x / src.y;

        let uv = if src_ratio > ratio {
            let w = ratio / src_ratio;

            egui::Rect::from_min_max(
                egui::pos2((1.0 - w) / 2.0, 0.0),
                egui::pos2((1.0 + w) / 2.0, 1.0),
            )
        } else {
            let h = src_ratio / ratio;

            egui::Rect::from_min_max(
                egui::pos2(0.0, (1.0 - h) / 2.0),
                egui::pos2(1.0, (1.0 + h) / 2.0),
            )
        };

        egui::Image::new(texture)
            .uv(uv)
            .corner_radius(crate::ui_helpers::SURFACE_RADIUS - 4)
            .paint_at(ui, rect.shrink(4.0));

        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.left() + 4.0, rect.bottom() - 26.0),
                egui::vec2(rect.width() - 8.0, 22.0),
            ),
            egui::CornerRadius {
                nw: 0,

                ne: 0,

                sw: crate::ui_helpers::SURFACE_RADIUS - 4,

                se: crate::ui_helpers::SURFACE_RADIUS - 4,
            },
            Color32::from_black_alpha(190),
        );

        painter.text(
            egui::pos2(rect.left() + 12.0, rect.bottom() - 15.0),
            egui::Align2::LEFT_CENTER,
            if size.x < 150.0 {
                "CANNA"
            } else {
                "CANNA / FAMILY PACK"
            },
            egui::FontId::proportional(9.0),
            GREEN,
        );
    } else {
        let c = rect.center();

        let r = size.y.min(size.x) * 0.3;

        painter.circle_filled(c, r, color.gamma_multiply(1.4));

        for angle in [-0.8_f32, 0.0, 0.8] {
            let end = c + egui::vec2(angle.sin() * r, -angle.cos() * r);

            painter.line_segment(
                [c + egui::vec2(0.0, r * 0.65), end],
                egui::Stroke::new(4.0_f32, GREEN),
            );
        }

        painter.text(
            egui::pos2(c.x, rect.bottom() - 15.0),
            egui::Align2::CENTER_CENTER,
            "CANNA",
            egui::FontId::proportional(10.0),
            TEXT,
        );
    }

    response
}

fn empty_source() -> Source {
    Source {
        owner: String::new(),

        repository: "manager-uploaded-mods".into(),

        branch: "main".into(),

        catalog_folder: String::new(),
    }
}

fn repository_label(source: &Source) -> String {
    if source.owner.is_empty() {
        "Repository not connected yet".into()
    } else {
        format!(
            "{}/{} · {} · {}",
            source.owner,
            source.repository,
            source.branch,
            if source.catalog_folder.is_empty() {
                "repository root"
            } else {
                &source.catalog_folder
            }
        )
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn catalog_details_preserve_the_exact_fork_and_match_actual_pack_metadata() {
        let mut game = crate::model::bopl();
        game.app_id = 1557740;
        game.name = "ROUNDS".into();
        game.folder = "rounds".into();
        let source = Source {
            owner: "canna".into(),
            repository: "server".into(),
            branch: "main".into(),
            catalog_folder: String::new(),
        };
        let item = crate::model::ModInfo {
            name: "HollowPurple Fixed".into(),
            version: "1.8.2".into(),
            content_type: "mod".into(),
            description: "Pinned fork, upstream credited".into(),
            provenance: serde_json::json!({"provider":"canna","source_url":"https://thunderstore.io/c/rounds/p/flofl/HollowPurple/"}),
            enabled: true,
            file: "Mods/00000000-0000-4000-8000-000000000001.zip".into(),
            sha256: "a".repeat(64),
            local_file: String::new(),
            dependencies: vec!["UnboundLib".into()],
        };
        let compatible =
            Modpack::create("Bopl".into(), String::new(), &game, source.clone(), vec![]);
        let wrong_game = Modpack::create(
            "ROUNDS".into(),
            String::new(),
            &crate::model::bopl(),
            source.clone(),
            vec![],
        );
        let mut page = PackUi::new();
        page.packs = vec![compatible.clone(), wrong_game.clone()];
        page.open_catalog_details(&game, Some(&source), Some(&compatible.id), item.clone());
        assert_eq!(
            serde_json::to_value(page.mod_details.as_ref().unwrap()).unwrap(),
            serde_json::to_value(&item).unwrap()
        );
        assert_eq!(
            page.mod_details_context
                .as_ref()
                .unwrap()
                .2
                .as_ref()
                .unwrap()
                .id,
            compatible.id
        );
        assert!(
            page.mod_download.is_none(),
            "Opening details must not download or import the upstream project"
        );
        page.open_catalog_details(&game, Some(&source), Some(&wrong_game.id), item.clone());
        assert!(page.mod_details_context.as_ref().unwrap().2.is_none());
        let mut different_source = source.clone();
        different_source.branch = "other".into();
        page.open_catalog_details(
            &game,
            Some(&different_source),
            Some(&compatible.id),
            item.clone(),
        );
        assert!(page.mod_details_context.as_ref().unwrap().2.is_none());
        let mut different_folder = game.clone();
        different_folder.folder = "other-rounds".into();
        assert!(!page.catalog_pack_matches(&compatible.id, &different_folder, Some(&source)));
        assert!(
            page.mod_details
                .as_ref()
                .unwrap()
                .file
                .ends_with("000000000001.zip")
        );
    }

    #[test]
    fn discover_cards_use_width_and_show_multiple_results() {
        let ctx = egui::Context::default();

        let mut page = PackUi::new();

        let mut game = crate::model::bopl();

        game.mods = (0..4)
            .map(|n| crate::model::ModInfo {
                name: format!("Mod {n}"),

                version: "1.0".into(),

                description: "A useful description. ".repeat(20),

                provenance: serde_json::json!({"source_url":"https://example.com/mod"}),

                content_type: String::new(),

                enabled: false,

                file: String::new(),

                sha256: String::new(),

                local_file: String::new(),

                dependencies: vec![],
            })
            .collect();

        let mut state = DiscoverState {
            game: game.app_id,

            ..Default::default()
        };

        for _ in 0..4 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 650.0),
                    )),

                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        page.discover(ui, &[game.clone()], None, &mut state, false);
                    });
                },
            );
        }

        assert!(
            page.discover_rects[0].width() > 1000.0,
            "{:?}",
            page.discover_rects
        );

        assert!(
            page.discover_rects[1].bottom() < 650.0,
            "{:?}",
            page.discover_rects
        );
    }

    #[test]
    fn details_modal_fits_small_windows_and_closes_on_escape() {
        for size in [egui::vec2(1240.0, 820.0), egui::vec2(840.0, 560.0)] {
            let ctx = egui::Context::default();

            let mut page = PackUi::new();

            page.mod_details = Some(crate::model::ModInfo {
                name: "A mod with a longer name".into(),

                version: "1.2.3".into(),

                description: "A long description for scrolling. ".repeat(200),

                provenance: serde_json::json!({"author_links":[{"name":"Author","url":"https://example.com/author"}],"source_url":"https://example.com/mod"}),

                content_type: String::new(),

                enabled: false,

                file: String::new(),

                sha256: String::new(),

                local_file: String::new(),

                dependencies: vec![],
            });

            let input = |events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),

                events,

                ..Default::default()
            };

            for _ in 0..10 {
                let _ = ctx.run(input(vec![]), |ctx| {
                    page.mod_details_window(ctx);
                });
            }

            let rect = page.modal_rect.unwrap();

            assert!(
                rect.left() >= 0.0
                    && rect.top() >= 0.0
                    && rect.right() <= size.x
                    && rect.bottom() <= size.y,
                "{size:?}: {rect:?}"
            );

            assert!(
                (rect.center() - egui::pos2(size.x / 2.0, size.y / 2.0)).length() < 2.0,
                "{rect:?}"
            );

            let _ = ctx.run(
                input(vec![egui::Event::Key {
                    key: egui::Key::Escape,

                    physical_key: None,

                    pressed: true,

                    repeat: false,

                    modifiers: Default::default(),
                }]),
                |ctx| {
                    page.mod_details_window(ctx);
                },
            );

            assert!(page.mod_details.is_none());
        }
    }

    #[test]
    fn discover_updates_only_target_pack_and_rejects_other_repositories() {
        let game = crate::model::bopl();

        let source = Source::from_settings(&crate::model::Settings::load());

        let item = crate::model::ModInfo {
            provenance: serde_json::Value::Null,

            content_type: String::new(),

            enabled: false,

            name: "Discovery fixture".into(),

            version: "1.0.0".into(),

            file: "Mods/fixture.zip".into(),

            description: String::new(),

            sha256: "a".repeat(64),

            local_file: String::new(),

            dependencies: Vec::new(),
        };

        let first = Modpack::create(
            "Discover fixture".into(),
            String::new(),
            &game,
            source.clone(),
            vec![item.clone()],
        );

        let second = Modpack::create(
            "Untouched fixture".into(),
            String::new(),
            &game,
            source.clone(),
            vec![],
        );

        let mut page = PackUi::new();

        page.packs = vec![first.clone(), second];

        let mut updated = item.clone();

        updated.version = "1.1.0".into();

        updated.file = "Mods/fixture-new.zip".into();

        assert!(
            page.add_catalog_mod(&first.id, &game, None, updated.clone())
                .is_err()
        );

        let mut wrong = game.clone();

        wrong.app_id += 1;

        assert!(
            page.add_catalog_mod(&first.id, &wrong, Some(&source), updated.clone())
                .is_err()
        );

        page.add_catalog_mod(&first.id, &game, Some(&source), updated)
            .unwrap();

        let saved: Modpack = serde_json::from_slice(
            &std::fs::read(crate::modpacks::directory().join(format!("{}.canna.json", first.id)))
                .unwrap(),
        )
        .unwrap();

        assert_eq!(saved.mods.len(), 1);

        assert_eq!(saved.mods[0].version, "1.1.0");

        assert!(!saved.mods[0].enabled);

        assert!(page.packs[1].mods.is_empty());

        std::fs::remove_file(crate::modpacks::directory().join(format!("{}.canna.json", first.id)))
            .unwrap();
    }

    #[test]
    fn catalog_addition_preserves_disabled_libraries_and_replacement_packages() {
        let root = std::env::temp_dir().join(format!(
            "canna-catalog-overrides-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::modpacks::with_test_root(root.clone(), || {
            let mut game = crate::model::bopl();
            let source = Source {
                owner: "fixture".into(),
                repository: "mods".into(),
                branch: "main".into(),
                catalog_folder: "games".into(),
            };
            let library = crate::model::ModInfo {
                name: "UnboundLib".into(),
                version: "pinned-library".into(),
                enabled: false,
                file: "Mods/legacy-library.zip".into(),
                sha256: "a".repeat(64),
                description: "Keep this disabled pin".into(),
                provenance: serde_json::json!({"original_source":"legacy-fixture"}),
                content_type: String::new(),
                local_file: String::new(),
                dependencies: vec!["MMHook".into()],
            };
            let mut replacement = library.clone();
            replacement.name = "DuctTape replacement fixture".into();
            replacement.file = "Mods/replacement.zip".into();
            replacement.enabled = true;
            replacement.dependencies.clear();
            let original_choices = vec![library.clone(), replacement];
            let pack = Modpack::create(
                "Replacement fixture".into(),
                String::new(),
                &game,
                source.clone(),
                original_choices.clone(),
            );
            let mut catalog_library = library;
            catalog_library.version = "new-catalog-library".into();
            catalog_library.file = "Mods/new-library.zip".into();
            catalog_library.enabled = true;
            game.mods = vec![catalog_library];
            let mut selected = game.mods[0].clone();
            selected.name = "Selected old mod".into();
            selected.file = "Mods/selected.zip".into();
            selected.dependencies = vec!["UnboundLib".into(), "MMHook".into()];
            let mut page = PackUi::new();
            page.packs = vec![pack.clone()];
            page.add_catalog_mod(&pack.id, &game, Some(&source), selected)
                .unwrap();
            assert_eq!(page.packs[0].mods.len(), 3);
            assert_eq!(
                serde_json::to_value(&page.packs[0].mods[..2]).unwrap(),
                serde_json::to_value(&original_choices).unwrap(),
                "Adding a mod must preserve every field of other chosen packages"
            );
            assert!(page.packs[0].mods[2].enabled);
            assert_eq!(
                page.packs[0].mods[2].dependencies,
                vec!["UnboundLib", "MMHook"]
            );
            let saved: Modpack = serde_json::from_slice(
                &std::fs::read(
                    crate::modpacks::directory().join(format!("{}.canna.json", pack.id)),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(&saved.mods).unwrap(),
                serde_json::to_value(&page.packs[0].mods).unwrap()
            );
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_addition_does_not_inject_absent_declared_dependencies() {
        let root = std::env::temp_dir().join(format!(
            "canna-catalog-no-auto-dependencies-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::modpacks::with_test_root(root.clone(), || {
            let mut game = crate::model::bopl();
            let source = Source {
                owner: "fixture".into(),
                repository: "mods".into(),
                branch: "main".into(),
                catalog_folder: "games".into(),
            };
            let library = crate::model::ModInfo {
                name: "UnboundLib".into(),
                version: "library-fixture".into(),
                enabled: true,
                file: "Mods/library.zip".into(),
                sha256: "a".repeat(64),
                description: String::new(),
                provenance: serde_json::Value::Null,
                content_type: String::new(),
                local_file: String::new(),
                dependencies: vec![],
            };
            game.mods = vec![library.clone()];
            let mut selected = library;
            selected.name = "Selected old mod".into();
            selected.file = "Mods/selected.zip".into();
            selected.dependencies = vec!["UnboundLib".into(), "Unavailable legacy package".into()];
            let pack = Modpack::create(
                "Target-only addition fixture".into(),
                String::new(),
                &game,
                source.clone(),
                vec![],
            );
            let mut page = PackUi::new();
            page.packs = vec![pack.clone()];
            page.add_catalog_mod(&pack.id, &game, Some(&source), selected.clone())
                .unwrap();
            assert_eq!(page.packs[0].mods.len(), 1);
            assert_eq!(page.packs[0].mods[0].name, selected.name);
            assert_eq!(page.packs[0].mods[0].dependencies, selected.dependencies);
            assert!(page.packs[0].mods[0].enabled);
        });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn right_click_on_card_opens_pack_actions_without_opening_the_pack() {
        let context = egui::Context::default();

        let game = crate::model::bopl();

        let source = empty_source();

        let pack = Modpack::create(
            "Context menu fixture".into(),
            String::new(),
            &game,
            source.clone(),
            vec![],
        );

        let id = pack.id.clone();

        let mut page = PackUi::new();

        page.packs = vec![pack];

        page.groups.clear();

        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 820.0),
            )),

            events,

            ..Default::default()
        };

        let render = |ctx: &egui::Context, page: &mut PackUi| {
            egui::CentralPanel::default().show(ctx, |ui| {
                page.show(
                    ui,
                    std::slice::from_ref(&game),
                    Some(&source),
                    game.app_id,
                    false,
                    &BTreeMap::new(),
                );
            });
        };

        for _ in 0..10 {
            let _ = context.run(input(vec![]), |ctx| render(ctx, &mut page));
        }

        let rect = context
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("pack_card", &id))))
            .unwrap();

        let pos = rect.min + egui::vec2(5.0, 5.0);

        for pressed in [true, false] {
            let _ = context.run(
                input(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,

                        button: egui::PointerButton::Secondary,

                        pressed,

                        modifiers: egui::Modifiers::NONE,
                    },
                ]),
                |ctx| render(ctx, &mut page),
            );
        }

        let _ = context.run(input(vec![]), |ctx| render(ctx, &mut page));

        assert!(page.selected.is_none());

        assert!(
            context
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("pack_delete", &id))))
                .is_some(),
            "Right-click menu must contain Delete modpack"
        );
    }

    #[test]
    fn add_mods_opens_discover_for_selected_pack() {
        let context = egui::Context::default();

        let game = crate::model::bopl();

        let source = Source {
            owner: "fortnitegamerboy2020".into(),

            repository: "manager-uploaded-mods".into(),

            branch: "main".into(),

            catalog_folder: String::new(),
        };

        let pack = Modpack::create(
            "Button fixture".into(),
            String::new(),
            &game,
            source.clone(),
            vec![],
        );

        let mut page = PackUi::new();

        page.packs = vec![pack.clone()];

        page.selected = Some(pack.id);

        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1240.0, 820.0),
            )),

            events,

            ..Default::default()
        };

        let render = |ctx: &egui::Context, page: &mut PackUi| {
            egui::CentralPanel::default().show(ctx, |ui| {
                page.show(
                    ui,
                    std::slice::from_ref(&game),
                    Some(&source),
                    game.app_id,
                    false,
                    &BTreeMap::new(),
                );
            });
        };

        for _ in 0..10 {
            let _ = context.run(input(vec![]), |ctx| render(ctx, &mut page));
        }

        let pos = page.add_mods_rect.unwrap().center();

        for pressed in [true, false] {
            let _ = context.run(
                input(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,

                        button: egui::PointerButton::Primary,

                        pressed,

                        modifiers: egui::Modifiers::NONE,
                    },
                ]),
                |ctx| render(ctx, &mut page),
            );
        }

        assert_eq!(page.discover_pack, page.selected);

        assert!(page.draft.is_none());
    }
}
