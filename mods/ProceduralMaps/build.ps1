param([string]$GamePath = 'D:\SteamLibrary\steamapps\common\Bopl Battle', [string]$FrameworkZip = 'C:\Users\t_tra\Downloads\bopl-battle\Framework\BepInEx.zip')
$ErrorActionPreference = 'Stop'
$cannaOutput = Join-Path $PSScriptRoot 'build'
New-Item -ItemType Directory -Path $cannaOutput -Force | Out-Null
$cannaReferences = Join-Path $cannaOutput 'references'
if (-not (Test-Path -LiteralPath $cannaReferences)) { Expand-Archive -LiteralPath $FrameworkZip -DestinationPath $cannaReferences }
$cannaManaged = Join-Path $GamePath 'BoplBattle_Data\Managed'
$cannaCompiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$cannaCore = Join-Path $cannaReferences 'BepInEx\core'
$cannaDll = Join-Path $cannaOutput 'Canna.ProceduralMaps.dll'
& $cannaCompiler /nologo /noconfig /nostdlib /target:library /optimize+ "/reference:$cannaManaged\mscorlib.dll" "/reference:$cannaManaged\netstandard.dll" "/reference:$cannaManaged\System.dll" "/reference:$cannaManaged\System.Core.dll" "/out:$cannaDll" "/reference:$cannaCore\BepInEx.dll" "/reference:$cannaCore\0Harmony.dll" "/reference:$cannaManaged\Assembly-CSharp.dll" "/reference:$cannaManaged\UnityEngine.dll" "/reference:$cannaManaged\UnityEngine.CoreModule.dll" "/reference:$cannaManaged\Facepunch.Steamworks.Win64.dll" "/reference:$cannaManaged\UnityEngine.JSONSerializeModule.dll" (Join-Path $PSScriptRoot 'Layout.cs') (Join-Path $PSScriptRoot 'Plugin.cs')
if ($LASTEXITCODE -ne 0) { throw 'Procedural Maps compilation failed.' }
$cannaPluginFolder = Join-Path $cannaOutput 'package\BepInEx\plugins\CannaProceduralMaps'
New-Item -ItemType Directory -Path $cannaPluginFolder -Force | Out-Null
Copy-Item -LiteralPath $cannaDll -Destination $cannaPluginFolder
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $cannaOutput 'package\README.md')
Compress-Archive -Path (Join-Path $cannaOutput 'package\*') -DestinationPath (Join-Path $cannaOutput 'Canna-ProceduralMaps-1.0.3.zip') -Force
Write-Output 'Built Canna-ProceduralMaps-1.0.3.zip'


