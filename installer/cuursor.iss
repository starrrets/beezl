; cuursor installer (Inno Setup 6). Built in CI:
;   iscc /DAppVersion=0.2.0 installer\cuursor.iss
; Output: target\installer\cuursor-setup.exe

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F1C2B7E-3D4A-4C8B-9E21-7A5D0C3B9F14}
AppName=cuursor
AppVersion={#AppVersion}
AppVerName=cuursor {#AppVersion}
AppPublisher=Eugene Staroselets
AppPublisherURL=https://github.com/starrrets/cuursor
DefaultDirName={autopf}\cuursor
DisableProgramGroupPage=yes
DisableDirPage=yes
DisableReadyPage=yes
DisableWelcomePage=no
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.15063
OutputDir=..\target\installer
OutputBaseFilename=cuursor-setup
SetupIconFile=..\assets\cuursor.ico
UninstallDisplayIcon={app}\cuursor.exe
UninstallDisplayName=cuursor
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
WizardSizePercent=100
CloseApplications=force
RestartApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
ru.LaunchNow=Запустить cuursor
en.LaunchNow=Launch cuursor

[Files]
Source: "..\target\release\cuursor.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\cuursor-ui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; Portable copy from v0.1.
Type: files; Name: "{localappdata}\cuursor\cuursor.exe"

[Icons]
Name: "{autoprograms}\cuursor"; Filename: "{app}\cuursor.exe"

[Run]
; Logon task that starts cuursor elevated (so it also works over admin windows).
Filename: "{app}\cuursor.exe"; Parameters: "--register"; Flags: runhidden waituntilterminated
Filename: "{app}\cuursor.exe"; Description: "{cm:LaunchNow}"; Flags: nowait postinstall

[UninstallRun]
Filename: "{app}\cuursor.exe"; Parameters: "--unregister"; Flags: runhidden waituntilterminated; RunOnceId: "Unregister"

[Code]
procedure StopRunning();
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im cuursor.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im cuursor-ui.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunning();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopRunning();
  Result := True;
end;
