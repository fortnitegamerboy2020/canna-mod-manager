param([string]$Target = '')
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    if ($Target) { cargo build --release --locked --bin canna-mod-manager --target $Target }
    else { cargo build --release --locked --bin canna-mod-manager }
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot 'dist') -Force | Out-Null
    $cannaTargetRoot = if ($env:CARGO_TARGET_DIR) { [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR) } else { Join-Path $PSScriptRoot 'target' }
    $cannaReleasePath = if ($Target) { Join-Path $cannaTargetRoot "$Target/release/canna-mod-manager.exe" } else { Join-Path $cannaTargetRoot 'release/canna-mod-manager.exe' }
    Copy-Item -LiteralPath $cannaReleasePath -Destination (Join-Path $PSScriptRoot 'dist\Canna Mod Manager.exe')
    & (Join-Path $PSScriptRoot 'scripts/Build-Maintenance.ps1') -Target $Target
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $PSScriptRoot 'dist\README.md')
    New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot 'dist\examples') -Force | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'examples') -File | Copy-Item -Destination (Join-Path $PSScriptRoot 'dist\examples')
    Compress-Archive -Path (Join-Path $PSScriptRoot 'repository-template\*') -DestinationPath (Join-Path $PSScriptRoot 'dist\repository-template.zip') -Force
    Compress-Archive -LiteralPath (Join-Path $PSScriptRoot 'dist\Canna Mod Manager.exe'),(Join-Path $PSScriptRoot 'dist\Canna Updater.exe'),(Join-Path $PSScriptRoot 'dist\README.md'),(Join-Path $PSScriptRoot 'dist\repository-template.zip'),(Join-Path $PSScriptRoot 'dist\examples') -DestinationPath (Join-Path $PSScriptRoot 'dist\Canna-Mod-Manager-Windows.zip') -Force
    Write-Output 'Built executable and portable Windows ZIP in dist'
} finally {
    Pop-Location
}
