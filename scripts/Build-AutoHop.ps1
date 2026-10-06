param([string]$Python = "python")
$ErrorActionPreference = 'Stop'
$cannaRoot = Split-Path $PSScriptRoot -Parent
$cannaVsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$cannaVs = & $cannaVsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$cannaVs) { throw 'Visual Studio C++ x86 tools are required.' }
$cannaEnvironment = Join-Path $cannaVs 'VC\Auxiliary\Build\vcvarsall.bat'
Push-Location $cannaRoot
try {
    New-Item -ItemType Directory -Force target | Out-Null
    & cmd /c "`"$cannaEnvironment`" x86 && cl /nologo /W4 /WX /O2 /MT /LD mods\CannaAutoHop\native.cpp /Fo:target\autohop-native.obj /link /OUT:target\canna_autohop.dll /IMPLIB:target\autohop-native.lib shell32.lib"
    if ($LASTEXITCODE) { throw 'Native Auto-Hop build failed.' }
    & $Python scripts/Build-AutoHop.py
    if ($LASTEXITCODE) { throw 'Auto-Hop packaging failed.' }
} finally { Pop-Location }
