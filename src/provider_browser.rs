//! Native metadata browser. Provider credentials never leave the Canna server.
use anyhow::{Context, Result, ensure};
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
            provider: "all".into(),
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
    game_search: String,
    #[cfg(test)]
    modal_rect: Option<egui::Rect>,
    #[cfg(test)]
    game_control: Option<(egui::Id, egui::Rect)>,
    #[cfg(test)]
    game_search_rect: Option<egui::Rect>,
    imported_id: String,
    pending_pack: Option<String>,
    queued_pack_additions: Vec<(String, String)>,
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
            game_search: String::new(),
            #[cfg(test)]
            modal_rect: None,
            #[cfg(test)]
            game_control: None,
            #[cfg(test)]
            game_search_rect: None,
            imported_id: String::new(),
            pending_pack: None,
            queued_pack_additions: Vec::new(),
        }
    }
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(180))
        .build()?)
}
fn download_button() -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new("Download").color(egui::Color32::from_rgb(16, 28, 19)))
        .fill(egui::Color32::from_rgb(160, 215, 133))
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
fn provider_requests(url: &reqwest::Url) -> Result<Vec<(String, reqwest::Url)>> {
    let fields: std::collections::BTreeMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let game = fields
        .get("game")
        .map(String::as_str)
        .unwrap_or("minecraft");
    let mut providers = if fields.get("provider").is_some_and(|s| s == "all") {
        if game == "minecraft" {
            vec!["modrinth", "curseforge"]
        } else {
            vec!["thunderstore", "curseforge"]
        }
    } else {
        vec![fields.get("provider").context("Missing provider")?.as_str()]
    };
    let category = fields
        .get("category")
        .map(String::as_str)
        .unwrap_or_default();
    let selection = category.split_once(':');
    if let Some((source, _)) = selection {
        providers.retain(|p| *p == source);
    }
    let mut requests = Vec::new();
    for provider in providers {
        // CurseForge has a fixed index ceiling; other sources can continue paging.
        if provider == "curseforge"
            && fields
                .get("page")
                .and_then(|s| s.parse::<u32>().ok())
                .is_some_and(|p| p > 416)
        {
            continue;
        }
        let mut next = url.clone();
        next.query_pairs_mut().clear();
        for (k, v) in &fields {
            let value = if k == "provider" {
                provider
            } else if k == "category" {
                selection.map(|(_, id)| id).unwrap_or(v)
            } else {
                v
            };
            next.query_pairs_mut().append_pair(k, value);
        }
        requests.push((provider.to_owned(), next));
    }
    Ok(requests)
}
fn merge_pages(pages: Vec<(String, Result<Value, String>)>, order: &str) -> Result<Value> {
    let mut items = Vec::new();
    let mut categories = Vec::new();
    let mut warnings = Vec::new();
    let mut more = false;
    let mut successful = 0;
    for (provider, result) in pages {
        match result {
            Err(error) => warnings.push(format!("{provider}: {error}")),
            Ok(mut data) => {
                successful += 1;
                more |= data["has_more"] == true;
                if data["stale"] == true {
                    warnings.push(format!("{provider}: showing cached metadata"));
                }
                for item in data["items"].as_array_mut().into_iter().flatten() {
                    item["provider"] = json!(provider);
                    items.push(item.clone());
                }
                for c in data["categories"].as_array().into_iter().flatten() {
                    let id = c["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| c["id"].to_string());
                    categories.push(json!({"id":format!("{provider}:{id}"),"name":format!("{} · {provider}",text(c,"name"))}));
                }
            }
        }
    }
    ensure!(successful > 0, "{}", warnings.join("; "));
    match order {
        "downloads" => {
            items.sort_by_key(|v| std::cmp::Reverse(v["downloads"].as_u64().unwrap_or(0)))
        }
        "rating" => items.sort_by_key(|v| std::cmp::Reverse(v["rating"].as_u64().unwrap_or(0))),
        "updated" => items.sort_by(|a, b| text(b, "updated").cmp(text(a, "updated"))),
        // Each provider supplies its own newest order. Interleave instead of inventing dates.
        _ => {}
    }
    Ok(
        json!({"items":items,"categories":categories,"has_more":more,"warnings":warnings,"provider":"all"}),
    )
}
fn browse_metadata(
    client: &reqwest::blocking::Client,
    token: &str,
    url: &reqwest::Url,
) -> Result<Value> {
    let requests = provider_requests(url)?;
    let combined = url
        .query_pairs()
        .any(|(k, v)| k == "provider" && v == "all");
    if !combined {
        let (provider, url) = requests
            .into_iter()
            .next()
            .context("No provider page available")?;
        let mut data = request(client, token, reqwest::Method::GET, url, None)?;
        for item in data["items"].as_array_mut().into_iter().flatten() {
            item["provider"] = json!(provider);
        }
        return Ok(data);
    }
    let pages = std::thread::scope(|scope| {
        let jobs = requests
            .into_iter()
            .map(|(provider, url)| {
                let job = scope.spawn(move || {
                    request(client, token, reqwest::Method::GET, url, None)
                        .map_err(|e| e.to_string())
                });
                (provider, job)
            })
            .collect::<Vec<_>>();
        jobs.into_iter()
            .map(|(provider, job)| {
                (
                    provider,
                    job.join()
                        .unwrap_or_else(|_| Err("Provider request interrupted".into())),
                )
            })
            .collect::<Vec<_>>()
    });
    let order = url
        .query_pairs()
        .find(|(k, _)| k == "order")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    merge_pages(pages, &order)
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
        .width(190.0)
        .height(340.0)
        .truncate()
        .selected_text(
            choices
                .iter()
                .find(|(v, _)| *v == value)
                .map(|(_, name)| *name)
                .unwrap_or(value),
        )
        .show_ui(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_min_width(230.0);
                for (v, name) in choices {
                    ui.selectable_value(value, (*v).into(), *name);
                }
            });
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
        if self
            .job
            .as_ref()
            .is_some_and(|j| matches!(j.kind, Kind::Browse | Kind::Preview))
        {
            self.job = None;
        }
        self.mode = 0;
        if app_id == u32::MAX {
            self.filters.game = "minecraft".into();
            self.filters.provider = "all".into();
        } else if let Some(g) = crate::game_profiles::games()
            .iter()
            .find(|g| g.app_id == app_id)
        {
            self.filters.game = g.community.clone();
            self.filters.provider = "all".into();
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
                let mut data=if matches!(kind,Kind::Browse) {browse_metadata(&client,&token,&url)?}else{request(&client,&token,method,url,body)?};
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
            self.pending_pack = None;
            self.queued_pack_additions.clear();
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
                Err(e) => {
                    if matches!(job.kind, Kind::Import) {
                        self.pending_pack = None;
                    }
                    self.status = e;
                }
                Ok(data) => {
                    self.status.clear();
                    match job.kind {
                        Kind::Browse => {
                            self.status = if let Some(warnings) =
                                data["warnings"].as_array().filter(|w| !w.is_empty())
                            {
                                warnings
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            } else if data["stale"] == true {
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
                            if let Some(pack) = self.pending_pack.take() {
                                self.queued_pack_additions
                                    .push((pack, self.imported_id.clone()));
                            }
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
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            let previous = target.clone();
            ui.horizontal_wrapped(|ui| {
                ui.label("Add downloads to:");
                packs.provider_target(ui, target);
            });
            if previous != *target
                && let Some(game) = target.as_deref().and_then(|id| packs.pack_game(id))
            {
                self.select_game(game);
            }
        });
        let before = self.filters.clone();
        let mut search = false;
        ui.add_enabled_ui(self.job.is_none(), |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
                |ui| {
                    combo(
                        ui,
                        "provider-source",
                        &mut self.filters.provider,
                        &[
                            ("all", "All sources"),
                            ("modrinth", "Modrinth"),
                            ("curseforge", "CurseForge"),
                            ("thunderstore", "Thunderstore"),
                        ],
                    );
                    if self.filters.provider != before.provider {
                        if self.filters.provider == "modrinth" {
                            self.filters.game = "minecraft".into();
                        } else if self.filters.provider == "thunderstore"
                            && self.filters.game == "minecraft"
                        {
                            self.filters.game = "rounds".into();
                        }
                        self.filters.category.clear();
                        self.game_search.clear();
                    }
                    let label = if self.filters.game == "minecraft" {
                        "Minecraft"
                    } else {
                        crate::game_profiles::by_community(&self.filters.game)
                            .map(|g| g.name.as_str())
                            .unwrap_or("Choose game")
                    };
                    let game_control = egui::ComboBox::from_id_salt("provider-game")
                        .width(240.0)
                        .height(380.0)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .truncate()
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_min_width(300.0);
                                let search = ui.add(
                                    egui::TextEdit::singleline(&mut self.game_search)
                                        .hint_text("Find a game…")
                                        .desired_width(280.0)
                                        .char_limit(80),
                                );
                                #[cfg(test)]
                                {
                                    self.game_search_rect = Some(search.rect);
                                }
                                #[cfg(not(test))]
                                let _ = search;
                                let query = self.game_search.to_lowercase();
                                if self.filters.provider != "thunderstore"
                                    && "minecraft".contains(&query)
                                    && ui
                                        .selectable_value(
                                            &mut self.filters.game,
                                            "minecraft".into(),
                                            "Minecraft",
                                        )
                                        .clicked()
                                {
                                    ui.close();
                                }
                                if self.filters.provider != "modrinth" {
                                    for g in crate::game_profiles::games()
                                        .iter()
                                        .filter(|g| g.name.to_lowercase().contains(&query))
                                    {
                                        if ui
                                            .selectable_value(
                                                &mut self.filters.game,
                                                g.community.clone(),
                                                &g.name,
                                            )
                                            .clicked()
                                        {
                                            ui.close();
                                        }
                                    }
                                }
                            });
                        });
                    #[cfg(test)]
                    {
                        self.game_control =
                            Some((game_control.response.id, game_control.response.rect));
                    }
                    #[cfg(not(test))]
                    let _ = game_control;
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
                        .width(210.0)
                        .height(340.0)
                        .truncate()
                        .selected_text(if self.filters.category.is_empty() {
                            "All categories"
                        } else {
                            &self.filters.category
                        })
                        .show_ui(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_min_width(280.0);
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
                                    ui.selectable_value(
                                        &mut self.filters.category,
                                        id,
                                        text(c, "name"),
                                    );
                                }
                            });
                        });
                },
            );
            if self.filters.game != before.game {
                self.filters.category.clear();
                self.filters.loader.clear();
                self.filters.version.clear();
                self.filters.content_type.clear();
            }
            if self.filters.game == "minecraft" {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
                    |ui| {
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
                    },
                );
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
        let mut download = None;
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
                            ui.label(text(item, "provider"));
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
                                if ui
                                    .add_enabled(self.job.is_none(), download_button())
                                    .clicked()
                                {
                                    download = Some(item.clone());
                                }
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
        if download.is_some()
            && self.filters.game == "minecraft"
            && (self.filters.version.is_empty() || self.filters.loader.is_empty())
        {
            selected = download.take();
        }
        if let Some(item) = selected {
            self.pending_pack = target.clone();
            self.imported_id.clear();
            self.start(
                ui.ctx(),
                Kind::Preview,
                reqwest::Method::POST,
                api("mods/external/preview"),
                Some(json!({"url":item["source_url"]})),
            );
        }
        if let Some(item) = download {
            self.pending_pack = target.clone();
            self.imported_id.clear();
            self.start(ui.ctx(), Kind::Import, reqwest::Method::POST, api("mods/external/import"),
                Some(json!({"url":item["source_url"],"loader":self.filters.loader,"game_version":self.filters.version,"include_optional":false})));
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
        self.queued_pack_additions.retain(|(pack, id)| {
            let Some((game, item)) = catalog.iter().find_map(|g| {
                g.mods
                    .iter()
                    .find(|m| m.file == format!("Mods/{id}.zip"))
                    .map(|m| (g, m))
            }) else {
                return true;
            };
            self.status = packs
                .provider_add(pack, game, source, item.clone())
                .unwrap_or_else(|e| format!("Downloaded, but could not add to modpack: {e}"));
            if self.imported_id == *id {
                self.imported_id.clear();
            }
            false
        });
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
                                .add_enabled(!self.release.is_empty(), download_button())
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
            let selected = project["versions"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|v| text(v, "id") == self.release);
            let version = if self.filters.version.is_empty() {
                selected
                    .and_then(|v| v["game_versions"][0].as_str())
                    .unwrap_or_default()
            } else {
                &self.filters.version
            };
            let loader = if self.filters.loader.is_empty() {
                selected
                    .and_then(|v| v["loaders"].as_array())
                    .and_then(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .find(|l| matches!(*l, "forge" | "fabric" | "neoforge" | "quilt"))
                    })
                    .unwrap_or_default()
            } else {
                &self.filters.loader
            };
            self.start(ctx,Kind::Import,reqwest::Method::POST,api("mods/external/import"),Some(json!({"url":project["source_url"],"version":self.release,"loader":loader,"game_version":version,"include_optional":self.optional})));
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
            for _ in 0..3 {
                let _ = ctx.run(
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
            }
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
    fn combined_sources_are_game_scoped_and_category_ids_stay_with_their_provider() {
        let f = Filters::default();
        let requests = provider_requests(&f.url().unwrap()).unwrap();
        assert_eq!(
            requests.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            vec!["modrinth", "curseforge"]
        );
        let f = Filters {
            game: "rounds".into(),
            ..Default::default()
        };
        assert_eq!(
            provider_requests(&f.url().unwrap())
                .unwrap()
                .iter()
                .map(|(p, _)| p.as_str())
                .collect::<Vec<_>>(),
            vec!["thunderstore", "curseforge"]
        );
        let f = Filters {
            category: "curseforge:123".into(),
            ..Default::default()
        };
        let urls = provider_requests(&f.url().unwrap()).unwrap();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].0, "curseforge");
        assert!(
            urls[0]
                .1
                .query_pairs()
                .any(|(k, v)| k == "category" && v == "123")
        );
    }
    #[test]
    fn a_failed_source_does_not_hide_results_or_break_pagination() {
        let a = json!({"items":[{"name":"one","downloads":10}],"categories":[{"id":"performance","name":"Performance"}],"has_more":true});
        let b = json!({"items":[{"name":"two","downloads":20}],"categories":[{"id":12,"name":"Resources"}],"has_more":false});
        let data = merge_pages(
            vec![
                ("modrinth".into(), Ok(a.clone())),
                ("curseforge".into(), Ok(b)),
            ],
            "downloads",
        )
        .unwrap();
        assert_eq!(data["items"][0]["provider"], "curseforge");
        assert_eq!(data["items"][1]["provider"], "modrinth");
        assert_eq!(data["categories"][0]["id"], "modrinth:performance");
        assert_eq!(data["has_more"], true);
        let partial = merge_pages(
            vec![
                ("modrinth".into(), Ok(a)),
                ("curseforge".into(), Err("temporarily unavailable".into())),
            ],
            "downloads",
        )
        .unwrap();
        assert_eq!(partial["items"].as_array().unwrap().len(), 1);
        assert_eq!(partial["warnings"].as_array().unwrap().len(), 1);
        assert!(
            merge_pages(
                vec![("thunderstore".into(), Err("403".into()))],
                "downloads"
            )
            .is_err()
        );
    }
    #[test]
    fn game_dropdown_opens_without_moving_controls_and_shows_a_useful_list() {
        let ctx = egui::Context::default();
        let size = egui::vec2(1240.0, 820.0);
        let mut b = Browser::default();
        b.preview_fixture();
        let mut packs = crate::pack_ui::PackUi::new();
        let mut target = None;
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        for _ in 0..3 {
            let _ = ctx.run(input(vec![]), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
            });
        }
        let (id, before) = b.game_control.unwrap();
        let point = before.center();
        let _ = ctx.run(
            input(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]),
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
            },
        );
        let _ = ctx.run(
            input(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
            },
        );
        assert!(egui::ComboBox::is_open(&ctx, id));
        for _ in 0..3 {
            let _ = ctx.run(input(vec![]), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
            });
        }
        let output = ctx.run(input(vec![]), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
        });
        assert_eq!(b.game_control.unwrap().1, before);
        let visible_games=output.shapes.iter().filter(|s|matches!(&s.shape,egui::Shape::Text(t) if t.pos.y>=s.clip_rect.min.y && t.pos.y+18.0<=s.clip_rect.max.y && crate::game_profiles::games().iter().any(|g|g.name==t.galley.text()))).count();
        assert!(visible_games >= 6, "Only {visible_games} game rows visible");
        let point = b.game_search_rect.unwrap().center();
        let _ = ctx.run(
            input(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]),
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
            },
        );
        let _ = ctx.run(
            input(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]),
            |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
            },
        );
        assert!(
            egui::ComboBox::is_open(&ctx, id),
            "Search focus closed the game menu"
        );
        let _ = ctx.run(input(vec![egui::Event::Text("rounds".into())]), |ctx| {
            egui::CentralPanel::default()
                .show(ctx, |ui| b.show(ui, &[], None, &mut packs, &mut target));
        });
        assert_eq!(b.game_search, "rounds");
        assert!(egui::ComboBox::is_open(&ctx, id));
        assert!(b.job.is_none());
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
    for game in ["bopl-battle", "rounds"] {
        for page in [1, 2] {
            let f = Filters {
                provider: "thunderstore".into(),
                game: game.into(),
                page,
                ..Default::default()
            };
            let data = request(&c, &token, reqwest::Method::GET, f.url()?, None)?;
            let count = data["items"].as_array().map(Vec::len).unwrap_or(0);
            ensure!(count > 0, "No live Thunderstore listings for {game}");
            println!("Thunderstore {game} page {page}: {count} metadata results");
        }
    }
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
    let combined = browse_metadata(&c, &token, &Filters::default().url()?)?;
    println!(
        "All sources: {} metadata results",
        combined["items"].as_array().map(Vec::len).unwrap_or(0)
    );
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
