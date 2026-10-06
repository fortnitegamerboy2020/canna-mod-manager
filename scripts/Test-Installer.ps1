param([string]$Compiler="$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe")
$ErrorActionPreference='Stop'
$cannaRoot=Split-Path $PSScriptRoot -Parent
$cannaFixtureRoot=[IO.Path]::GetFullPath((Join-Path $cannaRoot 'server/security/installer-test'))
if(!$cannaFixtureRoot.StartsWith([IO.Path]::GetFullPath($cannaRoot)+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Fixture path escaped workspace.'}
New-Item -ItemType Directory -Path $cannaFixtureRoot -Force | Out-Null
$cannaTestSetup=Join-Path $cannaRoot 'dist/Canna-Installer-Test.exe'
$cannaInstallDir=Join-Path $cannaFixtureRoot 'Programs/Canna Mod Manager'
$cannaShortcut=Join-Path ([Environment]::GetFolderPath('Programs')) 'Canna Installer Test.lnk'
if(Test-Path -LiteralPath $cannaShortcut){throw 'An existing installer test shortcut must be removed before this test.'}
$cannaVersion=[regex]::Match([IO.File]::ReadAllText((Join-Path $cannaRoot 'Cargo.toml')),'(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"').Groups[1].Value
& $Compiler "/DAppVersion=$cannaVersion" '/DTestInstall=1' (Join-Path $PSScriptRoot 'Canna-Installer.iss') | Out-Null
if($LASTEXITCODE -ne 0){throw 'Test installer compilation failed.'}
$cannaInstalled=$false
try {
 $cannaInstall=Start-Process -FilePath $cannaTestSetup -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/DIR="'+$cannaInstallDir+'"') -WindowStyle Hidden -Wait -PassThru
 if($cannaInstall.ExitCode -ne 0){throw 'Test installation failed.'};$cannaInstalled=$true
 $cannaTarget=Join-Path $cannaInstallDir 'Canna Mod Manager.exe'
 if(!(Test-Path -LiteralPath $cannaShortcut)){throw 'Start menu shortcut missing.'}
 $cannaLink=(New-Object -ComObject WScript.Shell).CreateShortcut($cannaShortcut)
 if($cannaLink.TargetPath -ne $cannaTarget){throw 'Shortcut targets the wrong EXE.'}
 $cannaBefore=(Get-FileHash -LiteralPath $cannaTarget -Algorithm SHA256).Hash
 if($cannaBefore -ne (Get-FileHash -LiteralPath (Join-Path $cannaRoot 'dist/Canna Mod Manager.exe')).Hash){throw 'Installed EXE differs from package.'}
 # A windowless fixture confirms the real update helper replaces and launches the installed path.
 $cannaMarker=Join-Path $cannaFixtureRoot 'updated.marker'
 $cannaSource=Join-Path $cannaFixtureRoot 'Fixture.cs'
 $cannaStaged=Join-Path $cannaFixtureRoot 'new-version.exe'
 $cannaMarkerLiteral=$cannaMarker.Replace('"','""')
 [IO.File]::WriteAllText($cannaSource,('class Fixture { static void Main() { System.IO.File.WriteAllText(@"'+$cannaMarkerLiteral+'", "updated"); } }'))
 & "$env:SystemRoot\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /target:winexe "/out:$cannaStaged" $cannaSource
 if($LASTEXITCODE -ne 0){throw 'Updater fixture compilation failed.'}
 $cannaHash=(Get-FileHash -LiteralPath $cannaStaged -Algorithm SHA256).Hash.ToLowerInvariant()
 $cannaRust=[IO.File]::ReadAllText((Join-Path $cannaRoot 'src/updater.rs'))
 $cannaHelper=[regex]::Match($cannaRust,'r#"(\$ErrorActionPreference.*?)"#,',[Text.RegularExpressions.RegexOptions]::Singleline).Groups[1].Value
 if(!$cannaHelper){throw 'Could not extract the application update helper.'}
 $cannaBackup=[IO.Path]::ChangeExtension($cannaTarget,'previous.exe');$cannaLog=Join-Path $cannaFixtureRoot 'update.log'
 function Canna-Literal([string]$value){"'"+$value.Replace("'","''")+"'"}
 foreach($pair in @(@('{target}',(Canna-Literal $cannaTarget)),@('{staged}',(Canna-Literal $cannaStaged)),@('{backup}',(Canna-Literal $cannaBackup)),@('{log}',(Canna-Literal $cannaLog)),@('{pid}','2147483647'),@('{hash}',$cannaHash))){$cannaHelper=$cannaHelper.Replace($pair[0],$pair[1])}
 $cannaHelper=$cannaHelper.Replace('{{','{').Replace('}}','}')
 $cannaHelperPath=Join-Path $cannaFixtureRoot 'apply-update.ps1';[IO.File]::WriteAllText($cannaHelperPath,$cannaHelper)
 $cannaUpdate=Start-Process -FilePath powershell.exe -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',('"'+$cannaHelperPath+'"') -WindowStyle Hidden -Wait -PassThru
 if($cannaUpdate.ExitCode -ne 0 -or (Get-FileHash -LiteralPath $cannaTarget).Hash.ToLowerInvariant() -ne $cannaHash){throw 'Installed-path update failed.'}
 for($i=0;$i -lt 50 -and !(Test-Path -LiteralPath $cannaMarker);$i++){Start-Sleep -Milliseconds 100}
 if(!(Test-Path -LiteralPath $cannaMarker)){throw 'Updated EXE was not relaunched.'}
 if((Get-FileHash -LiteralPath $cannaBackup).Hash -ne $cannaBefore){throw 'Update rollback copy differs from installed application.'}
 'Installer file/Start menu shortcut and real updater replacement, backup and relaunch passed.'
} finally {
 if($cannaInstalled){$cannaUninstall=Start-Process -FilePath (Join-Path $cannaInstallDir 'unins000.exe') -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -WindowStyle Hidden -Wait -PassThru;if($cannaUninstall.ExitCode -ne 0){throw 'Test uninstallation failed.'}}
 if(Test-Path -LiteralPath $cannaShortcut){throw 'Uninstaller left the test shortcut.'}
 Remove-Item -LiteralPath $cannaFixtureRoot -Recurse -Force
 Remove-Item -LiteralPath $cannaTestSetup -Force
}
