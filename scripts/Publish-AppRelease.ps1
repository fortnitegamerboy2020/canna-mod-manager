param([string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_canna_mod_manager.txt', [string]$Version = '0.2.15', [switch]$SourceOnly, [string]$CommitMessage = '')
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
    if (!$SourceOnly -and [Diagnostics.FileVersionInfo]::GetVersionInfo((Join-Path $cannaRoot 'dist/Canna Mod Manager.exe')).ProductVersion -ne $Version) { throw 'Release EXE version does not match the requested version.' }
    $cannaRepo = Invoke-CannaApi ''
    $cannaBranch = $cannaRepo.default_branch
    try { $cannaRef = Invoke-CannaApi "git/ref/heads/$cannaBranch" } catch {
        if ([int]$_.Exception.Response.StatusCode -ne 404 -and [int]$_.Exception.Response.StatusCode -ne 409) { throw }
        $null = Invoke-CannaApi 'contents/README.md' 'PUT' @{ message = 'Initialize private Canna application repository'; content = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("# Canna Mod Manager`n")); branch = $cannaBranch }
        $cannaRef = Invoke-CannaApi "git/ref/heads/$cannaBranch"
    }
    $cannaCommit = Invoke-CannaApi "git/commits/$($cannaRef.object.sha)"
    # Explicit source allowlist. Credential files, game assemblies, caches and build output are excluded.
    $cannaFiles = @('Cargo.toml', 'Cargo.lock', 'build.rs', 'build.ps1', '.gitignore', 'README.md', 'AGENTS.md', 'LICENSE', 'SECURITY.md')
    foreach ($cannaFolder in @('src', 'scripts', 'examples', 'repository-template', 'server/src', 'server/web', 'server/deploy')) {
        $cannaFiles += @(Get-ChildItem -LiteralPath (Join-Path $cannaRoot $cannaFolder) -Recurse -File | Where-Object { $_.Extension -ne ".pyc" -and $_.FullName -notmatch "[\\/]__pycache__[\\/]" } | ForEach-Object { [IO.Path]::GetRelativePath($cannaRoot, $_.FullName).Replace('\','/') })
    }
    foreach ($cannaFolder in @('mods/DrillThroughBall', 'mods/ProceduralMaps', 'mods/Anvil', 'mods/FamilyVisuals', 'mods/FamilyCatalog', 'mods/TimeStopTimer', 'mods/CannaAutoHop')) {
        $cannaFiles += @(Get-ChildItem -LiteralPath (Join-Path $cannaRoot $cannaFolder) -File | ForEach-Object { [IO.Path]::GetRelativePath($cannaRoot, $_.FullName).Replace('\','/') })
    }
    $cannaFiles += @('server/Cargo.toml', 'server/Cargo.lock', 'server/README.md')
    $cannaExistingTree = Invoke-CannaApi "git/trees/$($cannaCommit.tree.sha)?recursive=1"
    $cannaExistingBlobs = @{}
    foreach ($cannaEntry in $cannaExistingTree.tree) { if ($cannaEntry.type -eq 'blob') { $cannaExistingBlobs[$cannaEntry.path] = $cannaEntry.sha } }
    $cannaEntries = @()
    foreach ($cannaFile in $cannaFiles) {
        $cannaBytes = [IO.File]::ReadAllBytes((Join-Path $cannaRoot $cannaFile))
        $cannaBlobPrefix = [Text.Encoding]::UTF8.GetBytes("blob $($cannaBytes.Length)`0")
        $cannaGitBytes = [byte[]]::new($cannaBlobPrefix.Length + $cannaBytes.Length)
        [Array]::Copy($cannaBlobPrefix, 0, $cannaGitBytes, 0, $cannaBlobPrefix.Length)
        [Array]::Copy($cannaBytes, 0, $cannaGitBytes, $cannaBlobPrefix.Length, $cannaBytes.Length)
        $cannaHasher = [Security.Cryptography.SHA1]::Create()
        try { $cannaBlobSha = ([BitConverter]::ToString($cannaHasher.ComputeHash($cannaGitBytes))).Replace('-','').ToLowerInvariant() } finally { $cannaHasher.Dispose() }
        if ($cannaExistingBlobs[$cannaFile] -eq $cannaBlobSha) { $cannaBlob = @{sha=$cannaBlobSha} }
        else { $cannaBlob = Invoke-CannaApi 'git/blobs' 'POST' @{ content = [Convert]::ToBase64String($cannaBytes); encoding = 'base64' } }
        $cannaEntries += @{ path = $cannaFile; mode = '100644'; type = 'blob'; sha = $cannaBlob.sha }
    }
    $cannaTree = Invoke-CannaApi 'git/trees' 'POST' @{ base_tree = $cannaCommit.tree.sha; tree = $cannaEntries }
    if (!$CommitMessage) { $CommitMessage = "Canna ${Version}: source update" }
    $cannaNewCommit = Invoke-CannaApi 'git/commits' 'POST' @{ message = $CommitMessage; tree = $cannaTree.sha; parents = @($cannaRef.object.sha) }
    $null = Invoke-CannaApi "git/refs/heads/$cannaBranch" 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    if ($SourceOnly) { "Published application source commit $($cannaNewCommit.sha)."; exit 0 }
    $cannaReleaseNotes = @'
Canna 0.2.31 adds Play Lab: invite-only readiness lobbies, local archive/config recovery, dependency-closed test copies, local diagnostics and config editing, previewed anonymous compatibility reports and private support tickets, stable/experimental manifest channels, and local memory/benchmark tools. Server 0.3.48 enforces private ownership, retention and bounded records. No automatic telemetry, raw log upload or device identifiers are added. Shared tools cover supported Unity, Source and managed Minecraft profiles; new live game/multiplayer checks remain unverified, and Minecraft launch still needs API approval. Automated regression and desktop/mobile browser checks passed; see the workflow test matrix.
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
    # Only public release metadata and executable are mirrored; no publisher credentials leave this script.
    $cannaExe = Join-Path $cannaRoot 'dist/Canna Mod Manager.exe'
    $cannaManifest = Join-Path $cannaRoot 'dist/latest.json'
    $cannaDigest = (Get-FileHash -LiteralPath $cannaExe -Algorithm SHA256).Hash.ToLowerInvariant()
    @{ tag_name="v$Version"; draft=$false; prerelease=$false; assets=@(@{name='Canna-Mod-Manager.exe';size=(Get-Item -LiteralPath $cannaExe).Length;digest="sha256:$cannaDigest"}) } | ConvertTo-Json -Depth 5 | ForEach-Object { [IO.File]::WriteAllText($cannaManifest, $_, [Text.UTF8Encoding]::new($false)) }
    $cannaSshKey = Join-Path $env:USERPROFILE '.ssh/canna_server_ed25519'
    & scp -q -i $cannaSshKey $cannaExe "canna-admin@165.227.83.76:/home/canna-admin/canna-v$Version.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Server release upload failed' }
    & scp -q -i $cannaSshKey $cannaManifest 'canna-admin@165.227.83.76:/home/canna-admin/canna-latest.json'
    if ($LASTEXITCODE -ne 0) { throw 'Server manifest upload failed' }
    & ssh -i $cannaSshKey -o BatchMode=yes canna-admin@165.227.83.76 "sudo mkdir -p /opt/canna/releases && sudo install -m 644 /home/canna-admin/canna-v$Version.exe /opt/canna/releases/v$Version.exe && sudo install -m 644 /home/canna-admin/canna-latest.json /opt/canna/releases/latest.json.tmp && sudo mv /opt/canna/releases/latest.json.tmp /opt/canna/releases/latest.json && rm /home/canna-admin/canna-v$Version.exe /home/canna-admin/canna-latest.json"
    if ($LASTEXITCODE -ne 0) { throw 'Server release installation failed' }
    $null = Invoke-CannaApi "releases/$($cannaRelease.id)" 'PATCH' @{ draft = $false }
    "Published private Windows release v$Version and source commit $($cannaNewCommit.sha)."
} catch {
    'Application release publication failed; any draft release is retained for inspection.'
    if ($_.Exception.Response) { 'HTTP status: ' + [int]$_.Exception.Response.StatusCode }
    exit 1
} finally { $cannaToken = $null; $cannaHeaders = $null }

