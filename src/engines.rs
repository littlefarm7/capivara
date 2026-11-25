//! Localiza e instala os "motores" que o Capivara usa:
//! - yt-dlp  (extrai mídia de +1800 sites)
//! - ffmpeg  (junta vídeo+áudio, converte áudio, decodifica para o Whisper)
//! - deno    (runtime JS que o yt-dlp exige para o YouTube)
//! - modelos Whisper (transcrição local)

use crate::proc;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const EXE: &str = std::env::consts::EXE_SUFFIX;

#[derive(Clone)]
pub struct Paths {
    pub data: PathBuf,
    pub tools: PathBuf,
    pub models: PathBuf,
    pub tmp: PathBuf,
}

impl Paths {
    pub fn new(data_override: Option<PathBuf>) -> Result<Self> {
        let data = match data_override.or_else(portable_dir) {
            Some(d) => d,
            None => dirs::data_local_dir().context("sem diretório de dados")?.join("Capivara"),
        };
        let p = Self {
            tools: data.join("bin"),
            models: data.join("models"),
            tmp: data.join("tmp"),
            data,
        };
        for d in [&p.data, &p.tools, &p.models, &p.tmp] {
            std::fs::create_dir_all(d)?;
        }
        Ok(p)
    }
}

/// Modo portátil: um arquivo `portable` ao lado do executável faz tudo
/// (motores, modelos, configurações) ficar em `data/` na mesma pasta.
fn portable_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    dir.join("portable").is_file().then(|| dir.join("data"))
}

pub fn is_portable() -> bool {
    portable_dir().is_some()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Deno,
}

impl Tool {
    pub fn bin(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Deno => "deno",
        }
    }
    fn env(self) -> &'static str {
        match self {
            Tool::YtDlp => "CAPIVARA_YTDLP",
            Tool::Ffmpeg => "CAPIVARA_FFMPEG",
            Tool::Deno => "CAPIVARA_DENO",
        }
    }
}

/// Ordem: variável de ambiente → pasta do app → ao lado do executável (AppImage/.app) → PATH.
pub fn find(paths: &Paths, tool: Tool) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(tool.env()) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let name = format!("{}{}", tool.bin(), EXE);
    let mut candidates = vec![paths.tools.join(&name)];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(&name));
            candidates.push(dir.join("bin").join(&name));
            candidates.push(dir.join("../Resources/bin").join(&name)); // macOS .app
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(&name));
        }
    }
    // Homebrew nem sempre está no PATH de apps abertos pelo Finder.
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/opt/homebrew/bin").join(&name));
        candidates.push(PathBuf::from("/usr/local/bin").join(&name));
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[derive(Serialize, Clone)]
pub struct ToolStatus {
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
    pub installable: bool,
}

#[derive(Serialize, Clone)]
pub struct ModelStatus {
    pub name: String,
    pub label: String,
    pub size_mb: u32,
    pub installed: bool,
}

pub const MODELS: &[(&str, &str, &str, u32)] = &[
    ("tiny", "openai/whisper-tiny", "Tiny — rapidíssimo", 151),
    ("base", "openai/whisper-base", "Base — equilibrado", 290),
    ("small", "openai/whisper-small", "Small — preciso", 967),
    ("turbo", "openai/whisper-large-v3-turbo", "Turbo — máximo (lento em CPU)", 1620),
];

pub fn model_dir(paths: &Paths, name: &str) -> PathBuf {
    paths.models.join(name)
}

pub fn model_installed(paths: &Paths, name: &str) -> bool {
    let d = model_dir(paths, name);
    ["config.json", "tokenizer.json", "model.safetensors"].iter().all(|f| d.join(f).is_file())
}

pub async fn version(tool: Tool, path: &Path) -> Option<String> {
    let arg = match tool {
        Tool::YtDlp | Tool::Deno => "--version",
        _ => "-version",
    };
    let out = tokio::time::timeout(Duration::from_secs(20), proc::output(path, &[arg])).await.ok()?.ok()?;
    let line = out.lines().next()?.trim().to_string();
    Some(match tool {
        Tool::Ffmpeg => line
            .split_whitespace()
            .nth(2)
            .map(|v| v.chars().take(24).collect())
            .unwrap_or(line),
        Tool::Deno => line.replace("deno ", ""),
        _ => line,
    })
}

pub async fn status(paths: &Paths) -> (Vec<ToolStatus>, Vec<ModelStatus>) {
    let mut tools = vec![];
    for t in [Tool::YtDlp, Tool::Ffmpeg, Tool::Deno] {
        let path = find(paths, t);
        let version = match &path {
            Some(p) => version(t, p).await,
            None => None,
        };
        tools.push(ToolStatus {
            name: t.bin().into(),
            path: path.map(|p| p.display().to_string()),
            version,
            installable: install_supported(t),
        });
    }
    let models = MODELS
        .iter()
        .map(|(n, _, label, mb)| ModelStatus {
            name: n.to_string(),
            label: label.to_string(),
            size_mb: *mb,
            installed: model_installed(paths, n),
        })
        .collect();
    (tools, models)
}

fn install_supported(t: Tool) -> bool {
    match t {
        Tool::YtDlp => true,
        Tool::Ffmpeg => {
            cfg!(windows) || cfg!(target_os = "macos") || (cfg!(target_os = "linux") && matches!(std::env::consts::ARCH, "x86_64" | "aarch64"))
        }
        Tool::Deno => matches!(std::env::consts::ARCH, "x86_64" | "aarch64"),
    }
}

// ───────────────────────── download com progresso ─────────────────────────

async fn content_length(url: &str) -> Option<u64> {
    let out = proc::output("curl", &["-sIL", "--max-time", "30", url]).await.ok()?;
    out.lines()
        .filter_map(|l| {
            let l = l.to_ascii_lowercase();
            l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<u64>().ok())
        })
        .filter(|&n| n > 0)
        .last()
}

/// Baixa `url` para `dest` usando o curl do sistema (presente no Windows 10+, macOS e Linux).
pub async fn fetch(url: &str, dest: &Path, mut progress: impl FnMut(f32, u64)) -> Result<()> {
    let total = content_length(url).await;
    let part = dest.with_extension("part");
    let mut child = proc::command("curl")
        .args(["-fL", "--retry", "3", "--silent", "--show-error", "-o"])
        .arg(&part)
        .arg(url)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("curl não encontrado")?;
    loop {
        tokio::select! {
            st = child.wait() => {
                let st = st?;
                if !st.success() {
                    let _ = std::fs::remove_file(&part);
                    bail!("falha ao baixar {url}");
                }
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(400)) => {
                let got = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
                let p = total.map(|t| got as f32 / t as f32).unwrap_or(-1.0);
                progress(p.min(1.0), got);
            }
        }
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

fn extract_zip(zip_path: &Path, wanted: &[&str], dest_dir: &Path) -> Result<Vec<PathBuf>> {
    let f = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(f)?;
    let mut out = vec![];
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name()?.replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or("").to_string();
        if wanted.contains(&base.as_str()) {
            let target = dest_dir.join(&base);
            let mut w = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut w)?;
            out.push(target);
        }
    }
    Ok(out)
}

#[cfg(unix)]
fn make_exec(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}
#[cfg(not(unix))]
fn make_exec(_: &Path) -> Result<()> {
    Ok(())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(p);
        }
    }
    None
}

const ARCH: &str = std::env::consts::ARCH;

/// Instala um motor. `progress(fração, mensagem)`.
pub async fn install(paths: &Paths, what: &str, mut progress: impl FnMut(f32, String) + Send) -> Result<String> {
    let tools = &paths.tools;
    match what {
        "yt-dlp" => {
            let asset = if cfg!(windows) {
                "yt-dlp.exe"
            } else if cfg!(target_os = "macos") {
                "yt-dlp_macos"
            } else if ARCH == "aarch64" {
                "yt-dlp_linux_aarch64"
            } else {
                "yt-dlp_linux"
            };
            let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{asset}");
            let dest = tools.join(format!("yt-dlp{EXE}"));
            fetch(&url, &dest, |p, b| progress(p, format!("yt-dlp · {:.1} MB", b as f64 / 1e6))).await?;
            make_exec(&dest)?;
            Ok(dest.display().to_string())
        }
        "deno" => {
            let triple = match (std::env::consts::OS, ARCH) {
                ("windows", _) => "x86_64-pc-windows-msvc",
                ("macos", "aarch64") => "aarch64-apple-darwin",
                ("macos", _) => "x86_64-apple-darwin",
                ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
                ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
                (os, a) => bail!("deno não tem build oficial para {os}/{a} — instale node ou deno pelo gerenciador de pacotes"),
            };
            let url = format!("https://github.com/denoland/deno/releases/latest/download/deno-{triple}.zip");
            let zip = paths.tmp.join("deno.zip");
            fetch(&url, &zip, |p, b| progress(p, format!("deno · {:.1} MB", b as f64 / 1e6))).await?;
            progress(-1.0, "extraindo deno…".into());
            let name = format!("deno{EXE}");
            let got = extract_zip(&zip, &[&name], tools)?;
            let _ = std::fs::remove_file(&zip);
            let p = got.into_iter().next().context("deno não encontrado no zip")?;
            make_exec(&p)?;
            Ok(p.display().to_string())
        }
        "ffmpeg" => install_ffmpeg(paths, &mut progress).await,
        m if m.starts_with("model:") => {
            let name = &m[6..];
            let (_, repo, _, _) = MODELS.iter().find(|x| x.0 == name).context("modelo desconhecido")?;
            let dir = model_dir(paths, name);
            std::fs::create_dir_all(&dir)?;
            for (i, file) in ["config.json", "tokenizer.json", "model.safetensors"].iter().enumerate() {
                let dest = dir.join(file);
                if dest.is_file() {
                    continue;
                }
                let url = format!("https://huggingface.co/{repo}/resolve/main/{file}");
                fetch(&url, &dest, |p, b| {
                    let overall = if *file == "model.safetensors" { 0.05 + 0.95 * p.max(0.0) } else { 0.02 * i as f32 };
                    progress(overall, format!("Whisper {name} · {file} · {:.0} MB", b as f64 / 1e6))
                })
                .await?;
            }
            Ok(dir.display().to_string())
        }
        other => bail!("motor desconhecido: {other}"),
    }
}

async fn install_ffmpeg(paths: &Paths, progress: &mut (impl FnMut(f32, String) + Send)) -> Result<String> {
    let tools = &paths.tools;
    let work = paths.tmp.join("ffmpeg-install");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let result = async {
        if cfg!(target_os = "macos") {
            for (bin, url) in [
                ("ffmpeg", "https://evermeet.cx/ffmpeg/getrelease/zip"),
                ("ffprobe", "https://evermeet.cx/ffmpeg/getrelease/ffprobe/zip"),
            ] {
                let zip = work.join(format!("{bin}.zip"));
                fetch(url, &zip, |p, b| progress(p, format!("{bin} · {:.1} MB", b as f64 / 1e6))).await?;
                for f in extract_zip(&zip, &[bin], tools)? {
                    make_exec(&f)?;
                }
            }
        } else if cfg!(windows) {
            let url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
            let zip = work.join("ffmpeg.zip");
            fetch(url, &zip, |p, b| progress(p, format!("ffmpeg · {:.0} MB", b as f64 / 1e6))).await?;
            progress(-1.0, "extraindo ffmpeg…".into());
            let z = zip.clone();
            let t = tools.clone();
            tokio::task::spawn_blocking(move || extract_zip(&z, &["ffmpeg.exe", "ffprobe.exe"], &t)).await??;
        } else {
            let asset = if ARCH == "aarch64" { "linuxarm64" } else { "linux64" };
            let url = format!("https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-{asset}-gpl.tar.xz");
            let tar = work.join("ffmpeg.tar.xz");
            fetch(&url, &tar, |p, b| progress(p, format!("ffmpeg · {:.0} MB", b as f64 / 1e6))).await?;
            progress(-1.0, "extraindo ffmpeg…".into());
            proc::output("tar", &["-xJf", &tar.display().to_string(), "-C", &work.display().to_string()]).await?;
            for bin in ["ffmpeg", "ffprobe"] {
                let src = find_file(&work, bin).context("binário ausente no pacote")?;
                let dst = tools.join(bin);
                std::fs::copy(&src, &dst)?;
                make_exec(&dst)?;
            }
        }
        anyhow::Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&work);
    result?;
    Ok(tools.join(format!("ffmpeg{EXE}")).display().to_string())
}
