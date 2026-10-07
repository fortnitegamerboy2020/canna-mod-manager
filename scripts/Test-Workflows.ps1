param(
    [string]$Node = 'node',
    [string]$Python = 'python',
    [string]$PlaywrightModule = '',
    [switch]$Packaging,
    [string]$Server = '',
    [string]$Identity = "$env:USERPROFILE/.ssh/canna_server_ed25519"
)
$ErrorActionPreference = 'Stop'
$cannaRoot = Split-Path $PSScriptRoot -Parent
$cannaResults = [Collections.Generic.List[object]]::new()
function Invoke-CannaCheck([string]$Name, [scriptblock]$Work) {
    $cannaStarted = Get-Date
    Write-Output "Checking $Name"
    & $Work
    if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit $LASTEXITCODE" }
    $cannaResults.Add(@{check=$Name;status='passed';seconds=[Math]::Round(((Get-Date)-$cannaStarted).TotalSeconds,2)})
}
Push-Location $cannaRoot
try {
    if ($PlaywrightModule) {$env:CANNA_PLAYWRIGHT_MODULE=$PlaywrightModule}
    Invoke-CannaCheck 'Desktop tests (isolated fixtures; live tests excluded)' {cargo test --release --locked -- --test-threads=1}
    Invoke-CannaCheck 'Desktop strict lint' {cargo clippy --all-targets --locked -- -D warnings}
    foreach ($cannaFile in Get-ChildItem -LiteralPath (Join-Path $cannaRoot 'scripts') -Filter 'Test-*.cjs' | Sort-Object Name) {
        Invoke-CannaCheck $cannaFile.Name {& $Node $cannaFile.FullName}
    }
    Invoke-CannaCheck 'Review worker archive/packer fixtures' {& $Python scripts/Test-ReviewWorker.py}
    if ($Packaging) {
        Invoke-CannaCheck 'Maintenance replacement/rollback fixtures' {& ./scripts/Test-Maintenance.ps1}
        Invoke-CannaCheck 'Installer install/uninstall fixtures' {& ./scripts/Test-Installer.ps1}
    }
    if ($Server) {
        Invoke-CannaCheck 'Server tests and strict lint (isolated databases)' {
            & ssh -i $Identity -o BatchMode=yes $Server 'cd /home/canna-admin/canna-server && for f in deploy/*.sh; do bash -n "$f" || exit 1; done && /home/canna-admin/.cargo/bin/cargo test --release --locked -- --test-threads=1 && /home/canna-admin/.cargo/bin/cargo clippy --release --all-targets --locked -- -D warnings'
        }
    }
    $cannaReport=Join-Path $cannaRoot 'target/workflow-results.json'
    New-Item -ItemType Directory -Path (Split-Path $cannaReport -Parent) -Force | Out-Null
    @{completed=(Get-Date).ToUniversalTime().ToString('o');checks=$cannaResults;live_game_tests='Not performed by this runner';telemetry='Local report only'} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $cannaReport -Encoding utf8
    Write-Output "Workflow checks passed; local report: $cannaReport"
} finally {Pop-Location}
