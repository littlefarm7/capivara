// No Windows o app desktop não abre console; no modo `serve` reanexamos ao terminal pai.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod api;
mod engines;
mod proc;
mod state;
mod transcribe;
mod whisper;
mod whisper_model;
mod window;
mod ytdlp;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

const DESKTOP_PORT: u16 = 47800;

#[derive(Parser)]
#[command(name = "capivara", version, about = "🦫 Capivara — baixa vídeos, áudios e transcreve (YouTube, Shorts, TikTok, Reels, Stories…)")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Pasta de dados (motores, modelos, configurações)
    #[arg(long, env = "CAPIVARA_DATA", global = true)]
    data: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Abre o app desktop (padrão)
    Desktop {
        /// Só sobe o servidor e abre no navegador padrão
        #[arg(long)]
        no_window: bool,
    },
    /// Modo web: servidor para acessar de qualquer dispositivo (celular, outro PC…)
    Serve {
        #[arg(long, env = "CAPIVARA_HOST", default_value = "0.0.0.0")]
        host: String,
        #[arg(long, short, env = "CAPIVARA_PORT", default_value_t = 8080)]
        port: u16,
        /// Senha de acesso (recomendado ao expor na rede)
        #[arg(long, env = "CAPIVARA_TOKEN")]
        token: Option<String>,
    },
    /// Instala/atualiza os motores (yt-dlp, ffmpeg, deno) e um modelo Whisper
    Setup {
        /// Modelo Whisper (tiny, base, small, turbo) ou "none"
        #[arg(long, default_value = "base")]
        model: String,
        /// Não instala yt-dlp/ffmpeg/deno (só o modelo)
        #[arg(long)]
        no_engines: bool,
    },
    /// Transcreve um arquivo de áudio/vídeo local e imprime o texto (ou SRT)
    Transcribe {
        file: PathBuf,
        #[arg(long, default_value = "base")]
        model: String,
        /// Código do idioma (pt, en, es…) ou "auto"
        #[arg(long, default_value = "auto")]
        language: String,
        /// Traduz para inglês
        #[arg(long)]
        translate: bool,
        /// Saída em SRT em vez de texto
        #[arg(long)]
        srt: bool,
    },
}

#[cfg(windows)]
fn attach_console() {
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
#[cfg(not(windows))]
fn attach_console() {}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cmd = cli.cmd.unwrap_or(Cmd::Desktop { no_window: false });
    if !matches!(cmd, Cmd::Desktop { .. }) {
        attach_console();
    }
    let paths = engines::Paths::new(cli.data)?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        match cmd {
            Cmd::Desktop { no_window } => desktop(paths, no_window).await,
            Cmd::Serve { host, port, token } => serve(paths, host, port, token.filter(|t| !t.is_empty())).await,
            Cmd::Setup { model, no_engines } => setup(paths, model, no_engines).await,
            Cmd::Transcribe { file, model, language, translate, srt } => cli_transcribe(paths, file, model, language, translate, srt).await,
        }
    })
}

async fn cli_transcribe(paths: engines::Paths, file: PathBuf, model: String, language: String, translate: bool, srt: bool) -> anyhow::Result<()> {
    if !engines::model_installed(&paths, &model) {
        anyhow::bail!("modelo '{model}' não instalado — rode: capivara setup --model {model}");
    }
    let state = state::AppState::new(paths.clone(), false, None);
    let token = tokio_util::sync::CancellationToken::new();
    let t0 = std::time::Instant::now();
    let pcm = transcribe::decode_pcm(&state, &file, &token).await?;
    let secs = pcm.len() as f64 / 16000.0;
    let task = if translate { whisper::Task::Translate } else { whisper::Task::Transcribe };
    let dir = engines::model_dir(&paths, &model);
    let (segs, lang) = tokio::task::spawn_blocking(move || {
        let mut engine = whisper::Engine::load(&dir)?;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        engine.transcribe(&pcm, Some(&language), task, &cancel, |p, _| eprint!("\r{:>3.0}%", p * 100.0))
    })
    .await??;
    let el = t0.elapsed().as_secs_f64();
    eprintln!("\r✔ {secs:.0}s de áudio em {el:.1}s ({:.1}× tempo real) · idioma: {lang}", secs / el);
    println!("{}", if srt { transcribe::to_srt(&segs) } else { transcribe::to_text(&segs) });
    Ok(())
}

async fn already_running(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(Ok(mut s)) = tokio::time::timeout(
        std::time::Duration::from_millis(400),
        tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    else {
        return false;
    };
    let _ = s.write_all(b"GET /api/info HTTP/1.0\r\nHost: localhost\r\n\r\n").await;
    let mut buf = vec![0u8; 2048];
    let n = tokio::time::timeout(std::time::Duration::from_millis(800), s.read(&mut buf)).await.ok().and_then(|r| r.ok()).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).contains("\"capivara\"")
}

async fn desktop(paths: engines::Paths, no_window: bool) -> anyhow::Result<()> {
    // Instância única: se já está aberto, só abre outra janela.
    if already_running(DESKTOP_PORT).await {
        let url = format!("http://127.0.0.1:{DESKTOP_PORT}/");
        if no_window {
            let _ = open::that_detached(&url);
        } else {
            window::open(&url, &paths.data.join("window"));
        }
        return Ok(());
    }
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", DESKTOP_PORT)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
    };
    let addr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}/", addr.port());
    let state = state::AppState::new(paths.clone(), true, None);
    let shutdown = state.shutdown.clone();
    let app = api::router(state);
    let sd = shutdown.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).with_graceful_shutdown(sd.cancelled_owned()).await
    });

    let child = if no_window { let _ = open::that_detached(&url); None } else { window::open(&url, &paths.data.join("window")) };
    println!("🦫 Capivara rodando em {url}");
    match child {
        // Janela em modo app: fechou a janela → fecha o app; "Sair" no app → fecha a janela.
        Some(mut c) => {
            let start = std::time::Instant::now();
            loop {
                if let Ok(Some(_)) = c.try_wait() {
                    // Se o navegador devolveu na hora, ele entregou a janela para um processo já
                    // aberto: não dá pra saber quando fecha — fica rodando até "Sair" na interface.
                    if start.elapsed().as_secs() < 3 {
                        server.await??;
                    }
                    break;
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
                    _ = shutdown.cancelled() => {
                        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                        let _ = c.kill();
                        break;
                    }
                }
            }
        }
        None => {
            tokio::select! {
                r = server => { r??; }
                _ = tokio::signal::ctrl_c() => {}
            }
        }
    }
    Ok(())
}

async fn serve(paths: engines::Paths, host: String, port: u16, token: Option<String>) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let state = state::AppState::new(paths, false, token.clone());
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("🦫 Capivara (modo web) em http://{addr}/");
    if token.is_none() && !addr.ip().is_loopback() {
        println!("⚠️  Sem senha! Qualquer um na rede pode usar. Defina --token ou CAPIVARA_TOKEN.");
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn setup(paths: engines::Paths, model: String, no_engines: bool) -> anyhow::Result<()> {
    let mut items: Vec<String> = if no_engines { vec![] } else { vec!["yt-dlp".into(), "ffmpeg".into(), "deno".into()] };
    if !model.is_empty() && model != "none" {
        items.push(format!("model:{model}"));
    }
    for what in items {
        let tool = match what.as_str() {
            "yt-dlp" => Some(engines::Tool::YtDlp),
            "ffmpeg" => Some(engines::Tool::Ffmpeg),
            "deno" => Some(engines::Tool::Deno),
            _ => None,
        };
        let have = match tool {
            Some(t) => engines::find(&paths, t).is_some(),
            None => engines::model_installed(&paths, &what[6..]),
        };
        if have && what != "yt-dlp" {
            println!("✔ {what} já instalado");
            continue;
        }
        println!("↓ instalando {what}…");
        let mut last = -1i32;
        let r = engines::install(&paths, &what, |p, msg| {
            let pct = (p * 100.0) as i32;
            if pct / 10 != last / 10 {
                last = pct;
                println!("  {msg} {}", if p >= 0.0 { format!("{pct}%") } else { String::new() });
            }
        })
        .await;
        match r {
            Ok(p) => println!("✔ {what} → {p}"),
            Err(e) => println!("✘ {what}: {e:#}"),
        }
    }
    Ok(())
}
