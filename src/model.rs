use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    pub owner: String,
    pub repository: String,
    pub branch: String,
    #[serde(default)]
    pub catalog_folder: String,
    pub steam_path: String,
    #[serde(default)]
    pub low_end: bool,
    #[serde(default)]
    pub rebound_enabled: bool,
}
impl Settings {
    pub fn path() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("CannaModManager")
            .join("settings.json")
    }
    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| Self {
                owner: "fortnitegamerboy2020".into(),
                repository: "manager-uploaded-mods".into(),
                branch: "main".into(),
                ..Self::default()
            })
    }
    pub fn save(&self) -> anyhow::Result<()> {
        let p = Self::path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModInfo {
    #[serde(default)]
    pub provenance: serde_json::Value,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub description: String,
    pub file: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub local_file: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}
fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    #[test]
    fn rebound_is_opt_in_for_old_and_new_settings() {
        let old: super::Settings = serde_json::from_str(
            r#"{"owner":"canna","repository":"server","branch":"main","steam_path":""}"#,
        )
        .unwrap();
        assert!(!old.rebound_enabled);
        assert!(!super::Settings::default().rebound_enabled);
        let mut enabled = old;
        enabled.rebound_enabled = true;
        let saved = serde_json::to_vec(&enabled).unwrap();
        assert!(
            serde_json::from_slice::<super::Settings>(&saved)
                .unwrap()
                .rebound_enabled
        );
    }
    #[test]
    fn registry_separates_source_from_unity() {
        assert_eq!(super::framework(1557740), "bepinex");
        assert_eq!(super::framework(550), "source-vpk");
        assert!(super::source_addons(1172470).is_none());
        assert!(
            super::supported_catalog()
                .iter()
                .any(|g| g.app_id == 1557740)
        );
    }
    #[test]
    fn existing_mod_manifests_default_to_enabled() {
        let item: super::ModInfo = serde_json::from_str(
            r#"{"name":"Existing mod","version":"1.0.0","file":"Mods/example.zip"}"#,
        )
        .unwrap();
        assert!(item.enabled);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameInfo {
    pub app_id: u32,
    pub name: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub mods: Vec<ModInfo>,
    #[serde(default)]
    pub mod_folder_status: String,
}
pub fn bopl() -> GameInfo {
    GameInfo { app_id: 1686940, name: "Bopl Battle".into(), folder: "bopl-battle".into(), description: "Your family's first supported game. Connect your Canna account in Settings to browse the server library.".into(), icon: String::new(), mods: vec![], mod_folder_status: String::new() }
}
#[derive(Clone, Debug)]
pub struct InstalledGame {
    pub app_id: u32,
    pub name: String,
    pub path: PathBuf,
    pub loader: String,
    pub plugins: usize,
    pub icon: Option<Vec<u8>>,
}
#[derive(Default)]
pub struct Scan {
    pub games: Vec<InstalledGame>,
    pub excluded: usize,
    pub libraries: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

/// Explicit Source addon support, separate from Unity injection.
pub fn source_addons(app_id: u32) -> Option<&'static str> {
    match app_id {
        550 => Some("left4dead2/addons"),
        500 => Some("left4dead/addons"),
        _ => None,
    }
}
pub fn framework(app_id: u32) -> &'static str {
    if app_id == u32::MAX {
        "minecraft"
    } else if source_addons(app_id).is_some() {
        "source-vpk"
    } else {
        "bepinex"
    }
}
pub fn framework_label(app_id: u32) -> &'static str {
    if app_id == u32::MAX {
        "Minecraft instances"
    } else if source_addons(app_id).is_some() {
        "Source / VPK addons"
    } else {
        "BepInEx / Unity"
    }
}
pub fn supported_game(app_id: u32) -> bool {
    crate::game_profiles::supports_game(app_id)
}
pub fn supported_catalog() -> Vec<GameInfo> {
    let mut games = vec![bopl()];
    for (id, name, folder, description) in [
        (
            1557740,
            "ROUNDS",
            "rounds",
            "Unity modpacks with BepInEx 5 and Thunderstore dependencies.",
        ),
        (
            550,
            "Left 4 Dead 2",
            "left-4-dead-2",
            "VPK addon modpacks and separate -insecure practice launches. Native speedrunning plugins are not installed automatically.",
        ),
        (
            500,
            "Left 4 Dead",
            "left-4-dead",
            "VPK addon modpacks and separate -insecure practice launches.",
        ),
    ] {
        games.push(GameInfo {
            app_id: id,
            name: name.into(),
            folder: folder.into(),
            description: description.into(),
            icon: String::new(),
            mods: vec![],
            mod_folder_status: String::new(),
        });
    }
    for profile in crate::game_profiles::games() {
        if games.iter().any(|g| g.app_id == profile.app_id) {
            continue;
        }
        games.push(GameInfo {app_id:profile.app_id,name:profile.name.clone(),folder:profile.folder.clone(),description:"Thunderstore BepInEx profile · preview. Requires a compatible reviewed loader and supported package layout.".into(),icon:String::new(),mods:vec![],mod_folder_status:"Game profile available".into()});
    }
    games
}
