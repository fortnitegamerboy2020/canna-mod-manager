//! Beta authorization is checked on the server before any support cache access.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const ORIGIN: &str = "https://cannamods.vip";
const MANIFEST_PATH: &str = "/api/v1/rebound/support-manifest";
const DOWNLOAD_PATH: &str = "/api/v1/rebound/support";
const PROFILE: &str = "rounds-public-1.1.2";
const MAX_BYTES: u64 = 128 * 1024 * 1024;
static CACHE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Deserialize)]
struct Manifest {
    authorized: bool,
    sha256: String,
    size: u64,
    profile: String,
    download_path: String,
}
impl Manifest {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.authorized,
            "Canna Rebound requires Beta access on this account"
        );
        ensure!(
            self.profile == PROFILE && self.download_path == DOWNLOAD_PATH,
            "Unsupported Rebound support manifest"
        );
        ensure!(
            self.size > 0 && self.size <= MAX_BYTES && valid_hash(&self.sha256),
            "Invalid Rebound support checksum or size"
        );
        Ok(())
    }
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(180))
        .user_agent(concat!("CannaDesktop/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
fn cache_root() -> PathBuf {
    crate::modpacks::directory()
        .parent()
        .unwrap()
        .join("rebound-support")
}
fn purge_at(root: &Path) -> Result<()> {
    crate::runtime::no_links(root)?;
    for entry in fs::read_dir(root).into_iter().flatten() {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.strip_suffix(".zip").is_some_and(valid_hash)
            || name.starts_with("rebound-") && name.ends_with(".pending")
        {
            crate::runtime::no_links(&entry.path())?;
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
pub fn clear_cache() -> Result<()> {
    let _guard = CACHE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Rebound cache is unavailable"))?;
    purge_at(&cache_root())
}
fn manifest(client: &reqwest::blocking::Client, origin: &str, token: &str) -> Result<Manifest> {
    ensure!(
        !token.is_empty(),
        "Sign in with a Beta account to use Canna Rebound"
    );
    let response = client
        .get(format!("{origin}{MANIFEST_PATH}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(20))
        .send()?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "{}",
        match status.as_u16() {
            401 => "Sign in again to verify Canna Rebound access",
            403 => "Canna Rebound requires Beta access on this account",
            _ => "Could not verify Canna Rebound access; retry when the server is available",
        }
    );
    let mut bytes = Vec::new();
    response.take(32769).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 32768, "Rebound manifest is oversized");
    let manifest: Manifest =
        serde_json::from_slice(&bytes).context("Invalid Rebound support manifest")?;
    manifest.validate()?;
    Ok(manifest)
}
fn checked_manifest(
    client: &reqwest::blocking::Client,
    origin: &str,
    root: &Path,
    token: &str,
) -> Result<Manifest> {
    match manifest(client, origin, token) {
        Ok(manifest) => Ok(manifest),
        Err(error) => {
            purge_at(root)?;
            Err(error)
        }
    }
}
fn checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn validate_bundle(bytes: &[u8], manifest: &Manifest) -> Result<()> {
    ensure!(
        bytes.len() as u64 == manifest.size
            && bytes.starts_with(b"PK\x03\x04")
            && checksum(bytes) == manifest.sha256,
        "Rebound support checksum mismatch"
    );
    Ok(())
}
fn bundle_with(
    client: &reqwest::blocking::Client,
    origin: &str,
    root: &Path,
    token: &str,
) -> Result<Vec<u8>> {
    // No cache read, creation or download precedes this fresh server check.
    let manifest = checked_manifest(client, origin, root, token)?;
    crate::runtime::no_links(root)?;
    let cached = root.join(format!("{}.zip", manifest.sha256));
    crate::runtime::no_links(&cached)?;
    if let Ok(metadata) = fs::metadata(&cached)
        && metadata.len() == manifest.size
        && let Ok(bytes) = fs::read(&cached)
        && validate_bundle(&bytes, &manifest).is_ok()
    {
        return Ok(bytes);
    }
    let response = client
        .get(format!("{origin}{}", manifest.download_path))
        .bearer_auth(token)
        .send()?;
    if !response.status().is_success() {
        purge_at(root)?;
        anyhow::bail!("Rebound support download was denied or unavailable; recheck Beta access");
    }
    let mut bytes = Vec::new();
    response.take(manifest.size + 1).read_to_end(&mut bytes)?;
    validate_bundle(&bytes, &manifest)?;
    // A revoked account or changed release cannot save bytes from an older check.
    let current = checked_manifest(client, origin, root, token)?;
    ensure!(
        current.sha256 == manifest.sha256 && current.size == manifest.size,
        "Rebound support changed during download; retry"
    );
    fs::create_dir_all(root)?;
    let temp = root.join(format!(
        "rebound-{}-{}.pending",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if cached.exists() {
            fs::remove_file(&cached)?;
        }
        fs::rename(&temp, &cached)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    Ok(bytes)
}
pub fn authorized_bundle(token: &str) -> Result<Vec<u8>> {
    #[cfg(all(test, canna_rebound_local_preview))]
    if token == "private-rebound-fixture" {
        return Ok(include_bytes!(concat!(env!("OUT_DIR"), "/ducttape-support.zip")).to_vec());
    }
    let _guard = CACHE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Rebound cache is unavailable"))?;
    bundle_with(&client()?, ORIGIN, &cache_root(), token)
}
pub fn verify_current(token: &str, expected: &str) -> Result<()> {
    #[cfg(all(test, canna_rebound_local_preview))]
    if token == "private-rebound-fixture" {
        return Ok(());
    }
    let _guard = CACHE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Rebound cache is unavailable"))?;
    let manifest = checked_manifest(&client()?, ORIGIN, &cache_root(), token)?;
    ensure!(
        manifest.sha256 == expected,
        "Rebound support changed after preflight; prepare the pack again"
    );
    Ok(())
}

fn verify_current_read_only_with(
    client: &reqwest::blocking::Client,
    origin: &str,
    token: &str,
    expected: &str,
) -> Result<()> {
    ensure!(
        valid_hash(expected),
        "Invalid installed Rebound support checksum; reapply the pack"
    );
    // Launch checks do not read, download, create or purge support cache files.
    let current = manifest(client, origin, token)?;
    ensure!(
        current.sha256 == expected,
        "Canna Rebound support changed; reapply the pack before launching"
    );
    Ok(())
}

pub fn verify_current_read_only(token: &str, expected: &str) -> Result<()> {
    #[cfg(all(test, canna_rebound_local_preview))]
    if token == "private-rebound-fixture" {
        return Ok(());
    }
    verify_current_read_only_with(&client()?, ORIGIN, token, expected)
}

pub struct Access {
    session: Zeroizing<String>,
    pending: Option<Receiver<(String, Result<(), String>)>>,
    pub allowed: bool,
    pub status: String,
    checked_at: Option<Instant>,
}
impl Default for Access {
    fn default() -> Self {
        Self {
            session: Zeroizing::new(String::new()),
            pending: None,
            allowed: false,
            status: String::new(),
            checked_at: None,
        }
    }
}
impl Access {
    pub fn observe(&mut self, ctx: &egui::Context, session: &str) {
        if self.session.as_str() != session {
            let had_session = !self.session.is_empty();
            self.session = Zeroizing::new(session.into());
            self.allowed = false;
            self.pending = None;
            self.checked_at = None;
            self.status.clear();
            if had_session {
                std::thread::spawn(|| {
                    let _ = clear_cache();
                });
            }
        }
        if let Some((source, result)) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            if source == session {
                self.checked_at = Some(Instant::now());
                self.allowed = result.is_ok();
                self.status = result.err().unwrap_or_else(|| {
                    "Beta access verified. Support downloads when you prepare a ROUNDS pack.".into()
                });
            }
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if self
            .checked_at
            .is_some_and(|at| at.elapsed() >= Duration::from_secs(60))
        {
            self.allowed = false;
        }
    }
    pub fn checking(&self) -> bool {
        self.pending.is_some()
    }
    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        self.allowed = false;
        if self.session.is_empty() {
            self.status = "Sign in with a Beta account to use Canna Rebound.".into();
            return;
        }
        self.status = "Checking Beta access…".into();
        let session = self.session.clone();
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let _guard = CACHE_LOCK
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Rebound cache is unavailable"))?;
                checked_manifest(&client()?, ORIGIN, &cache_root(), &session)?;
                Ok(())
            })()
            .map_err(|error| error.to_string());
            let _ = tx.send((session.to_string(), result));
            ctx.request_repaint();
        });
    }
    pub fn needs_check(&self) -> bool {
        !self.session.is_empty()
            && self.pending.is_none()
            && self
                .checked_at
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::TcpListener;

    fn scratch() -> PathBuf {
        std::env::temp_dir().join(format!(
            "canna-rebound-access-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn payload() -> Vec<u8> {
        b"PK\x03\x04reviewed support fixture".to_vec()
    }
    fn manifest_bytes(bytes: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&json!({"authorized":true,"sha256":checksum(bytes),
            "size":bytes.len(),"profile":PROFILE,"download_path":DOWNLOAD_PATH}))
        .unwrap()
    }
    fn server(replies: Vec<(&'static str, u16, Vec<u8>)>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for (path, status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0u8; 1];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 16384);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {path} HTTP/1.1\r\n")));
                assert!(
                    request
                        .to_lowercase()
                        .contains("authorization: bearer beta-fixture\r\n")
                );
                write!(
                    stream,
                    "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (origin, worker)
    }
    #[test]
    fn denied_account_never_downloads_or_creates_support_cache() {
        let root = scratch();
        let (origin, server) = server(vec![(MANIFEST_PATH, 403, vec![])]);
        let error = bundle_with(&client().unwrap(), &origin, &root, "beta-fixture").unwrap_err();
        assert!(error.to_string().contains("Beta access"));
        assert!(!root.exists());
        server.join().unwrap();
    }
    #[test]
    fn read_only_launch_manifest_check_is_fresh_rejects_revocation_or_stale_support_and_preserves_cache()
     {
        let root = scratch();
        crate::modpacks::with_test_root(root.clone(), || {
            let cache = cache_root();
            fs::create_dir_all(&cache).unwrap();
            let bytes = payload();
            let cached = cache.join(format!("{}.zip", checksum(&bytes)));
            fs::write(&cached, &bytes).unwrap();
            fs::write(cache.join("keep.txt"), b"unrelated fixture").unwrap();
            let client = client().unwrap();
            for (status, reply, allowed) in [
                (200, manifest_bytes(&bytes), true),
                (403, vec![], false),
                (200, manifest_bytes(b"PK\x03\x04updated support"), false),
                (503, vec![], false),
                (200, manifest_bytes(&bytes), true),
            ] {
                let (origin, worker) = server(vec![(MANIFEST_PATH, status, reply)]);
                let result = verify_current_read_only_with(
                    &client,
                    &origin,
                    "beta-fixture",
                    &checksum(&bytes),
                );
                assert_eq!(result.is_ok(), allowed);
                worker.join().unwrap();
                assert_eq!(fs::read(&cached).unwrap(), bytes);
                assert_eq!(
                    fs::read(cache.join("keep.txt")).unwrap(),
                    b"unrelated fixture"
                );
                assert_eq!(fs::read_dir(&cache).unwrap().count(), 2);
            }
            assert!(
                verify_current_read_only_with(&client, "http://127.0.0.1:1", "", &checksum(&bytes))
                    .unwrap_err()
                    .to_string()
                    .contains("Sign in")
            );
            assert!(
                verify_current_read_only_with(
                    &client,
                    "http://127.0.0.1:1",
                    "beta-fixture",
                    "invalid"
                )
                .unwrap_err()
                .to_string()
                .contains("checksum")
            );
        });
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn authorized_download_is_pinned_and_cached_but_every_read_reauthorizes() {
        let root = scratch();
        let bytes = payload();
        let metadata = manifest_bytes(&bytes);
        let (origin, server) = server(vec![
            (MANIFEST_PATH, 200, metadata.clone()),
            (DOWNLOAD_PATH, 200, bytes.clone()),
            (MANIFEST_PATH, 200, metadata.clone()),
            (MANIFEST_PATH, 200, metadata),
        ]);
        let client = client().unwrap();
        assert_eq!(
            bundle_with(&client, &origin, &root, "beta-fixture").unwrap(),
            bytes
        );
        assert_eq!(
            fs::read(root.join(format!("{}.zip", checksum(&bytes)))).unwrap(),
            bytes
        );
        assert_eq!(
            bundle_with(&client, &origin, &root, "beta-fixture").unwrap(),
            bytes
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn revoked_access_purges_only_managed_support_files_without_reading_them() {
        let root = scratch();
        fs::create_dir(&root).unwrap();
        let cached = root.join(format!("{}.zip", "a".repeat(64)));
        fs::write(&cached, b"old support").unwrap();
        fs::write(root.join("keep.txt"), b"unrelated fixture").unwrap();
        let (origin, server) = server(vec![(MANIFEST_PATH, 403, vec![])]);
        assert!(bundle_with(&client().unwrap(), &origin, &root, "beta-fixture").is_err());
        assert!(!cached.exists());
        assert!(root.join("keep.txt").exists());
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_download_or_revocation_before_storage_never_saves_payload() {
        for revoked in [false, true] {
            let root = scratch();
            let bytes = payload();
            let mut replies = vec![(MANIFEST_PATH, 200, manifest_bytes(&bytes))];
            if revoked {
                replies.push((DOWNLOAD_PATH, 200, bytes));
                replies.push((MANIFEST_PATH, 403, vec![]));
            } else {
                replies.push((
                    DOWNLOAD_PATH,
                    200,
                    b"PK\x03\x04tampered support fixture".to_vec(),
                ));
            }
            let (origin, server) = server(replies);
            assert!(bundle_with(&client().unwrap(), &origin, &root, "beta-fixture").is_err());
            assert!(!root.exists());
            server.join().unwrap();
        }
    }
    #[test]
    fn manifest_cannot_authorize_external_paths_unknown_profiles_or_unbounded_payloads() {
        let baseline: Manifest = serde_json::from_slice(&manifest_bytes(&payload())).unwrap();
        assert!(baseline.validate().is_ok());
        for mutation in 0..6 {
            let mut manifest = baseline.clone();
            match mutation {
                0 => manifest.authorized = false,
                1 => manifest.download_path = "https://example.com/engine.zip".into(),
                2 => manifest.profile = "unknown".into(),
                3 => manifest.size = MAX_BYTES + 1,
                4 => manifest.sha256 = "../cached.zip".into(),
                _ => manifest.size = 0,
            }
            assert!(manifest.validate().is_err());
        }
    }
    #[test]
    fn account_switch_drops_old_authorization_and_inflight_metadata() {
        let ctx = egui::Context::default();
        let mut access = Access::default();
        access.observe(&ctx, "");
        let (tx, rx) = mpsc::channel();
        access.pending = Some(rx);
        access.allowed = true;
        tx.send(("old-account".into(), Ok(()))).unwrap();
        access.observe(&ctx, "new-account");
        assert!(!access.allowed);
        assert!(access.pending.is_none());
        assert!(access.needs_check());
    }
}
