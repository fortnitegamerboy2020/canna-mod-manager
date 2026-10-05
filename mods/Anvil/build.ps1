param([switch]$Audit, [string]$GamePath = 'D:\SteamLibrary\steamapps\common\Bopl Battle', [string]$FrameworkZip = 'C:\Users\t_tra\Downloads\bopl-battle\Framework\BepInEx.zip')
$ErrorActionPreference = 'Stop'
$cannaOutput = Join-Path $PSScriptRoot 'build'
New-Item -ItemType Directory -Path $cannaOutput -Force | Out-Null
$cannaReferences = Join-Path $cannaOutput 'references'
if (-not (Test-Path -LiteralPath $cannaReferences)) { Expand-Archive -LiteralPath $FrameworkZip -DestinationPath $cannaReferences }
$cannaManaged = Join-Path $GamePath 'BoplBattle_Data\Managed'
$cannaCompiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$cannaCore = Join-Path $cannaReferences 'BepInEx\core'
$cannaDll = Join-Path $cannaOutput 'Canna.Anvil.dll'
$cannaSources = @((Join-Path $PSScriptRoot 'Art.cs'), (Join-Path $PSScriptRoot 'Plugin.cs'))
if ($Audit) { $cannaSources += Join-Path $PSScriptRoot 'Audit.cs'; $cannaSources += Join-Path $PSScriptRoot 'MenuAudit.cs' }
& $cannaCompiler /nologo /noconfig /nostdlib /target:library /optimize+ "/reference:$cannaManaged\mscorlib.dll" "/reference:$cannaManaged\netstandard.dll" "/reference:$cannaManaged\System.dll" "/reference:$cannaManaged\System.Core.dll" "/out:$cannaDll" "/reference:$cannaCore\BepInEx.dll" "/reference:$cannaCore\0Harmony.dll" "/reference:$cannaManaged\Assembly-CSharp.dll" "/reference:$cannaManaged\UnityEngine.dll" "/reference:$cannaManaged\UnityEngine.CoreModule.dll" "/reference:$cannaManaged\Facepunch.Steamworks.Win64.dll" "/reference:$cannaManaged\UnityEngine.JSONSerializeModule.dll" "/reference:$cannaManaged\UnityEngine.ImageConversionModule.dll" "/reference:$cannaManaged\UnityEngine.ScreenCaptureModule.dll" @cannaSources
if ($LASTEXITCODE -ne 0) { throw 'Anvil compilation failed.' }
if ($Audit) { Write-Output 'Built development audit DLL; not packaged'; return }
$cannaPluginFolder = Join-Path $cannaOutput 'package\BepInEx\plugins\CannaAnvil'
New-Item -ItemType Directory -Path $cannaPluginFolder -Force | Out-Null
Copy-Item -LiteralPath $cannaDll -Destination $cannaPluginFolder
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $cannaOutput 'package\README.md')
Compress-Archive -Path (Join-Path $cannaOutput 'package\*') -DestinationPath (Join-Path $cannaOutput 'Canna-Anvil-1.0.2.zip') -Force
Write-Output 'Built Canna-Anvil-1.0.2.zip'




