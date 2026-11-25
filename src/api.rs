//! Servidor HTTP: serve a interface e a API usada por ela (desktop e web).

use crate::engines;
use crate::state::{AppState, FileOut, Settings, Shared};
use crate::transcribe::{self, TranscribeOpts};
use crate::ytdlp::{self, DownloadOpts};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn err(code: StatusCode, msg: impl ToString) -> (StatusCode, Json<Value>) {
    (code, Json(json!({ "error": msg.to_string() })))
}

macro_rules! asset {
    ($path:literal, $mime:literal) => {
        get(|| async { ([(header::CONTENT_TYPE, $mime), (header::CACHE_CONTROL, "no-cache")], include_str!($path)) })
    };
}

pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/api/info", get(info))
        .route("/api/engines", get(engines_status))
        .route("/api/engines/install", post(engines_install))
        .route("/api/settings", get(get_settings).post(set_settings))
        .route("/api/probe", post(probe))
        .route("/api/download", post(download))
        .route("/api/transcribe", post(transcribe_url))
        .route("/api/transcribe/upload", post(transcribe_upload))
        .route("/api/jobs", get(jobs))
        .route("/api/jobs/{id}", get(job).delete(remove_job))
        .route("/api/jobs/{id}/retry", post(retry_job))
        .route("/api/jobs/clear", post(clear_jobs))
        .route("/api/files/{id}/{idx}", get(file))
        .route("/api/reveal/{id}", post(reveal))
        .route("/api/open-output", post(open_output))
        .route("/api/quit", post(quit))
        .route("/api/events", get(events))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .route("/", asset!("../ui/index.html", "text/html; charset=utf-8"))
        .route("/app.js", asset!("../ui/app.js", "text/javascript; charset=utf-8"))
        .route("/style.css", asset!("../ui/style.css", "text/css; charset=utf-8"))
        .route("/icon.svg", asset!("../ui/icon.svg", "image/svg+xml"))
        .route("/manifest.webmanifest", asset!("../ui/manifest.webmanifest", "application/manifest+json"))
        .route("/sw.js", asset!("../ui/sw.js", "text/javascript; charset=utf-8"))
        .route("/api/login", post(login))
        .merge(api)
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024 * 1024))
        .with_state(state)
}

// ───────────────────────── autenticação (modo web) ─────────────────────────

fn token_from(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-capivara-token").and_then(|v| v.to_str().ok()) {
        return Some(v.to_string());
    }
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').find_map(|kv| kv.trim().strip_prefix("capivara_token=").map(str::to_string)))
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn auth(State(st): State<Shared>, req: Request, next: Next) -> Response {
    if let Some(expected) = &st.token {
        if req.uri().path() != "/api/info" {
            let ok = token_from(req.headers()).map(|t| ct_eq(&t, expected)).unwrap_or(false);
            if !ok {
                return err(StatusCode::UNAUTHORIZED, "senha necessária").into_response();
            }
        }
    }
    next.run(req).await
}

#[derive(Deserialize)]
struct Login {
    token: String,
}

async fn login(State(st): State<Shared>, Json(b): Json<Login>) -> Response {
    match &st.token {
        Some(t) if ct_eq(t, &b.token) => {
            let cookie = format!("capivara_token={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000", b.token);
            ([(header::SET_COOKIE, cookie)], Json(json!({"ok": true}))).into_response()
        }
        None => Json(json!({"ok": true})).into_response(),
        _ => err(StatusCode::UNAUTHORIZED, "senha errada").into_response(),
    }
}

async fn info(State(st): State<Shared>, headers: HeaderMap) -> Json<Value> {
    let authed = match &st.token {
        Some(t) => token_from(&headers).map(|x| ct_eq(&x, t)).unwrap_or(false),
        None => true,
    };
    Json(json!({
        "app": "capivara",
        "version": env!("CARGO_PKG_VERSION"),
        "mode": if st.desktop { "desktop" } else { "server" },
        "portable": engines::is_portable(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "auth_required": st.token.is_some(),
        "authed": authed,
    }))
}

// ───────────────────────── motores e configurações ─────────────────────────

async fn engines_status(State(st): State<Shared>) -> Json<Value> {
    let (tools, models) = engines::status(&st.paths).await;
    Json(json!({ "tools": tools, "models": models }))
}

#[derive(Deserialize)]
struct InstallReq {
    what: String,
}

async fn engines_install(State(st): State<Shared>, Json(b): Json<InstallReq>) -> ApiResult<Json<Value>> {
    let what = b.what.clone();
    let label = match what.as_str() {
        "yt-dlp" => "Motor yt-dlp".to_string(),
        "ffmpeg" => "Motor ffmpeg".to_string(),
        "deno" => "Motor deno (JS p/ YouTube)".to_string(),
        "update-yt-dlp" => "Atualizando yt-dlp".to_string(),
        m if m.starts_with("model:") => format!("Modelo Whisper {}", &m[6..]),
        _ => return Err(err(StatusCode::BAD_REQUEST, "motor desconhecido")),
    };
    let (job, token) = st.new_job("setup", "", "setup");
    st.update(&job.id, |j| j.title = label);
    let st2 = st.clone();
    let id = job.id.clone();
    tokio::spawn(async move {
        st2.update(&id, |j| {
            j.status = "running".into();
            j.stage = "Baixando…".into();
        });
        let fut = async {
            if what == "update-yt-dlp" {
                // yt-dlp instalado pelo app: baixar de novo é a forma mais robusta de atualizar.
                engines::install(&st2.paths, "yt-dlp", |p, m| st2.update(&id, |j| { j.progress = p; j.stage = m; })).await?;
            } else {
                engines::install(&st2.paths, &what, |p, m| st2.update(&id, |j| { j.progress = p; j.stage = m; })).await?;
            }
            anyhow::Ok(())
        };
        let res = tokio::select! {
            r = fut => r,
            _ = token.cancelled() => Err(anyhow::anyhow!("cancelado")),
        };
        st2.finish(&id, res, &token);
        let _ = st2.events.send(json!({"type": "engines"}).to_string());
    });
    Ok(Json(json!({ "id": job.id })))
}

async fn get_settings(State(st): State<Shared>) -> Json<Settings> {
    Json(st.settings())
}

async fn set_settings(State(st): State<Shared>, Json(s): Json<Settings>) -> ApiResult<Json<Settings>> {
    if !st.desktop && s.output_dir != st.settings().output_dir {
        // No modo servidor a pasta é do servidor; só aceitamos mudar se existir.
        if !std::path::Path::new(&s.output_dir).is_dir() {
            return Err(err(StatusCode::BAD_REQUEST, "pasta não existe no servidor"));
        }
    }
    st.save_settings(s.clone()).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(s))
}

// ───────────────────────── preview / download / transcrição ─────────────────────────

#[derive(Deserialize)]
struct UrlReq {
    url: String,
}

fn clean_url(u: &str) -> Result<String, (StatusCode, Json<Value>)> {
    let u = u.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err(err(StatusCode::BAD_REQUEST, "cole um link começando com http(s)://"));
    }
    Ok(u.to_string())
}

async fn probe(State(st): State<Shared>, Json(b): Json<UrlReq>) -> ApiResult<Json<ytdlp::Probe>> {
    let url = clean_url(&b.url)?;
    ytdlp::probe(&st, &url)
        .await
        .map(Json)
        .map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, crate::state::friendly_error(&format!("{e:#}"))))
}

fn spawn_download(st: &Shared, o: DownloadOpts) -> String {
    let (job, token) = st.new_job(&o.kind, &o.url, &o.platform);
    let st2 = st.clone();
    let id = job.id.clone();
    tokio::spawn(async move {
        let res = async {
            let _permit = tokio::select! {
                p = st2.downloads.acquire() => p?,
                _ = token.cancelled() => anyhow::bail!("cancelado"),
            };
            st2.update(&id, |j| {
                j.status = "running".into();
                j.stage = "Conectando…".into();
            });
            ytdlp::download(&st2, &id, &o, &token).await
        }
        .await;
        st2.finish(&id, res, &token);
    });
    job.id
}

fn spawn_transcribe(st: &Shared, o: TranscribeOpts) -> String {
    let (job, token) = st.new_job("transcript", &o.url, &o.platform);
    let st2 = st.clone();
    let id = job.id.clone();
    tokio::spawn(async move {
        let res = transcribe::from_url(&st2, &id, &o, &token).await;
        st2.finish(&id, res, &token);
    });
    job.id
}

async fn download(State(st): State<Shared>, Json(mut o): Json<DownloadOpts>) -> ApiResult<Json<Value>> {
    o.url = clean_url(&o.url)?;
    let id = spawn_download(&st, o);
    Ok(Json(json!({ "id": id })))
}

async fn transcribe_url(State(st): State<Shared>, Json(mut o): Json<TranscribeOpts>) -> ApiResult<Json<Value>> {
    o.url = clean_url(&o.url)?;
    let id = spawn_transcribe(&st, o);
    Ok(Json(json!({ "id": id })))
}

async fn transcribe_upload(State(st): State<Shared>, mut mp: Multipart) -> ApiResult<Json<Value>> {
    let mut opts = TranscribeOpts { source: "whisper".into(), ..Default::default() };
    let mut saved: Option<(std::path::PathBuf, String)> = None;
    while let Some(mut field) = mp.next_field().await.map_err(|e| err(StatusCode::BAD_REQUEST, e))? {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            let fname = field.file_name().unwrap_or("arquivo").to_string();
            let ext = std::path::Path::new(&fname).extension().and_then(|e| e.to_str()).unwrap_or("bin").to_string();
            let path = st.paths.tmp.join(format!("up-{}.{}", uuid::Uuid::new_v4().simple(), ext));
            let mut f = tokio::fs::File::create(&path).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
            use tokio::io::AsyncWriteExt;
            while let Some(chunk) = field.chunk().await.map_err(|e| err(StatusCode::BAD_REQUEST, e))? {
                f.write_all(&chunk).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e))?;
            }
            f.flush().await.ok();
            let title = std::path::Path::new(&fname).file_stem().and_then(|s| s.to_str()).unwrap_or("arquivo").to_string();
            saved = Some((path, title));
        } else {
            let v = field.text().await.unwrap_or_default();
            match name.as_str() {
                "language" => opts.language = v,
                "task" => opts.task = v,
                "model" => opts.model = v,
                _ => {}
            }
        }
    }
    let (path, title) = saved.ok_or_else(|| err(StatusCode::BAD_REQUEST, "nenhum arquivo enviado"))?;
    let (job, token) = st.new_job("transcript", "", "file");
    st.update(&job.id, |j| j.title = title);
    let st2 = st.clone();
    let id = job.id.clone();
    tokio::spawn(async move {
        let res = transcribe::from_file(&st2, &id, path, &opts, &token).await;
        st2.finish(&id, res, &token);
    });
    Ok(Json(json!({ "id": job.id })))
}

// ───────────────────────── fila ─────────────────────────

async fn jobs(State(st): State<Shared>) -> Json<Value> {
    Json(json!(st.list()))
}

async fn job(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    st.get(&id).map(|j| Json(json!(j))).ok_or_else(|| err(StatusCode::NOT_FOUND, "tarefa não existe"))
}

async fn remove_job(State(st): State<Shared>, Path(id): Path<String>) -> StatusCode {
    let running = st.cancels.lock().unwrap().get(&id).cloned();
    match running {
        // 1º clique cancela; se já terminou, remove da lista.
        Some(t) => t.cancel(),
        None => st.remove(&id),
    }
    StatusCode::NO_CONTENT
}

async fn retry_job(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let j = st.get(&id).ok_or_else(|| err(StatusCode::NOT_FOUND, "tarefa não existe"))?;
    if j.url.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "não dá pra repetir essa tarefa"));
    }
    st.remove(&id);
    let new = if j.kind == "transcript" {
        spawn_transcribe(&st, TranscribeOpts { url: j.url, platform: j.platform, ..Default::default() })
    } else {
        spawn_download(&st, DownloadOpts { url: j.url, kind: j.kind.clone(), platform: j.platform, format: if j.kind == "audio" { "mp3".into() } else { "mp4".into() }, ..Default::default() })
    };
    Ok(Json(json!({ "id": new })))
}

async fn clear_jobs(State(st): State<Shared>) -> StatusCode {
    let done: Vec<String> = st.list().into_iter().filter(|j| !matches!(j.status.as_str(), "running" | "queued")).map(|j| j.id).collect();
    for id in done {
        st.remove(&id);
    }
    StatusCode::NO_CONTENT
}

fn job_file(st: &AppState, id: &str, idx: usize) -> Option<FileOut> {
    st.jobs.lock().unwrap().get(id).and_then(|j| j.files.get(idx).cloned())
}

/// Entrega o arquivo para o navegador (essencial no modo web).
async fn file(State(st): State<Shared>, Path((id, idx)): Path<(String, usize)>) -> ApiResult<Response> {
    let f = job_file(&st, &id, idx).ok_or_else(|| err(StatusCode::NOT_FOUND, "arquivo não existe"))?;
    let file = tokio::fs::File::open(&f.path).await.map_err(|e| err(StatusCode::NOT_FOUND, e))?;
    let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let stream = tokio_util::io::ReaderStream::new(file);
    let ascii: String = f.name.chars().map(|c| if c.is_ascii_alphanumeric() || " .-_[]()".contains(c) { c } else { '_' }).collect();
    let encoded: String = f.name.bytes().map(|b| if b.is_ascii_alphanumeric() || b".-_".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    let disp = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}");
    let mut resp = Response::new(Body::from_stream(stream));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    if let Ok(v) = HeaderValue::from_str(&disp) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}

fn reveal_path(p: &std::path::Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = open::that_detached(p.parent().unwrap_or(p));
    }
}

async fn reveal(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    if !st.desktop {
        return Err(err(StatusCode::FORBIDDEN, "só no app desktop"));
    }
    let f = job_file(&st, &id, 0).ok_or_else(|| err(StatusCode::NOT_FOUND, "arquivo não existe"))?;
    reveal_path(&f.path);
    Ok(StatusCode::NO_CONTENT)
}

async fn open_output(State(st): State<Shared>) -> ApiResult<StatusCode> {
    if !st.desktop {
        return Err(err(StatusCode::FORBIDDEN, "só no app desktop"));
    }
    let _ = open::that_detached(st.output_dir());
    Ok(StatusCode::NO_CONTENT)
}

async fn quit(State(st): State<Shared>) -> StatusCode {
    if st.desktop {
        st.shutdown.cancel();
    }
    StatusCode::NO_CONTENT
}

async fn events(State(st): State<Shared>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let hello = futures_util::stream::once(async { Ok(Event::default().data(json!({"type": "hello"}).to_string())) });
    let rx = BroadcastStream::new(st.events.subscribe()).filter_map(|m| async move {
        match m {
            Ok(s) => Some(Ok(Event::default().data(s))),
            // Cliente atrasou: manda ressincronizar.
            Err(_) => Some(Ok(Event::default().data(json!({"type": "resync"}).to_string()))),
        }
    });
    Sse::new(hello.chain(rx)).keep_alive(KeepAlive::default())
}
