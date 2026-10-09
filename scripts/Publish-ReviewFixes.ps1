param(
    [Parameter(Mandatory=$true)][string]$SourceRoot,
    [string]$TokenFile='C:\Users\t_tra\Downloads\chatgpttoken_canna_mod_manager.txt'
)
$ErrorActionPreference='Stop'
$cannaReviewToken=$null
$cannaReviewHeaders=$null
try {
    $cannaReviewStage=[IO.Path]::GetFullPath($SourceRoot)
    $cannaReviewToken=[IO.File]::ReadAllText($TokenFile).Trim().TrimStart([char]0xFEFF).Trim()
    $cannaReviewHeaders=@{Authorization="Bearer $cannaReviewToken";'User-Agent'='Canna-Review-Publisher';Accept='application/vnd.github+json'}
    $cannaReviewApi='https://api.github.com/repos/fortnitegamerboy2020/canna-mod-manager'
    function Invoke-CannaReviewApi([string]$Path,[string]$Method='GET',$Body=$null) {
        $cannaReviewRequest=@{Uri="$cannaReviewApi/$Path".TrimEnd('/');Headers=$cannaReviewHeaders;Method=$Method;TimeoutSec=90}
        if($null-ne$Body){$cannaReviewRequest.Body=$Body|ConvertTo-Json -Depth 20 -Compress;$cannaReviewRequest.ContentType='application/json'}
        Invoke-RestMethod @cannaReviewRequest
    }
    # Only this release's review implementation and regression/documentation
    # files. Unrelated desktop/mod work and private target artifacts are excluded.
    $cannaReviewFiles=@(
        'server/Cargo.toml','server/Cargo.lock','server/README.md',
        'server/src/scans.rs','server/src/catalog.rs',
        'server/web/review.js','server/web/review.html','server/web/help.html',
        'server/deploy/review-worker.py','server/deploy/review_context.py','server/deploy/review_trace.py','server/deploy/test-review-trace.py',
        'server/deploy/test-review-context.py','server/deploy/test-review-packing.py',
        'server/deploy/test-review-adversarial.py','server/deploy/test-review-coverage.py','server/deploy/test-review-offline.py','server/deploy/test-review-live.py','server/deploy/test-review-permissions.py',
        'scripts/Replay-ReviewSources.py','scripts/Test-ModReview.cjs','scripts/Test-ReviewWorkspace.cjs','scripts/Test-ReviewTrace.cjs','scripts/Test-ReviewWorker.py',
        'scripts/Test-Workflows.ps1','scripts/WORKFLOW-TESTS.md','scripts/Publish-ReviewFixes.ps1'
    )
    foreach($cannaReviewFile in $cannaReviewFiles){if(!(Test-Path -LiteralPath (Join-Path $cannaReviewStage $cannaReviewFile))){throw 'Staged publication file missing'}}
    if([IO.File]::ReadAllText((Join-Path $cannaReviewStage 'server/Cargo.toml'))-notmatch '(?m)^version = "0\.3\.57"\r?$'){throw 'Unexpected staged server version'}
    $cannaReviewRepo=Invoke-CannaReviewApi ''
    $cannaReviewBranch=$cannaReviewRepo.default_branch
    $cannaReviewRef=Invoke-CannaReviewApi "git/ref/heads/$cannaReviewBranch"
    $cannaReviewParent=Invoke-CannaReviewApi "git/commits/$($cannaReviewRef.object.sha)"
    $cannaReviewEntries=@()
    foreach($cannaReviewFile in $cannaReviewFiles){
        $cannaReviewBytes=[IO.File]::ReadAllBytes((Join-Path $cannaReviewStage $cannaReviewFile))
        $cannaReviewBlob=Invoke-CannaReviewApi 'git/blobs' 'POST' @{content=[Convert]::ToBase64String($cannaReviewBytes);encoding='base64'}
        $cannaReviewEntries+=@{path=$cannaReviewFile;mode='100644';type='blob';sha=$cannaReviewBlob.sha}
    }
    $cannaReviewTree=Invoke-CannaReviewApi 'git/trees' 'POST' @{base_tree=$cannaReviewParent.tree.sha;tree=$cannaReviewEntries}
    $cannaReviewCommit=Invoke-CannaReviewApi 'git/commits' 'POST' @{message='Server 0.3.57: contextual mod review and false-positive corrections';tree=$cannaReviewTree.sha;parents=@($cannaReviewRef.object.sha)}
    # Compare-and-swap: a concurrent publication must not be overwritten.
    $cannaReviewLatest=Invoke-CannaReviewApi "git/ref/heads/$cannaReviewBranch"
    if($cannaReviewLatest.object.sha-ne$cannaReviewRef.object.sha){throw 'Source branch advanced during publication; retry against its new head'}
    $null=Invoke-CannaReviewApi "git/refs/heads/$cannaReviewBranch" 'PATCH' @{sha=$cannaReviewCommit.sha;force=$false}
    $cannaReviewVerified=Invoke-CannaReviewApi "git/ref/heads/$cannaReviewBranch"
    if($cannaReviewVerified.object.sha-ne$cannaReviewCommit.sha){throw 'Source ref verification failed'}
    Write-Output ("Published review source commit "+$cannaReviewCommit.sha+" ("+$cannaReviewFiles.Count+" bounded files).")
} catch {
    Write-Output 'Review source publication failed; credentials and response bodies are omitted.'
    if($_.Exception.Response){Write-Output ('HTTP status: '+[int]$_.Exception.Response.StatusCode)}
    exit 1
} finally {$cannaReviewToken=$null;$cannaReviewHeaders=$null}
