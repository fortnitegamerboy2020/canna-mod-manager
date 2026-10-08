param([string]$Stage, [string]$Game='D:\SteamLibrary\steamapps\common\ROUNDS')
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
if (!$Stage) { $Stage=(Get-ChildItem (Join-Path $root 'mods/HollowPurpleFixed/public-build') -Directory -Filter 'stage-*' | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName }
$original=Join-Path $root 'target/hollow-original'
function Check($ok,$message) { if (!$ok) { throw $message }; Write-Output ('PASS '+$message) }
$manifest=Get-Content -LiteralPath (Join-Path $Stage 'manifest.json') -Raw | ConvertFrom-Json
$upstream=Get-Content -LiteralPath (Join-Path $original 'manifest.json') -Raw | ConvertFrom-Json
Check ($manifest.description -ceq $upstream.description) 'Original description preserved'
foreach ($asset in @('icon.png','LICENSE','docs/ASSETS-LICENSE.md')) { Check ((Get-FileHash -LiteralPath (Join-Path $Stage $asset)).Hash -eq (Get-FileHash -LiteralPath (Join-Path $original $asset)).Hash) ('Original '+$asset+' preserved') }
Check ($manifest.dependencies.Count -eq 1 -and $manifest.dependencies[0] -eq 'BepInEx-BepInExPack_ROUNDS-5.4.1900') 'No legacy UnboundLib/MMHook dependency'
[Reflection.Assembly]::LoadFrom((Join-Path $Game 'BepInEx/core/Mono.Cecil.dll'))|Out-Null
$resolver=[Mono.Cecil.DefaultAssemblyResolver]::new()
foreach($dir in @((Join-Path $Game 'ROUNDS_Data/Managed'),(Join-Path $Game 'BepInEx/core'),(Join-Path $Stage 'BepInEx/plugins/HollowPurple'))) {$resolver.AddSearchDirectory($dir)}
$parameters=[Mono.Cecil.ReaderParameters]::new();$parameters.AssemblyResolver=$resolver
$failures=[Collections.Generic.List[string]]::new();$checked=0
try {
    foreach($file in Get-ChildItem (Join-Path $Stage 'BepInEx/plugins/HollowPurple') -Filter '*.dll') {
        $assembly=[Mono.Cecil.AssemblyDefinition]::ReadAssembly($file.FullName,$parameters)
        try {
            Check (@($assembly.MainModule.AssemblyReferences | Where-Object Name -match '^UnboundLib$|^MMHook').Count -eq 0) ($file.Name+' excludes old dependency assemblies')
            foreach($type in $assembly.MainModule.GetTypeReferences()) {
                try { if(!$type.Resolve()){$failures.Add('Missing type '+$type.FullName)} } catch {$failures.Add('Unresolved type '+$type.FullName)}
                $checked++
            }
            foreach($member in $assembly.MainModule.GetMemberReferences()) {
                try { $definition=$member.Resolve(); if(!$definition){$failures.Add('Missing member '+$member.FullName)} elseif($definition.IsPrivate -and $definition.DeclaringType.Module.Assembly.Name.Name -ne $assembly.Name.Name){$failures.Add('Inaccessible member '+$member.FullName)} } catch {$failures.Add('Unresolved member '+$member.FullName)}
                $checked++
            }
        } finally {$assembly.Dispose()}
    }
    Check ($failures.Count -eq 0) ('Public runtime metadata resolves '+$checked+' external type/member references: '+($failures -join '; '))
} finally {$resolver.Dispose()}
Write-Output 'Static checks do not establish gameplay or multiplayer compatibility; run the public-game diagnostics before publishing.'
