; The Windows installer (Inno Setup 6): taiko-rs-windows-x64-setup.exe, built by CI next to the zip.
;
;   iscc windows\installer.iss [/DAppVersion=0.1.0]
;
; after `cargo build --release`. Installs for the current user, without admin rights, to the same
; place as install.ps1 (%LOCALAPPDATA%\Programs\taiko-rs), so either installs over the other and
; the game's self-updater can swap its exe in place.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

[Setup]
; Never change: Windows knows the installed game (and its uninstaller) by it.
AppId={{6B0E4C3A-2F1D-4E7B-9A55-7C3D1E8F2A61}
AppName=taiko.rs
AppVersion={#AppVersion}
AppPublisher=Lynerrr
AppPublisherURL=https://github.com/Lynerrr/taiko-rs
AppSupportURL=https://github.com/Lynerrr/taiko-rs/issues
; Paths below are relative to the repository root.
SourceDir=..
OutputDir=.
OutputBaseFilename=taiko-rs-windows-x64-setup
SetupIconFile=assets\taiko-rs.ico
LicenseFile=LICENSE
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; {autopf} is %LOCALAPPDATA%\Programs for a per-user install.
DefaultDirName={autopf}\taiko-rs
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\taiko-rs.exe
UninstallDisplayName=taiko.rs
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; The exe can't be replaced while the game is running.
CloseApplications=force
RestartApplications=no

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "target\release\taiko-rs.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "assets\taiko-rs.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "THIRD-PARTY.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The same names install.ps1 gives its shortcuts, so they're replaced rather than doubled.
Name: "{autoprograms}\taiko-rs"; Filename: "{app}\taiko-rs.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\taiko-rs"; Filename: "{app}\taiko-rs.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
; taiko-rs:// links (a score's "Watch replay" button) open the game, as the game itself also
; registers them when it starts (src/link.rs).
Root: HKCU; Subkey: "Software\Classes\taiko-rs"; ValueType: string; ValueName: ""; ValueData: "URL:taiko.rs"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\taiko-rs"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\taiko-rs\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\taiko-rs.exe"",0"
Root: HKCU; Subkey: "Software\Classes\taiko-rs\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\taiko-rs.exe"" ""%1"""

[Run]
Filename: "{app}\taiko-rs.exe"; Description: "{cm:LaunchProgram,taiko.rs}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; What the self-updater leaves next to the exe (src/update.rs). Settings, scores and songs in
; %APPDATA%\taiko-rs stay.
Type: files; Name: "{app}\taiko-rs.exe.old"
Type: files; Name: "{app}\taiko-rs.exe.new"
Type: files; Name: "{app}\taiko-rs.exe.download"
Type: dirifempty; Name: "{app}"
