//! Native friend conversations use the member's existing authenticated session.
//! No message drafts, tokens or response bodies are written to disk or logs.
use super::*;

enum ResponseKind {
    Overview,
    Messages(i64),
    Mutation,
    Read,
}
struct Request {
    path: String,
    encoded: Zeroizing<String>,
}
type SocialResponse = (ResponseKind, std::result::Result<Value, (String, bool)>);
#[derive(Default)]
pub(super) struct Social {
    session: Zeroizing<String>,
    member: i64,
    peer: i64,
    overview: Value,
    messages: Value,
    pending: Option<Receiver<SocialResponse>>,
    retry: Option<Request>,
    draft: Zeroizing<String>,
    pack: String,
    status: String,
    refresh: bool,
    next_messages: bool,
    next_read: Option<(i64, i64)>,
    last_refresh: Option<std::time::Instant>,
}
fn pack_id(text: &str) -> Result<Option<String>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let id = text
        .trim()
        .strip_prefix("https://cannamods.vip/packs/")
        .unwrap_or(text.trim())
        .trim_end_matches('/');
    ensure!(
        id.len() == 36
            && id
                .bytes()
                .enumerate()
                .all(|(i, b)| if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }),
        "Attach a Canna shared modpack link"
    );
    Ok(Some(id.to_ascii_lowercase()))
}
fn request_identity() -> String {
    format!(
        "desktop-social-{:x}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
impl Social {
    fn select_peer(&mut self, peer: i64) {
        if self.peer != peer {
            self.peer = peer;
            self.messages = Value::Null;
            self.draft.zeroize();
            self.pack.clear();
        }
        self.next_messages = true;
    }
    fn job(
        &mut self,
        ctx: &egui::Context,
        path: String,
        encoded: Option<Zeroizing<String>>,
        kind: ResponseKind,
    ) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let session = self.session.clone();
        let member = self.member;
        let context = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> std::result::Result<Value, (String, bool)> {
                let client = client().map_err(|e| (e.to_string(), true))?;
                let request = if let Some(body) = encoded {
                    client
                        .post(format!("{API}/{path}"))
                        .header("content-type", "application/json")
                        .header("X-Canna-Member", member)
                        .body(body.to_string())
                } else {
                    client.get(format!("{API}/{path}"))
                };
                let response = request.bearer_auth(session.as_str()).send().map_err(|_| {
                    (
                        "Connection ended before confirmation; retry the original action.".into(),
                        true,
                    )
                })?;
                let status = response.status();
                let mut bytes = Zeroizing::new(Vec::new());
                response
                    .take(2_097_153)
                    .read_to_end(&mut bytes)
                    .map_err(|_| ("Response was interrupted.".into(), true))?;
                if bytes.len() > 2_097_152 {
                    return Err((
                        "Private message response exceeds the supported size".into(),
                        true,
                    ));
                }
                let value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| ("Private message response is unavailable".into(), true))?;
                if !status.is_success() {
                    return Err((
                        value["error"]
                            .as_str()
                            .unwrap_or("Request failed")
                            .to_owned(),
                        status.is_server_error(),
                    ));
                }
                if value["member_id"].as_i64() != Some(member) {
                    return Err((
                        "Your account changed; reopen Friends & messages.".into(),
                        false,
                    ));
                }
                Ok(value)
            })();
            let _ = tx.send((kind, result));
            context.request_repaint();
        });
    }
    fn mutate(&mut self, ui: &egui::Ui, path: &str, mut data: Value) {
        if self.pending.is_some() || self.retry.is_some() {
            return;
        }
        data["request_id"] = json!(request_identity());
        let request = Request {
            path: path.into(),
            encoded: Zeroizing::new(data.to_string()),
        };
        self.job(
            ui.ctx(),
            request.path.clone(),
            Some(request.encoded.clone()),
            ResponseKind::Mutation,
        );
        self.retry = Some(request);
    }
    fn friend(&mut self, ui: &egui::Ui, target: i64, action: &str) {
        self.mutate(
            ui,
            "social/friends",
            json!({"target":target,"action":action}),
        );
    }
    pub(super) fn show(&mut self, ui: &mut egui::Ui, session: &str, member: i64) {
        if self.session.as_str() != session || self.member != member {
            *self = Self {
                session: Zeroizing::new(session.to_owned()),
                member,
                refresh: true,
                ..Self::default()
            };
        }
        if let Some((kind, result)) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            match result {
                Ok(data) => match kind {
                    ResponseKind::Overview => {
                        self.overview = data;
                        self.next_messages = self.peer > 0;
                        self.last_refresh = Some(std::time::Instant::now());
                    }
                    ResponseKind::Messages(peer) => {
                        if self.peer == peer {
                            if let Some(seq) = data["next"].as_i64().filter(|s| *s > 0) {
                                self.next_read = Some((peer, seq));
                            }
                            self.messages = data;
                        }
                    }
                    ResponseKind::Mutation => {
                        if let Some(request) =
                            self.retry.as_ref().filter(|r| r.path == "social/messages")
                            && let Ok(payload) = serde_json::from_str::<Value>(&request.encoded)
                            && payload["target"] == self.peer
                            && payload["body"].as_str() == Some(self.draft.as_str())
                        {
                            self.draft.zeroize();
                            self.pack.clear();
                        }
                        self.retry = None;
                        self.refresh = true;
                        self.status = "Saved.".into();
                    }
                    ResponseKind::Read => {}
                },
                Err((message, uncertain)) => {
                    self.status = message;
                    if matches!(kind, ResponseKind::Mutation) && !uncertain {
                        self.retry = None;
                    }
                }
            }
        }
        ui.heading("Friends & private messages");
        ui.label("Private conversations and shared modpack cards.");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.pending.is_none(), egui::Button::new("Refresh"))
                .clicked()
            {
                self.refresh = true;
            }
            ui.hyperlink_to("Find members", "https://cannamods.vip/members");
        });
        if self.pending.is_none() {
            if let Some((peer, seq)) = self.next_read.take() {
                self.job(
                    ui.ctx(),
                    "social/messages/read".into(),
                    Some(Zeroizing::new(json!({"peer":peer,"seq":seq}).to_string())),
                    ResponseKind::Read,
                );
            } else if self.next_messages {
                self.next_messages = false;
                self.job(
                    ui.ctx(),
                    format!("social/messages/{}", self.peer),
                    None,
                    ResponseKind::Messages(self.peer),
                );
            } else if self.refresh
                || self
                    .last_refresh
                    .is_some_and(|t| t.elapsed().as_secs() >= 5)
            {
                self.refresh = false;
                self.job(ui.ctx(), "social".into(), None, ResponseKind::Overview);
            }
        }
        let locked = self.pending.is_some() || self.retry.is_some();
        egui::ScrollArea::vertical()
            .id_salt("desktop_friends")
            .max_height(180.0)
            .show(ui, |ui| {
                for row in self.overview["friends"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    let id = row["id"].as_i64().unwrap_or(0);
                    let name = row["username"].as_str().unwrap_or("Member");
                    let accepted = row["status"] == "accepted";
                    ui.horizontal(|ui| {
                        if ui.selectable_label(self.peer == id, name).clicked() {
                            self.select_peer(id);
                        }
                        if !accepted {
                            ui.small(if row["requester"] == self.member {
                                "Request sent"
                            } else {
                                "Incoming request"
                            });
                            if row["requester"] != self.member
                                && ui
                                    .add_enabled(!locked, egui::Button::new("Accept"))
                                    .clicked()
                            {
                                self.friend(ui, id, "accept");
                            }
                        }
                        if ui
                            .add_enabled(
                                !locked,
                                egui::Button::new(if accepted {
                                    "Remove friend"
                                } else {
                                    "Cancel / decline"
                                }),
                            )
                            .clicked()
                        {
                            self.friend(ui, id, "remove");
                        }
                    });
                }
                for row in self.overview["conversations"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    let id = row["id"].as_i64().unwrap_or(0);
                    if ui
                        .button(format!(
                            "{} · {} unread",
                            row["username"].as_str().unwrap_or("Member"),
                            row["unread"].as_i64().unwrap_or(0)
                        ))
                        .clicked()
                    {
                        self.select_peer(id);
                    }
                }
                for row in self.overview["blocks"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "Blocked: {}",
                            row["username"].as_str().unwrap_or("Member")
                        ));
                        if ui
                            .add_enabled(!locked, egui::Button::new("Unblock"))
                            .clicked()
                        {
                            self.friend(ui, row["id"].as_i64().unwrap_or(0), "unblock");
                        }
                    });
                }
            });
        if self.peer > 0 {
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading("Conversation");
                if ui
                    .add_enabled(!locked, egui::Button::new("Block member"))
                    .clicked()
                {
                    self.friend(ui, self.peer, "block");
                }
            });
            egui::ScrollArea::vertical()
                .id_salt(("desktop_dm", self.peer))
                .max_height(340.0)
                .show(ui, |ui| {
                    for row in self.messages["messages"].as_array().into_iter().flatten() {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.small(if row["sender"] == self.member {
                                "You"
                            } else {
                                "Friend"
                            });
                            if let Some(body) = row["body"].as_str() {
                                ui.label(body);
                            }
                            let pack = &row["pack"];
                            if pack.is_object() {
                                ui.strong(pack["name"].as_str().unwrap_or("Shared modpack"));
                                ui.small(format!(
                                    "{} · {} mods",
                                    pack["game"].as_str().unwrap_or("Game"),
                                    pack["mod_count"].as_u64().unwrap_or(0)
                                ));
                                if pack["available"] == true {
                                    if let Some(id) =
                                        pack["id"].as_str().filter(|s| pack_id(s).is_ok())
                                    {
                                        ui.hyperlink_to(
                                            "Open / download this modpack",
                                            format!("https://cannamods.vip/packs/{id}"),
                                        );
                                    }
                                } else {
                                    ui.label(if pack["requires_beta"] == true {
                                        "Canna Bliss Beta access required"
                                    } else {
                                        "This pack is unavailable"
                                    });
                                }
                            }
                        });
                    }
                });
            ui.label("Message");
            ui.add(
                egui::TextEdit::multiline(&mut *self.draft)
                    .desired_rows(3)
                    .char_limit(4000),
            );
            ui.label("Shared pack link (optional)");
            ui.text_edit_singleline(&mut self.pack);
            if ui
                .add_enabled(
                    !locked && self.messages["can_send"] == true,
                    egui::Button::new("Send private message"),
                )
                .clicked()
            {
                match pack_id(&self.pack) {
                    Ok(pack_id) => {
                        let target = self.peer;
                        let body = self.draft.to_string();
                        self.mutate(
                            ui,
                            "social/messages",
                            json!({"target":target,"body":body,"pack_id":pack_id}),
                        );
                    }
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
        if let Some(request) = &self.retry {
            let path = request.path.clone();
            let encoded = request.encoded.clone();
            if ui
                .add_enabled(
                    self.pending.is_none(),
                    egui::Button::new("Retry pending action"),
                )
                .clicked()
            {
                self.job(ui.ctx(), path, Some(encoded), ResponseKind::Mutation);
            }
        }
        if self.pending.is_some() {
            ui.spinner();
        }
        if !self.status.is_empty() {
            ui.label(&self.status);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(300));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachments_are_only_canna_pack_ids() {
        assert_eq!(
            pack_id("https://cannamods.vip/packs/11111111-1111-4111-8111-111111111111")
                .unwrap()
                .unwrap(),
            "11111111-1111-4111-8111-111111111111"
        );
        for invalid in [
            "https://evil.example/packs/11111111-1111-4111-8111-111111111111",
            "../../file",
            "11111111-1111-4111-8111-111111111111?token=secret",
        ] {
            assert!(pack_id(invalid).is_err());
        }
    }
}
