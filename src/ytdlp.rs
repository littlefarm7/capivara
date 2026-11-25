//! Integração com o yt-dlp: preview, download com progresso e legendas.

use crate::engines::{self, Tool};
use crate::proc;
use crate::state::{FileOut, Shared};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

pub fn ytdlp(state: &Shared) -> Result<PathBuf> {
    engines::find(&state.paths, Tool::YtDlp).context("yt-dlp not found — instale em Motores")
}

/// Argumentos comuns: ffmpeg, runtime JS, cookies.
pub fn base_args(state: &Shared) -> Vec<String> {
    let s = state.settings();
    let mut a: Vec<String> = vec![
        "--newline".into(),
        "--color".into(),
        "no_color".into(),
        "--no-warnings".into(),
        // Sem isso o yt-dlp no Windows usa a code page do console e quebra emojis/acentos.
        "--encoding".into(),
        "utf-8".into(),
    ];
    if let Some(ff) = engines::find(&state.paths, Tool::Ffmpeg) {
        a.push("--ffmpeg-location".into());
        a.push(ff.display().to_string());
    }
    if let Some(deno) = engines::find(&state.paths, Tool::Deno) {
        a.push("--js-runtimes".into());
        a.push(format!("deno:{}", deno.display()));
    }
    if !s.cookies_file.trim().is_empty() {
        a.push("--cookies".into());
        a.push(s.cookies_file.trim().into());
    } else if !s.cookies_browser.trim().is_empty() {
        a.push("--cookies-from-browser".into());
        a.push(s.cookies_browser.trim().into());
    }
    a
}

#[derive(Serialize, Default)]
pub struct Probe {
    pub title: String,
    pub uploader: String,
    pub duration: Option<f64>,
    pub thumbnail: String,
    pub extractor: String,
    pub webpage_url: String,
    pub is_playlist: bool,
    pub entries: usize,
    pub heights: Vec<u32>,
    pub subtitles: Vec<String>,
    pub auto_captions: bool,
    pub language: String,
    pub view_count: Option<u64>,
}

pub async fn probe_json(state: &Shared, url: &str, flat: bool) -> Result<serde_json::Value> {
    let bin = ytdlp(state)?;
    let mut args = base_args(state);
    args.extend(["-J".into(), "--no-playlist".into()]);
    if flat {
        args.push("--flat-playlist".into());
    }
    args.push("--".into());
    args.push(url.into());
    let out = proc::command(&bin).args(&args).output().await?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

pub async fn probe(state: &Shared, url: &str) -> Result<Probe> {
    let v = probe_json(state, url, true).await?;
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let is_playlist = v["_type"].as_str() == Some("playlist");
    let mut thumb = s("thumbnail");
    if thumb.is_empty() {
        thumb = v["thumbnails"]
            .as_array()
            .and_then(|a| a.last())
            .and_then(|t| t["url"].as_str())
            .unwrap_or("")
            .to_string();
    }
    if thumb.is_empty() && is_playlist {
        thumb = v["entries"][0]["thumbnails"]
            .as_array()
            .and_then(|a| a.last())
            .and_then(|t| t["url"].as_str())
            .unwrap_or("")
            .to_string();
    }
    let mut heights: Vec<u32> = v["formats"]
        .as_array()
        .map(|f| f.iter().filter_map(|x| x["height"].as_u64().map(|h| h as u32)).collect())
        .unwrap_or_default();
    heights.sort_unstable();
    heights.dedup();
    let subtitles = v["subtitles"]
        .as_object()
        .map(|o| o.keys().filter(|k| *k != "live_chat").cloned().collect())
        .unwrap_or_default();
    Ok(Probe {
        title: s("title"),
        uploader: [s("uploader"), s("channel"), s("creator"), s("uploader_id")].into_iter().find(|x| !x.is_empty()).unwrap_or_default(),
        duration: v["duration"].as_f64(),
        thumbnail: thumb,
        extractor: s("extractor_key"),
        webpage_url: s("webpage_url"),
        is_playlist,
        entries: v["entries"].as_array().map(|a| a.len()).unwrap_or(0),
        heights,
        subtitles,
        auto_captions: v["automatic_captions"].as_object().map(|o| !o.is_empty()).unwrap_or(false),
        language: s("language"),
        view_count: v["view_count"].as_u64(),
    })
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct DownloadOpts {
    pub url: String,
    /// video | audio
    pub kind: String,
    /// best | 2160 | 1440 | 1080 | 720 | 480 | 360
    pub quality: String,
    /// vídeo: mp4 | mkv | webm — áudio: mp3 | m4a | opus | wav | flac
    pub format: String,
    pub compat: bool,
    pub playlist: bool,
    pub subs: bool,
    pub thumbnail: bool,
    pub platform: String,
}

impl Default for DownloadOpts {
    fn default() -> Self {
        Self {
            url: String::new(),
            kind: "video".into(),
            quality: "best".into(),
            format: "mp4".into(),
            compat: true,
            playlist: false,
            subs: false,
            thumbnail: true,
            platform: String::new(),
        }
    }
}

const OUT_TEMPLATE: &str = "%(uploader,channel,creator,uploader_id|Capivara).60B - %(title,id).110B [%(id)s].%(ext)s";

fn fmt_bytes(b: f64) -> String {
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

fn fmt_eta(s: f64) -> String {
    let s = s as u64;
    if s >= 3600 { format!("{}h{:02}m", s / 3600, (s % 3600) / 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Roda o yt-dlp lendo as linhas de progresso. Retorna os arquivos finais.
async fn run_with_progress(state: &Shared, id: &str, args: Vec<String>, token: &CancellationToken) -> Result<Vec<PathBuf>> {
    let bin = ytdlp(state)?;
    let mut child = proc::command(&bin)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("não consegui iniciar o yt-dlp")?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let err_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut BufReader::new(stderr), &mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    });
    let mut files = vec![];
    // Lê bytes (não String): uma linha com encoding estranho não pode derrubar o download.
    let mut lines = BufReader::new(stdout).split(b'\n');
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                let _ = child.kill().await;
                bail!("cancelado");
            }
            line = lines.next_segment() => {
                let Some(line) = line? else { break };
                let line = String::from_utf8_lossy(&line);
                handle_line(state, id, line.trim_end_matches('\r'), &mut files);
            }
        }
    }
    let status = child.wait().await?;
    let err = err_task.await.unwrap_or_default();
    if !status.success() {
        if !files.is_empty() && args.iter().any(|a| a == "--yes-playlist") {
            // playlist com itens indisponíveis: entrega o que deu
            return Ok(files);
        }
        bail!("{}", if err.trim().is_empty() { "yt-dlp falhou" } else { err.trim() });
    }
    Ok(files)
}

fn handle_line(state: &Shared, id: &str, line: &str, files: &mut Vec<PathBuf>) {
    if let Some(rest) = line.strip_prefix("CAPDL|") {
        let p: Vec<&str> = rest.split('|').collect();
        let num = |i: usize| p.get(i).and_then(|v| v.trim().parse::<f64>().ok());
        let done = num(0).unwrap_or(0.0);
        let total = num(1).or(num(2));
        let speed = num(3);
        let eta = num(4);
        let idx = p.get(5).and_then(|v| v.trim().parse::<u32>().ok());
        let n = p.get(6).and_then(|v| v.trim().parse::<u32>().ok());
        state.update(id, |j| {
            j.status = "running".into();
            j.progress = total.map(|t| (done / t).min(1.0) as f32).unwrap_or(-1.0);
            j.speed = speed.map(|s| format!("{}/s", fmt_bytes(s))).unwrap_or_default();
            j.eta = eta.map(fmt_eta).unwrap_or_default();
            let base = match total {
                Some(t) => format!("Baixando · {} de {}", fmt_bytes(done), fmt_bytes(t)),
                None => format!("Baixando · {}", fmt_bytes(done)),
            };
            j.stage = match (idx, n) {
                (Some(i), Some(n)) if n > 1 => format!("Item {i}/{n} · {base}"),
                _ => base,
            };
        });
    } else if let Some(rest) = line.strip_prefix("CAPPP|") {
        let pp = rest.split('|').next().unwrap_or("").to_string();
        let label = match pp.as_str() {
            "Merger" => "Juntando vídeo e áudio…",
            "ExtractAudio" => "Convertendo áudio…",
            "EmbedThumbnail" => "Colocando capa…",
            "FFmpegMetadata" | "Metadata" => "Gravando metadados…",
            "EmbedSubtitle" => "Embutindo legendas…",
            "VideoConvertor" | "VideoRemuxer" => "Convertendo vídeo…",
            _ => "Processando…",
        };
        state.update(id, |j| {
            j.progress = -1.0;
            j.stage = label.into();
            j.speed.clear();
            j.eta.clear();
        });
    } else if let Some(rest) = line.strip_prefix("CAPMETA|") {
        let mut p = rest.splitn(2, '|');
        let title = p.next().unwrap_or("").to_string();
        let thumb = p.next().unwrap_or("").trim().to_string();
        state.update(id, |j| {
            if j.title.is_empty() || j.title == j.url {
                j.title = title;
            }
            if j.thumbnail.is_empty() && thumb.starts_with("http") {
                j.thumbnail = thumb;
            }
        });
    } else if let Some(rest) = line.strip_prefix("CAPFILE|") {
        let p = PathBuf::from(rest.trim());
        if !files.contains(&p) {
            files.push(p);
        }
    }
}

fn progress_args() -> Vec<String> {
    [
        "--progress",
        "--progress-template",
        "download:CAPDL|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.playlist_index)s|%(info.n_entries)s",
        "--progress-template",
        "postprocess:CAPPP|%(progress.postprocessor)s|%(progress.status)s",
        "--print",
        "before_dl:CAPMETA|%(title)s|%(thumbnail)s",
        "--print",
        "after_move:CAPFILE|%(filepath)s",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub async fn download(state: &Shared, id: &str, o: &DownloadOpts, token: &CancellationToken) -> Result<()> {
    let out = state.output_dir();
    let lang = state.settings().language;
    let mut a = base_args(state);
    a.extend(progress_args());
    a.push(if o.playlist { "--yes-playlist" } else { "--no-playlist" }.into());
    let template = if o.playlist { format!("%(playlist_title,playlist_id|Playlist).80B/{OUT_TEMPLATE}") } else { OUT_TEMPLATE.to_string() };
    a.extend(["-P".into(), out.display().to_string(), "-o".into(), template, "--no-mtime".into(), "--embed-metadata".into()]);
    if o.kind == "audio" {
        let fmt = match o.format.as_str() {
            "mp3" | "m4a" | "opus" | "wav" | "flac" => o.format.as_str(),
            _ => "mp3",
        };
        a.extend(["-f".into(), "ba/b".into(), "-x".into(), "--audio-format".into(), fmt.into(), "--audio-quality".into(), "0".into()]);
        if o.thumbnail && matches!(fmt, "mp3" | "m4a" | "flac" | "opus") {
            a.push("--embed-thumbnail".into());
        }
    } else {
        let container = match o.format.as_str() {
            "mkv" | "webm" | "mp4" => o.format.as_str(),
            _ => "mp4",
        };
        let mut sort = vec![];
        if let Ok(h) = o.quality.parse::<u32>() {
            sort.push(format!("res:{h}"));
        }
        if o.compat && container == "mp4" {
            sort.push("vcodec:h264".into());
            sort.push("acodec:m4a".into());
        }
        a.extend(["-f".into(), "bv*+ba/b".into(), "--merge-output-format".into(), container.into()]);
        if !sort.is_empty() {
            a.extend(["-S".into(), sort.join(",")]);
        }
        if o.thumbnail && container != "webm" {
            a.push("--embed-thumbnail".into());
        }
        if o.subs {
            let l = if lang == "auto" || lang.is_empty() { "pt.*,en.*".to_string() } else { format!("{lang}.*") };
            a.extend(["--write-subs".into(), "--embed-subs".into(), "--sub-langs".into(), l]);
        }
    }
    a.push("--".into());
    a.push(o.url.clone());
    let files = run_with_progress(state, id, a, token).await?;
    if files.is_empty() {
        bail!("o yt-dlp terminou sem gerar arquivo (talvez já exista ou o link não tenha mídia)");
    }
    state.update(id, |j| j.files = files.into_iter().map(FileOut::from_path).collect());
    Ok(())
}

/// Baixa só o melhor áudio para uma pasta temporária (para o Whisper).
pub async fn fetch_audio(state: &Shared, id: &str, url: &str, dir: &Path, token: &CancellationToken) -> Result<PathBuf> {
    let mut a = base_args(state);
    a.extend(progress_args());
    a.extend([
        "--no-playlist".into(),
        "-f".into(),
        "ba/b".into(),
        "-P".into(),
        dir.display().to_string(),
        "-o".into(),
        "audio.%(ext)s".into(),
        "--".into(),
        url.into(),
    ]);
    let files = run_with_progress(state, id, a, token).await?;
    files.into_iter().next().context("áudio não baixado")
}

/// Escolhe a melhor legenda do vídeo para o idioma pedido. Retorna (chave, automática?).
pub fn pick_caption(info: &serde_json::Value, lang: &str) -> Option<(String, bool)> {
    let manual: Vec<String> = info["subtitles"].as_object().map(|o| o.keys().filter(|k| *k != "live_chat").cloned().collect()).unwrap_or_default();
    let auto: Vec<String> = info["automatic_captions"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    let want: String = if lang.is_empty() || lang == "auto" {
        let l = info["language"].as_str().unwrap_or("");
        // A auto-legenda "xx-orig" do YouTube revela o idioma falado no vídeo.
        let orig = auto.iter().find_map(|k| k.strip_suffix("-orig")).map(str::to_string);
        if !l.is_empty() {
            l.to_string()
        } else if let Some(o) = orig {
            o
        } else if let Some(k) = manual.iter().find(|k| k.starts_with("en")).or(manual.first()) {
            return Some((k.clone(), false));
        } else {
            return None;
        }
    } else {
        lang.to_string()
    };
    let base = want.split(['-', '_']).next().unwrap_or(&want).to_lowercase();
    let matches = |k: &String| {
        let kl = k.to_lowercase();
        kl == want.to_lowercase() || kl.split(['-', '_']).next() == Some(base.as_str())
    };
    if let Some(k) = manual.iter().find(|k| matches(k)) {
        return Some((k.clone(), false));
    }
    // Auto-legenda original do idioma (YouTube usa "xx-orig"), senão a tradução automática.
    if let Some(k) = auto.iter().find(|k| k.to_lowercase() == format!("{base}-orig")) {
        return Some((k.clone(), true));
    }
    auto.iter().find(|k| k.to_lowercase() == base).map(|k| (k.clone(), true))
}

/// Baixa a legenda escolhida em VTT e devolve o caminho.
pub async fn fetch_caption(state: &Shared, url: &str, key: &str, auto: bool, dir: &Path, token: &CancellationToken) -> Result<PathBuf> {
    let bin = ytdlp(state)?;
    let mut a = base_args(state);
    a.extend([
        "--skip-download".into(),
        "--no-playlist".into(),
        if auto { "--write-auto-subs" } else { "--write-subs" }.into(),
        "--sub-langs".into(),
        key.into(),
        "--sub-format".into(),
        "vtt/srt/best".into(),
        "--convert-subs".into(),
        "vtt".into(),
        "-P".into(),
        dir.display().to_string(),
        "-o".into(),
        "sub.%(ext)s".into(),
        "--".into(),
        url.into(),
    ]);
    let fut = proc::command(&bin).args(&a).output();
    let out = tokio::select! {
        o = fut => o?,
        _ = token.cancelled() => bail!("cancelado"),
    };
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr));
    }
    for e in std::fs::read_dir(dir)?.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("vtt") {
            return Ok(p);
        }
    }
    bail!("legenda não encontrada")
}
