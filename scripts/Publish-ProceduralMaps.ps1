param([string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_mods.txt')
$ErrorActionPreference = 'Stop'
$cannaRoot = Split-Path $PSScriptRoot -Parent
$cannaToken = [IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
$cannaHeaders = @{ Authorization = "Bearer $cannaToken"; 'User-Agent' = 'Canna-Mod-Manager'; Accept = 'application/vnd.github+json' }
$cannaApi = 'https://api.github.com/repos/fortnitegamerboy2020/manager-uploaded-mods'
function Invoke-CannaApi([string]$Path, [string]$Method = 'GET', $Body = $null) {
    $cannaRequest = @{ Uri = "$cannaApi/$Path"; Headers = $cannaHeaders; Method = $Method; TimeoutSec = 60 }
    if ($null -ne $Body) { $cannaRequest.Body = ($Body | ConvertTo-Json -Depth 30 -Compress); $cannaRequest.ContentType = 'application/json' }
    Invoke-RestMethod @cannaRequest
}
try {
    $cannaRef = Invoke-CannaApi 'git/ref/heads/main'
    $cannaCommit = Invoke-CannaApi "git/commits/$($cannaRef.object.sha)"
    $cannaGameFile = Invoke-CannaApi 'contents/bopl-battle/game.json?ref=main'
    $cannaGame = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($cannaGameFile.content)) | ConvertFrom-Json
    $cannaModFile = Join-Path $cannaRoot 'mods\ProceduralMaps\build\Canna-ProceduralMaps-1.0.1.zip'
    $cannaHash = (Get-FileHash -LiteralPath $cannaModFile -Algorithm SHA256).Hash.ToLowerInvariant()
    $cannaMod = @{ name = 'Canna Procedural Maps'; version = '1.0.1'; description = 'Fresh generated islands, moving platforms and occasional moon gravity. Shared online round seed and matching-generator lobby checks. All players need this version enabled. Family online gameplay verification is pending.'; file = 'Mods/Canna-ProceduralMaps-1.0.1.zip'; sha256 = $cannaHash }
    $cannaGame.mods = @($cannaGame.mods | Where-Object { $_.name -ne $cannaMod.name }) + @($cannaMod)
    $cannaUploads = @(
        @{ path = 'bopl-battle/Mods/Canna-ProceduralMaps-1.0.1.zip'; bytes = [IO.File]::ReadAllBytes($cannaModFile) },
        @{ path = 'bopl-battle/Mods/ProceduralMaps-README.md'; bytes = [IO.File]::ReadAllBytes((Join-Path $cannaRoot 'mods\ProceduralMaps\README.md')) },
        @{ path = 'bopl-battle/game.json'; bytes = [Text.Encoding]::UTF8.GetBytes(($cannaGame | ConvertTo-Json -Depth 20)) }
    )
    $cannaEntries = @()
    foreach ($cannaUpload in $cannaUploads) {
        $cannaBlob = Invoke-CannaApi 'git/blobs' 'POST' @{ content = [Convert]::ToBase64String($cannaUpload.bytes); encoding = 'base64' }
        $cannaEntries += @{ path = $cannaUpload.path; mode = '100644'; type = 'blob'; sha = $cannaBlob.sha }
    }
    $cannaTree = Invoke-CannaApi 'git/trees' 'POST' @{ base_tree = $cannaCommit.tree.sha; tree = $cannaEntries }
    $cannaNewCommit = Invoke-CannaApi 'git/commits' 'POST' @{ message = 'Add Canna Procedural Maps 1.0.1 with shared seeds and moving islands'; tree = $cannaTree.sha; parents = @($cannaRef.object.sha) }
    $null = Invoke-CannaApi 'git/refs/heads/main' 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    "Published procedural mod and catalog: $($cannaNewCommit.sha)"
} catch {
    'Procedural mod publication failed.'
    if ($_.Exception.Response) { 'HTTP status: ' + [int]$_.Exception.Response.StatusCode }
    exit 1
} finally { $cannaToken = $null; $cannaHeaders = $null }
