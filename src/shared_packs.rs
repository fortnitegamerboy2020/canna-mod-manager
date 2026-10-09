use crate::modpacks::Modpack;
use anyhow::{Context, Result, ensure};
use eframe::egui;
use serde::Deserialize;
use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver},
    time::Duration,
};
const API: &str = "https://cannamods.vip/api/v1/packs";
#[derive(Clone, Deserialize)]
pub struct Info {
    pub author: String,
    pub revision: u64,
    pub can_update: bool,
    pub ready: bool,
    pub manifest: Modpack,
}
enum Reply {
    Info(Box<Info>),
    Published(Box<Modpack>, Box<Info>),
}
#[derive(Default)]
pub struct Sharing {
    receiver: Option<Receiver<Result<Reply, String>>>,
    info: Option<Info>,
    checked: Option<String>,
    pub status: String,
    review: bool,
}
fn response(mut response: reqwest::blocking::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 2 * 1024 * 1024, "Pack response too large");
    let data: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(
        status.is_success(),
        "{}",
        data["error"].as_str().unwrap_or("Pack request failed")
    );
    Ok(data)
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn publish(mut pack: Modpack, update: bool) -> Result<Reply> {
    pack.validate()?;
    let client = client()?;
    let token = crate::website::session();
    ensure!(!token.is_empty(), "Sign in using Account first");
    for item in &mut pack.mods {
        if !item.local_file.is_empty() {
            let mut data = Vec::new();
            std::fs::File::open(crate::modpacks::local_directory().join(&item.local_file))?
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut data)?;
            ensure!(data.len() <= 32 * 1024 * 1024, "Local mod exceeds 32 MiB");
            use sha2::{Digest, Sha256};
            ensure!(
                format!("{:x}", Sha256::digest(&data)) == item.sha256.to_lowercase(),
                "Local mod checksum mismatch"
            );
            let data = if item.local_file.ends_with(".zip") {
                data
            } else {
                let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
                zip.start_file(&item.local_file, zip::write::SimpleFileOptions::default())?;
                zip.write_all(&data)?;
                zip.finish()?.into_inner()
            };
            let mut url = reqwest::Url::parse("https://cannamods.vip/api/v1/mods")?;
            url.query_pairs_mut()
                .append_pair("app_id", &pack.game.app_id.to_string())
                .append_pair("name", &item.name)
                .append_pair("version", &item.version)
                .append_pair("description", &item.description);
            let archive_hash = format!("{:x}", Sha256::digest(&data));
            let previous = client
                .get(format!("{API}/local-mod/{archive_hash}"))
                .bearer_auth(&token)
                .send()?;
            let uploaded = if previous.status() == reqwest::StatusCode::NOT_FOUND {
                response(
                    client
                        .post(url)
                        .bearer_auth(&token)
                        .header("Content-Type", "application/zip")
                        .body(data)
                        .send()?,
                )?
            } else {
                response(previous)?
            };
            item.file = format!(
                "Mods/{}.zip",
                uploaded["id"].as_str().context("Upload ID missing")?
            );
            item.sha256 = uploaded["sha256"]
                .as_str()
                .context("Upload checksum missing")?
                .into();
            item.local_file.clear();
        }
        item.provenance = serde_json::Value::Null;
    }
    let body = if update {
        serde_json::json!({"expected_revision":pack.shared.as_ref().context("This pack has no shared link")?.revision,"manifest":pack})
    } else {
        serde_json::to_value(&pack)?
    };
    let url = if update {
        format!("{API}/{}", pack.shared.as_ref().unwrap().id)
    } else {
        API.into()
    };
    let info: Info = serde_json::from_value(response(
        client.post(url).bearer_auth(&token).json(&body).send()?,
    )?)?;
    let mut local = info.manifest.clone();
    local.id = pack.id;
    local.group = pack.group;
    local.theme = pack.theme;
    // Imported text configs remain local until server configuration sharing is available.
    local.imported_configs = pack.imported_configs;
    local.auto_update = pack.auto_update;
    local.save()?;
    Ok(Reply::Published(Box::new(local), Box::new(info)))
}
fn fetch(id: &str) -> Result<Reply> {
    let token = crate::website::session();
    ensure!(!token.is_empty(), "Sign in using Account first");
    Ok(Reply::Info(Box::new(serde_json::from_value(response(
        client()?
            .get(format!("{API}/{id}/info"))
            .bearer_auth(token)
            .send()?,
    )?)?)))
}
pub fn updated_pack(current: &Modpack, info: &Info) -> Result<Modpack> {
    let shared = current
        .shared
        .as_ref()
        .context("This pack has no shared link")?;
    let next = info
        .manifest
        .shared
        .as_ref()
        .context("Update has no shared link")?;
    ensure!(
        info.ready
            && shared.id == next.id
            && next.revision == info.revision
            && next.revision > shared.revision,
        "No installable newer revision"
    );
    ensure!(
        current.game.app_id == info.manifest.game.app_id,
        "Update belongs to a different game"
    );
    let mut pack = info.manifest.clone();
    pack.id = current.id.clone();
    pack.group = current.group.clone();
    pack.theme = current.theme;
    pack.validate()?;
    Ok(pack)
}
impl Sharing {
    fn start(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce() -> Result<Reply> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.status = "Contacting Canna server…".into();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work().map_err(|e| e.to_string()));
            ctx.request_repaint();
        });
    }
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Modpack> {
        if self.receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let result = self.receiver.as_ref()?.try_recv().ok()?;
        self.receiver = None;
        match result {
            Ok(Reply::Info(info)) => {
                self.status = if info.ready {
                    format!("Shared revision {} · by {}", info.revision, info.author)
                } else {
                    "Some mods are awaiting approval. The link is saved; installation is disabled until review completes.".into()
                };
                self.info = Some(*info);
                None
            }
            Ok(Reply::Published(pack, info)) => {
                self.status = if info.ready {
                    "Published. Copy the link to share this revision.".into()
                } else {
                    "Link created. Locally uploaded mods require staff approval before installation.".into()
                };
                self.checked = Some(pack.id.clone());
                self.info = Some(*info);
                Some(*pack)
            }
            Err(e) => {
                self.status = e;
                None
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, pack: &Modpack) -> Option<Modpack> {
        if self.checked.as_ref() != Some(&pack.id) && self.receiver.is_none() {
            self.checked = Some(pack.id.clone());
            self.info = None;
            self.status.clear();
            self.review = false;
            if let Some(shared) = &pack.shared {
                let id = shared.id.clone();
                self.start(ui.ctx(), move || fetch(&id));
            }
        }
        let signed_in = !crate::website::session().is_empty();
        if !pack.imported_configs.is_empty() {
            ui.label("Imported settings are included in file exports. Shared links carry mod selections; send the .canna.zip to include these settings.");
        }
        let busy = self.receiver.is_some();
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    signed_in && !busy,
                    egui::Button::new(if pack.shared.is_some() {
                        "Share as new link"
                    } else {
                        "Share with link"
                    }),
                )
                .clicked()
            {
                let pack = pack.clone();
                self.start(ui.ctx(), move || publish(pack, false));
            }
            if let Some(shared) = &pack.shared {
                let url = format!("https://cannamods.vip/packs/{}", shared.id);
                if ui.button("Copy shared link").clicked() {
                    ui.ctx().copy_text(url.clone());
                }
                ui.hyperlink_to("View shared pack", url);
                ui.label(format!("Saved revision {}", shared.revision));
                if ui
                    .add_enabled(signed_in && !busy, egui::Button::new("Check pack updates"))
                    .clicked()
                {
                    let id = shared.id.clone();
                    self.start(ui.ctx(), move || fetch(&id));
                }
                if let Some(info) = &self.info {
                    if info.can_update
                        && info.revision == shared.revision
                        && ui
                            .add_enabled(!busy, egui::Button::new("Publish pack update"))
                            .clicked()
                    {
                        let pack = pack.clone();
                        self.start(ui.ctx(), move || publish(pack, true));
                    } else if info.revision > shared.revision
                        && ui
                            .add_enabled(
                                info.ready && !busy,
                                egui::Button::new("Review pack update"),
                            )
                            .clicked()
                    {
                        self.review = true;
                    }
                }
            }
        });
        if !signed_in {
            ui.label("Sign in using Account to share packs and check updates.");
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        let mut replacement = None;
        if self.review
            && let Some(info) = self.info.clone()
        {
            let modal=egui::Modal::new(egui::Id::new("shared-pack-update")).show(ui.ctx(),|ui| {
                ui.set_max_width((ui.ctx().content_rect().width()-80.0).min(560.0));ui.heading(format!("Update to revision {}?",info.revision));ui.label(&info.manifest.name);ui.label(info.manifest.description.chars().take(240).collect::<String>());
                ui.label("This replaces your saved mod selections, including local edits. The previous manifest is kept in recovery. Apply modpack afterwards to install the new selections.");
                egui::ScrollArea::vertical().max_height((ui.ctx().content_rect().height()-250.0).clamp(80.0,260.0)).show(ui,|ui|{for item in &info.manifest.mods {let old=pack.mods.iter().find(|m|m.file==item.file);ui.label(format!("{} · {}{}",item.name,item.version,if old.is_none(){" · added / changed"}else{""}));}for old in &pack.mods {if !info.manifest.mods.iter().any(|m|m.file==old.file){ui.label(format!("{} · removed / replaced",old.name));}}});
                ui.horizontal(|ui| {
                    if ui.button("Update saved pack").clicked(){self.status=match accept_update(pack,&info) {Ok(next)=>{replacement=Some(next);"Saved the new revision. Use Apply modpack when the game is closed.".into()},Err(e)=>e.to_string()};self.review=false;}
                    if ui.button("Keep my current revision").clicked(){self.review=false;}
                });
            });
            if modal.should_close() {
                self.review = false;
            }
        }
        replacement
    }
}

fn accept_update(current: &Modpack, info: &Info) -> Result<Modpack> {
    let next = updated_pack(current, info)?;
    // Keep local file references in the recovery manifest without publishing them.
    current.save_in(&crate::modpacks::directory().join("recovery"))?;
    next.save()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cache::Source, modpacks::SharedPack};
    #[test]
    fn optional_updates_preserve_local_identity_and_cannot_cross_links_or_games() {
        let mut current = Modpack::create(
            "My copy".into(),
            "Local notes".into(),
            &crate::model::bopl(),
            Source {
                owner: "canna".into(),
                repository: "server".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            },
            vec![],
        );
        current.shared = Some(SharedPack {
            id: "11111111-1111-4111-8111-111111111111".into(),
            revision: 1,
        });
        current.group = "Private group".into();
        let mut incoming = current.clone();
        incoming.id = "22222222-2222-4222-8222-222222222222".into();
        incoming.name = "Creator update".into();
        incoming.shared.as_mut().unwrap().revision = 2;
        let mut info = Info {
            author: "Creator".into(),
            revision: 2,
            can_update: false,
            ready: true,
            manifest: incoming,
        };
        let next = updated_pack(&current, &info).unwrap();
        assert_eq!(current.name, "My copy");
        assert_eq!(current.shared.as_ref().unwrap().revision, 1);
        assert_eq!(next.name, "Creator update");
        assert_eq!(next.id, current.id);
        assert_eq!(next.group, "Private group");
        assert_eq!(next.shared.unwrap().revision, 2);
        let scratch = std::env::temp_dir().join(format!("canna-shared-update-{}", current.id));
        crate::modpacks::with_test_root(scratch.clone(), || {
            let mut edited = current.clone();
            edited.mods.push(serde_json::from_value(serde_json::json!({"name":"Local addition","version":"local","file":format!("Mods/{}.dll","a".repeat(64)),"local_file":format!("{}.dll","a".repeat(64)),"sha256":"a".repeat(64)})).unwrap());
            edited.save().unwrap();
            let accepted = accept_update(&edited, &info).unwrap();
            assert!(accepted.mods.is_empty());
            let saved: Modpack = serde_json::from_slice(
                &std::fs::read(
                    crate::modpacks::directory()
                        .join("recovery")
                        .join(format!("{}.canna.json", current.id)),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(saved.mods[0].name, "Local addition");
            assert_eq!(saved.shared.unwrap().revision, 1);
        });
        assert_eq!(
            scratch.canonicalize().unwrap().parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir_all(scratch).unwrap();
        info.ready = false;
        assert!(updated_pack(&current, &info).is_err());
        info.ready = true;
        info.manifest.shared.as_mut().unwrap().id = "33333333-3333-4333-8333-333333333333".into();
        assert!(updated_pack(&current, &info).is_err());
        info.manifest.shared = current.shared.clone();
        assert!(updated_pack(&current, &info).is_err());
        info.manifest.shared.as_mut().unwrap().revision = 2;
        info.manifest.game.app_id = 550;
        assert!(updated_pack(&current, &info).is_err());
    }
}
