# Builds an isolated-title desktop with automatic updates disabled. The app still
# verifies Beta access and downloads protected support; it has no embedded DLLs.
# An explicit -Support is available only to cfg(test) fixture code.
param([string]$Support = '')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$previousSupport = $env:CANNA_DUCTTAPE_SUPPORT
$previousTarget = $env:CARGO_TARGET_DIR
$previousLocalPreview = $env:CANNA_REBOUND_LOCAL_PREVIEW
try {
    if ($Support) {
        $supportPath = [IO.Path]::GetFullPath($Support)
        if (!(Test-Path -LiteralPath $supportPath -PathType Leaf)) { throw 'The explicit private support fixture is missing.' }
        $env:CANNA_DUCTTAPE_SUPPORT = $supportPath
    } else {
        Remove-Item Env:CANNA_DUCTTAPE_SUPPORT -ErrorAction SilentlyContinue
    }
    $env:CANNA_REBOUND_LOCAL_PREVIEW = '1'
    $env:CARGO_TARGET_DIR = Join-Path $root 'target/ducttape-native-preview'
    cargo build --release --locked --bin canna-mod-manager
    if ($LASTEXITCODE -ne 0) { throw 'Native compatibility preview build failed.' }
    $previewDir = Join-Path $root 'target/ducttape-plus-plus/desktop-preview'
    [IO.Directory]::CreateDirectory($previewDir) | Out-Null
    Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'release/canna-mod-manager.exe') -Destination (Join-Path $previewDir 'Canna Rebound Preview.exe') -Force
    Write-Output ('Built local preview: ' + $previewDir)
    Write-Output 'The preview EXE has no embedded support. Sign in with server-verified Beta access to use Rebound.'
    if ($Support) { Write-Output 'The explicit support fixture can be read by test executables only; it cannot authorize this app.' }
    Write-Output 'This command does not install, launch, or publish the preview.'
} finally {
    $env:CANNA_DUCTTAPE_SUPPORT = $previousSupport
    $env:CARGO_TARGET_DIR = $previousTarget
    $env:CANNA_REBOUND_LOCAL_PREVIEW = $previousLocalPreview
    Pop-Location
}
