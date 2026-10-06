use crate::{
    cache::Source,
    model::{GameInfo, ModInfo},
    repository::{valid_mod_file, valid_path, valid_slug},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PACKS: usize = 200;

#[derive(Clone, Serialize, Deserialize)]
pub struct PackGame {
    pub app_id: u32,
    pub name: String,
    pub folder: String,
    pub framework: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Modpack {
    pub format: String,
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub theme: u8,
    pub game: PackGame,
    pub repository: Source,
    pub mods: Vec<ModInfo>,
}
fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("pack-{nanos:x}-{:x}", std::process::id())
}
pub fn directory() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CannaModManager")
        .join("modpacks")
}
pub fn local_directory() -> PathBuf {
    directory().join("local-mods")
}
pub fn add_local(path: &Path) -> Result<ModInfo> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext != "dll" && ext != "zip" && ext != "vpk" {
        bail!("Choose a plugin DLL, VPK addon or ZIP")
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 32 * 1024 * 1024 {
        bail!("Local mods are limited to 32 MiB each")
    }
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let file = format!("{hash}.{ext}");
    std::fs::create_dir_all(local_directory())?;
    std::fs::write(local_directory().join(&file), bytes)?;
    Ok(ModInfo {
        provenance: serde_json::Value::Null,
        content_type: String::new(),
        enabled: true,
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        version: "local".into(),
        description: "Imported from this computer".into(),
        file: format!("Mods/{file}"),
        sha256: hash,
        local_file: file,
        dependencies: Vec::new(),
    })
}
impl Modpack {
    pub fn set_mod_enabled(&mut self, file: &str, enabled: bool) -> Result<()> {
        let index = self
            .mods
            .iter()
            .position(|m| m.file == file)
            .context("Mod no longer exists")?;
        let mut pending = vec![index];
        let mut visited = BTreeSet::new();
        while let Some(index) = pending.pop() {
            if !visited.insert(index) {
                continue;
            }
            if enabled {
                for name in &self.mods[index].dependencies {
                    pending.push(
                        self.mods
                            .iter()
                            .position(|m| &m.name == name)
                            .with_context(|| {
                                format!("Required mod {name} is missing; add it through Discover")
                            })?,
                    );
                }
            }
        }
        let mut changed = self.clone();
        for index in visited {
            changed.mods[index].enabled = enabled;
        }
        changed.validate()?;
        *self = changed;
        Ok(())
    }
    pub fn create(
        name: String,
        description: String,
        game: &GameInfo,
        repository: Source,
        mods: Vec<ModInfo>,
    ) -> Self {
        Self {
            format: "canna_modpack".into(),
            schema_version: 1,
            id: new_id(),
            name: name.trim().into(),
            description,
            group: String::new(),
            theme: 0,
            game: PackGame {
                app_id: game.app_id,
                name: game.name.clone(),
                folder: game.folder.clone(),
                framework: crate::model::framework(game.app_id).into(),
            },
            repository,
            mods,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.format != "canna_modpack" || self.schema_version != 1 {
            bail!("Unsupported Canna modpack format or version")
        }
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            bail!("Invalid modpack ID")
        }
        if self.name.trim().is_empty() || self.name.len() > 100 || self.description.len() > 2000 {
            bail!("Use a pack name of 1–100 bytes and description of at most 2000 bytes")
        }
        if self.group.len() > 80 || self.theme > 3 {
            bail!("Invalid pack group or cover color")
        }
        if self.game.app_id == 0
            || self.game.name.trim().is_empty()
            || self.game.name.len() > 200
            || !valid_path(&self.game.folder)
            || self.game.framework != crate::model::framework(self.game.app_id)
        {
            bail!("Invalid game or unsupported modding framework")
        }
        if self.mods.len() > 1000 {
            bail!("A modpack can contain at most 1000 mods")
        }
        let source = &self.repository;
        if !source.catalog_folder.is_empty() && !valid_path(&source.catalog_folder) {
            bail!("Invalid repository catalog folder")
        }
        if (!self.mods.is_empty() || !source.owner.is_empty())
            && (!valid_slug(&source.owner)
                || !valid_slug(&source.repository)
                || source.branch.trim().is_empty()
                || source.branch.len() > 200)
        {
            bail!("Modpacks with mods require a GitHub owner, repository and branch")
        }
        let mut files = BTreeSet::new();
        for item in &self.mods {
            anyhow::ensure!(
                item.provenance["external_only"] != true,
                "Original-site downloads cannot be installed as Canna modpack files"
            );
            let extension = item.file.to_ascii_lowercase();
            if (self.game.framework == "source-vpk" && extension.ends_with(".dll"))
                || (self.game.framework == "bepinex" && extension.ends_with(".vpk"))
            {
                bail!(
                    "{} is not compatible with this game's mod framework",
                    item.name
                );
            }
            if item.dependencies.len() > 32
                || item
                    .dependencies
                    .iter()
                    .any(|name| name.is_empty() || name.len() > 200)
            {
                bail!("Invalid dependency list for {}", item.name)
            }
            if item.enabled {
                for dependency in &item.dependencies {
                    if !self.mods.iter().any(|m| m.name == *dependency && m.enabled) {
                        bail!(
                            "{} requires {} enabled in this modpack",
                            item.name,
                            dependency
                        )
                    }
                }
            }
            if !item.local_file.is_empty()
                && (item.sha256.len() != 64
                    || ![
                        format!("{}.dll", item.sha256),
                        format!("{}.zip", item.sha256),
                        format!("{}.vpk", item.sha256),
                    ]
                    .contains(&item.local_file))
            {
                bail!("Invalid local mod reference")
            }
            if item.name.trim().is_empty()
                || item.name.len() > 200
                || item.version.trim().is_empty()
                || item.version.len() > 100
                || !valid_mod_file(&item.file)
                || !files.insert(&item.file)
            {
                bail!("Mod names, versions and unique relative Mods/ file paths are required")
            }
            if !item.sha256.is_empty()
                && (item.sha256.len() != 64 || !item.sha256.chars().all(|c| c.is_ascii_hexdigit()))
            {
                bail!("A supplied SHA-256 must contain 64 hexadecimal characters")
            }
        }
        Ok(())
    }
    pub fn save(&self) -> Result<()> {
        self.save_in(&directory())
    }
    pub fn delete(&self) -> Result<PathBuf> {
        self.delete_in(&directory())
    }
    fn delete_in(&self, folder: &Path) -> Result<PathBuf> {
        self.validate()?;
        let source = folder.join(format!("{}.canna.json", self.id));
        let metadata = std::fs::symlink_metadata(&source).context("Saved modpack was not found")?;
        if !metadata.file_type().is_file() {
            bail!("The saved modpack is not a regular file")
        }
        let trash = folder.join("deleted");
        std::fs::create_dir_all(&trash)?;
        let destination = trash.join(format!("{}.canna.json", new_id()));
        std::fs::rename(source, &destination).context("Could not delete modpack")?;
        Ok(destination)
    }
    pub fn restore_deleted(path: &Path) -> Result<Self> {
        Self::restore_into(path, &directory())
    }
    fn restore_into(path: &Path, folder: &Path) -> Result<Self> {
        let pack = read(path)?;
        let destination = folder.join(format!("{}.canna.json", pack.id));
        if destination.exists() {
            bail!("A modpack with this ID already exists")
        }
        std::fs::rename(path, destination).context("Could not restore modpack")?;
        Ok(pack)
    }
    fn save_in(&self, folder: &Path) -> Result<()> {
        self.validate()?;
        std::fs::create_dir_all(folder)?;
        let destination = folder.join(format!("{}.canna.json", self.id));
        if !destination.exists()
            && std::fs::read_dir(folder)?
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|x| x == "json"))
                .count()
                >= MAX_PACKS
        {
            bail!("The local library is limited to {MAX_PACKS} modpacks")
        }
        self.write(&destination)
    }
    pub fn export(&self, destination: &Path) -> Result<()> {
        self.validate()?;
        if destination
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
        {
            let temporary = destination.with_extension(format!("{}.tmp", new_id()));
            let result = (|| -> Result<()> {
                let file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                let mut archive = zip::ZipWriter::new(file);
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                archive.start_file("manifest.json", options)?;
                archive.write_all(&serde_json::to_vec_pretty(self)?)?;
                let mut included = BTreeSet::new();
                for item in self.mods.iter().filter(|m| !m.local_file.is_empty()) {
                    if !included.insert(&item.local_file) {
                        continue;
                    }
                    let data = std::fs::read(local_directory().join(&item.local_file))
                        .context("Local mod file is missing")?;
                    archive.start_file(format!("content/{}", item.local_file), options)?;
                    archive.write_all(&data)?;
                }
                archive.finish()?.sync_all()?;
                std::fs::rename(&temporary, destination)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(temporary);
            }
            return result;
        }
        if self.mods.iter().any(|m| !m.local_file.is_empty()) {
            bail!("Export as .canna.zip to include local mod files")
        }
        self.write(destination)
    }
    fn write(&self, destination: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("Modpack exceeds 2 MiB")
        }
        // The save dialog handles choosing and approving the destination; local saves use safe IDs.
        let temporary = destination.with_extension(format!("{}.tmp", new_id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temporary, destination).context("Could not save modpack")?;
        Ok(())
    }
    pub fn import(path: &Path) -> Result<Self> {
        Self::import_into(path, &directory())
    }
    fn import_into(path: &Path, folder: &Path) -> Result<Self> {
        let mut pack = if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
        {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(128 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 128 * 1024 * 1024 {
                bail!("Modpack ZIP exceeds 128 MiB")
            }
            let entries = crate::runtime::archive_files(&bytes)?;
            let data = &entries
                .iter()
                .find(|(p, _)| p == Path::new("manifest.json"))
                .context("No manifest.json in modpack")?
                .1;
            if data.len() as u64 > MAX_BYTES {
                bail!("Manifest exceeds 2 MiB")
            }
            let pack: Self = serde_json::from_slice(data)?;
            pack.validate()?;
            let mut local = Vec::new();
            for item in pack.mods.iter().filter(|m| !m.local_file.is_empty()) {
                let data = &entries
                    .iter()
                    .find(|(p, _)| p == &PathBuf::from(format!("content/{}", item.local_file)))
                    .context("Local mod missing from archive")?
                    .1;
                if data.len() > 32 * 1024 * 1024
                    || format!("{:x}", Sha256::digest(data)) != item.sha256.to_lowercase()
                {
                    bail!("Local mod checksum or size is invalid")
                }
                local.push((item.local_file.clone(), data));
            }
            std::fs::create_dir_all(local_directory())?;
            for (name, data) in local {
                std::fs::write(local_directory().join(name), data)?;
            }
            pack
        } else {
            read(path)?
        };
        // Import as a new local pack: incoming IDs never overwrite an existing family pack.
        pack.id = new_id();
        pack.save_in(folder)?;
        Ok(pack)
    }
}
fn read(path: &Path) -> Result<Modpack> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("Modpack exceeds 2 MiB")
    }
    let pack: Modpack = serde_json::from_slice(&bytes).context("Invalid Canna modpack JSON")?;
    pack.validate()?;
    Ok(pack)
}
pub fn load_all() -> (Vec<Modpack>, Vec<String>) {
    let (mut packs, warnings) = load_from(&directory());
    for p in &mut packs {
        p.repository.owner = "canna".into();
        p.repository.repository = "server".into();
        p.repository.branch = "main".into();
        p.repository.catalog_folder.clear();
    }
    (packs, warnings)
}
pub fn load_groups() -> Vec<String> {
    let path = directory().parent().unwrap().join("modpack-groups.json");
    let Ok(file) = std::fs::File::open(path) else {
        return vec![];
    };
    let mut bytes = Vec::new();
    if file.take(16385).read_to_end(&mut bytes).is_err() || bytes.len() > 16384 {
        return vec![];
    }
    let Ok(groups) = serde_json::from_slice::<Vec<String>>(&bytes) else {
        return vec![];
    };
    groups
        .into_iter()
        .filter(|g| !g.trim().is_empty() && g.len() <= 80)
        .take(50)
        .collect()
}
pub fn save_groups(groups: &[String]) -> Result<()> {
    if groups.len() > 50 || groups.iter().any(|g| g.trim().is_empty() || g.len() > 80) {
        bail!("Use at most 50 groups with names of 1–80 bytes")
    }
    let path = directory().parent().unwrap().join("modpack-groups.json");
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension(format!("{}.tmp", new_id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(groups)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
fn load_from(folder: &Path) -> (Vec<Modpack>, Vec<String>) {
    let mut packs = Vec::new();
    let mut warnings = Vec::new();
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (packs, warnings),
        Err(error) => return (packs, vec![format!("Cannot read modpacks: {error}")]),
    };
    let mut ids = BTreeSet::new();
    for entry in entries.flatten() {
        if !entry
            .file_type()
            .is_ok_and(|t| t.is_file() && !t.is_symlink())
            || !entry.path().extension().is_some_and(|x| x == "json")
        {
            continue;
        }
        if packs.len() >= MAX_PACKS {
            warnings.push(format!("Only the first {MAX_PACKS} valid packs are loaded"));
            break;
        }
        match read(&entry.path()) {
            Ok(pack) if ids.insert(pack.id.clone()) => packs.push(pack),
            Ok(_) => warnings.push(format!(
                "Duplicate pack ID in {}",
                entry.file_name().to_string_lossy()
            )),
            Err(error) => {
                warnings.push(format!("{}: {error}", entry.file_name().to_string_lossy()))
            }
        }
    }
    packs.sort_by_key(|pack| pack.name.to_lowercase());
    (packs, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleting_pack_removes_only_its_manifest_and_can_be_undone() {
        let folder = std::env::temp_dir().join(format!("canna-delete-{}", new_id()));
        let pack = fixture();
        pack.save_in(&folder).unwrap();
        let unrelated = folder.join("unrelated.json");
        std::fs::write(&unrelated, b"leave this alone").unwrap();
        let original = folder.join(format!("{}.canna.json", pack.id));
        let bytes = std::fs::read(&original).unwrap();
        let trash = pack.delete_in(&folder).unwrap();
        assert!(!original.exists());
        assert!(trash.exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"leave this alone");
        assert!(load_from(&folder).0.is_empty());
        let restored = Modpack::restore_into(&trash, &folder).unwrap();
        assert_eq!(restored.id, pack.id);
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert!(!trash.exists());
        let mut unsafe_pack = pack;
        unsafe_pack.id = "../outside".into();
        assert!(unsafe_pack.delete_in(&folder).is_err());
        assert!(original.exists());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn bundle_preserves_local_mod_and_rejects_tampering() {
        let folder = std::env::temp_dir().join(format!("canna-local-bundle-{}", new_id()));
        std::fs::create_dir_all(&folder).unwrap();
        let input = folder.join("test-plugin.dll");
        std::fs::write(&input, b"local mod bundle fixture").unwrap();
        let expected_hash = format!("{:x}", Sha256::digest(b"local mod bundle fixture"));
        let cache = local_directory().join(format!("{expected_hash}.dll"));
        let existed = cache.exists();
        let item = add_local(&input).unwrap();
        let mut pack = fixture();
        pack.mods = vec![item];
        let bundle = folder.join("family.canna.zip");
        pack.export(&bundle).unwrap();
        assert!(pack.export(&folder.join("bad.canna.json")).is_err());
        let imported = Modpack::import_into(&bundle, &folder.join("imported")).unwrap();
        assert_ne!(pack.id, imported.id);
        assert_eq!(imported.mods[0].local_file, pack.mods[0].local_file);
        assert_eq!(std::fs::read(&cache).unwrap(), b"local mod bundle fixture");
        let mut archive =
            zip::ZipWriter::new(std::fs::File::create(folder.join("tampered.zip")).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("manifest.json", options).unwrap();
        archive
            .write_all(&serde_json::to_vec(&pack).unwrap())
            .unwrap();
        archive
            .start_file(format!("content/{}", pack.mods[0].local_file), options)
            .unwrap();
        archive.write_all(b"tampered").unwrap();
        archive.finish().unwrap();
        assert!(
            Modpack::import_into(&folder.join("tampered.zip"), &folder.join("imported")).is_err()
        );
        if !existed {
            std::fs::remove_file(cache).unwrap();
        }
        std::fs::remove_dir_all(folder).unwrap();
    }
    fn fixture() -> Modpack {
        let source = Source {
            owner: "family".into(),
            repository: "manager-uploaded-mods".into(),
            branch: "main".into(),
            catalog_folder: "games".into(),
        };
        Modpack::create(
            "Family Bopl Night".into(),
            "Round-trip test".into(),
            &crate::model::bopl(),
            source,
            vec![ModInfo {
                provenance: serde_json::Value::Null,
                content_type: String::new(),
                enabled: true,
                name: "Fixture mod".into(),
                version: "1.2.3".into(),
                description: String::new(),
                file: "mods/fixture.zip".into(),
                sha256: "a".repeat(64),
                local_file: String::new(),
                dependencies: Vec::new(),
            }],
        )
    }
    #[test]
    fn dependencies_enable_together_and_cannot_be_removed_while_used() {
        let mut pack = fixture();
        let mut library = pack.mods[0].clone();
        library.name = "Required library".into();
        library.file = "Mods/library.zip".into();
        library.enabled = false;
        pack.mods[0].dependencies = vec![library.name.clone()];
        pack.mods[0].enabled = false;
        pack.mods.push(library);
        assert!(pack.validate().is_ok());
        pack.set_mod_enabled("mods/fixture.zip", true).unwrap();
        assert!(pack.mods.iter().all(|m| m.enabled));
        assert!(pack.set_mod_enabled("Mods/library.zip", false).is_err());
        assert!(pack.mods[1].enabled);
        pack.set_mod_enabled("mods/fixture.zip", false).unwrap();
        pack.set_mod_enabled("Mods/library.zip", false).unwrap();
        pack.mods.pop();
        assert!(pack.set_mod_enabled("mods/fixture.zip", true).is_err());
        assert!(!pack.mods[0].enabled);
    }
    #[test]
    fn export_import_preserves_pins_and_does_not_overwrite() {
        let folder = std::env::temp_dir().join(new_id());
        std::fs::create_dir_all(&folder).unwrap();
        let local = folder.join("profiles");
        let mut pack = fixture();
        pack.group = "Family nights".into();
        pack.theme = 2;
        pack.mods[0].enabled = false;
        pack.save_in(&local).unwrap();
        let exported = folder.join("family.canna.json");
        pack.export(&exported).unwrap();
        // Verify replacing an existing export works on Windows too.
        pack.export(&exported).unwrap();
        let imported = Modpack::import_into(&exported, &local).unwrap();
        assert_ne!(imported.id, pack.id);
        assert_eq!(imported.mods[0].version, "1.2.3");
        assert!(!imported.mods[0].enabled);
        assert_eq!(imported.mods[0].sha256, "a".repeat(64));
        assert_eq!(imported.repository.catalog_folder, "games");
        assert_eq!(imported.group, "Family nights");
        assert_eq!(imported.theme, 2);
        let (saved, warnings) = load_from(&local);
        assert_eq!(saved.len(), 2);
        assert!(warnings.is_empty());
        let export_text = std::fs::read_to_string(exported).unwrap();
        assert!(!export_text.contains("token"));
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn rejects_unsafe_and_inconsistent_packs() {
        let pack = fixture();
        let mut invalid = pack.clone();
        invalid.id = "../overwrite".into();
        assert!(invalid.validate().is_err());
        let mut invalid = pack.clone();
        invalid.mods[0].file = "mods/../../escape.dll".into();
        assert!(invalid.validate().is_err());
        let mut invalid = pack.clone();
        invalid.mods.push(invalid.mods[0].clone());
        assert!(invalid.validate().is_err());
        let mut invalid = pack.clone();
        invalid.game.framework = "unreal".into();
        assert!(invalid.validate().is_err());
        let mut invalid = pack.clone();
        invalid.repository.owner.clear();
        assert!(invalid.validate().is_err());
        let mut invalid = pack;
        invalid.mods[0].sha256 = "wrong".into();
        assert!(invalid.validate().is_err());
    }
}
