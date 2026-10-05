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
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub name: String,
    pub version: String,
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
    GameInfo { app_id: 1686940, name: "Bopl Battle".into(), folder: "bopl-battle".into(), description: "Your family's first supported game. Connect your private GitHub repository to browse its mods.".into(), icon: String::new(), mods: vec![], mod_folder_status: String::new() }
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
