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
    $cannaModFile = Join-Path $cannaRoot 'mods\Anvil\build\Canna-Anvil-1.0.2.zip'
    $cannaHash = (Get-FileHash -LiteralPath $cannaModFile -Algorithm SHA256).Hash.ToLowerInvariant()
    $cannaMod = @{ name = 'Canna Anvil'; version = '1.0.2'; description = 'Larger steel anvil with a quick 0.20-second slime morph and five-second duration. Native-sized circular eyeless HUD/menu icon. Requires AbilityScrollBar for the expanded picker. All players need this version; family online verification is pending.'; file = 'Mods/Canna-Anvil-1.0.2.zip'; sha256 = $cannaHash; dependencies = @('AbilityScrollBar') }
    $cannaScrollFile=Join-Path $cannaRoot 'mods/FamilyCatalog/build/YuralGonnadi-AbilityScrollBar-1.0.1.zip'
    $cannaScroll=@{name='AbilityScrollBar';version='1.0.1';description='Allows the expanded native ability picker to scroll with the wheel and follow selection. Original mod by YuralGonnadi.';file='Mods/YuralGonnadi-AbilityScrollBar-1.0.1.zip';sha256=(Get-FileHash -LiteralPath $cannaScrollFile -Algorithm SHA256).Hash.ToLowerInvariant()}
    $cannaGame.mods=@($cannaGame.mods|Where-Object {$_.name -ne 'AbilityScrollBar'})+@($cannaScroll)
    $cannaGame.mods = @($cannaGame.mods | Where-Object { $_.name -ne $cannaMod.name }) + @($cannaMod)
    $cannaUploads = @(
        @{ path = 'bopl-battle/Mods/YuralGonnadi-AbilityScrollBar-1.0.1.zip'; bytes = [IO.File]::ReadAllBytes($cannaScrollFile) },
        @{ path = 'bopl-battle/Mods/AbilityScrollBar-SOURCE.md'; bytes = [Text.Encoding]::UTF8.GetBytes("Original unmodified AbilityScrollBar 1.0.1 by YuralGonnadi.`nSource: https://thunderstore.io/c/bopl-battle/p/YuralGonnadi/AbilityScrollBar/`nThe upstream package README incorrectly describes a gravity bubble; the DLL is the ability scrolling mod.`n") },
        @{ path = 'bopl-battle/Mods/Canna-Anvil-1.0.2.zip'; bytes = [IO.File]::ReadAllBytes($cannaModFile) },
        @{ path = 'bopl-battle/Mods/Anvil-README.md'; bytes = [IO.File]::ReadAllBytes((Join-Path $cannaRoot 'mods\Anvil\README.md')) },
        @{ path = 'bopl-battle/game.json'; bytes = [Text.Encoding]::UTF8.GetBytes(($cannaGame | ConvertTo-Json -Depth 20)) }
    )
    foreach ($cannaSourceName in @('Plugin.cs', 'Art.cs', 'Audit.cs', 'MenuAudit.cs', 'build.ps1', 'README.md')) {
        $cannaUploads += @{ path = ('bopl-battle/Mods/Source/CannaAnvil/' + $cannaSourceName); bytes = [IO.File]::ReadAllBytes((Join-Path $cannaRoot ('mods/Anvil/' + $cannaSourceName))) }
    }
    $cannaUploads += @{ path = 'bopl-battle/Mods/Family-Anvil-Bopl.canna.json'; bytes = [IO.File]::ReadAllBytes((Join-Path $cannaRoot 'examples/Family-Anvil-Bopl.canna.json')) }
    $cannaEntries = @()
    foreach ($cannaUpload in $cannaUploads) {
        $cannaBlob = Invoke-CannaApi 'git/blobs' 'POST' @{ content = [Convert]::ToBase64String($cannaUpload.bytes); encoding = 'base64' }
        $cannaEntries += @{ path = $cannaUpload.path; mode = '100644'; type = 'blob'; sha = $cannaBlob.sha }
    }
    $cannaTree = Invoke-CannaApi 'git/trees' 'POST' @{ base_tree = $cannaCommit.tree.sha; tree = $cannaEntries }
    $cannaNewCommit = Invoke-CannaApi 'git/commits' 'POST' @{ message = 'Add selectable Anvil ability with native transformation lifecycle and heavy physics'; tree = $cannaTree.sha; parents = @($cannaRef.object.sha) }
    $null = Invoke-CannaApi 'git/refs/heads/main' 'PATCH' @{ sha = $cannaNewCommit.sha; force = $false }
    "Published anvil mod and catalog: $($cannaNewCommit.sha)"
} catch {
    'Anvil mod publication failed.'
    if ($_.Exception.Response) { 'HTTP status: ' + [int]$_.Exception.Response.StatusCode }
    exit 1
} finally { $cannaToken = $null; $cannaHeaders = $null }




