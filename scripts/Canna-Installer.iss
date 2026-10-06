; Build with scripts/Build-Installer.ps1. Packages the existing updater-enabled EXE.
#ifndef AppVersion
 #error AppVersion must be passed by the build script
#endif
#ifdef TestInstall
 #define AppLabel "Canna Installer Test"
 #define SetupId "{{113D89D9-EFDC-4080-A492-818557807011}"
 #define OutputName "Canna-Installer-Test"
#else
 #define AppLabel "Canna Mod Manager"
 #define SetupId "{{AA7CF289-FF5B-4AF9-9AD5-50B6AB8A1FA7}"
 #define OutputName "Canna-Setup-" + AppVersion
#endif
[Setup]
AppId={#SetupId}
AppName={#AppLabel}
AppVersion={#AppVersion}
AppPublisher=Canna
AppPublisherURL=https://cannamods.vip
AppSupportURL=https://cannamods.vip/help
AppUpdatesURL=https://cannamods.vip/updates/latest
DefaultDirName={localappdata}\Programs\Canna Mod Manager
DefaultGroupName=Canna Mod Manager
PrivilegesRequired=lowest
DisableDirPage=yes
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist
OutputBaseFilename={#OutputName}
SetupIconFile=..\src\assets\canna.ico
UninstallDisplayIcon={app}\Canna Mod Manager.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
LicenseFile=..\LICENSE
[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
[Files]
Source: "..\dist\Canna Mod Manager.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "Canna-UninstallCleanup.ps1"; DestDir: "{app}"; Flags: ignoreversion
[Icons]
Name: "{userprograms}\{#AppLabel}"; Filename: "{app}\Canna Mod Manager.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\{#AppLabel}"; Filename: "{app}\Canna Mod Manager.exe"; WorkingDir: "{app}"; Tasks: desktopicon
[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\{#AppLabel}.exe"; ValueType: string; ValueData: "{app}\Canna Mod Manager.exe"; Flags: uninsdeletekey
[UninstallDelete]
Type: filesandordirs; Name: "{app}"
[Run]
Filename: "{app}\Canna Mod Manager.exe"; Description: "Open Canna Mod Manager"; Flags: nowait postinstall skipifsilent

[Code]
function InitializeUninstall(): Boolean;
begin
  Result := ExtractFileName(RemoveBackslash(ExpandConstant('{app}'))) = 'Canna Mod Manager';
  if not Result then
    MsgBox('Uninstall refused: the installation folder is not a dedicated Canna Mod Manager folder.', mbError, MB_OK)
  else if not UninstallSilent then
    Result := MsgBox('Uninstall Canna and permanently delete its saved logins, settings, downloads, modpacks, skins, Minecraft instances and worlds? This cannot be undone.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Params: String;
  ResultCode: Integer;
begin
  if CurUninstallStep = usUninstall then
  begin
#ifdef TestInstall
    Params := ExpandConstant('-RoamingBase "{app}\..\..\cleanup-fixture\Roaming" -LocalBase "{app}\..\..\cleanup-fixture\Local" -TempBase "{app}\..\..\cleanup-fixture\Temp"');
#else
    Params := ExpandConstant('-RoamingBase "{userappdata}" -LocalBase "{localappdata}" -TempBase "{%TEMP}"');
#endif
    Params := ExpandConstant('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{app}\Canna-UninstallCleanup.ps1" ') + Params;
    if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'), Params, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
      RaiseException('Unable to run Canna data cleanup. Uninstall has stopped.');
    if ResultCode <> 0 then
      RaiseException('Some Canna data could not be removed. Close Canna and its Minecraft instances, then retry uninstall.');
  end;
end;
