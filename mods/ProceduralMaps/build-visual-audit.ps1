param([switch]$Run)
$ErrorActionPreference='Stop'
$cannaManaged='D:\SteamLibrary\steamapps\common\Bopl Battle\BoplBattle_Data\Managed'
$cannaCore=Join-Path $PSScriptRoot 'build\references\BepInEx\core'
$cannaArgs=@('/nologo','/noconfig','/nostdlib','/target:library',('/out:'+(Join-Path $PSScriptRoot 'build\Canna.VisualAudit.dll')))
foreach($cannaName in @('mscorlib','netstandard','System','System.Core','Assembly-CSharp','UnityEngine','UnityEngine.CoreModule','UnityEngine.ScreenCaptureModule')){$cannaArgs+=('/reference:'+(Join-Path $cannaManaged ($cannaName+'.dll')))}
foreach($cannaName in @('BepInEx','0Harmony')){$cannaArgs+=('/reference:'+(Join-Path $cannaCore ($cannaName+'.dll')))}
$cannaArgs+=('/reference:'+(Join-Path $PSScriptRoot 'build\Canna.ProceduralMaps.dll'))
$cannaArgs+=(Join-Path $PSScriptRoot 'VisualAudit.cs')
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" @cannaArgs
if($LASTEXITCODE -ne 0){throw 'Compile failed'}
if($Run){
 if(Get-Process BoplBattle -ErrorAction SilentlyContinue){throw 'Game running'}
 $cannaAudit='D:\SteamLibrary\steamapps\common\Bopl Battle\BepInEx\plugins\CannaSceneAudit'
 New-Item -ItemType Directory $cannaAudit -Force | Out-Null
 Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'build\Canna.VisualAudit.dll') -Destination $cannaAudit
 $cannaProcess=Start-Process -FilePath 'D:\SteamLibrary\steamapps\common\Bopl Battle\BoplBattle.exe' -WorkingDirectory 'D:\SteamLibrary\steamapps\common\Bopl Battle' -ArgumentList '--canna-visual-audit','-screen-fullscreen','0','-screen-width','1280','-screen-height','720' -WindowStyle Hidden -PassThru
 $cannaProcess.Id | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'build\visual-pid.txt')
 Write-Output $cannaProcess.Id
}
