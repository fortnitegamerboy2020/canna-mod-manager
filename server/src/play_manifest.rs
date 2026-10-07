//! Portable, allowlisted play manifests. Never include paths, credentials or device identifiers.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub game: u32,
    pub branch: String,
    pub build: String,
    pub loader: String,
    pub manager: String,
    pub mods: Vec<Project>,
    #[serde(default)]
    pub shared_configs: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub version: String,
    pub sha256: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    pub level: String,
    pub message: String,
}
fn text(value: &str, max: usize) -> bool {
    value.len() <= max
        && value
            .chars()
            .all(|c| !c.is_control() && !"/\\@<>".contains(c))
}
fn hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
impl Manifest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if serde_json::to_vec(self)
            .map_err(|_| "Manifest encoding failed")?
            .len()
            > 256 * 1024
        {
            return Err("Manifest exceeds 256 KiB");
        }
        if self.game == 0
            || self.mods.len() > 1000
            || self.shared_configs.len() > 100
            || !text(&self.branch, 80)
            || !text(&self.build, 40)
            || !text(&self.loader, 80)
            || !text(&self.manager, 30)
        {
            return Err("Invalid game or manifest limits");
        }
        let mut names = BTreeSet::new();
        for item in &self.mods {
            if item.name.is_empty()
                || !text(&item.name, 200)
                || !text(&item.version, 100)
                || item.version.is_empty()
                || (!item.sha256.is_empty() && !hash(&item.sha256))
                || item.dependencies.len() > 32
                || !names.insert(item.name.to_lowercase())
                || item
                    .dependencies
                    .iter()
                    .any(|d| d.is_empty() || !text(d, 200))
            {
                return Err("Invalid or duplicate project");
            }
        }
        if self
            .shared_configs
            .iter()
            .any(|(name, value)| name.is_empty() || !text(name, 100) || !hash(value))
        {
            return Err("Invalid shared config digest");
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> Result<String, &'static str> {
        self.validate()?;
        let mut normalized = self.clone();
        normalized.mods.sort_by_key(|m| m.name.to_lowercase());
        for item in &mut normalized.mods {
            item.sha256.make_ascii_lowercase();
            item.dependencies.sort();
            item.dependencies.dedup();
        }
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&normalized).map_err(|_| "Manifest encoding failed")?
            )
        ))
    }
    pub fn preflight(&self) -> Vec<Check> {
        let mut checks = vec![];
        if let Err(error) = self.validate() {
            checks.push(check("blocked", error));
            return checks;
        }
        if self.build.is_empty() || self.branch.is_empty() {
            checks.push(check(
                "unknown",
                "Game build or branch has not been measured",
            ));
        }
        if self.loader.is_empty() || self.loader == "Not detected" {
            checks.push(check("unknown", "Loader has not been verified"));
        }
        if self.loader.to_lowercase().contains("incomplete") {
            checks.push(check("blocked", "Loader installation is incomplete"));
        }
        if self.manager.is_empty() {
            checks.push(check("unknown", "Canna version has not been recorded"));
        }
        let present: BTreeSet<_> = self.mods.iter().map(|m| m.name.as_str()).collect();
        for item in &self.mods {
            if item.sha256.is_empty() {
                checks.push(check(
                    "unknown",
                    &format!("{} has no verified archive digest", item.name),
                ));
            }
            for dependency in &item.dependencies {
                if !present.contains(dependency.as_str()) {
                    checks.push(check(
                        "blocked",
                        &format!("{} requires {dependency}", item.name),
                    ));
                }
            }
        }
        if checks.is_empty() {
            checks.push(check("matched", "Manifest is internally consistent; a successful game session is still a separate check"));
        }
        checks
    }
}
fn check(level: &str, message: &str) -> Check {
    Check {
        level: level.into(),
        message: message.into(),
    }
}
pub fn compare(expected: &Manifest, actual: &Manifest) -> Vec<Check> {
    let mut result = actual.preflight();
    if expected.validate().is_err() {
        return vec![check("blocked", "Host manifest is invalid")];
    }
    for (label, a, b) in [
        ("Game", expected.game.to_string(), actual.game.to_string()),
        ("Branch", expected.branch.clone(), actual.branch.clone()),
        ("Build", expected.build.clone(), actual.build.clone()),
        ("Loader", expected.loader.clone(), actual.loader.clone()),
        (
            "Canna version",
            expected.manager.clone(),
            actual.manager.clone(),
        ),
    ] {
        if a != b {
            result.push(check(
                "blocked",
                &format!("{label} differs: expected {a}, found {b}"),
            ));
        }
    }
    let actual_mods: BTreeMap<_, _> = actual.mods.iter().map(|m| (m.name.as_str(), m)).collect();
    let expected_names: BTreeSet<_> = expected.mods.iter().map(|m| m.name.as_str()).collect();
    for item in &expected.mods {
        match actual_mods.get(item.name.as_str()) {
            None => result.push(check(
                "blocked",
                &format!("Missing {} {}", item.name, item.version),
            )),
            Some(found)
                if found.version != item.version
                    || !found.sha256.eq_ignore_ascii_case(&item.sha256)
                    || found.dependencies.iter().collect::<BTreeSet<_>>()
                        != item.dependencies.iter().collect::<BTreeSet<_>>() =>
            {
                result.push(check(
                    "blocked",
                    &format!("{} version, digest or dependencies differ", item.name),
                ))
            }
            _ => (),
        }
    }
    for item in &actual.mods {
        if !expected_names.contains(item.name.as_str()) {
            result.push(check("blocked", &format!("Extra project: {}", item.name)));
        }
    }
    if expected.shared_configs != actual.shared_configs {
        result.push(check(
            "blocked",
            "Selected shared configuration digests differ",
        ));
    }
    result
}

/// Conservative log preview: discard sensitive-looking lines rather than promise perfect anonymization.
pub fn redact(input: &str) -> String {
    let mut output = String::new();
    for line in input.lines().take(2000) {
        let lower = line.to_lowercase();
        let sensitive = lower.contains("session=")
            || lower.contains("session:")
            || line.contains('@')
            || line.contains('\\')
            || line.contains(":/")
            || line.contains('/')
            || line.contains("::")
            || [
                "token",
                "password",
                "authorization",
                "cookie",
                "secret",
                "sessionid",
                "session_id",
                "steamid",
                "hardware",
                "username",
                "hostname",
                "mac address",
                "email",
                "computername",
                "api_key",
                "apikey",
            ]
            .iter()
            .any(|word| lower.contains(word))
            || line
                .split(|c: char| !c.is_ascii_hexdigit() && c != '.' && c != ':' && c != '-')
                .any(|word| {
                    word.split('.').count() == 4 && word.split('.').all(|p| p.parse::<u8>().is_ok())
                        || word.len() >= 16 && word.bytes().all(|b| b.is_ascii_hexdigit())
                        || word.matches(':').count() >= 2
                        || word.matches('-').count() >= 5
                });
        if sensitive {
            output.push_str("[private line removed]\n");
        } else {
            output.extend(line.chars().filter(|c| !c.is_control()).take(500));
            output.push('\n');
        }
        if output.len() >= 32_000 {
            break;
        }
    }
    output
}
// Diagnostics run on the Windows client. The Linux server never receives raw logs.
#[cfg(any(test, windows))]
pub fn diagnose(log: &str) -> Vec<Check> {
    let lower = log.to_lowercase();
    let mut result = vec![];
    for (needles, message) in [
        (
            &[
                "missing dependency",
                "could not load file or assembly",
                "mod resolution failed",
            ][..],
            "Check missing dependencies and their required versions",
        ),
        (
            &["outofmemory", "out of memory", "java heap space"][..],
            "Memory exhaustion was reported; try a smaller pack and inspect the game's memory settings",
        ),
        (
            &["access denied", "permission denied", "sharing violation"][..],
            "A file is locked or access is denied; close the game and check folder permissions",
        ),
        (
            &["checksum mismatch", "hash mismatch"][..],
            "A downloaded file failed integrity verification; retrieve the exact approved release again",
        ),
        (
            &["incompatible", "version mismatch"][..],
            "Compare game build, loader, mods and shared settings against the working setup",
        ),
        (
            &["disk full", "no space left"][..],
            "Free disk space before retrying; retain the last working snapshot",
        ),
    ] {
        if needles.iter().any(|needle| lower.contains(needle)) {
            result.push(check("suggestion", message));
        }
    }
    if result.is_empty() {
        result.push(check(
            "unknown",
            "No recognized failure pattern. Compare recent changes or use an isolated test copy",
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> Manifest {
        Manifest {
            game: 550,
            branch: "public".into(),
            build: "123".into(),
            loader: "source-vpk".into(),
            manager: "test".into(),
            mods: vec![Project {
                name: "Hop".into(),
                version: "1".into(),
                sha256: "a".repeat(64),
                dependencies: vec![],
            }],
            shared_configs: BTreeMap::new(),
        }
    }
    #[test]
    fn fingerprint_ignores_order_but_not_versions() {
        let mut a = manifest();
        let mut b = a.clone();
        a.mods.push(Project {
            name: "Other".into(),
            ..a.mods[0].clone()
        });
        b.mods = a.mods.iter().rev().cloned().collect();
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.mods[0].version = "2".into();
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
    #[test]
    fn unknown_does_not_become_ready() {
        let mut a = manifest();
        a.build.clear();
        assert!(compare(&a, &a).iter().any(|c| c.level == "unknown"));
    }
    #[test]
    fn every_mismatch_blocks() {
        let a = manifest();
        for field in 0..7 {
            let mut b = a.clone();
            match field {
                0 => b.game = 500,
                1 => b.branch = "beta".into(),
                2 => b.build = "124".into(),
                3 => b.loader = "other".into(),
                4 => b.manager = "old".into(),
                5 => b.mods.clear(),
                _ => b.mods[0].sha256 = "b".repeat(64),
            }
            assert!(compare(&a, &b).iter().any(|c| c.level == "blocked"));
        }
    }
    #[test]
    fn dependency_and_extra_config_checks() {
        let a = manifest();
        let mut b = a.clone();
        b.mods[0].dependencies.push("Missing".into());
        assert!(b.preflight().iter().any(|c| c.level == "blocked"));
        b.shared_configs.insert("shared".into(), "a".repeat(64));
        assert!(compare(&a, &b).iter().any(|c| c.level == "blocked"));
    }
    #[test]
    fn private_fields_are_rejected() {
        let mut v = serde_json::to_value(manifest()).unwrap();
        v["device_name"] = "Alice-PC".into();
        assert!(serde_json::from_value::<Manifest>(v).is_err());
        let mut a = manifest();
        a.mods[0].name = "C:/Users/Alice".into();
        assert!(a.validate().is_err());
    }
    #[test]
    fn redaction_and_diagnosis_are_separate() {
        let value = redact(
            "Loading Hop\nC:\\Users\\Alice\\game\nIP 192.168.1.4\nBearer token=private\nuser@example.test\nSteamID 76561198000000000\nMAC AA:BB:CC:DD:EE:FF\nIPv6 2001:db8::1\nOut of memory",
        );
        assert!(!value.contains("Alice"));
        assert!(!value.contains("192.168"));
        assert!(!value.contains("example"));
        assert!(value.contains("Loading Hop"));
        assert_eq!(diagnose(&value)[0].level, "suggestion");
    }
    #[test]
    fn semantic_dependency_order_and_case_do_not_invent_mismatches() {
        let mut a = manifest();
        a.mods[0].dependencies = vec!["One".into(), "Two".into()];
        let mut b = a.clone();
        b.mods[0].dependencies.reverse();
        b.mods[0].sha256 = b.mods[0].sha256.to_uppercase();
        assert_eq!(a.fingerprint().unwrap(), b.fingerprint().unwrap());
        assert!(
            !compare(&a, &b)
                .iter()
                .any(|c| c.message.contains("mod differs"))
        );
    }
    #[test]
    fn nested_metadata_cannot_exceed_the_manifest_budget() {
        let mut value = manifest();
        let template = value.mods[0].clone();
        value.mods = (0..1000)
            .map(|i| {
                let mut m = template.clone();
                m.name = format!("Project {i}");
                m.dependencies = (0..32).map(|n| format!("{n}{}", "A".repeat(90))).collect();
                m
            })
            .collect();
        assert!(value.validate().is_err());
    }
    #[test]
    fn maximum_size_setup_has_bounded_dependency_comparison() {
        let mut m = manifest();
        let template = m.mods[0].clone();
        m.mods = (0..1000)
            .map(|i| {
                let mut item = template.clone();
                item.name = format!("Mod {i}");
                if i > 0 {
                    item.dependencies = vec![format!("Mod {}", i - 1)]
                };
                item
            })
            .collect();
        assert!(m.validate().is_ok());
        assert!(compare(&m, &m).iter().all(|c| c.level == "matched"));
        m.mods[999].dependencies.push("Unavailable".into());
        assert!(m.preflight().iter().any(|c| c.level == "blocked"));
    }
}
