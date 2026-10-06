use crate::{
    model::{GameInfo, Settings},
    repository::RepositoryData,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_CACHE_BYTES: u64 = 80 * 1024 * 1024;
const MAX_ICON_BYTES: usize = 4 * 1024 * 1024;
const MAX_ICON_TOTAL: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub owner: String,
    pub repository: String,
    pub branch: String,
    #[serde(default)]
    pub catalog_folder: String,
}
impl Source {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            owner: settings.owner.to_ascii_lowercase(),
            repository: settings.repository.to_ascii_lowercase(),
            branch: settings.branch.clone(),
            catalog_folder: settings.catalog_folder.clone(),
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    schema_version: u32,
    source: Source,
    saved_at: u64,
    games: Vec<GameInfo>,
    icons: BTreeMap<u32, Vec<u8>>,
}
fn path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CannaModManager")
        .join("catalog-cache.json")
}
pub fn load(source: &Source) -> Result<Option<RepositoryData>> {
    load_from(&path(), source)
}
fn load_from(location: &std::path::Path, source: &Source) -> Result<Option<RepositoryData>> {
    let file = match std::fs::File::open(location) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        bail!("Catalog cache exceeds size limit")
    }
    let snapshot: Snapshot = serde_json::from_slice(&bytes).context("Invalid catalog cache")?;
    if snapshot.schema_version != 1 || &snapshot.source != source {
        return Ok(None);
    }
    if snapshot.games.len() > 100 {
        bail!("Too many games in catalog cache")
    }
    let mut ids = std::collections::BTreeSet::new();
    for game in &snapshot.games {
        if game.app_id == 0
            || game.name.trim().is_empty()
            || !ids.insert(game.app_id)
            || game.mods.len() > 1000
        {
            bail!("Invalid game in catalog cache")
        }
        for item in &game.mods {
            if !crate::repository::valid_mod_file(&item.file) {
                bail!("Invalid mod path in catalog cache")
            }
        }
    }
    let mut total = 0usize;
    for (id, bytes) in &snapshot.icons {
        if !ids.contains(id) || bytes.len() > MAX_ICON_BYTES {
            bail!("Invalid cached icon")
        }
        total += bytes.len();
        if total > MAX_ICON_TOTAL {
            bail!("Cached icons exceed size limit")
        }
    }
    Ok(Some(RepositoryData {
        games: snapshot.games,
        icons: snapshot.icons,
        warnings: vec![],
        cached_at: Some(snapshot.saved_at),
    }))
}
pub fn save(source: &Source, data: &RepositoryData) -> Result<()> {
    save_to(&path(), source, data)
}
fn save_to(destination: &std::path::Path, source: &Source, data: &RepositoryData) -> Result<()> {
    let mut total = 0;
    let mut icons = BTreeMap::new();
    for (&id, bytes) in &data.icons {
        if bytes.len() <= MAX_ICON_BYTES && total + bytes.len() <= MAX_ICON_TOTAL {
            total += bytes.len();
            icons.insert(id, bytes.clone());
        }
    }
    let snapshot = Snapshot {
        schema_version: 1,
        source: source.clone(),
        saved_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        games: data.games.clone(),
        icons,
    };
    let bytes = serde_json::to_vec(&snapshot)?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        bail!("Catalog is too large to cache")
    }
    std::fs::create_dir_all(destination.parent().unwrap())?;
    // Replace only after a complete snapshot has been written; a failed sync never overwrites the cache.
    let temporary = destination.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, destination).context("Could not replace catalog cache")?;
    Ok(())
}
pub fn age_label(saved_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = now.saturating_sub(saved_at);
    if seconds < 60 {
        "less than a minute ago".into()
    } else if seconds < 3600 {
        format!("{} min ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{} h ago", seconds / 3600)
    } else {
        format!("{} days ago", seconds / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_matches_repository_branch_and_folder() {
        let folder = std::env::temp_dir().join(format!(
            "canna-cache-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = folder.join("catalog.json");
        let source = Source {
            owner: "family".into(),
            repository: "manager-uploaded-mods".into(),
            branch: "main".into(),
            catalog_folder: "games".into(),
        };
        let data = RepositoryData {
            games: vec![crate::model::bopl()],
            icons: BTreeMap::new(),
            warnings: vec![],
            cached_at: None,
        };
        save_to(&path, &source, &data).unwrap();
        save_to(&path, &source, &data).unwrap();
        assert_eq!(
            load_from(&path, &source).unwrap().unwrap().games[0].folder,
            "bopl-battle"
        );
        let mut other = source.clone();
        other.catalog_folder.clear();
        assert!(load_from(&path, &other).unwrap().is_none());
        let mut other = source.clone();
        other.branch = "other".into();
        assert!(load_from(&path, &other).unwrap().is_none());
        let mut other = source;
        other.repository = "other".into();
        assert!(load_from(&path, &other).unwrap().is_none());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
