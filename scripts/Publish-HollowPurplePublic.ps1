param([string]$TokenFile='C:\Users\t_tra\Downloads\chatgpttoken_mods.txt')
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$archive=Join-Path $root 'mods/HollowPurpleFixed/build/HollowPurple-Fixed-1.8.2.zip'
$evidence=Get-Content -LiteralPath (Join-Path $root 'mods/HollowPurpleFixed/build/public-evidence.json') -Raw|ConvertFrom-Json
$sha=(Get-FileHash -LiteralPath $archive).Hash.ToLowerInvariant()
if($evidence.archive_sha256-ne$sha-or$evidence.runtime_checks-ne70-or$evidence.adapter_checks-ne4){throw 'Release archive does not match runtime-tested package evidence.'}
$stage=Join-Path $root 'target/hollow-public-publish';[IO.Directory]::CreateDirectory($stage)|Out-Null
$name='HollowPurple-Fixed-1.8.2.zip';Copy-Item -LiteralPath $archive -Destination (Join-Path $stage $name) -Force
$original=Get-Content -LiteralPath (Join-Path $root 'target/hollow-original/manifest.json') -Raw|ConvertFrom-Json
$details=@{
    game='ROUNDS';folder='rounds';provider='canna';content_type='mod';authors='flofl + Canna';
    author_links=@(@{name='flofl';url='https://thunderstore.io/c/rounds/p/flofl/'},@{name='Canna';url='https://cannamods.vip/help'});
    source_url='https://thunderstore.io/c/rounds/p/flofl/HollowPurple/';
    icon_url='https://ccdn.thunderstore.io/live/repository/icons/flofl-HollowPurple-1.8.0.png';
    original_project='flofl-HollowPurple';original_version='1.8.0';original_sha256='27fcd1c99fb30b24fea84b730d046445c06889ab78b45d5dc46164fcb34a531b';
    id='canna-HollowPurpleFixed';filename=$name;catalog_file="rounds/Mods/$name";
    license='MIT (code); CC0 (original procedural assets)';loaders=@('BepInEx');
    dependencies=@('BepInEx-BepInExPack_ROUNDS-5.4.1900');dependency_ids=@('f04675bc-cdf7-4c41-9505-4c64c9d4f9fc');
    required_game_branch='public';game_versions=@('Public ROUNDS build 21020021 / Unity 2022.3.34');
    install_notes='Default-public ROUNDS preview; Steam Betas > None. Use a separate public pack and Canna 0.2.38 or later. Disable original HollowPurple. Do not include legacy UnboundLib 3.2.14/MMHook 1.0.0. Own narrow adapter replaces those dependencies for this mod only. All 70 automated local/offline and four scene/cleanup checks passed. Earlier runs included Card Control 1.1.2; the final run used this port and a test-only check plugin. User observed the final test looked fine; complete matches and two-client multiplayer remain unverified. One native audio shutdown exception was logged. All players would need this port version. Opt-in --hp-smoke diagnostics can overwrite files in their selected output directory.';
    runtime_verification=@{public_build='21020021';local_checks_passed=70;adapter_checks_passed=4;multiplayer_verified=$false};
    manual_review_required=$true
}
$entry=@{app_id=1557740;name='HollowPurple Fixed';version='1.8.2';description=$original.description;sha256=$sha;local_file=$name;origin="canna:hollowpurple-fixed:1.8.2:$sha";details=$details}
[IO.File]::WriteAllText((Join-Path $stage 'catalog-import.json'),(ConvertTo-Json -InputObject @($entry) -Depth 12),[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $stage 'assets-import.json'),'[]',[Text.UTF8Encoding]::new($false))
$token=[IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
$headers=@{Authorization="Bearer $token";'User-Agent'='Canna-Mod-Manager';Accept='application/vnd.github+json'}
$api='https://api.github.com/repos/fortnitegamerboy2020/manager-uploaded-mods'
function Request([string]$path,[string]$method='GET',$body=$null){
    $args=@{Uri="$api/$path";Method=$method;Headers=$headers;TimeoutSec=90}
    if($null-ne$body){$args.Body=$body|ConvertTo-Json -Depth 20 -Compress;$args.ContentType='application/json'}
    Invoke-RestMethod @args
}
try{
    $ref=Request 'git/ref/heads/main';$commit=Request "git/commits/$($ref.object.sha)"
    $entries=@()
    foreach($upload in @(@{path="rounds/Mods/$name";bytes=[IO.File]::ReadAllBytes($archive)},@{path='rounds/Mods/HollowPurple-Fixed-CANNA-FIX.md';bytes=[IO.File]::ReadAllBytes((Join-Path $root 'mods/HollowPurpleFixed/CANNA-FIX.md'))})){
        $blob=Request 'git/blobs' 'POST' @{content=[Convert]::ToBase64String($upload.bytes);encoding='base64'}
        $entries+=@{path=$upload.path;mode='100644';type='blob';sha=$blob.sha}
    }
    $tree=Request 'git/trees' 'POST' @{base_tree=$commit.tree.sha;tree=$entries}
    $new=Request 'git/commits' 'POST' @{message='HollowPurple Fixed 1.8.2: flofl + Canna public ROUNDS port; preserved assets; 70 local runtime checks';tree=$tree.sha;parents=@($ref.object.sha)}
    $null=Request 'git/refs/heads/main' 'PATCH' @{sha=$new.sha;force=$false}
    Write-Output ('Public archive mirrored at '+$new.sha+'; server import and manual review still required.')
}finally{$token=$null;$headers=$null}
