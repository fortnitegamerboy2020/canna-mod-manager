param([string]$Target = '')
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
try {
    if ($Target) { cargo build --release --locked --target $Target }
    else { cargo build --release --locked }
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot 'dist') -Force | Out-Null
    $cannaReleasePath = if ($Target) { Join-Path $PSScriptRoot "target\$Target\release\canna-mod-manager.exe" } else { Join-Path $PSScriptRoot 'target\release\canna-mod-manager.exe' }
    Copy-Item -LiteralPath $cannaReleasePath -Destination (Join-Path $PSScriptRoot 'dist\Canna Mod Manager.exe')
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'README.md') -Destination (Join-Path $PSScriptRoot 'dist\README.md')
    New-Item -ItemType Directory -Path (Join-Path $PSScriptRoot 'dist\examples') -Force | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'examples') -File | Copy-Item -Destination (Join-Path $PSScriptRoot 'dist\examples')
    Compress-Archive -Path (Join-Path $PSScriptRoot 'repository-template\*') -DestinationPath (Join-Path $PSScriptRoot 'dist\repository-template.zip') -Force
    Compress-Archive -LiteralPath (Join-Path $PSScriptRoot 'dist\Canna Mod Manager.exe'),(Join-Path $PSScriptRoot 'dist\README.md'),(Join-Path $PSScriptRoot 'dist\repository-template.zip'),(Join-Path $PSScriptRoot 'dist\examples') -DestinationPath (Join-Path $PSScriptRoot 'dist\Canna-Mod-Manager-Windows.zip') -Force
    Write-Output 'Built executable and portable Windows ZIP in dist'
} finally {
    Pop-Location
}
