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
