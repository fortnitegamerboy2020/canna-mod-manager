param([string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_canna_mod_manager.txt', [string]$Version = '0.2.10', [switch]$SourceOnly)
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
    $cannaFiles = @('Cargo.toml', 'Cargo.lock', 'build.rs', 'build.ps1', '.gitignore', 'README.md', 'AGENTS.md')
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
    $cannaNewCommit = Invoke-CannaApi 'git/commits' 'POST' @{ message = "Canna ${Version}: Minecraft artwork and consistent card cropping"; tree = $cannaTree.sha; parents = @($cannaRef.object.sha) }
    $null = Invoke-CannaApi "git/refs/heads/$cannaBranch" 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    if ($SourceOnly) { "Published application source commit $($cannaNewCommit.sha)."; exit 0 }
    $cannaReleaseNotes = @'
Minecraft uses the supplied 1920 x 1080 banner for its library and Discover cards, with centered cover cropping and the same rounded frame treatment as other games. Game library artwork now consistently fills its frame rather than leaving unused space.

The updated grass-block sidebar icon retains its original colors, is trimmed to its visible bounds and is downsampled for efficient display.

Minecraft API application has been submitted and approval is pending. This artwork update does not claim verified Minecraft login or launching.

Validation: Desktop tests and strict Clippy passed; the native library and Discover layouts were visually inspected.
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

