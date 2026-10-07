//! Pinned Thunderstore/r2modman game metadata; see bundled MIT attribution.
#![allow(dead_code)] // The shared schema has different consumers in desktop and server.
use serde::Deserialize;
use std::sync::OnceLock;
#[derive(Deserialize)]
pub struct GameProfile {
    pub app_id: u32,
    pub name: String,
    pub community: String,
    pub folder: String,
    pub data_folder: String,
    pub executables: Vec<String>,
}
#[derive(Deserialize)]
pub struct LoaderProfile {
    pub package: String,
    pub root: String,
}
#[derive(Deserialize)]
struct Registry {
    games: Vec<GameProfile>,
    loaders: Vec<LoaderProfile>,
}
fn registry() -> &'static Registry {
    static DATA: OnceLock<Registry> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../web/thunderstore-games.json"))
            .expect("Pinned game profiles")
    })
}
pub fn games() -> &'static [GameProfile] {
    &registry().games
}
pub fn by_id(id: u32) -> Option<&'static GameProfile> {
    games().iter().find(|g| g.app_id == id)
}
pub fn supports_game(id: u32) -> bool {
    matches!(id, u32::MAX | 1686940 | 1557740 | 550 | 500) || by_id(id).is_some()
}
pub fn by_community(community: &str) -> Option<&'static GameProfile> {
    games().iter().find(|g| g.community == community)
}
pub fn loader(package: &str) -> Option<&'static LoaderProfile> {
    registry().loaders.iter().find(|l| l.package == package)
}
#[cfg(test)]
mod tests {
    #[test]
    fn profiles_are_unique_and_paths_are_relative() {
        let mut ids = std::collections::BTreeSet::new();
        let mut communities = std::collections::BTreeSet::new();
        for g in super::games() {
            assert!(ids.insert(g.app_id));
            assert!(communities.insert(&g.community));
            assert!(!g.data_folder.contains(['/', '\\', ':']));
            assert!(!g.executables.is_empty());
        }
        assert!(super::games().len() > 150);
        assert_eq!(
            super::by_community("lethal-company").unwrap().app_id,
            1966720
        );
        assert!(super::loader("BepInEx-BepInExPack").is_some());
        assert!(super::loader("Impostor-BepInExPack").is_none());
    }
}
