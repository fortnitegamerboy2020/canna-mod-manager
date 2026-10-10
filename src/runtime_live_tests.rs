//! Explicitly opted-in Windows game smoke tests. Never run with ordinary cargo test.
use super::*;

#[test]
#[ignore = "Explicit opt-in: installs and starts MoreCompany in a separate owned Lethal Company copy"]
fn lethal_company_copy_install_launch_restore() {
    assert_eq!(
        std::env::var("CANNA_LIVE_GAME_TEST").as_deref(),
        Ok("lethal-company-copy")
    );
    let path = PathBuf::from(std::env::var("CANNA_LIVE_GAME_ROOT").unwrap());
    let allowed = fs::canonicalize(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/overnight-growth/live"),
    )
    .unwrap();
    let resolved = fs::canonicalize(&path).unwrap();
    assert!(resolved.starts_with(&allowed) && resolved != allowed);
    no_links(&path).unwrap();
    let game = InstalledGame {
        app_id: 1966720,
        name: "Lethal Company isolated copy".into(),
        path: path.clone(),
        loader: String::new(),
        plugins: 0,
        icon: None,
    };
    let out = PathBuf::from(std::env::var("CANNA_LIVE_GAME_REPORT").unwrap());
    crate::modpacks::with_test_root(out.join("test-state"), || {
        let info = crate::model::GameInfo {
            app_id: 1966720,
            name: game.name.clone(),
            folder: "lethal-company".into(),
            description: String::new(),
            icon: String::new(),
            mods: vec![],
            mod_folder_status: String::new(),
        };
        let mod_path = PathBuf::from(std::env::var("CANNA_LIVE_GAME_MOD").unwrap());
        let pack = Modpack::create(
            "Isolated startup test".into(),
            String::new(),
            &info,
            crate::cache::Source::from_settings(&Settings::load()),
            vec![crate::modpacks::add_local(&mod_path).unwrap()],
        );
        let framework = fs::read(std::env::var("CANNA_LIVE_GAME_FRAMEWORK").unwrap()).unwrap();
        ensure_closed(&game).unwrap();
        assert!(!path.join("winhttp.dll").exists());
        setup_with_framework(&game, &pack, "", Some(&framework)).unwrap();
        let prepared =
            prepare_install(&game, &pack, "", InstallOptions::default(), &|_| {}).unwrap();
        install_prepared(&game, prepared, "", &|_| {}).unwrap();
        fs::write(path.join("steam_appid.txt"), b"1966720").unwrap();
        let start = |modded: bool| {
            use std::os::windows::process::CommandExt;
            if modded {
                ensure_loader_ready(&path).unwrap();
                set_mode(&path, true).unwrap();
            } else {
                restore_vanilla(&game).unwrap();
            }
            let before = crate::owned_game::OwnedGame::now();
            let child = Command::new(path.join("Lethal Company.exe"))
                .current_dir(&path)
                .args([
                    "-screen-fullscreen",
                    "0",
                    "-screen-width",
                    "960",
                    "-screen-height",
                    "540",
                ])
                .creation_flags(0x08000000)
                .spawn()
                .unwrap();
            let owned = crate::owned_game::OwnedGame::capture(child.id(), &path, before).unwrap();
            (owned, child)
        };
        let (modded, mut modded_child) = start(true);
        std::thread::sleep(Duration::from_secs(45));
        let alive = modded.running();
        let log = fs::read_to_string(path.join("BepInEx/LogOutput.log")).unwrap_or_default();
        modded.stop().unwrap();
        modded_child.wait().unwrap();
        for _ in 0..50 {
            if !modded.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        restore_vanilla(&game).unwrap();
        let absent =
            !path.join("winhttp.dll").exists() && !path.join("BepInEx/plugins/Canna").exists();
        let (vanilla, mut vanilla_child) = start(false);
        std::thread::sleep(Duration::from_secs(25));
        let vanilla_alive = vanilla.running();
        vanilla.stop().unwrap();
        vanilla_child.wait().unwrap();
        let loaded = log.lines().any(|l| l.contains("Loading [MoreCompany"));
        let errors: Vec<_> = log
            .lines()
            .filter(|l| l.contains("[Error") || l.contains("[Fatal"))
            .collect();
        let report = serde_json::json!({"game":"Lethal Company","isolated_copy":true,"mod":"MoreCompany 1.14.0","modded_process_45s":alive,"vanilla_process_25s":vanilla_alive,"mod_loaded":loaded,"managed_files_removed":absent,"errors":errors,"scope":"Installation/startup/vanilla restoration only; no mission or multiplayer verification"});
        fs::create_dir_all(&out).unwrap();
        fs::write(
            out.join("lethal-copy-live.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        assert!(alive && loaded && absent && vanilla_alive);
    });
}

#[test]
fn mono6_requires_both_core_and_mono_adapter() {
    let temp = std::env::temp_dir().join(format!("canna-mono6-{}", std::process::id()));
    fs::create_dir_all(temp.join("BepInEx/core")).unwrap();
    fs::write(temp.join("BepInEx/core/BepInEx.Unity.Mono.dll"), b"inert").unwrap();
    assert!(!matching_core(&temp, false));
    fs::write(temp.join("BepInEx/core/BepInEx.Core.dll"), b"inert").unwrap();
    assert!(matching_core(&temp, false));
    assert!(!matching_core(&temp, true));
    fs::remove_dir_all(temp).unwrap();
}
#[test]
fn incomplete_loader_is_rejected_before_switching_it_on() {
    let root = std::env::temp_dir().join(format!("canna-loader-health-{}", std::process::id()));
    fs::create_dir_all(root.join("BepInEx/core")).unwrap();
    fs::write(root.join("winhttp.dll"), b"fixture").unwrap();
    assert!(
        ensure_loader_ready(&root)
            .unwrap_err()
            .to_string()
            .contains("core is missing")
    );
    fs::write(root.join("BepInEx/core/BepInEx.dll"), b"fixture").unwrap();
    ensure_loader_ready(&root).unwrap();
    fs::write(root.join("GameAssembly.dll"), b"fixture").unwrap();
    assert!(ensure_loader_ready(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Installs and starts an explicitly selected saved pack in an owned Bopl/ROUNDS game, then restores vanilla"]
fn saved_bopl_or_rounds_install_launch_restore() {
    let choice = std::env::var("CANNA_LIVE_GAME_TEST").unwrap();
    let id = match choice.as_str() {
        "bopl-battle" => 1686940,
        "rounds" => 1557740,
        _ => panic!("Explicitly select bopl-battle or rounds"),
    };
    let game = crate::steam::scan(&Settings::load().steam_path)
        .games
        .into_iter()
        .find(|g| g.app_id == id)
        .unwrap();
    ensure_closed(&game).unwrap();
    let token = crate::website::session();
    assert!(!token.is_empty(), "Existing signed-in account required");
    let mut pack = crate::modpacks::load_all()
        .0
        .into_iter()
        .find(|p| p.game.app_id == id)
        .unwrap();
    if id == 1686940 {
        let catalog = repository::sync(&Settings::load(), &token).unwrap();
        let info = catalog.games.iter().find(|g| g.app_id == id).unwrap();
        let anvil = info
            .mods
            .iter()
            .find(|m| m.name == "Canna Anvil" && m.version == "1.0.7")
            .expect("Reviewed Anvil 1.0.7 required")
            .clone();
        pack.mods = vec![anvil];
        pack.ignored_dependencies.clear();
        crate::dependencies::complete(&mut pack, info).unwrap();
        println!("Bopl isolated test pack: {} packages", pack.mods.len());
    }
    let options = InstallOptions {
        rebound_enabled: id == 1557740,
    };
    let progress = |s: &str| println!("{s}");
    let prepared = prepare_install(&game, &pack, &token, options, &progress).unwrap();
    let applied = prepared.effective_pack().clone();
    crate::play_backup::before_change(&game, &applied).unwrap();
    install_prepared(&game, prepared, &token, &progress).unwrap();
    let owned = launch(&game, true).unwrap();
    println!("{} MODDED GAME PID {}", game.name, owned.pid);
    std::thread::sleep(Duration::from_secs(45));
    let alive = owned.running();
    let log = fs::read_to_string(game.path.join("BepInEx/LogOutput.log")).unwrap_or_default();
    owned.stop().unwrap();
    for _ in 0..50 {
        if !owned.running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let restore = restore_vanilla(&game).unwrap();
    let vanilla = launch(&game, false).unwrap();
    println!("{} VANILLA GAME PID {}", game.name, vanilla.pid);
    std::thread::sleep(Duration::from_secs(25));
    let vanilla_alive = vanilla.running();
    vanilla.stop().unwrap();
    let output = PathBuf::from(std::env::var("CANNA_LIVE_GAME_REPORT").unwrap());
    fs::write(output.join(format!("{choice}-modded.log")), &log).unwrap();
    let loaded: Vec<_> = log.lines().filter(|l| l.contains("Loading [")).collect();
    let errors: Vec<_> = log
        .lines()
        .filter(|l| l.contains("[Error") || l.contains("[Fatal"))
        .collect();
    let report = serde_json::json!({"game":game.name,"modded_process_45s":alive,"vanilla_process_25s":vanilla_alive,"loaded":loaded,"errors":errors,"restore":restore,"managed_plugins_absent":!game.path.join("BepInEx/plugins/Canna").exists(),"scope":"Real installation, plugin initialization and launch smoke test; no card pick, ability interaction, mission or multiplayer verification"});
    fs::write(
        output.join(format!("{choice}-live.json")),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(alive && vanilla_alive && !loaded.is_empty());
}

#[test]
#[ignore = "Starts an owned Lethal Company process with a supplied mod, then restores vanilla"]
fn lethal_company_install_launch_restore() {
    assert_eq!(
        std::env::var("CANNA_LIVE_GAME_TEST").as_deref(),
        Ok("lethal-company")
    );
    let out = PathBuf::from(std::env::var("CANNA_LIVE_GAME_REPORT").unwrap());
    let game = InstalledGame {
        app_id: 1966720,
        name: "Lethal Company".into(),
        path: "D:/SteamLibrary/steamapps/common/Lethal Company".into(),
        loader: String::new(),
        plugins: 0,
        icon: None,
    };
    let framework = fs::read(std::env::var("CANNA_LIVE_GAME_FRAMEWORK").unwrap()).unwrap();
    let mod_path = PathBuf::from(std::env::var("CANNA_LIVE_GAME_MOD").unwrap());
    let info = crate::model::GameInfo {
        app_id: game.app_id,
        name: game.name.clone(),
        folder: "lethal-company".into(),
        description: String::new(),
        icon: String::new(),
        mods: vec![],
        mod_folder_status: String::new(),
    };
    let pack = Modpack::create(
        "Canna isolated live test".into(),
        String::new(),
        &info,
        crate::cache::Source::from_settings(&Settings::load()),
        vec![crate::modpacks::add_local(&mod_path).unwrap()],
    );
    ensure_closed(&game).unwrap();
    // A guard ensures this test does not overwrite a previous manual installation.
    assert!(
        !game.path.join("winhttp.dll").exists() && !game.path.join("BepInEx/core").exists(),
        "Back up and park the original runtime before opting into this test"
    );
    setup_with_framework(&game, &pack, "", Some(&framework)).unwrap();
    let prepared = prepare_install(&game, &pack, "", InstallOptions::default(), &|s| {
        println!("{s}")
    })
    .unwrap();
    install_prepared(&game, prepared, "", &|s| println!("{s}")).unwrap();
    let owned = launch(&game, true).unwrap();
    println!("MODDED GAME PID {}", owned.pid);
    std::thread::sleep(Duration::from_secs(45));
    assert!(owned.running(), "Modded process exited early");
    let log = fs::read_to_string(game.path.join("BepInEx/LogOutput.log")).unwrap();
    fs::write(out.join("lethal-modded.log"), &log).unwrap();
    owned.stop().unwrap();
    for _ in 0..50 {
        if !owned.running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let restore = restore_vanilla(&game).unwrap();
    assert!(!game.path.join("BepInEx/plugins/Canna").exists());
    assert!(!game.path.join("winhttp.dll").exists());
    let vanilla = launch(&game, false).unwrap();
    println!("VANILLA GAME PID {}", vanilla.pid);
    std::thread::sleep(Duration::from_secs(30));
    let running = vanilla.running();
    vanilla.stop().unwrap();
    let loaded = log
        .lines()
        .any(|line| line.contains("Loading [MoreCompany"));
    let failures: Vec<_> = log
        .lines()
        .filter(|line| line.contains("[Error") || line.contains("[Fatal"))
        .collect();
    let report = serde_json::json!({"game":game.name,"app_id":game.app_id,"modded_process_45s":true,"mod_loaded":loaded,"vanilla_process_30s":running,"restore":restore,"loader_proxy_absent":!game.path.join("winhttp.dll").exists(),"managed_plugins_absent":!game.path.join("BepInEx/plugins/Canna").exists(),"errors":failures,"scope":"Real installation, plugin initialization and launch smoke test; no hosted lobby, multiplayer or full mission verification"});
    fs::write(
        out.join("lethal-live.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    assert!(
        loaded && running,
        "See the live report for plugin/launch failures"
    );
}
