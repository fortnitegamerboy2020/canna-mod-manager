# Builds the ordinary Beta-gated desktop. Support belongs on the authenticated
# server; the desktop release never embeds a compatibility payload.
param([string]$Support = '', [string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$previousLocalPreview = $env:CANNA_REBOUND_LOCAL_PREVIEW
try {
    if ($Support) { Write-Output 'The legacy -Support argument is not embedded. Deploy reviewed support through the protected Beta server endpoint separately.' }
    $env:CANNA_REBOUND_LOCAL_PREVIEW = '0'
    & (Join-Path $root 'build.ps1') -Target $Target
    Write-Output 'Rebound requires server-verified Beta access before the desktop downloads support.'
} finally {
    $env:CANNA_REBOUND_LOCAL_PREVIEW = $previousLocalPreview
    Pop-Location
}
