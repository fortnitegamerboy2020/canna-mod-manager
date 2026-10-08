param([string]$Game = 'D:\SteamLibrary\steamapps\common\ROUNDS')
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$managed = Join-Path $Game 'ROUNDS_Data/Managed'
$core = Join-Path $Game 'BepInEx/core'
$archive = Join-Path $root 'target/hollow-original.zip'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne '27fcd1c99fb30b24fea84b730d046445c06889ab78b45d5dc46164fcb34a531b') { throw 'Unexpected upstream archive.' }
$build = Join-Path $root 'mods/HollowPurpleFixed/public-build'
[IO.Directory]::CreateDirectory($build) | Out-Null
$stage = Join-Path $build ('stage-' + [guid]::NewGuid().ToString('N'))
Expand-Archive -LiteralPath $archive -DestinationPath $stage
$pluginDirectory = Join-Path $stage 'BepInEx/plugins/HollowPurple'
$shim = Join-Path $pluginDirectory 'HollowPurple.PublicCompatibility.dll'
$rsp = @('/nologo', '/target:library', '/langversion:5', '/nostdlib+', ('/out:"' + $shim + '"'))
$rsp += Get-ChildItem -LiteralPath $managed -Filter '*.dll' | ForEach-Object { '/reference:"' + $_.FullName + '"' }
$rsp += @('/reference:"' + (Join-Path $core 'BepInEx.dll') + '"', '/reference:"' + (Join-Path $core '0Harmony.dll') + '"', '"' + (Join-Path $root 'mods/HollowPurpleFixed/PublicCompatibility.cs') + '"')
$response = Join-Path $build 'public-shim.rsp'
[IO.File]::WriteAllLines($response, $rsp, [Text.UTF8Encoding]::new($false))
& 'C:/Windows/Microsoft.NET/Framework64/v4.0.30319/csc.exe' /noconfig ('@' + $response)
if ($LASTEXITCODE -ne 0) { throw 'Public adapter compilation failed.' }
[Reflection.Assembly]::LoadFrom((Join-Path $core 'Mono.Cecil.dll')) | Out-Null
$resolver = [Mono.Cecil.DefaultAssemblyResolver]::new()
foreach ($dir in @($managed, $core, $pluginDirectory)) { $resolver.AddSearchDirectory($dir) }
$reader = [Mono.Cecil.ReaderParameters]::new(); $reader.AssemblyResolver = $resolver
$gameAssembly = [Mono.Cecil.AssemblyDefinition]::ReadAssembly((Join-Path $managed 'Assembly-CSharp.dll'))
$shimAssembly = [Mono.Cecil.AssemblyDefinition]::ReadAssembly($shim)
$dll = Join-Path $pluginDirectory 'HollowPurple.dll'
$assembly = [Mono.Cecil.AssemblyDefinition]::ReadAssembly($dll, $reader)
function Types($types) { foreach ($t in $types) { $t; Types $t.NestedTypes } }
try {
    $module = $assembly.MainModule
    $unity = [Mono.Cecil.AssemblyNameReference]::new('UnityEngine', [Version]'0.0.0.0')
    $module.AssemblyReferences.Add($unity)
    $input = @($module.GetTypeReferences() | Where-Object FullName -eq 'UnityEngine.Input')
    if ($input.Count -ne 1) { throw 'Unexpected Input reference.' }
    $input[0].Scope = $unity
    $pipeline = @($module.GetTypeReferences() | Where-Object FullName -eq 'UnityEngine.Experimental.Rendering.RenderPipelineAsset')
    if ($pipeline.Count -ne 1) { throw 'Unexpected legacy render pipeline reference.' }
    $pipeline[0].Namespace = 'UnityEngine.Rendering'
    $unbound = @($module.AssemblyReferences | Where-Object Name -eq UnboundLib)
    if ($unbound.Count -ne 1) { throw 'Unexpected dependency reference.' }
    $unbound[0].Name = 'HollowPurple.PublicCompatibility'; $unbound[0].Version = [Version]'0.0.0.0'
    $native = $shimAssembly.MainModule.Types | Where-Object FullName -eq 'Canna.HollowPurplePublic.Native'
    $wrappers = @{}
    foreach ($name in @('Damage', 'Rpc', 'CreatePlayer', 'Category', 'SmokePick')) { $wrappers[$name] = $module.ImportReference(($native.Methods | Where-Object Name -eq $name)) }
    $replacementGetters = @{}
    foreach ($entry in @(@('Player','playerID','get_PlayerID'), @('Player','teamID','get_TeamID'), @('CharacterData','maxHealth','get_MaxHealth'), @('CardInfo','cardName','get_CardName'))) {
        $type = $gameAssembly.MainModule.Types | Where-Object Name -eq $entry[0]
        $replacementGetters[$entry[0] + '::' + $entry[1]] = $module.ImportReference(($type.Methods | Where-Object Name -eq $entry[2]))
    }
    $dataType = $gameAssembly.MainModule.Types | Where-Object Name -eq CharacterData
    $setHealth = $module.ImportReference(($dataType.Methods | Where-Object Name -eq set_MaxHealth))
    $playerType = $gameAssembly.MainModule.Types | Where-Object Name -eq Player
    $setTeam = $module.ImportReference(($playerType.Methods | Where-Object Name -eq AssignTeamID))
    $changes = @{}
    foreach ($type in (Types $module.Types)) {
        foreach ($field in $type.Fields | Where-Object { $_.HasConstant -and $_.Constant -eq '1.8.0' }) { $field.Constant = '1.8.2' }
        foreach ($attribute in @($type.CustomAttributes | Where-Object { $_.AttributeType.FullName -eq 'BepInEx.BepInDependency' })) { $type.CustomAttributes.Remove($attribute) | Out-Null }
        foreach ($attribute in $type.CustomAttributes | Where-Object { $_.AttributeType.FullName -eq 'BepInEx.BepInPlugin' }) {
            $attribute.ConstructorArguments[1] = [Mono.Cecil.CustomAttributeArgument]::new($module.TypeSystem.String, 'HollowPurple Fixed')
            $attribute.ConstructorArguments[2] = [Mono.Cecil.CustomAttributeArgument]::new($module.TypeSystem.String, '1.8.2')
        }
        foreach ($method in $type.Methods | Where-Object HasBody) {
            foreach ($instruction in @($method.Body.Instructions)) {
                $operand = $instruction.Operand
                if ($operand -is [Mono.Cecil.FieldReference]) {
                    $key = $operand.DeclaringType.Name + '::' + $operand.Name
                    if ($replacementGetters.ContainsKey($key)) {
                        if ($instruction.OpCode.Name -eq 'ldfld') { $instruction.OpCode = [Mono.Cecil.Cil.OpCodes]::Callvirt; $instruction.Operand = $replacementGetters[$key] }
                        elseif ($instruction.OpCode.Name -eq 'ldflda') {
                            # Old code takes an address to format value fields. Public IDs
                            # are properties: take the address of a copied getter result.
                            if ($instruction.Next.OpCode.Name -notmatch '^call|^constrained\.') { throw ('Unexpected mutable field address: ' + $key) }
                            $getter = $replacementGetters[$key]
                            $local = [Mono.Cecil.Cil.VariableDefinition]::new($getter.ReturnType)
                            $method.Body.Variables.Add($local); $method.Body.InitLocals = $true
                            $instruction.OpCode = [Mono.Cecil.Cil.OpCodes]::Callvirt; $instruction.Operand = $getter
                            $il = $method.Body.GetILProcessor()
                            $store = $il.Create([Mono.Cecil.Cil.OpCodes]::Stloc, $local)
                            $il.InsertAfter($instruction, $store)
                            $il.InsertAfter($store, $il.Create([Mono.Cecil.Cil.OpCodes]::Ldloca, $local))
                        }
                        elseif ($instruction.OpCode.Name -eq 'stfld' -and $key -eq 'CharacterData::maxHealth') { $instruction.OpCode = [Mono.Cecil.Cil.OpCodes]::Callvirt; $instruction.Operand = $setHealth }
                        elseif ($instruction.OpCode.Name -eq 'stfld' -and $key -eq 'Player::teamID' -and $type.FullName.StartsWith('HollowPurple.Diagnostics.')) { $instruction.OpCode = [Mono.Cecil.Cil.OpCodes]::Callvirt; $instruction.Operand = $setTeam }
                        else { throw ('Unsupported old field operation: ' + $key + ' ' + $instruction.OpCode.Name + ' in ' + $method.FullName) }
                        $changes[$key] = 1 + $changes[$key]
                    }
                } elseif ($operand -is [Mono.Cecil.MethodReference]) {
                    $wrapper = $null
                    if ($operand.DeclaringType.Name -eq 'Damagable' -and $operand.Name -eq 'TakeDamage' -and $operand.Parameters.Count -eq 6) { $wrapper = 'Damage' }
                    elseif ($operand.DeclaringType.Name -eq 'PlayerAssigner' -and $operand.Name -eq 'CreatePlayer') { $wrapper = 'CreatePlayer' }
                    elseif ($operand.DeclaringType.Name -eq 'CardCategory' -and $operand.Name -eq '.ctor' -and $instruction.OpCode.Name -eq 'newobj') { $wrapper = 'Category' }
                    elseif ($operand.DeclaringType.Name -eq 'ApplyCardStats' -and $operand.Name -eq 'OFFLINE_Pick' -and $type.FullName.StartsWith('HollowPurple.Diagnostics.')) { $wrapper = 'SmokePick' }
                    elseif ($operand.DeclaringType.FullName -eq 'Photon.Pun.PhotonView' -and $operand.Name -eq 'RPC' -and $operand.Parameters.Count -eq 3 -and $operand.Parameters[1].ParameterType.Name -eq 'RpcTarget') { $wrapper = 'Rpc' }
                    if ($wrapper) { $instruction.OpCode = [Mono.Cecil.Cil.OpCodes]::Call; $instruction.Operand = $wrappers[$wrapper]; $changes[$wrapper] = 1 + $changes[$wrapper] }
                } elseif ($instruction.OpCode.Name -eq 'ldstr') {
                    if ($operand -eq '1.8.0') { $instruction.Operand = '1.8.2' }
                    elseif ($operand -eq 'fr.flofl.rounds.hollowpurple/protocol/1') { $instruction.Operand = 'fr.flofl.rounds.hollowpurple/canna-public/1' }
                    elseif ($operand -eq 'Hollow Purple 1.8.0 / Unity ') { $instruction.Operand = 'HollowPurple Fixed 1.8.2 public / flofl + Canna / Unity ' }
                }
            }
        }
    }
    if ($changes['SmokePick'] -ne 9) { throw 'Unexpected diagnostic card-pick coverage; review upstream lifecycle before packaging.' }
    $assembly.Write($dll + '.public')
    Write-Output ($changes | ConvertTo-Json -Compress)
} finally { $assembly.Dispose(); $gameAssembly.Dispose(); $shimAssembly.Dispose(); $resolver.Dispose() }
Move-Item -LiteralPath ($dll + '.public') -Destination $dll -Force
$manifestPath = Join-Path $stage 'manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$manifest.name = 'HollowPurple_Fixed'; $manifest.version_number = '1.8.2'
$manifest.website_url = 'https://thunderstore.io/c/rounds/p/flofl/HollowPurple/'
$manifest.dependencies = @('BepInEx-BepInExPack_ROUNDS-5.4.1900')
$manifest | Add-Member authors @('flofl','Canna')
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
# This staging directory is a development artifact until runtime tests pass.
Write-Output ('Public development package staged: ' + $stage)
