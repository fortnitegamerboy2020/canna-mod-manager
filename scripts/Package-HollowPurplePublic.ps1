param([Parameter(Mandatory=$true)][string]$Stage, [Parameter(Mandatory=$true)][string]$RuntimeReport)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
# Metadata checks are a separate build step. Packaging consumes the completed
# runtime result for the exact staged build without launching or retesting games.
$testedStage=[IO.File]::ReadAllText((Join-Path $root 'target/hollow-public-test-stage.txt')).Trim()
$testedRuntime=[IO.File]::ReadAllText((Join-Path $root 'target/hollow-public-test-root.txt')).Trim()
if([IO.Path]::GetFullPath($Stage)-ne[IO.Path]::GetFullPath($testedStage)-or[IO.Path]::GetFullPath($RuntimeReport)-ne(Join-Path $testedRuntime 'report.txt')){throw 'Package must use the recorded runtime-tested stage and report.'}
$report=[IO.File]::ReadAllLines($RuntimeReport)
if (@($report|Where-Object {$_ -like 'PASS *'}).Count -ne 70 -or @($report|Where-Object {$_ -like 'FAIL *'}).Count -ne 0) {throw 'Complete 70-check public runtime report required.'}
$adapterReport=[IO.File]::ReadAllLines((Join-Path (Split-Path $RuntimeReport -Parent) 'public-adapter-checks.txt'))
if ($adapterReport.Count -ne 4 -or @($adapterReport|Where-Object {$_ -notlike 'PASS *'}).Count -ne 0) {throw 'Four passing public adapter scene/lifecycle checks required.'}
Copy-Item -LiteralPath (Join-Path $root 'mods/HollowPurpleFixed/CANNA-FIX.md') -Destination (Join-Path $Stage 'CANNA-FIX.md') -Force
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $Stage 'CANNA-LICENSE.txt') -Force
# Include only assertion results, not game logs, device information or local paths.
$safeReport=@('Default public ROUNDS build 21020021 / Unity 2022.3.34f1; automated local/offline runtime.','Two-client multiplayer and human playtesting are not verified.')+@($report|Where-Object {$_ -like 'PASS *' -or $_ -like 'LIMIT:*'})+$adapterReport
[IO.File]::WriteAllLines((Join-Path $Stage 'CANNA-VALIDATION.txt'),$safeReport,[Text.UTF8Encoding]::new($false))
$out=Join-Path $root 'mods/HollowPurpleFixed/build/HollowPurple-Fixed-1.8.2.zip'
Compress-Archive -Path (Join-Path $Stage '*') -DestinationPath $out -Force
$evidence=@{version='1.8.2';archive_sha256=(Get-FileHash -LiteralPath $out).Hash.ToLowerInvariant();dll_sha256=(Get-FileHash -LiteralPath (Join-Path $Stage 'BepInEx/plugins/HollowPurple/HollowPurple.dll')).Hash.ToLowerInvariant();runtime_checks=70;adapter_checks=4;public_build='21020021';multiplayer_verified=$false}
[IO.File]::WriteAllText((Join-Path $root 'mods/HollowPurpleFixed/build/public-evidence.json'),($evidence|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
Write-Output ('Packaged public port: '+$out)
