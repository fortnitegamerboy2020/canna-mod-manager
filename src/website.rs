//! Website downloads use a two-minute, one-file capability, never a browser session.
use anyhow::{Context, Result, bail};
use eframe::egui;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};
const API: &str = "https://cannamods.vip/api/v1/download-tickets";
const ADDRESS: &str = "127.0.0.1:48736";
#[derive(Clone, Deserialize, Serialize)]
pub struct Download {
    pub name: String,
    pub version: String,
    pub app_id: u32,
    pub description: String,
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub pack_ids: Vec<String>,
    #[serde(default)]
    pub framework: bool,
}
#[derive(Deserialize)]
struct Claim {
    kind: String,
    filename: String,
    sha256: String,
    size: u64,
    receipt: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    app_id: u32,
    #[serde(default)]
    description: String,
}
pub fn session() -> String {
    crate::credentials::load("canna-session")
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}
pub fn disconnect() {
    let token = session();
    crate::credentials::remove("canna-session");
    if !token.is_empty() {
        std::thread::spawn(move || {
            let _ = reqwest::blocking::Client::new()
                .post("https://cannamods.vip/api/v1/logout")
                .bearer_auth(token)
                .send();
        });
    }
}
pub fn parse_uri(raw: &str) -> Result<String> {
    if let Some(t) = raw.strip_prefix("canna://connect/") {
        anyhow::ensure!(
            t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid connection ticket"
        );
        return Ok(format!("connect:{t}"));
    }
    let t = raw
        .strip_prefix("canna://download/")
        .context("Unsupported Canna link")?;
    anyhow::ensure!(
        t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid download ticket"
    );
    Ok(t.into())
}
fn root() -> PathBuf {
    crate::modpacks::directory()
        .parent()
        .unwrap()
        .join("downloads")
}
fn records() -> Vec<Download> {
    std::fs::read(root().join("library.json"))
        .or_else(|_| {
            std::fs::read(
                root()
                    .parent()
                    .unwrap()
                    .join("website-downloads/library.json"),
            )
        })
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
static RECORD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn save_records(items: &[Download]) -> Result<()> {
    std::fs::create_dir_all(root())?;
    let path = root().join("library.json");
    let temporary = path.with_extension("pending");
    std::fs::write(&temporary, serde_json::to_vec_pretty(items)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
fn record(mut item: Download) -> Result<()> {
    let _guard = RECORD_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Download history is unavailable"))?;
    let mut items = records();
    if let Some(existing) = items.iter_mut().find(|m| {
        m.sha256 == item.sha256 && m.app_id == item.app_id && m.framework == item.framework
    }) {
        item.pack_ids.extend(existing.pack_ids.clone());
        item.pack_ids.sort();
        item.pack_ids.dedup();
        *existing = item;
    } else {
        items.push(item);
    }
    save_records(&items)
}
pub fn remember_mod(
    pack: &crate::modpacks::Modpack,
    item: &crate::model::ModInfo,
    data: &[u8],
    framework: bool,
) -> Result<()> {
    let hash = format!("{:x}", Sha256::digest(data));
    let ext = if item.file.to_lowercase().ends_with(".dll") {
        "dll"
    } else if item.file.to_lowercase().ends_with(".vpk") {
        "vpk"
    } else {
        "zip"
    };
    std::fs::create_dir_all(root())?;
    let path = root().join(format!("{hash}.{ext}"));
    std::fs::write(&path, data)?;
    record(Download {
        name: item.name.clone(),
        version: item.version.clone(),
        app_id: pack.game.app_id,
        description: item.description.clone(),
        path,
        sha256: hash,
        source: if item.local_file.is_empty() {
            "Canna server"
        } else {
            "Local file"
        }
        .into(),
        pack_ids: vec![pack.id.clone()],
        framework,
    })
}
fn belongs(item: &Download, pack: &crate::modpacks::Modpack) -> bool {
    item.app_id == pack.game.app_id
        && (item.pack_ids.contains(&pack.id)
            || pack
                .mods
                .iter()
                .any(|m| !m.sha256.is_empty() && m.sha256.eq_ignore_ascii_case(&item.sha256)))
}
fn attach(item: &Download, pack: &mut crate::modpacks::Modpack) -> Result<()> {
    anyhow::ensure!(
        item.app_id == pack.game.app_id && !item.framework,
        "Choose a modpack for this mod's game"
    );
    let bytes = std::fs::read(&item.path)?;
    anyhow::ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == item.sha256,
        "Downloaded file changed or is corrupted"
    );
    let mut m = crate::modpacks::add_local(&item.path)?;
    m.name = item.name.clone();
    m.version = item.version.clone();
    m.description = item.description.clone();
    pack.mods.retain(|old| old.name != m.name);
    pack.mods.push(m);
    pack.save()?;
    let mut item = item.clone();
    item.pack_ids.push(pack.id.clone());
    record(item)
}
fn post(
    client: &reqwest::blocking::Client,
    path: &str,
    input: serde_json::Value,
) -> Result<reqwest::blocking::Response> {
    let r = client
        .post(format!("{API}/{path}"))
        .header("Content-Type", "application/json")
        .body(input.to_string())
        .send()?;
    if !r.status().is_success() {
        bail!(
            "Website download failed ({}). Return to the website and try again.",
            r.status()
        );
    }
    Ok(r)
}
pub(crate) fn receive_ticket(ticket: &str) -> Result<String> {
    ensure_ticket(ticket)?;
    fetch(ticket)
}
fn ensure_ticket(ticket: &str) -> Result<()> {
    anyhow::ensure!(
        ticket.len() == 64 && ticket.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid download ticket"
    );
    Ok(())
}
fn fetch(ticket: &str) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .build()?;
    if let Some(ticket) = ticket.strip_prefix("connect:") {
        let response = client
            .post("https://cannamods.vip/api/v1/desktop/claim")
            .header("Content-Type", "application/json")
            .body(serde_json::json!({"ticket":ticket}).to_string())
            .send()?
            .error_for_status()?;
        let mut bytes = Vec::new();
        response.take(4096).read_to_end(&mut bytes)?;
        let info: serde_json::Value = serde_json::from_slice(&bytes)?;
        let raw = info["token"].as_str().context("Missing account session")?;
        anyhow::ensure!(
            raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid account session"
        );
        crate::credentials::save("canna-session", raw.as_bytes())?;
        return Ok("Canna account connected. Syncing server library…".into());
    }
    let mut bytes = Vec::new();
    post(&client, "claim", serde_json::json!({"ticket":ticket}))?
        .take(64 * 1024)
        .read_to_end(&mut bytes)?;
    let claim: Claim = serde_json::from_slice(&bytes)?;
    let result = receive(&client, &claim);
    let _ = post(
        &client,
        "complete",
        serde_json::json!({"receipt":claim.receipt,"success":result.is_ok()}),
    );
    result
}
fn receive(client: &reqwest::blocking::Client, claim: &Claim) -> Result<String> {
    anyhow::ensure!(
        claim.size <= 128 * 1024 * 1024
            && claim.sha256.len() == 64
            && claim.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid download metadata"
    );
    let mut data = Vec::new();
    post(
        client,
        "transfer",
        serde_json::json!({"receipt":claim.receipt}),
    )?
    .take(claim.size + 1)
    .read_to_end(&mut data)?;
    anyhow::ensure!(
        data.len() as u64 == claim.size && format!("{:x}", Sha256::digest(&data)) == claim.sha256,
        "Download was interrupted or its checksum did not match"
    );
    std::fs::create_dir_all(root())?;
    if claim.kind == "packs" {
        let path = root().join(format!("{}.canna.json", claim.sha256));
        std::fs::write(&path, data)?;
        let pack = crate::modpacks::Modpack::import(&path)?;
        return Ok(format!("Imported modpack: {}", pack.name));
    }
    anyhow::ensure!(
        claim.kind == "mods",
        "This download is not a Steam game mod"
    );
    let ext = std::path::Path::new(&claim.filename)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or_default();
    anyhow::ensure!(
        matches!(ext, "zip" | "dll" | "jar"),
        "Unsupported mod file type"
    );
    let path = root().join(format!("{}.{ext}", claim.sha256));
    std::fs::write(&path, data)?;
    record(Download {
        name: claim.name.clone(),
        version: claim.version.clone(),
        app_id: claim.app_id,
        description: claim.description.clone(),
        path,
        sha256: claim.sha256.clone(),
        source: "Community library".into(),
        pack_ids: Vec::new(),
        framework: false,
    })?;
    Ok(format!(
        "Downloaded {} {}. Choose a matching modpack below.",
        claim.name, claim.version
    ))
}
#[cfg(windows)]
pub fn register_protocol() -> Result<()> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let exe = std::env::current_exe()?;
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey("Software\\Classes\\canna")?;
    key.set_value("", &"URL:Canna Mod Manager")?;
    key.set_value("URL Protocol", &"")?;
    let (command, _) = key.create_subkey("shell\\open\\command")?;
    command.set_value("", &format!("\"{}\" \"%1\"", exe.display()))?;
    Ok(())
}
#[cfg(not(windows))]
pub fn register_protocol() -> Result<()> {
    Ok(())
}
/// Forward URI invocations to an existing window; only opaque tickets cross the socket.
pub fn instance(ticket: Option<String>) -> Option<Receiver<String>> {
    match TcpListener::bind(ADDRESS) {
        Ok(listener) => {
            let (tx, rx) = mpsc::channel();
            if let Some(t) = ticket {
                let _ = tx.send(t);
            }
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    if stream.write_all(b"CANNA/1\n").is_err() {
                        continue;
                    }
                    let mut bytes = Vec::new();
                    if stream.take(80).read_to_end(&mut bytes).is_ok()
                        && let Ok(t) = String::from_utf8(bytes)
                        && t.strip_prefix("connect:").unwrap_or(&t).len() == 64
                        && t.strip_prefix("connect:")
                            .unwrap_or(&t)
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit())
                    {
                        let _ = tx.send(t);
                    }
                }
            });
            Some(rx)
        }
        Err(_) => {
            if let Ok(mut stream) =
                TcpStream::connect_timeout(&ADDRESS.parse().unwrap(), Duration::from_secs(1))
            {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut greeting = [0; 8];
                if stream.read_exact(&mut greeting).is_ok() && &greeting == b"CANNA/1\n" {
                    if let Some(t) = ticket {
                        let _ = stream.write_all(t.as_bytes());
                    }
                    return None;
                }
            }
            let (tx, rx) = mpsc::channel();
            if let Some(t) = ticket {
                let _ = tx.send(t);
            }
            Some(rx)
        }
    }
}
pub struct Website {
    pub discover_requested: bool,
    pairing: Option<Receiver<PairEvent>>,
    heartbeat: Option<Receiver<(String, bool)>>,
    last_heartbeat: std::time::Instant,
    pair_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub account_status: String,
    connection_prompt: Option<(String, String)>,
    tickets: Option<Receiver<String>>,
    pending: std::collections::VecDeque<String>,
    result: Option<Receiver<Result<String>>>,
    pub open: bool,
    status: String,
    items: Vec<Download>,
    target: String,
    destinations: std::collections::BTreeMap<String, String>,
}
impl Default for Website {
    fn default() -> Self {
        Self {
            pairing: None,
            heartbeat: None,
            last_heartbeat: std::time::Instant::now() - Duration::from_secs(61),
            pair_cancel: Default::default(),
            account_status: String::new(),
            connection_prompt: None,
            tickets: None,
            pending: Default::default(),
            result: None,
            discover_requested: false,
            open: std::env::args().any(|a| a == "--downloads"),
            status: String::new(),
            items: records(),
            target: String::new(),
            destinations: Default::default(),
        }
    }
}
impl Website {
    pub fn refresh_downloads(&mut self) {
        self.items = records();
    }
    pub fn busy(&self) -> bool {
        self.result.is_some() || !self.pending.is_empty() || self.pairing.is_some()
    }
    pub fn connecting(&self) -> bool {
        self.pairing.is_some()
    }
    pub fn connection_prompt(&self) -> Option<(&str, &str)> {
        self.connection_prompt
            .as_ref()
            .map(|(code, url)| (code.as_str(), url.as_str()))
    }
    pub fn preview_connection(&mut self) {
        let (_, rx) = mpsc::channel();
        self.pairing = Some(rx);
        self.connection_prompt = Some(("ABC123".into(), "https://cannamods.vip/connect".into()));
        self.account_status = "Visual fixture: enter this code on the verification page.".into();
    }
    pub fn cancel_sign_in(&mut self) {
        self.pair_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn start_sign_in(&mut self) {
        if self.pairing.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pairing = Some(rx);
        self.connection_prompt = None;
        self.pair_cancel = Default::default();
        let cancel = self.pair_cancel.clone();
        self.account_status = "Preparing a secure account connection…".into();
        std::thread::spawn(move || {
            let result = pair_account(&tx, &cancel).map_err(|e| e.to_string());
            let _ = tx.send(PairEvent::Finished(result));
        });
    }
    pub fn set_receiver(&mut self, rx: Receiver<String>) {
        self.tickets = Some(rx);
    }
    pub fn update(&mut self, ctx: &egui::Context) -> bool {
        let mut paired = false;
        if let Some(rx) = &self.heartbeat
            && let Ok((raw, revoked)) = rx.try_recv()
        {
            self.heartbeat = None;
            if revoked && session() == raw {
                crate::credentials::remove("canna-session");
                self.account_status =
                    "This device was signed out. Connect again to continue.".into();
                paired = true;
            }
        }
        if self.heartbeat.is_none() && self.last_heartbeat.elapsed() >= Duration::from_secs(60) {
            self.last_heartbeat = std::time::Instant::now();
            let raw = session();
            if !raw.is_empty() {
                let (tx, rx) = mpsc::channel();
                self.heartbeat = Some(rx);
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let revoked = reqwest::blocking::Client::builder()
                        .timeout(Duration::from_secs(10))
                        .build()
                        .ok()
                        .and_then(|c| {
                            c.get("https://cannamods.vip/api/v1/me")
                                .bearer_auth(&raw)
                                .send()
                                .ok()
                        })
                        .is_some_and(|r| r.status() == reqwest::StatusCode::UNAUTHORIZED);
                    let _ = tx.send((raw, revoked));
                    ctx.request_repaint();
                });
            }
        }
        ctx.request_repaint_after(Duration::from_secs(30));
        let mut finished = false;
        if let Some(rx) = &self.pairing {
            while let Ok(event) = rx.try_recv() {
                match event {
                    PairEvent::Started { code, url } => {
                        self.connection_prompt = Some((code.clone(), url.clone()));
                        self.account_status = format!(
                            "Enter connection code {code} on the verification page. Waiting for verification…"
                        );
                        ctx.copy_text(code);
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                    PairEvent::Finished(result) => {
                        paired = result.is_ok();
                        self.account_status = result.unwrap_or_else(|e| e);
                        finished = true;
                    }
                }
            }
        }
        if finished {
            self.pairing = None;
            self.connection_prompt = None;
        }
        if let Some(rx) = &self.tickets {
            while let Ok(t) = rx.try_recv() {
                self.pending.push_back(t);
                self.open = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        let mut reload = paired;
        if let Some(rx) = &self.result
            && let Ok(result) = rx.try_recv()
        {
            self.status = match result {
                Ok(s) => {
                    reload = true;
                    s
                }
                Err(e) => e.to_string(),
            };
            self.result = None;
            self.items = records();
        }
        if self.result.is_none()
            && let Some(ticket) = self.pending.pop_front()
        {
            self.status = "Connected to website. Downloading and verifying…".into();
            let (tx, rx) = mpsc::channel();
            self.result = Some(rx);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(fetch(&ticket));
                ctx.request_repaint();
            });
        }
        if self.result.is_some() || self.tickets.is_some() || self.pairing.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        reload
    }
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut reload = false;
        self.items = records();
        let (packs, warnings) = crate::modpacks::load_all();
        // Imported local content is also a download, even when it arrived in a pack bundle.
        for pack in &packs {
            for m in &pack.mods {
                if !m.local_file.is_empty()
                    && !self
                        .items
                        .iter()
                        .any(|d| d.sha256 == m.sha256 && d.app_id == pack.game.app_id)
                {
                    let path = crate::modpacks::local_directory().join(&m.local_file);
                    if path.is_file() {
                        self.items.push(Download {
                            name: m.name.clone(),
                            version: m.version.clone(),
                            app_id: pack.game.app_id,
                            description: m.description.clone(),
                            path,
                            sha256: m.sha256.clone(),
                            source: "Local file".into(),
                            pack_ids: vec![pack.id.clone()],
                            framework: false,
                        });
                    }
                }
            }
        }
        ui.heading("Your downloads");
        ui.label("Downloaded mods, framework packages and imported files, organized by modpack.");
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        if self.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Downloading and verifying…");
            });
        }
        for warning in warnings {
            ui.label(warning);
        }
        ui.add_space(12.0);
        egui::ScrollArea::horizontal()
            .id_salt("download-pack-tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.target,
                        String::new(),
                        format!("All ({})", self.items.len()),
                    );
                    let unassigned = self
                        .items
                        .iter()
                        .filter(|item| !packs.iter().any(|p| belongs(item, p)))
                        .count();
                    ui.selectable_value(
                        &mut self.target,
                        "unassigned".into(),
                        format!("Unassigned ({unassigned})"),
                    );
                    for pack in &packs {
                        let count = self.items.iter().filter(|item| belongs(item, pack)).count();
                        ui.selectable_value(
                            &mut self.target,
                            pack.id.clone(),
                            format!("{} ({count})", pack.name),
                        );
                    }
                });
            });
        if !self.target.is_empty()
            && self.target != "unassigned"
            && !packs.iter().any(|p| p.id == self.target)
        {
            self.target.clear();
        }
        let selected = packs.iter().find(|p| p.id == self.target);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Browse mods").clicked() {
                self.discover_requested = true;
            }
            if ui.button("Open downloads folder").clicked() {
                self.status = match std::fs::create_dir_all(root()).and_then(|_| {
                    std::process::Command::new("explorer.exe")
                        .arg(root())
                        .spawn()
                        .map(|_| ())
                }) {
                    Ok(()) => String::new(),
                    Err(e) => e.to_string(),
                };
            }
            if selected.is_some()
                && ui.button("Import local mod").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Unity mod", &["dll", "zip"])
                    .pick_file()
            {
                let result = (|| -> Result<()> {
                    let mut pack = selected.unwrap().clone();
                    let m = crate::modpacks::add_local(&path)?;
                    let bytes = std::fs::read(&path)?;
                    remember_mod(&pack, &m, &bytes, false)?;
                    pack.mods.push(m);
                    pack.save()?;
                    Ok(())
                })();
                self.status = match result {
                    Ok(()) => {
                        reload = true;
                        "Imported into this modpack.".into()
                    }
                    Err(e) => e.to_string(),
                };
            }
        });
        if let Some(pack) = selected {
            ui.label(format!("{} · {}", pack.name, pack.game.name));
        }
        ui.separator();
        let mut count = 0;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for item in &self.items {
                let visible = if let Some(pack) = selected { belongs(item, pack) } else if self.target == "unassigned" { !packs.iter().any(|p| belongs(item, p)) } else { true };
                if !visible { continue; }
                count += 1;
                egui::Frame::group(ui.style()).inner_margin(16).show(ui, |ui| {
                    ui.strong(format!("{} · {}", item.name, item.version));
                    ui.label(format!("{} · {}", if item.source.is_empty() { "Community library" } else { &item.source }, if item.path.is_file() { "Downloaded" } else { "File missing" }));
                    let assigned: Vec<_> = packs.iter().filter(|p| belongs(item, p)).map(|p| p.name.as_str()).collect();
                    ui.label(if assigned.is_empty() { "Unassigned".into() } else { assigned.join(" · ") });
                    if item.framework { ui.label("Framework package · installed automatically when setting up this game."); }
                    else if !item.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jar")) {
                        let row_key = format!("{}:{}", item.app_id, item.sha256);
                        let mut chosen = selected.map(|p| p.id.clone()).or_else(|| self.destinations.get(&row_key).cloned()).unwrap_or_default();
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt((&item.sha256, item.app_id)).height(340.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).selected_text(packs.iter().find(|p| p.id == chosen).map(|p| p.name.as_str()).unwrap_or("Choose modpack")).show_ui(ui, |ui| {
                                let choices=packs.iter().filter(|p|p.game.app_id==item.app_id).map(|p|(p.id.clone(),p.name.clone())).collect::<Vec<_>>();crate::ui_helpers::searchable_options(ui,&mut chosen,&choices);
                            });
                            // Preserve the row selection between frames separately from the active tab.
                            if !chosen.is_empty() { self.destinations.insert(format!("{}:{}", item.app_id, item.sha256), chosen.clone()); }
                            if chosen.is_empty() { chosen = self.destinations.get(&format!("{}:{}", item.app_id, item.sha256)).cloned().unwrap_or_default(); }
                            let target = packs.iter().find(|p| p.id == chosen && p.game.app_id == item.app_id);
                            if ui.add_enabled(target.is_some() && item.path.is_file(), egui::Button::new("Add to modpack")).clicked() {
                                let result = attach(item, &mut target.unwrap().clone());
                                self.status = match result { Ok(()) => { reload = true; "Added to modpack. Apply the pack to install it in the game.".into() }, Err(e) => e.to_string() };
                            }
                        });
                    } else { ui.label("Minecraft content · use the Minecraft instance folder until its content installer is available."); }
                    if ui.add_enabled(item.path.is_file(), egui::Button::new("Show file")).clicked() { let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", item.path.display())).spawn(); }
                });
                ui.add_space(8.0);
            }
            if count == 0 { ui.label("No downloads for this tab yet. Browse mods or import a local mod into a pack."); }
        });
        reload
    }
}
enum PairEvent {
    Started { code: String, url: String },
    Finished(std::result::Result<String, String>),
}
fn pair_account(
    tx: &mpsc::Sender<PairEvent>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let call = |path: &str, input: serde_json::Value| -> Result<serde_json::Value> {
        let response = client
            .post(format!("https://cannamods.vip/api/v1/desktop/{path}"))
            .json(&input)
            .send()?;
        anyhow::ensure!(
            response.status().is_success(),
            "Account connection failed ({}). Start again in Canna after signing in on the website.",
            response.status()
        );
        let mut bytes = Vec::new();
        response.take(8193).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 8192, "Invalid account connection response");
        Ok(serde_json::from_slice(&bytes)?)
    };
    let device = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows PC".into());
    let started = call(
        "start",
        serde_json::json!({"name":device.chars().filter(|c|!c.is_control()).take(60).collect::<String>()}),
    )?;
    let request = started["request"]
        .as_str()
        .context("Connection request missing")?;
    let proof = started["proof"]
        .as_str()
        .context("Connection proof missing")?;
    for raw in [request, proof] {
        anyhow::ensure!(
            raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid connection request"
        );
    }
    let code = started["code"]
        .as_str()
        .context("Update Canna to use code verification")?;
    anyhow::ensure!(
        code.len() == 6 && code.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid connection code"
    );
    let url = format!("https://cannamods.vip/connect?request={request}");
    let _ = tx.send(PairEvent::Started {
        code: code.into(),
        url,
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    while std::time::Instant::now() < deadline {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("Account connection cancelled.");
        }
        let status = call("poll", serde_json::json!({"request":request,"proof":proof}))?;
        if status["state"] == "connected" {
            let raw = status["token"]
                .as_str()
                .context("Account session missing")?;
            anyhow::ensure!(
                raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid account session"
            );
            crate::credentials::save("canna-session", raw.as_bytes())?;
            return Ok("Canna account connected. Server library is syncing.".into());
        }
        anyhow::ensure!(
            status["state"] == "waiting",
            "Account connection expired. Start again."
        );
        std::thread::sleep(Duration::from_secs(2));
    }
    bail!("Account connection expired. Click Sign in & connect account to try again.")
}
#[cfg(test)]
mod tests {
    #[test]
    fn download_tabs_match_pack_assignments_and_checksums_without_crossing_games() {
        let game = crate::model::bopl();
        let mut pack = crate::modpacks::Modpack::create(
            "Family pack".into(),
            String::new(),
            &game,
            crate::cache::Source::from_settings(&crate::model::Settings::load()),
            Vec::new(),
        );
        let mut item = super::Download {
            name: "Test mod".into(),
            version: "1".into(),
            app_id: game.app_id,
            description: String::new(),
            path: Default::default(),
            sha256: "a".repeat(64),
            source: "Canna server".into(),
            pack_ids: vec![pack.id.clone()],
            framework: false,
        };
        assert!(super::belongs(&item, &pack));
        item.app_id = 1557740;
        assert!(!super::belongs(&item, &pack));
        assert!(super::attach(&item, &mut pack).is_err());
        item.app_id = game.app_id;
        item.pack_ids.clear();
        assert!(!super::belongs(&item, &pack));
        pack.mods.push(crate::model::ModInfo {
            provenance: serde_json::Value::Null,
            content_type: String::new(),
            enabled: true,
            name: item.name.clone(),
            version: item.version.clone(),
            description: String::new(),
            file: "Mods/example.zip".into(),
            sha256: item.sha256.to_uppercase(),
            local_file: String::new(),
            dependencies: Vec::new(),
        });
        assert!(super::belongs(&item, &pack));
        item.framework = true;
        assert!(super::attach(&item, &mut pack).is_err());
    }
    #[test]
    fn links_cannot_override_server_or_include_commands() {
        assert!(super::parse_uri(&format!("canna://download/{}", "a".repeat(64))).is_ok());
        for s in [
            "canna://download/../../evil",
            "https://evil/download/x",
            "canna://download/x?url=https://evil",
        ] {
            assert!(super::parse_uri(s).is_err());
        }
    }
}
