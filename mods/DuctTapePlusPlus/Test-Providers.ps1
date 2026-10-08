param(
    [string]$Support = 'target/ducttape-plus-plus/support.zip',
    [string]$Game = 'D:\SteamLibrary\steamapps\common\ROUNDS'
)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$scratch = Join-Path $workspace ('target/rebound-providers-' + [guid]::NewGuid().ToString('N'))
$script:caseNumber = 0
[IO.Directory]::CreateDirectory($scratch) | Out-Null
$bundle = Join-Path $scratch 'support'
Expand-Archive -LiteralPath ([IO.Path]::GetFullPath($Support)) -DestinationPath $bundle
[Reflection.Assembly]::LoadFrom((Join-Path $bundle 'helper/Mono.Cecil.dll')) | Out-Null
$framework = [Mono.Cecil.ModuleDefinition]::ReadModule((Join-Path $Game 'BepInEx/core/BepInEx.dll'))
try {
    $pluginAttribute = $framework.GetType('BepInEx.BepInPlugin').Methods | Where-Object { $_.IsConstructor -and $_.Parameters.Count -eq 3 } | Select-Object -First 1
    $dependencyAttribute = $framework.GetType('BepInEx.BepInDependency').Methods | Where-Object { $_.IsConstructor -and $_.Parameters.Count -eq 2 -and $_.Parameters[1].ParameterType.FullName -eq 'System.String' } | Select-Object -First 1
    if (!$pluginAttribute -or !$dependencyAttribute) { throw 'Unexpected actual BepInEx attribute constructors' }
    function Add-LoaderAttribute($module, $type, [string]$kind, [string]$value, [bool]$foreign) {
        if ($foreign) {
            $base = [Mono.Cecil.TypeReference]::new('System', 'Attribute', $module, $module.TypeSystem.CoreLibrary)
            $fake = [Mono.Cecil.TypeDefinition]::new('BepInEx', $kind, [Mono.Cecil.TypeAttributes]::Public, $base)
            $module.Types.Add($fake)
            $flags = [Mono.Cecil.MethodAttributes]([int][Mono.Cecil.MethodAttributes]::Public -bor [int][Mono.Cecil.MethodAttributes]::HideBySig -bor [int][Mono.Cecil.MethodAttributes]::SpecialName -bor [int][Mono.Cecil.MethodAttributes]::RTSpecialName)
            $constructor = [Mono.Cecil.MethodDefinition]::new('.ctor', $flags, $module.TypeSystem.Void)
            $constructor.Parameters.Add([Mono.Cecil.ParameterDefinition]::new('value', [Mono.Cecil.ParameterAttributes]::None, $module.TypeSystem.String))
            $fake.Methods.Add($constructor)
            $baseConstructor = [Mono.Cecil.MethodReference]::new('.ctor', $module.TypeSystem.Void, $base)
            $baseConstructor.HasThis = $true
            $il = $constructor.Body.GetILProcessor()
            $il.Append($il.Create([Mono.Cecil.Cil.OpCodes]::Ldarg_0))
            $il.Append($il.Create([Mono.Cecil.Cil.OpCodes]::Call, $baseConstructor))
            $il.Append($il.Create([Mono.Cecil.Cil.OpCodes]::Ret))
        } else {
            $definition = $framework.GetType('BepInEx.' + $kind).Methods | Where-Object { $_.IsConstructor -and $_.Parameters.Count -eq 1 } | Select-Object -First 1
            $constructor = $module.ImportReference($definition)
        }
        $attribute = [Mono.Cecil.CustomAttribute]::new($constructor)
        $attribute.ConstructorArguments.Add([Mono.Cecil.CustomAttributeArgument]::new($module.TypeSystem.String, $value))
        $type.CustomAttributes.Add($attribute)
    }
    function New-Plugin([string]$directory, [string]$name, [string]$guid, [bool]$isReal, [bool]$isAbstract, [string]$dependency = '', [string]$minimum = '1.0.0', [string[]]$processFilters = @(), [string]$incompatibility = '', [bool]$foreign = $false, [bool]$inherited = $false) {
        $module = [Mono.Cecil.ModuleDefinition]::CreateModule($name + '.dll', [Mono.Cecil.ModuleKind]::Dll)
        try {
            $base = if ($isReal) { $module.ImportReference($framework.GetType('BepInEx.BaseUnityPlugin')) } else { $module.TypeSystem.Object }
            $attributes = [Mono.Cecil.TypeAttributes]::Public
            if ($isAbstract) { $attributes = $attributes -bor [Mono.Cecil.TypeAttributes]::Abstract }
            $type = [Mono.Cecil.TypeDefinition]::new('Fixtures', $name, $attributes, $base)
            $module.Types.Add($type)
            $plugin = [Mono.Cecil.CustomAttribute]::new($module.ImportReference($pluginAttribute))
            foreach ($value in @($guid, $name, '1.0.0')) {
                $plugin.ConstructorArguments.Add([Mono.Cecil.CustomAttributeArgument]::new($module.TypeSystem.String, $value))
            }
            $type.CustomAttributes.Add($plugin)
            $metadataType = $type
            if ($inherited) {
                $metadataType = [Mono.Cecil.TypeDefinition]::new('Fixtures', 'LoaderMetadataBase', ([Mono.Cecil.TypeAttributes]::Public -bor [Mono.Cecil.TypeAttributes]::Abstract), $base)
                $module.Types.Add($metadataType)
                $type.BaseType = $metadataType
            }
            foreach ($process in $processFilters) { Add-LoaderAttribute $module $metadataType 'BepInProcess' $process $foreign }
            if ($incompatibility) { Add-LoaderAttribute $module $metadataType 'BepInIncompatibility' $incompatibility $foreign }
            if ($dependency) {
                $hard = [Mono.Cecil.CustomAttribute]::new($module.ImportReference($dependencyAttribute))
                foreach ($value in @($dependency, $minimum)) {
                    $hard.ConstructorArguments.Add([Mono.Cecil.CustomAttributeArgument]::new($module.TypeSystem.String, $value))
                }
                $type.CustomAttributes.Add($hard)
            }
            $module.Write((Join-Path $directory ($name + '.dll')))
        } finally { $module.Dispose() }
    }
    function Run-Case([string]$name, [bool]$isReal, [bool]$isAbstract, [string]$minimum, [bool]$expected, [string[]]$processFilters = @(), [string]$incompatibility = '', [bool]$foreign = $false, [bool]$inherited = $false, [string]$expectedReason = '') {
        $script:caseNumber++
        $case = Join-Path $scratch ('c' + $script:caseNumber)
        $plugins = Join-Path $case 'plugins'
        [IO.Directory]::CreateDirectory($plugins) | Out-Null
        New-Plugin $plugins 'Provider' 'fixtures.rebound.provider' $isReal $isAbstract -processFilters $processFilters -incompatibility $incompatibility -foreign $foreign -inherited $inherited
        New-Plugin $plugins 'Consumer' 'fixtures.rebound.consumer' $true $false 'fixtures.rebound.provider' $minimum
        [IO.File]::WriteAllText((Join-Path $plugins '.canna-ducttape-staging'), 'canna.ducttape++/1')
        $before = @{}
        Get-ChildItem -LiteralPath $plugins -File | ForEach-Object { $before[$_.Name] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
        $request = Join-Path $case 'request.json'
        $report = Join-Path $case 'report.json'
        $data = @{ game_root = $Game; core = (Join-Path $Game 'BepInEx/core'); plugins = $plugins; declared_dependencies = @() }
        [IO.File]::WriteAllText($request, ($data | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
        & (Join-Path $bundle 'helper/Canna.DuctTapePlusPlus.Translate.exe') --request $request --report $report
        $exitCode = $LASTEXITCODE
        if (!(Test-Path -LiteralPath $report)) { throw ('Missing authorized report: ' + $name) }
        $result = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
        if ([bool]$result.ok -ne $expected -or (($exitCode -eq 0) -ne $expected)) { throw ('Provider case unexpected outcome: ' + $name) }
        if (!$expected) {
            foreach ($file in Get-ChildItem -LiteralPath $plugins -File) {
                if (!$before.ContainsKey($file.Name) -or (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne $before[$file.Name]) {
                    throw ('Rejected provider case changed original temporary input: ' + $name)
                }
            }
            if (@($result.errors).Count -eq 0) { throw ('Rejected provider case had no reason: ' + $name) }
            if ($expectedReason -and !(($result.errors -join "`n").Contains($expectedReason))) { throw ('Wrong rejection reason: ' + $name) }
        }
        Write-Output ('PASS: ' + $name)
    }
    Run-Case 'actual-concrete-provider' $true $false '1.0.0' $true
    Run-Case 'attributed-inert-class-cannot-satisfy-hard-guid' $false $false '1.0.0' $false
    Run-Case 'abstract-plugin-cannot-satisfy-hard-guid' $true $true '1.0.0' $false
    Run-Case 'insufficient-provider-version-blocks' $true $false '2.0.0' $false
    Run-Case 'rounds-exe-process-filter-loads' $true $false '1.0.0' $true -processFilters @('ROUNDS.exe')
    Run-Case 'rounds-process-casefold-loads' $true $false '1.0.0' $true -processFilters @('rounds')
    Run-Case 'any-matching-process-filter-loads' $true $false '1.0.0' $true -processFilters @('OtherGame.exe', 'rOuNdS.exe')
    Run-Case 'wrong-process-cannot-provide-hard-guid' $true $false '1.0.0' $false -processFilters @('OtherGame.exe') -expectedReason 'process filters do not permit'
    Run-Case 'uppercase-extension-matches-installed-loader-rejection' $true $false '1.0.0' $false -processFilters @('ROUNDS.EXE') -expectedReason 'process filters do not permit'
    Run-Case 'actual-incompatibility-with-required-guard-blocks' $true $false '1.0.0' $false -incompatibility 'canna.ducttapeplusplus.networkguard' -expectedReason 'Incompatible actual plugin providers'
    Run-Case 'absent-incompatible-provider-does-not-block' $true $false '1.0.0' $true -incompatibility 'fixtures.not.installed'
    Run-Case 'foreign-incompatibility-cannot-spoof-loader-metadata' $true $false '1.0.0' $false -incompatibility 'canna.ducttapeplusplus.networkguard' -foreign $true -expectedReason 'foreign-scope loader metadata'
    Run-Case 'foreign-process-cannot-spoof-loader-metadata' $true $false '1.0.0' $false -processFilters @('ROUNDS.exe') -foreign $true -expectedReason 'foreign-scope loader metadata'
    Run-Case 'inherited-process-filter-loads' $true $false '1.0.0' $true -processFilters @('ROUNDS.exe') -inherited $true
    Run-Case 'inherited-wrong-process-blocks' $true $false '1.0.0' $false -processFilters @('OtherGame.exe') -inherited $true -expectedReason 'process filters do not permit'
    Run-Case 'inherited-incompatibility-blocks' $true $false '1.0.0' $false -incompatibility 'canna.ducttapeplusplus.networkguard' -inherited $true -expectedReason 'Incompatible actual plugin providers'
} finally { $framework.Dispose() }
