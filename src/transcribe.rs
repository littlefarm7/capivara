//! Transcrição: usa a legenda da própria plataforma quando existe (instantâneo)
//! ou roda o Whisper localmente sobre o áudio.

use crate::engines::{self, Tool};
use crate::proc;
use crate::state::{FileOut, Shared};
use crate::whisper::{self, Segment, Task};
use crate::ytdlp;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct TranscribeOpts {
    pub url: String,
    /// auto | captions | whisper
    pub source: String,
    pub language: String,
    /// transcribe | translate (traduz para inglês)
    pub task: String,
    pub model: String,
    pub platform: String,
}

impl Default for TranscribeOpts {
    fn default() -> Self {
        Self {
            url: String::new(),
            source: "auto".into(),
            language: String::new(),
            task: "transcribe".into(),
            model: String::new(),
            platform: String::new(),
        }
    }
}

// ───────────────────────── formatos ─────────────────────────

fn parse_ts(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let mut secs = 0f64;
    for p in &parts {
        secs = secs * 60.0 + p.parse::<f64>().ok()?;
    }
    Some(secs)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

/// Lê VTT/SRT e remove as linhas repetidas das auto-legendas "roladas" do YouTube.
pub fn parse_vtt(text: &str) -> Vec<Segment> {
    let mut segs: Vec<Segment> = vec![];
    let mut last_line = String::new();
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(ti) = lines.iter().position(|l| l.contains("-->")) else { continue };
        let mut it = lines[ti].split("-->");
        let start = it.next().and_then(parse_ts);
        let end = it.next().and_then(|r| r.split_whitespace().next()).and_then(parse_ts);
        let (Some(start), Some(end)) = (start, end) else { continue };
        for l in &lines[ti + 1..] {
            let t = strip_tags(l);
            if t.is_empty() || t == last_line {
                continue;
            }
            last_line = t.clone();
            segs.push(Segment { start, end, text: t });
        }
    }
    segs
}

fn ts(secs: f64, sep: char) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{}{:03}", ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, sep, ms % 1000)
}

pub fn to_srt(segs: &[Segment]) -> String {
    segs.iter()
        .enumerate()
        .map(|(i, s)| format!("{}\n{} --> {}\n{}\n", i + 1, ts(s.start, ','), ts(s.end, ','), s.text))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn to_vtt(segs: &[Segment]) -> String {
    let mut o = String::from("WEBVTT\n\n");
    for s in segs {
        o.push_str(&format!("{} --> {}\n{}\n\n", ts(s.start, '.'), ts(s.end, '.'), s.text));
    }
    o
}

/// Texto corrido, com quebra de parágrafo em pausas longas.
pub fn to_text(segs: &[Segment]) -> String {
    let mut o = String::new();
    let mut prev_end = None;
    for s in segs {
        if let Some(pe) = prev_end {
            let gap: f64 = s.start - pe;
            o.push_str(if gap > 2.0 || o.ends_with(['.', '?', '!']) && gap > 1.0 { "\n\n" } else { " " });
        }
        o.push_str(s.text.trim());
        prev_end = Some(s.end);
    }
    o.trim().to_string()
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let s = s.trim().trim_end_matches('.').to_string();
    let s: String = s.chars().take(120).collect();
    if s.is_empty() { "transcricao".into() } else { s }
}

fn write_outputs(state: &Shared, title: &str, segs: &[Segment]) -> Result<Vec<FileOut>> {
    let dir = state.output_dir().join("Transcrições");
    std::fs::create_dir_all(&dir)?;
    let mut base = sanitize(title);
    // Não sobrescreve transcrições anteriores do mesmo vídeo.
    let mut n = 2;
    let orig = base.clone();
    while ["txt", "srt", "vtt"].iter().any(|e| dir.join(format!("{base}.{e}")).exists()) {
        base = format!("{orig} ({n})");
        n += 1;
    }
    let mut out = vec![];
    for (ext, content) in [("txt", to_text(segs)), ("srt", to_srt(segs)), ("vtt", to_vtt(segs))] {
        let p = dir.join(format!("{base}.{ext}"));
        std::fs::write(&p, content)?;
        out.push(FileOut::from_path(p));
    }
    Ok(out)
}

// ───────────────────────── whisper ─────────────────────────

/// Decodifica qualquer áudio/vídeo para PCM f32 mono 16 kHz via ffmpeg.
pub async fn decode_pcm(state: &Shared, input: &Path, token: &CancellationToken) -> Result<Vec<f32>> {
    let ff = engines::find(&state.paths, Tool::Ffmpeg).context("ffmpeg not found — instale em Motores")?;
    let fut = proc::command(&ff)
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(input)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
        .output();
    let out = tokio::select! {
        o = fut => o?,
        _ = token.cancelled() => bail!("cancelado"),
    };
    if !out.status.success() {
        bail!("ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

async fn run_whisper(state: &Shared, id: &str, pcm: Vec<f32>, opts: &TranscribeOpts, token: &CancellationToken) -> Result<(Vec<Segment>, String)> {
    let model = if opts.model.is_empty() { state.settings().whisper_model } else { opts.model.clone() };
    if !engines::model_installed(&state.paths, &model) {
        bail!("Modelo Whisper '{model}' não instalado — instale em Motores.");
    }
    state.update(id, |j| {
        j.stage = "Na fila do Whisper…".into();
        j.progress = -1.0;
    });
    let _permit = state.cpu.acquire().await?;
    let st = state.clone();
    let id2 = id.to_string();
    let tok = token.clone();
    let lang = if opts.language.is_empty() { state.settings().language } else { opts.language.clone() };
    let task = if opts.task == "translate" { Task::Translate } else { Task::Transcribe };
    let total_secs = pcm.len() as f64 / 16000.0;
    tokio::task::spawn_blocking(move || -> Result<(Vec<Segment>, String)> {
        let mut guard = st.whisper.lock().unwrap();
        if guard.as_ref().map(|(n, _)| n != &model).unwrap_or(true) {
            st.update(&id2, |j| j.stage = format!("Carregando modelo {model}…"));
            *guard = None;
            let engine = whisper::Engine::load(&engines::model_dir(&st.paths, &model))?;
            *guard = Some((model.clone(), engine));
        }
        let (_, engine) = guard.as_mut().unwrap();
        st.update(&id2, |j| {
            j.stage = "Ouvindo e escrevendo…".into();
            j.progress = 0.0;
        });
        let started = std::time::Instant::now();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let res = engine.transcribe(&pcm, Some(&lang), task, &cancel, |p, segs| {
            if tok.is_cancelled() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let el = started.elapsed().as_secs_f64();
            let speed = if el > 0.0 { p as f64 * total_secs / el } else { 0.0 };
            let eta = if p > 0.01 { el * (1.0 - p as f64) / p as f64 } else { 0.0 };
            let preview = whisper_preview(segs);
            st.update(&id2, |j| {
                j.progress = p;
                j.speed = format!("{speed:.1}× tempo real");
                j.eta = if eta > 0.0 { format!("{}:{:02}", eta as u64 / 60, eta as u64 % 60) } else { String::new() };
                j.stage = format!("Transcrevendo · {:.0}%", p * 100.0);
                j.transcript = Some(preview);
            });
        });
        res
    })
    .await?
}

fn whisper_preview(segs: &[Segment]) -> String {
    to_text(segs)
}

// ───────────────────────── pipelines ─────────────────────────

pub async fn from_url(state: &Shared, id: &str, opts: &TranscribeOpts, token: &CancellationToken) -> Result<()> {
    let tmp = state.paths.tmp.join(id);
    std::fs::create_dir_all(&tmp)?;
    let res = from_url_inner(state, id, opts, token, &tmp).await;
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

async fn from_url_inner(state: &Shared, id: &str, opts: &TranscribeOpts, token: &CancellationToken, tmp: &Path) -> Result<()> {
    state.update(id, |j| {
        j.status = "running".into();
        j.stage = "Lendo o link…".into();
    });
    let info = tokio::select! {
        r = ytdlp::probe_json(state, &opts.url, false) => r?,
        _ = token.cancelled() => bail!("cancelado"),
    };
    let title = info["title"].as_str().unwrap_or("transcricao").to_string();
    let vid = info["id"].as_str().unwrap_or("").to_string();
    let thumb = info["thumbnail"].as_str().unwrap_or("").to_string();
    state.update(id, |j| {
        j.title = title.clone();
        j.thumbnail = thumb;
    });
    let lang = if opts.language.is_empty() { state.settings().language } else { opts.language.clone() };
    let mut segments: Option<(Vec<Segment>, String)> = None;

    if opts.source != "whisper" && opts.task != "translate" {
        if let Some((key, auto)) = ytdlp::pick_caption(&info, &lang) {
            state.update(id, |j| j.stage = format!("Pegando legenda {}…", if auto { "automática" } else { "oficial" }));
            match ytdlp::fetch_caption(state, &opts.url, &key, auto, tmp, token).await {
                Ok(p) => {
                    let segs = parse_vtt(&std::fs::read_to_string(&p)?);
                    if !segs.is_empty() {
                        let l = key.split(['-', '_']).next().unwrap_or(&key).to_string();
                        segments = Some((segs, format!("{l} · legenda {}", if auto { "automática" } else { "oficial" })));
                    }
                }
                Err(e) if token.is_cancelled() => return Err(e),
                Err(_) => {}
            }
        }
        if segments.is_none() && opts.source == "captions" {
            bail!("Esse vídeo não tem legenda no idioma pedido. Use a fonte \"Whisper\" para transcrever o áudio.");
        }
    }

    let segments = match segments {
        Some(s) => s,
        None => {
            state.update(id, |j| j.stage = "Baixando o áudio…".into());
            let audio = ytdlp::fetch_audio(state, id, &opts.url, tmp, token).await?;
            state.update(id, |j| {
                j.stage = "Decodificando áudio…".into();
                j.progress = -1.0;
            });
            let pcm = decode_pcm(state, &audio, token).await?;
            let (segs, code) = run_whisper(state, id, pcm, opts, token).await?;
            (segs, format!("{code} · Whisper"))
        }
    };
    finish(state, id, &format!("{title} [{vid}]"), segments)
}

pub async fn from_file(state: &Shared, id: &str, file: PathBuf, opts: &TranscribeOpts, token: &CancellationToken) -> Result<()> {
    let res = async {
        state.update(id, |j| {
            j.status = "running".into();
            j.stage = "Decodificando áudio…".into();
        });
        let pcm = decode_pcm(state, &file, token).await?;
        if pcm.is_empty() {
            bail!("o arquivo não tem áudio");
        }
        let (segs, code) = run_whisper(state, id, pcm, opts, token).await?;
        let title = state.get(id).map(|j| j.title).unwrap_or_else(|| "arquivo".into());
        finish(state, id, &title, (segs, format!("{code} · Whisper")))
    }
    .await;
    let _ = std::fs::remove_file(&file);
    res
}

fn finish(state: &Shared, id: &str, title: &str, (segs, lang): (Vec<Segment>, String)) -> Result<()> {
    if segs.is_empty() {
        bail!("Não encontrei fala nesse áudio.");
    }
    let files = write_outputs(state, title, &segs)?;
    let text = to_text(&segs);
    state.update(id, |j| {
        j.transcript = Some(text);
        j.segments = segs;
        j.language = lang;
        j.files = files;
    });
    Ok(())
}
