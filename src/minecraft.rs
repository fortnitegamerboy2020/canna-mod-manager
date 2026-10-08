use anyhow::{Context, Result, bail};
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};
const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
#[path = "minecraft_play.rs"]
mod play;
pub use play::{create_play_candidate, play_setup, play_version, restore_play_pack};
#[derive(Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub version: String,
    pub loader: String,
    pub loader_version: String,
    pub java: String,
    pub memory: u32,
}
fn root() -> PathBuf {
    crate::modpacks::directory()
        .parent()
        .unwrap()
        .join("minecraft")
}
fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b))
}
fn dir(instance: &Instance) -> PathBuf {
    root().join("instances").join(&instance.id)
}
fn instances() -> Vec<Instance> {
    std::fs::read_dir(root().join("instances"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read(e.path().join("instance.json")).ok())
        .filter_map(|b| serde_json::from_slice(&b).ok())
        .collect()
}
fn save(i: &Instance) -> Result<()> {
    anyhow::ensure!(crate::repository::valid_slug(&i.id), "Invalid instance ID");
    std::fs::create_dir_all(dir(i))?;
    std::fs::write(dir(i).join("instance.json"), serde_json::to_vec_pretty(i)?)?;
    Ok(())
}
fn content_compatible(instance: &Instance, item: &crate::model::ModInfo) -> bool {
    let p = &item.provenance;
    let supports = |key: &str, value: &str| {
        p[key]
            .as_array()
            .is_none_or(|a| a.is_empty() || a.iter().any(|v| v.as_str() == Some(value)))
    };
    supports("game_versions", &instance.version)
        && (item.content_type != "mod" || supports("loaders", &instance.loader))
}
pub fn content_target_ui(
    ui: &mut egui::Ui,
    item: &crate::model::ModInfo,
    selected: &mut String,
    world: &mut String,
    ready: bool,
) -> Option<(Instance, String)> {
    let all = instances();
    egui::ComboBox::from_id_salt("provider-minecraft-instance")
        .height(340.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .selected_text(
            all.iter()
                .find(|i| i.id == *selected)
                .map(|i| i.name.as_str())
                .unwrap_or("Choose a Minecraft instance"),
        )
        .show_ui(ui, |ui| {
            let choices = all
                .iter()
                .filter(|i| content_compatible(i, item))
                .map(|i| {
                    (
                        i.id.clone(),
                        format!("{} · {} · {}", i.name, i.version, i.loader),
                    )
                })
                .collect::<Vec<_>>();
            crate::ui_helpers::searchable_options(ui, selected, &choices);
        });
    let instance = all
        .iter()
        .find(|i| i.id == *selected && content_compatible(i, item))?;
    if item.content_type == "datapack" {
        let worlds = std::fs::read_dir(dir(instance).join("saves"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect::<Vec<_>>();
        egui::ComboBox::from_id_salt("provider-minecraft-world")
            .height(340.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .selected_text(if world.is_empty() {
                "Choose a world"
            } else {
                world.as_str()
            })
            .show_ui(ui, |ui| {
                let choices = worlds
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect::<Vec<_>>();
                crate::ui_helpers::searchable_options(ui, world, &choices);
            });
        if !worlds.contains(world) {
            return None;
        }
    }
    if ui
        .add_enabled(
            ready,
            egui::Button::new(format!("Install {} into instance", item.name)),
        )
        .clicked()
    {
        Some((instance.clone(), world.clone()))
    } else {
        None
    }
}
fn content_plan(
    instance: &Instance,
    world: &str,
    game: &crate::model::GameInfo,
    item: &crate::model::ModInfo,
    mut fetch: impl FnMut(&crate::model::ModInfo) -> Result<Vec<u8>>,
) -> Result<Vec<(PathBuf, String, Vec<u8>)>> {
    use sha2::Digest;
    anyhow::ensure!(
        crate::repository::valid_slug(&instance.id),
        "Invalid instance ID"
    );
    anyhow::ensure!(game.app_id == u32::MAX, "This content is not for Minecraft");
    let mut queue = vec![item.clone()];
    let mut seen = std::collections::BTreeSet::new();
    let mut files = Vec::new();
    let mut total = 0;
    while let Some(m) = queue.pop() {
        if !seen.insert(m.file.clone()) {
            continue;
        }
        anyhow::ensure!(seen.len() <= 128, "Too many dependencies");
        anyhow::ensure!(
            content_compatible(instance, &m),
            "{} does not support this instance's game version or loader",
            m.name
        );
        for name in &m.dependencies {
            queue.push(
                game.mods
                    .iter()
                    .find(|d| d.name == *name && content_compatible(instance, d))
                    .cloned()
                    .with_context(|| {
                        format!("Required compatible dependency {name} is unavailable")
                    })?,
            );
        }
        let folder = match m.content_type.as_str() {
            "mod" => dir(instance).join("mods"),
            "shader" => dir(instance).join("shaderpacks"),
            "resourcepack" => dir(instance).join("resourcepacks"),
            "datapack" => {
                anyhow::ensure!(
                    !world.is_empty()
                        && world != "."
                        && world != ".."
                        && !world.contains(['/', '\\', ':'])
                        && dir(instance).join("saves").join(world).is_dir(),
                    "Choose an existing instance world"
                );
                dir(instance).join("saves").join(world).join("datapacks")
            }
            _ => bail!("This content type does not have a Minecraft installer"),
        };
        let bytes = fetch(&m)?;
        total += bytes.len();
        anyhow::ensure!(
            total <= 256 * 1024 * 1024,
            "Content dependency graph exceeds 256 MiB"
        );
        anyhow::ensure!(
            format!("{:x}", sha2::Sha256::digest(&bytes)).eq_ignore_ascii_case(&m.sha256),
            "Checksum mismatch for {}",
            m.name
        );
        let filename = m.provenance["filename"]
            .as_str()
            .context("Provider filename is missing")?;
        anyhow::ensure!(
            safe_component(filename) && (filename.ends_with(".jar") || filename.ends_with(".zip")),
            "Unsupported content filename"
        );
        files.push((folder, filename.to_owned(), bytes));
    }
    Ok(files)
}
pub fn install_catalog_content(
    instance: &Instance,
    world: &str,
    game: &crate::model::GameInfo,
    item: &crate::model::ModInfo,
    token: &str,
) -> Result<String> {
    let c = client()?;
    let files = content_plan(instance, world, game, item, |m| {
        crate::repository::fetch_optional(
            &c,
            &crate::model::Settings::default(),
            token,
            &m.file,
            128 * 1024 * 1024,
        )?
        .context("Mod was removed")
    })?;
    let (previous, installed) = play_setup(instance)?;
    crate::runtime::ensure_closed(&installed)?;
    let policy = crate::play_backup::Policy::load();
    if policy.automatic {
        crate::play_backup::capture(&policy, &installed, &previous, false)?;
    }
    let count = files.len();
    for (folder, name, bytes) in files {
        std::fs::create_dir_all(&folder)?;
        let pending = folder.join(format!("{name}.canna-pending"));
        std::fs::write(&pending, bytes)?;
        std::fs::rename(&pending, folder.join(name))?;
    }
    let (applied, installed) = play_setup(instance)?;
    crate::play_backup::remember_applied(&installed, &applied)?;
    Ok(format!(
        "Installed {count} content files into {}. Restart the instance to load them.",
        instance.name
    ))
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Canna-Minecraft/0.2.5")
        .build()?)
}
fn allowed(url: &str) -> Result<reqwest::Url> {
    let u = reqwest::Url::parse(url)?;
    anyhow::ensure!(
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port().is_none(),
        "Invalid Minecraft download URL"
    );
    anyhow::ensure!(
        matches!(
            u.host_str(),
            Some(
                "piston-meta.mojang.com"
                    | "piston-data.mojang.com"
                    | "launchermeta.mojang.com"
                    | "launcher.mojang.com"
                    | "libraries.minecraft.net"
                    | "resources.download.minecraft.net"
                    | "meta.fabricmc.net"
                    | "maven.fabricmc.net"
                    | "meta.quiltmc.org"
                    | "maven.quiltmc.org"
                    | "maven.minecraftforge.net"
                    | "files.minecraftforge.net"
                    | "maven.neoforged.net"
                    | "api.adoptium.net"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
            )
        ),
        "Unsupported Minecraft download host"
    );
    Ok(u)
}
fn get(url: &str, limit: u64) -> Result<Vec<u8>> {
    let client = client()?;
    let mut target = allowed(url)?;
    for _ in 0..5 {
        let r = client.get(target.clone()).send()?;
        if r.status().is_redirection() {
            target = allowed(
                target
                    .join(
                        r.headers()
                            .get("location")
                            .context("Missing download redirect")?
                            .to_str()?,
                    )?
                    .as_str(),
            )?;
            continue;
        }
        let mut bytes = Vec::new();
        r.error_for_status()?
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes.len() as u64 <= limit,
            "Minecraft download exceeded its limit"
        );
        return Ok(bytes);
    }
    bail!("Too many download redirects")
}
fn metadata(url: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&get(url, 16 * 1024 * 1024)?)?)
}
fn relative(root: &Path, path: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !path.is_empty() && path.split('/').all(safe_component),
        "Unsafe Minecraft file path"
    );
    let p = root.join(path);
    for ancestor in p.ancestors() {
        if let Ok(m) = std::fs::symlink_metadata(ancestor) {
            anyhow::ensure!(
                !m.file_type().is_symlink(),
                "Linked Minecraft folders are unsupported"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                anyhow::ensure!(
                    m.file_attributes() & 0x400 == 0,
                    "Linked Minecraft folders are unsupported"
                );
            }
        }
    }
    Ok(p)
}
fn download(url: &str, path: &Path, hash: &str, limit: u64) -> Result<()> {
    if !hash.is_empty() && path.is_file() {
        let bytes = std::fs::read(path)?;
        if format!("{:x}", Sha1::digest(bytes)) == hash {
            return Ok(());
        }
    }
    let bytes = get(url, limit)?;
    if !hash.is_empty() {
        anyhow::ensure!(
            format!("{:x}", Sha1::digest(&bytes)) == hash,
            "Minecraft file checksum mismatch"
        );
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension("download");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
fn java(major: u64, override_path: &str, progress: &Sender<Outcome>) -> Result<PathBuf> {
    if !override_path.trim().is_empty() {
        let path = PathBuf::from(override_path);
        anyhow::ensure!(path.is_file(), "Choose a Java executable");
        return Ok(path);
    }
    let target = root().join("java").join(major.to_string());
    if let Some(path) = find_java(&target) {
        return Ok(path);
    }
    let _ = progress.send(Outcome::Status(format!(
        "Downloading managed Java {major}…"
    )));
    let info = metadata(&format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse"
    ))?;
    let package = &info[0]["binary"]["package"];
    let url = package["link"]
        .as_str()
        .context("No Windows Java runtime is available for this Minecraft version")?;
    anyhow::ensure!(
        url.starts_with("https://github.com/adoptium/"),
        "Unexpected Java runtime source"
    );
    let bytes = get(url, 300 * 1024 * 1024)?;
    use sha2::Sha256;
    anyhow::ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == package["checksum"].as_str().unwrap_or_default(),
        "Java runtime checksum mismatch"
    );
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut total = 0u64;
    for n in 0..archive.len() {
        let mut file = archive.by_index(n)?;
        total += file.size();
        anyhow::ensure!(
            total <= 1024 * 1024 * 1024,
            "Java runtime archive too large"
        );
        if file.is_dir() {
            continue;
        }
        let path = relative(&target, file.name())?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut output = std::fs::File::create(path)?;
        std::io::copy(&mut file, &mut output)?;
    }
    find_java(&target).context("Java runtime archive has no executable")
}
fn find_java(root: &Path) -> Option<PathBuf> {
    if root.join("bin/java.exe").is_file() {
        return Some(root.join("bin/java.exe"));
    }
    std::fs::read_dir(root).ok()?.flatten().find_map(|e| {
        if e.path().is_dir() && e.path().join("bin/java.exe").is_file() {
            Some(e.path().join("bin/java.exe"))
        } else {
            None
        }
    })
}
fn coordinate(name: &str) -> Result<String> {
    let p: Vec<_> = name.split(':').collect();
    anyhow::ensure!(
        (3..=4).contains(&p.len()) && p.iter().all(|s| safe_component(s)),
        "Invalid Minecraft library coordinate"
    );
    Ok(format!(
        "{}/{}/{}/{}-{}{}.jar",
        p[0].replace('.', "/"),
        p[1],
        p[2],
        p[1],
        p[2],
        if p.len() == 4 {
            format!("-{}", p[3])
        } else {
            String::new()
        }
    ))
}
fn rules(value: &Value) -> bool {
    let Some(rules) = value["rules"].as_array() else {
        return true;
    };
    let mut allowed = false;
    for r in rules {
        let os = &r["os"];
        if r.get("features").is_some() {
            continue;
        }
        if os["name"].as_str().is_some_and(|s| s != "windows")
            || os["arch"]
                .as_str()
                .is_some_and(|s| !matches!(s, "x86_64" | "amd64"))
        {
            continue;
        }
        allowed = r["action"] == "allow";
    }
    allowed
}
fn install(i: &Instance, progress: Sender<Outcome>) -> Result<String> {
    let manifest = metadata(MANIFEST)?;
    let version = manifest["versions"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["id"] == i.version))
        .context("Minecraft version no longer exists")?;
    let bytes = get(
        version["url"]
            .as_str()
            .context("Version metadata missing")?,
        16 * 1024 * 1024,
    )?;
    anyhow::ensure!(
        format!("{:x}", Sha1::digest(&bytes)) == version["sha1"].as_str().unwrap_or_default(),
        "Version metadata checksum mismatch"
    );
    let base: Value = serde_json::from_slice(&bytes)?;
    let major = base["javaVersion"]["majorVersion"].as_u64().unwrap_or(8);
    let java = java(major, &i.java, &progress)?;
    let target = dir(i);
    std::fs::create_dir_all(&target)?;
    let game = root()
        .join("versions")
        .join(&i.version)
        .join(format!("{}.jar", i.version));
    let client_file = &base["downloads"]["client"];
    download(
        client_file["url"]
            .as_str()
            .context("Client download missing")?,
        &game,
        client_file["sha1"].as_str().unwrap_or_default(),
        128 * 1024 * 1024,
    )?;
    let mut profile = base.clone();
    if i.loader == "fabric" || i.loader == "quilt" {
        let (host, path) = if i.loader == "fabric" {
            ("meta.fabricmc.net", "v2")
        } else {
            ("meta.quiltmc.org", "v3")
        };
        let loaders = metadata(&format!(
            "https://{host}/{path}/versions/loader/{}",
            i.version
        ))?;
        let selected = loaders
            .as_array()
            .and_then(|a| {
                if i.loader_version.is_empty() {
                    a.iter()
                        .find(|l| l["loader"]["stable"] == true)
                        .or_else(|| a.first())
                } else {
                    a.iter()
                        .find(|l| l["loader"]["version"] == i.loader_version)
                }
            })
            .context("This loader does not support this Minecraft version")?;
        let loader_version = selected["loader"]["version"]
            .as_str()
            .context("Loader version missing")?;
        let extra = metadata(&format!(
            "https://{host}/{path}/versions/loader/{}/{loader_version}/profile/json",
            i.version
        ))?;
        merge(&mut profile, &extra);
    } else if i.loader == "forge" || i.loader == "neoforge" {
        let loader = if !i.loader_version.is_empty() {
            i.loader_version.clone()
        } else if i.loader == "forge" {
            let promos = metadata(
                "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json",
            )?;
            promos["promos"][format!("{}-recommended", i.version)]
                .as_str()
                .or_else(|| promos["promos"][format!("{}-latest", i.version)].as_str())
                .context("Forge is unavailable for this Minecraft version")?
                .into()
        } else {
            bail!("Choose a NeoForge loader version (for example 21.1.200 for Minecraft 1.21.1)")
        };
        anyhow::ensure!(
            crate::repository::valid_slug(&loader),
            "Invalid loader version"
        );
        let full = if i.loader == "forge" {
            format!("{}-{loader}", i.version)
        } else {
            loader.clone()
        };
        let url = if i.loader == "forge" {
            format!(
                "https://maven.minecraftforge.net/net/minecraftforge/forge/{full}/forge-{full}-installer.jar"
            )
        } else {
            format!(
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/{full}/neoforge-{full}-installer.jar"
            )
        };
        let installer = target.join("loader-installer.jar");
        let hash = String::from_utf8(get(&format!("{url}.sha1"), 1024)?)?;
        download(
            &url,
            &installer,
            hash.split_whitespace().next().unwrap_or_default(),
            32 * 1024 * 1024,
        )?;
        std::fs::write(target.join("launcher_profiles.json"), "{\"profiles\":{}}")?;
        let log = std::fs::File::create(target.join("loader-install.log"))?;
        let _ = progress.send(Outcome::Status(format!(
            "Running official {} installer…",
            i.loader
        )));
        let mut cmd = Command::new(&java);
        cmd.arg("-jar")
            .arg(&installer)
            .arg("--installClient")
            .arg(&target)
            .current_dir(&target)
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        hide(&mut cmd);
        anyhow::ensure!(
            cmd.status()?.success(),
            "Loader installer failed; see loader-install.log"
        );
        let profiles = std::fs::read_dir(target.join("versions"))?
            .flatten()
            .filter_map(|e| {
                std::fs::read(
                    e.path()
                        .join(format!("{}.json", e.file_name().to_string_lossy())),
                )
                .ok()
            })
            .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
            .collect::<Vec<_>>();
        let extra = profiles
            .iter()
            .find(|p| p["inheritsFrom"] == i.version)
            .context("Loader installer did not produce a compatible profile")?;
        merge(&mut profile, extra);
    }
    let _ = progress.send(Outcome::Status(
        "Downloading libraries and native files…".into(),
    ));
    let mut classpath = Vec::new();
    let natives = target.join("natives");
    std::fs::create_dir_all(&natives)?;
    for library in profile["libraries"]
        .as_array()
        .context("Minecraft libraries missing")?
    {
        if !rules(library) {
            continue;
        }
        let name = library["name"].as_str().context("Library name missing")?;
        let artifact = &library["downloads"]["artifact"];
        let path = artifact["path"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or(coordinate(name)?);
        let is_installed = target.join("libraries").join(&path);
        let local = if is_installed.is_file() {
            is_installed
        } else {
            relative(&root().join("libraries"), &path)?
        };
        if !local.is_file() || artifact["sha1"].as_str().is_some() {
            let url = artifact["url"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "{}{}",
                        library["url"]
                            .as_str()
                            .unwrap_or("https://libraries.minecraft.net/"),
                        path
                    )
                });
            download(
                &url,
                &local,
                artifact["sha1"].as_str().unwrap_or_default(),
                64 * 1024 * 1024,
            )?;
        }
        classpath.push(local);
        if let Some(native) = library["natives"]["windows"].as_str() {
            let classifier = native.replace("${arch}", "64");
            let file = &library["downloads"]["classifiers"][&classifier];
            if let Some(url) = file["url"].as_str() {
                let native_path = relative(
                    &root().join("libraries"),
                    file["path"].as_str().context("Native path missing")?,
                )?;
                download(
                    url,
                    &native_path,
                    file["sha1"].as_str().unwrap_or_default(),
                    32 * 1024 * 1024,
                )?;
                for (path, bytes) in crate::runtime::archive_files(&std::fs::read(native_path)?)? {
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
                    {
                        let output =
                            relative(&natives, &path.to_string_lossy().replace('\\', "/"))?;
                        std::fs::create_dir_all(output.parent().context("Invalid native path")?)?;
                        std::fs::write(output, bytes)?;
                    }
                }
            }
        }
    }
    classpath.push(game);
    let asset_info = &base["assetIndex"];
    let asset_bytes = get(
        asset_info["url"].as_str().context("Asset index missing")?,
        16 * 1024 * 1024,
    )?;
    anyhow::ensure!(
        format!("{:x}", Sha1::digest(&asset_bytes))
            == asset_info["sha1"]
                .as_str()
                .context("Asset index checksum missing")?,
        "Asset index checksum mismatch"
    );
    let assets: Value = serde_json::from_slice(&asset_bytes)?;
    let assets_root = root().join("assets");
    std::fs::create_dir_all(assets_root.join("indexes"))?;
    let asset_id = asset_info["id"]
        .as_str()
        .context("Asset index ID missing")?;
    std::fs::write(
        relative(&assets_root, &format!("indexes/{asset_id}.json"))?,
        asset_bytes,
    )?;
    let objects = assets["objects"]
        .as_object()
        .context("Asset objects missing")?;
    for (index, (_name, a)) in objects.iter().enumerate() {
        let hash = a["hash"].as_str().context("Asset hash missing")?;
        anyhow::ensure!(
            hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid asset hash"
        );
        if index % 100 == 0 {
            let _ = progress.send(Outcome::Status(format!(
                "Downloading assets {index}/{}…",
                objects.len()
            )));
        }
        download(
            &format!(
                "https://resources.download.minecraft.net/{}/{hash}",
                &hash[..2]
            ),
            &relative(&assets_root, &format!("objects/{}/{hash}", &hash[..2]))?,
            hash,
            16 * 1024 * 1024,
        )?;
    }
    std::fs::write(target.join("launch.json"),json!({"profile":profile,"java":java,"classpath":classpath,"assets":assets_root,"asset_id":asset_id}).to_string())?;
    save(i)?;
    Ok(format!("{} is installed", i.name))
}
fn merge(base: &mut Value, extra: &Value) {
    if let Some(libraries) = extra["libraries"].as_array() {
        base["libraries"]
            .as_array_mut()
            .unwrap()
            .extend(libraries.iter().cloned());
    }
    for kind in ["game", "jvm"] {
        if let Some(args) = extra["arguments"][kind].as_array() {
            if !base["arguments"][kind].is_array() {
                base["arguments"][kind] = json!([]);
            }
            base["arguments"][kind]
                .as_array_mut()
                .unwrap()
                .extend(args.iter().cloned());
        }
    }
    if extra["mainClass"].is_string() {
        base["mainClass"] = extra["mainClass"].clone();
    }
}
fn hide(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
}
fn expanded(value: &Value, vars: &BTreeMap<&str, String>) -> Vec<String> {
    let replace = |s: &str| {
        let mut s = s.to_owned();
        for (key, value) in vars {
            s = s.replace(&format!("${{{key}}}"), value);
        }
        s
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|v| {
            if let Some(s) = v.as_str() {
                vec![replace(s)]
            } else if rules(v) {
                if let Some(s) = v["value"].as_str() {
                    vec![replace(s)]
                } else {
                    v["value"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str().map(replace))
                        .collect()
                }
            } else {
                vec![]
            }
        })
        .collect()
}
fn launch(i: &Instance, id: &str) -> Result<Child> {
    let account = crate::minecraft_auth::ready(id)?;
    let folder = dir(i);
    let config: Value = serde_json::from_slice(
        &std::fs::read(folder.join("launch.json")).context("Install this instance first")?,
    )?;
    let profile = &config["profile"];
    let classpath = config["classpath"]
        .as_array()
        .context("Instance classpath missing")?
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let vars = BTreeMap::from([
        ("auth_player_name", account.name),
        ("auth_uuid", account.id),
        ("auth_access_token", account.access),
        ("auth_session", String::new()),
        ("user_type", "msa".into()),
        ("version_name", i.version.clone()),
        ("version_type", "release".into()),
        ("game_directory", folder.to_string_lossy().into_owned()),
        (
            "assets_root",
            config["assets"].as_str().unwrap_or_default().into(),
        ),
        (
            "assets_index_name",
            config["asset_id"].as_str().unwrap_or_default().into(),
        ),
        (
            "natives_directory",
            folder.join("natives").to_string_lossy().into_owned(),
        ),
        ("launcher_name", "Canna".into()),
        ("launcher_version", env!("CARGO_PKG_VERSION").into()),
        ("classpath", classpath.clone()),
        ("user_properties", "{}".into()),
        ("clientid", id.into()),
        ("auth_xuid", String::new()),
        (
            "library_directory",
            folder.join("libraries").to_string_lossy().into_owned(),
        ),
        ("classpath_separator", ";".into()),
    ]);
    let mut cmd = Command::new(config["java"].as_str().context("Instance Java missing")?);
    cmd.current_dir(&folder)
        .arg(format!("-Xmx{}M", i.memory.clamp(512, 16384)));
    let jvm = expanded(&profile["arguments"]["jvm"], &vars);
    if jvm.is_empty() {
        cmd.arg(format!(
            "-Djava.library.path={}",
            folder.join("natives").display()
        ))
        .arg("-cp")
        .arg(classpath);
    } else {
        cmd.args(jvm);
    }
    cmd.arg(
        profile["mainClass"]
            .as_str()
            .context("Minecraft main class missing")?,
    );
    if let Some(legacy) = profile["minecraftArguments"].as_str() {
        let list = json!(legacy.split_whitespace().collect::<Vec<_>>());
        cmd.args(expanded(&list, &vars));
    } else {
        cmd.args(expanded(&profile["arguments"]["game"], &vars));
    }
    let log = std::fs::File::create(folder.join("canna-launch.log"))?;
    cmd.stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    hide(&mut cmd);
    Ok(cmd.spawn()?)
}
enum Outcome {
    PlaySetup(Result<Box<(crate::modpacks::Modpack, crate::model::InstalledGame)>>),
    Status(String),
    SignIn(crate::minecraft_auth::SignInEvent),
    Versions(Vec<String>),
    Done(Result<String>),
    Launched(Result<(String, Child)>),
}
pub struct Minecraft {
    play_lab: crate::play_lab::Lab,
    play_setup: Option<(crate::modpacks::Modpack, crate::model::InstalledGame)>,
    pub open: bool,
    status: String,
    versions: Vec<String>,
    draft: Instance,
    pub creating: bool,
    pub discover_requested: bool,
    job: Option<Receiver<Outcome>>,
    running: BTreeMap<String, Child>,
    sign_in_popup: bool,
    sign_in_prompt: Option<crate::minecraft_auth::DevicePrompt>,
    sign_in_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
pub const CLIENT_ID: &str = "5c67b262-465a-4e7e-8486-c7d422d3eefc";
pub fn client_id() -> String {
    CLIENT_ID.into()
}

impl Default for Minecraft {
    fn default() -> Self {
        let preview = std::env::var_os("CANNA_SCREENSHOT").is_some()
            && std::env::args().any(|a| a == "--microsoft-sign-in-preview");
        Self {
            play_lab: Default::default(),
            play_setup: None,
            open: false,
            status: String::new(),
            versions: vec![],
            draft: Instance {
                id: String::new(),
                name: "Minecraft Family Pack".into(),
                version: "1.21.1".into(),
                loader: "fabric".into(),
                loader_version: String::new(),
                java: String::new(),
                memory: 4096,
            },
            creating: false,
            discover_requested: false,
            job: None,
            running: BTreeMap::new(),
            sign_in_popup: preview,
            sign_in_prompt: preview.then(|| crate::minecraft_auth::DevicePrompt {
                code: "EXAMPLE".into(),
                url: "https://www.microsoft.com/link".into(),
                expires: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + 900,
            }),
            sign_in_cancel: Default::default(),
        }
    }
}
impl Minecraft {
    pub fn library(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some((pack, game)) = self.play_setup.clone() {
            if ui.button("‹ Minecraft instances").clicked() {
                self.play_setup = None;
                return;
            }
            self.play_lab
                .show(ui, &pack, &[game], &[], self.job.is_some());
            if let Some(copy) = self.play_lab.changed.take() {
                self.status = format!(
                    "Test setup saved: {}. Use its Minecraft instance entry to install/launch after review.",
                    copy.name
                );
            }
            return;
        }
        ui.heading("Minecraft library · preview");
        ui.label(
            "Connect your Microsoft account to play. Minecraft launching is still being verified.",
        );
        ui.label(&self.status);
        let busy = self.job.is_some();
        ui.collapsing("Microsoft account", |ui| {
            ui.label(
                crate::minecraft_auth::account()
                    .map(|a| format!("Playing as {}", a.name))
                    .unwrap_or_else(|_| "Not signed in".into()),
            );
            if ui
                .add_enabled(!busy, egui::Button::new("Sign in with Microsoft"))
                .clicked()
            {
                self.sign_in_popup = true;
                self.sign_in_prompt = None;
                self.status = "Requesting Microsoft sign-in…".into();
                self.sign_in_cancel = Default::default();
                let cancel = self.sign_in_cancel.clone();
                let id = client_id();
                self.work(move |tx| {
                    Outcome::Done(crate::minecraft_auth::sign_in(
                        &id,
                        |s| {
                            let _ = tx.send(Outcome::SignIn(s));
                        },
                        &cancel,
                    ))
                });
            }
            if ui.button("Open Microsoft code page").clicked() {
                ctx.open_url(egui::OpenUrl::new_tab("https://www.microsoft.com/link"));
            }
            if ui
                .add_enabled(!busy, egui::Button::new("Sign out of Minecraft"))
                .clicked()
            {
                crate::minecraft_auth::sign_out();
            }
        });
        egui::ScrollArea::vertical()
            .max_height(430.0)
            .show(ui, |ui| {
                for i in instances() {
                    ui.separator();
                    ui.strong(&i.name);
                    ui.label(format!("{} · {} {}", i.version, i.loader, i.loader_version));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("Play Lab"))
                            .clicked()
                        {
                            let i = i.clone();
                            self.status = "Reading local instance content…".into();
                            self.work(move |_| Outcome::PlaySetup(play_setup(&i).map(Box::new)));
                        }
                        if ui
                            .add_enabled(
                                !busy && !self.running.contains_key(&i.id),
                                egui::Button::new("Play"),
                            )
                            .clicked()
                        {
                            let i = i.clone();
                            let client_id = client_id();
                            self.work(move |_| {
                                Outcome::Launched(launch(&i, &client_id).map(|p| (i.id, p)))
                            });
                        }
                        if ui
                            .add_enabled(
                                self.running.contains_key(&i.id),
                                egui::Button::new("Stop instance"),
                            )
                            .clicked()
                            && let Some(mut p) = self.running.remove(&i.id)
                        {
                            let _ = p.kill();
                            let _ = p.wait();
                        }
                        if ui
                            .add_enabled(
                                !busy && !self.running.contains_key(&i.id),
                                egui::Button::new("Install / repair"),
                            )
                            .clicked()
                        {
                            let i = i.clone();
                            self.work(move |tx| Outcome::Done(install(&i, tx)));
                        }
                        if ui.button("Browse compatible content").clicked() {
                            self.discover_requested = true;
                            self.open = false;
                        }
                        if ui.button("Open folder").clicked() {
                            let _ = Command::new("explorer.exe").arg(dir(&i)).spawn();
                        }
                    });
                }
            });
    }
    pub fn busy(&self) -> bool {
        self.job.is_some() || !self.running.is_empty()
    }
    fn work(&mut self, f: impl FnOnce(Sender<Outcome>) -> Outcome + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        self.job = Some(rx);
        std::thread::spawn(move || {
            let result = f(tx.clone());
            let _ = tx.send(result);
        });
    }
    pub fn ui(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.job {
            let mut done = false;
            while let Ok(outcome) = rx.try_recv() {
                match outcome {
                    Outcome::PlaySetup(result) => {
                        match result {
                            Ok(setup) => self.play_setup = Some(*setup),
                            Err(e) => self.status = e.to_string(),
                        };
                        done = true;
                    }
                    Outcome::Status(s) => self.status = s,
                    Outcome::SignIn(event) => match event {
                        crate::minecraft_auth::SignInEvent::Status(s) => self.status = s,
                        crate::minecraft_auth::SignInEvent::Device(prompt) => {
                            ctx.open_url(egui::OpenUrl::new_tab(&prompt.url));
                            self.status = "Waiting for Microsoft approval in your browser…".into();
                            self.sign_in_prompt = Some(prompt);
                        }
                    },
                    Outcome::Versions(v) => {
                        self.versions = v;
                        done = true;
                    }
                    Outcome::Done(r) => {
                        self.status = r.unwrap_or_else(|e| e.to_string());
                        self.sign_in_prompt = None;
                        done = true;
                    }
                    Outcome::Launched(r) => {
                        match r {
                            Ok((id, child)) => {
                                self.running.insert(id, child);
                                self.status = "Minecraft started".into();
                            }
                            Err(e) => self.status = e.to_string(),
                        };
                        done = true;
                    }
                }
            }
            if done {
                self.job = None;
            }
        }
        self.running
            .retain(|_, p| p.try_wait().is_ok_and(|v| v.is_none()));
        if self.job.is_some() || !self.running.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        let mut open = self.open;
        if open {
            egui::Window::new("Minecraft")
                .open(&mut open)
                .default_width(780.0)
                .show(ctx, |ui| self.library(ui));
        }
        self.open = open;
        if self.creating {
            let busy = self.job.is_some();
            egui::Window::new("Create Minecraft instance").show(ctx, |ui| {
                if ui.button("Cancel").clicked() {
                    self.creating = false;
                }
                ui.label("Name");
                ui.text_edit_singleline(&mut self.draft.name);
                ui.horizontal(|ui| {
                    ui.label("Minecraft version");
                    egui::ComboBox::from_id_salt("minecraft-version")
                        .height(340.0)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .selected_text(&self.draft.version)
                        .show_ui(ui, |ui| {
                            let choices = self
                                .versions
                                .iter()
                                .map(|v| (v.clone(), v.clone()))
                                .collect::<Vec<_>>();
                            crate::ui_helpers::searchable_options(
                                ui,
                                &mut self.draft.version,
                                &choices,
                            );
                        });
                    ui.text_edit_singleline(&mut self.draft.version);
                    if ui
                        .add_enabled(!busy, egui::Button::new("Load official versions"))
                        .clicked()
                    {
                        self.work(|_| match metadata(MANIFEST) {
                            Ok(v) => Outcome::Versions(
                                v["versions"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter(|v| v["type"] == "release")
                                    .filter_map(|v| v["id"].as_str().map(str::to_owned))
                                    .collect(),
                            ),
                            Err(e) => Outcome::Done(Err(e)),
                        });
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Loader");
                    for loader in ["vanilla", "fabric", "forge", "neoforge", "quilt"] {
                        ui.selectable_value(&mut self.draft.loader, loader.into(), loader);
                    }
                });
                ui.label("Loader version · empty selects stable/latest when supported");
                ui.text_edit_singleline(&mut self.draft.loader_version);
                ui.label("Java executable override · empty downloads a managed matching runtime");
                ui.text_edit_singleline(&mut self.draft.java);
                if ui.button("Choose Java").clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .add_filter("Java executable", &["exe"])
                        .pick_file()
                {
                    self.draft.java = p.to_string_lossy().into_owned();
                }
                ui.add(
                    egui::Slider::new(&mut self.draft.memory, 1024..=16384).text("Memory (MiB)"),
                );
                if ui
                    .add_enabled(!busy, egui::Button::new("Create & install instance"))
                    .clicked()
                {
                    let mut i = self.draft.clone();
                    i.id = format!(
                        "mc-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos()
                    );
                    self.creating = false;
                    if let Err(e) = save(&i) {
                        self.status = e.to_string();
                    } else {
                        self.work(move |tx| Outcome::Done(install(&i, tx)));
                    }
                }
            });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.creating = false;
            }
        }
        if self.sign_in_popup {
            let mut popup = true;
            egui::Window::new("Connect your Microsoft account")
                .id(egui::Id::new("microsoft-sign-in"))
                .frame(
                    egui::Frame::window(&ctx.style())
                        .fill(egui::Color32::from_rgb(23, 32, 27))
                        .inner_margin(18),
                )
                .open(&mut popup)
                .collapsible(false)
                .resizable(false)
                .default_width(440.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label("Complete sign-in in the Microsoft page opened in your browser.");
                    if let Some(prompt) = &self.sign_in_prompt {
                        ui.add_space(12.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(&prompt.code)
                                    .monospace()
                                    .size(32.0)
                                    .strong(),
                            );
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Copy code").clicked() {
                                ctx.copy_text(prompt.code.clone());
                            }
                            if ui.button("Open Microsoft page").clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(&prompt.url));
                            }
                        });
                        let remaining = prompt.expires.saturating_sub(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        );
                        ui.label(format!(
                            "Code expires in {}:{:02}",
                            remaining / 60,
                            remaining % 60
                        ));
                        ctx.request_repaint_after(Duration::from_secs(1));
                    }
                    ui.add_space(12.0);
                    ui.label(&self.status);
                    if self.job.is_some() {
                        ui.spinner();
                        if ui.button("Cancel sign-in").clicked() {
                            self.sign_in_cancel
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            self.status = "Cancelling Microsoft sign-in…".into();
                        }
                    } else if ui.button("Done").clicked() {
                        self.sign_in_popup = false;
                    }
                });
            if !popup {
                self.sign_in_popup = false;
                self.sign_in_cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_hosts_and_loader_arguments_are_checked() {
        assert!(allowed("https://libraries.minecraft.net/a.jar").is_ok());
        assert!(allowed("https://libraries.minecraft.net.evil/a.jar").is_err());
        assert!(relative(Path::new("."), "../outside").is_err());
        assert_eq!(
            coordinate("net.fabricmc:fabric-loader:0.16.0").unwrap(),
            "net/fabricmc/fabric-loader/0.16.0/fabric-loader-0.16.0.jar"
        );
        assert!(!rules(
            &json!({"rules":[{"action":"allow","os":{"name":"linux"}}]})
        ));
        let mut v = json!({"libraries":[],"arguments":{"game":[],"jvm":[]},"mainClass":"vanilla"});
        merge(
            &mut v,
            &json!({"libraries":[{"name":"example"}],"mainClass":"loader","arguments":{"game":["--example"]}}),
        );
        assert_eq!(v["mainClass"], "loader");
        assert_eq!(
            expanded(
                &json!(["${version_name}",{"rules":[{"action":"allow","os":{"name":"linux"}}],"value":"skip"}]),
                &BTreeMap::from([("version_name", "1.21.1".into())])
            ),
            vec!["1.21.1"]
        );
    }
}

#[cfg(test)]
mod native_content_tests {
    use super::*;
    fn item(name: &str, dependencies: Vec<String>) -> crate::model::ModInfo {
        use sha2::Digest;
        serde_json::from_value(json!({"name":name,"version":"1","content_type":"mod","file":format!("Mods/{name}.zip"),"sha256":format!("{:x}",sha2::Sha256::digest(b"fixture")),"dependencies":dependencies,"provenance":{"filename":format!("{name}.jar"),"loaders":["fabric"],"game_versions":["1.21.1"]}})).unwrap()
    }
    fn instance() -> Instance {
        Instance {
            id: "fixture".into(),
            name: "Fixture".into(),
            version: "1.21.1".into(),
            loader: "fabric".into(),
            loader_version: String::new(),
            java: String::new(),
            memory: 1024,
        }
    }
    #[test]
    fn dependency_plan_is_recursive_verified_and_confined_to_instance() {
        let root = item("root", vec!["dep".into()]);
        let dep = item("dep", vec!["leaf".into()]);
        let leaf = item("leaf", vec![]);
        let mut game = serde_json::from_value::<crate::model::GameInfo>(
            json!({"app_id":u32::MAX,"name":"Minecraft","folder":"minecraft"}),
        )
        .unwrap();
        game.mods = vec![root.clone(), dep, leaf];
        let i = instance();
        let mut fetched = Vec::new();
        let plan = content_plan(&i, "", &game, &root, |m| {
            fetched.push(m.name.clone());
            Ok(b"fixture".to_vec())
        })
        .unwrap();
        assert_eq!(fetched, vec!["root", "dep", "leaf"]);
        assert_eq!(plan.len(), 3);
        assert!(
            plan.iter()
                .all(|(folder, name, _)| folder == &dir(&i).join("mods") && name.ends_with(".jar"))
        );
        assert!(
            content_plan(&i, "", &game, &root, |_| Ok(b"tampered".to_vec()))
                .unwrap_err()
                .to_string()
                .contains("Checksum")
        );
    }
    #[test]
    fn wrong_loader_version_and_paths_fail_before_network_or_writes() {
        let m = item("root", vec![]);
        let game = serde_json::from_value::<crate::model::GameInfo>(
            json!({"app_id":u32::MAX,"name":"Minecraft","folder":"minecraft"}),
        )
        .unwrap();
        let mut i = instance();
        i.loader = "forge".into();
        assert!(
            content_plan(&i, "", &game, &m, |_| panic!(
                "must not fetch incompatible content"
            ))
            .is_err()
        );
        i.loader = "fabric".into();
        i.version = "1.20.1".into();
        assert!(!content_compatible(&i, &m));
        i = instance();
        i.id = "../outside".into();
        assert!(
            content_plan(&i, "", &game, &m, |_| panic!(
                "must not fetch unsafe target"
            ))
            .is_err()
        );
        i = instance();
        let mut unsafe_item = m;
        unsafe_item.provenance["filename"] = json!("../outside.jar");
        assert!(content_plan(&i, "", &game, &unsafe_item, |_| Ok(b"fixture".to_vec())).is_err());
    }
}
