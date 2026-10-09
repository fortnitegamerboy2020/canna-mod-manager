use crate::model::{GameInfo, Settings};
use anyhow::{Result, bail};
use std::{collections::BTreeMap, io::Read, time::Duration};
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

fn catalog_icon_path(game: &GameInfo) -> Result<Option<String>> {
    if game.icon.is_empty() {
        return Ok(None);
    }
    let path = format!("{}/{}", game.folder, game.icon);
    anyhow::ensure!(valid_path(&path), "Unsafe catalog path");
    Ok(Some(path))
}

pub(crate) fn fetch_optional(
    client: &reqwest::blocking::Client,
    _settings: &Settings,
    token: &str,
    path: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>> {
    anyhow::ensure!(valid_path(path), "Unsafe catalog path");
    let mut url = reqwest::Url::parse("https://cannamods.vip/api/v1/catalog/file")?;
    url.query_pairs_mut().append_pair("path", path);
    let response = client.get(url).bearer_auth(token).send()?;
    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    if response.status().as_u16() == 401 {
        bail!("Connect your Canna account in Settings. The server session expired or was revoked.");
    }
    if response.status().as_u16() == 403 {
        let mut body = String::new();
        response.take(4096).read_to_string(&mut body)?;
        let details: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let message = details["error"].as_str().unwrap_or(
            "The server refused this file. Its review or account access may need attention.",
        );
        let message: String = message
            .chars()
            .filter(|c| !c.is_control())
            .take(300)
            .collect();
        bail!("Catalog access denied (403) for {path}: {message}");
    }
    let response = response.error_for_status()?;
    let expected = response
        .headers()
        .get("x-canna-sha256")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    response.take(limit + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        "Catalog download exceeds size limit"
    );
    if let Some(hash) = expected {
        use sha2::Digest;
        anyhow::ensure!(
            format!("{:x}", sha2::Sha256::digest(&bytes)) == hash,
            "Server download checksum mismatch"
        );
    }
    Ok(Some(bytes))
}
pub fn sync(settings: &Settings, token: &str) -> Result<RepositoryData> {
    anyhow::ensure!(
        !token.is_empty(),
        "Connect your Canna account in Settings to browse the server library"
    );
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let response = client
        .get("https://cannamods.vip/api/v1/catalog")
        .bearer_auth(token)
        .send()?;
    if response.status().as_u16() == 401 {
        bail!("Canna session expired. Reconnect your account in Settings.");
    }
    let mut bytes = Vec::new();
    response
        .error_for_status()?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 8 * 1024 * 1024, "Catalog too large");
    #[derive(serde::Deserialize)]
    struct Catalog {
        games: Vec<GameInfo>,
    }
    let catalog: Catalog = serde_json::from_slice(&bytes)?;
    let mut data = RepositoryData {
        games: catalog.games,
        icons: BTreeMap::new(),
        warnings: vec![],
        cached_at: None,
    };
    anyhow::ensure!(data.games.len() <= 512, "Too many catalog games");
    for game in &data.games {
        anyhow::ensure!(
            game.app_id > 0
                && valid_path(&game.folder)
                && game.mods.iter().all(|m| valid_mod_file(&m.file)),
            "Invalid server catalog"
        );
        let icon = catalog_icon_path(game).and_then(|path| match path {
            Some(path) => fetch_optional(&client, settings, token, &path, 4 * 1024 * 1024),
            None => Ok(None),
        });
        match icon {
            Ok(Some(icon)) => {
                data.icons.insert(game.app_id, icon);
            }
            Ok(None) => {}
            Err(e) => data.warnings.push(format!("{} artwork: {e}", game.name)),
        }
    }
    Ok(data)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_catalog_artwork_keeps_path_checks() {
        for mut game in crate::model::supported_catalog() {
            assert_eq!(catalog_icon_path(&game).unwrap(), None, "{}", game.name);
            game.icon = "icon.png".into();
            assert_eq!(
                catalog_icon_path(&game).unwrap(),
                Some(format!("{}/icon.png", game.folder))
            );
            for invalid in [
                "../icon.png",
                "/icon.png",
                "https://example.test/icon.png",
                "icons/%2e%2e/icon.png",
                "icons\\icon.png",
            ] {
                game.icon = invalid.into();
                assert!(
                    catalog_icon_path(&game).is_err(),
                    "{}: {invalid}",
                    game.name
                );
            }
        }
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
}
