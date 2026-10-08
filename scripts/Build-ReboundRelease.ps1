param([string]$Support = 'target/ducttape-plus-plus/support.zip', [string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$previousSupport = $env:CANNA_DUCTTAPE_SUPPORT
$previousLocalPreview = $env:CANNA_REBOUND_LOCAL_PREVIEW
try {
    $supportPath = [IO.Path]::GetFullPath($Support)
    if (!(Test-Path -LiteralPath $supportPath -PathType Leaf)) { throw 'Build and verify Canna Rebound support first.' }
    $env:CANNA_DUCTTAPE_SUPPORT = $supportPath
    $env:CANNA_REBOUND_LOCAL_PREVIEW = '0'
    & (Join-Path $root 'build.ps1') -Target $Target
} finally {
    $env:CANNA_DUCTTAPE_SUPPORT = $previousSupport
    $env:CANNA_REBOUND_LOCAL_PREVIEW = $previousLocalPreview
    Pop-Location
}
