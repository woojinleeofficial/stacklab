; StackLab installer (Inno Setup 6). Per-user install, no administrator rights needed.
; Build:  ISCC.exe /DAppVersion=0.9.0 installer\stacklab.iss   (output: target\installer\)
; Silent: StackLab-Setup-<ver>.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART
; User data (%APPDATA%\StackLab: records, backups, performance logs) is kept on uninstall.

#ifndef AppVersion
  #define AppVersion "0.9.0"
#endif

[Setup]
AppId={{6F0C2B5E-7A41-4E1B-9C35-3B7D5A0E51A9}
AppName=StackLab
AppVersion={#AppVersion}
AppPublisher=StackLab
AppComments=Offline tetromino stacking trainer
DefaultDirName={localappdata}\Programs\StackLab
DefaultGroupName=StackLab
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\target\installer
OutputBaseFilename=StackLab-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\stacklab.exe
UninstallDisplayName=StackLab

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\target\release\stacklab.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\docs\USER_GUIDE.md"; DestDir: "{app}\docs"; DestName: "USER_GUIDE.txt"; Flags: ignoreversion
Source: "..\THIRD_PARTY_LICENSES.md"; DestDir: "{app}\docs"; DestName: "THIRD_PARTY_LICENSES.txt"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}\docs"; DestName: "README.txt"; Flags: ignoreversion

[Icons]
Name: "{group}\StackLab"; Filename: "{app}\stacklab.exe"
Name: "{group}\StackLab User Guide"; Filename: "{app}\docs\USER_GUIDE.txt"
Name: "{group}\{cm:UninstallProgram,StackLab}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\StackLab"; Filename: "{app}\stacklab.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\stacklab.exe"; Description: "{cm:LaunchProgram,StackLab}"; Flags: nowait postinstall skipifsilent
