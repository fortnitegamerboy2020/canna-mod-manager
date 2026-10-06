use anyhow::{Context, Result};
use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
#[derive(Clone, Serialize, Deserialize)]
struct Skin {
    name: String,
    file: String,
    #[serde(default)]
    slim: bool,
    #[serde(default)]
    source: String,
    #[serde(default)]
    page: String,
}
pub fn validate_png(bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Skin files are limited to 2 MiB"
    );
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    anyhow::ensure!(
        reader.format() == Some(image::ImageFormat::Png),
        "Choose a Minecraft PNG skin"
    );
    anyhow::ensure!(
        matches!(reader.into_dimensions()?, (64, 64) | (64, 32)),
        "Minecraft skins must be 64 × 64 or 64 × 32 pixels"
    );
    Ok(())
}
fn folder() -> PathBuf {
    crate::modpacks::directory().parent().unwrap().join("skins")
}
fn load() -> Vec<Skin> {
    std::fs::read(folder().join("skins.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn import(path: &Path) -> Result<Skin> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Skin files are limited to 2 MiB"
    );
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()?;
    anyhow::ensure!(
        reader.format() == Some(image::ImageFormat::Png),
        "Choose a Minecraft PNG skin"
    );
    let dimensions = reader.into_dimensions()?;
    anyhow::ensure!(
        matches!(dimensions, (64, 64) | (64, 32)),
        "Minecraft skins must be 64 × 64 or 64 × 32 pixels"
    );
    let image = image::load_from_memory(&bytes)?;
    let mut encoded = std::io::Cursor::new(Vec::new());
    image.write_to(&mut encoded, image::ImageFormat::Png)?;
    let file = format!("{:x}.png", Sha256::digest(encoded.get_ref()));
    std::fs::create_dir_all(folder())?;
    std::fs::write(folder().join(&file), encoded.into_inner())?;
    let skin = Skin {
        name: path
            .file_stem()
            .context("Invalid skin name")?
            .to_string_lossy()
            .into_owned(),
        file,
        slim: false,
        source: "Local file".into(),
        page: String::new(),
    };
    let mut skins = load();
    skins.retain(|s| s.file != skin.file);
    skins.push(skin.clone());
    std::fs::write(
        folder().join("skins.json"),
        serde_json::to_vec_pretty(&skins)?,
    )?;
    Ok(skin)
}
#[derive(Default)]
pub struct Skins {
    pub open: bool,
    selected: Option<Skin>,
    texture: Option<egui::TextureHandle>,
    status: String,
    applying: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    search: Option<std::sync::mpsc::Receiver<crate::skin_catalog::SearchResult>>,
    query: String,
    source: String,
    catalog: Vec<crate::skin_catalog::SkinResult>,
    previews: Vec<egui::TextureHandle>,
    browser: bool,
    page: usize,
    more: bool,
    append: bool,
}
impl Skins {
    pub fn busy(&self) -> bool {
        self.applying.is_some() || self.search.is_some()
    }
    pub fn search_for(&mut self, query: String) {
        if self.search.is_some() {
            return;
        }
        self.query = query;
        self.page = 0;
        self.more = true;
        self.catalog.clear();
        self.previews.clear();
        self.next_page();
    }
    fn next_page(&mut self) {
        if self.search.is_some() || !self.more {
            return;
        }
        self.page += 1;
        self.append = self.page > 1;
        self.browser = true;
        let query = self.query.clone();
        let source = self.source.clone();
        let page = self.page;
        let (tx, rx) = std::sync::mpsc::channel();
        self.search = Some(rx);
        self.status = "Loading skins…".into();
        std::thread::spawn(move || {
            let _ = tx.send(crate::skin_catalog::search_page(&query, &source, page));
        });
    }
    fn save_result(&mut self, index: usize, ctx: &egui::Context) -> Result<()> {
        let result = &self.catalog[index];
        let temporary = folder()
            .join("search-cache")
            .join(format!("{:x}.png", Sha256::digest(&result.bytes)));
        std::fs::create_dir_all(temporary.parent().unwrap())?;
        std::fs::write(&temporary, &result.bytes)?;
        let mut skin = import(&temporary)?;
        skin.name = result.title.clone();
        skin.source = result.source.clone();
        skin.page = result.page.clone();
        let mut items = load();
        if let Some(item) = items.iter_mut().find(|item| item.file == skin.file) {
            *item = skin.clone();
        }
        std::fs::write(
            folder().join("skins.json"),
            serde_json::to_vec_pretty(&items)?,
        )?;
        self.select(ctx, skin)?;
        self.browser = false;
        Ok(())
    }
    fn select(&mut self, ctx: &egui::Context, skin: Skin) -> Result<()> {
        let bytes = std::fs::read(folder().join(&skin.file))?;
        let image = image::load_from_memory(&bytes)?.to_rgba8();
        self.texture = Some(ctx.load_texture(
            "skin-preview",
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            egui::TextureOptions::NEAREST,
        ));
        self.selected = Some(skin);
        Ok(())
    }
    pub fn open_browser(&mut self) {
        self.open = true;
        self.browser = true;
        if self.catalog.is_empty() && self.search.is_none() {
            self.search_for(String::new());
        }
    }
    pub fn update(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.applying {
            if let Ok(result) = receiver.try_recv() {
                self.status = match result {
                    Ok(()) => "Skin applied to your Minecraft account.".into(),
                    Err(error) => error,
                };
                self.applying = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
        if let Some(rx) = &self.search {
            if let Ok(result) = rx.try_recv() {
                if !self.append {
                    self.previews.clear();
                    self.catalog.clear();
                }
                let old_count = self.catalog.len();
                self.more = result.more;
                for item in result.skins {
                    if self.catalog.iter().any(|old| old.bytes == item.bytes) {
                        continue;
                    }
                    if let Ok(image) = image::load_from_memory(&item.bytes) {
                        let image = image.to_rgba8();
                        self.previews.push(ctx.load_texture(
                            format!("skin-search-{}", self.catalog.len()),
                            egui::ColorImage::from_rgba_unmultiplied(
                                [image.width() as usize, image.height() as usize],
                                image.as_raw(),
                            ),
                            egui::TextureOptions::NEAREST,
                        ));
                        self.catalog.push(item);
                    }
                }
                if self.catalog.len() == old_count {
                    self.more = false;
                }
                self.status = result.messages.join(" · ");
                self.search = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.heading("Minecraft skins");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.browser, true, "Discover skins");
            ui.selectable_value(&mut self.browser, false, "Your saved skins");
        });
        ui.add_space(12.0);
        if self.browser {
            let searching = self.search.is_some();
            if ui.button("Skins home").clicked() && !searching {
                self.search_for(String::new());
            }
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search skins: robot, knight, cat…")
                        .desired_width(330.0),
                );
                egui::ComboBox::from_id_salt("skin-source")
                    .selected_text(if self.source.is_empty() {
                        "All sources"
                    } else {
                        &self.source
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.source, String::new(), "All sources");
                        for source in ["MinecraftSkins.net", "SkinsMC", "Skindex"] {
                            ui.selectable_value(&mut self.source, source.into(), source);
                        }
                    });
                if ui
                    .add_enabled(
                        !searching && !self.query.trim().is_empty(),
                        egui::Button::new("Search"),
                    )
                    .clicked()
                    || (!searching
                        && field.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    self.search_for(self.query.clone());
                }
            });
            ui.label(&self.status);
            if searching {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Searching sites and loading skin previews…");
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Open source search:");
                for source in ["MinecraftSkins.net", "SkinsMC", "Skindex"] {
                    if ui.link(source).clicked()
                        && let Ok(url) = crate::skin_catalog::search_url(source, &self.query)
                    {
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                }
            });
            ui.separator();
            let mut save = None;
            egui::ScrollArea::vertical()
                .id_salt(("skin-results", self.query.clone(), self.source.clone()))
                .show(ui, |ui| {
                    egui::Grid::new("skin-results")
                        .num_columns(4)
                        .spacing(egui::vec2(14.0, 14.0))
                        .show(ui, |ui| {
                            for (index, item) in self.catalog.iter().enumerate() {
                                egui::Frame::group(ui.style())
                                    .inner_margin(12)
                                    .show(ui, |ui| {
                                        ui.set_width(175.0);
                                        ui.vertical(|ui| {
                                            paint_skin(
                                                ui,
                                                &self.previews[index],
                                                false,
                                                egui::vec2(150.0, 160.0),
                                            );
                                            ui.label(egui::RichText::new(&item.title).strong());
                                            ui.label(egui::RichText::new(&item.source).small());
                                            ui.horizontal(|ui| {
                                                if ui.button("Save skin").clicked() {
                                                    save = Some(index);
                                                }
                                                if ui.link("Original").clicked() {
                                                    ctx.open_url(egui::OpenUrl::new_tab(
                                                        &item.page,
                                                    ));
                                                }
                                            });
                                        });
                                    });
                                if index % 4 == 3 {
                                    ui.end_row();
                                }
                            }
                        });
                    if self.catalog.is_empty() && !searching {
                        ui.label("No skins found. Try another search.");
                    }
                    let end = ui.label(if self.more {
                        "Scroll for more skins"
                    } else {
                        "End of results"
                    });
                    if self.more && !searching && ui.is_rect_visible(end.rect) {
                        self.next_page();
                    }
                });
            if let Some(index) = save {
                self.status = match self.save_result(index, &ctx) {
                    Ok(()) => {
                        "Skin saved. Choose classic or slim, then apply it after Microsoft sign-in."
                            .into()
                    }
                    Err(e) => e.to_string(),
                };
            }
            return;
        }
        ui.heading("Your Minecraft skins");
        ui.label("Import a PNG skin and choose its classic or slim model. Sign in through Minecraft to apply it to your account.");
        if ui.button("+ Import skin").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Minecraft skin", &["png"])
                .pick_file()
        {
            let result = import(&path).and_then(|skin| self.select(&ctx, skin));
            self.status = match result {
                Ok(()) => "Skin imported.".into(),
                Err(e) => e.to_string(),
            };
        }
        ui.label(&self.status);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(200.0);
                for skin in load() {
                    if ui
                        .selectable_label(
                            self.selected.as_ref().is_some_and(|s| s.file == skin.file),
                            &skin.name,
                        )
                        .clicked()
                        && let Err(e) = self.select(&ctx, skin)
                    {
                        self.status = e.to_string();
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                if let (Some(skin), Some(texture)) = (&mut self.selected, &self.texture) {
                    ui.strong(&skin.name);
                    let old = skin.slim;
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut skin.slim, false, "Classic");
                        ui.selectable_value(&mut skin.slim, true, "Slim");
                    });
                    if !skin.source.is_empty() {
                        ui.label(&skin.source);
                    }
                    if !skin.page.is_empty() && ui.link("Original skin page").clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab(&skin.page));
                    }
                    if ui
                        .add_enabled(
                            self.applying.is_none() && crate::minecraft_auth::account().is_ok(),
                            egui::Button::new("Apply to Minecraft account"),
                        )
                        .clicked()
                    {
                        let path = folder().join(&skin.file);
                        let slim = skin.slim;
                        let (sender, receiver) = std::sync::mpsc::channel();
                        self.applying = Some(receiver);
                        self.status = "Applying skin…".into();
                        std::thread::spawn(move || {
                            let result = super::minecraft_auth::apply_skin(
                                super::minecraft::CLIENT_ID,
                                &path,
                                slim,
                            )
                            .map_err(|e| e.to_string());
                            let _ = sender.send(result);
                        });
                    }
                    if skin.slim != old {
                        let mut items = load();
                        if let Some(s) = items.iter_mut().find(|s| s.file == skin.file) {
                            s.slim = skin.slim;
                        }
                        let _ = std::fs::write(
                            folder().join("skins.json"),
                            serde_json::to_vec_pretty(&items).unwrap(),
                        );
                    }
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(180.0, 300.0), egui::Sense::hover());
                    let scale = 8.0;
                    let center = rect.center_top() + egui::vec2(0.0, 10.0);
                    let dims = texture.size_vec2();
                    let part = |offset: egui::Vec2, size: egui::Vec2, uv: egui::Pos2| {
                        let target =
                            egui::Rect::from_min_size(center + offset * scale, size * scale);
                        let crop = egui::Rect::from_min_max(
                            egui::pos2(uv.x / dims.x, uv.y / dims.y),
                            egui::pos2((uv.x + size.x) / dims.x, (uv.y + size.y) / dims.y),
                        );
                        ui.painter()
                            .image(texture.id(), target, crop, egui::Color32::WHITE);
                    };
                    part(
                        egui::vec2(-4.0, 0.0),
                        egui::vec2(8.0, 8.0),
                        egui::pos2(8.0, 8.0),
                    );
                    part(
                        egui::vec2(-4.0, 8.0),
                        egui::vec2(8.0, 12.0),
                        egui::pos2(20.0, 20.0),
                    );
                    let arm = if skin.slim { 3.0 } else { 4.0 };
                    part(
                        egui::vec2(-4.0 - arm, 8.0),
                        egui::vec2(arm, 12.0),
                        egui::pos2(44.0, 20.0),
                    );
                    part(
                        egui::vec2(4.0, 8.0),
                        egui::vec2(arm, 12.0),
                        if dims.y == 64.0 {
                            egui::pos2(36.0, 52.0)
                        } else {
                            egui::pos2(44.0, 20.0)
                        },
                    );
                    part(
                        egui::vec2(-4.0, 20.0),
                        egui::vec2(4.0, 12.0),
                        egui::pos2(4.0, 20.0),
                    );
                    part(
                        egui::vec2(0.0, 20.0),
                        egui::vec2(4.0, 12.0),
                        if dims.y == 64.0 {
                            egui::pos2(20.0, 52.0)
                        } else {
                            egui::pos2(4.0, 20.0)
                        },
                    );
                    if ui.button("Export skin PNG").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_file_name(format!("{}.png", skin.name))
                            .save_file()
                    {
                        self.status = match std::fs::copy(folder().join(&skin.file), path) {
                            Ok(_) => "Skin exported.".into(),
                            Err(e) => e.to_string(),
                        };
                    }
                } else {
                    ui.label("Select or import a skin to preview it.");
                }
            });
        });
    }
}
fn paint_skin(ui: &mut egui::Ui, texture: &egui::TextureHandle, slim: bool, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let scale = (size.y / 33.0).min(size.x / 18.0);
    let center = rect.center_top() + egui::vec2(0.0, 4.0);
    let dims = texture.size_vec2();
    let part = |offset: egui::Vec2, size: egui::Vec2, uv: egui::Pos2| {
        let target = egui::Rect::from_min_size(center + offset * scale, size * scale);
        let crop = egui::Rect::from_min_max(
            egui::pos2(uv.x / dims.x, uv.y / dims.y),
            egui::pos2((uv.x + size.x) / dims.x, (uv.y + size.y) / dims.y),
        );
        ui.painter()
            .image(texture.id(), target, crop, egui::Color32::WHITE);
    };
    part(
        egui::vec2(-4.0, 0.0),
        egui::vec2(8.0, 8.0),
        egui::pos2(8.0, 8.0),
    );
    part(
        egui::vec2(-4.0, 8.0),
        egui::vec2(8.0, 12.0),
        egui::pos2(20.0, 20.0),
    );
    let arm = if slim { 3.0 } else { 4.0 };
    part(
        egui::vec2(-4.0 - arm, 8.0),
        egui::vec2(arm, 12.0),
        egui::pos2(44.0, 20.0),
    );
    part(
        egui::vec2(4.0, 8.0),
        egui::vec2(arm, 12.0),
        if dims.y == 64.0 {
            egui::pos2(36.0, 52.0)
        } else {
            egui::pos2(44.0, 20.0)
        },
    );
    part(
        egui::vec2(-4.0, 20.0),
        egui::vec2(4.0, 12.0),
        egui::pos2(4.0, 20.0),
    );
    part(
        egui::vec2(0.0, 20.0),
        egui::vec2(4.0, 12.0),
        if dims.y == 64.0 {
            egui::pos2(20.0, 52.0)
        } else {
            egui::pos2(4.0, 20.0)
        },
    );
    part(
        egui::vec2(-4.0, 0.0),
        egui::vec2(8.0, 8.0),
        egui::pos2(40.0, 8.0),
    );
}
