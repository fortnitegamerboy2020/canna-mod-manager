use crate::{model::InstalledGame, model::ModInfo, modpacks::Modpack, steam};
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

pub const ROUNDS_BRANCH: &str = "old-rounds-for-mods";

pub fn rounds_requirement(item: &ModInfo) -> bool {
    item.enabled
        && (item.provenance["required_game_branch"].as_str() == Some(ROUNDS_BRANCH)
            || (item.version == "1.8.0"
                && item.provenance["source_url"].as_str().is_some_and(|url| {
                    url.trim_end_matches('/')
                        == "https://thunderstore.io/c/rounds/p/flofl/HollowPurple"
                })))
}

pub fn required_branch(pack: &Modpack) -> Option<&'static str> {
    (pack.game.app_id == 1557740 && pack.mods.iter().any(rounds_requirement))
        .then_some(ROUNDS_BRANCH)
}

pub fn branch_status(required: &str, installed: Option<&steam::SteamVersion>) -> Result<()> {
    if installed.is_some_and(|v| v.branch.eq_ignore_ascii_case(required)) {
        return Ok(());
    }
    let current = installed
        .map(|v| v.branch.as_str())
        .unwrap_or("unknown (Steam manifest unavailable)");
    bail!(
        "This mod requires ROUNDS: Old ROUNDS for mods ({required}). Installed branch: {current}. In Steam open ROUNDS → Properties → Betas → Old ROUNDS for mods, wait for the update to finish, then retry."
    )
}

pub fn check_pack(game: &InstalledGame, pack: &Modpack) -> Result<()> {
    if let Some(required) = required_branch(pack) {
        let enabled: Vec<_> = pack.mods.iter().filter(|m| rounds_requirement(m)).collect();
        if enabled.iter().any(|m| m.name == "HollowPurple Fixed")
            && enabled.iter().any(|m| m.name == "HollowPurple")
        {
            bail!(
                "Disable or remove original HollowPurple before using HollowPurple Fixed; both share the original plugin identity and DLL path."
            );
        }
        branch_status(required, steam::installed_version(game).as_ref())?;
    }
    Ok(())
}

pub fn check_current(game: &InstalledGame) -> Result<()> {
    if game.app_id == 1557740 {
        let path = game
            .path
            .join("BepInEx/plugins/HollowPurple/HollowPurple.dll");
        if path.is_file() && std::fs::metadata(&path)?.len() <= 16 * 1024 * 1024 {
            crate::runtime::no_links(&path)?;
            let hash = format!("{:x}", Sha256::digest(std::fs::read(&path)?));
            if known_legacy_hash(&hash) {
                branch_status(ROUNDS_BRANCH, steam::installed_version(game).as_ref())?;
            }
        }
    }
    Ok(())
}

fn known_legacy_hash(hash: &str) -> bool {
    // Exact inspected upstream 1.8.0 and Canna fork 1.8.1 DLLs, not arbitrary
    // future mods using the same folder name.
    matches!(
        hash,
        "4cdeaaea4a0b296f1e609b39535058de0afdfd29e5467cd18ab48156aa7fef32"
            | "7a92423326722a445b29b5cf6d8e93ba38add6db1c6b8b8bc7be5a578eebb9e3"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hollow(enabled: bool) -> ModInfo {
        serde_json::from_value(serde_json::json!({"name":"HollowPurple", "version":"1.8.0", "file":"Mods/hollow.zip", "enabled":enabled, "provenance":{"source_url":"https://thunderstore.io/c/rounds/p/flofl/HollowPurple/"}})).unwrap()
    }
    fn pack(mods: Vec<ModInfo>) -> Modpack {
        let game: crate::model::GameInfo = serde_json::from_value(
            serde_json::json!({"app_id":1557740,"name":"ROUNDS","folder":"rounds"}),
        )
        .unwrap();
        Modpack::create(
            "Branch test".into(),
            String::new(),
            &game,
            crate::cache::Source {
                owner: "canna".into(),
                repository: "test".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            },
            mods,
        )
    }
    #[test]
    fn requirements_apply_only_to_enabled_rounds_content_and_block_before_setup() {
        assert_eq!(
            required_branch(&pack(vec![hollow(true)])),
            Some(ROUNDS_BRANCH)
        );
        assert_eq!(required_branch(&pack(vec![hollow(false)])), None);
        let mut other_game = pack(vec![hollow(true)]);
        other_game.game.app_id = 550;
        assert_eq!(required_branch(&other_game), None);
        let game = InstalledGame {
            app_id: 1557740,
            name: "ROUNDS".into(),
            path: std::env::temp_dir().join("canna-compat-missing-game"),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        let error = crate::runtime::setup(&game, &pack(vec![hollow(true)]), "")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Properties → Betas"));
        assert!(!game.path.join("BepInEx").exists());
    }
    #[test]
    fn author_confirmed_requirement_survives_old_cached_metadata_but_ignores_disabled_mods() {
        assert!(rounds_requirement(&hollow(true)));
        assert!(!rounds_requirement(&hollow(false)));
        let mut future = hollow(true);
        future.version = "2.0.0".into();
        assert!(!rounds_requirement(&future));
        let mut unrelated = hollow(true);
        unrelated.provenance = serde_json::json!({"source_url":"https://example.com/HollowPurple"});
        assert!(!rounds_requirement(&unrelated));
        unrelated.provenance["required_game_branch"] = ROUNDS_BRANCH.into();
        assert!(rounds_requirement(&unrelated));
    }
    #[test]
    fn original_and_fork_cannot_be_applied_together() {
        let mut fixed = hollow(true);
        fixed.name = "HollowPurple Fixed".into();
        fixed.version = "1.8.1".into();
        fixed.provenance["required_game_branch"] = ROUNDS_BRANCH.into();
        let game = InstalledGame {
            app_id: 1557740,
            name: "ROUNDS".into(),
            path: std::env::temp_dir().join("canna-compat-missing-game"),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        let error = check_pack(&game, &pack(vec![hollow(true), fixed]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Disable or remove original HollowPurple"));
    }
    #[test]
    fn wrong_or_unknown_branch_is_actionable_and_matching_branch_passes() {
        assert!(
            branch_status(ROUNDS_BRANCH, None)
                .unwrap_err()
                .to_string()
                .contains("manifest unavailable")
        );
        for branch in ["public", "version_1.1.1"] {
            let version = steam::SteamVersion {
                branch: branch.into(),
                build: "21020021".into(),
            };
            let error = branch_status(ROUNDS_BRANCH, Some(&version))
                .unwrap_err()
                .to_string();
            assert!(error.contains(branch) && error.contains("Properties → Betas"));
        }
        let version = steam::SteamVersion {
            branch: ROUNDS_BRANCH.into(),
            build: "".into(),
        };
        assert!(branch_status(ROUNDS_BRANCH, Some(&version)).is_ok());
    }
    #[test]
    fn unmodified_game_and_unrelated_plugins_do_not_require_a_beta() {
        assert!(!known_legacy_hash(&"0".repeat(64)));
        assert!(known_legacy_hash(
            "7a92423326722a445b29b5cf6d8e93ba38add6db1c6b8b8bc7be5a578eebb9e3"
        ));
        let game = InstalledGame {
            app_id: 1557740,
            name: "ROUNDS".into(),
            path: std::env::temp_dir().join("canna-compat-no-plugins"),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        assert!(check_current(&game).is_ok());
    }
}
