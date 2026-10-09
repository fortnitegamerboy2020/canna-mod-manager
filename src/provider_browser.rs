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
const CATALOG_PAGE_SIZE: usize = 20;
const MINECRAFT_VERSION_MANIFEST: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Clone, Debug, PartialEq)]
struct GameVersion {
    id: String,
    kind: String,
}

fn minecraft_versions(data: &Value) -> Result<Vec<GameVersion>> {
    let rows = data["versions"]
        .as_array()
        .context("Official version list is missing")?;
    ensure!(rows.len() <= 5000, "Official version list is oversized");
    let mut seen = std::collections::HashSet::new();
    let versions: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let id = row["id"].as_str()?;
            let kind = row["type"].as_str()?;
            (id.len() <= 40
                && !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
                && matches!(kind, "release" | "snapshot" | "old_beta" | "old_alpha")
                && seen.insert(id.to_owned()))
            .then(|| GameVersion {
                id: id.into(),
                kind: kind.into(),
            })
        })
        .collect();
    ensure!(!versions.is_empty(), "Official version list is empty");
    Ok(versions)
}

fn fetch_minecraft_versions() -> Result<Vec<GameVersion>> {
    let response = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?
        .get(MINECRAFT_VERSION_MANIFEST)
        .send()?
        .error_for_status()?;
    let mut bytes = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Official version list is oversized"
    );
    minecraft_versions(&serde_json::from_slice(&bytes)?)
}
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

struct CatalogRow<'a> {
    game: &'a crate::model::GameInfo,
    item: &'a crate::model::ModInfo,
    id: &'a str,
}
struct CatalogPage<'a> {
    rows: Vec<CatalogRow<'a>>,
    total: usize,
    has_more: bool,
}
#[derive(Clone)]
struct CatalogPin {
    game: crate::model::GameInfo,
    item: crate::model::ModInfo,
    source: Option<crate::cache::Source>,
}
impl CatalogPin {
    fn matches(
        &self,
        game: &crate::model::GameInfo,
        item: &crate::model::ModInfo,
        source: Option<&crate::cache::Source>,
    ) -> bool {
        self.source.as_ref() == source
            && self.game.app_id == game.app_id
            && self.game.folder == game.folder
            && self.item.file == item.file
            && self.item.version == item.version
            && self.item.sha256.eq_ignore_ascii_case(&item.sha256)
    }
}
fn catalog_id(item: &crate::model::ModInfo) -> Option<&str> {
    if !item.local_file.is_empty()
        || item.provenance["external_only"] == true
        || item.sha256.len() != 64
        || !item.sha256.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    let id = item.file.strip_prefix("Mods/")?.strip_suffix(".zip")?;
    (id.len() == 36
        && id.bytes().enumerate().all(|(index, c)| {
            if [8, 13, 18, 23].contains(&index) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        }))
    .then_some(id)
}
fn catalog_page<'a>(filters: &Filters, catalog: &'a [crate::model::GameInfo]) -> CatalogPage<'a> {
    let app_id = if filters.game == "minecraft" {
        Some(u32::MAX)
    } else {
        crate::game_profiles::by_community(&filters.game).map(|g| g.app_id)
    };
    let query = filters.q.trim().to_lowercase();
    let mut rows = Vec::new();
    if matches!(filters.provider.as_str(), "all" | "canna") && filters.category.is_empty() {
        for game in catalog.iter().filter(|g| Some(g.app_id) == app_id) {
            for item in &game.mods {
                let Some(id) = catalog_id(item) else { continue };
                if !query.is_empty()
                    && !format!("{} {}", item.name, item.description)
                        .to_lowercase()
                        .contains(&query)
                {
                    continue;
                }
                if !filters.content_type.is_empty() && item.content_type != filters.content_type {
                    continue;
                }
                if [
                    ("loaders", &filters.loader),
                    ("game_versions", &filters.version),
                ]
                .iter()
                .any(|(key, requested)| {
                    !requested.is_empty()
                        && !item.provenance[*key].as_array().is_some_and(|values| {
                            values
                                .iter()
                                .any(|value| value.as_str() == Some(requested.as_str()))
                        })
                }) {
                    continue;
                }
                rows.push(CatalogRow { game, item, id });
            }
        }
    }
    rows.sort_by(|a, b| {
        a.item
            .name
            .to_lowercase()
            .cmp(&b.item.name.to_lowercase())
            .then_with(|| a.item.version.cmp(&b.item.version))
            .then_with(|| a.id.cmp(b.id))
    });
    let total = rows.len();
    let offset = (filters.page.saturating_sub(1) as usize).saturating_mul(CATALOG_PAGE_SIZE);
    let has_more = total > offset.saturating_add(CATALOG_PAGE_SIZE);
    let rows = rows
        .into_iter()
        .skip(offset)
        .take(CATALOG_PAGE_SIZE)
        .collect();
    CatalogPage {
        rows,
        total,
        has_more,
    }
}
fn catalog_download_request(row: &CatalogRow<'_>) -> (Kind, reqwest::Url, Value) {
    (
        Kind::Download,
        api("download-tickets"),
        json!({"kind":"mods","id":row.id}),
    )
}
#[derive(Clone, Copy)]
enum Kind {
    Games,
    Browse,
    Preview,
    Subscriptions,
    Import,
    Status,
    Download,
    Remove,
    Install,
}
struct Job {
    kind: Kind,
    rx: Receiver<Result<Value, String>>,
    session: String,
    // A result belongs to the filters captured when the request started.
    filters: Option<Filters>,
}
#[derive(Clone)]
struct PendingDownload {
    id: String,
    pack: Option<String>,
    paused: bool,
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
    game_versions: Vec<GameVersion>,
    versions_job: Option<Receiver<Result<Vec<GameVersion>, String>>>,
    versions_attempted: bool,
    versions_error: String,
    version_search: String,
    page_filters: Option<Filters>,
    provider_games: Option<Value>,
    games_attempted: bool,
    #[cfg(test)]
    modal_rect: Option<egui::Rect>,
    #[cfg(test)]
    game_control: Option<(egui::Id, egui::Rect)>,
    #[cfg(test)]
    game_search_rect: Option<egui::Rect>,
    #[cfg(test)]
    version_control: Option<(egui::Id, egui::Rect)>,
    #[cfg(test)]
    version_search_rect: Option<egui::Rect>,
    imported_id: String,
    pending_pack: Option<String>,
    pending_downloads: std::collections::VecDeque<PendingDownload>,
    last_pending_poll: std::time::Instant,
    status_id: String,
    queued_pack_additions: Vec<(String, String)>,
    catalog_pins: std::collections::BTreeMap<String, CatalogPin>,
    steam_version: Option<crate::steam::SteamVersion>,
    checked_game: String,
    checked_at: std::time::Instant,
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
            game_versions: Vec::new(),
            versions_job: None,
            versions_attempted: cfg!(test),
            versions_error: String::new(),
            version_search: String::new(),
            page_filters: None,
            provider_games: None,
            games_attempted: false,
            #[cfg(test)]
            modal_rect: None,
            #[cfg(test)]
            game_control: None,
            #[cfg(test)]
            game_search_rect: None,
            #[cfg(test)]
            version_control: None,
            #[cfg(test)]
            version_search_rect: None,
            imported_id: String::new(),
            pending_pack: None,
            pending_downloads: Default::default(),
            last_pending_poll: std::time::Instant::now(),
            status_id: String::new(),
            queued_pack_additions: Vec::new(),
            catalog_pins: Default::default(),
            steam_version: None,
            checked_game: String::new(),
            checked_at: std::time::Instant::now(),
        }
    }
}
pub(crate) fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(180))
        .build()?)
}
fn download_button() -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new("Download").color(egui::Color32::from_rgb(16, 28, 19)))
        .fill(egui::Color32::from_rgb(160, 215, 133))
}
fn branch_compatible(item: &Value, installed: Option<&crate::steam::SteamVersion>) -> bool {
    let Some(v) = installed else {
        return true;
    };
    [
        ("steam_branches", v.branch.as_str()),
        ("steam_build_ids", v.build.as_str()),
    ]
    .iter()
    .all(|(key, value)| {
        item[*key]
            .as_array()
            .is_none_or(|a| a.is_empty() || a.iter().any(|x| x.as_str() == Some(value)))
    })
}
pub(crate) fn request(
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
    if fields.get("provider").is_some_and(|s| s == "canna") {
        return Ok(Vec::new());
    }
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
pub(crate) fn api(path: &str) -> reqwest::Url {
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
    ui.allocate_ui_with_layout(
        egui::vec2(190.0, ui.spacing().interact_size.y),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::ComboBox::from_id_salt(id)
                .width(190.0)
                .height(340.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
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
                        let choices = choices
                            .iter()
                            .map(|(v, name)| ((*v).to_owned(), (*name).to_owned()))
                            .collect::<Vec<_>>();
                        crate::ui_helpers::searchable_options(ui, value, &choices);
                    });
                });
        },
    );
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
    fn curseforge_supports(&self, community: &str) -> bool {
        community == "minecraft"
            || self
                .provider_games
                .as_ref()
                .and_then(|data| data["games"].as_array())
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|g| g["community"] == community && g["curseforge"] == true)
                })
    }
    pub fn preview_fixture(&mut self) {
        self.account = "ui-fixture".into();
        self.loaded = true;
        self.versions_attempted = true;
        self.game_versions = vec![GameVersion {
            id: "1.21.1".into(),
            kind: "release".into(),
        }];
        self.page_filters = Some(self.filters.clone());
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
        self.page_filters = None;
    }
    pub fn observe_installations(&mut self, games: &[crate::model::InstalledGame]) {
        if self.checked_game == self.filters.game
            && self.checked_at.elapsed() < Duration::from_secs(30)
        {
            return;
        }
        self.checked_game = self.filters.game.clone();
        self.checked_at = std::time::Instant::now();
        self.steam_version = crate::game_profiles::by_community(&self.filters.game)
            .and_then(|p| games.iter().find(|g| g.app_id == p.app_id))
            .and_then(crate::steam::installed_version);
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
        if !matches!(kind, Kind::Status) {
            self.status = match kind {
                Kind::Import => "Retrieving archive and dependencies; running server review…",
                Kind::Download => "Downloading verified archive…",
                _ => "Loading…",
            }
            .into();
        }
        self.job = Some(Job {
            kind,
            rx,
            session,
            filters: matches!(kind, Kind::Browse).then(|| self.filters.clone()),
        });
        std::thread::spawn(move || {
            let result=(|| {
                let request_id=body.as_ref().and_then(|b|b["id"].as_str()).unwrap_or_default().to_owned();
                let client=client()?;
                let mut data=if matches!(kind,Kind::Browse) {browse_metadata(&client,&token,&url)?}else{request(&client,&token,method,url,body)?};
                if matches!(kind,Kind::Browse) { thumbnails(&mut data); }
                if matches!(kind,Kind::Import) && data["approved"]==true {
                    let ticket=request(&client,&token,reqwest::Method::POST,api("download-tickets"),Some(json!({"kind":"mods","id":data["id"]})))?;
                    // Import and subscription succeeded even if the local transfer fails.
                    match crate::website::receive_ticket(text(&ticket,"ticket")) {Ok(message)=>{data["download_message"]=json!(message);data["downloaded"]=json!(true);},Err(e)=>data["download_message"]=json!(format!("Subscribed, but local download failed: {e}. Retry in Subscriptions."))}
                } else if matches!(kind,Kind::Download) {
                    let message=crate::website::receive_ticket(text(&data,"ticket"))?;data["download_message"]=json!(message);data["id"]=json!(request_id);
                }
                Ok(data)
            })().map_err(|e:anyhow::Error|e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub fn update(&mut self, ctx: &egui::Context, account: &str) -> bool {
        if let Some(result) = self
            .versions_job
            .as_ref()
            .and_then(|job| job.try_recv().ok())
        {
            self.versions_job = None;
            match result {
                Ok(versions) => {
                    self.game_versions = versions;
                    self.versions_error.clear();
                }
                Err(error) => self.versions_error = error,
            }
        }
        if account != self.account {
            self.account = account.to_owned();
            clear_artwork(ctx, &self.page);
            self.page = Value::Null;
            self.page_filters = None;
            self.subscriptions = Value::Null;
            self.preview = None;
            self.imported_id.clear();
            self.pending_pack = None;
            self.pending_downloads.clear();
            self.queued_pack_additions.clear();
            self.catalog_pins.clear();
            self.loaded = false;
            self.provider_games = None;
            self.games_attempted = false;
            self.subscriptions_loaded = false;
            self.status.clear();
        }
        let result = self.job.as_ref().and_then(|j| j.rx.try_recv().ok());
        if let Some(result) = result {
            let job = self.job.take().unwrap();
            if job.session != self.account {
                return false;
            }
            if matches!(job.kind, Kind::Browse)
                && job
                    .filters
                    .as_ref()
                    .is_some_and(|filters| filters != &self.filters)
            {
                self.loaded = false;
                clear_artwork(ctx, &self.page);
                self.page = Value::Null;
                self.page_filters = None;
                self.status.clear();
                return false;
            }
            match result {
                Err(e) => {
                    if matches!(job.kind, Kind::Import) {
                        self.pending_pack = None;
                    }
                    if matches!(job.kind, Kind::Download) {
                        if self.catalog_pins.remove(&self.imported_id).is_some() {
                            self.pending_downloads.retain(|p| p.id != self.imported_id);
                            self.imported_id.clear();
                        } else {
                            for waiting in &mut self.pending_downloads {
                                if waiting.id == self.imported_id {
                                    waiting.paused = true;
                                }
                            }
                        }
                    }
                    if matches!(job.kind, Kind::Status) && e.contains("Mod no longer available") {
                        self.pending_downloads.retain(|p| p.id != self.status_id);
                    }
                    self.status = e;
                }
                Ok(data) => {
                    self.status.clear();
                    match job.kind {
                        Kind::Games => {
                            if let Some(error) = data["curseforge_error"].as_str() {
                                self.status = error.into();
                            }
                            self.provider_games = Some(data);
                            if !self.curseforge_supports(&self.filters.game) {
                                self.filters.game = "minecraft".into();
                            }
                        }
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
                            self.page_filters = Some(self.filters.clone());
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
                            let pack = self.pending_pack.take();
                            if data["downloaded"] == true {
                                if let Some(pack) = pack {
                                    self.queued_pack_additions
                                        .push((pack, self.imported_id.clone()));
                                }
                            } else {
                                self.pending_downloads.retain(|p| p.id != self.imported_id);
                                self.pending_downloads.push_back(PendingDownload {
                                    id: self.imported_id.clone(),
                                    pack,
                                    paused: data["approved"] == true,
                                });
                            }
                            self.subscriptions_loaded = false;
                            self.changed = true;
                            self.status = if data["approved"] == true {
                                text(&data, "download_message").into()
                            } else {
                                "Waiting for server analysis. The download will continue when approved; keep Canna open.".into()
                            };
                        }
                        Kind::Status => {
                            let id = text(&data, "id").to_owned();
                            self.status =
                                format!("{}: {}", text(&data, "name"), text(&data, "message"));
                            if data["state"] == "ready"
                                && self
                                    .pending_downloads
                                    .iter()
                                    .any(|p| p.id == id && !p.paused)
                            {
                                self.imported_id = id.clone();
                                self.start(
                                    ctx,
                                    Kind::Download,
                                    reqwest::Method::POST,
                                    api("download-tickets"),
                                    Some(json!({"kind":"mods","id":id})),
                                );
                            } else if data["state"] == "denied" {
                                self.pending_downloads.retain(|p| p.id != id);
                            }
                        }
                        Kind::Download => {
                            let id = text(&data, "id");
                            if let Some(index) =
                                self.pending_downloads.iter().position(|p| p.id == id)
                                && let Some(pack) =
                                    self.pending_downloads.remove(index).and_then(|p| p.pack)
                            {
                                self.queued_pack_additions.push((pack, id.to_owned()));
                            }
                            self.changed = true;
                            self.subscriptions_loaded = false;
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
        if self.pending_downloads.iter().any(|p| !p.paused) {
            ctx.request_repaint_after(Duration::from_secs(1));
            if self.job.is_none()
                && self.last_pending_poll.elapsed() >= Duration::from_secs(3)
                && let Some(index) = self.pending_downloads.iter().position(|p| !p.paused)
            {
                let pending = self.pending_downloads.remove(index).unwrap();
                let id = pending.id.clone();
                self.pending_downloads.push_back(pending);
                self.last_pending_poll = std::time::Instant::now();
                self.status_id = id.clone();
                self.start(
                    ctx,
                    Kind::Status,
                    reqwest::Method::GET,
                    api(&format!("mods/{id}/status")),
                    None,
                );
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if self.versions_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        std::mem::take(&mut self.changed)
    }
    fn browse(&mut self, ctx: &egui::Context) {
        if self.job.is_some() {
            return;
        }
        self.loaded = true;
        clear_artwork(ctx, &self.page);
        self.page = Value::Null;
        self.page_filters = None;
        if self.filters.provider == "canna" {
            self.page_filters = Some(self.filters.clone());
            self.status.clear();
            return;
        }
        match self.filters.url() {
            Ok(url) => self.start(ctx, Kind::Browse, reqwest::Method::GET, url, None),
            Err(e) => self.status = e.to_string(),
        }
    }
    fn load_game_versions(&mut self, ctx: &egui::Context) {
        if self.versions_job.is_some() {
            return;
        }
        self.versions_attempted = true;
        self.versions_error.clear();
        let (tx, rx) = mpsc::channel();
        self.versions_job = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = fetch_minecraft_versions().map_err(|error| error.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    fn results_count(&self, catalog: &[crate::model::GameInfo]) -> String {
        if self.page_filters.as_ref() != Some(&self.filters) {
            return "Results: searching…".into();
        }
        let library = catalog_page(&self.filters, catalog);
        let count = library.rows.len() + self.page["items"].as_array().map_or(0, Vec::len);
        format!("{count} results on this page")
    }
    fn version_picker(&mut self, ui: &mut egui::Ui) {
        let control = egui::ComboBox::from_id_salt("provider-game-version")
            .width(170.0)
            .height(360.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .selected_text(if self.filters.version.is_empty() {
                "All versions"
            } else {
                &self.filters.version
            })
            .show_ui(ui, |ui| {
                ui.set_min_width(250.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.version_search)
                        .hint_text("Search versions…")
                        .char_limit(40)
                        .desired_width(250.0),
                );
                #[cfg(test)]
                {
                    self.version_search_rect = Some(search.rect);
                }
                #[cfg(not(test))]
                let _ = search;
                if ui
                    .selectable_value(&mut self.filters.version, String::new(), "All versions")
                    .clicked()
                {
                    ui.close();
                }
                let query = self.version_search.trim().to_lowercase();
                let mut found = false;
                for version in &self.game_versions {
                    if !version.id.to_lowercase().contains(&query) {
                        continue;
                    }
                    found = true;
                    let label = if version.kind == "release" {
                        version.id.clone()
                    } else {
                        format!("{} · {}", version.id, version.kind.replace('_', " "))
                    };
                    if ui
                        .selectable_value(&mut self.filters.version, version.id.clone(), label)
                        .clicked()
                    {
                        ui.close();
                    }
                }
                if self.versions_job.is_some() {
                    ui.label("Loading official versions…");
                } else if !self.versions_error.is_empty() {
                    ui.label("Official version list unavailable.");
                    if ui.button("Retry version list").clicked() {
                        self.load_game_versions(ui.ctx());
                    }
                } else if !found {
                    ui.label("No matching versions");
                }
            });
        #[cfg(test)]
        {
            self.version_control = Some((control.response.id, control.response.rect));
        }
        #[cfg(not(test))]
        let _ = control;
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
        if self.mode == 0 && self.filters.provider == "curseforge" && self.provider_games.is_none()
        {
            ui.heading("Supported CurseForge games");
            ui.label("Checking games available through Canna’s server API…");
            if !self.status.is_empty() {
                ui.label(&self.status);
            }
            if !self.games_attempted
                || ui
                    .add_enabled(self.job.is_none(), egui::Button::new("Retry game list"))
                    .clicked()
            {
                self.games_attempted = true;
                self.start(
                    ui.ctx(),
                    Kind::Games,
                    reqwest::Method::GET,
                    api("providers/games"),
                    None,
                );
            }
            return;
        }
        self.attachment(ui, catalog, source, packs, target);
        if self.mode == 2 {
            self.show_subscriptions(ui);
            return;
        }
        ui.heading("Browse mods");
        if self.filters.game == "minecraft" && !self.versions_attempted {
            self.load_game_versions(ui.ctx());
        }
        if let Some(v) = &self.steam_version {
            ui.label(format!(
                "Installed Steam branch: {} · Build: {}",
                v.branch,
                if v.build.is_empty() {
                    "unknown"
                } else {
                    &v.build
                }
            ));
            ui.label("Unlabelled mods have unknown branch compatibility. Steam build IDs are not game release version numbers.");
        }
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
                            ("canna", "Canna"),
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
                    ui.allocate_ui_with_layout(
                        egui::vec2(250.0, ui.spacing().interact_size.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            let game_control = egui::ComboBox::from_id_salt("provider-game")
                                .width(240.0)
                                .height(380.0)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .truncate()
                                .selected_text(label)
                                .show_ui(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.set_min_width(300.0);
                                        let mut choices = Vec::new();
                                        if self.filters.provider != "thunderstore" {
                                            choices.push(("minecraft".into(), "Minecraft".into()));
                                        }
                                        if self.filters.provider != "modrinth" {
                                            choices.extend(
                                                crate::game_profiles::games()
                                                    .iter()
                                                    .filter(|g| {
                                                        self.filters.provider != "curseforge"
                                                            || self
                                                                .curseforge_supports(&g.community)
                                                    })
                                                    .map(|g| (g.community.clone(), g.name.clone())),
                                            );
                                        }
                                        let search = crate::ui_helpers::filter_options(
                                            ui,
                                            &mut self.filters.game,
                                            &mut self.game_search,
                                            &choices,
                                        );
                                        #[cfg(test)]
                                        {
                                            self.game_search_rect = Some(search.rect);
                                        }
                                        #[cfg(not(test))]
                                        let _ = search;
                                    });
                                });
                            #[cfg(test)]
                            {
                                self.game_control =
                                    Some((game_control.response.id, game_control.response.rect));
                            }
                            #[cfg(not(test))]
                            let _ = game_control;
                        },
                    );
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
                    ui.allocate_ui_with_layout(
                        egui::vec2(220.0, ui.spacing().interact_size.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            egui::ComboBox::from_id_salt("provider-category")
                                .width(210.0)
                                .height(340.0)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .truncate()
                                .selected_text(if self.filters.category.is_empty() {
                                    "All categories"
                                } else {
                                    &self.filters.category
                                })
                                .show_ui(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.set_min_width(280.0);
                                        let mut choices =
                                            vec![(String::new(), "All categories".into())];
                                        choices.extend(
                                            self.page["categories"]
                                                .as_array()
                                                .into_iter()
                                                .flatten()
                                                .map(|c| {
                                                    (
                                                        c["id"]
                                                            .as_str()
                                                            .map(str::to_owned)
                                                            .unwrap_or_else(|| c["id"].to_string()),
                                                        text(c, "name").to_owned(),
                                                    )
                                                }),
                                        );
                                        crate::ui_helpers::searchable_options(
                                            ui,
                                            &mut self.filters.category,
                                            &choices,
                                        );
                                    });
                                });
                        },
                    );
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
                        self.version_picker(ui);
                        ui.label(self.results_count(catalog))
                            .on_hover_text("Visible provider listings plus approved Canna archives on this page. Sources can list the same project separately.");
                    },
                );
            }
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.filters.q)
                        .hint_text("Search mods…")
                        .desired_width((ui.available_width() - 115.0).max(100.0))
                        .char_limit(120),
                );
                search |= ui.button("Search").clicked()
                    || r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
        });
        if self.filters.provider == "curseforge"
            && self.provider_games.is_some()
            && !self.curseforge_supports(&self.filters.game)
        {
            self.filters.game = "minecraft".into();
        }
        let filter_changed = before.provider != self.filters.provider
            || before.game != self.filters.game
            || before.order != self.filters.order
            || before.category != self.filters.category
            || before.loader != self.filters.loader
            || before.version != self.filters.version
            || before.content_type != self.filters.content_type;
        if (search || filter_changed || !self.loaded) && self.job.is_none() {
            self.filters.page = 1;
            if self.filters.provider == "curseforge" && self.provider_games.is_none() {
                self.games_attempted = true;
                self.start(
                    ui.ctx(),
                    Kind::Games,
                    reqwest::Method::GET,
                    api("providers/games"),
                    None,
                );
            } else {
                self.browse(ui.ctx());
            }
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
        ui.label("Browsing loads metadata only. Canna library downloads use the approved pinned archive; provider downloads retrieve a release and subscribe.");
        self.feedback(ui);
        let library = catalog_page(&self.filters, catalog);
        self.pagination(ui, false, library.has_more);
        let mut selected = None;
        let mut download = None;
        let mut library_details = None;
        let mut library_download = None;
        egui::ScrollArea::vertical()
            .id_salt("native-provider-results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !library.rows.is_empty() {
                    ui.heading("Canna library");
                    ui.label(format!("{} matching approved archives · alphabetical", library.total));
                }
                for row in &library.rows {
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(29, 39, 33))
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                crate::ui_helpers::mod_art(ui, row.item, egui::vec2(64.0, 64.0));
                                ui.heading(&row.item.name);
                            });
                            ui.label(format!("Canna library · {}", row.item.version));
                            credits(ui, &row.item.provenance);
                            let description: String = row.item.description.chars().take(180).collect();
                            ui.label(format!("{}{}", description, if row.item.description.chars().count()>180 {"…"}else{""}));
                            let compatible_target = target.as_deref().filter(|id| packs.catalog_pack_matches(id, row.game, source));
                            if target.is_some() && compatible_target.is_none() {
                                ui.label(format!("Choose a {} modpack using this library to add this download.", row.game.name));
                            }
                            ui.horizontal_wrapped(|ui| {
                                if ui.add_enabled(self.job.is_none(), download_button()).clicked() {
                                    library_download = Some((row.id.to_owned(), compatible_target.map(str::to_owned), catalog_download_request(row), CatalogPin { game: row.game.clone(), item: row.item.clone(), source: source.cloned() }));
                                }
                                if ui.button("Details").clicked() {
                                    library_details = Some((row.game.clone(), row.item.clone()));
                                }
                            });
                        });
                    ui.add_space(8.0);
                }
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
                                    .add_enabled(self.job.is_none() && branch_compatible(item,self.steam_version.as_ref()), download_button())
                                    .clicked()
                                {
                                    download = Some(item.clone());
                                }
                                if !branch_compatible(item,self.steam_version.as_ref()){ui.label("Declared requirements do not match the installed Steam branch/build.");}
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
                if library.rows.is_empty() && (self.filters.provider=="canna" || self.page["items"].as_array().is_some_and(Vec::is_empty)) {
                    ui.label("No matching projects. Try another filter.");
                }
            });
        if let Some((game, item)) = library_details {
            packs.open_catalog_details(&game, source, target.as_deref(), item);
        }
        if let Some((id, pack, (kind, url, body), pin)) = library_download {
            self.imported_id = id.clone();
            self.catalog_pins.insert(id.clone(), pin);
            self.pending_downloads.retain(|p| p.id != id);
            self.pending_downloads.push_back(PendingDownload {
                id: id.clone(),
                pack,
                paused: true,
            });
            self.start(ui.ctx(), kind, reqwest::Method::POST, url, Some(body));
        }
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
        // Capture a Canna card's version/hash instead of silently adding a release
        // from a later catalog refresh after its download finishes.
        self.queued_pack_additions.retain(|(pack, id)| {
            let Some((game, item)) = catalog.iter().find_map(|g| {
                g.mods
                    .iter()
                    .find(|m| m.file == format!("Mods/{id}.zip"))
                    .map(|m| (g, m))
            }) else {
                if self.catalog_pins.remove(id).is_some() {
                    self.status = "Downloaded, but this Canna archive is no longer in the library. Refresh library to continue.".into();
                    if self.imported_id == *id {
                        self.imported_id.clear();
                    }
                    return false;
                }
                return true;
            };
            self.status = if let Some(pin) = self.catalog_pins.remove(id) {
                if pin.matches(game, item, source) {
                    packs.provider_add(pack, &pin.game, pin.source.as_ref(), pin.item)
                        .unwrap_or_else(|e| format!("Downloaded, but could not add to modpack: {e}"))
                } else {
                    "Downloaded, but the Canna library changed. Refresh library and choose the current release.".into()
                }
            } else {
                packs.provider_add(pack, game, source, item.clone())
                    .unwrap_or_else(|e| format!("Downloaded, but could not add to modpack: {e}"))
            };
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
            let finished = self.job.is_none()
                && !self
                    .pending_downloads
                    .iter()
                    .any(|p| p.id == self.imported_id);
            if !finished {
                return;
            }
            if self
                .catalog_pins
                .get(&self.imported_id)
                .is_some_and(|pin| !pin.matches(game, item, source))
            {
                ui.label(
                    "The Canna library changed. Refresh library and choose the current release.",
                );
                return;
            }
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
                    target
                        .as_deref()
                        .is_some_and(|id| packs.catalog_pack_matches(id, game, source)),
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
            if target
                .as_deref()
                .is_some_and(|id| !packs.catalog_pack_matches(id, game, source))
            {
                ui.label(format!(
                    "Choose a {} modpack using this library to add this download.",
                    game.name
                ));
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
            filters: None,
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
    fn pagination(&mut self, ui: &mut egui::Ui, subscriptions: bool, catalog_more: bool) {
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
        let more = data["has_more"] == true || (!subscriptions && catalog_more);
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
        self.pagination(ui, true, false);
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
                                item["download_status"]["message"]
                                    .as_str()
                                    .unwrap_or("Awaiting analysis or review")
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
                                    self.pending_downloads.retain(|p| p.id != text(item, "id"));
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
                                .width(300.0)
                                .height(340.0)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
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
                                    let choices = project["versions"]
                                        .as_array()
                                        .into_iter()
                                        .flatten()
                                        .filter(|v| {
                                            compatible(
                                                v,
                                                &self.filters.loader,
                                                &self.filters.version,
                                            )
                                        })
                                        .map(|v| {
                                            (
                                                text(v, "id").to_owned(),
                                                format!(
                                                    "{} · {} · {}",
                                                    text(v, "name"),
                                                    v["game_versions"],
                                                    v["loaders"]
                                                ),
                                            )
                                        })
                                        .collect::<Vec<_>>();
                                    crate::ui_helpers::searchable_options(
                                        ui,
                                        &mut self.release,
                                        &choices,
                                    );
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
                                    !self.release.is_empty()
                                        && branch_compatible(&project, self.steam_version.as_ref()),
                                    download_button(),
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
#[cfg(debug_assertions)]
pub fn live_check() -> Result<()> {
    let token = crate::website::session();
    let c = client()?;
    let history = request(
        &c,
        &token,
        reqwest::Method::GET,
        api("invites?sort=most&page=1"),
        None,
    )?;
    println!(
        "Owner invitation history: {} entries on page, {} total (codes omitted)",
        history["items"].as_array().map(Vec::len).unwrap_or(0),
        history["total"]
    );
    let mods = request(&c, &token, reqwest::Method::GET, api("mods"), None)?;
    for m in mods
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| matches!(text(m, "name"), "ArrowWall" | "BiggerLazerPush"))
    {
        println!(
            "{}: creator={}, uploader={}",
            m["name"], m["author"], m["uploader"]
        );
        ensure!(
            m["author"] != m["uploader"],
            "Original creator attribution missing"
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn library_item(number: u32, name: &str) -> crate::model::ModInfo {
        crate::model::ModInfo {
            name: name.into(),
            version: "1.8.2".into(),
            description: "Reviewed Canna archive fixture".into(),
            content_type: "mod".into(),
            provenance: json!({"provider":"canna","authors":["flofl"],"source_url":"https://thunderstore.io/c/rounds/p/flofl/HollowPurple/"}),
            enabled: true,
            file: format!("Mods/00000000-0000-4000-8000-{number:012x}.zip"),
            sha256: "a".repeat(64),
            local_file: String::new(),
            dependencies: vec![],
        }
    }

    fn rounds_library(items: Vec<crate::model::ModInfo>) -> crate::model::GameInfo {
        let mut game = crate::model::bopl();
        game.app_id = 1557740;
        game.name = "ROUNDS".into();
        game.folder = "rounds".into();
        game.mods = items;
        game
    }

    fn library_source() -> crate::cache::Source {
        crate::cache::Source {
            owner: "canna".into(),
            repository: "server".into(),
            branch: "main".into(),
            catalog_folder: String::new(),
        }
    }

    #[test]
    fn canna_and_all_sources_show_fixed_separately_from_the_original_credit_url() {
        let fixed = library_item(1, "HollowPurple Fixed");
        let mut original = library_item(2, "HollowPurple");
        original.provenance["provider"] = json!("thunderstore");
        original.version = "1.8.1".into();
        let catalog = vec![rounds_library(vec![fixed.clone(), original])];
        let mut filters = Filters {
            game: "rounds".into(),
            q: "hOLLOWpURPLE".into(),
            ..Default::default()
        };
        for provider in ["all", "canna"] {
            filters.provider = provider.into();
            let page = catalog_page(&filters, &catalog);
            assert_eq!(page.total, 2);
            assert_ne!(page.rows[0].id, page.rows[1].id);
            assert_eq!(
                page.rows[0].item.provenance["source_url"],
                page.rows[1].item.provenance["source_url"]
            );
            let row = page
                .rows
                .iter()
                .find(|row| row.item.name == "HollowPurple Fixed")
                .unwrap();
            let (kind, url, body) = catalog_download_request(row);
            assert!(matches!(kind, Kind::Download));
            assert_eq!(url.path(), "/api/v1/download-tickets");
            assert_eq!(
                body,
                json!({"kind":"mods","id":"00000000-0000-4000-8000-000000000001"})
            );
            assert_eq!(row.item.version, fixed.version);
            assert!(!body.to_string().contains("thunderstore"));
        }
        filters.provider = "thunderstore".into();
        assert_eq!(catalog_page(&filters, &catalog).total, 0);
        filters.provider = "canna".into();
        assert!(
            provider_requests(&filters.url().unwrap())
                .unwrap()
                .is_empty()
        );
        let mut browser = Browser {
            filters,
            page: json!({"items":[{"name":"Old external page"}]}),
            ..Default::default()
        };
        browser.browse(&egui::Context::default());
        assert!(browser.loaded);
        assert!(browser.page.is_null());
        assert!(
            browser.job.is_none(),
            "Canna browsing must not request an external provider"
        );
    }

    #[test]
    fn library_filters_use_known_game_and_content_metadata_without_inventing_categories() {
        let mut known = library_item(1, "HollowPurple Fixed");
        known.provenance["loaders"] = json!(["bepinex"]);
        known.provenance["game_versions"] = json!(["current"]);
        let mut pack = library_item(2, "HollowPurple maps");
        pack.content_type = "resourcepack".into();
        let mut bopl = crate::model::bopl();
        bopl.mods = vec![library_item(3, "HollowPurple Bopl")];
        let catalog = vec![rounds_library(vec![known, pack]), bopl];
        let mut filters = Filters {
            provider: "canna".into(),
            game: "rounds".into(),
            q: "HollowPurple".into(),
            ..Default::default()
        };
        assert_eq!(catalog_page(&filters, &catalog).total, 2);
        filters.content_type = "mod".into();
        filters.loader = "bepinex".into();
        filters.version = "current".into();
        assert_eq!(catalog_page(&filters, &catalog).total, 1);
        filters.version = "unknown".into();
        assert_eq!(catalog_page(&filters, &catalog).total, 0);
        filters.version.clear();
        filters.loader = "fabric".into();
        assert_eq!(catalog_page(&filters, &catalog).total, 0);
        filters.loader.clear();
        filters.category = "thunderstore:cards".into();
        assert_eq!(catalog_page(&filters, &catalog).total, 0);
        filters.category.clear();
        filters.game = "bopl-battle".into();
        let page = catalog_page(&filters, &catalog);
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].game.app_id, 1686940);
        filters.game = "unknown-game".into();
        assert_eq!(catalog_page(&filters, &catalog).total, 0);
    }

    #[test]
    fn recommendation_local_and_unverified_archives_never_become_canna_downloads() {
        let valid = library_item(1, "Reviewed archive");
        let mut items = vec![valid.clone()];
        let mut recommendation = valid.clone();
        recommendation.provenance["external_only"] = json!(true);
        items.push(recommendation);
        let mut missing_hash = valid.clone();
        missing_hash.sha256.clear();
        items.push(missing_hash);
        let mut bad_hash = valid.clone();
        bad_hash.sha256 = "x".repeat(64);
        items.push(bad_hash);
        let mut local = valid.clone();
        local.local_file = "local-mods/test.zip".into();
        items.push(local);
        let mut bad_archive = valid.clone();
        bad_archive.file = "Mods/../original.zip".into();
        items.push(bad_archive);
        let mut executable = valid.clone();
        executable.file = "Mods/00000000-0000-4000-8000-000000000001.dll".into();
        items.push(executable);
        let catalog = vec![rounds_library(items)];
        let page = catalog_page(
            &Filters {
                game: "rounds".into(),
                ..Default::default()
            },
            &catalog,
        );
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].item.file, valid.file);
    }

    #[test]
    fn canna_pages_remain_alphabetical_and_independent_of_external_provider_failures() {
        let items = (0..CATALOG_PAGE_SIZE + 1)
            .rev()
            .map(|n| library_item(n as u32, &format!("Mod {n:02}")))
            .collect();
        let catalog = vec![rounds_library(items)];
        let mut filters = Filters {
            game: "rounds".into(),
            order: "updated".into(),
            ..Default::default()
        };
        assert!(
            merge_pages(
                vec![("thunderstore".into(), Err("unavailable".into()))],
                &filters.order
            )
            .is_err()
        );
        let first = catalog_page(&filters, &catalog);
        assert_eq!(first.total, CATALOG_PAGE_SIZE + 1);
        assert_eq!(first.rows.len(), CATALOG_PAGE_SIZE);
        assert_eq!(first.rows[0].item.name, "Mod 00");
        assert!(first.has_more);
        filters.page = 2;
        let second = catalog_page(&filters, &catalog);
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.rows[0].item.name, "Mod 20");
        assert!(!second.has_more);
        filters.page = u32::MAX;
        assert!(catalog_page(&filters, &catalog).rows.is_empty());
    }

    #[test]
    fn canna_cards_render_at_desktop_and_compact_sizes_during_provider_outages() {
        let catalog = vec![rounds_library(vec![library_item(1, "HollowPurple Fixed")])];
        for size in [egui::vec2(1280.0, 800.0), egui::vec2(800.0, 600.0)] {
            for provider in ["all", "canna"] {
                let ctx = egui::Context::default();
                let mut browser = Browser {
                    filters: Filters {
                        provider: provider.into(),
                        game: "rounds".into(),
                        q: "HollowPurple".into(),
                        ..Default::default()
                    },
                    account: "ui-fixture".into(),
                    loaded: true,
                    status: "Provider temporarily unavailable".into(),
                    ..Default::default()
                };
                let mut packs = crate::pack_ui::PackUi::new();
                let mut target = None;
                let input = || egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                };
                for _ in 0..3 {
                    let _ = ctx.run(input(), |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            browser.show(ui, &catalog, None, &mut packs, &mut target)
                        });
                    });
                }
                let output = ctx.run(input(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        browser.show(ui, &catalog, None, &mut packs, &mut target)
                    });
                });
                assert!(browser.job.is_none());
                assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "HollowPurple Fixed" && t.pos.y >= s.clip_rect.min.y && t.pos.y + 18.0 <= s.clip_rect.max.y)), "Canna result not visible at {size:?} / {provider}");
                assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().contains("null downloads"))));
            }
        }
    }

    #[test]
    fn pinned_canna_download_uses_captured_pack_and_rejects_a_changed_catalog() {
        let root = std::env::temp_dir().join(format!(
            "canna-pinned-download-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::modpacks::with_test_root(root.clone(), || {
            let item = library_item(1, "HollowPurple Fixed");
            let game = rounds_library(vec![item.clone()]);
            let source = library_source();
            // Pack names are arbitrary: the game metadata determines compatibility.
            let first = crate::modpacks::Modpack::create(
                "Bopl".into(),
                String::new(),
                &game,
                source.clone(),
                vec![],
            );
            let stale = crate::modpacks::Modpack::create(
                "Stale download".into(),
                String::new(),
                &game,
                source.clone(),
                vec![],
            );
            let wrong = crate::modpacks::Modpack::create(
                "ROUNDS".into(),
                String::new(),
                &crate::model::bopl(),
                source.clone(),
                vec![],
            );
            for pack in [&first, &stale, &wrong] {
                pack.save().unwrap();
            }
            let mut packs = crate::pack_ui::PackUi::new();
            assert!(packs.catalog_pack_matches(&first.id, &game, Some(&source)));
            assert!(!packs.catalog_pack_matches(&wrong.id, &game, Some(&source)));
            let ctx = egui::Context::default();
            let id = catalog_id(&item).unwrap().to_owned();
            let deliver = |browser: &mut Browser, pack: &str| {
                browser.imported_id = id.clone();
                browser.catalog_pins.insert(
                    id.clone(),
                    CatalogPin {
                        game: game.clone(),
                        item: item.clone(),
                        source: Some(source.clone()),
                    },
                );
                browser.pending_downloads.push_back(PendingDownload {
                    id: id.clone(),
                    pack: Some(pack.into()),
                    paused: true,
                });
                let (tx, rx) = mpsc::channel();
                tx.send(Ok(json!({"id":id,"download_message":"Downloaded"})))
                    .unwrap();
                browser.job = Some(Job {
                    kind: Kind::Download,
                    rx,
                    session: "fixture".into(),
                    filters: None,
                });
                browser.update(&ctx, "fixture");
            };
            let mut browser = Browser {
                account: "fixture".into(),
                ..Default::default()
            };
            deliver(&mut browser, &first.id);
            let mut target = Some(wrong.id.clone());
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    browser.attachment(
                        ui,
                        std::slice::from_ref(&game),
                        Some(&source),
                        &mut packs,
                        &mut target,
                    )
                });
            });
            let saved = crate::modpacks::load_all().0;
            assert_eq!(
                saved.iter().find(|p| p.id == first.id).unwrap().mods[0].sha256,
                item.sha256
            );
            assert!(
                saved
                    .iter()
                    .find(|p| p.id == wrong.id)
                    .unwrap()
                    .mods
                    .is_empty()
            );
            assert!(browser.catalog_pins.is_empty());
            deliver(&mut browser, &stale.id);
            let mut changed = game.clone();
            changed.mods[0].version = "different-version".into();
            changed.mods[0].sha256 = "b".repeat(64);
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    browser.attachment(
                        ui,
                        std::slice::from_ref(&changed),
                        Some(&source),
                        &mut packs,
                        &mut target,
                    )
                });
            });
            assert!(
                crate::modpacks::load_all()
                    .0
                    .iter()
                    .find(|p| p.id == stale.id)
                    .unwrap()
                    .mods
                    .is_empty()
            );
            assert!(browser.status.contains("library changed"));
            assert!(browser.catalog_pins.is_empty());
            assert!(browser.queued_pack_additions.is_empty());
        });
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_canna_download_and_logout_clear_pins_without_enabling_attachment() {
        let ctx = egui::Context::default();
        let item = library_item(1, "HollowPurple Fixed");
        let id = catalog_id(&item).unwrap().to_owned();
        let pin = CatalogPin {
            game: rounds_library(vec![item.clone()]),
            item,
            source: Some(library_source()),
        };
        let mut browser = Browser {
            account: "fixture".into(),
            imported_id: id.clone(),
            ..Default::default()
        };
        browser.catalog_pins.insert(id.clone(), pin.clone());
        browser.pending_downloads.push_back(PendingDownload {
            id: id.clone(),
            pack: Some("captured-pack".into()),
            paused: true,
        });
        let (tx, rx) = mpsc::channel();
        tx.send(Err("Download checksum mismatch".into())).unwrap();
        browser.job = Some(Job {
            kind: Kind::Download,
            rx,
            session: "fixture".into(),
            filters: None,
        });
        browser.update(&ctx, "fixture");
        assert!(browser.catalog_pins.is_empty());
        assert!(browser.pending_downloads.is_empty());
        assert!(browser.queued_pack_additions.is_empty());
        assert!(browser.imported_id.is_empty());
        browser.catalog_pins.insert(id, pin);
        browser.update(&ctx, "");
        assert!(browser.catalog_pins.is_empty());
    }

    #[test]
    #[ignore = "Uses the signed-in account and downloads one already-approved small archive; no installation"]
    fn live_pending_download_resumes_after_ready_status() {
        let session = crate::website::session();
        assert!(!session.is_empty(), "Sign in first");
        let client = client().unwrap();
        let mods = request(&client, &session, reqwest::Method::GET, api("mods"), None).unwrap();
        let item = mods
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| {
                m["review_status"] == "approved"
                    && m["size"].as_u64().unwrap_or(u64::MAX) < 1024 * 1024
            })
            .min_by_key(|m| m["size"].as_u64().unwrap_or(u64::MAX))
            .expect("No small approved archive");
        let id = text(item, "id");
        let status = request(
            &client,
            &session,
            reqwest::Method::GET,
            api(&format!("mods/{id}/status")),
            None,
        )
        .unwrap();
        assert_eq!(status["state"], "ready");
        let mut browser = Browser {
            account: session.clone(),
            ..Default::default()
        };
        browser.pending_downloads.push_back(PendingDownload {
            id: id.into(),
            pack: None,
            paused: false,
        });
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(status)).unwrap();
        browser.job = Some(Job {
            kind: Kind::Status,
            rx,
            session: session.clone(),
            filters: None,
        });
        let ctx = egui::Context::default();
        browser.update(&ctx, &session);
        let start = std::time::Instant::now();
        while browser.job.is_some() && start.elapsed() < Duration::from_secs(90) {
            std::thread::sleep(Duration::from_millis(50));
            browser.update(&ctx, &session);
        }
        assert!(browser.job.is_none(), "Download timed out");
        assert!(browser.pending_downloads.is_empty(), "{}", browser.status);
        assert!(
            browser.status.starts_with("Downloaded "),
            "{}",
            browser.status
        );
    }
    #[test]
    fn waiting_import_retains_pack_until_actual_download_and_logout_clears_it() {
        let ctx = egui::Context::default();
        let mut b = Browser {
            account: "session".into(),
            pending_pack: Some("chosen-pack".into()),
            ..Default::default()
        };
        let deliver = |b: &mut Browser, kind, data| {
            let (tx, rx) = mpsc::channel();
            tx.send(Ok(data)).unwrap();
            b.job = Some(Job {
                kind,
                rx,
                session: "session".into(),
                filters: None,
            });
            b.update(&ctx, "session");
        };
        deliver(
            &mut b,
            Kind::Import,
            json!({"id":"waiting-mod","approved":false}),
        );
        assert!(b.queued_pack_additions.is_empty());
        assert_eq!(b.pending_downloads.len(), 1);
        deliver(
            &mut b,
            Kind::Status,
            json!({"id":"waiting-mod","name":"Mod","state":"needs_review","message":"Staff review required"}),
        );
        assert!(b.status.contains("Staff review"));
        assert_eq!(b.pending_downloads.len(), 1);
        deliver(
            &mut b,
            Kind::Download,
            json!({"id":"waiting-mod","download_message":"Downloaded"}),
        );
        assert_eq!(
            b.queued_pack_additions,
            vec![("chosen-pack".into(), "waiting-mod".into())]
        );
        assert!(b.pending_downloads.is_empty());
        b.pending_downloads.push_back(PendingDownload {
            id: "other".into(),
            pack: None,
            paused: false,
        });
        b.update(&ctx, "");
        assert!(b.pending_downloads.is_empty());
        assert!(b.queued_pack_additions.is_empty());
    }
    #[test]
    fn declared_steam_branches_and_builds_must_match_but_unknowns_remain_unknown() {
        let v = crate::steam::SteamVersion {
            branch: "previous".into(),
            build: "123".into(),
        };
        assert!(branch_compatible(&json!({}), Some(&v)));
        assert!(branch_compatible(
            &json!({"steam_branches":["previous"],"steam_build_ids":["123"]}),
            Some(&v)
        ));
        assert!(!branch_compatible(
            &json!({"steam_branches":["public"]}),
            Some(&v)
        ));
        assert!(!branch_compatible(
            &json!({"steam_build_ids":["124"]}),
            Some(&v)
        ));
    }
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
            filters: None,
        });
        tx.send(Ok(json!({"items":[{"name":"Old account"}]})))
            .unwrap();
        assert!(!b.update(&ctx, "different-account"));
        assert!(b.page.is_null());
        assert!(b.subscriptions.is_null());
        assert!(b.job.is_none());
        assert!(!b.subscriptions_loaded);
    }
    #[test]
    fn official_version_options_preserve_order_and_filter_invalid_metadata() {
        let rows = minecraft_versions(&json!({"versions":[
            {"id":"1.21.1","type":"release"},
            {"id":"24w33a","type":"snapshot"},
            {"id":"1.20.1","type":"release"},
            {"id":"1.21.1","type":"release"},
            {"id":"../unsafe","type":"release"},
            {"id":"unknown","type":"future-type"}
        ]}))
        .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            ["1.21.1", "24w33a", "1.20.1"]
        );
        assert!(minecraft_versions(&json!({"versions":[]})).is_err());
        assert!(minecraft_versions(&json!({})).is_err());
    }
    #[test]
    fn changed_version_discards_stale_provider_results_and_count() {
        let ctx = egui::Context::default();
        let mut browser = Browser::default();
        browser.preview_fixture();
        let previous = browser.filters.clone();
        let (tx, rx) = mpsc::channel();
        browser.job = Some(Job {
            kind: Kind::Browse,
            rx,
            session: "ui-fixture".into(),
            filters: Some(previous),
        });
        browser.filters.version = "1.20.1".into();
        tx.send(Ok(json!({"items":[{"name":"Wrong version"}],"total":900})))
            .unwrap();
        assert!(!browser.update(&ctx, "ui-fixture"));
        assert!(browser.page.is_null());
        assert!(!browser.loaded);
        assert_eq!(browser.results_count(&[]), "Results: searching…");
    }
    #[test]
    fn result_count_labels_visible_page_instead_of_inventing_provider_total() {
        let mut browser = Browser::default();
        browser.preview_fixture();
        browser.filters.game = "rounds".into();
        browser.page = json!({"items":[{"name":"A"},{"name":"B"}],"total":9000,"has_more":true});
        browser.page_filters = Some(browser.filters.clone());
        let catalog = vec![rounds_library(
            (0..25).map(|id| library_item(id, "Card")).collect(),
        )];
        assert_eq!(browser.results_count(&catalog), "22 results on this page");
        browser.filters.page = 2;
        assert_eq!(browser.results_count(&catalog), "Results: searching…");
        browser.page_filters = Some(browser.filters.clone());
        assert_eq!(browser.results_count(&catalog), "7 results on this page");
    }
    #[test]
    fn version_dropdown_search_keeps_menu_open_and_supports_selection_and_reset() {
        let ctx = egui::Context::default();
        let mut browser = Browser::default();
        browser.preview_fixture();
        browser.game_versions = minecraft_versions(&json!({"versions":[
            {"id":"1.21.1","type":"release"},{"id":"1.20.1","type":"release"}
        ]}))
        .unwrap();
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 650.0),
            )),
            events,
            ..Default::default()
        };
        let frame = |browser: &mut Browser, events| {
            ctx.run(input(events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    browser.version_picker(ui);
                });
            })
        };
        let click = |browser: &mut Browser, point: egui::Pos2| {
            for pressed in [true, false] {
                frame(
                    browser,
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                );
            }
        };
        for _ in 0..3 {
            frame(&mut browser, vec![]);
        }
        let (id, rect) = browser.version_control.unwrap();
        click(&mut browser, rect.center());
        for _ in 0..3 {
            frame(&mut browser, vec![]);
        }
        assert!(egui::ComboBox::is_open(&ctx, id));
        let search_point = browser.version_search_rect.unwrap().center();
        click(&mut browser, search_point);
        frame(&mut browser, vec![egui::Event::Text("1.20".into())]);
        assert_eq!(browser.version_search, "1.20");
        assert!(egui::ComboBox::is_open(&ctx, id));
        let output = frame(&mut browser, vec![]);
        let version_point = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "1.20.1" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap();
        click(&mut browser, version_point);
        assert_eq!(browser.filters.version, "1.20.1");
        click(&mut browser, rect.center());
        for _ in 0..3 {
            frame(&mut browser, vec![]);
        }
        let output = frame(&mut browser, vec![]);
        let reset_point = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "All versions" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap();
        click(&mut browser, reset_point);
        assert!(browser.filters.version.is_empty());
        assert!(browser.job.is_none());
    }
}
