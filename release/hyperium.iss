; Inno Setup script for Hyperium.
; Installs per-user (no admin) so in-place self-update can overwrite the exe
; without a UAC prompt on every update, then trusts the app's code-signing
; certificate in the CurrentUser stores so future self-updates verify.

#define MyAppName "Hyperium"
#define MyAppVersion "0.62.1"
#define MyAppPublisher "Stargate Dev"
#define MyAppExeName "hyperium.exe"
#define MyAppCert "stargate-dev.cer"

[Setup]
AppId={{9F2B6C2E-6C7B-4B7B-9C7C-3C6B7C7A9C10}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=.
OutputBaseFilename=HyperiumSetup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "stargate-dev.cer"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\{cm:UninstallProgram,{#MyAppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
; Trust the app's code-signing certificate for this user so self-update
; (Authenticode check in authenticode.rs) succeeds on future versions.
Filename: "certutil.exe"; Parameters: "-addstore -user Root ""{app}\{#MyAppCert}"""; Flags: runhidden; StatusMsg: "Registering security certificate..."
Filename: "certutil.exe"; Parameters: "-addstore -user TrustedPublisher ""{app}\{#MyAppCert}"""; Flags: runhidden; StatusMsg: "Registering security certificate..."
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Best-effort cleanup: remove the trusted certificate on uninstall.
Filename: "certutil.exe"; Parameters: "-delstore -user Root ""Stargate Dev Code Signing"""; Flags: runhidden; RunOnceId: "DelRootCert"
Filename: "certutil.exe"; Parameters: "-delstore -user TrustedPublisher ""Stargate Dev Code Signing"""; Flags: runhidden; RunOnceId: "DelPubCert"
