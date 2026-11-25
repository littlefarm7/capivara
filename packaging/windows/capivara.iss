; Instalador do Capivara para Windows (Inno Setup 6.3+ / 7).
; Compile com:  ISCC.exe /DAppVersion=0.1.0 /DSourceExe=..\..\target\release\capivara.exe packaging\windows\capivara.iss
; (o packaging\windows\build.ps1 faz isso sozinho)

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\capivara.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{6F2B7C1E-3A4D-4C59-9E61-CA91A7A0B0DE}
AppName=Capivara
AppVersion={#AppVersion}
AppVerName=Capivara {#AppVersion}
AppPublisher=Capivara
AppPublisherURL=https://github.com/
AppComments=Baixa vídeos, áudios e transcreve — YouTube, Shorts, TikTok, Reels, Stories e +1800 sites
DefaultDirName={autopf}\Capivara
DefaultGroupName=Capivara
DisableProgramGroupPage=yes
; Instala só para o usuário (sem UAC); quem quiser pode escolher "todos os usuários".
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Capivara-{#AppVersion}-windows-x64-setup
SetupIconFile=capivara.ico
UninstallDisplayIcon={app}\capivara.exe
UninstallDisplayName=Capivara
WizardStyle=modern
WizardSmallImageFile=capivara-512.png
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "ptbr"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
ptbr.Engines=Baixar os motores agora (yt-dlp, ffmpeg, deno · ~250 MB)
en.Engines=Download engines now (yt-dlp, ffmpeg, deno · ~250 MB)
ptbr.Model=Baixar o modelo de transcrição "base" (~290 MB)
en.Model=Download the "base" transcription model (~290 MB)
ptbr.Extras=Downloads opcionais (também dá para fazer depois, dentro do app):
en.Extras=Optional downloads (can also be done later inside the app):
ptbr.Downloading=Baixando motores e modelo… isso pode levar alguns minutos.
en.Downloading=Downloading engines and model… this may take a few minutes.
ptbr.RemoveData=Remover também os motores, modelos e configurações baixados pelo Capivara?%n%n(Seus vídeos baixados NÃO serão apagados.)
en.RemoveData=Also remove engines, models and settings downloaded by Capivara?%n%n(Your downloaded videos will NOT be deleted.)
ptbr.Launch=Abrir o Capivara
en.Launch=Launch Capivara

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "engines"; Description: "{cm:Engines}"; GroupDescription: "{cm:Extras}"
Name: "model"; Description: "{cm:Model}"; GroupDescription: "{cm:Extras}"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "capivara.exe"; Flags: ignoreversion
Source: "capivara.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme skipifsourcedoesntexist

[Icons]
Name: "{autoprograms}\Capivara"; Filename: "{app}\capivara.exe"; IconFilename: "{app}\capivara.ico"; Comment: "Baixa vídeos, áudios e transcreve"
Name: "{autodesktop}\Capivara"; Filename: "{app}\capivara.exe"; IconFilename: "{app}\capivara.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\capivara.exe"; Parameters: "setup --model base"; StatusMsg: "{cm:Downloading}"; Flags: runhidden waituntilterminated; Tasks: engines and model
Filename: "{app}\capivara.exe"; Parameters: "setup --model none"; StatusMsg: "{cm:Downloading}"; Flags: runhidden waituntilterminated; Tasks: engines and not model
Filename: "{app}\capivara.exe"; Parameters: "setup --model base --no-engines"; StatusMsg: "{cm:Downloading}"; Flags: runhidden waituntilterminated; Tasks: model and not engines
Filename: "{app}\capivara.exe"; Description: "{cm:Launch}"; Flags: nowait postinstall skipifsilent

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{localappdata}\Capivara');
    if DirExists(DataDir) and (not UninstallSilent) then
      if MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(DataDir, True, True, True);
  end;
end;
