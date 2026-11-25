# Gera os pacotes do Windows em dist\:
#   Capivara-<v>-windows-x64-setup.exe     instalador (leve; baixa os motores durante a instalação, opcional)
#   Capivara-<v>-windows-x64-portable.zip  versão portátil (motores embutidos; roda de pendrive, não mexe no sistema)
#
#   .\packaging\windows\build.ps1                 # os dois
#   .\packaging\windows\build.ps1 -SkipPortable   # só o instalador
#   .\packaging\windows\build.ps1 -SkipInstaller  # só o portátil
#
# Requer: Rust e Inno Setup 6.3+ (ISCC.exe — no PATH, em $env:ISCC ou no local padrão).
param([switch]$SkipInstaller, [switch]$SkipPortable)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
Set-Location $root
$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

Write-Host "==> compilando capivara $version"
cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build falhou' }
$exe = Get-ChildItem -Recurse -Path target -Filter capivara.exe |
  Where-Object { $_.FullName -match '\\release\\capivara\.exe$' } |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
New-Item -ItemType Directory -Force dist | Out-Null

# ───────────── instalador ─────────────
if (-not $SkipInstaller) {
  $iscc = @(
    $env:ISCC,
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
    "$env:USERPROFILE\.capivara-toolchain\innosetup\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 7\ISCC.exe"
  ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
  if (-not $iscc) { throw 'ISCC.exe (Inno Setup) não encontrado — instale em https://jrsoftware.org/isinfo.php' }
  Write-Host "==> instalador ($iscc)"
  & $iscc /Q "/DAppVersion=$version" "/DSourceExe=$($exe.FullName)" "/DOutputDir=$root\dist" packaging\windows\capivara.iss
  if ($LASTEXITCODE -ne 0) { throw 'ISCC falhou' }
}

# ───────────── portátil ─────────────
if (-not $SkipPortable) {
  Write-Host '==> versão portátil (com motores)'
  $stage = Join-Path $root 'target\win-dist\Capivara'
  $tmp = Join-Path $root 'target\win-dist\tmp'
  Remove-Item -Recurse -Force $stage, $tmp -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force $stage, $tmp | Out-Null
  Copy-Item $exe.FullName (Join-Path $stage 'capivara.exe')
  # A marca "portable" faz o app guardar tudo em .\data (nada vai para %LOCALAPPDATA%).
  Set-Content (Join-Path $stage 'portable') 'Este arquivo ativa o modo portatil do Capivara (dados em .\data).' -Encoding ascii
  curl.exe -fsSL -o (Join-Path $stage 'yt-dlp.exe') https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe
  curl.exe -fsSL -o "$tmp\ff.zip" https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip
  tar.exe -xf "$tmp\ff.zip" -C $tmp
  Get-ChildItem -Recurse $tmp -Include ffmpeg.exe, ffprobe.exe | Copy-Item -Destination $stage
  curl.exe -fsSL -o "$tmp\deno.zip" https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip
  tar.exe -xf "$tmp\deno.zip" -C $stage
  Remove-Item -Recurse -Force $tmp
  @"
CAPIVARA $version - versao portatil
====================================

1. Extraia esta pasta onde quiser (pode ser um pendrive).
2. Abra capivara.exe.

Ja vem com yt-dlp, ffmpeg e deno. O modelo de transcricao (Whisper) e
baixado pelo proprio app na primeira transcricao e fica em .\data.

Nada e gravado no sistema: configuracoes, modelos e cache ficam em .\data.
Os videos baixados vao para Downloads\Capivara (mude em Ajustes).

Para atualizar o yt-dlp: Motores > Atualizar.
"@ | Set-Content (Join-Path $stage 'LEIA-ME.txt') -Encoding utf8
  $zip = "dist\Capivara-$version-windows-x64-portable.zip"
  Remove-Item $zip -ErrorAction SilentlyContinue
  Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal
}

Get-ChildItem dist | Where-Object Name -like "Capivara-$version-windows*" |
  ForEach-Object { Write-Host ("OK -> dist\{0}  ({1:N1} MB)" -f $_.Name, ($_.Length / 1MB)) }
