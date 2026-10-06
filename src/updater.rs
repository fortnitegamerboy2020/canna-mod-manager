use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const API: &str = "https://cannamods.vip/updates";
const LIMIT: u64 = 150 * 1024 * 1024;
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    digest: Option<String>,
}
pub struct Ready {
    pub version: String,
    pub file: PathBuf,
    pub hash: String,
}

fn version(value: &str) -> Option<[u32; 3]> {
    let parts: Vec<_> = value.trim_start_matches('v').split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some([
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ])
}
pub fn check(current: &str) -> Result<Option<Ready>> {
    check_at(current, API, "Canna-Mod-Manager.exe", "Canna")
}
#[allow(dead_code)] // Used by the separate maintenance binary.
pub fn check_maintenance(current: &str) -> Result<Option<Ready>> {
    check_at(
        current,
        "https://cannamods.vip/updates/maintenance",
        "Canna-Updater.exe",
        "Canna-Updater",
    )
}
fn check_at(current: &str, api: &str, asset_name: &str, stage_name: &str) -> Result<Option<Ready>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Canna-Mod-Manager")
        .build()?;
    let response = client
        .get(format!("{api}/latest"))
        .send()
        .context("Could not reach release repository")?;
    if response.status() == 404 {
        bail!("Application update is not available yet");
    }
    if !response.status().is_success() {
        bail!("Release check returned HTTP {}", response.status());
    }
    let mut bytes = Vec::new();
    response.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("Release metadata exceeds limit");
    }
    let release: Release = serde_json::from_slice(&bytes)?;
    let remote = version(&release.tag_name).context("Invalid release version")?;
    if release.draft
        || release.prerelease
        || remote <= version(current).context("Invalid application version")?
    {
        return Ok(None);
    }
    let asset = release
        .assets
        .into_iter()
        .find(|a| a.name == asset_name)
        .context("Release has no Windows application")?;
    let hash = asset
        .digest
        .as_deref()
        .and_then(|h| h.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("Release has no valid SHA-256 digest")?
        .to_lowercase();
    if asset.size == 0 || asset.size > LIMIT {
        bail!("Update size is invalid");
    }
    let response = client
        .get(format!("{api}/{}", release.tag_name))
        .send()
        .context("Could not download update")?;
    if !response.status().is_success() {
        bail!("Asset download returned HTTP {}", response.status());
    }
    let mut binary = Vec::new();
    response.take(LIMIT + 1).read_to_end(&mut binary)?;
    if binary.len() as u64 != asset.size
        || binary.len() as u64 > LIMIT
        || !binary.starts_with(b"MZ")
    {
        bail!("Update is incomplete or is not a Windows executable");
    }
    if format!("{:x}", Sha256::digest(&binary)) != hash {
        bail!("Update checksum does not match the release");
    }
    let folder = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CannaModManager/updates");
    fs::create_dir_all(&folder)?;
    let file = folder.join(format!(
        "{stage_name}-{}-{}.exe",
        release.tag_name,
        std::process::id()
    ));
    fs::write(&file, binary)?;
    Ok(Some(Ready {
        version: release.tag_name,
        file,
        hash,
    }))
}
fn literal(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}
pub fn apply(ready: &Ready) -> Result<()> {
    apply_to(ready, &std::env::current_exe()?)
}
pub fn apply_to(ready: &Ready, target: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        anyhow::ensure!(
            target.is_absolute(),
            "Update target must be an absolute path"
        );
        let script = ready.file.with_extension("ps1");
        let backup = target.with_extension("previous.exe");
        let log = ready.file.with_extension("log");
        let content = format!(
            r#"$ErrorActionPreference = 'Stop'
$target = {target}
$staged = {staged}
$backup = {backup}
$log = {log}
try {{
    try {{ [Diagnostics.Process]::GetProcessById({pid}).WaitForExit() }} catch [ArgumentException] {{ }}
    $stream = [IO.File]::OpenRead($staged)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {{ $actual = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }} finally {{ $stream.Dispose(); $hasher.Dispose() }}
    if ($actual -ne '{hash}') {{ throw 'Staged checksum mismatch' }}
    $updateLock = [IO.File]::Open($target + '.update.lock', 'OpenOrCreate', 'ReadWrite', 'None')
    $installed = $false
    if ([IO.File]::Exists($target)) {{
      $oldStream = [IO.File]::OpenRead($target)
      $oldHasher = [Security.Cryptography.SHA256]::Create()
      try {{ $oldHash = [BitConverter]::ToString($oldHasher.ComputeHash($oldStream)).Replace('-', '').ToLowerInvariant() }} finally {{ $oldStream.Dispose(); $oldHasher.Dispose() }}
      if ([IO.File]::Exists($backup)) {{ [IO.File]::Delete($backup) }}
    }}
    for ($attempt = 0; $attempt -lt 30; $attempt++) {{
        try {{
            if ([IO.File]::Exists($target)) {{
              [IO.File]::Move($target, $backup)
              [IO.File]::WriteAllText($backup + '.sha256', $oldHash)
            }}
            $installed = $true
            break
        }} catch {{ [Threading.Thread]::Sleep(500) }}
    }}
    if (-not $installed) {{ throw 'Could not replace application; previous application retained' }}
    try {{
        [IO.File]::Copy($staged, $target, $true)
        $launch = New-Object Diagnostics.ProcessStartInfo
        $launch.FileName = $target
        $launch.WorkingDirectory = [IO.Path]::GetDirectoryName($target)
        $launch.UseShellExecute = $true
        $started = [Diagnostics.Process]::Start($launch)
        if ($started.WaitForExit(2000) -and $started.ExitCode -ne 0) {{ throw 'Updated application exited with an error' }}
    }} catch {{
        if ([IO.File]::Exists($target)) {{ [IO.File]::Delete($target) }}
        if ([IO.File]::Exists($backup)) {{
          [IO.File]::Move($backup, $target)
          $null = [Diagnostics.Process]::Start($target)
        }}
        throw 'Replacement or launch failed; restored previous application'
    }}
    [IO.File]::WriteAllText($log, 'Update installed successfully')
}} catch {{ [IO.File]::WriteAllText($log, $_.Exception.Message) }} finally {{
    if ($null -ne $updateLock) {{ $updateLock.Dispose(); [IO.File]::Delete($target + '.update.lock') }}
}}
"#,
            target = literal(target),
            staged = literal(&ready.file),
            backup = literal(&backup),
            log = literal(&log),
            pid = std::process::id(),
            hash = ready.hash
        );
        fs::write(&script, content)?;
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(script)
            .creation_flags(0x08000000)
            .spawn()
            .context("Could not start update helper")?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (ready, target);
        bail!("Automatic replacement currently supports Windows only")
    }
}
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Downloads and checks the published public server update, without applying it"]
    fn live_server_update_has_verified_digest() {
        let ready = super::check("0.1.0").unwrap().unwrap();
        assert!(ready.file.exists());
        assert!(super::check("999.0.0").unwrap().is_none());
        std::fs::remove_file(ready.file).unwrap();
    }

    use super::*;
    #[test]
    fn stable_versions_never_downgrade() {
        assert!(version("v0.2.0") > version("0.1.9"));
        assert!(version("0.10.0") > version("0.2.0"));
        assert_eq!(version("1.0.0"), version("v1.0.0"));
        assert!(version("1.0.0-beta").is_none());
        assert!(version("../1.0.0").is_none());
    }
    #[test]
    fn powershell_paths_are_literal() {
        assert_eq!(
            literal(Path::new("C:/User's `$ folder/app.exe")),
            "'C:/User''s `$ folder/app.exe'"
        );
    }
}
