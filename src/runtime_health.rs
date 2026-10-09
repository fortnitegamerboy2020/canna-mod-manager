//! Local-only setup inspection; reports do not read or upload log contents.
use crate::{model::InstalledGame, modpacks::Modpack};
use std::{collections::BTreeMap, fs, path::Path};

fn plugin_names(
    root: &Path,
    depth: usize,
    names: &mut BTreeMap<String, usize>,
    visited: &mut usize,
) {
    if depth > 8 || *visited >= 2000 || crate::runtime::no_links(root).is_err() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        *visited += 1;
        if *visited > 2000 {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            plugin_names(&entry.path(), depth + 1, names, visited);
        } else if entry
            .path()
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("dll"))
        {
            *names
                .entry(entry.file_name().to_string_lossy().to_lowercase())
                .or_default() += 1;
        }
    }
}
pub fn report(game: &InstalledGame, pack: Option<&Modpack>) -> String {
    let mut lines = vec![format!(
        "Setup health — {} · Canna {}",
        game.name,
        env!("CARGO_PKG_VERSION")
    )];
    if let Some(version) = crate::steam::installed_version(game) {
        lines.push(format!(
            "Steam branch {} · build {}",
            version.branch, version.build
        ));
    }
    if crate::model::framework(game.app_id) != "bepinex" {
        lines.push("This health check covers Unity/BepInEx games. Use Play Lab for this game's other setup checks.".into());
        return lines.join("\n");
    }
    match crate::unity_restore::has_parked_managed(game){Ok(true)=>lines.push("READY FOR VANILLA: managed plugins are parked outside the game. Launch modded will prepare them again.".into()),Err(error)=>lines.push(format!("CHECK RECOVERY: {error}. Preserve the recovery files; use Restore vanilla files with the game closed.")),_=>{}}
    let il2cpp = game.path.join("GameAssembly.dll").is_file();
    let core = game.path.join(if il2cpp {
        "BepInEx/core/BepInEx.Unity.IL2CPP.dll"
    } else {
        "BepInEx/core/BepInEx.dll"
    });
    let proxy = ["winhttp.dll", "version.dll"]
        .iter()
        .any(|p| game.path.join(p).is_file());
    let ini = game.path.join("doorstop_config.ini");
    if proxy && !core.is_file() {
        lines.push("INCOMPLETE LOADER: a bootstrap DLL is present, but the matching BepInEx core is missing. Back up a manual installation before repairing it. Canna will preserve conflicting files.".into());
    } else if core.is_file() && proxy && ini.is_file() {
        lines.push(format!(
            "Loader present · {}",
            if il2cpp { "IL2CPP" } else { "Mono" }
        ));
    } else {
        lines.push("Loader is inactive or not installed. Launch modded prepares the reviewed framework when needed.".into());
    }
    if ini.is_file() {
        match fs::metadata(&ini)
            .ok()
            .filter(|m| m.len() <= 256 * 1024)
            .and_then(|_| fs::read_to_string(&ini).ok())
        {
            Some(text) => {
                if crate::runtime::doorstop_text(&text, false).is_err() {
                    lines.push("CHECK LOADER CONFIG: General.enabled is missing. Canna cannot switch this custom bootstrap safely.".into());
                }
            }
            None => lines.push(
                "CHECK LOADER CONFIG: unreadable or oversized; preserve it before repair.".into(),
            ),
        }
    }
    let mut names = BTreeMap::new();
    let mut visited = 0;
    plugin_names(
        &game.path.join("BepInEx/plugins"),
        0,
        &mut names,
        &mut visited,
    );
    lines.push(format!(
        "{} active plugin DLL(s) · {} file(s) inspected",
        names.values().sum::<usize>(),
        visited
    ));
    for (name, count) in names.iter().filter(|(_, count)| **count > 1) {
        lines.push(format!("POSSIBLE DUPLICATE: {name} appears {count} times. Check manual plugins and the managed pack before removing copies."));
    }
    if visited >= 2000 {
        lines.push("Plugin scan reached its file limit; deeper files were not inspected.".into());
    }
    if let Some(pack) = pack {
        lines.push(format!(
            "Saved pack: {} enabled / {} selected mods",
            pack.mods.iter().filter(|m| m.enabled).count(),
            pack.mods.len()
        ));
        let mut dependencies = std::collections::BTreeSet::new();
        for item in pack.mods.iter().filter(|m| m.enabled) {
            for dependency in &item.dependencies {
                if !dependencies.insert(dependency) {
                    continue;
                }
                if pack.ignored_dependencies.contains(dependency) {
                    lines.push(format!("DEPENDENCY OVERRIDE: {dependency} intentionally excluded. Check replacement compatibility."));
                } else if !pack.mods.iter().any(|m| m.enabled && m.name == *dependency) {
                    lines.push(format!("CHECK DEPENDENCY: {dependency} is missing or disabled. Refresh approved downloads; review any intentional replacement."));
                }
            }
        }
    }
    lines.push("Local inspection only; plugin initialization, gameplay and multiplayer need a game test. No logs or personal paths are uploaded by this report.".into());
    lines.join("\n")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orphan_loader_and_duplicate_plugins_are_actionable_advisories() {
        let root = std::env::temp_dir().join(format!("canna-health-{}", std::process::id()));
        fs::create_dir_all(root.join("BepInEx/plugins/a")).unwrap();
        fs::create_dir_all(root.join("BepInEx/plugins/b")).unwrap();
        fs::write(root.join("winhttp.dll"), b"fixture").unwrap();
        for name in ["a/Mod.dll", "b/mod.dll"] {
            fs::write(root.join("BepInEx/plugins").join(name), b"fixture").unwrap();
        }
        let game = InstalledGame {
            app_id: 1966720,
            name: "Fixture".into(),
            path: root.clone(),
            loader: String::new(),
            plugins: 0,
            icon: None,
        };
        let text = report(&game, None);
        assert!(text.contains("INCOMPLETE LOADER") && text.contains("POSSIBLE DUPLICATE: mod.dll"));
        assert!(!text.contains(root.to_string_lossy().as_ref()));
        fs::remove_dir_all(root).unwrap();
    }
}
