//! Updates only exact approved project identities, retaining local/disabled selections.
use crate::{model::GameInfo, modpacks::Modpack};
use anyhow::Result;
fn identity(item: &crate::model::ModInfo) -> Option<(String, String, String)> {
    let p = &item.provenance;
    let provider = p["provider"].as_str()?;
    if !matches!(
        provider,
        "github" | "thunderstore" | "modrinth" | "curseforge" | "catalog"
    ) {
        return None;
    }
    let id = p["id"].as_str()?;
    let url = p["source_url"].as_str()?;
    if id.is_empty() || url.is_empty() {
        return None;
    }
    Some((provider.into(), id.into(), url.trim_end_matches('/').into()))
}
fn numeric(version: &str) -> Option<Vec<u64>> {
    version
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse().ok())
        .collect()
}
pub fn select(pack: &Modpack, games: &[GameInfo]) -> Result<(Modpack, usize)> {
    let mut updated = pack.clone();
    let mut count = 0;
    let Some(game) = games
        .iter()
        .find(|g| g.app_id == pack.game.app_id && g.folder == pack.game.folder)
    else {
        return Ok((updated, 0));
    };
    for item in &mut updated.mods {
        // Ticket downloads are local bytes, but their exact approved hash can
        // recover metadata. Ordinary local imports never acquire a catalog identity.
        let exact: Vec<_> = game
            .mods
            .iter()
            .filter(|m| {
                m.local_file.is_empty()
                    && m.provenance["external_only"] != true
                    && !item.sha256.is_empty()
                    && m.sha256 == item.sha256
                    && m.version == item.version
                    && m.name == item.name
                    && (m.file == item.file || item.provenance["canna_download"] == true)
            })
            .collect();
        if item.enabled
            && exact.len() == 1
            && (item.local_file.is_empty() || item.provenance["canna_download"] == true)
        {
            let local = item.local_file.clone();
            let enabled = item.enabled;
            *item = exact[0].clone();
            item.local_file = local;
            item.enabled = enabled;
        }
        if !pack.auto_update
            || !item.enabled
            || !item.local_file.is_empty()
            || item.provenance["compatibility_profile"].is_string()
        {
            continue;
        }
        let candidates: Vec<_> = game
            .mods
            .iter()
            .filter(|m| {
                m.local_file.is_empty()
                    && m.provenance["external_only"] != true
                    && m.provenance["compatibility_profile"].is_null()
                    && ((identity(item).is_some() && identity(item) == identity(m))
                        || (identity(item).is_none() && item.file == m.file))
            })
            .collect();
        if candidates.len() != 1 {
            continue;
        }
        let next = candidates[0];
        if next.version == item.version && next.sha256 == item.sha256 {
            continue;
        }
        if let (Some(old), Some(new)) = (numeric(&item.version), numeric(&next.version))
            && new < old
        {
            continue;
        }
        if next.sha256.len() != 64 {
            continue;
        }
        *item = next.clone();
        item.enabled = true;
        count += 1;
    }
    crate::dependencies::complete(&mut updated, game)?;
    updated.validate()?;
    Ok((updated, count))
}
pub fn refresh(pack: &Modpack, token: &str, progress: &dyn Fn(&str)) -> Result<Modpack> {
    if pack.mods.iter().all(|m| {
        !m.enabled
            || (!m.local_file.is_empty()
                && m.provenance["canna_download"] != true
                && m.dependencies.is_empty())
    }) {
        return Ok(pack.clone());
    }
    progress("Checking approved updates and required dependencies…");
    let catalog = crate::repository::sync(&crate::runtime::settings(pack), token)?;
    let (mut updated, count) = select(pack, &catalog.games)?;
    let added = updated.mods.len().saturating_sub(pack.mods.len());
    if let Some(game) = catalog
        .games
        .iter()
        .find(|g| g.app_id == pack.game.app_id && g.folder == pack.game.folder)
    {
        let mut resolution = crate::dependencies::complete(&mut updated, game)?;
        resolution.added = added;
        progress(&resolution.message());
    }
    progress(&format!(
        "{count} approved mod update(s) selected; checking compatibility before saving."
    ));
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, version: &str) -> crate::model::ModInfo {
        serde_json::from_value(serde_json::json!({"name":"Same display name","version":version,"file":format!("Mods/{id}-{version}.zip"),"sha256":"a".repeat(64),"provenance":{"provider":"thunderstore","id":id,"source_url":format!("https://thunderstore.io/c/rounds/p/{id}/")}})).unwrap()
    }
    fn pack(items: Vec<crate::model::ModInfo>) -> Modpack {
        Modpack::create(
            "Update fixture".into(),
            String::new(),
            &crate::model::bopl(),
            crate::cache::Source {
                owner: "fixture".into(),
                repository: "fixture".into(),
                branch: "main".into(),
                catalog_folder: String::new(),
            },
            items,
        )
    }
    #[test]
    fn prepare_repairs_nested_dependencies_even_with_version_updates_disabled() {
        let mut root = item("root", "1.0.0");
        root.name = "Root".into();
        root.dependencies = vec!["Library".into()];
        let mut library = item("library", "1.0.0");
        library.name = "Library".into();
        library.dependencies = vec!["Leaf".into()];
        let mut leaf = item("leaf", "1.0.0");
        leaf.name = "Leaf".into();
        let mut original = pack(vec![root.clone()]);
        original.auto_update = false;
        let mut game = crate::model::bopl();
        game.mods = vec![root, library, leaf];
        let (mut prepared, updates) = select(&original, &[game.clone()]).unwrap();
        assert_eq!(updates, 0);
        assert_eq!(prepared.mods.len(), 3);
        assert_eq!(prepared.mods[0].version, "1.0.0");
        let removed = prepared.mods[1].file.clone();
        prepared.remove_mod(&removed).unwrap();
        let (again, _) = select(&prepared, &[game]).unwrap();
        assert_eq!(again.mods.len(), 2);
        assert!(!again.mods.iter().any(|m| m.name == "Library"));
    }
    #[test]
    fn ticket_metadata_requires_exact_hash_version_name_and_game() {
        let mut catalog = item("root", "1.0.0");
        catalog.name = "Root".into();
        catalog.dependencies = vec!["Library".into()];
        let mut library = item("library", "1.0.0");
        library.name = "Library".into();
        let mut ticket = catalog.clone();
        ticket.file = format!("Mods/{}.zip", ticket.sha256);
        ticket.local_file = format!("{}.zip", ticket.sha256);
        ticket.dependencies.clear();
        ticket.provenance = serde_json::json!({"canna_download":true});
        let mut game = crate::model::bopl();
        game.mods = vec![catalog.clone(), library];
        let (prepared, _) = select(&pack(vec![ticket.clone()]), &[game.clone()]).unwrap();
        assert_eq!(prepared.mods.len(), 2);
        assert_eq!(prepared.mods[0].local_file, ticket.local_file);
        assert_eq!(prepared.mods[0].provenance, catalog.provenance);
        for mutation in 0..5 {
            let mut other = ticket.clone();
            let mut other_game = game.clone();
            match mutation {
                0 => {
                    other.sha256 = "b".repeat(64);
                    other.local_file = format!("{}.zip", other.sha256);
                    other.file = format!("Mods/{}", other.local_file);
                }
                1 => other.version = "different".into(),
                2 => other.name = "Lookalike".into(),
                3 => other.provenance = serde_json::Value::Null,
                _ => other_game.app_id = 1557740,
            }
            assert_eq!(
                select(&pack(vec![other]), &[other_game])
                    .unwrap()
                    .0
                    .mods
                    .len(),
                1
            );
        }
    }
    #[test]
    fn exact_project_update_preserves_disabled_local_and_original_selections() {
        let first = item("one", "1.0.0");
        let mut disabled = item("two", "1.0.0");
        disabled.enabled = false;
        let mut local = item("three", "1.0.0");
        local.local_file = format!("{}.zip", local.sha256);
        let original = pack(vec![first, disabled.clone(), local.clone()]);
        let mut game = crate::model::bopl();
        game.mods = vec![
            item("one", "2.0.0"),
            item("two", "2.0.0"),
            item("three", "2.0.0"),
            item("lookalike", "99.0.0"),
        ];
        let (updated, count) = select(&original, &[game]).unwrap();
        assert_eq!(count, 1);
        assert_eq!(updated.mods[0].version, "2.0.0");
        assert_eq!(original.mods[0].version, "1.0.0");
        assert_eq!(
            serde_json::to_value(&updated.mods[1]).unwrap(),
            serde_json::to_value(disabled).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&updated.mods[2]).unwrap(),
            serde_json::to_value(local).unwrap()
        );
    }
    #[test]
    fn ambiguous_downgraded_external_and_unhashed_candidates_are_not_updates() {
        let original = pack(vec![item("one", "2.0.0")]);
        let mut game = crate::model::bopl();
        for candidates in [
            vec![item("one", "1.0.0")],
            vec![item("one", "3.0.0"), item("one", "4.0.0")],
            vec![{
                let mut m = item("one", "3.0.0");
                m.sha256.clear();
                m
            }],
            vec![{
                let mut m = item("one", "3.0.0");
                m.provenance["external_only"] = true.into();
                m
            }],
        ] {
            game.mods = candidates;
            assert_eq!(select(&original, &[game.clone()]).unwrap().1, 0);
        }
        let mut pinned = original;
        pinned.auto_update = false;
        game.mods = vec![item("one", "3.0.0")];
        assert_eq!(select(&pinned, &[game]).unwrap().1, 0);
    }
    #[test]
    fn older_pack_manifests_default_to_automatic_updates() {
        let mut old = serde_json::to_value(pack(vec![])).unwrap();
        old.as_object_mut().unwrap().remove("auto_update");
        assert!(serde_json::from_value::<Modpack>(old).unwrap().auto_update);
    }
}
