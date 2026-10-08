use anyhow::{Context, Result, ensure};
use eframe::egui::{self, RichText};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

const API: &str = "https://cannamods.vip/api/v1";
const BINDING: &str = "__Host-canna_login";
#[derive(Default, PartialEq)]
enum Mode {
    #[default]
    Choose,
    Desktop,
}
struct Challenge {
    id: String,
    binding: Zeroizing<String>,
}
enum Outcome {
    Challenge(Challenge),
    Session(Zeroizing<String>),
    Profile { session: String, data: Value },
}
#[derive(Default)]
pub struct Account {
    pub open: bool,
    mode: Mode,
    username: String,
    password: Zeroizing<String>,
    code: Zeroizing<String>,
    challenge: Option<Challenge>,
    pending: Option<Receiver<std::result::Result<Outcome, String>>>,
    profile: Option<Value>,
    profile_session: String,
    status: String,
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!(
            "CannaDesktop/",
            env!("CARGO_PKG_VERSION"),
            " (Windows)"
        ))
        .build()?)
}
fn body(response: reqwest::blocking::Response) -> Result<Value> {
    let status = response.status();
    let mut bytes = Vec::new();
    response.take(131073).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 131072, "Account response exceeds limit");
    let data: Value = serde_json::from_slice(&bytes).context("Invalid account response")?;
    ensure!(
        status.is_success(),
        "{}",
        data["error"]
            .as_str()
            .unwrap_or("Account request failed; try again")
    );
    Ok(data)
}
fn login_result(data: Value, cookie: Option<String>) -> Result<Outcome> {
    if data["verification_required"] == true {
        anyhow::bail!("Verify your email on the website before signing in here.");
    }
    if data["two_factor_required"] == true {
        let id = data["challenge"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 64)
            .context("Invalid sign-in challenge")?;
        let binding = cookie.context("Sign-in challenge binding missing; start again")?;
        let raw = binding
            .strip_prefix(&format!("{BINDING}="))
            .context("Invalid sign-in challenge binding")?;
        ensure!(
            raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid sign-in challenge binding"
        );
        return Ok(Outcome::Challenge(Challenge {
            id: id.into(),
            binding: Zeroizing::new(binding),
        }));
    }
    let token = data["token"]
        .as_str()
        .context("Sign-in did not return a session")?;
    ensure!(
        token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid account session"
    );
    Ok(Outcome::Session(Zeroizing::new(token.into())))
}
fn login(username: String, password: Zeroizing<String>) -> Result<Outcome> {
    let response = client()?
        .post(format!("{API}/login"))
        .json(&json!({"username":username,"password":*password}))
        .send()?;
    let binding = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|h| h.to_str().ok())
        .filter_map(|s| s.split(';').next())
        .find(|s| s.starts_with(&format!("{BINDING}=")))
        .map(str::to_owned);
    login_result(body(response)?, binding)
}
fn verify(id: String, binding: Zeroizing<String>, code: Zeroizing<String>) -> Result<Outcome> {
    ensure!(
        code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()),
        "Enter the six-digit email code"
    );
    let response = client()?
        .post(format!("{API}/login/verify"))
        .header("origin", "https://cannamods.vip")
        .header("cookie", binding.as_str())
        .json(&json!({"challenge":id,"code":*code,"trust_device":false}))
        .send()?;
    login_result(body(response)?, None)
}
impl Account {
    pub fn hide(&mut self) {
        self.open = false;
        self.password.zeroize();
        self.code.zeroize();
    }
    pub fn preview(&mut self, profile: bool) {
        self.open = true;
        if profile {
            self.profile_session = "ui-fixture".into();
            self.profile = Some(
                json!({"username":"Canna Demo","role":"member","status":"Ready to play","bio":"Visual fixture only","kash":100}),
            );
        }
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    fn job(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce() -> Result<Outcome> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work().map_err(|e| e.to_string()));
            ctx.request_repaint();
        });
    }
    pub fn open(&mut self, ctx: &egui::Context, session: &str) {
        self.open = true;
        if !session.is_empty() && !self.busy() {
            self.load_profile(ctx, session);
        }
    }
    fn load_profile(&mut self, ctx: &egui::Context, session: &str) {
        let session = session.to_owned();
        self.status = "Loading profile…".into();
        self.job(ctx, move || {
            let client = client()?;
            let me = body(
                client
                    .get(format!("{API}/me"))
                    .bearer_auth(&session)
                    .send()?,
            )?;
            let id = me["id"]
                .as_i64()
                .filter(|id| *id > 0)
                .context("Invalid account profile")?;
            let mut data = body(
                client
                    .get(format!("{API}/profiles/{id}"))
                    .bearer_auth(&session)
                    .send()?,
            )?;
            data["kash"] = me["kash"].clone();
            data["roles"] = me["roles"].clone();
            data["can_rebound"] = me["can_rebound"].clone();
            Ok(Outcome::Profile { session, data })
        });
    }
    pub fn update(&mut self, ctx: &egui::Context, session: &str) -> bool {
        if self.profile_session != session {
            self.profile = None;
            self.profile_session = session.into();
            if self.open && !session.is_empty() && !self.busy() {
                self.load_profile(ctx, session);
            }
        }
        let result = self.pending.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(result) = result {
            self.pending = None;
            match result {
                Ok(Outcome::Challenge(challenge)) => {
                    self.challenge = Some(challenge);
                    self.status = "Check your email for your sign-in code.".into();
                }
                Ok(Outcome::Session(token)) => {
                    self.challenge = None;
                    self.code.zeroize();
                    match crate::credentials::save("canna-session", token.as_bytes()) {
                        Ok(()) => {
                            self.status = "Signed in.".into();
                            self.load_profile(ctx, &token);
                            return true;
                        }
                        Err(e) => self.status = e.to_string(),
                    }
                }
                Ok(Outcome::Profile {
                    session: source,
                    data,
                }) => {
                    if source == session {
                        self.profile = Some(data);
                        self.status.clear();
                    }
                }
                Err(error) => self.status = error,
            }
        }
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        false
    }
    pub fn reset(&mut self) {
        self.pending = None;
        self.profile = None;
        self.profile_session.zeroize();
        self.challenge = None;
        self.password.zeroize();
        self.code.zeroize();
        self.mode = Mode::Choose;
        self.status.clear();
    }
    // Returns a logout request; website authorization uses the existing pairing flow.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &str,
        website: &mut crate::website::Website,
    ) -> bool {
        let mut logout = false;
        if ui.button("‹ Back").clicked() {
            self.open = false;
            self.password.zeroize();
            self.code.zeroize();
        }
        ui.add_space(16.0);
        egui::ScrollArea::vertical()
            .id_salt("account_screen")
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_max_width(520.0);
                    ui.heading(if session.is_empty() {
                        "Log in to Canna"
                    } else {
                        "Your account"
                    });
                    ui.add_space(20.0);
                    if !session.is_empty() {
                        if let Some(p) = &self.profile {
                            let name = p["username"].as_str().unwrap_or("Canna member");
                            ui.label(
                                RichText::new(
                                    name.chars()
                                        .next()
                                        .unwrap_or('C')
                                        .to_uppercase()
                                        .to_string(),
                                )
                                .size(48.0)
                                .color(crate::GREEN),
                            );
                            ui.heading(name);
                            ui.label(p["role"].as_str().unwrap_or("member"));
                            if p["roles"]
                                .as_array()
                                .is_some_and(|roles| roles.iter().any(|role| role == "beta"))
                            {
                                ui.label("Beta · Canna Rebound access");
                            }
                            if let Some(status) = p["status"].as_str().filter(|s| !s.is_empty()) {
                                ui.label(status);
                            }
                            if let Some(bio) = p["bio"].as_str().filter(|s| !s.is_empty()) {
                                ui.label(bio);
                            }
                            ui.label(format!("{} Kash", p["kash"].as_i64().unwrap_or(0)));
                        }
                        ui.add_space(12.0);
                        if ui
                            .add_enabled(!self.busy(), egui::Button::new("Refresh profile"))
                            .clicked()
                        {
                            self.load_profile(ui.ctx(), session);
                        }
                        if ui.button("Manage logged-in devices").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(
                                "https://cannamods.vip/?devices=1",
                            ));
                        }
                        if ui.button("Log out").clicked() {
                            self.reset();
                            logout = true;
                        }
                    } else if website.connecting() {
                        ui.label("Authorize this device on the website");
                        if let Some((code, url)) = website.connection_prompt() {
                            ui.add_space(12.0);
                            ui.label(
                                RichText::new(code)
                                    .monospace()
                                    .size(40.0)
                                    .color(crate::GREEN),
                            );
                            ui.add_space(12.0);
                            if ui.button("Copy code").clicked() {
                                ui.ctx().copy_text(code.into());
                            }
                            if ui.button("Open verification page").clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                            }
                        }
                        ui.label(&website.account_status);
                        if ui.button("Cancel authorization").clicked() {
                            website.cancel_sign_in();
                        }
                    } else if self.mode == Mode::Choose {
                        ui.label("Choose how to sign in.");
                        ui.add_space(12.0);
                        if ui
                            .add_sized(
                                [300.0, 44.0],
                                egui::Button::new("Log in through the desktop app"),
                            )
                            .clicked()
                        {
                            self.mode = Mode::Desktop;
                            self.status.clear();
                        }
                        if ui
                            .add_enabled(
                                !self.busy(),
                                egui::Button::new("Authorize through the website")
                                    .min_size(egui::vec2(300.0, 44.0)),
                            )
                            .clicked()
                        {
                            website.start_sign_in();
                        }
                    } else {
                        ui.add_enabled_ui(!self.busy(), |ui| {
                            if let Some(challenge) = &self.challenge {
                                ui.label("Email verification code");
                                let field = ui.add(
                                    egui::TextEdit::singleline(&mut *self.code)
                                        .font(egui::FontId::monospace(32.0))
                                        .desired_width(280.0)
                                        .char_limit(12)
                                        .hint_text("123456"),
                                );
                                let submit = ui.button("Verify & log in").clicked()
                                    || field.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if submit {
                                    self.code.retain(|c| !c.is_whitespace());
                                    if self.code.len() == 6
                                        && self.code.bytes().all(|b| b.is_ascii_digit())
                                    {
                                        let id = challenge.id.clone();
                                        let binding = challenge.binding.clone();
                                        let code = Zeroizing::new(std::mem::take(&mut *self.code));
                                        self.status = "Verifying…".into();
                                        self.job(ui.ctx(), move || verify(id, binding, code));
                                    } else {
                                        self.status = "Enter the six-digit email code".into();
                                    }
                                }
                                if ui.button("Start again").clicked() {
                                    self.challenge = None;
                                    self.code.zeroize();
                                    self.status.clear();
                                }
                            } else {
                                ui.label("Username");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.username)
                                        .desired_width(300.0)
                                        .char_limit(80),
                                );
                                ui.label("Password");
                                let field = ui.add(
                                    egui::TextEdit::singleline(&mut *self.password)
                                        .password(true)
                                        .desired_width(300.0)
                                        .char_limit(256),
                                );
                                if ui.button("Log in").clicked()
                                    || field.lost_focus()
                                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                {
                                    if self.username.trim().is_empty() || self.password.is_empty() {
                                        self.status = "Enter your username and password".into();
                                    } else {
                                        let username = self.username.trim().to_owned();
                                        let password =
                                            Zeroizing::new(std::mem::take(&mut *self.password));
                                        self.status = "Signing in…".into();
                                        self.job(ui.ctx(), move || login(username, password));
                                    }
                                }
                                if ui.button("Forgot password?").clicked() {
                                    ui.ctx()
                                        .open_url(egui::OpenUrl::new_tab("https://cannamods.vip"));
                                }
                            }
                            if ui.button("Choose another sign-in method").clicked() {
                                self.challenge = None;
                                self.password.zeroize();
                                self.code.zeroize();
                                self.mode = Mode::Choose;
                            }
                        });
                    }
                    if self.busy() {
                        ui.spinner();
                    }
                    if !self.status.is_empty() {
                        ui.label(&self.status);
                    }
                    ui.add_space(20.0);
                    ui.hyperlink_to("Help / FAQ", "https://cannamods.vip/help");
                });
            });
        logout
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_requires_bound_email_verification_and_valid_sessions() {
        let token = "a".repeat(64);
        let data = json!({"two_factor_required":true,"challenge":"challenge","token":token});
        assert!(login_result(data.clone(), None).is_err());
        assert!(
            login_result(data.clone(), Some(format!("{BINDING}=bad\r\nCookie: fake"))).is_err()
        );
        assert!(matches!(
            login_result(data, Some(format!("{BINDING}={token}"))).unwrap(),
            Outcome::Challenge(_)
        ));
        assert!(matches!(
            login_result(json!({"token":token}), None).unwrap(),
            Outcome::Session(_)
        ));
        assert!(login_result(json!({"token":"invalid"}), None).is_err());
        assert!(login_result(json!({"verification_required":true,"token":token}), None).is_err());
    }
    #[test]
    fn logout_clears_sensitive_state_and_discards_late_login_completion() {
        let mut account = Account {
            password: Zeroizing::new("private".into()),
            code: Zeroizing::new("123456".into()),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        account.pending = Some(rx);
        account.reset();
        assert!(
            account.password.is_empty() && account.code.is_empty() && account.challenge.is_none()
        );
        assert!(
            tx.send(Ok(Outcome::Session(Zeroizing::new("a".repeat(64)))))
                .is_err()
        );
    }
}
