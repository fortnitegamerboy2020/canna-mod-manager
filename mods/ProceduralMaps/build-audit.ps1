param([string]$GamePath = 'D:\SteamLibrary\steamapps\common\Bopl Battle', [switch]$Run)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'build.ps1') -GamePath $GamePath -Audit
if (-not $Run) { return }
if (Get-Process BoplBattle -ErrorAction SilentlyContinue) { throw 'Close Bopl Battle before the audit.' }
$cannaInstalled = @(Get-ChildItem -LiteralPath (Join-Path $GamePath 'BepInEx\plugins') -Filter 'Canna.ProceduralMaps.dll' -Recurse -File)
if ($cannaInstalled.Count -ne 1) { throw 'Expected exactly one installed Procedural Maps DLL.' }
$cannaDestination = $cannaInstalled[0].FullName
$cannaBackup = Join-Path $PSScriptRoot 'build\pre-audit-production.dll'
Copy-Item -LiteralPath $cannaDestination -Destination $cannaBackup
$cannaProcess = $null
try {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'build\Canna.ProceduralMaps.dll') -Destination $cannaDestination
    $cannaProcess = Start-Process -FilePath (Join-Path $GamePath 'BoplBattle.exe') -WorkingDirectory $GamePath -ArgumentList '-batchmode','-nographics','--canna-map-audit' -WindowStyle Hidden -PassThru
    for ($cannaWait = 0; $cannaWait -lt 24 -and -not $cannaProcess.HasExited; $cannaWait++) { $null = $cannaProcess.WaitForExit(10000) }
    if (-not $cannaProcess.HasExited) { throw 'Owned scene audit timed out.' }
    $cannaResults = Join-Path $GamePath 'BepInEx\config\CannaMaps'
    if (Test-Path -LiteralPath (Join-Path $cannaResults 'audit-failure.txt')) { throw ([IO.File]::ReadAllText((Join-Path $cannaResults 'audit-failure.txt'))) }
    Get-Content -LiteralPath (Join-Path $cannaResults 'audit-complete.txt')
} finally {
    if ($null -ne $cannaProcess -and -not $cannaProcess.HasExited) {
        $cannaOwned = Get-CimInstance Win32_Process -Filter "ProcessId=$($cannaProcess.Id)"
        if ($cannaOwned.CommandLine -match '--canna-map-audit' -and $cannaOwned.ExecutablePath -eq (Join-Path $GamePath 'BoplBattle.exe')) {
            Stop-Process -Id $cannaProcess.Id -Force
            $null = $cannaProcess.WaitForExit(10000)
        }
    }
    Copy-Item -LiteralPath $cannaBackup -Destination $cannaDestination
}