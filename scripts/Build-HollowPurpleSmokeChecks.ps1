param([string]$Game='D:\SteamLibrary\steamapps\common\ROUNDS')
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$output=Join-Path $root 'target/HollowPurple.PublicSmokeChecks.dll'
$rsp=@('/nologo','/target:library','/langversion:5','/nostdlib+',('/out:"'+$output+'"'))
$rsp+=Get-ChildItem -LiteralPath (Join-Path $Game 'ROUNDS_Data/Managed') -Filter '*.dll' | ForEach-Object {'/reference:"'+$_.FullName+'"'}
$rsp+=('/reference:"'+(Join-Path $Game 'BepInEx/core/BepInEx.dll')+'"')
$rsp+=('"'+(Join-Path $root 'mods/HollowPurpleFixed/PublicSmokeChecks.cs')+'"')
$response=Join-Path $root 'target/hollow-public-smoke-checks.rsp'
[IO.File]::WriteAllLines($response,$rsp,[Text.UTF8Encoding]::new($false))
& 'C:/Windows/Microsoft.NET/Framework64/v4.0.30319/csc.exe' /noconfig ('@'+$response)
if($LASTEXITCODE-ne0){throw 'Smoke-check plugin compilation failed.'}
Write-Output ('Built test-only checks: '+$output)
