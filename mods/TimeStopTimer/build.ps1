param([string]$GamePath='D:\SteamLibrary\steamapps\common\Bopl Battle')
$ErrorActionPreference='Stop'
$cannaManaged=Join-Path $GamePath 'BoplBattle_Data\Managed'
$cannaRefs=@('mscorlib.dll','netstandard.dll','System.dll','System.Core.dll','Assembly-CSharp.dll','UnityEngine.dll','UnityEngine.CoreModule.dll','UnityEngine.IMGUIModule.dll','UnityEngine.TextRenderingModule.dll','Facepunch.Steamworks.Win64.dll')|ForEach-Object {'/reference:'+(Join-Path $cannaManaged $_)}
$cannaRefs+=('/reference:'+(Join-Path $GamePath 'BepInEx\core\BepInEx.dll'));$cannaRefs+=('/reference:'+(Join-Path $GamePath 'BepInEx\core\0Harmony.dll'))
New-Item -ItemType Directory (Join-Path $PSScriptRoot 'build') -Force|Out-Null
$cannaOut=Join-Path $PSScriptRoot 'build\TimeStopTimerRepair.dll'
& "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /noconfig /nostdlib /target:library @cannaRefs "/out:$cannaOut" (Join-Path $PSScriptRoot 'Plugin.cs')
if($LASTEXITCODE -ne 0){throw 'Timer build failed'}
New-Item -ItemType Directory (Join-Path $PSScriptRoot 'build\package') -Force|Out-Null
Copy-Item -LiteralPath $cannaOut -Destination (Join-Path $PSScriptRoot 'build\package')
Copy-Item -LiteralPath (Join-Path (Split-Path $PSScriptRoot -Parent) 'FamilyCatalog\build\extracted\Antimality-TimeStopTimer\TimeStopTimer.DLL') -Destination (Join-Path $PSScriptRoot 'build\package')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $PSScriptRoot 'build\package')
Compress-Archive -Path (Join-Path $PSScriptRoot 'build\package\*') -DestinationPath (Join-Path $PSScriptRoot 'build\TimeStopTimer-1.1.2-canna.zip') -Force
