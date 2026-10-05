param([string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_mods.txt')
$ErrorActionPreference = 'Stop'
$cannaRoot = Split-Path $PSScriptRoot -Parent
$cannaToken = [System.IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
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
    $cannaGame = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($cannaGameFile.content)) | ConvertFrom-Json
    $cannaModFile = Join-Path $cannaRoot 'mods\DrillThroughBall\build\Canna-DrillThroughBall-1.0.5.zip'
    $cannaHash = (Get-FileHash -LiteralPath $cannaModFile -Algorithm SHA256).Hash.ToLowerInvariant()
    $cannaMod = @{ name = 'Drill Through Ball'; version = '1.0.5'; description = 'An actively spinning drill wins confirmed Bounce Ball contact; scaled attack hitboxes cut balls too. Steel Anvils are excluded and retain their native lethal collision. All players need the same mod. Gameplay testing is pending.'; file = 'Mods/Canna-DrillThroughBall-1.0.5.zip'; sha256 = $cannaHash }
    $cannaGame.mods = @($cannaGame.mods | Where-Object { $_.name -ne $cannaMod.name }) + @($cannaMod)
    $cannaUploads = @(
        @{ path = 'bopl-battle/Framework/LICENSE.txt'; bytes = [System.IO.File]::ReadAllBytes('C:\Users\t_tra\Downloads\bopl-battle\Framework\LICENSE.txt') },
        @{ path = 'bopl-battle/Framework/BepInEx.zip'; bytes = [System.IO.File]::ReadAllBytes('C:\Users\t_tra\Downloads\bopl-battle\Framework\BepInEx.zip') },
        @{ path = $cannaMod.file.Insert(0, 'bopl-battle/'); bytes = [System.IO.File]::ReadAllBytes($cannaModFile) },
        @{ path = 'bopl-battle/Mods/DrillThroughBall-README.md'; bytes = [System.IO.File]::ReadAllBytes((Join-Path $cannaRoot 'mods\DrillThroughBall\README.md')) },
        @{ path = 'bopl-battle/Framework/README.md'; bytes = [System.Text.Encoding]::UTF8.GetBytes("# BepInEx for Bopl Battle`n`nUnmodified official Windows x64 BepInEx 5.4.23.5, including upstream Harmony libraries.`nSource: https://github.com/BepInEx/BepInEx/releases/tag/v5.4.23.5`nLicense: https://github.com/BepInEx/BepInEx/blob/v5.4.23.5/LICENSE`nCanna downloads Framework/BepInEx.zip for automatic setup.`n") },
        @{ path = 'bopl-battle/game.json'; bytes = [System.Text.Encoding]::UTF8.GetBytes(($cannaGame | ConvertTo-Json -Depth 20)) }
    )
    $cannaTreeEntries = @()
    foreach ($cannaUpload in $cannaUploads) {
        $cannaBlob = Invoke-CannaApi 'git/blobs' 'POST' @{ content = [Convert]::ToBase64String($cannaUpload.bytes); encoding = 'base64' }
        $cannaTreeEntries += @{ path = $cannaUpload.path; mode = '100644'; type = 'blob'; sha = $cannaBlob.sha }
    }
    $cannaTree = Invoke-CannaApi 'git/trees' 'POST' @{ base_tree = $cannaCommit.tree.sha; tree = $cannaTreeEntries }
    $cannaNewCommit = Invoke-CannaApi 'git/commits' 'POST' @{ message = 'Exclude steel Anvils from Drill Through Ball piercing and death protection (1.0.5)'; tree = $cannaTree.sha; parents = @($cannaRef.object.sha) }
    $null = Invoke-CannaApi 'git/refs/heads/main' 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    Write-Output "Published framework, drill mod and catalog metadata in commit $($cannaNewCommit.sha)."
} catch {
    Write-Output 'Repository publication failed.'
    if ($_.Exception.Response) { Write-Output ('HTTP status: ' + [int]$_.Exception.Response.StatusCode) }
    exit 1
} finally { $cannaToken = $null; $cannaHeaders = $null }
