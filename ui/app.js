/* Capivara — interface (JS puro, sem build) */
'use strict';

const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

const LANGS = [
  ['auto', 'Detectar'], ['pt', 'Português'], ['en', 'English'], ['es', 'Español'], ['fr', 'Français'],
  ['de', 'Deutsch'], ['it', 'Italiano'], ['ja', '日本語'], ['ko', '한국어'], ['zh', '中文'], ['ru', 'Русский'],
  ['ar', 'العربية'], ['hi', 'हिन्दी'], ['tr', 'Türkçe'], ['nl', 'Nederlands'], ['pl', 'Polski'], ['uk', 'Українська'],
];

const PLATFORMS = [
  { re: /youtube\.com\/shorts\//i, name: 'Shorts', short: 'SHORTS', color: '#FF8A3D', vertical: true },
  { re: /music\.youtube\.com/i, name: 'YouTube Music', short: 'YT MUSIC', color: '#FF4747' },
  { re: /(youtube\.com|youtu\.be)/i, name: 'YouTube', short: 'YT', color: '#FF4747' },
  { re: /tiktok\.com/i, name: 'TikTok', short: 'TIKTOK', color: '#3DE8E0', vertical: true },
  { re: /instagram\.com\/stories\//i, name: 'Instagram Stories', short: 'STORIES', color: '#FFB2CC', vertical: true, login: true },
  { re: /instagram\.com\/(reel|reels)\//i, name: 'Instagram Reels', short: 'REELS', color: '#FF5C8A', vertical: true },
  { re: /instagram\.com/i, name: 'Instagram', short: 'INSTA', color: '#FF5C8A' },
  { re: /(twitter\.com|x\.com)\//i, name: 'X / Twitter', short: 'X', color: '#FFFFFF' },
  { re: /(facebook\.com|fb\.watch)/i, name: 'Facebook', short: 'FB', color: '#4D9DFF' },
  { re: /vimeo\.com/i, name: 'Vimeo', short: 'VIMEO', color: '#7FD4FF' },
  { re: /soundcloud\.com/i, name: 'SoundCloud', short: 'SC', color: '#FFA552', audio: true },
  { re: /twitch\.tv/i, name: 'Twitch', short: 'TWITCH', color: '#A06CFF' },
  { re: /reddit\.com|redd\.it/i, name: 'Reddit', short: 'REDDIT', color: '#FF7A45' },
  { re: /kwai/i, name: 'Kwai', short: 'KWAI', color: '#FF8A3D', vertical: true },
  { re: /pinterest\./i, name: 'Pinterest', short: 'PIN', color: '#FF4747' },
  { re: /threads\.net/i, name: 'Threads', short: 'THREADS', color: '#FFFFFF' },
  { re: /dailymotion\./i, name: 'Dailymotion', short: 'DM', color: '#4D9DFF' },
  { re: /bilibili\./i, name: 'Bilibili', short: 'BILI', color: '#7FD4FF' },
];

const KIND = {
  video: { label: 'Vídeo', ico: '▶', tag: 'g' },
  audio: { label: 'Áudio', ico: '♫', tag: 'b' },
  transcript: { label: 'Transcrição', ico: '✎', tag: 'v' },
  setup: { label: 'Motor', ico: '⚙', tag: 'y' },
};

const S = {
  info: {},
  jobs: new Map(),
  mode: localStorage.getItem('cap.mode') || 'video',
  opts: Object.assign(
    { quality: 'best', vformat: 'mp4', aformat: 'mp3', source: 'auto', task: 'transcribe', compat: true, thumbnail: true, subs: false, playlist: false },
    JSON.parse(localStorage.getItem('cap.opts') || '{}'),
  ),
  probe: null,
  probeSeq: 0,
  engines: null,
  settings: null,
  trJob: null,
};

// ───────────────────────── API ─────────────────────────
async function api(path, { method = 'GET', body, form } = {}) {
  const opt = { method, headers: {}, credentials: 'same-origin' };
  if (form) opt.body = form;
  else if (body !== undefined) { opt.body = JSON.stringify(body); opt.headers['content-type'] = 'application/json'; }
  const r = await fetch(path, opt);
  if (r.status === 401) { showLogin(); throw new Error('senha necessária'); }
  if (r.status === 204) return null;
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(data.error || `erro ${r.status}`);
  return data;
}

function toast(msg, type = 'info', ms = 3800) {
  const t = document.createElement('div');
  t.className = `toast ${type}`;
  t.textContent = msg;
  $('#toasts').append(t);
  setTimeout(() => { t.style.transition = 'opacity .3s, transform .3s'; t.style.opacity = 0; t.style.transform = 'translateX(30px)'; }, ms);
  setTimeout(() => t.remove(), ms + 350);
}

const fmtDur = (s) => {
  if (!s && s !== 0) return '';
  s = Math.round(s);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = s % 60;
  return h ? `${h}:${String(m).padStart(2, '0')}:${String(x).padStart(2, '0')}` : `${m}:${String(x).padStart(2, '0')}`;
};
const fmtSize = (b) => b >= 1e9 ? `${(b / 1e9).toFixed(2)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1e3))} KB`;
const fmtNum = (n) => n >= 1e6 ? `${(n / 1e6).toFixed(1)} mi` : n >= 1e3 ? `${(n / 1e3).toFixed(1)} mil` : String(n);
const platformOf = (url) => PLATFORMS.find((p) => p.re.test(url || ''));

// ───────────────────────── link + preview ─────────────────────────
const urlInput = $('#url');
let probeTimer;

function extractUrl(text) {
  const m = String(text || '').match(/https?:\/\/[^\s"'<>]+/i);
  return m ? m[0] : '';
}

function onUrlChange() {
  const url = urlInput.value.trim();
  $('#btn-clear').hidden = !url;
  const p = platformOf(url);
  const badge = $('#plat-badge');
  badge.textContent = p ? p.short : url ? 'LINK' : '🔗';
  badge.style.background = p ? p.color : url ? 'var(--yellow)' : '';
  badge.classList.remove('pop'); void badge.offsetWidth; if (url) badge.classList.add('pop');
  const hint = $('#url-hint');
  hint.className = 'hint';
  hint.textContent = '';
  if (p?.login && !(S.settings?.cookies_browser || S.settings?.cookies_file)) {
    hint.className = 'hint warn';
    hint.innerHTML = 'Stories exigem login: escolha o navegador onde você está logado em <button class="link" data-open="settings">Ajustes → Cookies</button>.';
  }
  if (p?.audio && S.mode === 'video') setMode('audio');
  clearTimeout(probeTimer);
  updateGo();
  if (!/^https?:\/\/\S+\.\S+/.test(url)) { $('#preview').hidden = true; S.probe = null; return; }
  probeTimer = setTimeout(() => runProbe(url), 450);
}

async function runProbe(url) {
  if (!S.engines?.ytdlp) return; // sem motor, sem preview
  const seq = ++S.probeSeq;
  const pv = $('#preview');
  pv.hidden = false;
  $('#pv-thumb').className = 'thumb skeleton';
  $('#pv-thumb').style.backgroundImage = '';
  $('#pv-thumb').textContent = '';
  $('#pv-title').innerHTML = '<span class="skeleton" style="display:block;height:22px;width:80%"></span>';
  $('#pv-meta').innerHTML = '<span class="skeleton" style="display:block;height:14px;width:45%;margin-top:8px"></span>';
  $('#pv-badges').innerHTML = '';
  try {
    const d = await api('/api/probe', { method: 'POST', body: { url } });
    if (seq !== S.probeSeq) return;
    S.probe = d;
    renderPreview(d, url);
  } catch (e) {
    if (seq !== S.probeSeq) return;
    S.probe = null;
    pv.hidden = true;
    const hint = $('#url-hint');
    hint.className = 'hint err';
    hint.textContent = '⚠ ' + e.message.split('\n')[0];
  }
  updateGo();
}

function renderPreview(d, url) {
  const p = platformOf(url);
  const th = $('#pv-thumb');
  th.className = 'thumb' + (p?.vertical ? ' vertical' : '');
  if (d.thumbnail) th.style.backgroundImage = `url("${d.thumbnail.replace(/"/g, '%22')}")`;
  else { th.textContent = KIND[S.mode]?.ico || '▶'; th.style.background = p?.color || 'var(--yellow)'; }
  $('#pv-title').textContent = d.title || 'Sem título';
  const meta = [d.uploader, d.duration ? fmtDur(d.duration) : '', d.view_count ? `${fmtNum(d.view_count)} views` : ''].filter(Boolean);
  $('#pv-meta').textContent = meta.join(' · ');
  const badges = [];
  badges.push(`<span class="tag" style="background:${p?.color || 'var(--yellow)'};color:#111">${esc(p?.name || d.extractor || 'Link')}</span>`);
  if (d.is_playlist) badges.push(`<span class="tag p">Playlist · ${d.entries} itens</span>`);
  const maxH = Math.max(0, ...(d.heights || []));
  if (maxH) badges.push(`<span class="tag g">até ${maxH >= 2160 ? '4K' : maxH + 'p'}</span>`);
  if (d.subtitles?.length) badges.push(`<span class="tag v">legenda: ${esc(d.subtitles.slice(0, 3).join(', '))}${d.subtitles.length > 3 ? '…' : ''}</span>`);
  else if (d.auto_captions) badges.push('<span class="tag v">legenda automática</span>');
  $('#pv-badges').innerHTML = badges.join('');
  // qualidades possíveis
  $$('[data-key="quality"] .chip').forEach((c) => {
    const v = +c.dataset.v;
    c.disabled = !!(v && maxH && v > maxH);
  });
  if (d.is_playlist) { $$('[data-flag="playlist"]').forEach((t) => t.hidden = false); }
  syncPlaylistToggles();
}

function syncPlaylistToggles() {
  const isPl = !!S.probe?.is_playlist;
  $$('[data-flag="playlist"]').forEach((t) => { t.hidden = !isPl; });
}

// ───────────────────────── modos e opções ─────────────────────────
function setMode(m) {
  S.mode = m;
  localStorage.setItem('cap.mode', m);
  $$('.mode').forEach((b) => b.classList.toggle('is-on', b.dataset.mode === m));
  $$('[data-panel]').forEach((p) => { p.hidden = p.dataset.panel !== m; });
  updateGo();
}

function saveOpts() { localStorage.setItem('cap.opts', JSON.stringify(S.opts)); }

function bindChips() {
  $$('.chips[data-key]').forEach((group) => {
    const key = group.dataset.key;
    const set = (v) => $$('.chip', group).forEach((c) => c.classList.toggle('is-on', c.dataset.v === v));
    set(S.opts[key]);
    group.addEventListener('click', (e) => {
      const c = e.target.closest('.chip');
      if (!c || c.disabled) return;
      S.opts[key] = c.dataset.v;
      set(c.dataset.v);
      saveOpts();
      updateGo();
    });
  });
  $$('.toggle[data-flag]').forEach((t) => {
    const k = t.dataset.flag;
    t.classList.toggle('is-on', !!S.opts[k]);
    t.addEventListener('click', () => {
      S.opts[k] = !S.opts[k];
      $$(`.toggle[data-flag="${k}"]`).forEach((x) => x.classList.toggle('is-on', S.opts[k]));
      saveOpts();
    });
  });
}

function fillSelect(sel, items, value) {
  sel.innerHTML = items.map(([v, l]) => `<option value="${esc(v)}">${esc(l)}</option>`).join('');
  if (value != null) sel.value = value;
}

function modelItems() {
  const ms = S.engines?.models || [];
  return ms.map((m) => [m.name, `${m.label}${m.installed ? '' : ` · baixar ${m.size_mb >= 1000 ? (m.size_mb / 1000).toFixed(1) + ' GB' : m.size_mb + ' MB'}`}`]);
}

function updateGo() {
  const btn = $('#btn-go');
  const url = urlInput.value.trim();
  const labels = { video: 'Baixar vídeo', audio: 'Baixar áudio', transcript: 'Transcrever' };
  let label = labels[S.mode];
  if (S.mode === 'audio') label += ` ${S.opts.aformat.toUpperCase()}`;
  if (S.mode === 'video' && S.opts.quality !== 'best') label += ` ${S.opts.quality === '2160' ? '4K' : S.opts.quality + 'p'}`;
  $('#go-label').textContent = label;
  btn.classList.toggle('is-audio', S.mode === 'audio');
  btn.classList.toggle('is-transcript', S.mode === 'transcript');
  btn.disabled = !/^https?:\/\/\S+\.\S+/.test(url);
}

async function go() {
  const url = urlInput.value.trim();
  if (!url) return urlInput.focus();
  if (!S.engines?.ytdlp) { toast('Instale os motores primeiro ⚡', 'err'); openEngines(); return; }
  const platform = platformOf(url)?.name || S.probe?.extractor || '';
  const btn = $('#btn-go');
  btn.disabled = true;
  try {
    if (S.mode === 'transcript') {
      const model = $('#t-model').value;
      const m = S.engines.models.find((x) => x.name === model);
      if (S.opts.source !== 'captions' && m && !m.installed) {
        if (S.opts.source === 'whisper' || !(S.probe?.subtitles?.length || S.probe?.auto_captions)) {
          toast(`Baixando o modelo ${model} primeiro…`, 'info');
          await api('/api/engines/install', { method: 'POST', body: { what: 'model:' + model } });
        }
      }
      await api('/api/transcribe', { method: 'POST', body: { url, platform, source: S.opts.source, language: $('#t-lang').value, task: S.opts.task, model } });
    } else {
      const audio = S.mode === 'audio';
      await api('/api/download', {
        method: 'POST',
        body: {
          url, platform, kind: S.mode,
          quality: S.opts.quality,
          format: audio ? S.opts.aformat : S.opts.vformat,
          compat: S.opts.compat, thumbnail: S.opts.thumbnail, subs: S.opts.subs,
          playlist: !!(S.probe?.is_playlist && S.opts.playlist),
        },
      });
    }
    toast('Na fila! 🦫', 'ok', 1800);
    urlInput.value = '';
    S.probe = null;
    onUrlChange();
    $('.queue').scrollIntoView({ behavior: 'smooth', block: 'start' });
  } catch (e) {
    toast(e.message, 'err');
  } finally {
    updateGo();
  }
}

// ───────────────────────── fila ─────────────────────────
function jobHTML(j) {
  const k = KIND[j.kind] || KIND.video;
  const running = j.status === 'running' || j.status === 'queued';
  const pct = j.progress >= 0 ? Math.round(j.progress * 100) : null;
  const thumb = j.thumbnail ? `style="background-image:url(&quot;${esc(j.thumbnail)}&quot;)"` : '';
  const tags = [`<span class="tag ${k.tag}">${k.label}</span>`];
  if (j.platform && j.platform !== 'setup' && j.platform !== 'file') tags.push(`<span class="tag">${esc(j.platform)}</span>`);
  if (j.platform === 'file') tags.push('<span class="tag">Arquivo</span>');
  if (j.language) tags.push(`<span class="tag y">${esc(j.language)}</span>`);
  if (j.status === 'done') tags.push('<span class="done-stamp">Pronto!</span>');
  const meta = [j.speed, j.eta ? `faltam ${j.eta}` : ''].filter(Boolean).join(' · ');
  let actions = '';
  if (running) actions += `<button class="btn btn-sm" data-act="cancel">Cancelar</button>`;
  else {
    if (j.status === 'done' && j.kind === 'transcript') actions += `<button class="btn btn-sm" style="--c:var(--purple);color:#111" data-act="view">Ver texto</button>`;
    if (j.status === 'done' && j.files?.length) {
      const f = j.files.find((x) => /\.(mp4|mkv|webm|mp3|m4a|opus|wav|flac|txt)$/i.test(x.name)) || j.files[0];
      const idx = j.files.indexOf(f);
      actions += `<a class="btn btn-sm" style="--c:var(--green);color:#111" href="/api/files/${j.id}/${idx}" download title="${esc(f.name)}">⬇ Salvar${j.files.length > 1 && j.kind !== 'transcript' ? ` (${j.files.length})` : ''}</a>`;
      if (S.info.mode === 'desktop') actions += `<button class="btn btn-sm" data-act="reveal">📂 Pasta</button>`;
    }
    if ((j.status === 'error' || j.status === 'canceled') && j.url) actions += `<button class="btn btn-sm" style="--c:var(--yellow);color:#111" data-act="retry">↻ De novo</button>`;
    actions += `<button class="btn btn-sm" data-act="remove">Remover</button>`;
  }
  const fileList = j.status === 'done' && j.files?.length > 1 && j.kind !== 'transcript'
    ? `<div class="job-tags" style="margin-top:8px">${j.files.map((f, i) => `<a class="tag" href="/api/files/${j.id}/${i}" download>${esc(f.name.slice(0, 40))} · ${fmtSize(f.size)}</a>`).join('')}</div>`
    : '';
  const size = j.status === 'done' && j.files?.length === 1 ? ` · ${fmtSize(j.files[0].size)}` : '';
  return `
    <div class="thumb" ${thumb}>${j.thumbnail ? '' : k.ico}</div>
    <div class="job-main">
      <div class="job-tags">${tags.join('')}</div>
      <div class="job-title" title="${esc(j.title || j.url)}">${esc(j.title || j.url || 'Tarefa')}</div>
      ${j.status === 'done' || j.status === 'error' || j.status === 'canceled' ? '' : `<div class="bar ${pct == null ? 'indet' : ''}"><i style="width:${pct ?? 0}%"></i></div>`}
      <div class="job-stage"><span>${esc(j.stage)}${pct != null && running ? ` · ${pct}%` : ''}${esc(size)}</span><span class="meta">${esc(meta)}</span></div>
      ${j.error ? `<div class="job-err">${esc(j.error)}</div>` : ''}
      ${j.kind === 'transcript' && j.transcript ? `<div class="job-snippet">${esc(j.transcript.slice(0, 280))}</div>` : ''}
      ${fileList}
    </div>
    <div class="job-actions">${actions}</div>`;
}

function renderJob(j) {
  let el = document.getElementById('job-' + j.id);
  if (!el) {
    el = document.createElement('article');
    el.id = 'job-' + j.id;
    const list = $('#jobs');
    // ordem: mais novo primeiro
    const after = [...list.children].find((c) => +c.dataset.created < j.created);
    list.insertBefore(el, after || null);
  }
  el.dataset.created = j.created;
  el.className = `job k-${j.kind} s-${j.status}`;
  el.innerHTML = jobHTML(j);
}

function renderCounts() {
  const n = S.jobs.size;
  $('#count').textContent = n;
  $('#empty').hidden = n > 0;
}

function upsertJob(j) {
  const prev = S.jobs.get(j.id);
  S.jobs.set(j.id, j);
  renderJob(j);
  renderCounts();
  if (prev && prev.status !== j.status) {
    if (j.status === 'done') {
      toast(`✔ ${j.kind === 'setup' ? j.title + ' instalado' : (j.title || 'Pronto')}`, 'ok');
      notify(j);
      if (j.kind === 'setup') loadEngines();
    } else if (j.status === 'error') toast(`✘ ${j.title || 'Falhou'}`, 'err');
  }
}

function notify(j) {
  if (document.hasFocus() || !('Notification' in window) || Notification.permission !== 'granted') return;
  try { new Notification('Capivara 🦫', { body: `${KIND[j.kind]?.label || ''} pronto: ${j.title || ''}`, icon: '/icon.svg' }); } catch {}
}

$('#jobs').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-act]');
  if (!b) return;
  const id = b.closest('.job').id.slice(4);
  const act = b.dataset.act;
  try {
    if (act === 'cancel' || act === 'remove') {
      await api('/api/jobs/' + id, { method: 'DELETE' });
    } else if (act === 'retry') {
      await api(`/api/jobs/${id}/retry`, { method: 'POST' });
    } else if (act === 'reveal') {
      await api('/api/reveal/' + id, { method: 'POST' });
    } else if (act === 'view') {
      openTranscript(id);
    }
  } catch (err) { toast(err.message, 'err'); }
});

async function loadJobs() {
  const list = await api('/api/jobs');
  S.jobs.clear();
  $('#jobs').innerHTML = '';
  list.reverse().forEach((j) => { S.jobs.set(j.id, j); renderJob(j); });
  renderCounts();
}

function connectEvents() {
  const es = new EventSource('/api/events');
  es.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.type === 'job') upsertJob(m.job);
    else if (m.type === 'removed') { S.jobs.delete(m.id); document.getElementById('job-' + m.id)?.remove(); renderCounts(); }
    else if (m.type === 'resync' || m.type === 'hello') loadJobs().catch(() => {});
    else if (m.type === 'engines') loadEngines();
  };
  es.onerror = () => { es.close(); setTimeout(connectEvents, 2000); };
}

// ───────────────────────── transcrição (viewer) ─────────────────────────
async function openTranscript(id) {
  const j = await api('/api/jobs/' + id);
  S.trJob = j;
  $('#tr-title').textContent = j.title || 'Transcrição';
  $('#tr-meta').textContent = [j.language, j.segments?.length ? `${j.segments.length} trechos` : '', j.segments?.length ? fmtDur(j.segments.at(-1).end) : ''].filter(Boolean).join(' · ');
  $('#tr-files').innerHTML = (j.files || []).map((f, i) => `<a class="btn btn-sm" href="/api/files/${j.id}/${i}" download>${esc(f.name.split('.').pop().toUpperCase())}</a>`).join('');
  $('#tr-search').value = '';
  renderTranscript();
  $('#dlg-transcript').showModal();
}

function renderTranscript() {
  const j = S.trJob;
  if (!j) return;
  const q = $('#tr-search').value.trim().toLowerCase();
  const hl = (t) => {
    const e = esc(t);
    if (!q) return e;
    return e.replace(new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/&/g, '&amp;'), 'gi'), (m) => `<mark>${m}</mark>`);
  };
  const times = $('#tr-times').classList.contains('is-on');
  const body = $('#tr-body');
  if (times && j.segments?.length) {
    body.innerHTML = j.segments
      .filter((s) => !q || s.text.toLowerCase().includes(q))
      .map((s) => `<div class="seg"><time data-t="${s.start}">${fmtDur(s.start)}</time><span>${hl(s.text)}</span></div>`)
      .join('') || '<p class="muted">Nada encontrado.</p>';
  } else {
    body.innerHTML = hl(j.transcript || '');
  }
}

$('#tr-search').addEventListener('input', renderTranscript);
$('#tr-times').addEventListener('click', (e) => { e.currentTarget.classList.toggle('is-on'); renderTranscript(); });
$('#tr-copy').addEventListener('click', async () => {
  try { await navigator.clipboard.writeText(S.trJob?.transcript || ''); toast('Copiado!', 'ok', 1500); }
  catch { const r = document.createRange(); r.selectNodeContents($('#tr-body')); getSelection().removeAllRanges(); getSelection().addRange(r); document.execCommand('copy'); toast('Copiado!', 'ok', 1500); }
});
$('#tr-body').addEventListener('click', (e) => {
  const t = e.target.closest('time');
  if (!t || !S.trJob?.url) return;
  // abre o vídeo no ponto exato (YouTube aceita ?t=)
  try { const u = new URL(S.trJob.url); u.searchParams.set('t', Math.floor(+t.dataset.t) + 's'); window.open(u, '_blank'); } catch {}
});

// ───────────────────────── motores ─────────────────────────
async function loadEngines() {
  try {
    const d = await api('/api/engines');
    const by = Object.fromEntries(d.tools.map((t) => [t.name, t]));
    S.engines = { ...d, ytdlp: !!by['yt-dlp']?.path, ffmpeg: !!by.ffmpeg?.path, deno: !!by.deno?.path };
    const missing = d.tools.filter((t) => !t.path).map((t) => t.name);
    $('#engines-dot').classList.toggle('warn', missing.length > 0);
    $('#setup-banner').hidden = missing.length === 0;
    $('#setup-missing').textContent = missing.length ? `Faltam: ${missing.join(', ')} (~${missing.includes('ffmpeg') ? 250 : 60} MB, uma vez só).` : '';
    const models = modelItems();
    fillSelect($('#t-model'), models, $('#t-model').value || S.settings?.whisper_model || 'base');
    fillSelect($('#s-model'), models, S.settings?.whisper_model || 'base');
    renderEngines();
  } catch (e) { /* login pendente */ }
}

const ENGINE_INFO = {
  'yt-dlp': ['YT', 'Baixa de +1800 sites'],
  ffmpeg: ['FF', 'Junta vídeo e áudio, converte formatos'],
  deno: ['JS', 'Necessário para o YouTube funcionar'],
};

function renderEngines() {
  if (!S.engines) return;
  $('#engines-list').innerHTML = S.engines.tools.map((t) => {
    const [ico, desc] = ENGINE_INFO[t.name] || ['?', ''];
    const btn = t.path
      ? (t.name === 'yt-dlp' ? `<button class="btn btn-sm" data-install="update-yt-dlp">Atualizar</button>` : '')
      : t.installable ? `<button class="btn btn-sm btn-pink" data-install="${t.name}">Instalar</button>` : '<span class="tag">instale pelo sistema</span>';
    return `<div class="eng"><div class="eng-ico ${t.path ? 'ok' : 'no'}">${ico}</div>
      <div><strong>${esc(t.name)}</strong> ${t.version ? `<span class="tag g">${esc(t.version)}</span>` : ''}<small>${esc(t.path || desc)}</small></div>${btn}</div>`;
  }).join('');
  $('#models-list').innerHTML = S.engines.models.map((m) => `<div class="eng"><div class="eng-ico ${m.installed ? 'ok' : 'no'}">${m.name[0].toUpperCase()}</div>
      <div><strong>${esc(m.label)}</strong><small>${m.size_mb >= 1000 ? (m.size_mb / 1000).toFixed(1) + ' GB' : m.size_mb + ' MB'}</small></div>
      ${m.installed ? '<span class="tag g">instalado</span>' : `<button class="btn btn-sm btn-pink" data-install="model:${m.name}">Baixar</button>`}</div>`).join('');
}

$('#dlg-engines').addEventListener('click', async (e) => {
  const b = e.target.closest('[data-install]');
  if (!b) return;
  b.disabled = true;
  b.textContent = '…';
  try { await api('/api/engines/install', { method: 'POST', body: { what: b.dataset.install } }); toast('Instalando — acompanhe na fila', 'info'); }
  catch (err) { toast(err.message, 'err'); b.disabled = false; }
});

async function installAll() {
  const missing = S.engines.tools.filter((t) => !t.path && t.installable).map((t) => t.name);
  const model = S.settings?.whisper_model || 'base';
  if (!S.engines.models.find((m) => m.name === model)?.installed) missing.push('model:' + model);
  for (const what of missing) await api('/api/engines/install', { method: 'POST', body: { what } }).catch((e) => toast(e.message, 'err'));
  toast('Instalando motores — acompanhe na fila ⚡', 'info');
  $('#setup-banner').hidden = true;
  $('.queue').scrollIntoView({ behavior: 'smooth' });
}

function openEngines() { renderEngines(); $('#dlg-engines').showModal(); loadEngines(); }

// ───────────────────────── ajustes ─────────────────────────
async function loadSettings() {
  S.settings = await api('/api/settings');
  const f = $('#settings-form');
  for (const [k, v] of Object.entries(S.settings)) if (f.elements[k]) f.elements[k].value = v;
  fillSelect($('#s-lang'), LANGS, S.settings.language);
  fillSelect($('#t-lang'), LANGS, S.settings.language);
}

$('#settings-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const f = e.currentTarget;
  const body = { ...S.settings };
  for (const el of f.elements) if (el.name) body[el.name] = el.type === 'number' ? +el.value : el.value;
  try {
    S.settings = await api('/api/settings', { method: 'POST', body });
    fillSelect($('#t-lang'), LANGS, S.settings.language);
    $('#t-model').value = S.settings.whisper_model;
    toast('Salvo!', 'ok', 1500);
    $('#dlg-settings').close();
    onUrlChange();
  } catch (err) { toast(err.message, 'err'); }
});

// ───────────────────────── login (modo web) ─────────────────────────
function showLogin() { $('#login').hidden = false; $('#login-pass').focus(); }
$('#login-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  try {
    await api('/api/login', { method: 'POST', body: { token: $('#login-pass').value } });
    $('#login').hidden = true;
    boot();
  } catch { toast('Senha errada', 'err'); }
});

// ───────────────────────── arquivos locais ─────────────────────────
async function uploadFiles(files) {
  for (const file of files) {
    if (!/^(audio|video)\//.test(file.type) && !/\.(mp3|m4a|wav|flac|ogg|opus|aac|mp4|mkv|webm|mov|avi|wma|m4v|3gp)$/i.test(file.name)) {
      toast(`${file.name}: não parece áudio/vídeo`, 'err');
      continue;
    }
    const model = $('#t-model').value || S.settings?.whisper_model || 'base';
    const m = S.engines?.models.find((x) => x.name === model);
    if (m && !m.installed) {
      toast(`Baixando o modelo ${model} primeiro…`, 'info');
      await api('/api/engines/install', { method: 'POST', body: { what: 'model:' + model } }).catch(() => {});
    }
    const form = new FormData();
    form.append('language', $('#t-lang').value || 'auto');
    form.append('task', S.opts.task);
    form.append('model', model);
    form.append('file', file);
    toast(`Enviando ${file.name}…`, 'info', 2000);
    try { await api('/api/transcribe/upload', { method: 'POST', form }); }
    catch (e) { toast(e.message, 'err'); }
  }
}

$('#drop').addEventListener('click', (e) => { if (!e.target.closest('a')) $('#file').click(); });
$('#file').addEventListener('change', (e) => { uploadFiles([...e.target.files]); e.target.value = ''; });

let dragDepth = 0;
addEventListener('dragenter', (e) => { e.preventDefault(); dragDepth++; $('#dropping').hidden = false; });
addEventListener('dragleave', () => { if (--dragDepth <= 0) { dragDepth = 0; $('#dropping').hidden = true; } });
addEventListener('dragover', (e) => e.preventDefault());
addEventListener('drop', (e) => {
  e.preventDefault();
  dragDepth = 0;
  $('#dropping').hidden = true;
  const files = [...(e.dataTransfer?.files || [])];
  if (files.length) return uploadFiles(files);
  const text = e.dataTransfer?.getData('text/uri-list') || e.dataTransfer?.getData('text/plain');
  const u = extractUrl(text);
  if (u) { urlInput.value = u; onUrlChange(); }
});

// ───────────────────────── eventos de UI ─────────────────────────
urlInput.addEventListener('input', onUrlChange);
urlInput.addEventListener('paste', (e) => {
  const u = extractUrl(e.clipboardData?.getData('text'));
  if (u) { e.preventDefault(); urlInput.value = u; onUrlChange(); }
});
$('#url-form').addEventListener('submit', (e) => { e.preventDefault(); go(); });
$('#btn-go').addEventListener('click', go);
$('#btn-clear').addEventListener('click', () => { urlInput.value = ''; onUrlChange(); urlInput.focus(); });
$('#btn-paste').addEventListener('click', async () => {
  try {
    const t = await navigator.clipboard.readText();
    const u = extractUrl(t);
    if (!u) return toast('Não achei link na área de transferência', 'err');
    urlInput.value = u;
    onUrlChange();
  } catch { urlInput.focus(); toast('Use Ctrl+V no campo', 'info', 2000); }
});
$$('.mode').forEach((b) => b.addEventListener('click', () => setMode(b.dataset.mode)));
$('#btn-engines').addEventListener('click', openEngines);
$('#btn-setup-all').addEventListener('click', installAll);
$('#btn-settings').addEventListener('click', () => $('#dlg-settings').showModal());
$('#btn-pick').addEventListener('click', (e) => { e.stopPropagation(); $('#file').click(); });
$('#btn-clear-done').addEventListener('click', () => api('/api/jobs/clear', { method: 'POST' }).catch((e) => toast(e.message, 'err')));
$('#btn-open-folder').addEventListener('click', () => api('/api/open-output', { method: 'POST' }).catch((e) => toast(e.message, 'err')));
$('#btn-quit').addEventListener('click', async () => {
  const running = [...S.jobs.values()].some((j) => j.status === 'running');
  if (running && !confirm('Tem tarefa rodando. Fechar mesmo assim?')) return;
  await api('/api/quit', { method: 'POST' }).catch(() => {});
  document.body.innerHTML = '<div class="login"><div class="card login-card"><img src="/icon.svg" width="72"><h2>Até mais! 🦫</h2><p class="muted">Pode fechar esta janela.</p></div></div>';
  setTimeout(() => window.close(), 400);
});
$('#btn-theme').addEventListener('click', () => {
  const t = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
  document.documentElement.dataset.theme = t;
  localStorage.setItem('cap.theme', t);
});
document.addEventListener('click', (e) => {
  const c = e.target.closest('[data-close]');
  if (c) c.closest('dialog').close();
  const o = e.target.closest('[data-open="settings"]');
  if (o) $('#dlg-settings').showModal();
});
$$('dialog').forEach((d) => d.addEventListener('click', (e) => { if (e.target === d) d.close(); }));
// Ctrl+V em qualquer lugar cola o link
document.addEventListener('paste', (e) => {
  if (e.target.closest('input, textarea, select')) return;
  const u = extractUrl(e.clipboardData?.getData('text'));
  if (u) { urlInput.value = u; onUrlChange(); }
});
document.addEventListener('keydown', (e) => {
  if (e.key === '/' && !e.target.closest('input, textarea, select')) { e.preventDefault(); urlInput.focus(); }
});

// ───────────────────────── início ─────────────────────────
async function boot() {
  S.info = await api('/api/info');
  $('#ver').textContent = 'v' + S.info.version + (S.info.portable ? ' · portátil' : '');
  if (S.info.auth_required && !S.info.authed) return showLogin();
  const desktop = S.info.mode === 'desktop';
  $('#btn-quit').hidden = !desktop;
  $('#btn-open-folder').hidden = !desktop;
  await loadSettings();
  await loadEngines();
  await loadJobs();
  connectEvents();
  // link compartilhado pelo celular (PWA share target) ou ?url=
  const qs = new URLSearchParams(location.search);
  const shared = extractUrl(qs.get('url') || qs.get('text') || qs.get('title'));
  if (shared) { urlInput.value = shared; history.replaceState(null, '', '/'); }
  onUrlChange();
  if ('Notification' in window && Notification.permission === 'default') {
    document.addEventListener('click', () => Notification.requestPermission().catch(() => {}), { once: true });
  }
  if ('serviceWorker' in navigator && isSecureContext) navigator.serviceWorker.register('/sw.js').catch(() => {});
}

bindChips();
setMode(S.mode);
syncPlaylistToggles();
boot().catch((e) => toast('Não consegui falar com o Capivara: ' + e.message, 'err', 8000));
