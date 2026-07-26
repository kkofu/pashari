; pashari インストーラ（Inno Setup 6）。
;
; 管理者権限不要のユーザー単位インストール（PrivilegesRequired=lowest）。
; ポータブル zip 配布はそのまま残し、これは別の配布物として追加する。
;
; ビルド:
;   ISCC.exe /DMyAppVersion=0.5.0 installer\pashari.iss
; （MyAppVersion を渡さない場合は "0.0.0-dev" になる。CI では release.yml が
;  タグ名から算出した値を渡す。）
;
; AppId は絶対に変更しないこと。変更すると「上書きアップグレード」ではなく
; 別アプリとして二重インストールされてしまう。

#define MyAppName "pashari"
#define MyAppPublisher "yozba"
#define MyAppURL "https://github.com/yozba/pashari"
#define MyAppExeName "pashari.exe"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

[Setup]
AppId={{59CEB6BF-D0A0-4B1C-9C5D-56068B5FD365}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
; 管理者権限不要（ユーザーごとのインストール）。管理者が実行した場合や
; コマンドラインからは per-machine も選べるようにしておく。
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline dialog
SetupIconFile=..\assets\icon.ico
LicenseFile=..\LICENSE
Compression=lzma
SolidCompression=yes
WizardStyle=modern
OutputDir=..\dist_installer
OutputBaseFilename=pashari-{#MyAppVersion}-setup
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop icon"; GroupDescription: "Additional icons:"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent
