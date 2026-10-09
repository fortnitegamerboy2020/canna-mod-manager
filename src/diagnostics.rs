//! Explicitly opted-in, fixed-schema launcher diagnostics; never upload raw logs.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime};

fn session_nonce() -> Option<String> {
    let mut bytes = [0u8; 16];
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
        };
        // A fresh OS-generated nonce identifies this run, never a device or account.
        if unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        } < 0
        {
            return None;
        }
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .ok()?
            .read_exact(&mut bytes)
            .ok()?;
    }
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Clone, Serialize)]
pub struct Report {
    schema: u8,
    consent: bool,
    session: String,
    desktop_version: String,
    platform: String,
    game_id: u32,
    operation: String,
    phase: String,
    code: String,
    http_status: Option<u16>,
    mod_count: usize,
    bliss_enabled: bool,
    loader_present: bool,
}
pub fn classify(error: &str) -> (&'static str, Option<u16>) {
    let text = error.to_ascii_lowercase();
    let status = [401, 403, 404, 409, 429, 500, 502, 503, 504]
        .into_iter()
        .find(|status| {
            text.contains(&format!("({status}"))
                || text.contains(&format!("{status} "))
                || text.contains(&format!("http {status}"))
        });
    let code = if status == Some(401) || text.contains("session expired") {
        "session_expired"
    } else if status == Some(403)
        || text.contains("access denied")
        || text.contains("permission denied")
    {
        "access_denied"
    } else if status == Some(429) {
        "rate_limited"
    } else if text.contains("checksum") || text.contains("sha-256") || text.contains("corrupt") {
        "checksum_mismatch"
    } else if text.contains("unsupported") || text.contains("incompatible") {
        "unsupported_compatibility"
    } else if text.contains("dependency") || text.contains("dependencies") {
        "dependency_unavailable"
    } else if text.contains("still running") || text.contains("close the game") {
        "game_running"
    } else if text.contains("changed after") || text.contains("preflight") {
        "prepared_state_changed"
    } else if text.contains("network")
        || text.contains("timed out")
        || text.contains("connect")
        || status.is_some()
    {
        "network_failure"
    } else if text.contains("process") || text.contains("launch") || text.contains("steam") {
        "launch_failure"
    } else if text.contains("file") || text.contains("directory") || text.contains("os error") {
        "filesystem_failure"
    } else {
        "setup_failure"
    };
    (code, status)
}
pub fn phase(progress: &str) -> &'static str {
    let text = progress.to_ascii_lowercase();
    if text.contains("bepinex") || text.contains("framework") {
        "loader"
    } else if text.contains("compatibility") || text.contains("bliss") {
        "compatibility"
    } else if text.contains("preparing mod") || text.contains("downloading mod") {
        "mod_download"
    } else if text.contains("approved") || text.contains("catalog") || text.contains("dependencies")
    {
        "catalog"
    } else if text.contains("restore") || text.contains("vanilla files") {
        "restore"
    } else if text.contains("launch") || text.contains("steam") {
        "launch"
    } else {
        "prepare"
    }
}
pub fn report(
    game_id: u32,
    operation: &str,
    phase: &str,
    error: &str,
    mod_count: usize,
    bliss_enabled: bool,
    loader_present: bool,
) -> Report {
    let (code, http_status) = classify(error);
    Report {
        schema: 1,
        consent: true,
        session: String::new(),
        desktop_version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        game_id,
        operation: operation.into(),
        phase: phase.into(),
        code: code.into(),
        http_status,
        mod_count: mod_count.min(1000),
        bliss_enabled,
        loader_present,
    }
}
/// Only classify fresh launcher/loader markers from a launch Canna initiated.
/// Generic gameplay exceptions and old log files do not become reports.
pub fn startup_code(
    snapshot: &crate::console::Snapshot,
    requested: SystemTime,
) -> Option<&'static str> {
    for file in &snapshot.files {
        if !file.modified.is_some_and(|time| time >= requested) {
            continue;
        }
        for line in file.text.lines() {
            let text = line.to_ascii_lowercase();
            if text.contains("canna bliss")
                && (text.contains("mismatch") || text.contains("incompatible"))
            {
                return Some("compatibility_mismatch");
            }
            let failure = text.contains("error")
                || text.contains("exception")
                || text.contains("failed")
                || text.contains("could not load")
                || text.contains("cannot load");
            if failure && (text.contains("chainloader") || text.contains("preloader")) {
                return Some("loader_initialization");
            }
            if failure
                && text.contains("plugin")
                && (text.contains("dependency")
                    || text.contains("could not load")
                    || text.contains("cannot load"))
            {
                return Some("plugin_load_failure");
            }
        }
    }
    None
}
pub fn startup_report(
    game_id: u32,
    code: &str,
    bliss_enabled: bool,
    loader_present: bool,
) -> Report {
    let mut report = report(
        game_id,
        "startup",
        "launch",
        "",
        0,
        bliss_enabled,
        loader_present,
    );
    report.code = code.into();
    report.phase = match code {
        "compatibility_mismatch" => "compatibility",
        "loader_initialization" | "plugin_load_failure" => "loader",
        _ => "launch",
    }
    .into();
    report
}
struct State {
    sent: std::collections::BTreeSet<String>,
    last: Option<Instant>,
    in_flight: bool,
    status: &'static str,
}
fn transmit(endpoint: &str, body: Vec<u8>, enabled: &AtomicBool) -> anyhow::Result<bool> {
    use std::io::Read;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    if !enabled.load(Ordering::SeqCst) {
        return Ok(false);
    }
    let response = client
        .post(endpoint)
        .header("Content-Type", "application/json")
        .body(body)
        .send()?;
    if !response.status().is_success() {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    response.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Ok(false);
    }
    Ok(serde_json::from_slice::<serde_json::Value>(&bytes)?
        .get("accepted")
        .and_then(|v| v.as_bool())
        == Some(true))
}
#[derive(Clone)]
pub struct Reporter {
    enabled: Arc<AtomicBool>,
    session: String,
    state: Arc<Mutex<State>>,
}
impl Reporter {
    pub fn new(enabled: bool) -> Self {
        let session = session_nonce().unwrap_or_default();
        Self {
            enabled: Arc::new(AtomicBool::new(enabled)),
            session,
            state: Arc::new(Mutex::new(State {
                sent: Default::default(),
                last: None,
                in_flight: false,
                status: "Ready; no reports sent this run.",
            })),
        }
    }
    pub fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    pub fn status(&self) -> String {
        if !self.enabled.load(Ordering::SeqCst) || self.session.is_empty() {
            return "Off. Launcher reports are not sent.".into();
        }
        self.state
            .lock()
            .map(|state| state.status.to_owned())
            .unwrap_or_else(|_| "Reporting unavailable.".into())
    }
    #[cfg_attr(test, allow(dead_code))]
    fn begin(&self, body: &[u8]) -> bool {
        if !self.enabled.load(Ordering::SeqCst) || self.session.is_empty() {
            return false;
        }
        let digest = format!("{:x}", Sha256::digest(body));
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.in_flight
            || state.sent.len() >= 20
            || state.sent.contains(&digest)
            || state
                .last
                .is_some_and(|last| last.elapsed() < Duration::from_secs(60))
        {
            return false;
        }
        state.sent.insert(digest);
        state.last = Some(Instant::now());
        state.in_flight = true;
        state.status = "Sending anonymous diagnostic codes…";
        true
    }
    #[cfg(test)]
    pub fn submit(&self, _report: Report) {}
    #[cfg(not(test))]
    pub fn submit(&self, mut report: Report) {
        report.session = self.session.clone();
        let Ok(body) = serde_json::to_vec(&report) else {
            return;
        };
        if !self.begin(&body) {
            return;
        }
        let reporter = self.clone();
        std::thread::spawn(move || {
            let accepted = transmit(
                "https://cannamods.vip/api/v1/launcher/diagnostics",
                body,
                &reporter.enabled,
            )
            .unwrap_or(false);
            if let Ok(mut state) = reporter.state.lock() {
                state.in_flight = false;
                state.status = if accepted {
                    "Anonymous report sent."
                } else {
                    "Report unavailable; your game setup is unchanged."
                };
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_never_retain_raw_paths_accounts_tokens_or_log_text() {
        let error = "403 Forbidden for https://private.example?token=secret C:\\Users\\private-name\\game.dll username=player";
        let body = serde_json::to_string(&report(
            1686940, "prepare", "loader", error, 14, false, false,
        ))
        .unwrap();
        for private in [
            "private-name",
            "secret",
            "private.example",
            "username",
            "game.dll",
            "Forbidden",
        ] {
            assert!(!body.contains(private));
        }
        assert!(body.contains("access_denied"));
        assert!(body.contains("403"));
        assert_eq!(phase("Checking BepInEx…"), "loader");
    }
    #[test]
    fn disabled_reports_are_not_queued_and_repeated_reports_are_bounded() {
        let reporter = Reporter::new(false);
        assert_eq!(reporter.session.len(), 32);
        assert!(!reporter.begin(b"first"));
        reporter.enable(true);
        assert!(reporter.begin(b"first"));
        assert!(!reporter.begin(b"second"));
        {
            let mut state = reporter.state.lock().unwrap();
            state.in_flight = false;
            state.last = Some(Instant::now() - Duration::from_secs(61));
        }
        assert!(!reporter.begin(b"first"));
        assert!(reporter.begin(b"second"));
        reporter.enable(false);
        assert!(!reporter.begin(b"third"));
        assert_eq!(reporter.status(), "Off. Launcher reports are not sent.");
    }
    #[test]
    fn startup_reports_ignore_old_output_and_unrelated_gameplay_errors() {
        let now = SystemTime::now();
        let mut snapshot = crate::console::Snapshot {running: Ok(true), files: vec![crate::console::LogFile {path: None, modified: Some(now-Duration::from_secs(1)), text: "[Error:Chainloader] Cannot load plugin private-name dependency C:\\Users\\private".into()}]};
        assert_eq!(startup_code(&snapshot, now), None);
        snapshot.files[0].modified = Some(now);
        assert_eq!(startup_code(&snapshot, now), Some("loader_initialization"));
        snapshot.files[0].text = "NullReferenceException: gameplay card private-name".into();
        assert_eq!(startup_code(&snapshot, now), None);
        snapshot.files[0].text =
            "[Error: BepInEx] Could not load plugin due to missing dependency private-name".into();
        assert_eq!(startup_code(&snapshot, now), Some("plugin_load_failure"));
        let body =
            serde_json::to_string(&startup_report(1557740, "plugin_load_failure", true, true))
                .unwrap();
        assert!(!body.contains("private-name"));
        assert!(body.contains("\"phase\":\"loader\""));
    }
    #[test]
    fn transport_uses_no_account_credentials_and_requires_server_acceptance() {
        use std::io::{Read, Write};
        for (status, answer, expected) in [
            ("200 OK", "{\"accepted\":true}", true),
            ("200 OK", "{}", false),
            ("403 Forbidden", "{}", false),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/diagnostics", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut received = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let size = socket.read(&mut chunk).unwrap();
                    assert!(size > 0);
                    received.extend_from_slice(&chunk[..size]);
                    if let Some(end) = received.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers =
                            String::from_utf8_lossy(&received[..end]).to_ascii_lowercase();
                        let count: usize = headers
                            .lines()
                            .find_map(|s| s.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if received.len() >= end + 4 + count {
                            break;
                        }
                    }
                }
                let raw = String::from_utf8(received).unwrap();
                assert!(!raw.to_ascii_lowercase().contains("authorization:"));
                assert!(!raw.to_ascii_lowercase().contains("cookie:"));
                assert!(!raw.contains("private-player"));
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",answer.len()).as_bytes()).unwrap();
            });
            let body = serde_json::to_vec(&report(
                1686940,
                "prepare",
                "loader",
                "403 private-player",
                14,
                false,
                false,
            ))
            .unwrap();
            assert_eq!(
                transmit(&endpoint, body, &AtomicBool::new(true)).unwrap(),
                expected
            );
            server.join().unwrap();
        }
        assert!(!transmit("http://127.0.0.1:1", vec![], &AtomicBool::new(false)).unwrap());
    }
}
