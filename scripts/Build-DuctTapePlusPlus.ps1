param(
    [string]$Game = 'D:\SteamLibrary\steamapps\common\ROUNDS',
    [string]$DotNet = '',
    [string]$Output = 'target/ducttape-plus-plus',
    [string]$UpstreamSource = '',
    [string]$ToolkitSource = ''
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (!$DotNet) {
        $candidate = Join-Path $root 'target/build-tools/dotnet-8.0.425/dotnet.exe'
        if (Test-Path -LiteralPath $candidate) { $DotNet = $candidate }
        else { $DotNet = (Get-Command dotnet -ErrorAction Stop).Source }
    }
    $python = 'C:/Users/t_tra/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe'
    if (!(Test-Path -LiteralPath $python)) { $python = (Get-Command python -ErrorAction Stop).Source }
    $buildArgs = @('mods/DuctTapePlusPlus/build.py', '--dotnet', $DotNet, '--game', $Game, '--output', $Output)
    if ($UpstreamSource) { $buildArgs += @('--upstream-source', $UpstreamSource) }
    if ($ToolkitSource) { $buildArgs += @('--toolkit-source', $ToolkitSource) }
    # The builder reads game assemblies only. It never invokes upstream projects
    # that have copy-to-game postbuild targets or starts a game process.
    & $python @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Canna Bliss build failed.' }
    Write-Output ('Local preview support: ' + [IO.Path]::GetFullPath((Join-Path $Output 'support.zip')))
} finally { Pop-Location }
