//! Transcrição local com Whisper (OpenAI) rodando 100% em Rust via `candle`.
//! Nada sai da máquina: o áudio é decodificado pelo ffmpeg e processado aqui.

use anyhow::{Context, Result, bail};
use crate::whisper_model::{Config, Model};
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as m, audio};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, serde::Serialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Transcribe,
    Translate,
}

/// Tokenizer mínimo do Whisper (BPE em bytes, estilo GPT-2) — só precisamos decodificar.
struct Tokenizer {
    id_to_tok: Vec<String>,
    tok_to_id: HashMap<String, u32>,
    byte_dec: HashMap<char, u8>,
}

impl Tokenizer {
    fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).context("lendo tokenizer.json")?;
        let v: serde_json::Value = serde_json::from_str(&raw)?;
        let mut tok_to_id = HashMap::new();
        if let Some(vocab) = v["model"]["vocab"].as_object() {
            for (k, id) in vocab {
                if let Some(id) = id.as_u64() {
                    tok_to_id.insert(k.clone(), id as u32);
                }
            }
        }
        if let Some(added) = v["added_tokens"].as_array() {
            for t in added {
                if let (Some(id), Some(c)) = (t["id"].as_u64(), t["content"].as_str()) {
                    tok_to_id.insert(c.to_string(), id as u32);
                }
            }
        }
        let max = tok_to_id.values().copied().max().unwrap_or(0) as usize;
        let mut id_to_tok = vec![String::new(); max + 1];
        for (k, &id) in &tok_to_id {
            id_to_tok[id as usize] = k.clone();
        }
        // Inverso do bytes_to_unicode do GPT-2.
        let mut bs: Vec<u32> = (b'!' as u32..=b'~' as u32)
            .chain(0xA1..=0xAC)
            .chain(0xAE..=0xFF)
            .collect();
        let mut cs = bs.clone();
        let mut n = 0;
        for b in 0..256u32 {
            if !bs.contains(&b) {
                bs.push(b);
                cs.push(256 + n);
                n += 1;
            }
        }
        let byte_dec = bs
            .iter()
            .zip(cs.iter())
            .map(|(&b, &c)| (char::from_u32(c).unwrap(), b as u8))
            .collect();
        Ok(Self { id_to_tok, tok_to_id, byte_dec })
    }

    fn id(&self, tok: &str) -> Result<u32> {
        self.tok_to_id
            .get(tok)
            .copied()
            .with_context(|| format!("token {tok} ausente no tokenizer"))
    }

    fn decode(&self, ids: &[u32]) -> String {
        let mut bytes = Vec::new();
        for &id in ids {
            if let Some(t) = self.id_to_tok.get(id as usize) {
                for ch in t.chars() {
                    if let Some(&b) = self.byte_dec.get(&ch) {
                        bytes.push(b);
                    }
                }
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Banco de filtros mel (mesma fórmula do librosa usada pela OpenAI: escala/normalização "slaney").
fn mel_filters(n_mels: usize) -> Vec<f32> {
    let sr = m::SAMPLE_RATE as f64;
    let n_fft = m::N_FFT;
    let n_freqs = n_fft / 2 + 1;
    let hz_to_mel = |f: f64| {
        let f_sp = 200.0 / 3.0;
        let min_log_hz = 1000.0;
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f64).ln() / 27.0;
        if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp }
    };
    let mel_to_hz = |m: f64| {
        let f_sp = 200.0 / 3.0;
        let min_log_hz = 1000.0;
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f64).ln() / 27.0;
        if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { f_sp * m }
    };
    let fftfreqs: Vec<f64> = (0..n_freqs).map(|i| i as f64 * sr / n_fft as f64).collect();
    let (mmin, mmax) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..n_mels + 2)
        .map(|i| mel_to_hz(mmin + (mmax - mmin) * i as f64 / (n_mels + 1) as f64))
        .collect();
    let mut w = vec![0f32; n_mels * n_freqs];
    for i in 0..n_mels {
        let enorm = 2.0 / (mel_f[i + 2] - mel_f[i]);
        for (j, &f) in fftfreqs.iter().enumerate() {
            let lower = (f - mel_f[i]) / (mel_f[i + 1] - mel_f[i]);
            let upper = (mel_f[i + 2] - f) / (mel_f[i + 2] - mel_f[i + 1]);
            w[i * n_freqs + j] = (lower.min(upper).max(0.0) * enorm) as f32;
        }
    }
    w
}

/// Menos tentativas que o Whisper original (6): em CPU cada uma custa caro.
const TEMPERATURES: [f64; 4] = [0.0, 0.3, 0.6, 0.9];

struct WindowResult {
    tokens: Vec<u32>,
    avg_logprob: f64,
    no_speech_prob: f64,
    looped: bool,
}

/// O mesmo bloco de 1..=12 tokens repetido 5× seguidas no fim = "alucinação" em loop.
fn looping(toks: &[u32]) -> bool {
    const REPS: usize = 5;
    (1..=12).any(|p| {
        let need = p * REPS;
        if toks.len() < need {
            return false;
        }
        let tail = &toks[toks.len() - need..];
        (1..REPS).all(|r| tail[r * p..(r + 1) * p] == tail[..p])
    })
}

pub struct Engine {
    model: Model,
    tok: Tokenizer,
    device: Device,
    filters: Vec<f32>,
    sot: u32,
    eot: u32,
    transcribe: u32,
    translate: u32,
    no_speech: Option<u32>,
    ts_begin: u32,
    suppress: Vec<u32>,
}

impl Engine {
    /// `dir` precisa ter config.json, tokenizer.json e model.safetensors.
    pub fn load(dir: &Path) -> Result<Self> {
        let device = Device::Cpu;
        let config: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let tok = Tokenizer::load(&dir.join("tokenizer.json"))?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], m::DTYPE, &device)?
        };
        let filters = mel_filters(config.num_mel_bins);
        let suppress = config.suppress_tokens.clone();
        let model = Model::load(&vb, config)?;
        let no_ts = tok.id(m::NO_TIMESTAMPS_TOKEN)?;
        let no_speech = m::NO_SPEECH_TOKENS.iter().find_map(|t| tok.id(t).ok());
        Ok(Self {
            sot: tok.id(m::SOT_TOKEN)?,
            eot: tok.id(m::EOT_TOKEN)?,
            transcribe: tok.id(m::TRANSCRIBE_TOKEN)?,
            translate: tok.id(m::TRANSLATE_TOKEN)?,
            ts_begin: no_ts + 1,
            no_speech,
            suppress,
            filters,
            device,
            model,
            tok,
        })
    }

    fn language_ids(&self) -> std::ops::Range<u32> {
        // Tokens de idioma ficam entre <|startoftranscript|> e <|translate|>.
        (self.sot + 1)..self.translate.min(self.transcribe)
    }

    /// Alimenta `new` tokens no decoder (incremental) e devolve logits da última posição
    /// (e da primeira, se pedido — usada para a probabilidade de "sem fala").
    fn step(&mut self, new: &[u32], af: &Tensor, want_first: bool) -> Result<(Vec<f32>, Option<Vec<f32>>)> {
        let h = self.model.decoder.forward(new, af)?;
        let last = self.model.decoder.logits(&h, new.len() - 1)?;
        let first = if want_first { Some(self.model.decoder.logits(&h, 0)?) } else { None };
        Ok((last, first))
    }

    fn detect_language(&mut self, af: &Tensor) -> Result<u32> {
        self.model.decoder.reset();
        let (logits, _) = self.step(&[self.sot], af, false)?;
        let r = self.language_ids();
        let best = r
            .clone()
            .filter(|&i| (i as usize) < logits.len())
            .max_by(|&a, &b| logits[a as usize].total_cmp(&logits[b as usize]))
            .context("sem tokens de idioma")?;
        Ok(best)
    }

    pub fn language_code(&self, id: u32) -> String {
        self.tok.id_to_tok.get(id as usize).map(|s| s.trim_matches(|c| c == '<' || c == '|' || c == '>').to_string()).unwrap_or_default()
    }

    fn decode_window(&mut self, af: &Tensor, lang: u32, task: Task, temperature: f64, rng: &mut u64) -> Result<WindowResult> {
        let max_len = self.model.config.max_target_positions / 2;
        let mut tokens = vec![self.sot, lang, if task == Task::Transcribe { self.transcribe } else { self.translate }];
        let sample_begin = tokens.len();
        let mut sum_logprob = 0f64;
        let mut no_speech_prob = 0f64;
        let mut looped = false;
        let vocab = self.model.config.vocab_size;
        self.model.decoder.reset();
        let mut pending = tokens.clone();
        for i in 0..max_len {
            let (mut logits, first) = self.step(&pending, af, i == 0)?;
            logits.truncate(vocab);
            if let (Some(first), Some(ns)) = (first, self.no_speech) {
                let p = softmax(&first);
                no_speech_prob = p.get(ns as usize).copied().unwrap_or(0.0) as f64;
            }
            let ninf = f32::NEG_INFINITY;
            for &s in &self.suppress {
                if (s as usize) < logits.len() {
                    logits[s as usize] = ninf;
                }
            }
            // Tokens especiais (exceto EOT e timestamps) nunca são gerados.
            for id in (self.eot + 1)..self.ts_begin {
                logits[id as usize] = ninf;
            }
            let ts = self.ts_begin as usize;
            let sampled = &tokens[sample_begin..];
            let is_ts = |t: u32| t >= self.ts_begin;
            if sampled.is_empty() {
                logits[..ts].fill(ninf); // começa sempre com timestamp
                let max_initial = ts + 50; // até 1.0s
                if max_initial + 1 < logits.len() {
                    logits[max_initial + 1..].fill(ninf);
                }
            } else {
                let last_ts = is_ts(*sampled.last().unwrap());
                let penult_ts = sampled.len() < 2 || is_ts(sampled[sampled.len() - 2]);
                if last_ts {
                    if penult_ts {
                        logits[ts..].fill(ninf);
                    } else {
                        logits[..self.eot as usize].fill(ninf);
                    }
                }
                if let Some(&last) = sampled.iter().rev().find(|&&t| is_ts(t)) {
                    let lim = (if last_ts && !penult_ts { last } else { last + 1 } as usize).min(logits.len());
                    logits[ts..lim].fill(ninf);
                }
            }
            let logp = log_softmax(&logits);
            let ts_lp = logsumexp(&logp[ts..]);
            let max_text = logp[..ts].iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if ts_lp > max_text {
                logits[..ts].fill(ninf);
            }
            let logp = log_softmax(&logits);
            let next = if temperature > 0.0 {
                sample(&logits, temperature as f32, rng)
            } else {
                argmax(&logits)
            };
            sum_logprob += logp[next as usize] as f64;
            tokens.push(next);
            pending = vec![next];
            if next == self.eot {
                break;
            }
            if looping(&tokens[sample_begin..]) {
                looped = true;
                break;
            }
        }
        let out: Vec<u32> = tokens[sample_begin..].iter().copied().filter(|&t| t != self.eot).collect();
        let n = out.len().max(1) as f64;
        Ok(WindowResult { tokens: out, avg_logprob: sum_logprob / n, no_speech_prob, looped })
    }

    /// Transcreve PCM mono 16 kHz. `lang`: código ("pt", "en"...) ou None para detectar.
    pub fn transcribe(
        &mut self,
        pcm: &[f32],
        lang: Option<&str>,
        task: Task,
        cancel: &AtomicBool,
        mut progress: impl FnMut(f32, &[Segment]),
    ) -> Result<(Vec<Segment>, String)> {
        let content_frames = pcm.len() / m::HOP_LENGTH;
        let mut padded = pcm.to_vec();
        padded.extend(std::iter::repeat_n(0f32, m::N_SAMPLES));
        let mel = audio::pcm_to_mel(&self.model.config, &padded, &self.filters);
        let n_mels = self.model.config.num_mel_bins;
        let mel_len = mel.len() / n_mels;
        let mel = Tensor::from_vec(mel, (1, n_mels, mel_len), &self.device)?;

        let mut lang_id = match lang {
            Some(code) if !code.is_empty() && code != "auto" => Some(self.tok.id(&format!("<|{code}|>"))?),
            _ => None,
        };
        let mut segments: Vec<Segment> = Vec::new();
        let mut seek = 0usize;
        let mut rng = 0x9E3779B97F4A7C15u64;
        while seek < content_frames {
            if cancel.load(Ordering::Relaxed) {
                bail!("cancelado");
            }
            let seg_size = (content_frames - seek).min(m::N_FRAMES);
            let window = mel.narrow(2, seek, m::N_FRAMES.min(mel_len - seek))?;
            let af = self.model.encoder.forward(&window)?;
            let lid = match lang_id {
                Some(l) => l,
                None => {
                    let l = self.detect_language(&af)?;
                    lang_id = Some(l);
                    l
                }
            };
            // Fallback de temperatura: tenta de novo com amostragem se a saída parecer ruim
            // e, se nenhuma for boa, fica com a mais confiante que não entrou em loop.
            let mut attempts: Vec<(WindowResult, bool)> = vec![];
            for &t in TEMPERATURES.iter() {
                if cancel.load(Ordering::Relaxed) {
                    bail!("cancelado");
                }
                let r = self.decode_window(&af, lid, task, t, &mut rng)?;
                let text = self.tok.decode(&r.tokens.iter().copied().filter(|&x| x < self.eot).collect::<Vec<_>>());
                let repetitive = r.looped || is_repetitive(&text);
                let bad = repetitive || r.avg_logprob < m::LOGPROB_THRESHOLD;
                let silent = r.no_speech_prob > m::NO_SPEECH_THRESHOLD;
                let stop = !bad || silent;
                attempts.push((r, repetitive));
                if stop {
                    break;
                }
            }
            let pick = if attempts.len() == 1 || {
                let (r, rep) = attempts.last().unwrap();
                !rep && r.avg_logprob >= m::LOGPROB_THRESHOLD
            } {
                attempts.len() - 1
            } else {
                attempts
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, rep))| !rep)
                    .max_by(|a, b| a.1.0.avg_logprob.total_cmp(&b.1.0.avg_logprob))
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            };
            let r = attempts.swap_remove(pick).0;
            let offset = seek as f64 * m::HOP_LENGTH as f64 / m::SAMPLE_RATE as f64;
            if r.no_speech_prob > m::NO_SPEECH_THRESHOLD && r.avg_logprob < m::LOGPROB_THRESHOLD {
                seek += seg_size;
                progress(seek as f32 / content_frames as f32, &segments);
                continue;
            }
            // Converte tokens em segmentos <|t0|> texto <|t1|>.
            let mut closed: Vec<Segment> = Vec::new();
            let mut last_closed_idx: Option<u32> = None;
            let mut start: Option<f64> = None;
            let mut text_toks: Vec<u32> = Vec::new();
            for &t in &r.tokens {
                if t >= self.ts_begin {
                    let secs = (t - self.ts_begin) as f64 * 0.02;
                    if !text_toks.is_empty() {
                        closed.push(Segment {
                            start: offset + start.unwrap_or(0.0),
                            end: offset + secs,
                            text: self.tok.decode(&text_toks).trim().to_string(),
                        });
                        text_toks.clear();
                        start = None;
                        last_closed_idx = Some(t - self.ts_begin);
                    } else {
                        start = Some(secs);
                    }
                } else if t < self.eot {
                    text_toks.push(t);
                }
            }
            let window_secs = seg_size as f64 * m::HOP_LENGTH as f64 / m::SAMPLE_RATE as f64;
            let advance = if !text_toks.is_empty() {
                if closed.is_empty() {
                    closed.push(Segment {
                        start: offset + start.unwrap_or(0.0),
                        end: offset + window_secs,
                        text: self.tok.decode(&text_toks).trim().to_string(),
                    });
                    seg_size
                } else {
                    // Texto incompleto no fim: redecodifica a partir do último timestamp fechado.
                    (last_closed_idx.unwrap_or(0) as usize * 2).clamp(1, seg_size)
                }
            } else {
                seg_size
            };
            for s in closed {
                if !s.text.is_empty() {
                    segments.push(s);
                }
            }
            seek += advance.max(1);
            progress((seek as f32 / content_frames as f32).min(1.0), &segments);
        }
        let code = lang_id.map(|l| self.language_code(l)).unwrap_or_default();
        Ok((segments, code))
    }
}

fn softmax(x: &[f32]) -> Vec<f32> {
    let mx = x.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let e: Vec<f32> = x.iter().map(|v| (v - mx).exp()).collect();
    let s: f32 = e.iter().sum();
    e.into_iter().map(|v| v / s).collect()
}

fn log_softmax(x: &[f32]) -> Vec<f32> {
    let lse = logsumexp(x);
    x.iter().map(|v| v - lse).collect()
}

fn logsumexp(x: &[f32]) -> f32 {
    let mx = x.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if mx == f32::NEG_INFINITY {
        return mx;
    }
    mx + x.iter().map(|v| (v - mx).exp()).sum::<f32>().ln()
}

fn argmax(x: &[f32]) -> u32 {
    x.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i as u32).unwrap_or(0)
}

fn sample(logits: &[f32], t: f32, rng: &mut u64) -> u32 {
    let scaled: Vec<f32> = logits.iter().map(|v| v / t).collect();
    let p = softmax(&scaled);
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    let mut r = (*rng >> 11) as f64 / (1u64 << 53) as f64;
    for (i, &pi) in p.iter().enumerate() {
        r -= pi as f64;
        if r <= 0.0 {
            return i as u32;
        }
    }
    argmax(logits)
}

/// Heurística simples para "alucinações" em loop (substitui a taxa de compressão gzip).
fn is_repetitive(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 20 {
        return false;
    }
    let uniq: std::collections::HashSet<&str> = words.iter().copied().collect();
    (uniq.len() as f64 / words.len() as f64) < 0.25
}
