use crate::{
    model::{GameInfo, InstalledGame},
    modpacks::Modpack,
    play_backup,
    play_manifest::{Manifest, Project},
};
use anyhow::{Context, Result};
use eframe::egui;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc::{self, Receiver},
    time::Instant,
};

pub fn manifest(pack: &Modpack, game: Option<&InstalledGame>) -> Manifest {
    let version = game.and_then(crate::steam::installed_version);
    let minecraft = game
        .filter(|g| g.app_id == u32::MAX)
        .and_then(|g| crate::minecraft::play_version(&g.path));
    Manifest {
        game: pack.game.app_id,
        branch: if minecraft.is_some() {
            "instance".into()
        } else {
            version
                .as_ref()
                .map(|v| v.branch.clone())
                .unwrap_or_default()
        },
        build: if let Some((version, _)) = &minecraft {
            version.clone()
        } else {
            version
                .as_ref()
                .map(|v| v.build.clone())
                .unwrap_or_default()
        },
        loader: game.map(|g| g.loader.clone()).unwrap_or_default(),
        manager: env!("CARGO_PKG_VERSION").into(),
        mods: pack
            .mods
            .iter()
            .filter(|m| m.enabled)
            .map(|m| Project {
                name: m.name.clone(),
                version: m.version.clone(),
                sha256: m.sha256.clone(),
                dependencies: m.dependencies.clone(),
            })
            .collect(),
        shared_configs: BTreeMap::new(),
    }
}
pub fn adopt(expected: &Manifest, pack: &Modpack, catalog: &[GameInfo]) -> Result<Modpack> {
    expected.validate().map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        expected.game == pack.game.app_id,
        "This release is for another game"
    );
    anyhow::ensure!(
        expected.shared_configs.is_empty(),
        "Shared config differences need explicit configuration review"
    );
    let mut mods = vec![];
    for item in &expected.mods {
        let matches = |m: &&crate::model::ModInfo| {
            m.name == item.name
                && m.version == item.version
                && m.sha256.eq_ignore_ascii_case(&item.sha256)
        };
        let mut found = pack
            .mods
            .iter()
            .find(matches)
            .or_else(|| {
                catalog
                    .iter()
                    .filter(|g| g.app_id == pack.game.app_id)
                    .flat_map(|g| &g.mods)
                    .find(matches)
            })
            .with_context(|| {
                format!(
                    "Pinned {} {} is not in the connected catalog; retrieve that release first",
                    item.name, item.version
                )
            })?
            .clone();
        found.enabled = true;
        mods.push(found);
    }
    let result = Modpack::create(
        format!("{} candidate", pack.name)
            .chars()
            .take(100)
            .collect(),
        "Isolated candidate; game build and loader changes need separate review".into(),
        &GameInfo {
            app_id: pack.game.app_id,
            name: pack.game.name.clone(),
            folder: pack.game.folder.clone(),
            ..crate::model::bopl()
        },
        pack.repository.clone(),
        mods,
    );
    result.validate()?;
    Ok(result)
}
/// Dependency-closed halves. Copies change no active files and keep original version pins.
pub fn test_half(pack: &Modpack, second: bool) -> Result<Modpack> {
    let enabled: Vec<_> = pack.mods.iter().filter(|m| m.enabled).collect();
    anyhow::ensure!(
        enabled.len() > 1,
        "At least two enabled projects are needed for a split test"
    );
    let mid = enabled.len().div_ceil(2);
    let selected = if second {
        &enabled[mid..]
    } else {
        &enabled[..mid]
    };
    let mut names: BTreeSet<String> = selected.iter().map(|m| m.name.clone()).collect();
    loop {
        let before = names.len();
        for item in &enabled {
            if names.contains(&item.name) {
                for dep in &item.dependencies {
                    anyhow::ensure!(
                        enabled.iter().any(|m| &m.name == dep),
                        "Missing dependency {dep}"
                    );
                    names.insert(dep.clone());
                }
            }
        }
        if before == names.len() {
            break;
        }
    }
    let mut copy=Modpack::create(format!("{} test {}",pack.name,if second{"B"}else{"A"}).chars().take(100).collect(),"Dependency-closed test copy. Apply and test explicitly; it is not evidence of compatibility.".into(),&GameInfo{app_id:pack.game.app_id,name:pack.game.name.clone(),folder:pack.game.folder.clone(),..crate::model::bopl()},pack.repository.clone(),pack.mods.iter().filter(|m|names.contains(&m.name)).cloned().collect());
    for item in &mut copy.mods {
        item.enabled = true;
    }
    copy.validate()?;
    Ok(copy)
}
fn save_candidate(copy: &Modpack, game: Option<&InstalledGame>) -> Result<()> {
    if copy.game.app_id == u32::MAX {
        crate::minecraft::create_play_candidate(
            copy,
            game.context("Minecraft instance is not selected")?,
        )?;
    }
    copy.save()
}
fn request(body: Value, session: String) -> Result<Value> {
    anyhow::ensure!(
        !session.is_empty(),
        "Connect your Canna account in Settings first"
    );
    let response = crate::runtime::client()?
        .post("https://cannamods.vip/api/v1/play/action")
        .bearer_auth(session)
        .json(&body)
        .send()?;
    decode(response)
}
fn decode(response: reqwest::blocking::Response) -> Result<Value> {
    use std::io::Read;
    let status = response.status();
    let mut bytes = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Play response exceeds limits"
    );
    let value: Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        status.is_success(),
        "{}",
        value["error"].as_str().unwrap_or("Play request failed")
    );
    Ok(value)
}
enum Outcome {
    Log(String),
    Online(Value),
    Text(String),
    Pack(Box<Modpack>),
    Recovery(Result<Box<Modpack>, String>, Vec<String>),
}
pub struct Lab {
    pub console_requested: bool,
    pub runtime_progress: Vec<String>,
    page: u32,
    editor: crate::play_config::Editor,
    key: String,
    account: String,
    job: Option<Receiver<Result<Outcome, String>>>,
    pub changed: Option<Modpack>,
    policy: play_backup::Policy,
    folder: String,
    alias: String,
    invite: String,
    room: String,
    room_data: Value,
    channel: String,
    release: String,
    listing: Value,
    note: String,
    preview: bool,
    preview_data: Value,
    outcome: String,
    log: String,
    diagnoses: Vec<crate::play_manifest::Check>,
    status: String,
    snapshots: Vec<play_backup::Snapshot>,
    last_sync: Instant,
    benchmark: String,
    confirm: Option<play_backup::Snapshot>,
    config_fields: Vec<(String, String)>,
    config_paths: Vec<std::path::PathBuf>,
    last_configs: Instant,
}
impl Default for Lab {
    fn default() -> Self {
        let policy = play_backup::Policy::load();
        Self {
            console_requested: false,
            runtime_progress: vec![],
            page: 1,
            editor: Default::default(),
            folder: policy.directory.display().to_string(),
            policy,
            key: String::new(),
            account: String::new(),
            job: None,
            changed: None,
            alias: "Player".into(),
            invite: String::new(),
            room: String::new(),
            room_data: Value::Null,
            channel: String::new(),
            release: String::new(),
            listing: Value::Null,
            note: String::new(),
            preview: false,
            preview_data: Value::Null,
            outcome: "worked".into(),
            log: String::new(),
            diagnoses: vec![],
            status: String::new(),
            snapshots: vec![],
            last_sync: Instant::now(),
            benchmark: "Game FPS and startup: unmeasured".into(),
            confirm: None,
            config_fields: vec![],
            config_paths: vec![],
            last_configs: Instant::now(),
        }
    }
}
impl Lab {
    fn work(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce() -> Result<Outcome> + Send + 'static,
    ) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = work().map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn online(&mut self, ctx: &egui::Context, body: Value) {
        let session = crate::website::session();
        self.work(ctx, move || Ok(Outcome::Online(request(body, session)?)));
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        let session = crate::website::session();
        let channel = self.channel.clone();
        let page = self.page.to_string();
        self.work(ctx, move || {
            let response = crate::runtime::client()?
                .get("https://cannamods.vip/api/v1/play")
                .query(&[("channel", channel), ("page", page)])
                .bearer_auth(session)
                .send()?;
            anyhow::ensure!(
                response.status().is_success(),
                "Connect your account to view Play Lab"
            );
            Ok(Outcome::Online(decode(response)?))
        });
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        pack: &Modpack,
        games: &[InstalledGame],
        catalog: &[GameInfo],
        busy: bool,
    ) {
        self.show_with_options(ui, pack, games, catalog, busy, Default::default());
    }
    pub fn show_with_options(
        &mut self,
        ui: &mut egui::Ui,
        pack: &Modpack,
        games: &[InstalledGame],
        catalog: &[GameInfo],
        busy: bool,
        options: crate::runtime::InstallOptions,
    ) {
        let session = crate::website::session();
        if self.key != pack.id || self.account != session {
            let policy = self.policy.clone();
            *self = Self::default();
            self.policy = policy;
            self.key = pack.id.clone();
            self.account = session;
        }
        if let Some(job) = &self.job {
            if let Ok(result) = job.try_recv() {
                self.job = None;
                match result {
                    Ok(Outcome::Pack(p)) => {
                        self.changed = Some(*p);
                        self.status="Candidate saved. Apply it explicitly to test; the original pack is preserved.".into();
                    }
                    Ok(Outcome::Recovery(result, messages)) => {
                        self.runtime_progress.extend(messages);
                        match result {
                            Ok(pack) => {
                                self.changed = Some(*pack);
                                self.status = "Snapshot restored. Read Console preparation warnings before testing gameplay.".into();
                            }
                            Err(error) => self.status = error,
                        }
                    }
                    Ok(Outcome::Log(text)) => {
                        self.log = text;
                        self.status =
                            "Game log loaded locally; expand Troubleshoot to view it".into();
                    }
                    Ok(Outcome::Text(text)) => {
                        self.status = text;
                        self.snapshots =
                            play_backup::list(&self.policy, pack.game.app_id).unwrap_or_default();
                    }
                    Ok(Outcome::Online(value)) => {
                        if value["members"].is_array() {
                            self.room = value["id"].as_str().unwrap_or_default().into();
                            if let Some(invite) = value["invite"].as_str() {
                                self.invite = invite.into();
                            }
                            self.room_data = value;
                        } else if value["reports"].is_array() {
                            self.listing = value;
                        } else {
                            if let Some(channel) = value["channel"].as_str() {
                                self.channel = channel.into();
                            }
                            if let Some(release) = value["release"].as_str() {
                                self.release = release.into();
                            }
                            if value["closed"] == true || value["left"] == true {
                                self.room.clear();
                                self.invite.clear();
                                self.room_data = Value::Null;
                            }
                            self.status = if value["ticket"].is_string() {
                                format!(
                                    "Support ticket created: {}",
                                    value["ticket"].as_str().unwrap()
                                )
                            } else {
                                "Saved. Refresh community records to see the change.".into()
                            };
                        }
                    }
                    Err(error) => self.status = error,
                }
            } else {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        let game = games.iter().find(|g| g.app_id == pack.game.app_id);
        if self.last_configs.elapsed().as_secs() >= 5 {
            self.last_configs = Instant::now();
            for (index, path) in self.config_paths.iter().enumerate() {
                let value = (|| -> Result<String> {
                    crate::runtime::no_links(path)?;
                    anyhow::ensure!(
                        std::fs::metadata(path)?.len() <= 1024 * 1024,
                        "Config exceeds limits"
                    );
                    use sha2::{Digest, Sha256};
                    Ok(format!("{:x}", Sha256::digest(std::fs::read(path)?)))
                })();
                match value {
                    Ok(hash) => self.config_fields[index].1 = hash,
                    Err(e) => {
                        self.config_fields[index].1 = "0".repeat(64);
                        self.status = format!("Shared config is unavailable: {e}");
                    }
                }
            }
        }
        let mut current = manifest(pack, game);
        for (key, value) in &self.config_fields {
            current.shared_configs.insert(key.clone(), value.clone());
        }
        let active = self.job.is_some() || busy;
        ui.heading("Play Lab");
        ui.label("Private local checks. Nothing is sent until you create/join a lobby, publish a release or submit a previewed report.");
        if ui.button("Export private play manifest…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("canna-play-manifest.json")
                .save_file()
        {
            self.status = match serde_json::to_vec_pretty(&current)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| std::fs::write(path, bytes).map_err(anyhow::Error::from))
            {
                Ok(()) => "Manifest exported without local paths or device information".into(),
                Err(e) => e.to_string(),
            };
        }
        egui::ScrollArea::vertical().id_salt("play-lab").show(ui,|ui|{
            ui.collapsing("Readiness & group lobby",|ui|{
                if game.is_none(){ui.label("Game is not installed; install or locate it before launching.");}
                for check in current.preflight(){ui.label(format!("{}: {}",check.level,check.message));}
                ui.label(format!("Manifest: {}",current.fingerprint().unwrap_or_else(|e|e.into())));
                ui.horizontal_wrapped(|ui|{ui.label("Lobby display name");ui.text_edit_singleline(&mut self.alias);if ui.add_enabled(!active,egui::Button::new("Create invite-only lobby")).clicked(){self.online(ui.ctx(),json!({"action":"create","alias":self.alias,"manifest":current}));}});
                ui.label("Paste a lobby invitation");ui.text_edit_singleline(&mut self.invite);
                if ui.add_enabled(!active,egui::Button::new("Join lobby")).clicked(){if let Some((id,code))=self.invite.split_once(':'){let body=json!({"action":"join","id":id,"code":code,"alias":self.alias,"manifest":current});self.online(ui.ctx(),body);}else{self.status="Paste the complete lobby invitation".into();}}
                if !self.room.is_empty(){ui.horizontal_wrapped(|ui|{if ui.button("Copy lobby invitation").clicked(){ui.ctx().copy_text(self.invite.clone());}
 if ui.add_enabled(!active,egui::Button::new("Update readiness")).clicked(){self.online(ui.ctx(),json!({"action":"update","id":self.room,"manifest":current}));}
 if ui.add_enabled(!active,egui::Button::new(if self.room_data["host"]==true{"Close lobby"}else{"Leave lobby"})).clicked(){self.online(ui.ctx(),json!({"action":if self.room_data["host"]==true{"close"}else{"leave"},"id":self.room}));}});
                    if let Ok(host)=serde_json::from_value::<Manifest>(self.room_data["manifest"].clone()){for check in crate::play_manifest::compare(&host,&current).into_iter().filter(|c|c.level!="matched"){ui.label(check.message);}}
                    if let Some(members)=self.room_data["members"].as_array(){for member in members{ui.strong(format!("{} · {}",member["alias"].as_str().unwrap_or("Player"),if member["ready"]==true{"Matched"}else if member["active"]!=true{"Offline / stale"}else{"Needs checks"}));if let Some(checks)=member["checks"].as_array(){for check in checks.iter().filter(|c|c["level"]!="matched"){ui.label(check["message"].as_str().unwrap_or_default());}}}}
                    if ui.add_enabled(!active,egui::Button::new("Create copy matching host mods")).clicked(){match serde_json::from_value::<Manifest>(self.room_data["manifest"].clone()).map_err(anyhow::Error::from).and_then(|m|adopt(&m,pack,catalog)){Ok(copy)=>match save_candidate(&copy,game){Ok(())=>self.changed=Some(copy),Err(e)=>self.status=e.to_string()},Err(e)=>self.status=e.to_string()}}
                }
                ui.label("Personal settings are excluded. Share selected configuration digests explicitly; their values are never uploaded.");
                if let Some(game)=game && ui.button("Select shared config file…").clicked() && let Some(path)=rfd::FileDialog::new().add_filter("Config",&["cfg"]).pick_file(){let allowed=crate::play_config::scope(game);let result=(||->Result<(String,String)>{anyhow::ensure!(self.config_fields.len()<100,"Select at most 100 configuration files");crate::runtime::no_links(&path)?;anyhow::ensure!(path.canonicalize()?.starts_with(allowed.canonicalize()?),"Choose a file inside this game's config folder");anyhow::ensure!(std::fs::metadata(&path)?.len()<=1024*1024,"Config exceeds limits");use sha2::{Digest,Sha256};Ok((format!("shared-{}",self.config_fields.len()+1),format!("{:x}",Sha256::digest(std::fs::read(&path)?))))})();match result{Ok(pair)=>{self.config_fields.push(pair);self.config_paths.push(path);},Err(e)=>self.status=e.to_string()}}
                if ui.button("Clear shared config selection").clicked(){self.config_fields.clear();self.config_paths.clear();}
            });
            ui.collapsing("Recovery snapshots",|ui|{
                ui.label("Local archive/config snapshots. World saves are never downgraded by recovery.");ui.text_edit_singleline(&mut self.folder);ui.add(egui::DragValue::new(&mut self.policy.budget_gb).range(1..=1000).suffix(" GB budget"));ui.checkbox(&mut self.policy.automatic,"Snapshot the last applied pack before replacing it");
                if ui.button("Save backup settings").clicked(){self.policy.directory=self.folder.clone().into();self.status=match self.policy.save(){Ok(())=>"Backup settings saved".into(),Err(e)=>e.to_string()};}
                if ui.button("Refresh snapshots").clicked(){match play_backup::list(&self.policy,pack.game.app_id){Ok(items)=>self.snapshots=items,Err(e)=>self.status=e.to_string()}}
                if let Some(game)=game && ui.add_enabled(!active,egui::Button::new("Snapshot this pack now")).clicked(){let policy=self.policy.clone();let game=game.clone();let pack=pack.clone();self.work(ui.ctx(),move||{play_backup::capture(&policy,&game,&pack,false)?;Ok(Outcome::Text("Snapshot saved".into()))});}
                for snapshot in self.snapshots.clone(){ui.horizontal_wrapped(|ui|{ui.label(format!("{} · {:.1} MiB{}",snapshot.id,snapshot.bytes as f64/1048576.,if snapshot.working{" · Last working"}else{""}));if ui.add_enabled(!active,egui::Button::new("Mark working")).clicked(){self.status=match play_backup::mark_working(&self.policy,&snapshot){Ok(())=>"Last working snapshot selected".into(),Err(e)=>e.to_string()};}
 if ui.add_enabled(!active&&game.is_some(),egui::Button::new("Restore…")).clicked(){self.confirm=Some(snapshot.clone());}
 if ui.add_enabled(!active&&!snapshot.working,egui::Button::new("Delete snapshot")).clicked(){self.status=match play_backup::remove(&self.policy,&snapshot){Ok(())=>"Snapshot deleted".into(),Err(e)=>e.to_string()};self.snapshots=play_backup::list(&self.policy,pack.game.app_id).unwrap_or_default();}});}
            });
            ui.collapsing("Troubleshoot & test copies",|ui|{
                if let Some(game)=game && ui.add_enabled(!active,egui::Button::new("Inspect local game logs")).clicked(){let game=game.clone();self.work(ui.ctx(),move||{let snapshot=crate::console::collect(&game);let raw=snapshot.files.into_iter().map(|f|f.text).collect::<Vec<_>>().join("\n");let checks=crate::play_manifest::diagnose(&raw);Ok(Outcome::Text(checks.into_iter().map(|c|c.message).collect::<Vec<_>>().join("\n")))});}
                ui.label("Paste a local error log, or load a log file. This stays on your device.");ui.add(egui::TextEdit::multiline(&mut self.log).desired_rows(4).char_limit(256*1024));
                if ui.button("Load log…").clicked() && let Some(path)=rfd::FileDialog::new().pick_file(){use std::io::Read;self.status=match std::fs::File::open(path).and_then(|f|{let mut data=String::new();f.take(256*1024).read_to_string(&mut data)?;self.log=data;Ok(())}){Ok(())=>"Log loaded locally".into(),Err(e)=>e.to_string()};}
                if ui.button("Explain failure patterns").clicked(){self.diagnoses=crate::play_manifest::diagnose(&self.log);}for c in &self.diagnoses{ui.label(&c.message);}
                ui.horizontal_wrapped(|ui|{for (second,label) in [(false,"Create test half A"),(true,"Create test half B")]{if ui.add_enabled(!active,egui::Button::new(label)).clicked(){match test_half(pack,second).and_then(|copy|{save_candidate(&copy,game)?;Ok(copy)}){Ok(copy)=>self.changed=Some(copy),Err(e)=>self.status=e.to_string()}}}});
                ui.label("Test each copy and record the outcome. Required dependencies stay enabled; overlapping halves cannot conclusively identify a culprit.");
            });
            ui.collapsing("Compatibility reports & support",|ui|{
                ui.horizontal(|ui|{ui.selectable_value(&mut self.outcome,"worked".into(),"Worked in a session");ui.selectable_value(&mut self.outcome,"failed".into(),"Failed in a session");});ui.add(egui::TextEdit::multiline(&mut self.note).desired_rows(3).char_limit(2000).hint_text("Describe the test or issue without personal details"));
                if ui.button("Preview report").clicked(){self.note=crate::play_manifest::redact(&self.note);self.preview=true;self.preview_data=json!({"manifest":current,"outcome":self.outcome,"note":self.note});}
                if self.preview{ui.label("Review exactly what will be sent: game/build/loader, enabled mod names, versions, hashes, selected config digests and this note. Logs are excluded.");ui.label(serde_json::to_string_pretty(&self.preview_data).unwrap_or_default());ui.horizontal_wrapped(|ui|{if ui.add_enabled(!active,egui::Button::new("Share anonymous compatibility report")).clicked(){let mut body=self.preview_data.clone();body["action"]=json!("report");self.online(ui.ctx(),body);self.preview=false;}
 if ui.add_enabled(!active,egui::Button::new("Send private support ticket")).clicked(){let mut body=self.preview_data.clone();body["action"]=json!("issue");body.as_object_mut().unwrap().remove("outcome");self.online(ui.ctx(),body);self.preview=false;}});}
                if ui.add_enabled(!active,egui::Button::new("Refresh community reports / releases")).clicked(){self.refresh(ui.ctx());}
                ui.horizontal_wrapped(|ui|{if ui.add_enabled(!active&&self.page>1,egui::Button::new("Previous records")).clicked(){self.page-=1;self.refresh(ui.ctx());}ui.label(format!("Page {}",self.page));if ui.add_enabled(!active&&(self.listing["total"].as_u64().unwrap_or(0).max(self.listing["channel_total"].as_u64().unwrap_or(0))>self.page as u64*50),egui::Button::new("Next records")).clicked(){self.page+=1;self.refresh(ui.ctx());}});if let Some(reports)=self.listing["reports"].as_array(){for report in reports.iter().filter(|r|r["manifest"]["game"]==pack.game.app_id){ui.label(format!("{} · {} · {}",report["outcome"].as_str().unwrap_or("unknown"),report["manifest"]["build"].as_str().unwrap_or("unknown build"),report["note"].as_str().unwrap_or_default()));}}
                ui.label("Reports are member observations, not safety certification. Shared reports omit account names; staff can moderate abusive submissions. Reports expire after 30 days.");
            });
            ui.collapsing("Stable & experimental releases",|ui|{
                ui.label("Publish a manifest without uploading local paths or configs. New releases start experimental; promotion requires your successful session report for that exact manifest.");ui.label("Channel ID (empty creates one)");ui.text_edit_singleline(&mut self.channel);
                if ui.add_enabled(!active,egui::Button::new("Publish experimental manifest")).clicked(){self.online(ui.ctx(),json!({"action":"publish","id":self.channel,"alias":"Game night","manifest":current}));}
                if let Some(channels)=self.listing["channels"].as_array().cloned(){for channel in channels{if ui.button(format!("Open channel: {}",channel["name"].as_str().unwrap_or("Game night"))).clicked(){self.channel=channel["id"].as_str().unwrap_or_default().into();self.refresh(ui.ctx());}}}ui.label("Release ID");ui.text_edit_singleline(&mut self.release);
                if ui.add_enabled(!active,egui::Button::new("Promote tested release to stable")).clicked(){self.online(ui.ctx(),json!({"action":"promote","id":self.release}));}
                if let Some(releases)=self.listing["releases"].as_array().cloned(){for release in releases{ui.horizontal_wrapped(|ui|{ui.label(format!("{} · {}",release["stage"].as_str().unwrap_or_default(),release["id"].as_str().unwrap_or_default()));if ui.button("Create candidate from release").clicked(){let id=release["id"].as_str().unwrap_or_default().to_string();let pack=pack.clone();let catalog=catalog.to_vec();let game=game.cloned();let session=crate::website::session();self.work(ui.ctx(),move||{let value=request(json!({"action":"read-release","id":id}),session)?;let expected:Manifest=serde_json::from_value(value["manifest"].clone())?;let copy=adopt(&expected,&pack,&catalog)?;save_candidate(&copy,game.as_ref())?;Ok(Outcome::Pack(Box::new(copy)))});}});}}
            });
            ui.collapsing("Game tools & low-end testing",|ui|{
                ui.label(if pack.game.app_id==u32::MAX{"Minecraft: clone content into test instances, inspect local configs, and restore content snapshots without touching worlds."}else if crate::model::source_addons(pack.game.app_id).is_some(){"Source: Launch modded uses -insecure and game logging. Add Canna Auto-Hop to the pack for hold-jump; vanilla launch stays separate."}else{"Unity/BepInEx: edit local mod configuration files and use dependency-closed test copies. Game-specific options come from the installed mods."});
                if let Some(game)=game{ui.horizontal_wrapped(|ui|{if ui.button("Open mod configs").clicked(){let folder=if game.app_id==u32::MAX{game.path.join("config")}else if crate::model::source_addons(game.app_id).is_some(){game.path.join(if game.app_id==550{"left4dead2/cfg"}else{"left4dead/cfg"})}else{game.path.join("BepInEx/config")};let _=std::process::Command::new("explorer.exe").arg(folder).spawn();}
 if ui.add_enabled(!active,egui::Button::new(if game.app_id==u32::MAX{"Load Minecraft log"}else{"Open game console"})).clicked(){if game.app_id==u32::MAX{let game=game.clone();self.work(ui.ctx(),move||Ok(Outcome::Log(crate::console::collect(&game).files.into_iter().map(|f|f.text).collect::<Vec<_>>().join("\n"))));}else{self.console_requested=true;}}});}
                if ui.button("Run local manifest benchmark").clicked(){let start=Instant::now();for _ in 0..1000{let _=current.fingerprint();}self.benchmark=format!("1000 manifest checks: {:.2} ms locally. Game FPS and startup: unmeasured",start.elapsed().as_secs_f64()*1000.);}
                if let Some(game)=game{self.editor.show(ui,game,active);}
                if let Some(m)=crate::play_metrics::app(){ui.label(format!("Canna working set: {:.1} MiB; peak {:.1} MiB (local only)",m.current as f64/1048576.,m.peak as f64/1048576.));}
                ui.label(&self.benchmark);ui.label("No hardware fingerprint or benchmark result is uploaded. Smaller test packs help compare changes; this synthetic check does not measure game performance.");
            });
            if !self.status.is_empty(){ui.separator();ui.label(&self.status);}
        });
        if let Some(snapshot) = self.confirm.clone() {
            let mut close = false;
            egui::Modal::new(egui::Id::new("restore-play-snapshot")).show(ui.ctx(),|ui|{ui.heading("Restore this setup?");ui.label("Close the game first. Canna will restore this pack's pinned archives and backed-up mod configuration. World saves are untouched.");ui.horizontal(|ui|{if ui.button("Cancel").clicked(){close=true;}
 if ui.add_enabled(!active,egui::Button::new("Restore snapshot")).clicked(){if let Some(game)=game{let policy=self.policy.clone();let game=game.clone();let token=crate::website::session();self.work(ui.ctx(),move||{let messages=std::cell::RefCell::new(vec![]);let result=play_backup::restore(&policy,&game,&snapshot,&token,options,&|message|messages.borrow_mut().push(message.to_owned())).map(Box::new).map_err(|error|format!("{error:#}"));Ok(Outcome::Recovery(result,messages.into_inner()))});}close=true;}});});
            if close {
                self.confirm = None;
            }
        }
        if !self.room.is_empty() && !active && self.last_sync.elapsed().as_secs() >= 20 {
            self.last_sync = Instant::now();
            self.online(
                ui.ctx(),
                json!({"action":"update","id":self.room,"manifest":current}),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_supported_game_families_have_private_manifest_checks() {
        for g in crate::model::supported_catalog() {
            let p = Modpack::create(
                "Fixture".into(),
                "".into(),
                &g,
                crate::cache::Source {
                    owner: "canna".into(),
                    repository: "server".into(),
                    branch: "main".into(),
                    catalog_folder: "".into(),
                },
                vec![],
            );
            let value = manifest(&p, None);
            assert_eq!(value.game, g.app_id);
            assert!(value.preflight().iter().any(|c| c.level == "unknown"));
            let text = serde_json::to_string(&value).unwrap();
            assert!(!text.contains("path"));
            assert!(!text.contains("device"));
        }
    }
    #[test]
    fn test_halves_keep_dependencies_and_original() {
        let g = crate::model::bopl();
        let base = crate::model::ModInfo {
            provenance: Value::Null,
            enabled: true,
            name: "Library".into(),
            version: "1".into(),
            content_type: String::new(),
            description: String::new(),
            file: "Mods/library.dll".into(),
            sha256: "a".repeat(64),
            local_file: String::new(),
            dependencies: vec![],
        };
        let p = Modpack::create(
            "Original".into(),
            "".into(),
            &g,
            crate::cache::Source {
                owner: "canna".into(),
                repository: "server".into(),
                branch: "main".into(),
                catalog_folder: "".into(),
            },
            vec![
                base.clone(),
                crate::model::ModInfo {
                    name: "Other".into(),
                    file: "Mods/other.dll".into(),
                    dependencies: vec!["Library".into()],
                    ..base
                },
            ],
        );
        let a = test_half(&p, false).unwrap();
        let b = test_half(&p, true).unwrap();
        assert_ne!(a.id, p.id);
        assert_eq!(b.mods.len(), 2);
        assert_eq!(p.mods.len(), 2);
        b.validate().unwrap();
    }
}
