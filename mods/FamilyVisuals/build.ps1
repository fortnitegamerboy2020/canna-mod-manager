param([string]$GamePath='D:\SteamLibrary\steamapps\common\Bopl Battle')
$ErrorActionPreference='Stop'
$cannaManaged=Join-Path $GamePath 'BoplBattle_Data\Managed'
$cannaReferences=@('mscorlib.dll','netstandard.dll','System.dll','System.Core.dll','Assembly-CSharp.dll','UnityEngine.dll','UnityEngine.CoreModule.dll','UnityEngine.IMGUIModule.dll','Unity.InputSystem.dll','Facepunch.Steamworks.Win64.dll')|ForEach-Object{'/reference:'+(Join-Path $cannaManaged $_)}
$cannaCore=Join-Path $GamePath 'BepInEx\core'
$cannaReferences+=('/reference:'+(Join-Path $cannaCore 'BepInEx.dll'))
$cannaReferences+=('/reference:'+(Join-Path $cannaCore '0Harmony.dll'))
New-Item -ItemType Directory (Join-Path $PSScriptRoot 'build') -Force|Out-Null
foreach($cannaModule in @('SharedColors','FriendsTrajectories')){
    $cannaOut=Join-Path $PSScriptRoot "build\Canna.$cannaModule.dll"
    $cannaSource=Join-Path $PSScriptRoot "$cannaModule.cs"
    & "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /noconfig /nostdlib /target:library /optimize+ @cannaReferences "/out:$cannaOut" $cannaSource
    if($LASTEXITCODE -ne 0){throw "$cannaModule build failed"}
}
'Built shared-color and online-trajectory extensions.'
