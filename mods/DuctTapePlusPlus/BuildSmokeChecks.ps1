param([string]$GameDir = 'D:\SteamLibrary\steamapps\common\ROUNDS', [string]$Dotnet = '')
$ErrorActionPreference = 'Stop'
$repository = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (!$Dotnet) { $Dotnet = Join-Path $repository 'target/build-tools/dotnet-8.0.425/dotnet.exe' }
$output = Join-Path $repository 'target/ducttape-plusplus-smokechecks'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_SKIP_FIRST_TIME_EXPERIENCE = '1'
$env:DOTNET_GENERATE_ASPNET_CERTIFICATE = 'false'
$env:DOTNET_ADD_GLOBAL_TOOLS_TO_PATH = 'false'
$env:DOTNET_CLI_HOME = Join-Path $output 'dotnet-home'
$env:NUGET_PACKAGES = Join-Path $output 'nuget'
& $Dotnet build (Join-Path $PSScriptRoot 'SmokeChecks.csproj') -c Release ('-p:GameDir=' + $GameDir) -o $output --nologo
if ($LASTEXITCODE -ne 0) { throw 'Test-only smoke plugin build failed.' }
Write-Output (Join-Path $output 'Canna.DuctTapePlusPlus.SmokeChecks.dll')
