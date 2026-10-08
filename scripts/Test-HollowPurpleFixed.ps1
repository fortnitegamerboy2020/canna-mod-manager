param([Parameter(Mandatory)][string]$Original, [Parameter(Mandatory)][string]$Fixed,
    [Parameter(Mandatory)][string]$Cecil, [Parameter(Mandatory)][string]$Managed)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
[Reflection.Assembly]::LoadFrom($Cecil) | Out-Null
function Require($condition, [string]$message) { if (!$condition) { throw $message } }
function Digest([byte[]]$bytes) {
    $h = [Security.Cryptography.SHA256]::Create()
    try { ([BitConverter]::ToString($h.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant() } finally { $h.Dispose() }
}
function Entries([string]$path) {
    $zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path $path))
    $result = @{}
    try {
        foreach ($entry in $zip.Entries) {
            if (!$entry.Name) { continue }
            $s = $entry.Open(); $m = [IO.MemoryStream]::new()
            try { $s.CopyTo($m); $result[$entry.FullName.Replace('\','/')] = $m.ToArray() } finally { $s.Dispose(); $m.Dispose() }
        }
    } finally { $zip.Dispose() }
    return $result
}
function AllTypes($types) { foreach ($t in $types) { $t; AllTypes $t.NestedTypes } }
function Bodies($assembly) {
    @(AllTypes $assembly.MainModule.Types | ForEach-Object { $_.Methods } | Where-Object HasBody | ForEach-Object {
        $_.FullName
        $_.Body.InitLocals; $_.Body.MaxStackSize
        $_.Body.Variables | ForEach-Object { $_.ToString() }
        $_.Body.Instructions | ForEach-Object { $_.ToString() }
        $_.Body.ExceptionHandlers | ForEach-Object { '{0}|{1}|{2}|{3}|{4}|{5}|{6}' -f $_.HandlerType,$_.CatchType,$_.TryStart,$_.TryEnd,$_.HandlerStart,$_.HandlerEnd,$_.FilterStart }
    }) -join "`n"
}
$a = Entries $Original; $b = Entries $Fixed
$dll = 'BepInEx/plugins/HollowPurple/HollowPurple.dll'
foreach ($key in $a.Keys) {
    Require ($b.ContainsKey($key)) ('Missing original file: ' + $key)
    if ($key -notin @($dll, 'manifest.json')) { Require ((Digest $a[$key]) -eq (Digest $b[$key])) ('Original file changed: ' + $key) }
}
Require ($b.Count -eq $a.Count + 1 -and $b.ContainsKey('CANNA-FIX.md')) 'Unexpected package changes.'
$ma = [Text.Encoding]::UTF8.GetString($a['manifest.json']) | ConvertFrom-Json
$mb = [Text.Encoding]::UTF8.GetString($b['manifest.json']) | ConvertFrom-Json
Require ($mb.description -ceq $ma.description) 'Original description changed.'
Require (($mb.dependencies -join '|') -ceq ($ma.dependencies -join '|')) 'Original dependency pins changed.'
Require ($mb.name -eq 'HollowPurple_Fixed' -and $mb.version_number -eq '1.8.1' -and ($mb.authors -join '+') -eq 'flofl+Canna') 'Fork identity/credits invalid.'
$resolver = [Mono.Cecil.DefaultAssemblyResolver]::new(); $resolver.AddSearchDirectory($Managed)
$reader = [Mono.Cecil.ReaderParameters]::new(); $reader.AssemblyResolver = $resolver
$sa = [IO.MemoryStream]::new([byte[]]$a[$dll]); $sb = [IO.MemoryStream]::new([byte[]]$b[$dll])
$aa = [Mono.Cecil.AssemblyDefinition]::ReadAssembly($sa); $ab = [Mono.Cecil.AssemblyDefinition]::ReadAssembly($sb, $reader)
try {
    Require ((Bodies $aa) -ceq (Bodies $ab)) 'Gameplay method bodies or exception handlers changed.'
    $originalAttributes = @(AllTypes $aa.MainModule.Types | ForEach-Object { $_.CustomAttributes } | Where-Object { $_.AttributeType.FullName -ne 'BepInEx.BepInPlugin' } | ForEach-Object { $_.AttributeType.FullName + ':' + (Digest $_.GetBlob()) }) -join '|'
    $fixedAttributes = @(AllTypes $ab.MainModule.Types | ForEach-Object { $_.CustomAttributes } | Where-Object { $_.AttributeType.FullName -ne 'BepInEx.BepInPlugin' } | ForEach-Object { $_.AttributeType.FullName + ':' + (Digest $_.GetBlob()) }) -join '|'
    Require ($originalAttributes -ceq $fixedAttributes) 'Dependency or other type attributes changed.'
    $originalTypes = @($aa.MainModule.GetTypeReferences()); $fixedTypes = @($ab.MainModule.GetTypeReferences())
    Require ($originalTypes.Count -eq $fixedTypes.Count) 'Type reference count changed.'
    foreach ($t in $originalTypes) {
        $v = @($fixedTypes | Where-Object FullName -eq $t.FullName)
        Require ($v.Count -eq 1) ('Type reference identity changed: ' + $t.FullName)
        $expectedScope = if ($t.FullName -eq 'UnityEngine.Input') { 'UnityEngine' } else { $t.Scope.Name }
        Require ($v[0].Scope.Name -eq $expectedScope) ('Unexpected scope change: ' + $t.FullName)
    }
    foreach ($t in $fixedTypes | Where-Object Namespace -eq 'UnityEngine') { Require ($null -ne $t.Resolve()) ('Unity type did not resolve: ' + $t.FullName) }
    foreach ($m in $ab.MainModule.GetMemberReferences() | Where-Object { $_.DeclaringType.FullName -eq 'UnityEngine.Input' }) { Require ($null -ne $m.Resolve()) ('Input member did not resolve: ' + $m.FullName) }
    Require ($aa.MainModule.Resources.Count -eq $ab.MainModule.Resources.Count) 'Resource count changed.'
    foreach ($r in $aa.MainModule.Resources) {
        $fixedResource = @($ab.MainModule.Resources | Where-Object Name -eq $r.Name)
        Require ($fixedResource.Count -eq 1 -and (Digest $r.GetResourceData()) -eq (Digest $fixedResource[0].GetResourceData())) ('Embedded resource changed: ' + $r.Name)
    }
    $plugin = @($ab.MainModule.Types | ForEach-Object { $_.CustomAttributes } | Where-Object { $_.AttributeType.FullName -eq 'BepInEx.BepInPlugin' })
    Require ($plugin.Count -eq 1 -and $plugin[0].ConstructorArguments[0].Value -eq 'fr.flofl.rounds.hollowpurple' -and $plugin[0].ConstructorArguments[1].Value -eq 'HollowPurple Fixed' -and $plugin[0].ConstructorArguments[2].Value -eq '1.8.1') 'Plugin identity incorrect.'
    Write-Output 'PASS: corrected Unity/Input resolution; unchanged IL, resources, logo, description, dependency pins and licenses; fork credits and identity verified.'
} finally { $aa.Dispose(); $ab.Dispose(); $sa.Dispose(); $sb.Dispose(); $resolver.Dispose() }
