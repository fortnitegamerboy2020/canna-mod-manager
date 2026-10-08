param(
    [string]$SourceRoot = (Split-Path $PSScriptRoot -Parent),
    [ValidateSet('0.3.58', '0.3.59', '0.3.60')][string]$Version = '0.3.60',
    [switch]$CheckOnly,
    [string]$ReceiptFile = '',
    [string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_canna_mod_manager.txt'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
$cannaCommunityRoot = [IO.Path]::GetFullPath($SourceRoot).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
$cannaCommunityPublisher = [IO.Path]::GetFullPath($PSCommandPath)
$cannaCommunityToken = $null
$cannaCommunityHeaders = $null
$cannaCommunityPhase = 'local preflight'
$cannaCommunityReceipt = $null
$cannaCommunityApi = 'https://api.github.com/repos/fortnitegamerboy2020/canna-mod-manager'
$cannaCommunityUtf8 = [Text.UTF8Encoding]::new($false, $true)

function Get-CannaCommunityHash([byte[]]$Bytes, [string]$Algorithm = 'SHA256') {
    $cannaCommunityHasher = [Security.Cryptography.HashAlgorithm]::Create($Algorithm)
    try { ([BitConverter]::ToString($cannaCommunityHasher.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $cannaCommunityHasher.Dispose() }
}
function Get-CannaCommunityBlobHash([byte[]]$Bytes) {
    $cannaCommunityPrefix = [Text.Encoding]::UTF8.GetBytes(('blob ' + $Bytes.Length + [char]0))
    $cannaCommunityGitBytes = [byte[]]::new($cannaCommunityPrefix.Length + $Bytes.Length)
    [Array]::Copy($cannaCommunityPrefix, 0, $cannaCommunityGitBytes, 0, $cannaCommunityPrefix.Length)
    [Array]::Copy($Bytes, 0, $cannaCommunityGitBytes, $cannaCommunityPrefix.Length, $Bytes.Length)
    Get-CannaCommunityHash $cannaCommunityGitBytes 'SHA1'
}
function Resolve-CannaCommunityFile([string]$Relative) {
    if ($Relative -notmatch '^[A-Za-z0-9_. /-]+$' -or $Relative -match '(^|/)\.?\.(/|$)' -or $Relative.StartsWith('/')) { throw 'Unsafe source path' }
    # This one fixed overlay publishes the executing helper without modifying a
    # frozen server build stage. Every other file must stay within SourceRoot.
    $cannaCommunityFileRoot = $cannaCommunityRoot
    $cannaCommunityPath = [IO.Path]::GetFullPath((Join-Path $cannaCommunityRoot $Relative))
    if ($Relative -eq 'scripts/Publish-CommunityReviewUpdate.ps1') {
        $cannaCommunityPath = $cannaCommunityPublisher
        $cannaCommunityFileRoot = [IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
    }
    if (!$cannaCommunityPath.StartsWith($cannaCommunityFileRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Source escapes staged root' }
    $cannaCommunityWalk = $cannaCommunityPath
    while ($cannaCommunityWalk -ne $cannaCommunityFileRoot) {
        $cannaCommunityInfo = Get-Item -LiteralPath $cannaCommunityWalk -Force
        if ($cannaCommunityInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Source contains a link' }
        $cannaCommunityWalk = Split-Path $cannaCommunityWalk -Parent
    }
    if (!(Test-Path -LiteralPath $cannaCommunityPath -PathType Leaf)) { throw 'Source file missing' }
    $cannaCommunityPath
}
function Get-CannaCommunityFiles {
    # Directory scope is limited to Rust source. Runtime files, private fixtures,
    # credentials, target outputs and arbitrary deployment files are never globbed.
    $cannaCommunityFiles = @('server/Cargo.toml', 'server/Cargo.lock', 'server/README.md',
        'server/src/fixtures/rounds-dependencies.json', 'scripts/Publish-CommunityReviewUpdate.ps1')
    if ($Version -eq '0.3.60') { $cannaCommunityFiles += 'README.md' }
    $cannaCommunitySourceDirectory = Join-Path $cannaCommunityRoot 'server/src'
    $cannaCommunityPending = [Collections.Generic.Stack[string]]::new()
    $cannaCommunityPending.Push($cannaCommunitySourceDirectory)
    while ($cannaCommunityPending.Count) {
        $cannaCommunityDirectory = $cannaCommunityPending.Pop()
        if ((Get-Item -LiteralPath $cannaCommunityDirectory -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Source directory contains a link' }
        foreach ($cannaCommunityEntry in Get-ChildItem -LiteralPath $cannaCommunityDirectory -Force) {
            if ($cannaCommunityEntry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Source directory contains a link' }
            if ($cannaCommunityEntry.PSIsContainer) {
                if ($cannaCommunityEntry.Name -in @('target', 'cache', '__pycache__', '.git', 'node_modules', 'fixtures', 'private')) { continue }
                if ($cannaCommunityEntry.Name.StartsWith('.')) { throw 'Unexpected hidden source directory' }
                $cannaCommunityPending.Push($cannaCommunityEntry.FullName)
                continue
            }
            $cannaCommunityRelative = $cannaCommunityEntry.FullName.Substring($cannaCommunityRoot.Length + 1).Replace('\', '/')
            if ($cannaCommunityEntry.Extension -eq '.rs' -or $cannaCommunityRelative -eq 'server/src/rounds-dependencies.json') {
                if ($cannaCommunityEntry.Name -match '(?i)^(credentials?|private[-_.]|secrets?[-_.]|tokens?[-_.])') { throw 'Private source filename refused' }
                $cannaCommunityFiles += $cannaCommunityRelative
            }
        }
    }
    # Fixed web documents and source assets; no member-upload directories.
    $cannaCommunityWebFiles = @('admin-games.css', 'admin.js', 'app.js', 'brand-logo.png',
        'community.js', 'confirm.js', 'connect.html', 'connect.js', 'favicon.png',
        'filter-menus.js', 'forum.css', 'gambling.js', 'help.css', 'help.html',
        'index.html', 'library.js', 'live.js', 'login.html', 'login.js', 'lounge.js',
        'notifications.js', 'play-lab.js', 'profiles.js', 'provider-browser.js',
        'review-workspace.css', 'review.html', 'review.js', 'sections.js',
        'shared-packs.js', 'source-recommendations.json', 'support.html', 'support.js',
        'thunderstore-games-LICENSE.txt', 'thunderstore-games.json',
        'cosmetics/catalog.json', 'cosmetics/SOURCES.md')
    foreach ($cannaCommunityWebFile in $cannaCommunityWebFiles) { $cannaCommunityFiles += 'server/web/' + $cannaCommunityWebFile }
    # Only the exact reviewed catalog grants permission to include cosmetic bytes.
    $cannaCommunityCatalog = [IO.File]::ReadAllText((Resolve-CannaCommunityFile 'server/web/cosmetics/catalog.json'), $cannaCommunityUtf8) | ConvertFrom-Json
    if (@($cannaCommunityCatalog.items).Count -ne 315) { throw 'Reviewed cosmetic catalog count differs' }
    $cannaCommunityCosmeticNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($cannaCommunityCosmetic in $cannaCommunityCatalog.items) {
        if ($cannaCommunityCosmetic.filename -notmatch '^[a-z0-9][a-z0-9-]*\.(svg|png|jpg|webp)$' -or $cannaCommunityCosmetic.sha256 -notmatch '^[0-9a-f]{64}$') { throw 'Unsafe cosmetic catalog entry' }
        if (!$cannaCommunityCosmeticNames.Add($cannaCommunityCosmetic.filename)) { throw 'Duplicate cosmetic catalog filename' }
        $cannaCommunityCosmeticFile = 'server/web/cosmetics/' + $cannaCommunityCosmetic.filename
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath (Resolve-CannaCommunityFile $cannaCommunityCosmeticFile)).Hash.ToLowerInvariant() -ne $cannaCommunityCosmetic.sha256) { throw 'Cosmetic hash differs from reviewed catalog' }
        $cannaCommunityFiles += $cannaCommunityCosmeticFile
    }
    # Shipped worker, contextual rules, configuration and public regression tools.
    $cannaCommunityDeployFiles = @('backup.sh', 'bootstrap.sh', 'Caddyfile',
        'canna-backup.service', 'canna-backup.timer', 'canna-review.service',
        'canna.service', 'encrypted-upgrade.sh', 'harden-ssh.sh',
        'Open-Canna-AdminInvite.ps1', 'review_context.py', 'review-worker.py',
        'security-prepare.sh', 'test-review-adversarial.py', 'test-review-context.py',
        'test-review-coverage.py', 'test-review-decompilation.py', 'test-review-live.py',
        'test-review-offline.py', 'test-review-packing.py', 'test-review-permissions.py',
        'test-review-rules.py')
    if ($Version -in @('0.3.59', '0.3.60')) {
        $cannaCommunityDeployFiles += @('test-review-documentation.py', 'test-review-metadata.py',
            'test-fixtures/licenses/GPL-3.0.txt', 'test-fixtures/licenses/Apache-2.0.txt',
            'test-fixtures/licenses/MIT.txt', 'test-fixtures/licenses/SOURCES.md')
    }
    foreach ($cannaCommunityDeployFile in $cannaCommunityDeployFiles) { $cannaCommunityFiles += 'server/deploy/' + $cannaCommunityDeployFile }
    @($cannaCommunityFiles | Sort-Object -Unique)
}
function Assert-CannaCommunitySnapshot {
    $cannaCommunityCurrentFiles = @(Get-CannaCommunityFiles)
    if (($cannaCommunityCurrentFiles -join "`n") -cne ($cannaCommunityFiles -join "`n")) { throw 'Publication inventory changed after preflight' }
    foreach ($cannaCommunityRecord in $cannaCommunityRecords) {
        if ((Get-FileHash -Algorithm SHA256 -LiteralPath (Resolve-CannaCommunityFile $cannaCommunityRecord.path)).Hash.ToLowerInvariant() -ne $cannaCommunityRecord.sha256) { throw 'Publication bytes changed after preflight' }
    }
}
function Save-CannaCommunityReceipt {
    [IO.File]::WriteAllText($cannaCommunityReceiptPath, ($cannaCommunityReceipt | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
}
function Invoke-CannaCommunityApi([string]$Path, [string]$Method = 'GET', $Body = $null) {
    $cannaCommunityRequest = @{Uri = "$cannaCommunityApi/$Path".TrimEnd('/'); Headers = $cannaCommunityHeaders; Method = $Method; TimeoutSec = 90}
    if ($null -ne $Body) { $cannaCommunityRequest.Body = $Body | ConvertTo-Json -Depth 20 -Compress; $cannaCommunityRequest.ContentType = 'application/json' }
    Invoke-RestMethod @cannaCommunityRequest
}
try {
    if ((Get-Item -LiteralPath $cannaCommunityRoot -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Staged root contains a link' }
    $cannaCommunityFiles = @(Get-CannaCommunityFiles)
    if (!$cannaCommunityFiles.Count -or $cannaCommunityFiles.Count -gt 600) { throw 'Source count exceeds bounded allowlist' }
    $cannaCommunityPackage = [IO.File]::ReadAllText((Resolve-CannaCommunityFile 'server/Cargo.toml'), $cannaCommunityUtf8)
    $cannaCommunityPackageSection = [regex]::Match($cannaCommunityPackage, '(?ms)^\[package\]\s*\r?\n(?<body>.*?)(?=^\[|\z)')
    if (!$cannaCommunityPackageSection.Success -or $cannaCommunityPackageSection.Groups['body'].Value -notmatch ('(?m)^version\s*=\s*"' + [regex]::Escape($Version) + '"\s*$')) { throw 'Staged version differs' }
    if ((Get-FileHash -LiteralPath (Resolve-CannaCommunityFile 'scripts/Publish-CommunityReviewUpdate.ps1')).Hash -ne (Get-FileHash -LiteralPath $cannaCommunityPublisher).Hash) { throw 'Staged publisher differs from executing publisher' }
    $cannaCommunitySnapshots = @{}
    $cannaCommunityRecords = @()
    $cannaCommunityBytesTotal = 0L
    foreach ($cannaCommunityFile in $cannaCommunityFiles) {
        $cannaCommunityFilePath = Resolve-CannaCommunityFile $cannaCommunityFile
        if ((Get-Item -LiteralPath $cannaCommunityFilePath).Length -gt 8MB) { throw 'Source file exceeds 8MiB limit' }
        $cannaCommunityBytes = [IO.File]::ReadAllBytes($cannaCommunityFilePath)
        $cannaCommunityBytesTotal += $cannaCommunityBytes.Length
        if ($cannaCommunityBytesTotal -gt 32MB) { throw 'Source snapshot exceeds 32MiB limit' }
        if ([IO.Path]::GetExtension($cannaCommunityFile) -notin @('.png', '.jpg', '.webp')) {
            $cannaCommunityText = $cannaCommunityUtf8.GetString($cannaCommunityBytes)
            if ($cannaCommunityText -match '(?m)-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|(?:github_pat_[A-Za-z0-9_]{40,}|gh[pousr]_[A-Za-z0-9]{30,})') { throw 'Credential-shaped source content refused' }
        }
        $cannaCommunitySnapshots[$cannaCommunityFile] = $cannaCommunityBytes
        $cannaCommunityRecords += @{path = $cannaCommunityFile; size = $cannaCommunityBytes.Length; sha256 = (Get-CannaCommunityHash $cannaCommunityBytes); git_blob = (Get-CannaCommunityBlobHash $cannaCommunityBytes)}
    }
    $cannaCommunityReceiptPath = if ($ReceiptFile) { [IO.Path]::GetFullPath($ReceiptFile) } else { Join-Path (Split-Path $PSScriptRoot -Parent) "target/community-source-publication-$Version.json" }
    $cannaCommunityReceiptParent = Split-Path $cannaCommunityReceiptPath -Parent
    $null = New-Item -ItemType Directory -Path $cannaCommunityReceiptParent -Force
    if ($cannaCommunityFiles | Where-Object { (Resolve-CannaCommunityFile $_) -eq $cannaCommunityReceiptPath }) { throw 'Receipt cannot overwrite a publication source' }
    $cannaCommunityReceipt = @{version = $Version; check_only = [bool]$CheckOnly; status = 'checked'; source_count = $cannaCommunityRecords.Count; source_bytes = $cannaCommunityBytesTotal; sources = $cannaCommunityRecords; repository = 'fortnitegamerboy2020/canna-mod-manager'; branch = $null; parent = $null; commit = $null; changed_count = 0; uploaded_count = 0; ref_updated = $false}
    Assert-CannaCommunitySnapshot
    Save-CannaCommunityReceipt
    # CheckOnly exits before reading credentials or making any network request.
    if ($CheckOnly) { Write-Output "Offline source preflight passed: $($cannaCommunityRecords.Count) files, $cannaCommunityBytesTotal bytes. Receipt: $cannaCommunityReceiptPath"; exit 0 }
    $cannaCommunityPhase = 'repository preflight'
    $cannaCommunityToken = [IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
    if ([string]::IsNullOrWhiteSpace($cannaCommunityToken)) { throw 'Publication credential is empty' }
    $cannaCommunityHeaders = @{Authorization = "Bearer $cannaCommunityToken"; 'User-Agent' = 'Canna-Community-Source-Publisher'; Accept = 'application/vnd.github+json'}
    $cannaCommunityRepository = Invoke-CannaCommunityApi ''
    if ($cannaCommunityRepository.full_name -ne 'fortnitegamerboy2020/canna-mod-manager') { throw 'Unexpected source repository identity' }
    $cannaCommunityReceipt.repository_visibility = if ($cannaCommunityRepository.private) { 'private' } else { 'public' }
    $cannaCommunityBranch = $cannaCommunityRepository.default_branch
    if ($cannaCommunityBranch -notmatch '^[A-Za-z0-9_./-]+$' -or $cannaCommunityBranch -match '(^|/)\.\.?(/|$)') { throw 'Unsafe default branch' }
    $cannaCommunityBranchPath = [Uri]::EscapeDataString($cannaCommunityBranch)
    $cannaCommunityRef = Invoke-CannaCommunityApi "git/ref/heads/$cannaCommunityBranchPath"
    $cannaCommunityParent = Invoke-CannaCommunityApi "git/commits/$($cannaCommunityRef.object.sha)"
    $cannaCommunityRemoteTree = Invoke-CannaCommunityApi "git/trees/$($cannaCommunityParent.tree.sha)?recursive=1"
    if ($cannaCommunityRemoteTree.truncated -or @($cannaCommunityRemoteTree.tree).Count -gt 100000) { throw 'Remote source tree exceeds bounds' }
    $cannaCommunityExisting = @{}
    foreach ($cannaCommunityEntry in $cannaCommunityRemoteTree.tree) { if ($cannaCommunityEntry.type -eq 'blob') { $cannaCommunityExisting[$cannaCommunityEntry.path] = $cannaCommunityEntry } }
    $cannaCommunityReceipt.branch = $cannaCommunityBranch
    $cannaCommunityReceipt.parent = $cannaCommunityRef.object.sha
    $cannaCommunityEntries = @()
    $cannaCommunityPhase = 'snapshot and source blob upload'
    Assert-CannaCommunitySnapshot
    foreach ($cannaCommunityRecord in $cannaCommunityRecords) {
        if ($cannaCommunityExisting.ContainsKey($cannaCommunityRecord.path) -and $cannaCommunityExisting[$cannaCommunityRecord.path].sha -eq $cannaCommunityRecord.git_blob -and $cannaCommunityExisting[$cannaCommunityRecord.path].mode -in @('100644', '100755')) { continue }
        $cannaCommunityBlob = Invoke-CannaCommunityApi 'git/blobs' 'POST' @{content = [Convert]::ToBase64String($cannaCommunitySnapshots[$cannaCommunityRecord.path]); encoding = 'base64'}
        if ($cannaCommunityBlob.sha -ne $cannaCommunityRecord.git_blob) { throw 'Uploaded source blob checksum differs' }
        $cannaCommunityMode = '100644'
        if ($cannaCommunityExisting.ContainsKey($cannaCommunityRecord.path) -and $cannaCommunityExisting[$cannaCommunityRecord.path].mode -in @('100644', '100755')) { $cannaCommunityMode = $cannaCommunityExisting[$cannaCommunityRecord.path].mode }
        $cannaCommunityEntries += @{path = $cannaCommunityRecord.path; mode = $cannaCommunityMode; type = 'blob'; sha = $cannaCommunityBlob.sha}
        $cannaCommunityReceipt.uploaded_count++
        # Keep content mutations sequential and below a burst of new asset writes.
        Start-Sleep -Milliseconds 850
    }
    $cannaCommunityReceipt.changed_count = $cannaCommunityEntries.Count
    if (!$cannaCommunityEntries.Count) {
        Assert-CannaCommunitySnapshot
        $cannaCommunityReceipt.commit = $cannaCommunityRef.object.sha
        $cannaCommunityReceipt.status = 'already-matched'
        Save-CannaCommunityReceipt
        Write-Output "All bounded source blobs already match commit $($cannaCommunityRef.object.sha)."
        exit 0
    }
    $cannaCommunityTree = Invoke-CannaCommunityApi 'git/trees' 'POST' @{base_tree = $cannaCommunityParent.tree.sha; tree = $cannaCommunityEntries}
    $cannaCommunityCommit = Invoke-CannaCommunityApi 'git/commits' 'POST' @{message = "Community $Version`: admin, Kash games, Beta access, review workspace and account recovery"; tree = $cannaCommunityTree.sha; parents = @($cannaCommunityRef.object.sha)}
    $cannaCommunityReceipt.commit = $cannaCommunityCommit.sha
    Save-CannaCommunityReceipt
    $cannaCommunityPhase = 'source snapshot and branch compare-and-swap'
    Assert-CannaCommunitySnapshot
    $cannaCommunityLatest = Invoke-CannaCommunityApi "git/ref/heads/$cannaCommunityBranchPath"
    if ($cannaCommunityLatest.object.sha -ne $cannaCommunityRef.object.sha) { throw 'Source branch advanced; retry against its new head' }
    Assert-CannaCommunitySnapshot
    # force=false also refuses a concurrent non-fast-forward between GET and PATCH.
    $null = Invoke-CannaCommunityApi "git/refs/heads/$cannaCommunityBranchPath" 'PATCH' @{sha = $cannaCommunityCommit.sha; force = $false}
    $cannaCommunityReceipt.ref_updated = $true
    Save-CannaCommunityReceipt
    $cannaCommunityPhase = 'published source verification'
    $cannaCommunityVerifiedRef = Invoke-CannaCommunityApi "git/ref/heads/$cannaCommunityBranchPath"
    if ($cannaCommunityVerifiedRef.object.sha -ne $cannaCommunityCommit.sha) { throw 'Published source ref verification failed' }
    $cannaCommunityVerifiedTree = Invoke-CannaCommunityApi "git/trees/$($cannaCommunityCommit.tree.sha)?recursive=1"
    if ($cannaCommunityVerifiedTree.truncated) { throw 'Published source tree truncated' }
    $cannaCommunityVerifiedBlobs = @{}
    foreach ($cannaCommunityEntry in $cannaCommunityVerifiedTree.tree) { if ($cannaCommunityEntry.type -eq 'blob') { $cannaCommunityVerifiedBlobs[$cannaCommunityEntry.path] = $cannaCommunityEntry } }
    foreach ($cannaCommunityRecord in $cannaCommunityRecords) {
        if (!$cannaCommunityVerifiedBlobs.ContainsKey($cannaCommunityRecord.path) -or $cannaCommunityVerifiedBlobs[$cannaCommunityRecord.path].sha -ne $cannaCommunityRecord.git_blob -or $cannaCommunityVerifiedBlobs[$cannaCommunityRecord.path].mode -notin @('100644', '100755')) { throw 'Published source inventory checksum differs' }
    }
    $cannaCommunityReceipt.status = 'published'
    Save-CannaCommunityReceipt
    Write-Output "Published and verified Community $Version source commit $($cannaCommunityCommit.sha); $($cannaCommunityEntries.Count) changed blobs, $($cannaCommunityRecords.Count) bounded files."
} catch {
    if ($null -ne $cannaCommunityReceipt) { $cannaCommunityReceipt.status = 'failed'; $cannaCommunityReceipt.failure_phase = $cannaCommunityPhase; try { Save-CannaCommunityReceipt } catch {} }
    Write-Output "Community source publication failed during $cannaCommunityPhase; credentials, paths and response bodies are omitted."
    if ($_.Exception.PSObject.Properties['Response'] -and $_.Exception.Response) { Write-Output ('HTTP status: ' + [int]$_.Exception.Response.StatusCode) }
    exit 1
} finally { $cannaCommunityToken = $null; $cannaCommunityHeaders = $null }
