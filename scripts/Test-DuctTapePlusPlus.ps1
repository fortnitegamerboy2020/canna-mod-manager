param(
    [string]$Support = 'target/ducttape-plus-plus/support.zip',
    [string]$Game = 'D:\SteamLibrary\steamapps\common\ROUNDS'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    $work = Join-Path $root ('target/ducttape-tests-' + [guid]::NewGuid().ToString('N'))
    [IO.Directory]::CreateDirectory($work) | Out-Null
    $bundle = Join-Path $work 'support'
    Expand-Archive -LiteralPath ([IO.Path]::GetFullPath($Support)) -DestinationPath $bundle
    $helper = Join-Path $bundle 'helper/Canna.DuctTapePlusPlus.Translate.exe'
    [Reflection.Assembly]::LoadFrom((Join-Path $bundle 'helper/Mono.Cecil.dll')) | Out-Null
    $script:passed = 0
    function Check([bool]$condition, [string]$message) {
        if (!$condition) { throw ('FAIL: ' + $message) }
        $script:passed++
        Write-Output ('PASS: ' + $message)
    }
    function Fixture([string]$directory, [string]$identity, [string]$field, [string]$assemblyName = 'Assembly-CSharp') {
        [IO.Directory]::CreateDirectory($directory) | Out-Null
        $module = [Mono.Cecil.ModuleDefinition]::CreateModule($identity + '.dll', [Mono.Cecil.ModuleKind]::Dll)
        try {
            $reference = [Mono.Cecil.AssemblyNameReference]::new($assemblyName, [Version]'0.0.0.0')
            $module.AssemblyReferences.Add($reference)
            $player = [Mono.Cecil.TypeReference]::new('', 'Player', $module, $reference)
            $type = [Mono.Cecil.TypeDefinition]::new('Fixtures', 'LegacyCalls', [Mono.Cecil.TypeAttributes]::Public, $module.TypeSystem.Object)
            $module.Types.Add($type)
            $method = [Mono.Cecil.MethodDefinition]::new('Read', ([Mono.Cecil.MethodAttributes]::Public -bor [Mono.Cecil.MethodAttributes]::Static), $module.TypeSystem.Int32)
            $method.Parameters.Add([Mono.Cecil.ParameterDefinition]::new('player', [Mono.Cecil.ParameterAttributes]::None, $player))
            $type.Methods.Add($method)
            $il = $method.Body.GetILProcessor()
            $il.Append($il.Create([Mono.Cecil.Cil.OpCodes]::Ldarg_0))
            $il.Append($il.Create([Mono.Cecil.Cil.OpCodes]::Ldfld, [Mono.Cecil.FieldReference]::new($field, $module.TypeSystem.Int32, $player)))
            $il.Append($il.Create([Mono.Cecil.Cil.OpCodes]::Ret))
            $path = Join-Path $directory ($identity + '.dll')
            $module.Write($path)
            return $path
        } finally { $module.Dispose() }
    }
    function Prepare([string]$name, [string]$plugins, [string]$gameRoot = $Game, [bool]$allowMissingReport = $false) {
        $case = Split-Path $plugins -Parent
        [IO.Directory]::CreateDirectory($case) | Out-Null
        [IO.File]::WriteAllText((Join-Path $plugins '.canna-ducttape-staging'), 'canna.ducttape++/1')
        $request = Join-Path $case ($name + '-request.json')
        $report = Join-Path $case ($name + '-report.json')
        $requestData = @{ game_root = $gameRoot; plugins = $plugins; core = (Join-Path $Game 'BepInEx/core'); declared_dependencies = @() }
        [IO.File]::WriteAllText($request, ($requestData | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
        & $helper --request $request --report $report
        $code = $LASTEXITCODE
        if (!(Test-Path -LiteralPath $report)) {
            if ($allowMissingReport -and $code -ne 0) { return @{ exit = $code; report = @{ ok = $false; errors = @('Request rejected before a writable report destination was established') }; plugins = $plugins } }
            throw ('Missing helper report for ' + $name)
        }
        return @{ exit = $code; report = (Get-Content -LiteralPath $report -Raw | ConvertFrom-Json); plugins = $plugins }
    }
    $first = Join-Path $work 'legacy-one/plugins'
    $source = Fixture $first 'DependencyFreeLegacy' 'playerID'
    $before = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    $one = Prepare 'legacy-one' $first
    Check ($one.exit -eq 0 -and $one.report.ok) 'Dependency-free legacy fixture passes supported translation'
    Check ([bool]$one.report.required) 'Old game calls trigger compatibility without declared dependencies'
    Check ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $before) 'Legacy caller is rewritten in temporary copy'
    $rewritten = [Mono.Cecil.ModuleDefinition]::ReadModule($source)
    try {
        Check (@($rewritten.GetMemberReferences() | Where-Object { $_.DeclaringType.FullName -eq 'Player' -and $_.Name -eq 'playerID' }).Count -eq 0) 'Old playerID field reference removed'
        Check (@($rewritten.GetMemberReferences() | Where-Object { $_.DeclaringType.FullName -eq 'Player' -and $_.Name -eq 'get_PlayerID' }).Count -gt 0) 'Current PlayerID property is used'
    } finally { $rewritten.Dispose() }
    $reapplied = Prepare 'legacy-one-reapply' $first
    Check ($reapplied.exit -eq 0 -and $reapplied.report.ok -and $reapplied.report.fingerprint -eq $one.report.fingerprint) 'Prepared output can be reapplied without duplicate runtime identities or digest drift'
    # Preserve the same input bytes/MVID; local directory names must not affect output.
    $original = Join-Path $work 'original-input/plugins'
    $pristine = Fixture $original 'DependencyFreeLegacy' 'playerID'
    # New clean roots avoid outputs from the first preparation affecting parity.
    $detA = Join-Path $work 'deterministic-a/plugins'
    $detB = Join-Path $work 'arbitrary-pack-name/plugins'
    [IO.Directory]::CreateDirectory($detA) | Out-Null
    [IO.Directory]::CreateDirectory($detB) | Out-Null
    Copy-Item -LiteralPath $pristine -Destination $detA
    Copy-Item -LiteralPath $pristine -Destination $detB
    $a = Prepare 'deterministic-a' $detA
    $b = Prepare 'arbitrary-pack-name' $detB
    Check ($a.report.ok -and $b.report.ok -and $a.report.fingerprint -eq $b.report.fingerprint) 'Equivalent prepared content has matching multiplayer digest across paths'
    $unsupported = Join-Path $work 'unsupported/plugins'
    Fixture $unsupported 'UnsupportedLegacy' 'RemovedUnknownField' | Out-Null
    $bad = Prepare 'unsupported' $unsupported
    Check ($bad.exit -ne 0 -and !$bad.report.ok -and @($bad.report.errors).Count -gt 0) 'Unknown legacy game API blocks preparation with reasons'
    $unresolved = Join-Path $work 'unresolved/plugins'
    Fixture $unresolved 'UnknownDependency' 'playerID' 'Unsupported.Dependency' | Out-Null
    $missing = Prepare 'unresolved' $unresolved
    Check ($missing.exit -ne 0 -and !$missing.report.ok) 'Unresolved required assembly blocks preparation'
    $duplicates = Join-Path $work 'duplicates/plugins'
    $dup = Fixture (Join-Path $duplicates 'one') 'DuplicateIdentity' 'playerID'
    [IO.Directory]::CreateDirectory((Join-Path $duplicates 'two')) | Out-Null
    Copy-Item -LiteralPath $dup -Destination (Join-Path $duplicates 'two/DuplicateIdentity.dll')
    $collision = Prepare 'duplicates' $duplicates
    Check ($collision.exit -ne 0 -and !$collision.report.ok) 'Duplicate managed identity blocks preparation'
    $fakeGame = Join-Path $work 'fake-game'
    $inside = Join-Path $fakeGame 'BepInEx/plugins'
    $never = Fixture $inside 'NeverWriteGame' 'playerID'
    $neverHash = (Get-FileHash -LiteralPath $never -Algorithm SHA256).Hash
    $guarded = Prepare 'game-path-rejected' $inside $fakeGame $true
    Check ($guarded.exit -ne 0 -and !$guarded.report.ok) 'Helper refuses a plugins path inside its declared game root'
    Check ((Get-FileHash -LiteralPath $never -Algorithm SHA256).Hash -eq $neverHash -and !(Test-Path -LiteralPath (Join-Path $inside 'DuctTapePlusPlus'))) 'Rejected game path retains its original content'
    $summary = @{ passed = $script:passed; work = $work; game = $Game; live_gameplay = $false; two_client_multiplayer = $false }
    [IO.File]::WriteAllText((Join-Path $work 'summary.json'), ($summary | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Write-Output ('Offline helper checks passed: ' + $script:passed)
    Write-Output ('Evidence: ' + $work)
} finally { Pop-Location }
