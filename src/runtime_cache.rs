//! Per-installation runtime storage, outside the Steam game directory.
use crate::model::InstalledGame;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
pub fn root(game: &InstalledGame) -> PathBuf {
    #[cfg(test)]
    if std::env::var_os("CANNA_LIVE_GAME_TEST").is_none() {
        return game.path.with_file_name(format!(
            "{}-runtime-cache",
            game.path.file_name().unwrap().to_string_lossy()
        ));
    }
    {
        let identity = format!(
            "{}:{}",
            game.app_id,
            game.path
                .to_string_lossy()
                .replace('/', "\\")
                .to_lowercase()
        );
        crate::modpacks::directory()
            .parent()
            .unwrap()
            .join("runtime-cache")
            .join(format!("{:x}", Sha256::digest(identity.as_bytes())))
    }
}
