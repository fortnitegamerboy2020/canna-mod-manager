param([string]$TokenFile = '')
$ErrorActionPreference='Stop'
$cannaRoot=Split-Path $PSScriptRoot -Parent
if(!$TokenFile){$TokenFile=Join-Path $cannaRoot 'canna-token.txt'}
$cannaKey=[IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
$cannaHeaders=@{Authorization="Bearer $cannaKey";'User-Agent'='Canna-Website-Catalog';Accept='application/vnd.github+json'}
$cannaApi='https://api.github.com/repos/fortnitegamerboy2020/manager-uploaded-mods/contents'
$cannaStaging=Join-Path $cannaRoot 'server\staging'
New-Item -ItemType Directory -Path $cannaStaging -Force | Out-Null
try {
    $cannaRootEntries=Invoke-RestMethod "$cannaApi/?ref=main" -Headers $cannaHeaders
    $cannaCatalog=@{games=@($cannaRootEntries | Where-Object {$_.type -eq 'dir'} | ForEach-Object {$_.name})}
    $cannaEntries=@()
    $cannaAssets=@()
    foreach($cannaGamePath in $cannaCatalog.games){
        $cannaGameFile=Invoke-RestMethod "$cannaApi/$cannaGamePath/game.json?ref=main" -Headers $cannaHeaders
        $cannaGame=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($cannaGameFile.content)) | ConvertFrom-Json
        foreach($cannaAsset in @(@{path=$cannaGame.icon;kind='icon'},@{path='Framework/BepInEx.zip';kind='framework'})){
            if(!$cannaAsset.path){continue}
            $cannaLocal=Join-Path $cannaStaging "$($cannaGame.app_id)-$([IO.Path]::GetFileName($cannaAsset.path))"
            $cannaRawHeaders=@{Authorization="Bearer $cannaKey";'User-Agent'='Canna';Accept='application/vnd.github.raw+json'}
            Invoke-WebRequest "$cannaApi/$cannaGamePath/$($cannaAsset.path)?ref=main" -Headers $cannaRawHeaders -OutFile $cannaLocal
            $cannaAssets+=@{alias="$cannaGamePath/$($cannaAsset.path)";game_id=$cannaGame.app_id;kind=$cannaAsset.kind;local_file=[IO.Path]::GetFileName($cannaLocal);sha256=(Get-FileHash $cannaLocal -Algorithm SHA256).Hash.ToLowerInvariant()}
            if($cannaAsset.kind -eq 'icon'){$cannaAssets+=@{alias="$cannaGamePath/icon.png";game_id=$cannaGame.app_id;kind='icon';local_file=[IO.Path]::GetFileName($cannaLocal);sha256=(Get-FileHash $cannaLocal -Algorithm SHA256).Hash.ToLowerInvariant()}}
        }
        foreach($cannaMod in $cannaGame.mods){
            if($cannaMod.file -notmatch '^Mods/[A-Za-z0-9_./-]+\.zip$' -or $cannaMod.file.Contains('..')){throw 'Invalid catalog path'}
            $cannaRelative="$cannaGamePath/$($cannaMod.file)"
            $cannaLocal=Join-Path $cannaStaging "$($cannaGame.app_id)-$([IO.Path]::GetFileName($cannaMod.file))"
            $cannaRawHeaders=@{Authorization="Bearer $cannaKey";'User-Agent'='Canna-Website-Catalog';Accept='application/vnd.github.raw+json'}
            Invoke-WebRequest "$cannaApi/$cannaRelative`?ref=main" -Headers $cannaRawHeaders -OutFile $cannaLocal
            $cannaHash=(Get-FileHash -LiteralPath $cannaLocal -Algorithm SHA256).Hash.ToLowerInvariant()
            if($cannaHash -ne $cannaMod.sha256){throw "Checksum mismatch: $($cannaMod.name)"}
            $cannaEntries+=@{app_id=$cannaGame.app_id;name=$cannaMod.name;version=$cannaMod.version;description=$cannaMod.description;sha256=$cannaHash;local_file=[IO.Path]::GetFileName($cannaLocal);origin="catalog:$cannaRelative`:$cannaHash";details=@{game=$cannaGame.name;provider='catalog';folder=$cannaGamePath;catalog_file=$cannaRelative;filename=[IO.Path]::GetFileName($cannaMod.file);dependencies=@($cannaMod.dependencies);source_url="https://github.com/fortnitegamerboy2020/manager-uploaded-mods/tree/main/$cannaRelative";loaders=@('BepInEx')}}
            Write-Output "Staged $($cannaMod.name) $($cannaMod.version)"
        }
        # Keep historical archives addressable for existing modpacks with pinned versions.
        $cannaArchives=Invoke-RestMethod "$cannaApi/$cannaGamePath/Mods?ref=main" -Headers $cannaHeaders
        foreach($cannaArchive in $cannaArchives){
            if($cannaArchive.type -ne 'file' -or $cannaArchive.name -notmatch '^[A-Za-z0-9_.+-]+\.zip$'){continue}
            $cannaRelative="$cannaGamePath/Mods/$($cannaArchive.name)"
            if($cannaEntries.details.catalog_file -contains $cannaRelative){continue}
            $cannaLocal=Join-Path $cannaStaging "$($cannaGame.app_id)-$($cannaArchive.name)"
            Invoke-WebRequest "$cannaApi/$cannaRelative`?ref=main" -Headers $cannaRawHeaders -OutFile $cannaLocal
            $cannaAssets+=@{alias=$cannaRelative;game_id=$cannaGame.app_id;kind='archive';local_file=[IO.Path]::GetFileName($cannaLocal);sha256=(Get-FileHash $cannaLocal -Algorithm SHA256).Hash.ToLowerInvariant()}
            Write-Output "Preserved historical archive $($cannaArchive.name)"
        }
    }
    [IO.File]::WriteAllText((Join-Path $cannaStaging 'catalog-import.json'),($cannaEntries|ConvertTo-Json -Depth 15),[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $cannaStaging 'assets-import.json'),($cannaAssets|ConvertTo-Json -Depth 15),[Text.UTF8Encoding]::new($false))
    Write-Output "Staged $($cannaEntries.Count) catalog mods and $($cannaAssets.Count) assets."
} finally {$cannaKey=$null;$cannaHeaders=$null;$cannaRawHeaders=$null}
