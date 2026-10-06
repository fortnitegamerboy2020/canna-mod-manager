param([string]$Compiler = '')
$ErrorActionPreference='Stop'
$cannaRoot=Split-Path $PSScriptRoot -Parent
$cannaVersion=[regex]::Match([IO.File]::ReadAllText((Join-Path $cannaRoot 'Cargo.toml')),'(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"').Groups[1].Value
if(!$cannaVersion){throw 'Could not read application version.'}
$cannaExe=Join-Path $cannaRoot 'dist/Canna Mod Manager.exe'
if(!(Test-Path -LiteralPath $cannaExe)){throw 'Build the desktop application first, or place the existing release EXE in dist.'}
if([Diagnostics.FileVersionInfo]::GetVersionInfo($cannaExe).ProductVersion -ne $cannaVersion){throw 'Desktop EXE version does not match Cargo.toml. Build the new launcher before packaging.'}
if(!(Test-Path -LiteralPath (Join-Path $cannaRoot 'dist/Canna Updater.exe'))){throw 'Run scripts/Build-Maintenance.ps1 before packaging the installer.'}
if(!$Compiler){
 $cannaCandidates=@("${env:ProgramFiles}\Inno Setup 7\ISCC.exe","${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe","${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe","$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe","$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe")
 $Compiler=$cannaCandidates | Where-Object {Test-Path -LiteralPath $_} | Select-Object -First 1
}
if(!$Compiler -or !(Test-Path -LiteralPath $Compiler)){throw 'Inno Setup is needed to compile the installer. Download it from https://jrsoftware.org/isdl.php and pass -Compiler if necessary.'}
# Package only: no desktop build or change to the executable/update channel.
$cannaHashBefore=(Get-FileHash -LiteralPath $cannaExe -Algorithm SHA256).Hash
& $Compiler "/DAppVersion=$cannaVersion" (Join-Path $PSScriptRoot 'Canna-Installer.iss')
if($LASTEXITCODE -ne 0){throw 'Installer compilation failed.'}
if((Get-FileHash -LiteralPath $cannaExe -Algorithm SHA256).Hash -ne $cannaHashBefore){throw 'Desktop EXE changed while packaging.'}
$cannaSetup=Join-Path $cannaRoot "dist/Canna-Setup-$cannaVersion.exe"
$cannaManifest=@{version="v$cannaVersion";size=(Get-Item -LiteralPath $cannaSetup).Length;sha256=(Get-FileHash -LiteralPath $cannaSetup -Algorithm SHA256).Hash.ToLowerInvariant()}
[IO.File]::WriteAllText((Join-Path $cannaRoot 'dist/installer-latest.json'),($cannaManifest|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
Write-Output "Installer ready: $cannaSetup"
