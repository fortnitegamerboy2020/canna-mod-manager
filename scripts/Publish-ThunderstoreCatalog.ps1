param([string]$TokenFile='C:\Users\t_tra\Downloads\chatgpttoken_mods.txt')
$ErrorActionPreference='Stop'
$cannaRoot=Split-Path $PSScriptRoot -Parent
$cannaBuild=Join-Path $cannaRoot 'mods\FamilyCatalog\build'
$cannaToken=[IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
$cannaHeaders=@{Authorization="Bearer $cannaToken";'User-Agent'='Canna-Mod-Manager';Accept='application/vnd.github+json'}
$cannaApi='https://api.github.com/repos/fortnitegamerboy2020/manager-uploaded-mods'
function Invoke-CannaApi([string]$Path,[string]$Method='GET',$Body=$null){
 $cannaRequest=@{Uri="$cannaApi/$Path";Headers=$cannaHeaders;Method=$Method;TimeoutSec=60}
 if($null -ne $Body){$cannaRequest.Body=($Body|ConvertTo-Json -Depth 35 -Compress);$cannaRequest.ContentType='application/json'}
 Invoke-RestMethod @cannaRequest
}
try {
 $cannaSelected=Get-Content -LiteralPath (Join-Path $cannaBuild 'selected-with-dependencies.json') -Raw|ConvertFrom-Json
 # Verify the pinned downloads are still the latest before mirroring anything.
 $cannaLatest=Invoke-RestMethod 'https://thunderstore.io/c/bopl-battle/api/v1/package/' -TimeoutSec 120
 foreach($cannaPackage in $cannaSelected){
  $cannaCurrent=$cannaLatest|Where-Object full_name -eq $cannaPackage.full_name|Select-Object -First 1
  if(!$cannaCurrent -or $cannaCurrent.versions[0].version_number -ne $cannaPackage.versions[0].version_number){throw 'Upstream version changed; repeat compatibility audit before publishing'}
 }
 $cannaRef=Invoke-CannaApi 'git/ref/heads/main'
 $cannaCommit=Invoke-CannaApi "git/commits/$($cannaRef.object.sha)"
 $cannaFile=Invoke-CannaApi 'contents/bopl-battle/game.json?ref=main'
 $cannaGame=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($cannaFile.content))|ConvertFrom-Json
 $cannaUploads=@();$cannaEntries=@();$cannaProvenance=@()
 foreach($cannaPackage in $cannaSelected){
  $cannaVersion=$cannaPackage.versions[0]
  $cannaVersionName=$cannaVersion.version_number
  $cannaSourceZip=Join-Path $cannaBuild "$($cannaPackage.full_name)-$cannaVersionName.zip"
  $cannaDependencies=@($cannaVersion.dependencies|Where-Object {$_ -notlike 'BepInEx-*'}|ForEach-Object {
   $cannaDependency=$_
   $cannaMatch=$cannaSelected|Where-Object {$cannaDependency -like "$($_.full_name)-*"}|Select-Object -First 1
   if(!$cannaMatch){throw 'Unresolved upstream dependency'}
   $cannaMatch.name
  })
  $cannaDescription="$($cannaVersion.description) Author: $($cannaPackage.owner)."
  if($cannaPackage.name -in @('CustomLocalColorsRedux','ArrowTrajectories')){
   $cannaVersionName+='-canna.1'
   $cannaStage=Join-Path $cannaBuild "release-$($cannaPackage.name)"
   New-Item -ItemType Directory -Path $cannaStage -Force|Out-Null
   Expand-Archive -LiteralPath $cannaSourceZip -DestinationPath $cannaStage -Force
   $cannaModule=if($cannaPackage.name -eq 'CustomLocalColorsRedux'){'SharedColors'}else{'FriendsTrajectories'}
   Copy-Item -LiteralPath (Join-Path $cannaRoot "mods\FamilyVisuals\build\Canna.$cannaModule.dll") -Destination $cannaStage
   Copy-Item -LiteralPath (Join-Path $cannaRoot 'mods\FamilyVisuals\README.md') -Destination (Join-Path $cannaStage 'Canna-README.md')
   $cannaSourceZip=Join-Path $cannaBuild "$($cannaPackage.full_name)-$cannaVersionName.zip"
   Compress-Archive -Path (Join-Path $cannaStage '*') -DestinationPath $cannaSourceZip -Force
   if($cannaModule -eq 'SharedColors'){$cannaDescription+=' Canna extension: F8 color picker; colors shared with modded family lobby members.'}
   else{$cannaDescription+=' Canna extension: F9 own/team/opponent visibility; native arrow prediction works online. Opponent paths default off.'}
  }
  if($cannaPackage.name -eq 'AcidTrip'){$cannaDescription='PHOTOSENSITIVITY WARNING: flashing/changing colors. Leave disabled if sensitive. '+$cannaDescription}
  $cannaPath="Mods/$($cannaPackage.full_name)-$cannaVersionName.zip"
  $cannaEntries+=@{name=$cannaPackage.name;version=$cannaVersionName;description=$cannaDescription;file=$cannaPath;sha256=(Get-FileHash -LiteralPath $cannaSourceZip -Algorithm SHA256).Hash.ToLowerInvariant();dependencies=$cannaDependencies}
  $cannaUploads+=@{path="bopl-battle/$cannaPath";bytes=[IO.File]::ReadAllBytes($cannaSourceZip)}
  $cannaProvenance+=@{name=$cannaPackage.name;author=$cannaPackage.owner;version=$cannaVersion.version_number;source=$cannaPackage.package_url;download=$cannaVersion.download_url;dependencies=$cannaVersion.dependencies;sha256=(Get-FileHash -LiteralPath (Join-Path $cannaBuild "$($cannaPackage.full_name)-$($cannaVersion.version_number).zip")).Hash.ToLowerInvariant()}
 }
 $cannaNames=@($cannaEntries|ForEach-Object name)
 $cannaGame.mods=@($cannaGame.mods|Where-Object {$_.name -notin $cannaNames})+$cannaEntries
 $cannaUploads+=@{path='bopl-battle/Mods/THUNDERSTORE-SOURCES.json';bytes=[Text.Encoding]::UTF8.GetBytes(($cannaProvenance|ConvertTo-Json -Depth 12))}
 $cannaUploads+=@{path='bopl-battle/Mods/Canna-Family-Visuals.md';bytes=[IO.File]::ReadAllBytes((Join-Path $cannaRoot 'mods\FamilyVisuals\README.md'))}
 $cannaUploads+=@{path='bopl-battle/game.json';bytes=[Text.Encoding]::UTF8.GetBytes(($cannaGame|ConvertTo-Json -Depth 25))}
 $cannaTreeEntries=@()
 foreach($cannaUpload in $cannaUploads){
  $cannaBlob=Invoke-CannaApi 'git/blobs' 'POST' @{content=[Convert]::ToBase64String($cannaUpload.bytes);encoding='base64'}
  $cannaTreeEntries+=@{path=$cannaUpload.path;mode='100644';type='blob';sha=$cannaBlob.sha}
 }
 $cannaTree=Invoke-CannaApi 'git/trees' 'POST' @{base_tree=$cannaCommit.tree.sha;tree=$cannaTreeEntries}
 $cannaNew=Invoke-CannaApi 'git/commits' 'POST' @{message='Add 20 requested Bopl mods, dependencies and family online visual extensions';tree=$cannaTree.sha;parents=@($cannaRef.object.sha)}
 $null=Invoke-CannaApi 'git/refs/heads/main' 'PATCH' @{sha=$cannaNew.sha;force=$false}
 $cannaGame|ConvertTo-Json -Depth 25|Set-Content -LiteralPath (Join-Path $cannaBuild 'published-game.json') -Encoding utf8
 "Published $($cannaEntries.Count) catalog packages in commit $($cannaNew.sha)."
}catch{
 'Thunderstore catalog publication failed.'
 if($_.Exception.Response){'HTTP status: '+[int]$_.Exception.Response.StatusCode}
 exit 1
}finally{$cannaToken=$null;$cannaHeaders=$null}
