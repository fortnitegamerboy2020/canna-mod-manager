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
}
impl Skins {
    pub fn busy(&self) -> bool {
        self.applying.is_some()
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
    pub fn ui(&mut self, ctx: &egui::Context) {
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
        let mut open = self.open;
        egui::Window::new("Skins").open(&mut open).default_width(640.0).show(ctx,|ui| {
            ui.heading("Your Minecraft skins");ui.label("Import a PNG skin and choose its classic or slim model. Sign in through Minecraft to apply it to your account.");
            if ui.button("+ Import skin").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Minecraft skin",&["png"]).pick_file() {
                let result=import(&path).and_then(|skin|self.select(ctx,skin));self.status=match result {Ok(())=>"Skin imported.".into(),Err(e)=>e.to_string()};
            }
            ui.label(&self.status);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {ui.set_width(200.0);for skin in load() {if ui.selectable_label(self.selected.as_ref().is_some_and(|s|s.file==skin.file),&skin.name).clicked() && let Err(e)=self.select(ctx,skin) {self.status=e.to_string();}}});
                ui.separator();ui.vertical(|ui| {
                    if let (Some(skin),Some(texture))=(&mut self.selected,&self.texture) {
                        ui.strong(&skin.name);let old=skin.slim;ui.horizontal(|ui| {ui.selectable_value(&mut skin.slim,false,"Classic");ui.selectable_value(&mut skin.slim,true,"Slim");});
                        if ui.add_enabled(self.applying.is_none() && crate::minecraft_auth::account().is_ok(), egui::Button::new("Apply to Minecraft account")).clicked() {
                            let path = folder().join(&skin.file); let slim = skin.slim;
                            let (sender, receiver) = std::sync::mpsc::channel(); self.applying = Some(receiver);
                            self.status = "Applying skin…".into();
                            std::thread::spawn(move || {
                                let result = std::fs::read_to_string(super::minecraft::client_id_path()).map_err(|e| e.to_string()).and_then(|id| super::minecraft_auth::apply_skin(id.trim(), &path, slim).map_err(|e| e.to_string()));
                                let _ = sender.send(result);
                            });
                        }
                        if skin.slim!=old {let mut items=load();if let Some(s)=items.iter_mut().find(|s|s.file==skin.file) {s.slim=skin.slim;}let _=std::fs::write(folder().join("skins.json"),serde_json::to_vec_pretty(&items).unwrap());}
                        let (rect,_)=ui.allocate_exact_size(egui::vec2(180.0,300.0),egui::Sense::hover());let scale=8.0;let center=rect.center_top()+egui::vec2(0.0,10.0);
                        let dims=texture.size_vec2();let part=|offset:egui::Vec2,size:egui::Vec2,uv:egui::Pos2| {let target=egui::Rect::from_min_size(center+offset*scale,size*scale);let crop=egui::Rect::from_min_max(egui::pos2(uv.x/dims.x,uv.y/dims.y),egui::pos2((uv.x+size.x)/dims.x,(uv.y+size.y)/dims.y));ui.painter().image(texture.id(),target,crop,egui::Color32::WHITE);};
                        part(egui::vec2(-4.0,0.0),egui::vec2(8.0,8.0),egui::pos2(8.0,8.0));part(egui::vec2(-4.0,8.0),egui::vec2(8.0,12.0),egui::pos2(20.0,20.0));
                        let arm=if skin.slim {3.0} else {4.0};part(egui::vec2(-4.0-arm,8.0),egui::vec2(arm,12.0),egui::pos2(44.0,20.0));part(egui::vec2(4.0,8.0),egui::vec2(arm,12.0),if dims.y==64.0 {egui::pos2(36.0,52.0)} else {egui::pos2(44.0,20.0)});
                        part(egui::vec2(-4.0,20.0),egui::vec2(4.0,12.0),egui::pos2(4.0,20.0));part(egui::vec2(0.0,20.0),egui::vec2(4.0,12.0),if dims.y==64.0 {egui::pos2(20.0,52.0)} else {egui::pos2(4.0,20.0)});
                        if ui.button("Export skin PNG").clicked() && let Some(path)=rfd::FileDialog::new().set_file_name(format!("{}.png",skin.name)).save_file() {self.status=match std::fs::copy(folder().join(&skin.file),path) {Ok(_)=>"Skin exported.".into(),Err(e)=>e.to_string()};}
                    } else {ui.label("Select or import a skin to preview it.");}
                });
            });
        });
        self.open = open;
    }
}
