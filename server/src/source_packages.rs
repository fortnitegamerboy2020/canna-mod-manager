//! Updates only the licensed, curated Source packages, never arbitrary GitHub projects.
use super::*;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
};
pub async fn refresh(
    app: &App,
    user: i64,
    data: &Value,
    origin: &str,
) -> ApiResult<Option<String>> {
    let repo = data["source_url"]
        .as_str()
        .unwrap_or_default()
        .strip_prefix("https://github.com/")
        .ok_or_else(|| bad("Unsupported source project"))?;
    let (game, practice) = match repo {
        "originalgrego/L4D2-Practice-Script" => (550, true),
        "jpobzy/L4dAutoConfig" | "jpobzy/L4dRemovedMainMenuMusic" => (500, false),
        _ => return Err(bad("Unsupported source project")),
    };
    let info = external::metadata(&format!("https://api.github.com/repos/{repo}"), false).await?;
    let license = info["license"]["spdx_id"].as_str().unwrap_or_default();
    if !matches!(license, "MIT" | "GPL-2.0") {
        return Err(bad("Source license changed; manual review required"));
    }
    let branch = info["default_branch"].as_str().unwrap_or_default();
    if branch.is_empty()
        || !branch
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    {
        return Err(bad("Unsupported source branch"));
    }
    let commit = external::metadata(
        &format!("https://api.github.com/repos/{repo}/commits/{branch}"),
        false,
    )
    .await?;
    let sha = commit["sha"].as_str().unwrap_or_default();
    if sha.len() != 40 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad("Invalid source revision"));
    }
    let revision = if repo == "jpobzy/L4dRemovedMainMenuMusic" {
        ":vpk2"
    } else {
        ""
    };
    let new_origin = format!("github:{repo}:{sha}{revision}");
    if new_origin == origin {
        return Ok(None);
    }
    if let Some(id) = app
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT mod_id FROM mod_details WHERE origin=?1",
            [&new_origin],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(Some(id));
    }
    let mut response = external::client()?
        .get(format!("https://codeload.github.com/{repo}/zip/{sha}"))
        .send()
        .await
        .map_err(|_| bad("Source download failed"))?;
    if !response.status().is_success() {
        return Err(bad("Source download refused"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| bad("Source download interrupted"))?
    {
        if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err(bad("Source archive exceeds limits"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let packed = package(&bytes, practice)?;
    let mut details = data.clone();
    details["commit"] = json!(sha);
    details["license"] = json!(license);
    let id = external::store(
        app,
        user,
        game,
        info["name"]
            .as_str()
            .ok_or_else(|| bad("Invalid project name"))?,
        &format!(
            "{}{}",
            &sha[..12],
            if revision.is_empty() { "" } else { ".2" }
        ),
        info["description"].as_str().unwrap_or_default(),
        &new_origin,
        &details,
        &packed,
    )
    .await?;
    Ok(Some(id))
}
fn package(bytes: &[u8], practice: bool) -> ApiResult<Vec<u8>> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| bad("Invalid source ZIP"))?;
    if zip.len() > 1000 {
        return Err(bad("Source archive exceeds limits"));
    }
    let mut source = BTreeMap::new();
    let mut names = std::collections::HashSet::new();
    let mut total = 0;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|_| bad("Invalid source file"))?;
        if file.is_dir() {
            continue;
        }
        if file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(bad("Source symlink rejected"));
        }
        let name = file
            .name()
            .split_once('/')
            .ok_or_else(|| bad("Invalid source path"))?
            .1
            .to_owned();
        if name.contains(['\\', ':', '\0'])
            || name
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || !names.insert(name.to_lowercase())
        {
            return Err(bad("Unsafe or duplicate source path"));
        }
        if file.size() > 4 * 1024 * 1024 {
            return Err(bad("Source file exceeds limits"));
        }
        let mut raw = Vec::new();
        Read::by_ref(&mut file)
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)?;
        total += raw.len();
        if raw.len() > 4 * 1024 * 1024 || total > 16 * 1024 * 1024 {
            return Err(bad("Expanded source exceeds limits"));
        }
        source.insert(name, raw);
    }
    let license = source
        .get("LICENSE")
        .ok_or_else(|| bad("Source license missing"))?;
    let readme = source
        .get("README.md")
        .ok_or_else(|| bad("Source README missing"))?;
    let mut files = BTreeMap::new();
    for (name, raw) in &source {
        files.insert(format!("canna-source/{name}"), raw.clone());
    }
    if practice {
        files.insert(
            "cfg/l4d2_practice.cfg".into(),
            source
                .get("l4d2_practice.cfg")
                .ok_or_else(|| bad("Practice configuration missing"))?
                .clone(),
        );
        files.insert("addoninfo.txt".into(),br#""AddonInfo" { "addonSteamAppID" "550" "addontitle" "L4D2 Practice Script" "addonauthor" "originalgrego" }"#.to_vec());
    } else {
        for (name, raw) in &source {
            if !name.starts_with('.') && !matches!(name.as_str(), "LICENSE" | "README.md") {
                files.insert(name.clone(), raw.clone());
            }
        }
    }
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let original_vpk = source.get("removedmainlobbymusic.vpk");
    let addon = if let Some(raw) = original_vpk {
        if !raw.starts_with(&0x55aa1234u32.to_le_bytes()) {
            return Err(bad("Invalid original VPK"));
        }
        raw.clone()
    } else {
        vpk(&files)
    };
    for (name, raw) in [
        ("addon.vpk", addon),
        ("LICENSE", license.clone()),
        ("README.md", readme.clone()),
    ] {
        output
            .start_file(name, options)
            .map_err(|_| bad("Package creation failed"))?;
        output.write_all(&raw)?;
    }
    if original_vpk.is_some() {
        for (name, raw) in &source {
            output
                .start_file(format!("canna-source/{name}"), options)
                .map_err(|_| bad("Source preservation failed"))?;
            output.write_all(raw)?;
        }
    }
    Ok(output
        .finish()
        .map_err(|_| bad("Package creation failed"))?
        .into_inner())
}
fn vpk(files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    type PackageGroups<'a> = BTreeMap<String, BTreeMap<String, Vec<(String, &'a Vec<u8>)>>>;
    let mut groups: PackageGroups<'_> = BTreeMap::new();
    for (path, raw) in files {
        let (dir, file) = path.rsplit_once('/').unwrap_or((" ", path));
        let (name, ext) = file.rsplit_once('.').unwrap_or((file, " "));
        groups
            .entry(ext.into())
            .or_default()
            .entry(dir.into())
            .or_default()
            .push((name.into(), raw));
    }
    let mut tree: Vec<u8> = Vec::new();
    let mut payload: Vec<u8> = Vec::new();
    for (ext, dirs) in groups {
        tree.extend(ext.as_bytes());
        tree.push(0);
        for (dir, entries) in dirs {
            tree.extend(dir.as_bytes());
            tree.push(0);
            for (name, raw) in entries {
                tree.extend(name.as_bytes());
                tree.push(0);
                tree.extend(crc32fast::hash(raw).to_le_bytes());
                tree.extend(0u16.to_le_bytes());
                tree.extend(0x7fffu16.to_le_bytes());
                tree.extend((payload.len() as u32).to_le_bytes());
                tree.extend((raw.len() as u32).to_le_bytes());
                tree.extend(0xffffu16.to_le_bytes());
                payload.extend(raw);
            }
            tree.push(0);
        }
        tree.push(0);
    }
    tree.push(0);
    let mut result = Vec::new();
    for field in [0x55aa1234u32, 1, tree.len() as u32] {
        result.extend(field.to_le_bytes());
    }
    result.extend(tree);
    result.extend(payload);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Live official GitHub metadata and package update in isolated temporary encrypted storage"]
    async fn curated_update_imports_new_revision_then_reports_current() {
        let (_dir, app) = crate::tests::fixture();
        crate::tests::account(&app, "fixture-owner", true);
        let d = json!({"provider":"github","source_url":"https://github.com/originalgrego/L4D2-Practice-Script","game":"Left 4 Dead 2"});
        let id = refresh(
            &app,
            1,
            &d,
            "github:originalgrego/L4D2-Practice-Script:previous-fixture",
        )
        .await
        .unwrap()
        .unwrap();
        let origin: String = app
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT origin FROM mod_details WHERE mod_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(refresh(&app, 1, &d, &origin).await.unwrap().is_none());
        crate::catalog::audit(&app).await.unwrap();
    }
    fn archive(path: &str) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in ["root/LICENSE", "root/README.md", path] {
            w.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(b"original bytes").unwrap();
        }
        w.finish().unwrap().into_inner()
    }
    #[test]
    fn package_preserves_original_source_and_license() {
        let result = package(&archive("root/l4d2_practice.cfg"), true).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(result)).unwrap();
        let mut bytes = Vec::new();
        z.by_name("addon.vpk")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(&bytes[..4], &0x55aa1234u32.to_le_bytes());
        assert!(bytes.windows(12).any(|w| w == b"canna-source"));
        assert_eq!(z.by_name("LICENSE").unwrap().size(), 14);
    }
    #[test]
    fn rejects_traversal() {
        assert!(package(&archive("root/../evil.cfg"), false).is_err());
    }
}
