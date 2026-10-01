; Inno Setup script for PingAgent. Built in CI with:
;   ISCC.exe /DMyAppVersion=<version> installer\PingAgent.iss
; Produces installer\Output\PingAgent-Setup.exe

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#define MyAppName "PingAgent"
#define MyAppExe "PingAgent.exe"
#define MyRunKey "Software\Microsoft\Windows\CurrentVersion\Run"

[Setup]
AppId={{B7F1C2E4-5D3A-4F6B-9C8D-2E1F0A3B4C5D}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher=BeardyWalrus
AppPublisherURL=https://github.com/BeardyWalrus/ping-agent
AppSupportURL=https://github.com/BeardyWalrus/ping-agent/issues
AppUpdatesURL=https://github.com/BeardyWalrus/ping-agent/actions?query=branch%3Amain
DefaultDirName={localappdata}\Programs\{#MyAppName}
DisableDirPage=auto
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=Output
OutputBaseFilename=PingAgent-Setup
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
; We stop the running copy ourselves (see [Code]); the Restart Manager can't
; see a tray-only app reliably.
CloseApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start {#MyAppName} when I sign in to Windows"; GroupDescription: "Startup:"

[Files]
Source: "..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"

[Registry]
; Same value the app's own "Start with Windows" checkbox manages.
Root: HKCU; Subkey: "{#MyRunKey}"; ValueType: string; ValueName: "{#MyAppName}"; ValueData: """{app}\{#MyAppExe}"""; Tasks: autostart

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "Launch {#MyAppName} now"; Flags: nowait postinstall skipifsilent

[Code]
procedure StopRunningApp;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#MyAppExe}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Sleep(500);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningApp;
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopRunningApp;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    { Remove the start-with-Windows entry whether the installer or the app created it.
      The settings file under %APPDATA%\PingAgent is deliberately left in place. }
    RegDeleteValue(HKEY_CURRENT_USER, '{#MyRunKey}', '{#MyAppName}');
  end;
end;
