param([string]$Target='')
$ErrorActionPreference='Stop'
$cannaRoot=Split-Path $PSScriptRoot -Parent
Push-Location $cannaRoot
$cannaPreviousBuild=$env:CANNA_MAINTENANCE_BUILD
try {
 $env:CANNA_MAINTENANCE_BUILD='1'
 if($Target){cargo build --release --locked --bin canna-updater --target $Target}else{cargo build --release --locked --bin canna-updater}
 if($LASTEXITCODE -ne 0){throw 'Maintenance build failed.'}
}finally{$env:CANNA_MAINTENANCE_BUILD=$cannaPreviousBuild;Pop-Location}
$cannaVersion=[regex]::Match([IO.File]::ReadAllText((Join-Path $cannaRoot 'src/bin/canna-updater.rs')),'MAINTENANCE_VERSION: &str = "(\d+\.\d+\.\d+)"').Groups[1].Value
if(!$cannaVersion){throw 'Invalid maintenance version.'}
$cannaExe=Join-Path $cannaRoot 'dist/Canna Updater.exe'
$cannaArtifact=if($Target){Join-Path $cannaRoot "target/$Target/release/canna-updater.exe"}else{Join-Path $cannaRoot 'target/release/canna-updater.exe'}
Copy-Item -LiteralPath $cannaArtifact -Destination $cannaExe -Force
$cannaHash=(Get-FileHash -LiteralPath $cannaExe -Algorithm SHA256).Hash.ToLowerInvariant()
$cannaManifest=@{tag_name="v$cannaVersion";draft=$false;prerelease=$false;assets=@(@{name='Canna-Updater.exe';size=(Get-Item -LiteralPath $cannaExe).Length;digest="sha256:$cannaHash"})}
[IO.File]::WriteAllText((Join-Path $cannaRoot 'dist/maintenance-latest.json'),($cannaManifest|ConvertTo-Json -Depth 5),[Text.UTF8Encoding]::new($false))
"Built Canna Maintenance $cannaVersion; launcher EXE unchanged."
