param(
    [string]$Archive = '',
    [string]$Cecil = 'D:\SteamLibrary\steamapps\common\ROUNDS\BepInEx\core\Mono.Cecil.dll',
    [string]$Managed = 'D:\SteamLibrary\steamapps\common\ROUNDS\ROUNDS_Data\Managed'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if (!$Archive) { $Archive = Join-Path $root 'target/hollow-original.zip' }
$expected = '27fcd1c99fb30b24fea84b730d046445c06889ab78b45d5dc46164fcb34a531b'
if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw 'Unexpected upstream archive. This patch applies only to the inspected HollowPurple 1.8.0 release.'
}
$out = Join-Path $root 'mods/HollowPurpleFixed/build'
[IO.Directory]::CreateDirectory($out) | Out-Null
$stage = Join-Path $out ('stage-' + [guid]::NewGuid().ToString('N'))
Expand-Archive -LiteralPath $Archive -DestinationPath $stage
[Reflection.Assembly]::LoadFrom($Cecil) | Out-Null
$dll = Join-Path $stage 'BepInEx/plugins/HollowPurple/HollowPurple.dll'
$original = [Mono.Cecil.AssemblyDefinition]::ReadAssembly($dll)
try {
    if ($original.Name.HasPublicKey) { throw 'Refusing to modify a signed assembly.' }
    $input = @($original.MainModule.GetTypeReferences() | Where-Object FullName -eq 'UnityEngine.Input')
    if ($input.Count -ne 1 -or $input[0].Scope.Name -ne 'UnityEngine.CoreModule') { throw 'Unexpected Input reference.' }
    $scope = [Mono.Cecil.AssemblyNameReference]::new('UnityEngine', [Version]'0.0.0.0')
    $original.MainModule.AssemblyReferences.Add($scope)
    $input[0].Scope = $scope
    $plugins = @($original.MainModule.Types | ForEach-Object { $_.CustomAttributes } | Where-Object { $_.AttributeType.FullName -eq 'BepInEx.BepInPlugin' })
    if ($plugins.Count -ne 1 -or $plugins[0].ConstructorArguments[0].Value -ne 'fr.flofl.rounds.hollowpurple' -or $plugins[0].ConstructorArguments[2].Value -ne '1.8.0') { throw 'Unexpected plugin identity.' }
    $stringType = $original.MainModule.TypeSystem.String
    $plugins[0].ConstructorArguments[1] = [Mono.Cecil.CustomAttributeArgument]::new($stringType, 'HollowPurple Fixed')
    $plugins[0].ConstructorArguments[2] = [Mono.Cecil.CustomAttributeArgument]::new($stringType, '1.8.1')
    $original.Write($dll + '.fixed')
} finally { $original.Dispose() }
Move-Item -LiteralPath ($dll + '.fixed') -Destination $dll -Force
$manifestFile = Join-Path $stage 'manifest.json'
$manifest = [IO.File]::ReadAllText($manifestFile) | ConvertFrom-Json
$manifest.name = 'HollowPurple_Fixed'
$manifest.version_number = '1.8.1'
$manifest.website_url = 'https://thunderstore.io/c/rounds/p/flofl/HollowPurple/'
$manifest | Add-Member authors @('flofl', 'Canna')
[IO.File]::WriteAllText($manifestFile, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
Copy-Item -LiteralPath (Join-Path $root 'mods/HollowPurpleFixed/CANNA-FIX.md') -Destination $stage
$zip = Join-Path $out 'HollowPurple-Fixed-1.8.1.zip'
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
& (Join-Path $PSScriptRoot 'Test-HollowPurpleFixed.ps1') -Original $Archive -Fixed $zip -Cecil $Cecil -Managed $Managed
if (!$?) { throw 'Fixed package verification failed.' }
Write-Output ('Built HollowPurple Fixed 1.8.1: ' + (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant())
