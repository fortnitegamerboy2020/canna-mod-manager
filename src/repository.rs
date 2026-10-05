use crate::model::{GameInfo, Settings};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    time::Duration,
};

#[derive(Deserialize)]
struct Catalog {
    schema_version: u32,
    games: Vec<String>,
}
pub struct RepositoryData {
    pub games: Vec<GameInfo>,
    pub icons: BTreeMap<u32, Vec<u8>>,
    pub warnings: Vec<String>,
    pub cached_at: Option<u64>,
}
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '?', '#', '%'])
        && path.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_. ".contains(c))
        })
}
pub fn valid_mod_file(path: &str) -> bool {
    valid_path(path) && (path.starts_with("Mods/") || path.starts_with("mods/"))
}
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        && s != "."
        && s != ".."
}
fn fetch(
    client: &reqwest::blocking::Client,
    s: &Settings,
    token: &str,
    path: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    fetch_optional(client, s, token, path, limit)?.ok_or_else(|| {
        anyhow::anyhow!("GitHub file not found: {path}. Check branch, path and repository access.")
    })
}
pub(crate) fn fetch_optional(
    client: &reqwest::blocking::Client,
    s: &Settings,
    token: &str,
    path: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>> {
    if !path.is_empty() && !valid_path(path) {
        bail!("Unsafe repository path: {path}")
    }
    let mut url = reqwest::Url::parse("https://api.github.com")?;
    {
        let mut parts = url.path_segments_mut().unwrap();
        parts.extend(["repos", &s.owner, &s.repository, "contents"]);
        if !path.is_empty() {
            parts.extend(path.split('/'));
        }
    }
    url.query_pairs_mut().append_pair("ref", &s.branch);
    let mut request = client
        .get(url)
        .header("Accept", "application/vnd.github.raw+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if !token.trim().is_empty() {
        request = request.bearer_auth(token.trim());
    }
    let response = request.send().context("Cannot reach GitHub")?;
    let status = response.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        bail!(
            "GitHub returned {} for {}. {}",
            status.as_u16(),
            path,
            match status.as_u16() {
                401 => "Token is invalid or expired.",
                403 => "Check repository read access or API rate limits.",
                404 => "Check owner, repository, branch, file, and private repository access.",
                _ => "Try again later.",
            }
        );
    }
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("Repository file exceeds size limit: {path}")
    }
    Ok(Some(bytes))
}
#[derive(Deserialize)]
struct Entry {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}
fn list_folder(
    client: &reqwest::blocking::Client,
    s: &Settings,
    token: &str,
    path: &str,
) -> Result<Option<Vec<Entry>>> {
    fetch_optional(client, s, token, path, 1024 * 1024)?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .with_context(|| format!("Invalid directory listing: {path}"))
        })
        .transpose()
}
pub fn sync(s: &Settings, token: &str) -> Result<RepositoryData> {
    if !valid_slug(&s.owner) || !valid_slug(&s.repository) || s.branch.trim().is_empty() {
        bail!("Enter a GitHub owner, repository, and branch in Settings")
    }
    let prefix = catalog_prefix(&s.catalog_folder)?;
    let client = reqwest::blocking::Client::builder()
        .user_agent("Canna-Mod-Manager/0.1")
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let catalog_file = fetch_optional(
        &client,
        s,
        token,
        &format!("{prefix}catalog.json"),
        1024 * 1024,
    )?;
    let discovered = catalog_file.is_none();
    let catalog: Catalog = match catalog_file {
        Some(bytes) => serde_json::from_slice(&bytes).context("Invalid catalog.json")?,
        None => {
            let listing = list_folder(&client, s, token, &s.catalog_folder)?.ok_or_else(|| {
                anyhow::anyhow!("Repository folder is unavailable. Check repository read access.")
            })?;
            let folders: Vec<String> = listing
                .into_iter()
                .filter(|e| e.kind == "dir" && valid_path(&e.name))
                .map(|e| e.name)
                .collect();
            if folders.len() > 100 {
                bail!("Too many folders to discover; add catalog.json")
            }
            Catalog {
                schema_version: 1,
                games: folders,
            }
        }
    };
    if catalog.schema_version != 1 {
        bail!("Unsupported catalog schema version")
    }
    if catalog.games.len() > 100 {
        bail!("Catalog exceeds 100 games")
    }
    let mut data = RepositoryData {
        games: vec![],
        icons: BTreeMap::new(),
        warnings: vec![],
        cached_at: None,
    };
    let mut ids = BTreeSet::new();
    for folder in catalog.games {
        if !valid_path(&folder) {
            bail!("Invalid game folder in catalog")
        }
        let Some(bytes) = fetch_optional(
            &client,
            s,
            token,
            &format!("{prefix}{folder}/game.json"),
            1024 * 1024,
        )?
        else {
            if discovered {
                continue;
            }
            bail!("Missing {folder}/game.json")
        };
        let mut game: GameInfo = serde_json::from_slice(&bytes)
            .with_context(|| format!("Invalid {folder}/game.json"))?;
        game.folder = folder.clone();
        if game.app_id == 0 || game.name.trim().is_empty() || !ids.insert(game.app_id) {
            bail!("Empty name, invalid or duplicate Steam app ID in {folder}")
        }
        if game.mods.len() > 1000 {
            bail!("Too many mods in {folder}")
        }
        for item in &game.mods {
            if !valid_mod_file(&item.file) {
                bail!("Mod file must be beneath {folder}/Mods/")
            }
        }
        let mods = list_folder(&client, s, token, &format!("{prefix}{folder}/Mods"))?;
        let mods = if mods.is_none() {
            list_folder(&client, s, token, &format!("{prefix}{folder}/mods"))?
        } else {
            mods
        };
        game.mod_folder_status = if mods.is_some() {
            "Mods folder ready"
        } else {
            "Mods folder not uploaded yet — this game can still be used"
        }
        .into();
        if mods.is_none() && !game.mods.is_empty() {
            data.warnings.push(format!(
                "{} lists mods, but its Mods folder is missing",
                game.name
            ));
        }
        if !game.icon.is_empty() {
            match fetch(
                &client,
                s,
                token,
                &format!("{prefix}{folder}/{}", game.icon),
                4 * 1024 * 1024,
            ) {
                Ok(bytes) => {
                    data.icons.insert(game.app_id, bytes);
                }
                Err(e) => data.warnings.push(format!("{} icon: {e}", game.name)),
            }
        }
        data.games.push(game);
    }
    if discovered && data.games.is_empty() {
        data.warnings.push(
            "No game folders with game.json were found; add catalog.json or a game folder.".into(),
        );
    }
    Ok(data)
}
fn catalog_prefix(folder: &str) -> Result<String> {
    if folder.is_empty() {
        return Ok(String::new());
    }
    if !valid_path(folder) {
        bail!("Catalog folder must be a relative repository path, such as games")
    }
    Ok(format!("{folder}/"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Live read-only verification against the family's repository"]
    fn family_repository_read() {
        let settings = Settings {
            owner: "fortnitegamerboy2020".into(),
            repository: "manager-uploaded-mods".into(),
            branch: "main".into(),
            ..Default::default()
        };
        let data = sync(&settings, crate::EMBEDDED_GITHUB_TOKEN).unwrap();
        let game = data
            .games
            .iter()
            .find(|g| g.app_id == 1686940)
            .expect("Bopl Battle should be discovered");
        assert!(!game.mod_folder_status.is_empty());
        assert!(data.icons.contains_key(&1686940));
        println!(
            "{}: {}; {} catalog mods",
            game.name,
            game.mod_folder_status,
            game.mods.len()
        );
    }
    #[test]
    fn catalog_folder_is_relative_and_optional() {
        assert_eq!(
            format!("{}catalog.json", catalog_prefix("").unwrap()),
            "catalog.json"
        );
        assert_eq!(
            format!(
                "{}bopl-battle/game.json",
                catalog_prefix("family/games").unwrap()
            ),
            "family/games/bopl-battle/game.json"
        );
        for folder in ["../games", "/games", "C:\\games", "games//mods"] {
            assert!(catalog_prefix(folder).is_err());
        }
    }
    #[test]
    #[ignore = "Explicit live GitHub API check; requires network access"]
    fn github_public_read() {
        let settings = Settings {
            owner: "emilk".into(),
            repository: "egui".into(),
            branch: "main".into(),
            ..Default::default()
        };
        let client = reqwest::blocking::Client::builder()
            .user_agent("Canna-Mod-Manager-Test/0.1")
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let bytes = fetch(&client, &settings, "", "Cargo.toml", 1024 * 1024).unwrap();
        assert!(String::from_utf8(bytes).unwrap().contains("[workspace]"));
    }
    #[test]
    fn paths_stay_in_repository() {
        for p in [
            "../icon.png",
            "mods/../../x",
            "/abs",
            "mods\\x",
            "https://host/token",
            "a//b",
            "mods/%2e%2e/x",
        ] {
            assert!(!valid_path(p), "{p}");
        }
        assert!(valid_path("bopl-battle/mods/family-pack.zip"));
        assert!(valid_mod_file("Mods/family-pack.zip"));
        assert!(valid_mod_file("mods/legacy-pack.zip"));
        assert!(!valid_mod_file("Mods/../../outside.zip"));
        assert!(!valid_mod_file("Other/family-pack.zip"));
    }
    #[test]
    fn repository_example_is_valid() {
        let c: Catalog =
            serde_json::from_str(include_str!("../repository-template/catalog.json")).unwrap();
        assert_eq!(c.schema_version, 1);
        let g: GameInfo =
            serde_json::from_str(include_str!("../repository-template/bopl-battle/game.json"))
                .unwrap();
        assert_eq!(g.app_id, 1686940);
        assert!(valid_path(&g.icon));
    }
}
