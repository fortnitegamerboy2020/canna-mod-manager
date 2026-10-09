//! Select required catalog packages without changing explicit local/disabled choices.
use crate::{
    model::{GameInfo, ModInfo},
    modpacks::Modpack,
};
use anyhow::Result;
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Resolution {
    pub added: usize,
    pub unavailable: BTreeSet<String>,
}
impl Resolution {
    pub fn message(&self) -> String {
        let mut text = format!("{} required dependency package(s) added", self.added);
        if !self.unavailable.is_empty() {
            text.push_str(&format!(
                "; unavailable or ambiguous in the approved catalog: {}",
                self.unavailable
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        text
    }
}
fn approved(item: &ModInfo) -> bool {
    item.local_file.is_empty()
        && item.provenance["external_only"] != true
        && !item.provenance["framework_root"].is_string()
        && item.sha256.len() == 64
        && item.sha256.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn complete(pack: &mut Modpack, game: &GameInfo) -> Result<Resolution> {
    anyhow::ensure!(
        pack.game.app_id == game.app_id && pack.game.folder == game.folder,
        "Dependency catalog belongs to a different game"
    );
    let mut next = pack.clone();
    let mut result = Resolution::default();
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < next.mods.len() {
        let parent = next.mods[index].clone();
        index += 1;
        if !parent.enabled || parent.provenance["compatibility_profile"].is_string() {
            continue;
        }
        for name in &parent.dependencies {
            if !seen.insert(name.clone())
                || next.ignored_dependencies.contains(name)
                || next.mods.iter().any(|m| m.name == *name)
            {
                continue;
            }
            let mut candidates: Vec<_> = game
                .mods
                .iter()
                .filter(|m| m.name == *name && approved(m))
                .collect();
            // Provider imports retain exact reviewed archive IDs. A same-name
            // package from another project must never satisfy that graph.
            if let Some(ids) = parent.provenance["dependency_ids"].as_array() {
                candidates.retain(|m| {
                    ids.iter()
                        .filter_map(serde_json::Value::as_str)
                        .any(|id| m.file == format!("Mods/{id}.zip"))
                });
            }
            if candidates.len() != 1 {
                result.unavailable.insert(name.clone());
                continue;
            }
            anyhow::ensure!(
                next.mods.len() < 1000,
                "Required dependencies exceed the modpack limit"
            );
            let mut item = candidates[0].clone();
            item.enabled = true;
            next.mods.push(item);
            result.added += 1;
        }
    }
    next.validate()?;
    *pack = next;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(name: &str, deps: &[&str]) -> ModInfo {
        serde_json::from_value(serde_json::json!({"name":name,"version":"1","file":format!("Mods/{name}.zip"),"sha256":"a".repeat(64),"dependencies":deps})).unwrap()
    }
    fn pack(mods: Vec<ModInfo>) -> Modpack {
        Modpack::create(
            "Dependencies".into(),
            String::new(),
            &crate::model::bopl(),
            crate::cache::Source {
                owner: "fixture".into(),
                repository: "mods".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            },
            mods,
        )
    }
    #[test]
    fn recursive_diamonds_cycles_and_repeat_preparation_are_bounded() {
        let mut game = crate::model::bopl();
        game.mods = vec![
            item("A", &["B", "C"]),
            item("B", &["D"]),
            item("C", &["D"]),
            item("D", &["A"]),
        ];
        let mut p = pack(vec![game.mods[0].clone()]);
        assert_eq!(complete(&mut p, &game).unwrap().added, 3);
        assert_eq!(p.mods.len(), 4);
        assert!(p.mods.iter().all(|m| m.enabled));
        assert_eq!(complete(&mut p, &game).unwrap().added, 0);
    }
    #[test]
    fn manual_disabled_local_and_removed_choices_survive_resolution_and_roundtrip() {
        let mut game = crate::model::bopl();
        game.mods = vec![
            item("Root", &["Disabled", "Local", "Removed", "Required"]),
            item("Disabled", &[]),
            item("Local", &[]),
            item("Removed", &[]),
            item("Required", &[]),
        ];
        let mut disabled = game.mods[1].clone();
        disabled.enabled = false;
        let mut local = game.mods[2].clone();
        local.local_file = format!("{}.zip", local.sha256);
        let mut p = pack(vec![
            game.mods[0].clone(),
            disabled.clone(),
            local.clone(),
            game.mods[3].clone(),
        ]);
        p.remove_mod(&game.mods[3].file).unwrap();
        let mut p: Modpack = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
        assert_eq!(complete(&mut p, &game).unwrap().added, 1);
        assert_eq!(
            serde_json::to_value(&p.mods[1]).unwrap(),
            serde_json::to_value(disabled).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&p.mods[2]).unwrap(),
            serde_json::to_value(local).unwrap()
        );
        assert!(!p.mods.iter().any(|m| m.name == "Removed"));
        p.ignored_dependencies.clear();
        assert_eq!(complete(&mut p, &game).unwrap().added, 1);
    }
    #[test]
    fn missing_ambiguous_unapproved_and_wrong_identity_are_not_guessed() {
        let mut root = item(
            "Root",
            &[
                "Missing",
                "Ambiguous",
                "External",
                "Unhashed",
                "Wrong",
                "Right",
            ],
        );
        root.provenance = serde_json::json!({"dependency_ids":["Ambiguous","other","External","Unhashed","Right"]});
        let mut game = crate::model::bopl();
        let mut external = item("External", &[]);
        external.provenance = serde_json::json!({"external_only":true});
        let mut unhashed = item("Unhashed", &[]);
        unhashed.sha256.clear();
        game.mods = vec![
            item("Ambiguous", &[]),
            item("Ambiguous", &[]),
            external,
            unhashed,
            item("Wrong", &[]),
            item("Right", &[]),
        ];
        let mut p = pack(vec![root]);
        let result = complete(&mut p, &game).unwrap();
        assert_eq!(result.added, 1);
        assert_eq!(result.unavailable.len(), 5);
        assert_eq!(p.mods[1].name, "Right");
        let before = serde_json::to_value(&p).unwrap();
        game.app_id = 1557740;
        assert!(complete(&mut p, &game).is_err());
        assert_eq!(serde_json::to_value(p).unwrap(), before);
    }
}
