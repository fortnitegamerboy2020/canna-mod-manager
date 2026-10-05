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

const API: &str = "https://api.github.com/repos/fortnitegamerboy2020/canna-mod-manager";
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
    id: u64,
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
pub fn check(token: &str, current: &str) -> Result<Option<Ready>> {
    if token.trim().is_empty() {
        bail!("Updater has no read credential for the private release repository");
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Canna-Mod-Manager")
        .build()?;
    let response = client
        .get(format!("{API}/releases/latest"))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .context("Could not reach release repository")?;
    if response.status() == 404 {
        bail!("No accessible release yet; check the desktop token's repository selection");
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
        .find(|a| a.name == "Canna-Mod-Manager.exe")
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
    let mut response = client
        .get(format!("{API}/releases/assets/{}", asset.id))
        .bearer_auth(token)
        .header("Accept", "application/octet-stream")
        .send()
        .context("Could not download update")?;
    for _ in 0..4 {
        if !response.status().is_redirection() {
            break;
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .context("Update redirect missing")?
            .to_str()?;
        let url = reqwest::Url::parse(location).context("Invalid update redirect")?;
        let host = url.host_str().unwrap_or("");
        if url.scheme() != "https"
            || !(host == "release-assets.githubusercontent.com"
                || host == "objects.githubusercontent.com"
                || host == "github.com")
        {
            bail!("Untrusted update download host");
        }
        // Never forward the private repository token to an asset host.
        response = client.get(url).send().context("Asset download failed")?;
    }
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
    let file = folder.join(format!("Canna-{}.exe", release.tag_name));
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
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let target = std::env::current_exe()?;
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
    $parent = Get-Process -Id {pid} -ErrorAction SilentlyContinue
    if ($parent) {{ $parent.WaitForExit() }}
    if ((Get-FileHash -LiteralPath $staged -Algorithm SHA256).Hash.ToLowerInvariant() -ne '{hash}') {{ throw 'Staged checksum mismatch' }}
    $installed = $false
    if (Test-Path -LiteralPath $backup) {{ Remove-Item -LiteralPath $backup -Force }}
    for ($attempt = 0; $attempt -lt 30; $attempt++) {{
        try {{
            Move-Item -LiteralPath $target -Destination $backup -Force
            $installed = $true
            break
        }} catch {{ Start-Sleep -Milliseconds 500 }}
    }}
    if (-not $installed) {{ throw 'Could not replace application; previous application retained' }}
    try {{
        Copy-Item -LiteralPath $staged -Destination $target -Force
        Start-Process -FilePath $target -WorkingDirectory (Split-Path -LiteralPath $target)
    }} catch {{
        if (Test-Path -LiteralPath $target) {{ Remove-Item -LiteralPath $target -Force }}
        Move-Item -LiteralPath $backup -Destination $target -Force
        Start-Process -FilePath $target
        throw 'Replacement or launch failed; restored previous application'
    }}
    'Update installed successfully' | Set-Content -LiteralPath $log
}} catch {{ $_.Exception.Message | Set-Content -LiteralPath $log }}
"#,
            target = literal(&target),
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
        let _ = ready;
        bail!("Automatic replacement currently supports Windows only")
    }
}
#[cfg(test)]
mod tests {
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
