# 🦫 Capivara

**Cola o link. A capivara resolve.**

O Capivara é um app em Rust para baixar **vídeos** e **áudios** e fazer **transcrições** de YouTube (vídeos normais e Shorts), TikTok, Instagram (Reels, posts e Stories), X/Twitter, Facebook, Vimeo, SoundCloud, Twitch, Reddit e mais de 1800 outros sites. A interface é neobrutalista e responsiva, e o mesmo binário roda como **app desktop** ou como **servidor web**.

| Função | Como funciona |
|---|---|
| **Baixar vídeo** | Qualidade de 360p até 4K. Sai em MP4, MKV ou WEBM. A opção "Compatível" prefere H.264, que abre em qualquer player. Pode embutir capa e legendas e baixar playlists inteiras. |
| **Baixar áudio** | MP3, M4A, OPUS, WAV ou FLAC, sempre na melhor qualidade e com a capa embutida. |
| **Transcrever link** | No modo **Inteligente**, usa a legenda oficial ou automática do site, o que é instantâneo. Se o vídeo não tiver legenda, ouve o áudio com o **Whisper rodando localmente**. |
| **Transcrever arquivo** | Arraste qualquer áudio ou vídeo do computador. O resultado sai em **TXT, SRT e VTT**. |
| **Traduzir → EN** | O Whisper transcreve e já traduz para o inglês. |

A transcrição é **100% local**: o Whisper roda em Rust puro (via [candle](https://github.com/huggingface/candle)), com decodificação incremental e cache KV. Nenhum áudio sai da sua máquina. Numa CPU comum, o modelo `base` transcreve português a cerca de **3,5× o tempo real**.

---

## Baixar

Os pacotes ficam na página de **[Releases](../../releases/latest)**:

| Sistema | Arquivo | |
|---|---|---|
| **Windows (instalador)** | `Capivara-x.y.z-windows-x64-setup.exe` | Instala sem pedir administrador. Cria atalhos e já pode baixar os motores e o modelo durante a instalação. |
| **Windows (portátil)** | `Capivara-x.y.z-windows-x64-portable.zip` | É só extrair e abrir. Já vem com os motores e guarda tudo na própria pasta, então roda até de um pendrive. |
| **Linux** | `Capivara-x.y.z-x86_64.AppImage` / `-aarch64.AppImage` | `chmod +x` e abrir. |
| **macOS** | `Capivara-x.y.z-macos.dmg` | Universal: Intel + Apple Silicon. |
| **Servidor web** | `ghcr.io/<dono>/capivara` | Imagem Docker. |

> O Windows SmartScreen pode avisar que o app é "desconhecido", porque o executável não tem assinatura digital paga. Clique em **Mais informações → Executar assim mesmo**.

## Usar

### Desktop (Windows, macOS, Linux)
Abra o `capivara`. A interface aparece numa janela própria, que usa o Edge, o Chrome, o Brave ou o Chromium em "modo app", sem abas nem barra de endereço. Se nenhum desses estiver instalado, ela abre no navegador padrão.

Na primeira vez, clique em **Instalar tudo ⚡**. O app baixa os motores (yt-dlp, ffmpeg, deno) e o modelo Whisper para a pasta de dados dele:

| Sistema | Pasta de dados |
|---|---|
| Windows | `%LOCALAPPDATA%\Capivara` |
| macOS | `~/Library/Application Support/Capivara` |
| Linux | `~/.local/share/Capivara` |

Os downloads vão para `Downloads/Capivara`. Dá para mudar isso em **Ajustes**.

### Web (celular, tablet, outro PC)
```bash
capivara serve --port 8080 --token minhasenha
```
Depois é só abrir `http://IP-DO-PC:8080` em qualquer navegador. No celular, dá para "Adicionar à tela inicial" e usar como app (PWA). Ele também aparece no menu **Compartilhar**, então você manda um link do TikTok ou do YouTube direto para o Capivara. Os arquivos ficam no servidor, e o botão **⬇ Salvar** baixa cada um para o aparelho.

Com Docker:
```bash
docker compose up -d        # edite CAPIVARA_TOKEN no docker-compose.yml antes
```

### Terminal
```bash
capivara setup --model base              # instala motores + modelo
capivara transcribe aula.mp3 --language pt --srt > aula.srt
capivara serve --host 127.0.0.1 --port 8080
capivara desktop --no-window             # só servidor + navegador padrão
```

### Stories do Instagram e conteúdo com login
O Instagram exige login para Stories, e muitas vezes também para Reels. Em **Ajustes → Cookies do navegador**, escolha o navegador onde você está logado (Chrome, Firefox, Edge…) ou aponte um arquivo `cookies.txt`. O app só lê os cookies localmente, através do yt-dlp.

---

## Compilar

Pré-requisito: [Rust](https://rustup.rs) estável.

```bash
cargo build --release          # → target/release/capivara(.exe)
```

| Plataforma | Comando | Saída |
|---|---|---|
| **Linux → AppImage** | `bash packaging/linux/build-appimage.sh` | `dist/Capivara-<v>-x86_64.AppImage` (também `ARCH=aarch64`) |
| **macOS → .app / .dmg** | `bash packaging/macos/build-app.sh` | `dist/Capivara-<v>-macos.dmg` (universal: Intel + Apple Silicon) |
| **Windows → instalador + portátil** | `.\packaging\windows\build.ps1` (requer [Inno Setup](https://jrsoftware.org/isinfo.php)) | `dist\Capivara-<v>-windows-x64-setup.exe` e `-portable.zip` |
| **Web → Docker** | `docker build -t capivara .` | imagem `linux/amd64` e `linux/arm64` |

A AppImage, o .dmg e o portátil do Windows já vêm com yt-dlp, ffmpeg e deno embutidos. Na AppImage e no .dmg, `BUNDLE_ENGINES=0` gera uma versão leve, que baixa os motores no primeiro uso. O instalador do Windows também é leve e oferece baixar os motores durante a instalação.

**Modo portátil:** se existir um arquivo chamado `portable` ao lado do executável, o app guarda motores, modelos e configurações em `data\`, na mesma pasta, em vez do perfil do usuário. O ícone do Windows é gerado a partir de `ui/icon.svg` por `packaging\windows\make-icon.ps1`.

**Release automática:** o workflow `.github/workflows/release.yml` compila tudo isso (AppImage x86_64 e aarch64, Windows, macOS universal e Docker multi-arch) sempre que você der push numa tag `v*`.

> **Windows sem Visual Studio:** esta máquina foi configurada com o toolchain `x86_64-pc-windows-gnullvm` + [llvm-mingw](https://github.com/mstorsjo/llvm-mingw), que dispensa o Visual Studio. Com o toolchain padrão MSVC também funciona.

---

## Arquitetura

```
src/
  main.rs           CLI: desktop | serve | setup | transcribe
  api.rs            servidor axum: API REST + eventos em tempo real (SSE) + UI embutida
  state.rs          fila de tarefas, configurações, mensagens de erro amigáveis
  ytdlp.rs          preview, download com progresso, escolha de legendas
  transcribe.rs     pipeline legenda→Whisper, parser VTT, export TXT/SRT/VTT
  whisper.rs        decodificação (timestamps, detecção de idioma, fallback, anti-loop)
  whisper_model.rs  rede do Whisper com cache KV incremental
  engines.rs        localiza/instala yt-dlp, ffmpeg, deno e modelos
  window.rs         janela desktop (Chromium em modo app)
ui/                 HTML + CSS + JS puro (embutidos no binário, sem build)
packaging/          AppImage, .app/.dmg, zip do Windows
```

O binário tem cerca de **5 MB** e não depende de WebView do sistema, Node ou Python.

---

## Aviso
Baixe só o que você tem direito de baixar: o seu conteúdo, conteúdo licenciado ou uso permitido pela lei. Respeite os termos de cada plataforma. O ffmpeg embutido nos pacotes é distribuído sob GPL, e o yt-dlp e o deno sob licenças livres próprias.
