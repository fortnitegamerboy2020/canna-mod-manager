//! Per-installation runtime storage, outside the Steam game directory.
use crate::model::InstalledGame;
#[cfg(not(test))]
use sha2::{Digest, Sha256};
use std::path::PathBuf;
pub fn root(game: &InstalledGame) -> PathBuf {
    #[cfg(test)]
    {
        game.path.with_file_name(format!(
            "{}-runtime-cache",
            game.path.file_name().unwrap().to_string_lossy()
        ))
    }
    #[cfg(not(test))]
    {
        let identity = format!(
            "{}:{}",
            game.app_id,
            game.path.to_string_lossy().to_lowercase()
        );
        crate::modpacks::directory()
            .parent()
            .unwrap()
            .join("runtime-cache")
            .join(format!("{:x}", Sha256::digest(identity.as_bytes())))
    }
}
