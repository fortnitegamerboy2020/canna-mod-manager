param([string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_canna_mod_manager.txt', [string]$Version = '0.2.5', [switch]$SourceOnly)
$ErrorActionPreference = 'Stop'
$cannaRoot = Split-Path $PSScriptRoot -Parent
$cannaToken = [IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
$cannaHeaders = @{ Authorization = "Bearer $cannaToken"; 'User-Agent' = 'Canna-Mod-Manager'; Accept = 'application/vnd.github+json' }
$cannaApi = 'https://api.github.com/repos/fortnitegamerboy2020/canna-mod-manager'
function Invoke-CannaApi([string]$Path, [string]$Method = 'GET', $Body = $null) {
    $cannaRequest = @{ Uri = "$cannaApi/$Path".TrimEnd('/'); Headers = $cannaHeaders; Method = $Method; TimeoutSec = 60 }
    if ($null -ne $Body) { $cannaRequest.Body = ($Body | ConvertTo-Json -Depth 30 -Compress); $cannaRequest.ContentType = 'application/json' }
    Invoke-RestMethod @cannaRequest
}
try {
    if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid release version' }
    $cannaPackage=[IO.File]::ReadAllText((Join-Path $cannaRoot 'Cargo.toml'))
    if ($cannaPackage -notmatch ('(?m)^version\s*=\s*"'+[regex]::Escape($Version)+'"')) { throw 'Cargo package version does not match release' }
    if (!$SourceOnly) {
        foreach($cannaArtifact in @('dist/Canna Mod Manager.exe','dist/Canna-Mod-Manager-Windows.zip')) {
            if (!(Test-Path -LiteralPath (Join-Path $cannaRoot $cannaArtifact))) { throw 'Release build is missing' }
        }
    }
    $cannaRepo = Invoke-CannaApi ''
    $cannaBranch = $cannaRepo.default_branch
    try { $cannaRef = Invoke-CannaApi "git/ref/heads/$cannaBranch" } catch {
        if ([int]$_.Exception.Response.StatusCode -ne 404 -and [int]$_.Exception.Response.StatusCode -ne 409) { throw }
        $null = Invoke-CannaApi 'contents/README.md' 'PUT' @{ message = 'Initialize private Canna application repository'; content = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("# Canna Mod Manager`n")); branch = $cannaBranch }
        $cannaRef = Invoke-CannaApi "git/ref/heads/$cannaBranch"
    }
    $cannaCommit = Invoke-CannaApi "git/commits/$($cannaRef.object.sha)"
    # Explicit source allowlist. Credential files, game assemblies, caches and build output are excluded.
    $cannaFiles = @('Cargo.toml', 'Cargo.lock', 'build.rs', 'build.ps1', '.gitignore', 'README.md')
    foreach ($cannaFolder in @('src', 'scripts', 'examples', 'repository-template')) {
        $cannaFiles += @(Get-ChildItem -LiteralPath (Join-Path $cannaRoot $cannaFolder) -Recurse -File | ForEach-Object { [IO.Path]::GetRelativePath($cannaRoot, $_.FullName).Replace('\','/') })
    }
    foreach ($cannaFolder in @('mods/DrillThroughBall', 'mods/ProceduralMaps', 'mods/Anvil', 'mods/FamilyVisuals', 'mods/FamilyCatalog', 'mods/TimeStopTimer')) {
        $cannaFiles += @(Get-ChildItem -LiteralPath (Join-Path $cannaRoot $cannaFolder) -File | ForEach-Object { [IO.Path]::GetRelativePath($cannaRoot, $_.FullName).Replace('\','/') })
    }
    $cannaEntries = @()
    foreach ($cannaFile in $cannaFiles) {
        $cannaBlob = Invoke-CannaApi 'git/blobs' 'POST' @{ content = [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $cannaRoot $cannaFile))); encoding = 'base64' }
        $cannaEntries += @{ path = $cannaFile; mode = '100644'; type = 'blob'; sha = $cannaBlob.sha }
    }
    $cannaTree = Invoke-CannaApi 'git/trees' 'POST' @{ base_tree = $cannaCommit.tree.sha; tree = $cannaEntries }
    $cannaNewCommit = Invoke-CannaApi 'git/commits' 'POST' @{ message = "Canna ${Version}: server catalog, account connection, website downloads and Minecraft preview"; tree = $cannaTree.sha; parents = @($cannaRef.object.sha) }
    $null = Invoke-CannaApi "git/refs/heads/$cannaBranch" 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    if ($SourceOnly) { "Published application source commit $($cannaNewCommit.sha)."; exit 0 }
    $cannaReleaseNotes = @'
Mod downloads, game artwork and BepInEx now come from the authenticated Canna server. Connect the desktop account through Settings and the website; Windows DPAPI protects the desktop session. The mod-repository token is no longer embedded.

Website downloads open Canna through a short-lived, single-use link and verify the downloaded file before it can be added to a matching Steam modpack. Existing Bopl modpacks keep access to migrated and historical archives. The website library includes external imports and game/provider/content filters.

Minecraft preview includes instance setup, managed Java, loader selection, Microsoft device sign-in and local skin import/export/application. Microsoft sign-in requires a registered public client ID. Live Minecraft login/launch and complete Minecraft modpack/content support are still pending; this is not a complete Minecraft launcher release.

Automatic updates wait for managed games, installation jobs, website transfers and skin application to finish. Application updates continue to use the separate private GitHub releases repository.

Validation: desktop unit/UI tests and strict Clippy; server authorization, transfer, catalog and encryption tests; migration checksum audit. Real Microsoft sign-in and family multiplayer were not tested for this release.
'@
    $cannaRelease = Invoke-CannaApi 'releases' 'POST' @{ tag_name = "v$Version"; target_commitish = $cannaNewCommit.sha; name = "Canna Mod Manager $Version"; draft = $true; prerelease = $false; body = $cannaReleaseNotes }
    foreach ($cannaUpload in @(
        @{ file = 'dist/Canna Mod Manager.exe'; name = 'Canna-Mod-Manager.exe'; mime = 'application/octet-stream' },
        @{ file = 'dist/Canna-Mod-Manager-Windows.zip'; name = 'Canna-Mod-Manager-Windows.zip'; mime = 'application/zip' }
    )) {
        $cannaUploadUri = "https://uploads.github.com/repos/fortnitegamerboy2020/canna-mod-manager/releases/$($cannaRelease.id)/assets?name=$($cannaUpload.name)"
        $cannaAsset = Invoke-RestMethod -Uri $cannaUploadUri -Method POST -Headers $cannaHeaders -InFile (Join-Path $cannaRoot $cannaUpload.file) -ContentType $cannaUpload.mime -TimeoutSec 300
        $cannaLocalHash = (Get-FileHash -LiteralPath (Join-Path $cannaRoot $cannaUpload.file) -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($cannaAsset.digest -ne "sha256:$cannaLocalHash") { throw 'GitHub asset digest mismatch' }
    }
    $null = Invoke-CannaApi "releases/$($cannaRelease.id)" 'PATCH' @{ draft = $false }
    "Published private Windows release v$Version and source commit $($cannaNewCommit.sha)."
} catch {
    'Application release publication failed; any draft release is retained for inspection.'
    if ($_.Exception.Response) { 'HTTP status: ' + [int]$_.Exception.Response.StatusCode }
    exit 1
} finally { $cannaToken = $null; $cannaHeaders = $null }

