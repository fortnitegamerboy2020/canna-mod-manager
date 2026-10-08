param([string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_canna_mod_manager.txt', [string]$Version = '0.2.39', [switch]$SourceOnly, [string]$CommitMessage = '')
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
    if (!$SourceOnly) {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $cannaZip = [IO.Compression.ZipFile]::OpenRead((Join-Path $cannaRoot 'dist/Canna-Mod-Manager-Windows.zip'))
        try {
            $cannaExecutables = @($cannaZip.Entries | Where-Object FullName -eq 'Canna Mod Manager.exe')
            if ($cannaExecutables.Count -ne 1) { throw 'Portable ZIP must contain one launcher EXE.' }
            $cannaZipStream = $cannaExecutables[0].Open()
            $cannaZipHasher = [Security.Cryptography.SHA256]::Create()
            try { $cannaZipHash = ([BitConverter]::ToString($cannaZipHasher.ComputeHash($cannaZipStream))).Replace('-','').ToLowerInvariant() }
            finally { $cannaZipStream.Dispose(); $cannaZipHasher.Dispose() }
            if ($cannaZipHash -ne (Get-FileHash -LiteralPath (Join-Path $cannaRoot 'dist/Canna Mod Manager.exe')).Hash.ToLowerInvariant()) { throw 'Portable ZIP launcher differs from release EXE.' }
        } finally { $cannaZip.Dispose() }
    }
    $cannaRepo = Invoke-CannaApi ''
    $cannaBranch = $cannaRepo.default_branch
    try { $cannaRef = Invoke-CannaApi "git/ref/heads/$cannaBranch" } catch {
        if ([int]$_.Exception.Response.StatusCode -ne 404 -and [int]$_.Exception.Response.StatusCode -ne 409) { throw }
        $null = Invoke-CannaApi 'contents/README.md' 'PUT' @{ message = 'Initialize private Canna application repository'; content = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("# Canna Mod Manager`n")); branch = $cannaBranch }
        $cannaRef = Invoke-CannaApi "git/ref/heads/$cannaBranch"
    }
    $cannaCommit = Invoke-CannaApi "git/commits/$($cannaRef.object.sha)"
    # Bounded publication for the dependency selection fix. Never enumerate the
    # shared checkout: other chat work and private artifacts must not be included.
    $cannaFiles = @('Cargo.toml','Cargo.lock','src/modpacks.rs','src/pack_ui.rs',
        'src/game_compat.rs','README.md','server/web/help.html','scripts/Publish-DependencyFix.ps1')
    $cannaExistingTree = Invoke-CannaApi "git/trees/$($cannaCommit.tree.sha)?recursive=1"
    $cannaExistingBlobs = @{}
    foreach ($cannaEntry in $cannaExistingTree.tree) { if ($cannaEntry.type -eq 'blob') { $cannaExistingBlobs[$cannaEntry.path] = $cannaEntry.sha } }
    $cannaEntries = @()
    foreach ($cannaFile in $cannaFiles) {
        $cannaBytes = [IO.File]::ReadAllBytes((Join-Path $cannaRoot $cannaFile))
        if ($cannaFile.EndsWith('.sh')) {
            $cannaBytes = [Text.Encoding]::UTF8.GetBytes([Text.Encoding]::UTF8.GetString($cannaBytes).Replace("`r`n", "`n"))
        }
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
    $cannaLatest = Invoke-CannaApi "git/ref/heads/$cannaBranch"
    if ($cannaLatest.object.sha -ne $cannaRef.object.sha) { throw 'Source branch advanced during publication; retry against its new head.' }
    $null = Invoke-CannaApi "git/refs/heads/$cannaBranch" 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    if ($SourceOnly) { "Published application source commit $($cannaNewCommit.sha)."; exit 0 }
    $cannaReleaseNotes = @'
Canna 0.2.39 lets you disable or remove declared mod dependencies and saves those choices. Adding or updating one selected mod no longer adds, replaces or re-enables other packages automatically. Dependency metadata remains visible for manual setup and diagnostics; archive, hash, path and game checks remain in place.

An enabled official Thunderstore kieron_exe-DuctTape package permits its UnboundLib 3.2.14 / MMHook 1.0.0 library replacements on public ROUNDS. Explicit legacy requirements, original HollowPurple requirements and duplicate-plugin checks remain. DuctTape's author says to keep UnboundLib, MMHook and RoundsWithFriends installed because its patcher swaps assemblies at game startup. This release has fixture and unit verification; DuctTape gameplay and multiplayer were not tested.

Apply the edited pack with the game closed to change Canna-managed game files. The previously published HollowPurple Fixed 1.8.2 public preview remains available.
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
    & ssh -i $cannaSshKey -o BatchMode=yes canna-admin@165.227.83.76 "set -e; test `"`$(sha256sum /home/canna-admin/canna-v$Version.exe | cut -d ' ' -f 1)`" = '$cannaDigest'; sudo mkdir -p /opt/canna/releases && sudo install -m 644 /home/canna-admin/canna-v$Version.exe /opt/canna/releases/v$Version.exe && sudo install -m 644 /home/canna-admin/canna-latest.json /opt/canna/releases/latest.json.tmp && sudo mv /opt/canna/releases/latest.json.tmp /opt/canna/releases/latest.json && rm /home/canna-admin/canna-v$Version.exe /home/canna-admin/canna-latest.json"
    if ($LASTEXITCODE -ne 0) { throw 'Server release installation failed' }
    $null = Invoke-CannaApi "releases/$($cannaRelease.id)" 'PATCH' @{ draft = $false }
    "Published Windows release v$Version and source commit $($cannaNewCommit.sha)."
} catch {
    'Application release publication failed; any draft release is retained for inspection.'
    if ($_.Exception.Response) { 'HTTP status: ' + [int]$_.Exception.Response.StatusCode }
    exit 1
} finally { $cannaToken = $null; $cannaHeaders = $null }

