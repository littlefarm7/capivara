//! Estado compartilhado: configurações, fila de tarefas e canal de eventos (SSE).

use crate::engines::Paths;
use crate::whisper;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub output_dir: String,
    /// "" | chrome | firefox | edge | brave | opera | vivaldi | safari | chromium
    pub cookies_browser: String,
    pub cookies_file: String,
    pub whisper_model: String,
    /// idioma padrão de transcrição ("auto" detecta)
    pub language: String,
    pub max_parallel: usize,
}

impl Default for Settings {
    fn default() -> Self {
        let out = match std::env::var("CAPIVARA_OUTPUT") {
            Ok(p) if !p.is_empty() => PathBuf::from(p),
            _ => dirs::download_dir()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("Capivara"),
        };
        Self {
            output_dir: out.display().to_string(),
            cookies_browser: String::new(),
            cookies_file: String::new(),
            whisper_model: "base".into(),
            language: "auto".into(),
            max_parallel: 3,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct FileOut {
    pub name: String,
    pub size: u64,
    #[serde(skip)]
    pub path: PathBuf,
}

impl FileOut {
    pub fn from_path(p: PathBuf) -> Self {
        Self {
            name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            size: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
            path: p,
        }
    }
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Job {
    pub id: String,
    /// video | audio | transcript | setup
    pub kind: String,
    pub url: String,
    pub title: String,
    pub thumbnail: String,
    pub platform: String,
    /// queued | running | done | error | canceled
    pub status: String,
    /// 0..1, ou negativo para indeterminado
    pub progress: f32,
    pub stage: String,
    pub speed: String,
    pub eta: String,
    pub files: Vec<FileOut>,
    pub transcript: Option<String>,
    pub segments: Vec<whisper::Segment>,
    pub language: String,
    pub error: Option<String>,
    pub created: u64,
}

pub struct AppState {
    pub paths: Paths,
    pub settings: Mutex<Settings>,
    pub jobs: Mutex<HashMap<String, Job>>,
    pub cancels: Mutex<HashMap<String, CancellationToken>>,
    pub events: broadcast::Sender<String>,
    pub downloads: Semaphore,
    pub cpu: Semaphore,
    pub whisper: Mutex<Option<(String, whisper::Engine)>>,
    pub desktop: bool,
    pub token: Option<String>,
    pub shutdown: CancellationToken,
}

pub type Shared = Arc<AppState>;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl AppState {
    pub fn new(paths: Paths, desktop: bool, token: Option<String>) -> Shared {
        let settings: Settings = std::fs::read_to_string(paths.data.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let (tx, _) = broadcast::channel(512);
        let par = settings.max_parallel.clamp(1, 8);
        Arc::new(Self {
            paths,
            settings: Mutex::new(settings),
            jobs: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            events: tx,
            downloads: Semaphore::new(par),
            cpu: Semaphore::new(1),
            whisper: Mutex::new(None),
            desktop,
            token,
            shutdown: CancellationToken::new(),
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn save_settings(&self, s: Settings) -> anyhow::Result<()> {
        std::fs::write(self.paths.data.join("settings.json"), serde_json::to_string_pretty(&s)?)?;
        *self.settings.lock().unwrap() = s;
        Ok(())
    }

    pub fn output_dir(&self) -> PathBuf {
        let p = PathBuf::from(self.settings().output_dir);
        let _ = std::fs::create_dir_all(&p);
        p
    }

    pub fn new_job(&self, kind: &str, url: &str, platform: &str) -> (Job, CancellationToken) {
        let job = Job {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            kind: kind.into(),
            url: url.into(),
            platform: platform.into(),
            status: "queued".into(),
            progress: -1.0,
            stage: "Na fila…".into(),
            created: now(),
            ..Default::default()
        };
        let token = CancellationToken::new();
        self.cancels.lock().unwrap().insert(job.id.clone(), token.clone());
        self.jobs.lock().unwrap().insert(job.id.clone(), job.clone());
        self.emit(&job);
        (job, token)
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut Job)) {
        let snapshot = {
            let mut jobs = self.jobs.lock().unwrap();
            let Some(j) = jobs.get_mut(id) else { return };
            f(j);
            j.clone()
        };
        self.emit(&snapshot);
    }

    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    fn emit(&self, job: &Job) {
        // Transcrição completa pode ser grande — o SSE manda sem segmentos.
        let mut light = job.clone();
        light.segments.clear();
        if let Ok(s) = serde_json::to_string(&serde_json::json!({"type": "job", "job": light})) {
            let _ = self.events.send(s);
        }
    }

    pub fn remove(&self, id: &str) {
        if let Some(t) = self.cancels.lock().unwrap().remove(id) {
            t.cancel();
        }
        self.jobs.lock().unwrap().remove(id);
        let _ = self.events.send(serde_json::json!({"type": "removed", "id": id}).to_string());
    }

    pub fn list(&self) -> Vec<Job> {
        let mut v: Vec<Job> = self
            .jobs
            .lock()
            .unwrap()
            .values()
            .cloned()
            .map(|mut j| {
                j.segments.clear();
                j
            })
            .collect();
        v.sort_by(|a, b| b.created.cmp(&a.created));
        v
    }

    /// Marca fim da tarefa (sucesso, erro ou cancelamento).
    pub fn finish(&self, id: &str, res: anyhow::Result<()>, token: &CancellationToken) {
        self.cancels.lock().unwrap().remove(id);
        match res {
            Ok(()) => self.update(id, |j| {
                j.status = "done".into();
                j.progress = 1.0;
                j.stage = "Pronto!".into();
                j.speed.clear();
                j.eta.clear();
            }),
            Err(_) if token.is_cancelled() => self.update(id, |j| {
                j.status = "canceled".into();
                j.stage = "Cancelado".into();
            }),
            Err(e) => {
                let msg = friendly_error(&format!("{e:#}"));
                self.update(id, |j| {
                    j.status = "error".into();
                    j.stage = "Deu ruim".into();
                    j.error = Some(msg);
                })
            }
        }
    }
}

/// Traduz erros comuns do yt-dlp em algo útil para humanos.
pub fn friendly_error(raw: &str) -> String {
    let l = raw.to_lowercase();
    let hint = if l.contains("login") || l.contains("cookies") || l.contains("rate-limit") || l.contains("sign in") {
        Some("Esse conteúdo exige login. Em Configurações, escolha o navegador onde você está logado (cookies) e tente de novo.")
    } else if l.contains("ip address is blocked") || l.contains("not available in your country") || l.contains("geo restrict") || l.contains("geo-restrict") {
        Some("O site bloqueou esse conteúdo para a sua região/IP.")
    } else if l.contains("private") {
        Some("O conteúdo é privado.")
    } else if l.contains("unsupported url") {
        Some("Esse link não é suportado.")
    } else if l.contains("not found") && l.contains("yt-dlp") {
        Some("Motor yt-dlp não instalado — abra Motores e instale.")
    } else if l.contains("ffmpeg") && (l.contains("not found") || l.contains("not installed")) {
        Some("ffmpeg não instalado — abra Motores e instale.")
    } else if l.contains("http error 403") || l.contains("challenge") || l.contains("js runtime") {
        Some("O site bloqueou o acesso. Atualize o yt-dlp em Motores (e confira se o deno está instalado).")
    } else {
        None
    };
    let last = raw
        .lines()
        .filter(|s| !s.trim().is_empty())
        .last()
        .unwrap_or(raw)
        .trim()
        .trim_start_matches("ERROR: ")
        .chars()
        .take(400)
        .collect::<String>();
    match hint {
        Some(h) => format!("{h}\n\n{last}"),
        None => last,
    }
}
