use crate::{model::InstalledGame, model::ModInfo, modpacks::Modpack, steam};
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

pub const ROUNDS_BRANCH: &str = "old-rounds-for-mods";
pub const PUBLIC_BRANCH: &str = "public";

fn public_requirement(item: &ModInfo) -> bool {
    item.enabled && item.provenance["required_game_branch"].as_str() == Some(PUBLIC_BRANCH)
}

fn ducttape_public_requirement(item: &ModInfo) -> bool {
    item.enabled
        && item.local_file.is_empty()
        && item.provenance["provider"].as_str() == Some("thunderstore")
        && item.provenance["id"].as_str() == Some("kieron_exe-DuctTape")
        && matches!(
            item.provenance["source_url"].as_str(),
            Some("https://thunderstore.io/c/rounds/p/kieron_exe/DuctTape")
                | Some("https://thunderstore.io/c/rounds/p/kieron_exe/DuctTape/")
        )
}

fn legacy_library_requirement(item: &ModInfo) -> bool {
    item.enabled
        && matches!(
            (item.name.as_str(), item.version.as_str()),
            ("UnboundLib", "3.2.14") | ("MMHook", "1.0.0")
        )
}

pub fn rounds_requirement(item: &ModInfo) -> bool {
    item.enabled
        && (item.provenance["required_game_branch"].as_str() == Some(ROUNDS_BRANCH)
            || legacy_library_requirement(item)
            || (item.version == "1.8.0"
                && item.provenance["source_url"].as_str().is_some_and(|url| {
                    url.trim_end_matches('/')
                        == "https://thunderstore.io/c/rounds/p/flofl/HollowPurple"
                })))
}

fn effective_rounds_requirement(item: &ModInfo, ducttape: bool) -> bool {
    // DuctTape keeps these packages installed and replaces their libraries at
    // launch. It does not establish compatibility for other legacy mods.
    rounds_requirement(item)
        && !(ducttape
            && legacy_library_requirement(item)
            && item.provenance["required_game_branch"].as_str() != Some(ROUNDS_BRANCH))
}

pub fn required_branch(pack: &Modpack) -> Option<&'static str> {
    if pack.game.app_id != 1557740 {
        return None;
    }
    let ducttape = pack.mods.iter().any(ducttape_public_requirement);
    if pack
        .mods
        .iter()
        .any(|item| effective_rounds_requirement(item, ducttape))
    {
        Some(ROUNDS_BRANCH)
    } else if ducttape || pack.mods.iter().any(public_requirement) {
        Some(PUBLIC_BRANCH)
    } else {
        None
    }
}

pub fn branch_status(required: &str, installed: Option<&steam::SteamVersion>) -> Result<()> {
    if installed.is_some_and(|v| v.branch.eq_ignore_ascii_case(required)) {
        return Ok(());
    }
    let current = installed
        .map(|v| v.branch.as_str())
        .unwrap_or("unknown (Steam manifest unavailable)");
    if required == PUBLIC_BRANCH {
        bail!(
            "This mod requires the default public ROUNDS version. Installed branch: {current}. In Steam open ROUNDS → Properties → Betas → None, wait for the update to finish, then retry."
        );
    }
    bail!(
        "This mod requires ROUNDS: Old ROUNDS for mods ({required}). Installed branch: {current}. In Steam open ROUNDS → Properties → Betas → Old ROUNDS for mods, wait for the update to finish, then retry."
    )
}

pub fn check_pack(game: &InstalledGame, pack: &Modpack) -> Result<()> {
    let ducttape = pack.game.app_id == 1557740 && pack.mods.iter().any(ducttape_public_requirement);
    if pack.game.app_id == 1557740 && (ducttape || pack.mods.iter().any(public_requirement)) {
        if pack
            .mods
            .iter()
            .any(|item| effective_rounds_requirement(item, ducttape))
        {
            bail!(
                "This pack mixes public ROUNDS and Old ROUNDS for mods requirements. Disable the mod requiring Old ROUNDS for mods or use separate packs. DuctTape does not override an explicit legacy requirement or original HollowPurple's requirement."
            );
        }
        if !ducttape && pack.mods.iter().any(legacy_library_requirement) {
            bail!(
                "HollowPurple Fixed's public port uses its own compatibility adapter. Remove legacy UnboundLib/MMHook from this pack; other mods that need those versions belong in a separate Old ROUNDS for mods pack."
            );
        }
    }
    if let Some(required) = required_branch(pack) {
        let enabled: Vec<_> = pack.mods.iter().filter(|m| m.enabled).collect();
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
            } else if hash == "8037efb454f073c837bb75039b087d49fdd77ec16149966c92d4eb42accd54b0" {
                branch_status(PUBLIC_BRANCH, steam::installed_version(game).as_ref())?;
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
    fn ducttape() -> ModInfo {
        serde_json::from_value(serde_json::json!({
            "name":"DuctTape", "version":"1.0.0", "file":"Mods/ducttape.zip",
            "provenance":{
                "provider":"thunderstore", "id":"kieron_exe-DuctTape",
                "source_url":"https://thunderstore.io/c/rounds/p/kieron_exe/DuctTape/"
            }
        }))
        .unwrap()
    }
    fn legacy_libraries() -> Vec<ModInfo> {
        [("UnboundLib", "3.2.14"), ("MMHook", "1.0.0")]
            .into_iter()
            .map(|(name, version)| {
                serde_json::from_value(serde_json::json!({
                    "name":name,"version":version,"file":format!("Mods/{name}.zip")
                }))
                .unwrap()
            })
            .collect()
    }
    struct SteamFixture {
        root: std::path::PathBuf,
        game: InstalledGame,
    }
    impl SteamFixture {
        fn new(branch: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "canna-ducttape-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let game = InstalledGame {
                app_id: 1557740,
                name: "ROUNDS".into(),
                path: root.join("steamapps/common/ROUNDS"),
                loader: String::new(),
                plugins: 0,
                icon: None,
            };
            std::fs::create_dir_all(&game.path).unwrap();
            let fixture = Self { root, game };
            fixture.set_branch(branch);
            fixture
        }
        fn set_branch(&self, branch: &str) {
            std::fs::write(
                self.root.join("steamapps/appmanifest_1557740.acf"),
                format!(
                    r#""AppState" {{ "buildid" "21020021" "UserConfig" {{ "BetaKey" "{branch}" }} }}"#
                ),
            )
            .unwrap();
        }
    }
    impl Drop for SteamFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn official_ducttape_selects_public_with_its_legacy_libraries() {
        let mut selected = legacy_libraries();
        assert_eq!(
            required_branch(&pack(selected.clone())),
            Some(ROUNDS_BRANCH)
        );
        selected.push(ducttape());
        assert_eq!(
            required_branch(&pack(selected.clone())),
            Some(PUBLIC_BRANCH)
        );
        selected.reverse();
        assert_eq!(required_branch(&pack(selected)), Some(PUBLIC_BRANCH));
        let mut without_slash = ducttape();
        without_slash.provenance["source_url"] =
            "https://thunderstore.io/c/rounds/p/kieron_exe/DuctTape".into();
        assert_eq!(
            required_branch(&pack(vec![without_slash])),
            Some(PUBLIC_BRANCH)
        );
        let mut other_game = pack(vec![ducttape()]);
        other_game.game.app_id = 1686940;
        assert_eq!(required_branch(&other_game), None);
    }
    #[test]
    fn disabled_local_and_mismatched_ducttape_cannot_override_legacy_libraries() {
        let official = ducttape();
        let mut disabled = official.clone();
        disabled.enabled = false;
        let mut local = official.clone();
        local.local_file = "local-ducttape.zip".into();
        let mut name_only = official.clone();
        name_only.provenance = serde_json::Value::Null;
        let mut invalid = vec![disabled, local, name_only];
        for (field, value) in [
            ("provider", "github"),
            ("id", "another_author-DuctTape"),
            (
                "source_url",
                "https://thunderstore.io/c/rounds/p/another_author/DuctTape/",
            ),
            (
                "source_url",
                "https://thunderstore.io/c/bopl-battle/p/kieron_exe/DuctTape/",
            ),
            (
                "source_url",
                "https://thunderstore.io/c/rounds/p/kieron_exe/DuctTape/?official=true",
            ),
            (
                "source_url",
                "https://thunderstore.io.example.com/c/rounds/p/kieron_exe/DuctTape/",
            ),
        ] {
            let mut item = official.clone();
            item.provenance[field] = value.into();
            invalid.push(item);
        }
        for item in invalid {
            assert_eq!(required_branch(&pack(vec![item.clone()])), None);
            let mut selected = legacy_libraries();
            selected.push(item);
            assert_eq!(required_branch(&pack(selected)), Some(ROUNDS_BRANCH));
        }
    }
    #[test]
    fn ducttape_keeps_explicit_legacy_and_original_hollowpurple_requirements() {
        let fixture = SteamFixture::new(PUBLIC_BRANCH);
        let mut explicit_library = legacy_libraries().remove(0);
        explicit_library.provenance = serde_json::json!({"required_game_branch":ROUNDS_BRANCH});
        let mut unrelated = explicit_library.clone();
        unrelated.name = "Author-declared legacy mod".into();
        unrelated.file = "Mods/declared.zip".into();
        for item in [explicit_library, unrelated, hollow(true)] {
            let selected = pack(vec![ducttape(), item]);
            assert_eq!(required_branch(&selected), Some(ROUNDS_BRANCH));
            assert!(
                check_pack(&fixture.game, &selected)
                    .unwrap_err()
                    .to_string()
                    .contains("mixes public ROUNDS")
            );
        }
    }
    #[test]
    fn public_pack_with_ducttape_and_legacy_libraries_passes_both_guards() {
        let fixture = SteamFixture::new(PUBLIC_BRANCH);
        let mut mods = legacy_libraries();
        mods.push(ducttape());
        let mut selected = pack(mods);
        assert!(check_pack(&fixture.game, &selected).is_ok());
        let mut fixed = hollow(true);
        fixed.name = "HollowPurple Fixed".into();
        fixed.version = "1.8.2".into();
        fixed.file = "Mods/fixed.zip".into();
        fixed.provenance["required_game_branch"] = PUBLIC_BRANCH.into();
        selected.mods.push(fixed);
        assert!(check_pack(&fixture.game, &selected).is_ok());
        selected
            .mods
            .iter_mut()
            .find(|m| m.name == "DuctTape")
            .unwrap()
            .enabled = false;
        assert!(
            check_pack(&fixture.game, &selected)
                .unwrap_err()
                .to_string()
                .contains("mixes public ROUNDS")
        );
        selected
            .mods
            .iter_mut()
            .find(|m| m.name == "DuctTape")
            .unwrap()
            .enabled = true;
        fixture.set_branch(ROUNDS_BRANCH);
        assert!(
            check_pack(&fixture.game, &selected)
                .unwrap_err()
                .to_string()
                .contains("Betas → None")
        );
        assert!(!fixture.game.path.join("BepInEx").exists());
    }
    #[test]
    fn ducttape_does_not_allow_duplicate_hollowpurple_plugins() {
        let fixture = SteamFixture::new(PUBLIC_BRANCH);
        let mut original = hollow(true);
        original.version = "2.0.0".into();
        let mut fixed = original.clone();
        fixed.name = "HollowPurple Fixed".into();
        fixed.file = "Mods/fixed.zip".into();
        fixed.provenance["required_game_branch"] = PUBLIC_BRANCH.into();
        assert!(
            check_pack(&fixture.game, &pack(vec![ducttape(), original, fixed]))
                .unwrap_err()
                .to_string()
                .contains("Disable or remove original HollowPurple")
        );
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
    fn public_port_rejects_legacy_branch_and_mixed_dependencies() {
        let mut fixed = hollow(true);
        fixed.name = "HollowPurple Fixed".into();
        fixed.version = "1.8.2".into();
        fixed.provenance["required_game_branch"] = PUBLIC_BRANCH.into();
        let game = InstalledGame {
            app_id: 1557740,
            name: "ROUNDS".into(),
            path: std::env::temp_dir().join("canna-compat-missing-game"),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        assert_eq!(
            required_branch(&pack(vec![fixed.clone()])),
            Some(PUBLIC_BRANCH)
        );
        let public = steam::SteamVersion {
            branch: PUBLIC_BRANCH.into(),
            build: "21020021".into(),
        };
        assert!(branch_status(PUBLIC_BRANCH, Some(&public)).is_ok());
        let old = steam::SteamVersion {
            branch: ROUNDS_BRANCH.into(),
            build: String::new(),
        };
        assert!(
            branch_status(PUBLIC_BRANCH, Some(&old))
                .unwrap_err()
                .to_string()
                .contains("Betas → None")
        );
        assert!(
            check_pack(&game, &pack(vec![hollow(true), fixed.clone()]))
                .unwrap_err()
                .to_string()
                .contains("mixes public ROUNDS")
        );
        let mut unbound = hollow(true);
        unbound.name = "UnboundLib".into();
        unbound.version = "3.2.14".into();
        unbound.provenance = serde_json::json!({});
        assert!(
            check_pack(&game, &pack(vec![unbound.clone(), fixed.clone()]))
                .unwrap_err()
                .to_string()
                .contains("mixes public ROUNDS")
        );
        unbound.enabled = false;
        assert!(
            check_pack(&game, &pack(vec![unbound, fixed]))
                .unwrap_err()
                .to_string()
                .contains("manifest unavailable")
        );
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
