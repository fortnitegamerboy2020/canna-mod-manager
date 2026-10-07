//! Native metadata browser. Provider credentials never leave the Canna server.
use anyhow::{Result, ensure};
use eframe::egui;
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::mpsc::{self, Receiver},
    time::Duration,
};
const API: &str = "https://cannamods.vip/api/v1/";
#[derive(Clone, Debug, PartialEq)]
struct Filters {
    provider: String,
    game: String,
    q: String,
    order: String,
    category: String,
    version: String,
    loader: String,
    content_type: String,
    page: u32,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            provider: "modrinth".into(),
            game: "minecraft".into(),
            q: String::new(),
            order: "downloads".into(),
            category: String::new(),
            version: String::new(),
            loader: String::new(),
            content_type: String::new(),
            page: 1,
        }
    }
}
impl Filters {
    fn url(&self) -> Result<reqwest::Url> {
        let mut url = reqwest::Url::parse(&format!("{API}providers/search"))?;
        url.query_pairs_mut().extend_pairs([
            ("provider", self.provider.as_str()),
            ("game", &self.game),
            ("q", &self.q),
            ("order", &self.order),
            ("category", &self.category),
            ("version", &self.version),
            ("loader", &self.loader),
            ("content_type", &self.content_type),
            ("page", &self.page.to_string()),
        ]);
        Ok(url)
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Browse,
    Preview,
    Subscriptions,
    Import,
    Download,
    Remove,
    Install,
}
struct Job {
    kind: Kind,
    rx: Receiver<Result<Value, String>>,
    session: String,
}
pub struct Browser {
    pub mode: u8,
    filters: Filters,
    page: Value,
    subscriptions: Value,
    subscription_page: u32,
    subscription_query: String,
    job: Option<Job>,
    status: String,
    preview: Option<Value>,
    release: String,
    optional: bool,
    loaded: bool,
    subscriptions_loaded: bool,
    changed: bool,
    account: String,
    last_subscriptions: std::time::Instant,
    link: String,
    instance_target: String,
    world_target: String,
    #[cfg(test)]
    modal_rect: Option<egui::Rect>,
    imported_id: String,
}
impl Default for Browser {
    fn default() -> Self {
        Self {
            mode: 0,
            filters: Filters::default(),
            page: Value::Null,
            subscriptions: Value::Null,
            subscription_page: 1,
            subscription_query: String::new(),
            job: None,
            status: String::new(),
            preview: None,
            release: String::new(),
            optional: false,
            loaded: false,
            subscriptions_loaded: false,
            changed: false,
            account: String::new(),
            last_subscriptions: std::time::Instant::now(),
            link: String::new(),
            instance_target: String::new(),
            world_target: String::new(),
            #[cfg(test)]
            modal_rect: None,
            imported_id: String::new(),
        }
    }
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(180))
        .build()?)
}
fn request(
    client: &reqwest::blocking::Client,
    token: &str,
    method: reqwest::Method,
    url: reqwest::Url,
    body: Option<Value>,
) -> Result<Value> {
    ensure!(url.as_str().starts_with(API), "Invalid Canna API address");
    ensure!(!token.is_empty(), "Connect your Canna account in Settings");
    let mut req = client.request(method, url).bearer_auth(token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let response = req.send()?;
    let code = response.status();
    let mut bytes = Vec::new();
    response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "Server response exceeds limit"
    );
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    ensure!(
        code.is_success(),
        "{}",
        if code.as_u16() == 401 {
            "Reconnect your Canna account in Settings".into()
        } else {
            value["error"]
                .as_str()
                .or(value["message"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Canna request failed ({code})"))
        }
    );
    ensure!(!value.is_null(), "Invalid server response");
    Ok(value)
}
fn thumbnails(page: &mut Value) {
    use base64::Engine;
    let Some(items) = page["items"].as_array_mut() else {
        return;
    };
    // Four bounded workers keep image loading away from the render thread.
    for batch in items.chunks_mut(4) {
        std::thread::scope(|scope| {
            for item in batch {
                scope.spawn(move || {
                    let result = (|| -> Result<String> {
                        let url = reqwest::Url::parse(text(item, "icon_url"))?;
                        ensure!(
                            url.scheme() == "https"
                                && url.username().is_empty()
                                && url.password().is_none()
                                && url.port().is_none()
                                && matches!(
                                    url.host_str(),
                                    Some(
                                        "cdn.modrinth.com"
                                            | "media.forgecdn.net"
                                            | "mediafilez.forgecdn.net"
                                            | "gcdn.thunderstore.io"
                                            | "ccdn.thunderstore.io"
                                    )
                                ),
                            "Unsupported artwork host"
                        );
                        let c = reqwest::blocking::Client::builder()
                            .redirect(reqwest::redirect::Policy::none())
                            .timeout(Duration::from_secs(2))
                            .build()?;
                        let mut bytes = Vec::new();
                        c.get(url)
                            .send()?
                            .error_for_status()?
                            .take(1024 * 1024 + 1)
                            .read_to_end(&mut bytes)?;
                        ensure!(bytes.len() <= 1024 * 1024, "Artwork too large");
                        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
                            .with_guessed_format()?;
                        let mut limits = image::Limits::default();
                        limits.max_image_width = Some(2048);
                        limits.max_image_height = Some(2048);
                        reader.limits(limits);
                        let image = reader.decode()?.thumbnail(128, 128);
                        let mut png = std::io::Cursor::new(Vec::new());
                        image.write_to(&mut png, image::ImageFormat::Png)?;
                        Ok(base64::engine::general_purpose::STANDARD.encode(png.into_inner()))
                    })();
                    if let Ok(data) = result {
                        item["icon_data"] = json!(data);
                    }
                });
            }
        });
    }
}
fn clear_artwork(ctx: &egui::Context, page: &Value) {
    for item in page["items"].as_array().into_iter().flatten() {
        let key = egui::Id::new(("mod-art", text(item, "source_url"), "metadata"));
        ctx.data_mut(|d| d.remove::<egui::TextureHandle>(key));
    }
}
fn artwork(ui: &mut egui::Ui, item: &Value) {
    if let Ok(info) = serde_json::from_value::<crate::model::ModInfo>(
        json!({"name":text(item,"name"),"version":"metadata","file":text(item,"source_url"),"provenance":item}),
    ) {
        crate::ui_helpers::mod_art(ui, &info, egui::vec2(56.0, 56.0));
    }
}
fn api(path: &str) -> reqwest::Url {
    reqwest::Url::parse(&format!("{API}{path}")).unwrap()
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}
fn compatible(v: &Value, loader: &str, version: &str) -> bool {
    let has = |key: &str, want: &str| {
        want.is_empty()
            || v[key]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(want)))
    };
    has("loaders", loader) && has("game_versions", version)
}
fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, choices: &[(&str, &str)]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(
            choices
                .iter()
                .find(|(v, _)| *v == value)
                .map(|(_, name)| *name)
                .unwrap_or(value),
        )
        .show_ui(ui, |ui| {
            for (v, name) in choices {
                ui.selectable_value(value, (*v).into(), *name);
            }
        });
}
fn credits(ui: &mut egui::Ui, item: &Value) {
    ui.horizontal_wrapped(|ui| {
        let authors = text(item, "authors");
        let author_url = text(item, "author_url");
        if !authors.is_empty() {
            if author_url.starts_with("https://") {
                ui.hyperlink_to(format!("By {authors}"), author_url);
            } else {
                ui.label(format!("By {authors}"));
            }
        }
        if text(item, "source_url").starts_with("https://") {
            ui.hyperlink_to("Original project", text(item, "source_url"));
        }
    });
}
impl Browser {
    pub fn preview_fixture(&mut self) {
        self.account = "ui-fixture".into();
        self.loaded = true;
        self.page = json!({"items":[{"name":"Native browser fixture","description":"Metadata listings are browsable inside Canna. Select a release to retrieve its archive and subscribe.","authors":"Fixture author","source_url":"https://modrinth.com/mod/fixture","author_url":"https://modrinth.com/user/fixture","downloads":12345,"rating":120,"rating_label":"followers"}],"categories":[{"id":"performance","name":"Performance"}],"has_more":true});
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn select_game(&mut self, app_id: u32) {
        self.mode = 0;
        if app_id == u32::MAX {
            self.filters.game = "minecraft".into();
            self.filters.provider = "modrinth".into();
        } else if let Some(g) = crate::game_profiles::games()
            .iter()
            .find(|g| g.app_id == app_id)
        {
            self.filters.game = g.community.clone();
            self.filters.provider = "thunderstore".into();
        } else {
            self.mode = 1;
            return;
        }
        self.filters.category.clear();
        self.filters.loader.clear();
        self.filters.version.clear();
        self.filters.content_type.clear();
        self.filters.q.clear();
        self.filters.page = 1;
        self.loaded = false;
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        kind: Kind,
        method: reqwest::Method,
        url: reqwest::Url,
        body: Option<Value>,
    ) {
        if self.job.is_some() {
            return;
        }
        let token = crate::website::session();
        let session = token.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.status = match kind {
            Kind::Import => "Retrieving archive and dependencies; running server review…",
            Kind::Download => "Downloading verified archive…",
            _ => "Loading…",
        }
        .into();
        self.job = Some(Job { kind, rx, session });
        std::thread::spawn(move || {
            let result=(|| {
                let client=client()?;
                let mut data=request(&client,&token,method,url,body)?;
                if matches!(kind,Kind::Browse) { thumbnails(&mut data); }
                if matches!(kind,Kind::Import) && data["approved"]==true {
                    let ticket=request(&client,&token,reqwest::Method::POST,api("download-tickets"),Some(json!({"kind":"mods","id":data["id"]})))?;
                    // Import and subscription succeeded even if the local transfer fails.
                    match crate::website::receive_ticket(text(&ticket,"ticket")) {Ok(message)=>data["download_message"]=json!(message),Err(e)=>data["download_message"]=json!(format!("Subscribed, but local download failed: {e}. Retry in Subscriptions."))}
                } else if matches!(kind,Kind::Download) {
                    let message=crate::website::receive_ticket(text(&data,"ticket"))?;data["download_message"]=json!(message);
                }
                Ok(data)
            })().map_err(|e:anyhow::Error|e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub fn update(&mut self, ctx: &egui::Context, account: &str) -> bool {
        if account != self.account {
            self.account = account.to_owned();
            clear_artwork(ctx, &self.page);
            self.page = Value::Null;
            self.subscriptions = Value::Null;
            self.preview = None;
            self.imported_id.clear();
            self.loaded = false;
            self.subscriptions_loaded = false;
            self.status.clear();
        }
        let result = self.job.as_ref().and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = result {
            let job = self.job.take().unwrap();
            if job.session != self.account {
                return false;
            }
            match result {
                Err(e) => self.status = e,
                Ok(data) => {
                    self.status.clear();
                    match job.kind {
                        Kind::Browse => {
                            self.status = if data["stale"] == true {
                                "Showing cached provider metadata; provider is temporarily unavailable.".into()
                            } else {
                                String::new()
                            };
                            clear_artwork(ctx, &self.page);
                            self.page = data;
                        }
                        Kind::Preview => {
                            self.release = data["versions"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find(|v| {
                                    compatible(v, &self.filters.loader, &self.filters.version)
                                })
                                .map(|v| text(v, "id").to_owned())
                                .unwrap_or_default();
                            self.preview = Some(data);
                            self.optional = false;
                        }
                        Kind::Subscriptions => self.subscriptions = data,
                        Kind::Import => {
                            self.imported_id = text(&data, "id").into();
                            self.subscriptions_loaded = false;
                            self.changed = true;
                            self.status = if data["approved"] == true {
                                text(&data, "download_message").into()
                            } else {
                                "Subscribed. The mod is awaiting server analysis or moderator review. Refresh Subscriptions to check its status.".into()
                            };
                        }
                        Kind::Download => {
                            self.changed = true;
                            self.status = text(&data, "download_message").into();
                        }
                        Kind::Install => self.status = text(&data, "message").into(),
                        Kind::Remove => {
                            self.subscriptions_loaded = false;
                            self.status = "Unsubscribed.".into();
                        }
                    }
                }
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        std::mem::take(&mut self.changed)
    }
    fn browse(&mut self, ctx: &egui::Context) {
        self.loaded = true;
        clear_artwork(ctx, &self.page);
        self.page = Value::Null;
        match self.filters.url() {
            Ok(url) => self.start(ctx, Kind::Browse, reqwest::Method::GET, url, None),
            Err(e) => self.status = e.to_string(),
        }
    }
    fn subs(&mut self, ctx: &egui::Context) {
        self.subscriptions_loaded = true;
        self.last_subscriptions = std::time::Instant::now();
        let mut url = api("mods/subscriptions");
        url.query_pairs_mut()
            .append_pair("page", &self.subscription_page.to_string())
            .append_pair("q", &self.subscription_query);
        self.start(ctx, Kind::Subscriptions, reqwest::Method::GET, url, None);
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        catalog: &[crate::model::GameInfo],
        source: Option<&crate::cache::Source>,
        packs: &mut crate::pack_ui::PackUi,
        target: &mut Option<String>,
    ) {
        if self.account.is_empty() {
            ui.label(
                "Connect your Canna account above to browse providers and manage subscriptions.",
            );
            return;
        }
        self.attachment(ui, catalog, source, packs, target);
        if self.mode == 2 {
            self.show_subscriptions(ui);
            return;
        }
        ui.heading("Browse mods");
        let before = self.filters.clone();
        let mut search = false;
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                combo(
                    ui,
                    "provider-source",
                    &mut self.filters.provider,
                    &[
                        ("modrinth", "Modrinth"),
                        ("curseforge", "CurseForge"),
                        ("thunderstore", "Thunderstore"),
                    ],
                );
                if self.filters.provider != before.provider {
                    self.filters.game = if self.filters.provider == "thunderstore" {
                        "rounds"
                    } else {
                        "minecraft"
                    }
                    .into();
                    self.filters.category.clear();
                    self.filters.loader.clear();
                    self.filters.version.clear();
                    self.filters.content_type.clear();
                }
                let label = if self.filters.game == "minecraft" {
                    "Minecraft"
                } else {
                    crate::game_profiles::by_community(&self.filters.game)
                        .map(|g| g.name.as_str())
                        .unwrap_or("Choose game")
                };
                egui::ComboBox::from_id_salt("provider-game")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        if self.filters.provider != "thunderstore" {
                            ui.selectable_value(
                                &mut self.filters.game,
                                "minecraft".into(),
                                "Minecraft",
                            );
                        }
                        if self.filters.provider != "modrinth" {
                            for g in crate::game_profiles::games() {
                                ui.selectable_value(
                                    &mut self.filters.game,
                                    g.community.clone(),
                                    &g.name,
                                );
                            }
                        }
                    });
                combo(
                    ui,
                    "provider-order",
                    &mut self.filters.order,
                    &[
                        ("downloads", "Most downloaded"),
                        ("rating", "Top rated / popular"),
                        ("updated", "Recently updated"),
                        ("newest", "Newest"),
                    ],
                );
                egui::ComboBox::from_id_salt("provider-category")
                    .selected_text(if self.filters.category.is_empty() {
                        "All categories"
                    } else {
                        &self.filters.category
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.filters.category,
                            String::new(),
                            "All categories",
                        );
                        for c in self.page["categories"].as_array().into_iter().flatten() {
                            let id = c["id"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| c["id"].to_string());
                            ui.selectable_value(&mut self.filters.category, id, text(c, "name"));
                        }
                    });
            });
            if self.filters.game != before.game {
                self.filters.category.clear();
                self.filters.loader.clear();
                self.filters.version.clear();
                self.filters.content_type.clear();
            }
            if self.filters.game == "minecraft" {
                ui.horizontal_wrapped(|ui| {
                    combo(
                        ui,
                        "provider-type",
                        &mut self.filters.content_type,
                        &[
                            ("", "All content"),
                            ("mod", "Mods"),
                            ("shader", "Shaders"),
                            ("resourcepack", "Resource packs"),
                            ("datapack", "Data packs"),
                        ],
                    );
                    combo(
                        ui,
                        "provider-loader",
                        &mut self.filters.loader,
                        &[
                            ("", "All loaders"),
                            ("fabric", "Fabric"),
                            ("forge", "Forge"),
                            ("neoforge", "NeoForge"),
                            ("quilt", "Quilt"),
                        ],
                    );
                    ui.label("Game version");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.filters.version)
                            .desired_width(90.0)
                            .char_limit(40),
                    );
                    search |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
            }
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.filters.q)
                        .hint_text("Search provider mods…")
                        .desired_width((ui.available_width() - 115.0).max(100.0))
                        .char_limit(120),
                );
                search |= ui.button("Search").clicked()
                    || r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
        });
        let filter_changed = before.provider != self.filters.provider
            || before.game != self.filters.game
            || before.order != self.filters.order
            || before.category != self.filters.category
            || before.loader != self.filters.loader
            || before.content_type != self.filters.content_type;
        if (search || filter_changed || !self.loaded) && self.job.is_none() {
            self.filters.page = 1;
            self.browse(ui.ctx());
        }
        ui.collapsing("Import a project link", |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.link)
                        .hint_text("Or paste a Modrinth, CurseForge or Thunderstore project link")
                        .desired_width((ui.available_width() - 150.0).max(120.0))
                        .char_limit(1000),
                );
                if ui
                    .add_enabled(
                        self.job.is_none() && !self.link.trim().is_empty(),
                        egui::Button::new("Look up link"),
                    )
                    .clicked()
                {
                    self.start(
                        ui.ctx(),
                        Kind::Preview,
                        reqwest::Method::POST,
                        api("mods/external/preview"),
                        Some(json!({"url":self.link.trim()})),
                    );
                }
            });
        });
        ui.label("Browsing loads metadata only. Choose a release to download and subscribe.");
        self.feedback(ui);
        self.pagination(ui, false);
        let mut selected = None;
        egui::ScrollArea::vertical()
            .id_salt("native-provider-results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for item in self.page["items"].as_array().into_iter().flatten() {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(29, 39, 33))
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                artwork(ui, item);
                                ui.heading(text(item, "name"));
                            });
                            credits(ui, item);
                            let description = text(item, "description");
                            let preview: String = description.chars().take(180).collect();
                            ui.label(format!(
                                "{}{}",
                                preview,
                                if description.chars().count() > 180 {
                                    "…"
                                } else {
                                    ""
                                }
                            ));
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!("{} downloads", item["downloads"]));
                                if !item["rating"].is_null() {
                                    ui.label(format!(
                                        "{} {}",
                                        item["rating"],
                                        text(item, "rating_label")
                                    ));
                                }
                                if ui
                                    .add_enabled(
                                        self.job.is_none(),
                                        egui::Button::new("Details / choose release"),
                                    )
                                    .clicked()
                                {
                                    selected = Some(item.clone());
                                }
                            });
                        });
                    ui.add_space(8.0);
                }
                if self.page["items"].as_array().is_some_and(Vec::is_empty) {
                    ui.label("No matching projects. Try another filter.");
                }
            });
        if let Some(item) = selected {
            self.start(
                ui.ctx(),
                Kind::Preview,
                reqwest::Method::POST,
                api("mods/external/preview"),
                Some(json!({"url":item["source_url"]})),
            );
        }
    }
    fn attachment(
        &mut self,
        ui: &mut egui::Ui,
        catalog: &[crate::model::GameInfo],
        source: Option<&crate::cache::Source>,
        packs: &mut crate::pack_ui::PackUi,
        target: &mut Option<String>,
    ) {
        // Native target selection uses the existing pack compatibility/dependency checks.
        if !self.imported_id.is_empty()
            && let Some((game, item)) = catalog.iter().find_map(|g| {
                g.mods
                    .iter()
                    .find(|m| m.file == format!("Mods/{}.zip", self.imported_id))
                    .map(|m| (g, m))
            })
        {
            if game.app_id == u32::MAX {
                let target = crate::minecraft::content_target_ui(
                    ui,
                    item,
                    &mut self.instance_target,
                    &mut self.world_target,
                    self.job.is_none(),
                );
                if let Some((instance, world)) = target {
                    self.install_content(ui.ctx(), instance, world, game.clone(), item.clone());
                }
                return;
            }
            packs.provider_target(ui, target);
            if ui
                .add_enabled(
                    target.is_some(),
                    egui::Button::new(format!("Add {} to selected modpack", item.name)),
                )
                .clicked()
            {
                self.status = packs
                    .provider_add(
                        target.as_deref().unwrap_or_default(),
                        game,
                        source,
                        item.clone(),
                    )
                    .unwrap_or_else(|e| e.to_string());
            }
        }
    }
    fn install_content(
        &mut self,
        ctx: &egui::Context,
        instance: crate::minecraft::Instance,
        world: String,
        game: crate::model::GameInfo,
        item: crate::model::ModInfo,
    ) {
        let token = crate::website::session();
        let session = token.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.job = Some(Job {
            kind: Kind::Install,
            rx,
            session,
        });
        self.status = "Installing compatible Minecraft content and dependencies…".into();
        std::thread::spawn(move || {
            let result =
                crate::minecraft::install_catalog_content(&instance, &world, &game, &item, &token)
                    .map(|message| json!({"message":message}))
                    .map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn feedback(&self, ui: &mut egui::Ui) {
        if !self.status.is_empty() {
            ui.horizontal_wrapped(|ui| {
                if self.job.is_some() {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
        }
    }
    fn pagination(&mut self, ui: &mut egui::Ui, subscriptions: bool) {
        let page = if subscriptions {
            self.subscription_page
        } else {
            self.filters.page
        };
        let data = if subscriptions {
            &self.subscriptions
        } else {
            &self.page
        };
        let more = data["has_more"] == true;
        let mut next = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    page > 1 && self.job.is_none(),
                    egui::Button::new("Previous"),
                )
                .clicked()
            {
                next = Some(page - 1);
            }
            ui.label(format!("Page {page}"));
            if ui
                .add_enabled(more && self.job.is_none(), egui::Button::new("Next"))
                .clicked()
            {
                next = Some(page + 1);
            }
        });
        if let Some(page) = next {
            if subscriptions {
                self.subscription_page = page;
                self.subs(ui.ctx());
            } else {
                self.filters.page = page;
                self.browse(ui.ctx());
            }
        }
    }
    fn show_subscriptions(&mut self, ui: &mut egui::Ui) {
        ui.heading("Subscriptions");
        ui.label("Approved updates appear here. Your installed modpacks keep their pinned versions until you update them.");
        let mut reload = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.subscription_query)
                    .hint_text("Search subscriptions…")
                    .char_limit(120),
            );
            reload = ui
                .add_enabled(self.job.is_none(), egui::Button::new("Refresh"))
                .clicked()
                || r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });
        if (reload
            || !self.subscriptions_loaded
            || self.last_subscriptions.elapsed() > Duration::from_secs(30))
            && self.job.is_none()
        {
            if reload {
                self.subscription_page = 1;
            }
            self.subs(ui.ctx());
        }
        self.feedback(ui);
        self.pagination(ui, true);
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("native-subscriptions")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for item in self.subscriptions["items"].as_array().into_iter().flatten() {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(29, 39, 33))
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                artwork(ui, item);
                                ui.heading(text(item, "name"));
                            });
                            ui.label(format!(
                                "{} · {}",
                                text(item, "provider"),
                                text(item, "version")
                            ));
                            credits(ui, item);
                            ui.label(if item["approved"] == true {
                                "Ready to download"
                            } else {
                                "Awaiting analysis or review"
                            });
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        self.job.is_none() && item["approved"] == true,
                                        egui::Button::new("Download"),
                                    )
                                    .clicked()
                                {
                                    action = Some((
                                        Kind::Download,
                                        reqwest::Method::POST,
                                        api("download-tickets"),
                                        json!({"kind":"mods","id":item["id"]}),
                                    ));
                                }
                                if ui
                                    .add_enabled(
                                        self.job.is_none(),
                                        egui::Button::new("Unsubscribe"),
                                    )
                                    .clicked()
                                {
                                    action = Some((
                                        Kind::Remove,
                                        reqwest::Method::DELETE,
                                        api("mods/subscriptions"),
                                        json!({"source":item["source"]}),
                                    ));
                                }
                            });
                        });
                    ui.add_space(8.0);
                }
                if self.subscriptions["items"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
                {
                    ui.label("Download a provider project to subscribe to it.");
                }
            });
        if let Some((kind, method, url, body)) = action {
            if matches!(kind, Kind::Download) {
                self.imported_id = text(&body, "id").into();
            }
            self.start(ui.ctx(), kind, method, url, Some(body));
        }
    }
    pub fn modal(&mut self, ctx: &egui::Context) {
        let Some(project) = self.preview.clone() else {
            return;
        };
        let mut close = false;
        let mut import = false;
        let modal = egui::Modal::new(egui::Id::new("provider-release-modal"))
            .backdrop_color(egui::Color32::from_black_alpha(185))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 80.0).clamp(260.0, 760.0));
                ui.horizontal(|ui| {
                    ui.heading(text(&project, "name"));
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("provider-modal-body")
                    .max_height((ctx.content_rect().height() - 160.0).max(120.0))
                    .show(ui, |ui| {
                        credits(ui, &project);
                        egui::ScrollArea::vertical()
                            .id_salt("provider-description")
                            .max_height((ctx.content_rect().height() - 290.0).clamp(120.0, 360.0))
                            .show(ui, |ui| {
                                ui.label(text(&project, "description"));
                            });
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            egui::ComboBox::from_id_salt("provider-release")
                                .selected_text(
                                    project["versions"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .find(|v| text(v, "id") == self.release)
                                        .map(|v| text(v, "name"))
                                        .unwrap_or("No compatible release"),
                                )
                                .show_ui(ui, |ui| {
                                    for v in
                                        project["versions"].as_array().into_iter().flatten().filter(
                                            |v| {
                                                compatible(
                                                    v,
                                                    &self.filters.loader,
                                                    &self.filters.version,
                                                )
                                            },
                                        )
                                    {
                                        ui.selectable_value(
                                            &mut self.release,
                                            text(v, "id").into(),
                                            format!(
                                                "{} · {} · {}",
                                                text(v, "name"),
                                                v["game_versions"],
                                                v["loaders"]
                                            ),
                                        );
                                    }
                                });
                            if let Some(v) = project["versions"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .find(|v| text(v, "id") == self.release)
                            {
                                ui.label(format!(
                                    "Game versions: {}\nLoaders: {}",
                                    v["game_versions"], v["loaders"]
                                ));
                                ui.label(format!("Dependencies: {}", v["dependencies"]));
                            }
                            ui.checkbox(&mut self.optional, "Include optional dependencies");
                            import = ui
                                .add_enabled(
                                    !self.release.is_empty(),
                                    egui::Button::new("Download & subscribe"),
                                )
                                .clicked();
                        });
                        self.feedback(ui);
                    });
            });
        #[cfg(test)]
        {
            self.modal_rect = Some(modal.response.rect);
        }
        if close || modal.should_close() {
            self.preview = None;
        }
        if import {
            self.start(ctx,Kind::Import,reqwest::Method::POST,api("mods/external/import"),Some(json!({"url":project["source_url"],"version":self.release,"loader":self.filters.loader,"game_version":self.filters.version,"include_optional":self.optional})));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_encode_search_and_never_request_archives() {
        let f = Filters {
            q: "rain & snow".into(),
            loader: "fabric".into(),
            page: 3,
            ..Default::default()
        };
        let u = f.url().unwrap();
        assert_eq!(u.path(), "/api/v1/providers/search");
        let q: std::collections::HashMap<_, _> = u.query_pairs().collect();
        assert_eq!(q["q"], "rain & snow");
        assert_eq!(q["page"], "3");
        assert_eq!(q["loader"], "fabric");
        assert!(!u.as_str().contains("import"));
    }
    #[test]
    fn release_selection_requires_both_game_version_and_loader() {
        let v = json!({"loaders":["fabric"],"game_versions":["1.21.1"]});
        assert!(compatible(&v, "fabric", "1.21.1"));
        assert!(!compatible(&v, "forge", "1.21.1"));
        assert!(!compatible(&v, "fabric", "1.20.1"));
        assert!(compatible(&v, "", ""));
    }
    #[test]
    fn native_browser_and_modal_fit_without_implicit_downloads() {
        for size in [egui::vec2(1240.0, 800.0), egui::vec2(800.0, 600.0)] {
            let ctx = egui::Context::default();
            let mut browser = Browser::default();
            browser.preview_fixture();
            let mut packs = crate::pack_ui::PackUi::new();
            let mut target = None;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        browser.show(ui, &[], None, &mut packs, &mut target)
                    });
                },
            );
            assert!(browser.job.is_none());
            assert!(output.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("Native browser fixture"))));
            browser.preview = Some(
                json!({"name":"Details fixture","description":"Long description ".repeat(300),"versions":[{"id":"release","name":"1.0","loaders":["fabric"],"game_versions":["1.21.1"],"dependencies":[]}]}),
            );
            browser.release = "release".into();
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| browser.modal(ctx),
                );
            }
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| browser.modal(ctx),
            );
            assert!(browser.job.is_none());
            assert!(out.shapes.iter().any(|s|matches!(&s.shape,egui::Shape::Text(t) if t.galley.text().contains("Details fixture"))));
            let rect = browser.modal_rect.unwrap();
            assert!(
                rect.min.x >= 0.0
                    && rect.max.x <= size.x
                    && rect.min.y >= 0.0
                    && rect.max.y <= size.y,
                "Modal outside viewport: {rect:?} / {size:?}"
            );
        }
    }
    #[test]
    fn account_switch_drops_private_results_and_inflight_responses() {
        let ctx = egui::Context::default();
        let mut b = Browser::default();
        b.preview_fixture();
        b.subscriptions = json!({"items":[{"name":"Private fixture"}]});
        let (tx, rx) = mpsc::channel();
        b.job = Some(Job {
            kind: Kind::Subscriptions,
            rx,
            session: "ui-fixture".into(),
        });
        tx.send(Ok(json!({"items":[{"name":"Old account"}]})))
            .unwrap();
        assert!(!b.update(&ctx, "different-account"));
        assert!(b.page.is_null());
        assert!(b.subscriptions.is_null());
        assert!(b.job.is_none());
        assert!(!b.subscriptions_loaded);
    }
}

#[cfg(debug_assertions)]
pub fn live_check() -> Result<()> {
    let token = crate::website::session();
    let c = client()?;
    for provider in ["modrinth", "curseforge"] {
        let f = Filters {
            provider: provider.into(),
            content_type: "mod".into(),
            ..Default::default()
        };
        let data = request(&c, &token, reqwest::Method::GET, f.url()?, None)?;
        println!(
            "{provider}: {} metadata results; more={}",
            data["items"].as_array().map(Vec::len).unwrap_or(0),
            data["has_more"]
        );
    }
    let data = request(
        &c,
        &token,
        reqwest::Method::GET,
        api("mods/subscriptions?page=1"),
        None,
    )?;
    println!(
        "Private subscriptions: {} results",
        data["items"].as_array().map(Vec::len).unwrap_or(0)
    );
    Ok(())
}
