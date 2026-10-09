//! Imported text settings stay in the pack; activation is transactional and launch-only.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub path: String,
    pub contents: String,
}

pub fn safe_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 240 || path.contains(['\\', ':', '\0']) {
        return false;
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > 16
        || parts.iter().any(|p| {
            let device = p.split('.').next().unwrap_or_default().to_ascii_uppercase();
            p.is_empty()
                || *p == "."
                || *p == ".."
                || p.ends_with(['.', ' '])
                || p.chars().any(|c| c.is_control() || "<>\"|?*".contains(c))
                || matches!(
                    device.as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "CONIN$"
                        | "CONOUT$"
                        | "COM\u{b9}"
                        | "COM\u{b2}"
                        | "COM\u{b3}"
                        | "LPT\u{b9}"
                        | "LPT\u{b2}"
                        | "LPT\u{b3}"
                )
                || (device.len() == 4
                    && (device.starts_with("COM") || device.starts_with("LPT"))
                    && device.as_bytes()[3].is_ascii_digit())
        })
    {
        return false;
    }
    Path::new(path).extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_ascii_lowercase().as_str(),
            "cfg" | "toml" | "json" | "ini" | "txt" | "xml" | "yaml" | "yml"
        )
    })
}

pub fn validate(configs: &[Config]) -> Result<()> {
    ensure!(
        configs.len() <= 1000,
        "At most 1000 imported configs are supported"
    );
    let mut names = BTreeSet::new();
    let mut total = 0;
    for config in configs {
        ensure!(
            safe_path(&config.path) && names.insert(config.path.to_lowercase()),
            "Unsafe or duplicate imported config path: {}",
            config.path
        );
        ensure!(
            config.contents.len() <= 256 * 1024 && !config.contents.contains('\0'),
            "Config exceeds text limits: {}",
            config.path
        );
        total += config.contents.len();
        ensure!(
            total <= 1024 * 1024,
            "Imported text configs exceed 1 MiB; export a smaller profile"
        );
    }
    Ok(())
}

pub fn pending(
    game: &crate::model::InstalledGame,
    pack: &crate::modpacks::Modpack,
) -> Result<Vec<Config>> {
    if pack.imported_configs.is_empty() {
        return Ok(vec![]);
    }
    validate(&pack.imported_configs)?;
    // Preserve subsequent edits when relaunching the same imported profile.
    if crate::play_backup::last_applied(game)?
        .is_some_and(|old| old.id == pack.id && old.imported_configs == pack.imported_configs)
    {
        return Ok(vec![]);
    }
    Ok(pack.imported_configs.clone())
}

pub fn merge(mut existing: Vec<(PathBuf, Vec<u8>)>, configs: &[Config]) -> Vec<(PathBuf, Vec<u8>)> {
    existing.retain(|(path, _)| {
        !configs.iter().any(|c| {
            c.path
                .eq_ignore_ascii_case(&path.to_string_lossy().replace('\\', "/"))
        })
    });
    existing.extend(
        configs
            .iter()
            .map(|c| (PathBuf::from(&c.path), c.contents.as_bytes().to_vec())),
    );
    existing.sort_by(|a, b| a.0.cmp(&b.0));
    existing
}

/// Keep backups outside the game and roll back every touched config on failure.
pub fn activate<T>(
    game: &crate::model::InstalledGame,
    configs: &[Config],
    work: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if configs.is_empty() {
        return work();
    }
    validate(configs)?;
    crate::runtime::ensure_closed(game)?;
    let root = game.path.join("BepInEx/config");
    let backup = crate::runtime_cache::root(game)
        .join("imported-config-backups")
        .join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
    crate::runtime::no_links(&backup)?;
    fs::create_dir_all(&backup)?;
    let mut previous = Vec::new();
    // Inspect all targets and back them up before touching the first file.
    for (index, config) in configs.iter().enumerate() {
        let target = root.join(&config.path);
        crate::runtime::no_links(&target)?;
        let bytes = match fs::symlink_metadata(&target) {
            Ok(meta) => {
                ensure!(
                    meta.is_file() && meta.len() <= 1024 * 1024,
                    "Existing config cannot be safely backed up: {}",
                    config.path
                );
                let bytes = fs::read(&target)?;
                fs::write(backup.join(format!("{index}.cfg")), &bytes)?;
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        previous.push((target, bytes));
    }
    fs::write(
        backup.join("manifest.json"),
        serde_json::to_vec(&configs.iter().map(|c| &c.path).collect::<Vec<_>>())?,
    )?;
    let mut changed = 0;
    let result = (|| {
        for (config, (target, _)) in configs.iter().zip(&previous) {
            atomic_write(target, config.contents.as_bytes())?;
            changed += 1;
        }
        work()
    })();
    if result.is_err() {
        for (target, bytes) in previous.into_iter().take(changed).rev() {
            if let Some(bytes) = bytes {
                atomic_write(&target, &bytes).context(
                    "Could not restore config; backup retained in Canna's runtime cache",
                )?;
            } else {
                crate::runtime::no_links(&target)?;
                fs::remove_file(target)?;
            }
        }
    }
    result
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::runtime::no_links(path)?;
    fs::create_dir_all(path.parent().context("Missing config directory")?)?;
    let temp = path.with_extension(format!("canna-import-{}.pending", std::process::id()));
    crate::runtime::no_links(&temp)?;
    let mut created = false;
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        created = true;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_paths_and_executable_settings() {
        for path in [
            "../a.cfg",
            "a/../../x.cfg",
            "C:/a.cfg",
            "a\\b.cfg",
            "a.exe",
            "plugin.dll",
            "CON.cfg",
            "COM\u{b9}.cfg",
            "a/LPT\u{b2}.txt",
            "COM\u{b3}/settings.cfg",
            "a/NUL.txt",
            "a./b.cfg",
            "a /b.cfg",
            "a:stream.cfg",
            "a.cfg\0",
        ] {
            assert!(!safe_path(path), "{path}");
        }
        assert!(safe_path("Mod Settings/settings.cfg"));
        assert!(
            validate(&[
                Config {
                    path: "a.cfg".into(),
                    contents: "ok".into()
                },
                Config {
                    path: "A.cfg".into(),
                    contents: "other".into()
                }
            ])
            .is_err()
        );
    }
    #[test]
    fn subsequent_edits_survive_relaunch_and_switching_back_reapplies_profile() {
        let base = std::env::temp_dir().join(format!(
            "canna-config-selection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::modpacks::with_test_root(base.join("state"), || {
            let game = crate::model::InstalledGame {
                app_id: 1686940,
                name: "Fixture".into(),
                path: base.join("game"),
                loader: String::new(),
                plugins: 0,
                icon: None,
            };
            let mut pack = crate::modpacks::Modpack::create(
                "Imported".into(),
                String::new(),
                &crate::model::bopl(),
                crate::cache::Source {
                    owner: "canna".into(),
                    repository: "server".into(),
                    branch: "main".into(),
                    catalog_folder: String::new(),
                },
                vec![],
            );
            pack.imported_configs = vec![Config {
                path: "game.cfg".into(),
                contents: "original".into(),
            }];
            assert_eq!(pending(&game, &pack).unwrap().len(), 1);
            crate::play_backup::remember_applied(&game, &pack).unwrap();
            assert!(pending(&game, &pack).unwrap().is_empty());
            let mut another = pack.clone();
            another.id.push_str("-other");
            crate::play_backup::remember_applied(&game, &another).unwrap();
            assert_eq!(pending(&game, &pack).unwrap().len(), 1);
        });
        assert!(base.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn activation_rolls_back_files_and_keeps_backup() {
        let base = std::env::temp_dir().join(format!(
            "canna-config-import-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = crate::model::InstalledGame {
            app_id: 1686940,
            name: "Fixture".into(),
            path: base.join("game"),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        fs::create_dir_all(game.path.join("BepInEx/config")).unwrap();
        fs::write(game.path.join("BepInEx/config/old.cfg"), "old").unwrap();
        let configs = vec![
            Config {
                path: "old.cfg".into(),
                contents: "new".into(),
            },
            Config {
                path: "nested/new.cfg".into(),
                contents: "added".into(),
            },
        ];
        let result: Result<()> = activate(&game, &configs, || {
            assert_eq!(
                fs::read_to_string(game.path.join("BepInEx/config/old.cfg")).unwrap(),
                "new"
            );
            anyhow::bail!("Fixture activation failure")
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(game.path.join("BepInEx/config/old.cfg")).unwrap(),
            "old"
        );
        assert!(!game.path.join("BepInEx/config/nested/new.cfg").exists());
        activate(&game, &configs, || Ok(())).unwrap();
        assert_eq!(
            fs::read_to_string(game.path.join("BepInEx/config/old.cfg")).unwrap(),
            "new"
        );
        assert!(
            crate::runtime_cache::root(&game)
                .join("imported-config-backups")
                .is_dir()
        );
        assert!(base.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(base).unwrap();
    }
}
