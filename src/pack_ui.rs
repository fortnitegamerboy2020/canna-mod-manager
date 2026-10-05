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
    pub game: u32,
    pub target: Option<String>,
}
pub struct PackUi {
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
        } = state;
        ui.label(RichText::new("Discover").size(32.0).strong().color(TEXT));
        ui.label(RichText::new("Find your family's next favorite mod.").color(MUTED));
        ui.add_space(14.0);
        ui.add_sized(
            [ui.available_width(), 40.0],
            egui::TextEdit::singleline(query)
                .hint_text("Search mods, games, or descriptions…")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("discover_game")
                .selected_text(
                    catalog
                        .iter()
                        .find(|g| g.app_id == *game_filter)
                        .map(|g| g.name.as_str())
                        .unwrap_or("All games"),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(game_filter, 0, "All games");
                    for game in catalog {
                        ui.selectable_value(game_filter, game.app_id, &game.name);
                    }
                });
            ui.label("Add to");
            egui::ComboBox::from_id_salt("discover_pack")
                .selected_text(
                    self.packs
                        .iter()
                        .find(|p| Some(&p.id) == target.as_ref())
                        .map(|p| p.name.as_str())
                        .unwrap_or("Choose a modpack"),
                )
                .show_ui(ui, |ui| {
                    for pack in &self.packs {
                        ui.selectable_value(
                            target,
                            Some(pack.id.clone()),
                            format!("{} · {}", pack.name, pack.game.name),
                        );
                    }
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            for game in catalog.iter().filter(|g| *game_filter==0 || g.app_id==*game_filter) {
                for item in &game.mods {
                    if !format!("{} {} {}",game.name,item.name,item.description).to_lowercase().contains(&query) { continue; }
                    matches += 1;
                    egui::Frame::new().fill(SURFACE).corner_radius(crate::ui_helpers::SURFACE_RADIUS).inner_margin(20).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&item.name).size(20.0).strong());
                            ui.label(RichText::new(format!("v{}",item.version)).color(GREEN));
                        });
                        ui.label(RichText::new(&game.name).color(GREEN));
                        ui.label(&item.description);
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
                    ui.add_space(12.0);
                }
            }
            if matches == 0 { empty_panel(ui,"Nothing here yet.","Try another search, or add mods to your game's Mods folder and game.json on GitHub."); }
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
    pub fn new() -> Self {
        let (packs, warnings) = crate::modpacks::load_all();
        let mut groups = crate::modpacks::load_groups();
        for pack in &packs {
            if !pack.group.is_empty() && !groups.contains(&pack.group) {
                groups.push(pack.group.clone());
            }
        }
        Self {
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
                            "{}  /  BepInEx  /  {} mods",
                            pack.game.name,
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
                if ui.button("Export…").clicked() {
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
            ui.add_space(16.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.detail_tab, 0, "Content");
                ui.selectable_value(&mut self.detail_tab, 1, "Pack details");
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
                    .selected_text(["Name A–Z", "Name Z–A", "Most mods"][self.sort as usize])
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sort, 0, "Name A–Z");
                        ui.selectable_value(&mut self.sort, 1, "Name Z–A");
                        ui.selectable_value(&mut self.sort, 2, "Most mods");
                    });
                let games: BTreeMap<u32, String> = self
                    .packs
                    .iter()
                    .map(|p| (p.game.app_id, p.game.name.clone()))
                    .collect();
                egui::ComboBox::from_id_salt("pack_game_filter")
                    .selected_text(
                        games
                            .get(&self.game_filter)
                            .map(String::as_str)
                            .unwrap_or("All games"),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.game_filter, 0, "All games");
                        for (id, name) in games {
                            ui.selectable_value(&mut self.game_filter, id, name);
                        }
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
                                                "{} mods  /  BepInEx",
                                                pack.mods.len()
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
                    egui::ComboBox::from_id_salt("editor_game").width(480.0).selected_text(&pack.game.name).show_ui(ui, |ui| {for game in catalog {ui.selectable_value(&mut id, game.app_id, &game.name);}});
                    if id != pack.game.app_id && let Some(game) = catalog.iter().find(|g| g.app_id == id) {pack.game = crate::modpacks::PackGame {app_id: game.app_id, name: game.name.clone(), folder: game.folder.clone(), framework: "bepinex".into()};pack.repository = source.cloned().unwrap_or_else(empty_source);pack.mods.clear();}
                    ui.horizontal(|ui| {ui.label(RichText::new("FRAMEWORK").small().color(MUTED)); ui.label(RichText::new("BepInEx / Unity").color(GREEN));});
                    ui.strong("Group"); egui::ComboBox::from_id_salt("editor_group").width(480.0).selected_text(if pack.group.is_empty() {"Ungrouped"} else {&pack.group}).show_ui(ui, |ui| {ui.selectable_value(&mut pack.group, String::new(), "Ungrouped");for group in &self.groups {ui.selectable_value(&mut pack.group, group.clone(), group);}});
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
                if let Some(item) = pack.mods.iter_mut().find(|item| item.file == file) {
                    item.enabled = enabled;
                }
                match pack.save() {
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
                    .set_title("Add a local plugin DLL or ZIP")
                    .add_filter("Mods", &["dll", "zip"])
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
    fn discover_updates_only_target_pack_and_rejects_other_repositories() {
        let game = crate::model::bopl();
        let source = Source::from_settings(&crate::model::Settings::load());
        let item = crate::model::ModInfo {
            enabled: false,
            name: "Discovery fixture".into(),
            version: "1.0.0".into(),
            file: "Mods/fixture.zip".into(),
            description: String::new(),
            sha256: "a".repeat(64),
            local_file: String::new(),
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
        for _ in 0..3 {
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
        for _ in 0..3 {
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
