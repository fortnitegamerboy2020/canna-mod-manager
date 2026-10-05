param([string]$GamePath = 'D:\SteamLibrary\steamapps\common\Bopl Battle', [switch]$Run)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build.ps1') -GamePath $GamePath
$cannaManaged = Join-Path $GamePath 'BoplBattle_Data\Managed'
$cannaCore = Join-Path $PSScriptRoot 'build\references\BepInEx\core'
$cannaCompiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
& $cannaCompiler /nologo /noconfig /nostdlib /target:library "/out:$PSScriptRoot\build\Canna.MapAudit.dll" "/reference:$cannaManaged\mscorlib.dll" "/reference:$cannaManaged\netstandard.dll" "/reference:$cannaManaged\System.dll" "/reference:$cannaManaged\System.Core.dll" "/reference:$cannaCore\BepInEx.dll" "/reference:$cannaCore\0Harmony.dll" "/reference:$cannaManaged\Assembly-CSharp.dll" "/reference:$cannaManaged\UnityEngine.dll" "/reference:$cannaManaged\UnityEngine.CoreModule.dll" "/reference:$PSScriptRoot\build\Canna.ProceduralMaps.dll" "$PSScriptRoot\AuditScenes.cs"
if ($LASTEXITCODE -ne 0) { throw 'Audit compilation failed' }
if ($Run) {
    if (Get-Process BoplBattle -ErrorAction SilentlyContinue) { throw 'Close Bopl Battle before the audit' }
    $cannaFolder = Join-Path $GamePath 'BepInEx\plugins\CannaSceneAudit'
    New-Item -ItemType Directory -Path $cannaFolder -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'build\Canna.MapAudit.dll'),(Join-Path $PSScriptRoot 'build\Canna.ProceduralMaps.dll') -Destination $cannaFolder
    $cannaProcess = Start-Process -FilePath (Join-Path $GamePath 'BoplBattle.exe') -ArgumentList '--canna-map-audit','-screen-fullscreen','0','-screen-width','800','-screen-height','600' -WorkingDirectory $GamePath -PassThru
    $cannaProcess.Id | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'build\audit-pid.txt')
    'Started scene audit process ' + $cannaProcess.Id
}
