use crate::model::{InstalledGame, Scan};
use anyhow::{Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
enum Value {
    Text(String),
    Map(BTreeMap<String, Value>),
}
impl Value {
    fn map(&self) -> Option<&BTreeMap<String, Value>> {
        if let Self::Map(m) = self {
            Some(m)
        } else {
            None
        }
    }
    fn text(&self) -> Option<&str> {
        if let Self::Text(s) = self {
            Some(s)
        } else {
            None
        }
    }
}
// Valve KeyValues: quoted strings, escaped backslashes, nesting, and line comments.
fn parse(input: &str) -> Result<BTreeMap<String, Value>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '{' | '}' => tokens.push(c.to_string()),
            '"' => {
                let mut s = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    if c == '"' {
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        match chars.peek() {
                            Some('\\' | '"') => s.push(chars.next().unwrap()),
                            _ => s.push(c),
                        }
                    } else {
                        s.push(c);
                    }
                }
                if !closed {
                    bail!("Unterminated VDF string")
                }
                tokens.push(s);
            }
            _ => {
                let mut s = c.to_string();
                while chars
                    .peek()
                    .is_some_and(|c| !c.is_whitespace() && *c != '{' && *c != '}')
                {
                    s.push(chars.next().unwrap());
                }
                tokens.push(s);
            }
        }
    }
    fn object(
        t: &[String],
        p: &mut usize,
        nested: bool,
        depth: usize,
    ) -> Result<BTreeMap<String, Value>> {
        if depth > 32 {
            bail!("VDF nesting limit exceeded")
        }
        let mut m = BTreeMap::new();
        while *p < t.len() {
            let key = t[*p].clone();
            *p += 1;
            if key == "}" {
                if nested {
                    return Ok(m);
                }
                bail!("Unexpected closing brace")
            }
            if key == "{" {
                bail!("Expected VDF key")
            }
            let v = t
                .get(*p)
                .ok_or_else(|| anyhow::anyhow!("Missing VDF value"))?
                .clone();
            *p += 1;
            let value = if v == "{" {
                Value::Map(object(t, p, true, depth + 1)?)
            } else if v == "}" {
                bail!("Missing VDF value")
            } else {
                Value::Text(v)
            };
            m.insert(key.to_lowercase(), value);
        }
        if nested {
            bail!("Unclosed VDF object")
        }
        Ok(m)
    }
    object(&tokens, &mut 0, false, 0)
}
fn roots(override_path: &str) -> Vec<PathBuf> {
    if !override_path.trim().is_empty() {
        return vec![PathBuf::from(override_path.trim())];
    }
    let mut roots = vec![];
    if let Some(p) = std::env::var_os("CANNA_STEAM_PATH") {
        roots.push(p.into());
    }
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::*};
        for (hive, key, value) in [
            (HKEY_CURRENT_USER, "Software\\Valve\\Steam", "SteamPath"),
            (
                HKEY_LOCAL_MACHINE,
                "SOFTWARE\\WOW6432Node\\Valve\\Steam",
                "InstallPath",
            ),
            (HKEY_LOCAL_MACHINE, "SOFTWARE\\Valve\\Steam", "InstallPath"),
        ] {
            if let Ok(k) = RegKey::predef(hive).open_subkey(key)
                && let Ok(p) = k.get_value::<String, _>(value)
            {
                roots.push(p.into());
            }
        }
        for env in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = std::env::var_os(env) {
                roots.push(PathBuf::from(p).join("Steam"));
            }
        }
    }
    roots
}
fn safe_dir(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['/', '\\', ':'])
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn loader(path: &Path) -> String {
    let core = path.join("BepInEx/core");
    let assembly = core.join("BepInEx.dll").is_file() || core.join("BepInEx.Core.dll").is_file();
    let proxy = path.join("winhttp.dll").is_file() || path.join("version.dll").is_file();
    if assembly && proxy && path.join("doorstop_config.ini").is_file() {
        "BepInEx detected"
    } else if path.join("BepInEx").exists() || assembly {
        "BepInEx incomplete"
    } else {
        "BepInEx not detected"
    }
    .into()
}
fn plugins(path: &Path, depth: usize) -> usize {
    if depth > 8 {
        return 0;
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| {
            let Ok(t) = e.file_type() else { return 0 };
            if t.is_symlink() {
                0
            } else if t.is_dir() {
                plugins(&e.path(), depth + 1)
            } else {
                usize::from(
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("dll")),
                )
            }
        })
        .sum()
}
fn icon(roots: &[PathBuf], id: u32) -> Option<Vec<u8>> {
    for root in roots {
        let cache = root.join("appcache/librarycache");
        for suffix in ["_library_600x900.jpg", "_header.jpg", "_icon.jpg"] {
            if let Ok(b) = std::fs::read(cache.join(format!("{id}{suffix}"))) {
                return Some(b);
            }
        }
        if let Ok(entries) = std::fs::read_dir(cache.join(id.to_string())) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().contains("library_600x900")
                    && let Ok(b) = std::fs::read(e.path())
                {
                    return Some(b);
                }
            }
        }
    }
    None
}
fn normalize_library(path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        PathBuf::from(
            resolved
                .to_string_lossy()
                .strip_prefix(r"\\?\")
                .unwrap_or(&resolved.to_string_lossy()),
        )
    }
    #[cfg(not(windows))]
    {
        resolved
    }
}
fn is_unity_game(path: &Path) -> bool {
    path.join("UnityPlayer.dll").is_file()
        && std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                entry.file_name().to_string_lossy().ends_with("_Data")
                    && entry
                        .file_type()
                        .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
            })
}
pub fn scan(override_path: &str) -> Scan {
    let roots = roots(override_path);
    let mut out = Scan::default();
    let mut libraries = BTreeSet::new();
    for root in &roots {
        if !root.join("steamapps").is_dir() {
            continue;
        }
        libraries.insert(normalize_library(root));
        let file = root.join("steamapps/libraryfolders.vdf");
        if let Ok(data) = std::fs::read_to_string(&file) {
            match parse(&data) {
                Ok(m) => {
                    if let Some(m) = m.get("libraryfolders").and_then(Value::map) {
                        for (key, v) in m {
                            if key.parse::<u32>().is_ok() {
                                let p = v.text().or_else(|| v.map()?.get("path")?.text());
                                if let Some(p) = p {
                                    libraries.insert(normalize_library(Path::new(p)));
                                }
                            }
                        }
                    }
                }
                Err(e) => out.warnings.push(format!("{}: {e}", file.display())),
            }
        }
    }
    let mut seen = BTreeSet::new();
    for lib in &libraries {
        let apps = lib.join("steamapps");
        let entries = match std::fs::read_dir(&apps) {
            Ok(e) => e,
            Err(e) => {
                out.warnings.push(format!("{}: {e}", apps.display()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let game = (|| -> Result<Option<InstalledGame>> {
                let m = parse(&std::fs::read_to_string(entry.path())?)?;
                let app = m
                    .get("appstate")
                    .and_then(Value::map)
                    .ok_or_else(|| anyhow::anyhow!("Missing AppState"))?;
                let get = |k| {
                    app.get(k)
                        .and_then(Value::text)
                        .ok_or_else(|| anyhow::anyhow!("Missing {k}"))
                };
                let id = get("appid")?.parse::<u32>()?;
                let dir = get("installdir")?;
                if !safe_dir(dir) {
                    bail!("Invalid install directory")
                }
                let path = apps.join("common").join(dir);
                if !path.is_dir() || seen.contains(&id) {
                    return Ok(None);
                }
                if !is_unity_game(&path)
                    && !crate::model::source_addons(id).is_some_and(|addons| {
                        path.join(addons)
                            .parent()
                            .is_some_and(|content| content.join("gameinfo.txt").is_file())
                    })
                {
                    seen.insert(id);
                    out.excluded += 1;
                    return Ok(None);
                }
                Ok(Some(InstalledGame {
                    app_id: id,
                    name: get("name")?.into(),
                    loader: if crate::model::source_addons(id).is_some() {
                        "Source VPK addons".into()
                    } else {
                        loader(&path)
                    },
                    plugins: plugins(&path.join("BepInEx/plugins"), 0),
                    path,
                    icon: icon(&roots, id),
                }))
            })();
            match game {
                Ok(Some(g)) => {
                    seen.insert(g.app_id);
                    out.games.push(g)
                }
                Ok(None) => {}
                Err(e) => out.warnings.push(format!("{name}: {e}")),
            }
        }
    }
    out.games
        .sort_by_key(|g| (g.app_id != 1686940, g.name.to_lowercase()));
    out.libraries = libraries.into_iter().collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modern_and_legacy_vdf() {
        let m =
            parse(r#""libraryfolders" { "0" { "path" "C:\\Steam" } "1" "D:\\Games" } // comment"#)
                .unwrap();
        let m = m["libraryfolders"].map().unwrap();
        assert_eq!(m["0"].map().unwrap()["path"].text(), Some("C:\\Steam"));
        assert_eq!(m["1"].text(), Some("D:\\Games"));
    }
    #[test]
    fn rejects_broken_vdf_and_traversal() {
        assert!(parse("\"x\" { \"y\" \"z\"").is_err());
        assert!(parse("\"unfinished").is_err());
        for p in ["..", "../escape", "C:\\Windows", "x/y", ""] {
            assert!(!safe_dir(p));
        }
        assert!(safe_dir("Bopl Battle"));
    }
    #[test]
    fn source_scan_requires_known_app_and_game_content() {
        let base = std::env::temp_dir().join(format!("canna-source-scan-{}", std::process::id()));
        let apps = base.join("steamapps");
        for (id, name, content) in [
            (550, "Left 4 Dead 2", "left4dead2"),
            (500, "Left 4 Dead", "left4dead"),
        ] {
            let path = apps.join("common").join(name).join(content);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("gameinfo.txt"), "fixture").unwrap();
            std::fs::write(apps.join(format!("appmanifest_{id}.acf")),format!("\"AppState\" {{ \"appid\" \"{id}\" \"name\" \"{name}\" \"installdir\" \"{name}\" }}")).unwrap();
        }
        let found = scan(base.to_str().unwrap());
        assert_eq!(found.games.len(), 2);
        assert!(found.games.iter().all(|g| g.loader == "Source VPK addons"));
        std::fs::remove_file(apps.join("common/Left 4 Dead 2/left4dead2/gameinfo.txt")).unwrap();
        assert_eq!(scan(base.to_str().unwrap()).games.len(), 1);
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn scans_secondary_library_and_loader() {
        let base = std::env::temp_dir().join(format!("canna-test-{}", std::process::id()));
        let root = base.join("steam");
        let other = base.join("secondary");
        let game = other.join("steamapps/common/Bopl Battle");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(game.join("BepInEx/core")).unwrap();
        std::fs::create_dir_all(game.join("BoplBattle_Data")).unwrap();
        std::fs::write(game.join("UnityPlayer.dll"), b"fixture").unwrap();
        let unreal = other.join("steamapps/common/Bodycam");
        std::fs::create_dir_all(unreal.join("Engine/Binaries/Win64")).unwrap();
        // A stray BepInEx directory must not turn an Unreal game into a Unity game.
        std::fs::create_dir_all(unreal.join("BepInEx/core")).unwrap();
        std::fs::write(
            other.join("steamapps/appmanifest_2406770.acf"),
            r#""AppState" { "appid" "2406770" "name" "Bodycam" "installdir" "Bodycam" }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"1\" {{ \"path\" {} }} }}",
                serde_json::to_string(&other.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
        std::fs::write(
            other.join("steamapps/appmanifest_1686940.acf"),
            r#""AppState" { "appid" "1686940" "name" "Bopl Battle" "installdir" "Bopl Battle" }"#,
        )
        .unwrap();
        let s = scan(root.to_str().unwrap());
        assert_eq!(s.games.len(), 1);
        assert_eq!(s.excluded, 1);
        assert_eq!(s.games[0].app_id, 1686940);
        assert_eq!(s.games[0].loader, "BepInEx incomplete");
        for p in [
            "BepInEx/core/BepInEx.dll",
            "winhttp.dll",
            "doorstop_config.ini",
        ] {
            std::fs::write(game.join(p), b"fixture").unwrap();
        }
        assert_eq!(
            scan(root.to_str().unwrap()).games[0].loader,
            "BepInEx detected"
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}

pub fn launch_practice(app_id: u32) -> Result<()> {
    if crate::model::source_addons(app_id).is_none() {
        bail!("Unsupported practice game");
    }
    let steam = roots("")
        .into_iter()
        .map(|root| root.join("steam.exe"))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            anyhow::anyhow!("Steam executable not found; check your Steam installation")
        })?;
    std::process::Command::new(steam)
        .args(["-applaunch", &app_id.to_string(), "-insecure", "-console"])
        .spawn()?;
    Ok(())
}
