param(
    [string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_canna_mod_manager.txt',
    [string]$Version = '0.2.41',
    [switch]$SourceOnly,
    [switch]$CheckOnly,
    [string]$CommitMessage = '',
    [string]$ReleaseNotesFile = 'target/canna-rebound-release-notes.txt'
)
$ErrorActionPreference = 'Stop'
$cannaRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$cannaToken = $null
$cannaHeaders = $null
$cannaPhase = 'local preflight'
$cannaApi = 'https://api.github.com/repos/fortnitegamerboy2020/canna-mod-manager'
$cannaUtf8 = [Text.UTF8Encoding]::new($false, $true)

function Invoke-CannaApi([string]$Path, [string]$Method = 'GET', $Body = $null) {
    $cannaRequest = @{ Uri = "$cannaApi/$Path".TrimEnd('/'); Headers = $cannaHeaders; Method = $Method; TimeoutSec = 60 }
    if ($null -ne $Body) { $cannaRequest.Body = ($Body | ConvertTo-Json -Depth 30 -Compress); $cannaRequest.ContentType = 'application/json' }
    Invoke-RestMethod @cannaRequest
}
function Assert-CannaReleaseUnused {
    # The tag endpoint can omit drafts, so authenticated release listings are
    # checked too. Bound pagination and fail closed rather than miss an old draft.
    $cannaExists = $true
    try { $null = Invoke-CannaApi "releases/tags/v$Version" }
    catch {
        if (!$_.Exception.Response -or [int]$_.Exception.Response.StatusCode -ne 404) { throw }
        $cannaExists = $false
    }
    if ($cannaExists) { throw 'This release version already exists; inspect it rather than republish' }
    for ($cannaPage = 1; $cannaPage -le 5; $cannaPage++) {
        $cannaReleases = @(Invoke-CannaApi "releases?per_page=100&page=$cannaPage")
        if (@($cannaReleases | Where-Object tag_name -eq "v$Version").Count) { throw 'This release version has an existing release or draft; inspect it rather than republish' }
        if ($cannaReleases.Count -lt 100) { return }
    }
    throw 'Release history exceeds bounded preflight; inspect version availability'
}
function Assert-CannaVersionUnused {
    $cannaExists = $true
    try { $null = Invoke-CannaApi "git/ref/tags/v$Version" }
    catch {
        if (!$_.Exception.Response -or [int]$_.Exception.Response.StatusCode -ne 404) { throw }
        $cannaExists = $false
    }
    if ($cannaExists) { throw 'This release tag already exists; inspect it rather than republish' }
    Assert-CannaReleaseUnused
}
function Assert-CannaTagCommit([string]$Expected) {
    $cannaTag = Invoke-CannaApi "git/ref/tags/v$Version"
    $cannaObject = $cannaTag.object
    for ($cannaDepth = 0; $cannaDepth -lt 8; $cannaDepth++) {
        if ($cannaObject.type -eq 'commit') {
            if ($cannaObject.sha -ne $Expected) { throw 'Release tag does not resolve to the exact publication commit' }
            return
        }
        if ($cannaObject.type -ne 'tag' -or $cannaObject.sha -notmatch '^[0-9a-f]{40}$') { throw 'Unexpected release tag object' }
        $cannaObject = (Invoke-CannaApi "git/tags/$($cannaObject.sha)").object
    }
    throw 'Release tag exceeds annotated-tag resolution limit'
}
function Get-CannaHash([byte[]]$Bytes, [string]$Algorithm = 'SHA256') {
    $cannaHasher = [Security.Cryptography.HashAlgorithm]::Create($Algorithm)
    try { ([BitConverter]::ToString($cannaHasher.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $cannaHasher.Dispose() }
}
function Get-CannaBlobHash([byte[]]$Bytes) {
    $cannaPrefix = [Text.Encoding]::UTF8.GetBytes(('blob ' + $Bytes.Length + [char]0))
    $cannaGitBytes = [byte[]]::new($cannaPrefix.Length + $Bytes.Length)
    [Array]::Copy($cannaPrefix, 0, $cannaGitBytes, 0, $cannaPrefix.Length)
    [Array]::Copy($Bytes, 0, $cannaGitBytes, $cannaPrefix.Length, $Bytes.Length)
    Get-CannaHash $cannaGitBytes 'SHA1'
}
function Resolve-CannaFile([string]$Relative) {
    if ($Relative -notmatch '^[a-zA-Z0-9_.~/ -]+$' -or $Relative -match '(^|/)\.\.?(/|$)' -or $Relative.StartsWith('/')) { throw 'Unsafe publication source path' }
    $cannaPath = [IO.Path]::GetFullPath((Join-Path $cannaRoot $Relative))
    if (!$cannaPath.StartsWith($cannaRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Publication source escapes workspace' }
    $cannaWalk = $cannaPath
    while ($cannaWalk -ne $cannaRoot) {
        $cannaInfo = Get-Item -LiteralPath $cannaWalk -Force
        if ($cannaInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Publication source contains a link' }
        $cannaWalk = Split-Path $cannaWalk -Parent
    }
    if (!(Test-Path -LiteralPath $cannaPath -PathType Leaf)) { throw 'Publication source file is missing' }
    $cannaPath
}
function Get-CannaComponentFiles {
    $cannaComponent = Join-Path $cannaRoot 'mods/DuctTapePlusPlus'
    $cannaPending = [Collections.Generic.Stack[string]]::new()
    $cannaPending.Push($cannaComponent)
    $cannaAllowedExtensions = @('.cs', '.csproj', '.py', '.ps1', '.md', '.json', '.tsv')
    $cannaExcludedDirectories = @('bin', 'obj', 'build', 'cache', '__pycache__', '.git', '.vs', '.idea', '.vscode', 'target', 'dist', 'downloads', 'nuget', 'node_modules')
    while ($cannaPending.Count) {
        $cannaDirectory = $cannaPending.Pop()
        if ((Get-Item -LiteralPath $cannaDirectory -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Component source directory contains a link' }
        foreach ($cannaEntry in Get-ChildItem -LiteralPath $cannaDirectory -Force) {
            if ($cannaEntry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Component source contains a link' }
            if ($cannaEntry.PSIsContainer) {
                if ($cannaExcludedDirectories -notcontains $cannaEntry.Name) { $cannaPending.Push($cannaEntry.FullName) }
                continue
            }
            $cannaRelative = $cannaEntry.FullName.Substring($cannaRoot.Length + 1).Replace('\', '/')
            if ($cannaEntry.Name -match '(?i)(credential|secret|session|ticket|^token|^steam_appid)') { throw 'Unexpected private file name in component source' }
            $cannaNotice = $cannaEntry.Extension -eq '.txt' -and $cannaRelative -match '^mods/DuctTapePlusPlus/licenses/[A-Za-z0-9_.-]+\.txt$'
            if ($cannaAllowedExtensions -contains $cannaEntry.Extension -or $cannaNotice) { $cannaRelative }
        }
    }
}
function Assert-CannaSnapshot {
    foreach ($cannaRecord in $cannaSourceRecords) {
        if ((Get-FileHash -LiteralPath (Resolve-CannaFile $cannaRecord.path) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $cannaRecord.sha256) { throw 'Publication source changed after preflight; rerun with current source' }
    }
    foreach ($cannaArtifact in $cannaArtifacts) {
        if ((Get-FileHash -LiteralPath $cannaArtifact.path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $cannaArtifact.sha256) { throw 'Release artifact changed after preflight; rerun with verified artifacts' }
    }
}
try {
    if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid release version' }
    $cannaPackage = [IO.File]::ReadAllText((Resolve-CannaFile 'Cargo.toml'), $cannaUtf8)
    $cannaPackageMatch = [regex]::Match($cannaPackage, '(?ms)^\[package\]\s*\r?\n(?<body>.*?)(?=^\[|\z)')
    if (!$cannaPackageMatch.Success -or $cannaPackageMatch.Groups['body'].Value -notmatch ('(?m)^version\s*=\s*"' + [regex]::Escape($Version) + '"\s*$')) { throw 'Cargo package version does not match release' }
    # Native integration only. Do not publish scanners, provider registries, unrelated mods or catalogs.
    $cannaFiles = @('Cargo.toml', 'Cargo.lock', 'build.rs',
        'src/main.rs', 'src/model.rs', 'src/runtime.rs', 'src/ducttape.rs', 'src/play_backup.rs', 'src/pack_ui.rs', 'src/play_lab.rs',
        'src/account.rs', 'src/provider_browser.rs', 'src/rebound_support.rs',
        'README.md', 'server/web/help.html',
        'scripts/Build-DuctTapePlusPlus.ps1', 'scripts/Build-DuctTapePreview.ps1', 'scripts/Build-ReboundRelease.ps1',
        'scripts/Test-DuctTapePlusPlus.ps1', 'scripts/Publish-Rebound.ps1')
    $cannaFiles += @(Get-CannaComponentFiles)
    # These exact reviewed upstream resources are needed to reproduce the helper.
    # No wildcard DLL/binary allowance: every resource is additionally bound to upstream.lock.json.
    $cannaResourceHashes = @{
        'vendor/curated/Bknibb-UnboundLib-4.2.5~UnboundLib.dll.bsdf' = 'e9a7d3c2310750bb82c0785281ccfe184437e3bc1c00990655e1b3e4f30a8cc5'
        'vendor/curated/Bknibb-UnboundLib-4.2.7~UnboundLib.dll.bsdf' = '174844c2993faa17457be358abb264062237b6eacdf550ecdd5bf3e3bdf212cc'
        'vendor/curated/BossSloth-CardBarPatch-2.1.1~CardBarPatch.dll.bsdf' = '29e1e6ed2fa8f1ab8489b5a3dbb3a7590fd4b90bf552e519d5c2e82a434a295d'
        'vendor/curated/olavim-MapsExtended-1.4.2~plugins~MapsExtended.dll.bsdf' = '5ccba5a4cdcd5bf794a7a53b63612ae545cdde9b53b56649924a5ca10b99526a'
        'vendor/curated/olavim-RoundsWithFriends-2.2.2~plugins~RoundsWithFriends.dll.bsdf' = '7bae4619f5547d062a4fd2122a93e74617c65681c2f22c5eb1f51c4b7ff27c7a'
        'vendor/curated/Pykess-GunUnblockablePatch-0.0.0~GunUnblockablePatch.dll.bsdf' = '71bd491b6e15e85b906a7118a26120523c3d3d269b4f1fdbb09f3286583bd29d'
        'vendor/curated/Pykess-ModdingUtils-0.4.8~ModdingUtils.dll.bsdf' = '84cf7f2abdd73c748072f9a93cd1a7bd4451e5f8ef72b42c88dd5593e6918280'
        'vendor/curated/Pykess-TemporaryStatsPatch-0.0.2~TemporaryStatsPatch.dll.bsdf' = '1053e47226eadc540073d087cbc5d46fc3740a1a3e1a97651fc7c58586f0ee04'
        'vendor/curated/Root-Classes_Manager_Reborn-1.5.5~ClassesManagerReborn.dll.bsdf' = 'edecbf9203f5ba2aca2953df729b99c879576110c3bc8dd3f1c507cbd98fa5d0'
        'vendor/curated/Root-RarityLib-1.3.0~RarityLib.dll.bsdf' = '595b7d8812c5cfb73b8ca830e029f3adc6cf305aa5c56973d86549154d236be3'
        'vendor/curated/RoundsModding-Grow_Patch-0.0.0~GrowPatch.dll.bsdf' = '10c7420bd4a87edb53899cad29d7d14c9dc87cdb778c9fa8ee9c09246dd0967d'
        'vendor/curated/RoundsModding-Performance_Improvements-0.2.0~plugins~PerformanceImprovements.dll.bsdf' = 'bf67c26693e0cc6211734447e158bf8124aec9122d87d94d69bf03366c85ffb0'
        'vendor/curated/willis81808-MMHook-1.0.0~plugins~MMHOOK_Assembly-CSharp.dll.bsdf' = '1f4c858756c1370ce1618d82d535b7c8363591394ebe739c3e832dc463b79192'
        'vendor/curated/willis81808-ModsPlus-1.6.2~plugins~ModsPlus.dll.bsdf' = 'b41ef9296eeba896e44c5b065c48d9aa138a549a3d9b31893f1c708234c02e2b'
        'vendor/curated/willis81808-UnboundLib-3.2.14~plugins~UnboundLib.dll.bsdf' = '50e2c3a8d29c9c07be4551e67a728c172eb1acd4503eeb8f18aba5951463b730'
        'vendor/curated/willuwontu-GunChargePatch-0.0.4~GunChargePatch.dll.bsdf' = 'ef807f97d86dd9744a1b3166799b1b246d8ee1975f7b81dff8eb2fc971895d3f'
        'vendor/curated/willuwontu-WillsWackyMapObjects-1.2.4~WillsWackyMapObjects.dll.bsdf' = '2b0e5da17564aec75b479a4c1adb9778737c7d881bd3d4412909a62c95ab05f4'
        'vendor/curated/XAngelMoonX-CR-2.7.0~CosmicRounds.dll.bsdf' = 'd9b210781d901430aa5efd5f3501c32bac80828e492501246fa98fa099fd4fd8'
        'vendor/toolkit/compathelpers.dll' = '5f111a7e56f2d8cf751b49e6721d789d5e49fbe4f2756ccf2758fb947709dbcb'
        'vendor/toolkit/old-game-types.txt' = '7fe9598723d4c686e0a01dc7dd4fcf3be08730a15b1a9d87baecd8a42588f173'
    }
    $cannaLock = [IO.File]::ReadAllText((Resolve-CannaFile 'mods/DuctTapePlusPlus/upstream.lock.json'), $cannaUtf8) | ConvertFrom-Json
    foreach ($cannaResource in $cannaResourceHashes.Keys) {
        $cannaPinned = @($cannaLock.files | Where-Object file -eq $cannaResource)
        if ($cannaPinned.Count -ne 1 -or $cannaPinned[0].vendored_sha256 -ne $cannaResourceHashes[$cannaResource]) { throw 'Reviewed build resource lock differs from publisher pins' }
        $cannaResourcePath = 'mods/DuctTapePlusPlus/' + $cannaResource
        if ((Get-FileHash -LiteralPath (Resolve-CannaFile $cannaResourcePath) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $cannaResourceHashes[$cannaResource]) { throw 'Reviewed build resource checksum differs from publisher pins' }
        $cannaFiles += $cannaResourcePath
    }
    $cannaFiles = @($cannaFiles | Sort-Object -Unique)
    if ($cannaFiles.Count -gt 200) { throw 'Publication source count exceeds bounded allowlist' }
    $cannaSnapshots = @{}
    $cannaSourceRecords = @()
    $cannaSourceSize = 0L
    foreach ($cannaFile in $cannaFiles) {
        $cannaBytes = [IO.File]::ReadAllBytes((Resolve-CannaFile $cannaFile))
        $cannaSourceSize += $cannaBytes.Length
        if ($cannaBytes.Length -gt 8MB -or $cannaSourceSize -gt 32MB) { throw 'Publication source size exceeds limits' }
        $cannaSnapshots[$cannaFile] = $cannaBytes
        $cannaSourceRecords += @{ path = $cannaFile; size = $cannaBytes.Length; sha256 = (Get-CannaHash $cannaBytes); git_blob = (Get-CannaBlobHash $cannaBytes) }
    }
    $cannaArtifacts = @()
    $cannaReleaseNotes = $null
    $cannaNotesHash = $null
    if (!$SourceOnly) {
        $cannaNotesPath = if ([IO.Path]::IsPathRooted($ReleaseNotesFile)) { [IO.Path]::GetFullPath($ReleaseNotesFile) } else { [IO.Path]::GetFullPath((Join-Path $cannaRoot $ReleaseNotesFile)) }
        $cannaNotesBytes = [IO.File]::ReadAllBytes($cannaNotesPath)
        if (!$cannaNotesBytes.Length -or $cannaNotesBytes.Length -gt 100KB) { throw 'Release notes are empty or exceed limits' }
        $cannaReleaseNotes = $cannaUtf8.GetString($cannaNotesBytes).TrimStart([char]0xFEFF)
        if ([string]::IsNullOrWhiteSpace($cannaReleaseNotes)) { throw 'Release notes are empty' }
        $cannaNotesHash = Get-CannaHash $cannaNotesBytes
        foreach ($cannaArtifactFile in @('dist/Canna Mod Manager.exe', 'dist/Canna-Mod-Manager-Windows.zip')) {
            $cannaArtifactPath = Resolve-CannaFile $cannaArtifactFile
            $cannaArtifacts += @{ path = $cannaArtifactPath; file = $cannaArtifactFile; size = (Get-Item -LiteralPath $cannaArtifactPath).Length; sha256 = (Get-FileHash -LiteralPath $cannaArtifactPath -Algorithm SHA256).Hash.ToLowerInvariant() }
        }
        if ([Diagnostics.FileVersionInfo]::GetVersionInfo($cannaArtifacts[0].path).ProductVersion -ne $Version) { throw 'Release EXE version does not match requested version' }
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $cannaZip = [IO.Compression.ZipFile]::OpenRead($cannaArtifacts[1].path)
        try {
            $cannaExecutables = @($cannaZip.Entries | Where-Object FullName -eq 'Canna Mod Manager.exe')
            if ($cannaExecutables.Count -ne 1) { throw 'Portable ZIP must contain one launcher EXE' }
            $cannaZipStream = $cannaExecutables[0].Open()
            $cannaZipHasher = [Security.Cryptography.SHA256]::Create()
            try { $cannaZipHash = ([BitConverter]::ToString($cannaZipHasher.ComputeHash($cannaZipStream))).Replace('-', '').ToLowerInvariant() }
            finally { $cannaZipStream.Dispose(); $cannaZipHasher.Dispose() }
            if ($cannaZipHash -ne $cannaArtifacts[0].sha256) { throw 'Portable ZIP launcher differs from release EXE' }
        } finally { $cannaZip.Dispose() }
    }
    $cannaResultPath = Join-Path $cannaRoot "target/rebound-publication-$Version.json"
    $null = New-Item -ItemType Directory -Path (Split-Path $cannaResultPath -Parent) -Force
    $cannaResult = @{ version = $Version; source_only = [bool]$SourceOnly; check_only = [bool]$CheckOnly; source_count = $cannaSourceRecords.Count; source_bytes = $cannaSourceSize; sources = $cannaSourceRecords; artifacts = $cannaArtifacts; release_notes_sha256 = $cannaNotesHash }
    [IO.File]::WriteAllText($cannaResultPath, ($cannaResult | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Assert-CannaSnapshot
    if ($CheckOnly) { "Local publication preflight passed: $($cannaSourceRecords.Count) bounded source files. Manifest: $cannaResultPath"; exit 0 }
    # Credentials are read only after the entire local preflight; -CheckOnly never accesses them or the network.
    $cannaToken = [IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
    if ([string]::IsNullOrWhiteSpace($cannaToken)) { throw 'Publisher token is empty' }
    $cannaHeaders = @{ Authorization = "Bearer $cannaToken"; 'User-Agent' = 'Canna-Rebound-Publisher'; Accept = 'application/vnd.github+json' }
    $cannaPhase = 'source publication'
    $cannaRepo = Invoke-CannaApi ''
    $cannaBranch = $cannaRepo.default_branch
    if ($cannaBranch -notmatch '^[A-Za-z0-9_. /-]+$' -or $cannaBranch -match '(^|/)\.\.?(/|$)') { throw 'Unexpected default branch' }
    if (!$SourceOnly) { Assert-CannaVersionUnused }
    $cannaRef = Invoke-CannaApi "git/ref/heads/$cannaBranch"
    $cannaCommit = Invoke-CannaApi "git/commits/$($cannaRef.object.sha)"
    $cannaExistingTree = Invoke-CannaApi "git/trees/$($cannaCommit.tree.sha)?recursive=1"
    if ($cannaExistingTree.truncated) { throw 'Remote source tree is truncated' }
    $cannaExistingBlobs = @{}
    foreach ($cannaEntry in $cannaExistingTree.tree) { if ($cannaEntry.type -eq 'blob') { $cannaExistingBlobs[$cannaEntry.path] = $cannaEntry.sha } }
    $cannaEntries = @()
    foreach ($cannaRecord in $cannaSourceRecords) {
        $cannaFile = $cannaRecord.path
        if ($cannaExistingBlobs[$cannaFile] -eq $cannaRecord.git_blob) { $cannaBlob = @{ sha = $cannaRecord.git_blob } }
        else { $cannaBlob = Invoke-CannaApi 'git/blobs' 'POST' @{ content = [Convert]::ToBase64String($cannaSnapshots[$cannaFile]); encoding = 'base64' } }
        if ($cannaBlob.sha -ne $cannaRecord.git_blob) { throw 'GitHub source blob hash differs from local snapshot' }
        $cannaEntries += @{ path = $cannaFile; mode = '100644'; type = 'blob'; sha = $cannaBlob.sha }
    }
    $cannaTree = Invoke-CannaApi 'git/trees' 'POST' @{ base_tree = $cannaCommit.tree.sha; tree = $cannaEntries }
    if (!$CommitMessage) { $CommitMessage = "Canna ${Version}: Canna Rebound integration" }
    $cannaNewCommit = Invoke-CannaApi 'git/commits' 'POST' @{ message = $CommitMessage; tree = $cannaTree.sha; parents = @($cannaRef.object.sha) }
    Assert-CannaSnapshot
    $cannaLatest = Invoke-CannaApi "git/ref/heads/$cannaBranch"
    if ($cannaLatest.object.sha -ne $cannaRef.object.sha) { throw 'Source branch advanced during publication; retry against new head' }
    $null = Invoke-CannaApi "git/refs/heads/$cannaBranch" 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    $cannaVerified = Invoke-CannaApi "git/ref/heads/$cannaBranch"
    if ($cannaVerified.object.sha -ne $cannaNewCommit.sha) { throw 'Source ref verification failed' }
    $cannaResult.source_commit = $cannaNewCommit.sha
    [IO.File]::WriteAllText($cannaResultPath, ($cannaResult | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    if ($SourceOnly) { "Published Rebound source commit $($cannaNewCommit.sha) ($($cannaSourceRecords.Count) bounded files)."; exit 0 }
    $cannaPhase = 'draft release assets'
    # Recheck after source work, then create a lightweight tag atomically. The
    # GitHub release API ignores target_commitish when a tag already exists.
    Assert-CannaVersionUnused
    $null = Invoke-CannaApi 'git/refs' 'POST' @{ ref = "refs/tags/v$Version"; sha = $cannaNewCommit.sha }
    Assert-CannaTagCommit $cannaNewCommit.sha
    Assert-CannaReleaseUnused
    $cannaRelease = Invoke-CannaApi 'releases' 'POST' @{ tag_name = "v$Version"; target_commitish = $cannaNewCommit.sha; name = "Canna Mod Manager $Version"; draft = $true; prerelease = $false; body = $cannaReleaseNotes }
    if (!$cannaRelease.draft -or $cannaRelease.tag_name -ne "v$Version") { throw 'Unexpected draft release metadata' }
    $cannaResult.release_id = $cannaRelease.id
    $cannaResult.release_tag_commit = $cannaNewCommit.sha
    [IO.File]::WriteAllText($cannaResultPath, ($cannaResult | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    foreach ($cannaUpload in @(
        @{ index = 0; name = 'Canna-Mod-Manager.exe'; mime = 'application/octet-stream' },
        @{ index = 1; name = 'Canna-Mod-Manager-Windows.zip'; mime = 'application/zip' }
    )) {
        Assert-CannaSnapshot
        $cannaArtifact = $cannaArtifacts[$cannaUpload.index]
        $cannaUploadUri = "https://uploads.github.com/repos/fortnitegamerboy2020/canna-mod-manager/releases/$($cannaRelease.id)/assets?name=$($cannaUpload.name)"
        $cannaAsset = Invoke-RestMethod -Uri $cannaUploadUri -Method POST -Headers $cannaHeaders -InFile $cannaArtifact.path -ContentType $cannaUpload.mime -TimeoutSec 300
        if ($cannaAsset.digest -ne "sha256:$($cannaArtifact.sha256)" -or $cannaAsset.size -ne $cannaArtifact.size) { throw 'GitHub asset digest or size mismatch' }
    }
    Assert-CannaSnapshot
    Assert-CannaTagCommit $cannaNewCommit.sha
    $cannaPhase = 'server release mirror'
    $cannaExe = $cannaArtifacts[0].path
    $cannaDigest = $cannaArtifacts[0].sha256
    $cannaManifest = Join-Path $cannaRoot 'dist/latest.json'
    @{ tag_name = "v$Version"; draft = $false; prerelease = $false; assets = @(@{ name = 'Canna-Mod-Manager.exe'; size = $cannaArtifacts[0].size; digest = "sha256:$cannaDigest" }) } | ConvertTo-Json -Depth 5 | ForEach-Object { [IO.File]::WriteAllText($cannaManifest, $_, [Text.UTF8Encoding]::new($false)) }
    $cannaManifestDigest = (Get-FileHash -LiteralPath $cannaManifest -Algorithm SHA256).Hash.ToLowerInvariant()
    $cannaSshKey = Join-Path $env:USERPROFILE '.ssh/canna_server_ed25519'
    & scp -q -i $cannaSshKey $cannaExe "canna-admin@165.227.83.76:/home/canna-admin/canna-v$Version.exe"
    if ($LASTEXITCODE -ne 0) { throw 'Server release upload failed' }
    & scp -q -i $cannaSshKey $cannaManifest "canna-admin@165.227.83.76:/home/canna-admin/canna-latest-$Version.json"
    if ($LASTEXITCODE -ne 0) { throw 'Server manifest upload failed' }
    $cannaRemote = @'
set -eu
release_version=$1
exe_sha=$2
manifest_sha=$3
exe_input=/home/canna-admin/canna-v${release_version}.exe
manifest_input=/home/canna-admin/canna-latest-${release_version}.json
exe_target=/opt/canna/releases/v${release_version}.exe
manifest_stage=/opt/canna/releases/latest-${release_version}.json.tmp
printf '%s  %s\n' "$exe_sha" "$exe_input" | sha256sum -c - >/dev/null
printf '%s  %s\n' "$manifest_sha" "$manifest_input" | sha256sum -c - >/dev/null
assert_latest_version() {
python3 - "$release_version" "$exe_sha" "$manifest_input" <<'PY'
import json, pathlib, re, sys
def version(value):
    if not isinstance(value, str) or not re.fullmatch(r'v?\d+\.\d+\.\d+', value):
        raise SystemExit('Release version metadata is invalid')
    return tuple(map(int, value.lstrip('v').split('.')))
incoming_version = version(sys.argv[1])
incoming_path = pathlib.Path(sys.argv[3])
if incoming_path.stat().st_size > 65536:
    raise SystemExit('Incoming release metadata is oversized')
incoming = json.loads(incoming_path.read_text(encoding='utf-8-sig'))
if (incoming.get('tag_name') != 'v' + sys.argv[1] or incoming.get('draft') is not False
        or incoming.get('prerelease') is not False or len(incoming.get('assets', [])) != 1
        or incoming['assets'][0].get('name') != 'Canna-Mod-Manager.exe'
        or incoming['assets'][0].get('digest') != 'sha256:' + sys.argv[2]):
    raise SystemExit('Incoming release metadata does not match the verified launcher')
current_path = pathlib.Path('/opt/canna/releases/latest.json')
if current_path.exists():
    if current_path.stat().st_size > 65536:
        raise SystemExit('Current release metadata is oversized')
    current = json.loads(current_path.read_text(encoding='utf-8-sig'))
    if version(current.get('tag_name')) >= incoming_version:
        raise SystemExit('Refusing to replace the same or a newer updater release')
PY
}
assert_latest_version
mkdir -p /opt/canna/releases
if test -e "$exe_target"; then
    printf '%s  %s\n' "$exe_sha" "$exe_target" | sha256sum -c - >/dev/null
else
    install -m 644 "$exe_input" "${exe_target}.tmp"
    mv "${exe_target}.tmp" "$exe_target"
fi
install -m 644 "$manifest_input" "$manifest_stage"
assert_latest_version
printf '%s  %s\n' "$exe_sha" "$exe_target" | sha256sum -c - >/dev/null
printf '%s  %s\n' "$manifest_sha" "$manifest_stage" | sha256sum -c - >/dev/null
mv "$manifest_stage" /opt/canna/releases/latest.json
rm "$exe_input" "$manifest_input"
'@
    # Share the review release lock. Both monotonic checks and activation execute
    # in one lock scope; staging and manifest names are specific to this version.
    # Base64 carries LF-only script bytes without Windows pipeline CRLF changes.
    $cannaRemoteBytes = [Text.Encoding]::UTF8.GetBytes($cannaRemote.Replace(([char]13).ToString(), ''))
    $cannaRemoteEncoded = [Convert]::ToBase64String($cannaRemoteBytes)
    & ssh -i $cannaSshKey -o BatchMode=yes canna-admin@165.227.83.76 "printf '%s' '$cannaRemoteEncoded' | base64 -d | sudo flock -w 30 /run/lock/canna-review-release.lock sh -s -- '$Version' '$cannaDigest' '$cannaManifestDigest'"
    if ($LASTEXITCODE -ne 0) { throw 'Server release installation failed' }
    $cannaPhase = 'release publication'
    Assert-CannaSnapshot
    Assert-CannaTagCommit $cannaNewCommit.sha
    $cannaPublished = Invoke-CannaApi "releases/$($cannaRelease.id)" 'PATCH' @{ draft = $false }
    if ($cannaPublished.id -ne $cannaRelease.id -or $cannaPublished.tag_name -ne "v$Version" -or $cannaPublished.draft -ne $false -or $cannaPublished.prerelease -ne $false) { throw 'Published release metadata verification failed' }
    Assert-CannaTagCommit $cannaNewCommit.sha
    $cannaResult.published = $true
    [IO.File]::WriteAllText($cannaResultPath, ($cannaResult | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    "Published Windows release v$Version and Rebound source commit $($cannaNewCommit.sha)."
} catch {
    "Rebound publication failed during $cannaPhase; any draft is retained. Credentials and response bodies are omitted."
    if ($_.Exception.Response) { 'HTTP status: ' + [int]$_.Exception.Response.StatusCode }
    elseif ($cannaPhase -eq 'local preflight') { 'Preflight reason: ' + $_.Exception.Message }
    exit 1
} finally { $cannaToken = $null; $cannaHeaders = $null; $cannaSnapshots = $null }
