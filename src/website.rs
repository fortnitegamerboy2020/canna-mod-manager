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
        .join("website-downloads")
}
fn records() -> Vec<Download> {
    std::fs::read(root().join("library.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
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
    let mut items = records();
    items.retain(|m| m.sha256 != claim.sha256);
    items.push(Download {
        name: claim.name.clone(),
        version: claim.version.clone(),
        app_id: claim.app_id,
        description: claim.description.clone(),
        path,
        sha256: claim.sha256.clone(),
    });
    std::fs::write(
        root().join("library.json"),
        serde_json::to_vec_pretty(&items)?,
    )?;
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
    tickets: Option<Receiver<String>>,
    pending: std::collections::VecDeque<String>,
    result: Option<Receiver<Result<String>>>,
    pub open: bool,
    status: String,
    items: Vec<Download>,
    target: String,
}
impl Default for Website {
    fn default() -> Self {
        Self {
            tickets: None,
            pending: Default::default(),
            result: None,
            open: false,
            status: String::new(),
            items: records(),
            target: String::new(),
        }
    }
}
impl Website {
    pub fn busy(&self) -> bool {
        self.result.is_some() || !self.pending.is_empty()
    }
    pub fn set_receiver(&mut self, rx: Receiver<String>) {
        self.tickets = Some(rx);
    }
    pub fn update(&mut self, ctx: &egui::Context) -> bool {
        if let Some(rx) = &self.tickets {
            while let Ok(t) = rx.try_recv() {
                self.pending.push_back(t);
                self.open = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }
        let mut reload = false;
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
        if self.result.is_some() || self.tickets.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        reload
    }
    pub fn ui(&mut self, ctx: &egui::Context) -> bool {
        let mut reload = false;
        egui::Window::new("Website downloads").open(&mut self.open).default_width(600.0).show(ctx,|ui| {
            ui.label(&self.status);ui.label("Browse the private community library on cannamods.vip. Downloads arrive here after clicking Download on the website.");
            if ui.button("Open community library").clicked() {ui.ctx().open_url(egui::OpenUrl::new_tab("https://cannamods.vip/"));}
            if self.items.is_empty() {ui.label("No website downloads yet.");}
            let(packs,_)=crate::modpacks::load_all();
            egui::ScrollArea::vertical().max_height(450.0).show(ui,|ui|for item in &self.items {
                ui.separator();ui.strong(format!("{} · {}",item.name,item.version));ui.label(format!("Steam game {}",item.app_id));
                egui::ComboBox::from_id_salt(&item.sha256).selected_text(packs.iter().find(|p|p.id==self.target && p.game.app_id==item.app_id).map(|p|p.name.as_str()).unwrap_or("Choose a matching modpack")).show_ui(ui,|ui|for p in packs.iter().filter(|p|p.game.app_id==item.app_id) {ui.selectable_value(&mut self.target,p.id.clone(),&p.name);});
                let target=packs.iter().find(|p|p.id==self.target && p.game.app_id==item.app_id);
                if ui.add_enabled(target.is_some(),egui::Button::new("Add to modpack")).clicked() {
                    let result=(||->Result<()> {let mut pack=target.unwrap().clone();let mut m=crate::modpacks::add_local(&item.path)?;m.name=item.name.clone();m.version=item.version.clone();m.description=item.description.clone();pack.mods.retain(|old|old.name!=m.name);pack.mods.push(m);pack.save()?;Ok(())})();
                    match result {Ok(())=>{self.status="Mod added. Apply the modpack to install it in the game.".into();reload=true;},Err(e)=>self.status=e.to_string()}
                }
            });
        });
        reload
    }
}
#[cfg(test)]
mod tests {
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
