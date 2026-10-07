param(
 [Parameter(Mandatory=$true)][string]$RoamingBase,
 [Parameter(Mandatory=$true)][string]$LocalBase,
 [Parameter(Mandatory=$true)][string]$TempBase
)
$ErrorActionPreference='Stop'
# Delete only the app's known data roots. Do not follow junctions/symlinks into other folders.
function Remove-CannaTree([string]$Path) {
 if(!(Test-Path -LiteralPath $Path)){return}
 $cannaItem=Get-Item -LiteralPath $Path -Force
 if($cannaItem.Attributes -band [IO.FileAttributes]::ReparsePoint){
  if($cannaItem.PSIsContainer){[IO.Directory]::Delete($cannaItem.FullName,$false)}else{[IO.File]::Delete($cannaItem.FullName)}
  return
 }
 if($cannaItem.PSIsContainer){
  foreach($cannaChild in @(Get-ChildItem -LiteralPath $cannaItem.FullName -Force)){Remove-CannaTree $cannaChild.FullName}
  [IO.Directory]::Delete($cannaItem.FullName,$false)
 }else{
  $cannaItem.Attributes=$cannaItem.Attributes -band (-bnot [IO.FileAttributes]::ReadOnly)
  [IO.File]::Delete($cannaItem.FullName)
 }
}
try {
 # External recovery data is removable only with Canna's dedicated-folder marker.
 $cannaPolicyPath=Join-Path $RoamingBase 'CannaModManager/play-lab/policy.json'
 if(Test-Path -LiteralPath $cannaPolicyPath){
  if((Get-Item -LiteralPath $cannaPolicyPath).Length -gt 8192){throw 'Invalid recovery policy.'}
  $cannaPolicy=Get-Content -LiteralPath $cannaPolicyPath -Raw -Encoding UTF8 | ConvertFrom-Json
  if(![IO.Path]::IsPathRooted($cannaPolicy.directory)){throw 'Invalid recovery folder.'}
  $cannaRecovery=[IO.Path]::GetFullPath($cannaPolicy.directory).TrimEnd('\','/')
  if($cannaRecovery -eq [IO.Path]::GetPathRoot($cannaRecovery).TrimEnd('\','/')){throw 'Recovery folder cannot be a drive root.'}
  $cannaCursor=$cannaRecovery
  while($cannaCursor){
   if((Test-Path -LiteralPath $cannaCursor) -and ((Get-Item -LiteralPath $cannaCursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw 'Recovery folder cannot traverse a link.'}
   $cannaCursor=[IO.Path]::GetDirectoryName($cannaCursor)
  }
  $cannaMarker=Join-Path $cannaRecovery '.canna-recovery-owner'
  if(Test-Path -LiteralPath $cannaMarker){
   $cannaMarkerItem=Get-Item -LiteralPath $cannaMarker -Force
   if(($cannaMarkerItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $cannaMarkerItem.Length -gt 64){throw 'Invalid recovery ownership marker.'}
   if([IO.File]::ReadAllText($cannaMarker) -ne 'CannaRecovery-v1'){throw 'Invalid recovery ownership marker.'}
   $cannaSnapshots=[IO.Path]::GetFullPath((Join-Path $cannaRecovery 'snapshots'))
   if([IO.Path]::GetDirectoryName($cannaSnapshots) -ne $cannaRecovery){throw 'Invalid recovery cleanup path.'}
   Remove-CannaTree $cannaSnapshots
   Remove-CannaTree $cannaMarker
   if(!(Get-ChildItem -LiteralPath $cannaRecovery -Force | Select-Object -First 1)){[IO.Directory]::Delete($cannaRecovery,$false)}
  }
 }
 foreach($cannaBase in @($RoamingBase,$LocalBase,$TempBase)){
  if(![IO.Path]::IsPathRooted($cannaBase)){throw 'Cleanup base must be an absolute directory.'}
  $cannaParent=[IO.Path]::GetFullPath($cannaBase).TrimEnd('\','/')
  $cannaTarget=[IO.Path]::GetFullPath((Join-Path $cannaParent 'CannaModManager'))
  if([IO.Path]::GetDirectoryName($cannaTarget) -ne $cannaParent){throw 'Invalid Canna cleanup path.'}
  Remove-CannaTree $cannaTarget
 }
 # Console export files are the only production files written directly into Temp.
 foreach($cannaLog in @(Get-ChildItem -LiteralPath $TempBase -Filter 'canna-console-*.log' -File -Force -ErrorAction SilentlyContinue)){
  if($cannaLog.Name -match '^canna-console-\d+\.log$'){Remove-CannaTree $cannaLog.FullName}
 }
 exit 0
}catch{
 Write-Error ('Could not remove all Canna data. Close Canna and its Minecraft instances, then retry uninstall. '+$_.Exception.Message) -ErrorAction Continue
 exit 1
}
