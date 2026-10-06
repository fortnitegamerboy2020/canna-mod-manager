param()
$ErrorActionPreference='Stop'
$cannaRoot=Split-Path $PSScriptRoot -Parent
$cannaTest=[IO.Path]::GetFullPath((Join-Path $cannaRoot 'server/security/maintenance-test'))
if(!$cannaTest.StartsWith([IO.Path]::GetFullPath($cannaRoot)+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Unsafe fixture directory.'}
New-Item -ItemType Directory -Path $cannaTest -Force | Out-Null
$cannaRust=Get-Content (Join-Path $cannaRoot 'src/updater.rs') -Raw
$cannaTemplate=[regex]::Match($cannaRust,'r#"(\$ErrorActionPreference.*?)"#,',[Text.RegularExpressions.RegexOptions]::Singleline).Groups[1].Value
if(!$cannaTemplate){throw 'Update helper missing.'}
function Canna-Literal([string]$value){"'"+$value.Replace("'","''")+"'"}
function Invoke-TestHelper([string]$target,[string]$staged,[string]$digest,[string]$label){
 $backup=[IO.Path]::ChangeExtension($target,'previous.exe');$log=Join-Path $cannaTest "$label.log"
 $helper=$cannaTemplate
 foreach($pair in @(@('{target}',(Canna-Literal $target)),@('{staged}',(Canna-Literal $staged)),@('{backup}',(Canna-Literal $backup)),@('{log}',(Canna-Literal $log)),@('{pid}','2147483647'),@('{hash}',$digest))){$helper=$helper.Replace($pair[0],$pair[1])}
 $helper=$helper.Replace('{{','{').Replace('}}','}')
 $script=Join-Path $cannaTest "$label.ps1";[IO.File]::WriteAllText($script,$helper)
 $process=Start-Process powershell.exe -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',('"'+$script+'"') -WindowStyle Hidden -Wait -PassThru
 if(!(Test-Path -LiteralPath $log)){throw 'Helper log missing.'}
 if(Test-Path -LiteralPath ($target+'.update.lock')){throw 'Update lock leaked.'}
 return [IO.File]::ReadAllText($log)
}
try {
 $good=Join-Path $cannaTest 'good.exe';$bad=Join-Path $cannaTest 'bad.exe'
 foreach($fixture in @(@($good,0),@($bad,23))){
  $source=Join-Path $cannaTest ('fixture-'+$fixture[1]+'.cs')
  [IO.File]::WriteAllText($source,('class Fixture { static int Main() { return '+$fixture[1]+'; } }'))
  & "$env:SystemRoot\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /target:winexe ("/out:"+$fixture[0]) $source
  if($LASTEXITCODE -ne 0){throw 'Fixture compilation failed.'}
 }
 $goodHash=(Get-FileHash $good).Hash.ToLowerInvariant();$badHash=(Get-FileHash $bad).Hash.ToLowerInvariant()
 $target=Join-Path $cannaTest 'Canna Updater.exe'
 # Repair can create a missing updater, as needed by the independent recovery copy.
 if((Invoke-TestHelper $target $good $goodHash 'repair') -ne 'Update installed successfully'){throw 'Missing updater repair failed.'}
 if((Get-FileHash $target).Hash.ToLowerInvariant() -ne $goodHash){throw 'Repaired updater differs from verified download.'}
 # Reject tampering before modifying the existing updater.
 if((Invoke-TestHelper $target $bad $goodHash 'tamper') -notmatch 'checksum mismatch'){throw 'Tampering was accepted.'}
 if((Get-FileHash $target).Hash.ToLowerInvariant() -ne $goodHash){throw 'Tampering changed the installed updater.'}
 # A replacement which fails to start cleanly restores the original executable.
 if((Invoke-TestHelper $target $bad $badHash 'crash') -notmatch 'restored previous'){throw 'Crash did not trigger rollback.'}
 if((Get-FileHash $target).Hash.ToLowerInvariant() -ne $goodHash){throw 'Rollback did not restore the original updater.'}
 if((Invoke-TestHelper $target $good $goodHash 'update') -ne 'Update installed successfully'){throw 'Updater replacement failed.'}
 $backup=[IO.Path]::ChangeExtension($target,'previous.exe')
 if([IO.File]::ReadAllText($backup+'.sha256') -ne (Get-FileHash $backup).Hash.ToLowerInvariant()){throw 'Rollback digest not recorded correctly.'}
 'Maintenance helper: repair missing updater, reject tampering, crash rollback, replacement and backup digest passed.'
}finally{Remove-Item -LiteralPath $cannaTest -Recurse -Force}
