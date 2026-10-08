param([string]$TokenFile = 'C:\Users\t_tra\Downloads\chatgpttoken_mods.txt')
$ErrorActionPreference = 'Stop'
throw 'HollowPurple Fixed 1.8.1 was withdrawn: it targets legacy ROUNDS. Use the separately tested public-port release workflow.'
$root = Split-Path $PSScriptRoot -Parent
$archive = Join-Path $root 'mods/HollowPurpleFixed/build/HollowPurple-Fixed-1.8.1.zip'
$sha = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
$stage = Join-Path $root 'target/hollow-fixed-publish'
[IO.Directory]::CreateDirectory($stage) | Out-Null
$name = 'HollowPurple-Fixed-1.8.1.zip'
Copy-Item -LiteralPath $archive -Destination (Join-Path $stage $name) -Force
$original = [IO.File]::ReadAllText((Join-Path $root 'target/hollow-original/manifest.json')) | ConvertFrom-Json
$details = @{
    game='ROUNDS';folder='rounds';provider='canna';content_type='mod';authors='flofl + Canna';
    author_links=@(@{name='flofl';url='https://thunderstore.io/c/rounds/p/flofl/'},@{name='Canna';url='https://cannamods.vip/help'});
    source_url='https://thunderstore.io/c/rounds/p/flofl/HollowPurple/';
    icon_url='https://ccdn.thunderstore.io/live/repository/icons/flofl-HollowPurple-1.8.0.png';
    original_project='flofl-HollowPurple';original_version='1.8.0';original_sha256='27fcd1c99fb30b24fea84b730d046445c06889ab78b45d5dc46164fcb34a531b';
    id='canna-HollowPurpleFixed';filename=$name;catalog_file="rounds/Mods/$name";
    license='MIT (code); CC0 (original procedural assets)';loaders=@('BepInEx');
    dependencies=@($original.dependencies);
    dependency_ids=@('f04675bc-cdf7-4c41-9505-4c64c9d4f9fc','2afd9103-0cc4-4df1-b940-b9ced317e62f','00664687-81e6-4ded-98d6-abe3419a7a60');
    required_game_branch='old-rounds-for-mods';game_versions=@('Old ROUNDS for mods (old-rounds-for-mods)');
    install_notes='Canna fork of flofl HollowPurple 1.8.0: corrects UnityEngine.Input assembly reference. Steam ROUNDS > Properties > Betas > Old ROUNDS for mods is required. Disable/remove original HollowPurple first; both share the same plugin identity and DLL path. All players need this fork version. Static checks passed; live gameplay/multiplayer verification pending. Original opt-in diagnostic output can overwrite files in its selected directory.';
    manual_review_required=$true
}
$entry=@{app_id=1557740;name='HollowPurple Fixed';version='1.8.1';description=$original.description;sha256=$sha;local_file=$name;origin="canna:hollowpurple-fixed:1.8.1:$sha";details=$details}
[IO.File]::WriteAllText((Join-Path $stage 'catalog-import.json'),(ConvertTo-Json -InputObject @($entry) -Depth 12),[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $stage 'assets-import.json'),'[]',[Text.UTF8Encoding]::new($false))
$token = [IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
$headers=@{Authorization="Bearer $token";'User-Agent'='Canna-Mod-Manager';Accept='application/vnd.github+json'}
$api='https://api.github.com/repos/fortnitegamerboy2020/manager-uploaded-mods'
function Request([string]$path,[string]$method='GET',$body=$null) {
    $args=@{Uri="$api/$path";Method=$method;Headers=$headers;TimeoutSec=90}
    if($null-ne $body){$args.Body=$body|ConvertTo-Json -Depth 20 -Compress;$args.ContentType='application/json'}
    Invoke-RestMethod @args
}
try {
    $ref=Request 'git/ref/heads/main';$commit=Request "git/commits/$($ref.object.sha)"
    # The authenticated Canna server is the catalog authority for this fork and its
    # reviewed dependency IDs. Mirror its pinned archive without replacing game lists.
    $uploads=@(
        @{path="rounds/Mods/$name";bytes=[IO.File]::ReadAllBytes($archive)},
        @{path='rounds/Mods/HollowPurple-Fixed-CANNA-FIX.md';bytes=[IO.File]::ReadAllBytes((Join-Path $root 'mods/HollowPurpleFixed/CANNA-FIX.md'))}
    )
    $entries=@()
    foreach($upload in $uploads){$blob=Request 'git/blobs' 'POST' @{content=[Convert]::ToBase64String($upload.bytes);encoding='base64'};$entries+=@{path=$upload.path;mode='100644';type='blob';sha=$blob.sha}}
    $tree=Request 'git/trees' 'POST' @{base_tree=$commit.tree.sha;tree=$entries}
    $new=Request 'git/commits' 'POST' @{message='Publish HollowPurple Fixed 1.8.1: flofl original, Canna Input module fix, preserved assets and branch requirements';tree=$tree.sha;parents=@($ref.object.sha)}
    $null=Request 'git/refs/heads/main' 'PATCH' @{sha=$new.sha;force=$false}
    Write-Output ('Published separate catalog fork at '+$new.sha+'; server staging ready at '+$stage)
} finally {$token=$null;$headers=$null}
