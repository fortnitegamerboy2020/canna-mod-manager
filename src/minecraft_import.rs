//! Manager exports become new Canna instances; launcher commands and account data
//! are never imported. Inspection is read-only, and failed downloads leave no instance.
use super::*;
use anyhow::ensure;
use sha2::{Digest, Sha512};
use std::collections::BTreeSet;

#[derive(Clone)]
enum Content {
    Bundled(Vec<u8>),
    Download {
        urls: Vec<String>,
        sha1: String,
        sha512: String,
        size: u64,
    },
}
#[derive(Clone)]
struct File {
    path: String,
    content: Content,
    optional: bool,
}
#[derive(Clone)]
pub(super) struct Plan {
    pub instance: Instance,
    pub kind: &'static str,
    pub notes: Vec<String>,
    pub include_optional: bool,
    files: Vec<File>,
}
impl Plan {
    pub fn count(&self) -> usize {
        self.files
            .iter()
            .filter(|f| !f.optional || self.include_optional)
            .count()
    }
    pub fn optional_count(&self) -> usize {
        self.files.iter().filter(|f| f.optional).count()
    }
}
fn identifier(value: &str) -> bool {
    value.len() <= 100 && safe_component(value)
}
fn content_path(path: &str) -> bool {
    if relative(Path::new("."), path).is_err() || path.contains('\\') || path.len() > 240 {
        return false;
    }
    // Reuse Windows reserved-name checks; never accept a program at the instance root.
    if !crate::pack_configs::safe_path(&format!("{path}.txt")) {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    if lower.starts_with("mods/") {
        return lower.ends_with(".jar") || lower.ends_with(".jar.disabled");
    }
    if lower.starts_with("resourcepacks/") || lower.starts_with("shaderpacks/") {
        return lower.ends_with(".zip");
    }
    if let Some(config) = path.strip_prefix("config/") {
        return crate::pack_configs::safe_path(config);
    }
    lower == "options.txt"
}
fn download_url(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("cdn.modrinth.com")
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.path().starts_with("/data/"),
        "Pack download must use the Modrinth content CDN"
    );
    Ok(url)
}
fn hash(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn instance(
    name: String,
    version: String,
    loader: String,
    loader_version: String,
) -> Result<Instance> {
    ensure!(
        !name.trim().is_empty() && name.len() <= 100,
        "Invalid imported instance name"
    );
    ensure!(
        identifier(&version)
            && ["vanilla", "fabric", "quilt", "forge", "neoforge"].contains(&loader.as_str())
            && (loader == "vanilla" || identifier(&loader_version)),
        "Unsupported game or loader version"
    );
    let loader_version = if loader == "forge" {
        loader_version
            .strip_prefix(&format!("{version}-"))
            .unwrap_or(&loader_version)
            .to_owned()
    } else {
        loader_version
    };
    Ok(Instance {
        id: format!(
            "import-{:x}-{:x}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ),
        name,
        version,
        loader,
        loader_version,
        java: String::new(),
        memory: 4096,
    })
}
pub(super) fn inspect(path: &Path) -> Result<Plan> {
    crate::runtime::no_links(path)?;
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "Minecraft pack exceeds 128 MiB"
    );
    inspect_entries(crate::runtime::archive_files(&bytes)?)
}
fn inspect_entries(mut entries: Vec<(PathBuf, Vec<u8>)>) -> Result<Plan> {
    let manifests = ["modrinth.index.json", "mmc-pack.json", "manifest.json"];
    let found = entries
        .iter()
        .filter(|(p, _)| {
            p.file_name()
                .is_some_and(|n| manifests.iter().any(|m| n == *m))
        })
        .collect::<Vec<_>>();
    ensure!(
        found.len() == 1,
        "Choose one Modrinth or Prism/MultiMC instance export"
    );
    let parent = found[0].0.parent().unwrap_or(Path::new("")).to_path_buf();
    if !parent.as_os_str().is_empty() {
        ensure!(
            entries.iter().all(|(p, _)| p.starts_with(&parent)),
            "Archive mixes an instance with outside files"
        );
        for (p, _) in &mut entries {
            *p = p.strip_prefix(&parent)?.into();
        }
    }
    let (manifest_path, data) = entries
        .iter()
        .find(|(p, _)| manifests.iter().any(|m| p == Path::new(m)))
        .unwrap();
    ensure!(
        data.len() <= 2 * 1024 * 1024,
        "Minecraft manifest exceeds 2 MiB"
    );
    let value: Value = serde_json::from_slice(data)?;
    let mut files = BTreeMap::<String, File>::new();
    let mut notes = vec![];
    let (info, kind) = if manifest_path == Path::new("modrinth.index.json") {
        ensure!(
            value["formatVersion"] == 1 && value["game"] == "minecraft",
            "Unsupported Modrinth pack format"
        );
        let deps = value["dependencies"]
            .as_object()
            .context("Missing pack dependencies")?;
        let version = deps
            .get("minecraft")
            .and_then(Value::as_str)
            .context("Missing Minecraft version")?
            .to_owned();
        let loader_keys = [
            ("fabric-loader", "fabric"),
            ("quilt-loader", "quilt"),
            ("forge", "forge"),
            ("neoforge", "neoforge"),
        ];
        let loaders = loader_keys
            .iter()
            .filter(|(key, _)| deps.contains_key(*key))
            .collect::<Vec<_>>();
        ensure!(
            loaders.len() <= 1
                && deps
                    .keys()
                    .all(|key| key == "minecraft" || loader_keys.iter().any(|(k, _)| key == k)),
            "Unsupported or conflicting pack loaders"
        );
        let (loader, loader_version) = if let Some((key, loader)) = loaders.first() {
            (
                (*loader).to_owned(),
                deps[*key]
                    .as_str()
                    .context("Invalid loader version")?
                    .to_owned(),
            )
        } else {
            ("vanilla".into(), String::new())
        };
        let rows = value["files"].as_array().context("Missing pack files")?;
        ensure!(rows.len() <= 1000, "Pack exceeds 1000 downloads");
        for row in rows {
            let path = row["path"]
                .as_str()
                .context("Missing content path")?
                .to_owned();
            ensure!(content_path(&path), "Unsupported content path: {path}");
            let client = row["env"]
                .get("client")
                .and_then(Value::as_str)
                .unwrap_or("required");
            ensure!(
                ["required", "optional", "unsupported"].contains(&client),
                "Invalid client environment"
            );
            if client == "unsupported" {
                notes.push(format!("Server-only file skipped: {path}"));
                continue;
            }
            let sha1 = row["hashes"]["sha1"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let sha512 = row["hashes"]["sha512"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            ensure!(
                hash(&sha1, 40) && hash(&sha512, 128),
                "Missing or invalid content checksums"
            );
            let urls = row["downloads"]
                .as_array()
                .context("Missing download URLs")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .context("Invalid download URL")
                        .map(str::to_owned)
                })
                .collect::<Result<Vec<_>>>()?;
            ensure!(
                !urls.is_empty() && urls.len() <= 5,
                "Invalid download mirror count"
            );
            for url in &urls {
                download_url(url)?;
            }
            let size = row["fileSize"].as_u64().context("Missing file size")?;
            ensure!(
                size > 0 && size <= 64 * 1024 * 1024,
                "Content file exceeds 64 MiB"
            );
            let key = path.to_lowercase();
            ensure!(
                files
                    .insert(
                        key,
                        File {
                            path,
                            content: Content::Download {
                                urls,
                                sha1,
                                sha512,
                                size
                            },
                            optional: client == "optional"
                        }
                    )
                    .is_none(),
                "Duplicate pack content path"
            );
        }
        (
            instance(
                value["name"].as_str().context("Missing pack name")?.into(),
                version,
                loader,
                loader_version,
            )?,
            "Modrinth .mrpack",
        )
    } else if manifest_path == Path::new("mmc-pack.json") {
        ensure!(
            value["formatVersion"] == 1,
            "Unsupported Prism/MultiMC format"
        );
        let components = value["components"]
            .as_array()
            .context("Missing instance components")?;
        ensure!(components.len() <= 10, "Too many instance components");
        let mut version = None;
        let mut loader = "vanilla".to_owned();
        let mut loader_version = String::new();
        let mut seen = BTreeSet::new();
        for c in components {
            let uid = c["uid"].as_str().context("Invalid instance component")?;
            ensure!(seen.insert(uid), "Duplicate instance component");
            let v = c["version"].as_str().context("Missing component version")?;
            match uid {
                "net.minecraft" => version = Some(v.to_owned()),
                "net.fabricmc.fabric-loader"
                | "org.quiltmc.quilt-loader"
                | "net.minecraftforge"
                | "net.neoforged" => {
                    ensure!(loader == "vanilla", "Conflicting instance loaders");
                    loader = match uid {
                        "net.fabricmc.fabric-loader" => "fabric",
                        "org.quiltmc.quilt-loader" => "quilt",
                        "net.minecraftforge" => "forge",
                        _ => "neoforge",
                    }
                    .into();
                    loader_version = v.into();
                }
                "org.lwjgl" | "org.lwjgl3" => {}
                _ => bail!("Unsupported launcher component: {uid}"),
            }
        }
        let cfg = entries
            .iter()
            .find(|(p, _)| p == Path::new("instance.cfg"))
            .context("Missing instance.cfg")?;
        ensure!(cfg.1.len() <= 64 * 1024, "Instance settings exceed limits");
        let text = std::str::from_utf8(&cfg.1)?;
        let names = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .filter(|(key, _)| key.trim() == "name")
            .collect::<Vec<_>>();
        ensure!(names.len() <= 1, "Duplicate instance name");
        let name = names
            .first()
            .map(|(_, v)| v.trim().to_owned())
            .unwrap_or_else(|| "Imported Minecraft instance".into());
        notes.push("Launcher hooks, custom Java arguments, account data, worlds and external paths are not imported.".into());
        (
            instance(
                name,
                version.context("Missing Minecraft component")?,
                loader,
                loader_version,
            )?,
            "Prism / MultiMC bundled instance",
        )
    } else {
        bail!(
            "CurseForge manifest exports require provider API resolution. Export a bundled Prism/MultiMC ZIP or Modrinth pack instead."
        );
    };
    // Client overrides take precedence over shared overrides as specified by mrpack.
    for prefix in if kind.starts_with("Modrinth") {
        vec!["overrides/", "client-overrides/"]
    } else {
        vec![".minecraft/", "minecraft/"]
    } {
        for (path, data) in &entries {
            let text = path.to_string_lossy().replace('\\', "/");
            let Some(relative) = text.strip_prefix(prefix) else {
                continue;
            };
            if !content_path(relative) {
                ensure!(notes.len() < 100, "Too many unsupported files");
                notes.push(format!("Not imported: {relative}"));
                continue;
            }
            ensure!(
                data.len() <= 64 * 1024 * 1024,
                "Bundled content exceeds 64 MiB"
            );
            files.insert(
                relative.to_lowercase(),
                File {
                    path: relative.into(),
                    content: Content::Bundled(data.clone()),
                    optional: false,
                },
            );
        }
    }
    let plan = Plan {
        instance: info,
        kind,
        notes,
        include_optional: false,
        files: files.into_values().collect(),
    };
    validate(&plan)?;
    Ok(plan)
}
fn validate(plan: &Plan) -> Result<()> {
    ensure!(
        identifier(&plan.instance.id) && plan.files.len() <= 1000,
        "Invalid instance import"
    );
    instance(
        plan.instance.name.clone(),
        plan.instance.version.clone(),
        plan.instance.loader.clone(),
        plan.instance.loader_version.clone(),
    )?;
    let mut total = 0u64;
    for file in &plan.files {
        ensure!(content_path(&file.path), "Unsafe content destination");
        total += match &file.content {
            Content::Bundled(data) => data.len() as u64,
            Content::Download { size, .. } => *size,
        };
    }
    ensure!(
        total <= 512 * 1024 * 1024,
        "Import exceeds 512 MiB of content"
    );
    Ok(())
}
fn verify(file: &File, bytes: &[u8]) -> Result<()> {
    if let Content::Download {
        sha1, sha512, size, ..
    } = &file.content
    {
        ensure!(
            bytes.len() as u64 == *size
                && format!("{:x}", Sha1::digest(bytes)) == *sha1
                && format!("{:x}", Sha512::digest(bytes)) == *sha512,
            "Content checksum/size mismatch: {}",
            file.path
        );
    }
    if file.path.to_lowercase().ends_with(".jar")
        || file.path.to_lowercase().ends_with(".jar.disabled")
        || file.path.to_lowercase().ends_with(".zip")
    {
        ensure!(
            bytes.starts_with(b"PK"),
            "Pack content is not a JAR/ZIP: {}",
            file.path
        );
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        ensure!(
            archive.len() <= 100_000,
            "Content archive exceeds entry limits"
        );
        let mut total = 0u64;
        for i in 0..archive.len() {
            total = total
                .checked_add(archive.by_index(i)?.size())
                .context("Content archive size overflow")?;
        }
        ensure!(
            total <= 512 * 1024 * 1024,
            "Expanded content archive exceeds limits"
        );
    } else {
        ensure!(
            bytes.len() <= 256 * 1024 && !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok(),
            "Settings must be bounded UTF-8 text"
        );
    }
    Ok(())
}
pub(super) fn commit(plan: Plan, progress: Sender<Outcome>) -> Result<String> {
    commit_at(&plan, &root().join("instances"), |file| {
        let _ = progress.send(Outcome::Status(format!("Importing {}…", file.path)));
        match &file.content {
            Content::Bundled(bytes) => Ok(bytes.clone()),
            Content::Download { urls, size, .. } => {
                let client = client()?;
                let mut last = None;
                for url in urls {
                    let result = (|| -> Result<Vec<u8>> {
                        let response = client.get(download_url(url)?).send()?.error_for_status()?;
                        ensure!(response.status().is_success(), "Pack download redirected");
                        let mut bytes = vec![];
                        response.take(size + 1).read_to_end(&mut bytes)?;
                        verify(file, &bytes)?;
                        Ok(bytes)
                    })();
                    match result {
                        Ok(bytes) => return Ok(bytes),
                        Err(e) => last = Some(e),
                    }
                }
                Err(last.context("No pack download mirror")?)
            }
        }
    })?;
    Ok(format!(
        "Imported {} · {} content files. Use Install / update runtime before launching.",
        plan.instance.name,
        plan.count()
    ))
}
fn commit_at(
    plan: &Plan,
    instances: &Path,
    fetch: impl Fn(&File) -> Result<Vec<u8>>,
) -> Result<()> {
    validate(plan)?;
    crate::runtime::no_links(instances)?;
    std::fs::create_dir_all(instances)?;
    let target = instances.join(&plan.instance.id);
    crate::runtime::no_links(&target)?;
    ensure!(
        !target.exists(),
        "An instance with this identity already exists"
    );
    let stage = instances.join(format!(".canna-stage-{}", plan.instance.id));
    crate::runtime::no_links(&stage)?;
    std::fs::create_dir(&stage)?;
    let result = (|| -> Result<()> {
        for file in plan
            .files
            .iter()
            .filter(|f| !f.optional || plan.include_optional)
        {
            let bytes = fetch(file)?;
            verify(file, &bytes)?;
            let destination = relative(&stage, &file.path)?;
            crate::runtime::no_links(&destination)?;
            std::fs::create_dir_all(destination.parent().unwrap())?;
            use std::io::Write;
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(destination)?
                .write_all(&bytes)?;
        }
        std::fs::write(
            stage.join("instance.json"),
            serde_json::to_vec_pretty(&plan.instance)?,
        )?;
        ensure!(!target.exists(), "Instance appeared during import");
        std::fs::rename(&stage, &target)?;
        Ok(())
    })();
    if result.is_err() {
        crate::runtime::no_links(&stage)?;
        ensure!(
            stage.parent() == Some(instances)
                && stage
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".canna-stage-import-"),
            "Invalid import cleanup target"
        );
        std::fs::remove_dir_all(&stage)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn jar() -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        zip.start_file("mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap().into_inner()
    }
    fn row(path: &str, bytes: &[u8], env: &str) -> Value {
        json!({"path":path,"hashes":{"sha1":format!("{:x}",Sha1::digest(bytes)),"sha512":format!("{:x}",Sha512::digest(bytes))},"downloads":["https://cdn.modrinth.com/data/project/versions/release/test.jar"],"fileSize":bytes.len(),"env":{"client":env}})
    }
    fn pack(rows: Vec<Value>) -> Vec<(PathBuf, Vec<u8>)> {
        vec![("modrinth.index.json".into(),serde_json::to_vec(&json!({"formatVersion":1,"game":"minecraft","name":"Family","dependencies":{"minecraft":"1.21.1","fabric-loader":"0.16.5"},"files":rows})).unwrap())]
    }
    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "canna-mc-import-tests-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn mrpack_preserves_loader_hashes_and_client_override_precedence() {
        let bytes = jar();
        let mut entries = pack(vec![
            row("mods/required.jar", &bytes, "required"),
            row("mods/optional.jar", &bytes, "optional"),
            row("mods/server.jar", &bytes, "unsupported"),
        ]);
        entries.push(("overrides/config/a.cfg".into(), b"shared".to_vec()));
        entries.push(("client-overrides/config/a.cfg".into(), b"client".to_vec()));
        let mut plan = inspect_entries(entries).unwrap();
        assert_eq!(plan.instance.loader, "fabric");
        assert_eq!(plan.instance.loader_version, "0.16.5");
        assert_eq!(plan.count(), 2);
        assert_eq!(plan.optional_count(), 1);
        let root = temp();
        commit_at(&plan, &root, |file| match &file.content {
            Content::Bundled(b) => Ok(b.clone()),
            _ => Ok(bytes.clone()),
        })
        .unwrap();
        assert_eq!(
            std::fs::read(root.join(&plan.instance.id).join("config/a.cfg")).unwrap(),
            b"client"
        );
        assert!(
            !root
                .join(&plan.instance.id)
                .join("mods/optional.jar")
                .exists()
        );
        assert!(commit_at(&plan, &root, |_| Ok(bytes.clone())).is_err());
        plan.include_optional = true;
        assert_eq!(plan.count(), 3);
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_hash_leaves_no_partial_instance() {
        let bytes = jar();
        let plan = inspect_entries(pack(vec![row("mods/test.jar", &bytes, "required")])).unwrap();
        let root = temp();
        assert!(commit_at(&plan, &root, |_| Ok(b"wrong content".to_vec())).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn prism_imports_disabled_jars_but_ignores_launcher_commands() {
        let entries=vec![("Export/mmc-pack.json".into(),br#"{"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.20.1"},{"uid":"net.minecraftforge","version":"1.20.1-47.2.0"},{"uid":"org.lwjgl3","version":"3.3.1"}]}"#.to_vec()),("Export/instance.cfg".into(),b"name=Friends\nPreLaunchCommand=powershell bad\nJvmArgs=-javaagent:bad.jar\nJavaPath=C:/bad/java.exe".to_vec()),("Export/.minecraft/mods/Test.jar.disabled".into(),jar()),("Export/.minecraft/config/setting.cfg".into(),b"value=true".to_vec()),("Export/.minecraft/saves/World/level.dat".into(),vec![0])];
        let plan = inspect_entries(entries).unwrap();
        assert_eq!(plan.instance.loader_version, "47.2.0");
        assert!(plan.instance.java.is_empty());
        assert_eq!(plan.count(), 2);
        assert!(plan.files.iter().any(|f| f.path.ends_with(".jar.disabled")));
        let root = temp();
        commit_at(&plan, &root, |file| match &file.content {
            Content::Bundled(b) => Ok(b.clone()),
            _ => unreachable!(),
        })
        .unwrap();
        assert!(!root.join(&plan.instance.id).join("instance.cfg").exists());
        assert!(!root.join(&plan.instance.id).join("saves").exists());
        assert!(root.starts_with(std::env::temp_dir()));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unsafe_destinations_providers_hashes_and_ambiguous_manifests_are_refused() {
        let bytes = jar();
        for path in [
            "../outside.jar",
            "C:/outside.jar",
            "mods/CON.jar",
            "mods/test.exe",
            "mods/test.jar:extra",
            "config/evil.dll",
            "mods/a/../../outside.jar",
        ] {
            assert!(
                inspect_entries(pack(vec![row(path, &bytes, "required")])).is_err(),
                "{path}"
            );
        }
        let mut invalid = row("mods/test.jar", &bytes, "required");
        invalid["downloads"] = json!(["https://localhost/private"]);
        assert!(inspect_entries(pack(vec![invalid])).is_err());
        let mut invalid = row("mods/test.jar", &bytes, "required");
        invalid["hashes"]["sha512"] = json!("bad");
        assert!(inspect_entries(pack(vec![invalid])).is_err());
        assert!(
            inspect_entries(pack(vec![
                row("mods/test.jar", &bytes, "required"),
                row("mods/Test.jar", &bytes, "required")
            ]))
            .is_err()
        );
        let mut invalid = pack(vec![]);
        invalid.push(("mmc-pack.json".into(), b"{}".to_vec()));
        assert!(inspect_entries(invalid).is_err());
        for url in [
            "http://cdn.modrinth.com/data/a",
            "https://cdn.modrinth.com.evil/data/a",
            "https://user@cdn.modrinth.com/data/a",
            "https://cdn.modrinth.com:8443/data/a",
        ] {
            assert!(download_url(url).is_err());
        }
    }
}
