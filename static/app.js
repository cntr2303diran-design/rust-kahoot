'use strict';
/* =====================================================================
   Kuis Rust — frontend (tanpa framework). Server: Rust (std only).
   ===================================================================== */

/* ───────────── util ───────────── */
function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v === false || v == null) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === 'class') el.className = v;
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k in el && k !== 'list') el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  }
  return el;
}
const SHAPES = ['▲', '◆', '●', '■'];
const STD_TIMES = [5, 10, 20, 30, 60, 90, 120, 240];
const DEFAULT_SETTINGS = {
  time_override: null, read_secs: 3, points_mode: 'standard', streak_bonus: true,
  shuffle_questions: false, shuffle_answers: false, show_board: true, auto_next_secs: 0,
  allow_late_join: true, show_text_on_player: true, max_players: 300,
};
const fmtPin = (p) => (p || '').slice(0, 3) + ' ' + (p || '').slice(3);
const pctOf = (a, b) => (b ? Math.round((a * 100) / b) : 0);
const lvl = (p) => (p >= 70 ? 'good' : p >= 40 ? 'warn' : 'low');

const store = {
  get(k) { try { return JSON.parse(localStorage.getItem(k)); } catch { return null; } },
  set(k, v) { try { localStorage.setItem(k, JSON.stringify(v)); } catch { /* abaikan */ } },
  del(k) { try { localStorage.removeItem(k); } catch { /* abaikan */ } },
};

function toast(msg, kind = '') {
  const t = h('div', { class: 'toast ' + kind }, msg);
  $('#toast').append(t);
  setTimeout(() => t.remove(), 3600);
}
function $(s, r = document) { return r.querySelector(s); }

async function api(path, opts) {
  const r = await fetch(path, opts);
  let data = null;
  try { data = await r.json(); } catch { /* bukan JSON */ }
  if (!r.ok) throw new Error((data && data.error) || `Terjadi kesalahan (${r.status})`);
  return data;
}

/* ───────────── state global ───────────── */
const S = {
  view: 'home', // home | loading | setup | host | player
  ws: null, role: null, pin: null, token: null, pid: null, name: '',
  st: null, closing: false, retry: 0,
  draft: null, issues: [], issueSummary: '',
  tab: 'rank', open: new Set(), sortHard: true, onlyWrong: true, sel: new Set(),
  joinErr: '', deadlineAt: 0, totalMs: 1, autoAt: 0, lastKey: '',
};

/* ───────────── koneksi WebSocket ───────────── */
function setNet(ok) { $('#net').classList.toggle('hidden', ok); }
function wsSend(m) { if (S.ws && S.ws.readyState === 1) S.ws.send(JSON.stringify(m)); }

function connect() {
  clearTimeout(S.reconT);
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const q = new URLSearchParams({ pin: S.pin, role: S.role });
  if (S.role === 'host') q.set('token', S.token);
  const ws = new WebSocket(`${proto}://${location.host}/ws?${q}`);
  S.ws = ws;
  ws.onopen = () => {
    S.retry = 0; setNet(true);
    if (S.role === 'player') wsSend({ t: 'join', name: S.name, pid: S.pid });
    clearInterval(S.pingT);
    S.pingT = setInterval(() => wsSend({ t: 'ping' }), 25000);
  };
  ws.onmessage = (e) => { let m; try { m = JSON.parse(e.data); } catch { return; } onMsg(m); };
  ws.onclose = () => {
    clearInterval(S.pingT);
    if (S.ws !== ws || S.closing) return;
    setNet(false);
    S.retry++;
    S.reconT = setTimeout(connect, Math.min(800 * S.retry, 5000));
  };
}

function leave(goHome = true) {
  S.closing = true;
  clearTimeout(S.reconT);
  if (S.ws) { try { S.ws.close(); } catch { /* */ } }
  S.ws = null; S.st = null; setNet(true);
  if (goHome) { S.view = 'home'; render(); }
}

function onMsg(m) {
  switch (m.t) {
    case 'state': {
      const prev = S.st;
      if (!prev || prev.phase !== m.phase || prev.qi !== m.qi) S.sel = new Set();
      if (!prev || prev.phase !== 'final') { if (m.phase === 'final') S.tab = 'rank'; }
      S.st = m;
      if (m.phase === 'read' || m.phase === 'question') {
        S.deadlineAt = performance.now() + m.ms_left;
        S.totalMs = m.phase === 'question' ? Math.max(m.q.time * 1000, m.ms_left) : Math.max(m.ms_left, 1);
      }
      if (m.auto_ms != null) S.autoAt = performance.now() + m.auto_ms;
      S.view = S.role === 'host' ? 'host' : 'player';
      render();
      break;
    }
    case 'joined':
      S.pid = m.pid; S.name = m.name;
      store.set('player', { pin: S.pin, name: m.name, pid: m.pid });
      break;
    case 'join_error':
      S.joinErr = m.msg; store.del('player'); leave(true);
      break;
    case 'error':
      if (m.code === 'nogame') {
        store.del('host'); store.del('player');
        S.joinErr = m.msg || 'Game tidak ditemukan';
        leave(true);
        toast(S.joinErr, 'err');
      } else toast(m.msg, 'err');
      break;
    case 'kicked':
      store.del('player'); S.joinErr = 'Kamu dikeluarkan oleh host'; leave(true);
      break;
    case 'progress': {
      const a = $('#ansN'), o = $('#onlN');
      if (a) a.textContent = m.answered;
      if (o) o.textContent = m.online;
      break;
    }
    default: break;
  }
}

/* animasi timer (tanpa render ulang) */
function tick() {
  const now = performance.now();
  const left = Math.max(0, S.deadlineAt - now);
  document.querySelectorAll('[data-timer]').forEach((el) => { el.textContent = Math.ceil(left / 1000); });
  document.querySelectorAll('[data-timerbar]').forEach((el) => { el.style.width = Math.min(100, (100 * left) / S.totalMs) + '%'; });
  const al = Math.max(0, S.autoAt - now);
  document.querySelectorAll('[data-auto]').forEach((el) => { el.textContent = Math.ceil(al / 1000) + ' dtk'; });
  requestAnimationFrame(tick);
}

/* ───────────── render ───────────── */
function render() {
  let node;
  switch (S.view) {
    case 'home': node = viewHome(); break;
    case 'setup': node = viewSetup(); break;
    case 'host': node = viewHost(); break;
    case 'player': node = viewPlayer(); break;
    default: node = viewLoading();
  }
  $('#app').replaceChildren(node);
  const key = S.view + ':' + (S.st ? S.st.phase + ':' + S.st.qi : '');
  if (key !== S.lastKey) { window.scrollTo(0, 0); S.lastKey = key; }
}

function viewLoading() {
  return h('div', { class: 'home' },
    h('div', { class: 'logo' }, 'Menyambungkan', h('span', { class: 'dots' })),
    h('button', { class: 'btn line', onclick: () => { store.del('host'); store.del('player'); leave(true); } }, 'Batal'));
}

/* ───────────── HOME ───────────── */
function viewHome() {
  const pre = new URLSearchParams(location.search).get('pin') || '';
  const pinI = h('input', { class: 'inp pin', inputmode: 'numeric', maxlength: '7', placeholder: 'PIN game', autocomplete: 'off', value: pre });
  const nameI = h('input', { class: 'inp', maxlength: '20', placeholder: 'Nama panggilan', autocomplete: 'off', value: S.name || '' });
  const err = h('div', { class: 'err' }, S.joinErr || '');
  S.joinErr = '';
  const form = h('form', {
    class: 'card join',
    onsubmit: (ev) => {
      ev.preventDefault();
      const pin = pinI.value.replace(/\D/g, '');
      const name = nameI.value.trim();
      if (pin.length !== 6) { err.textContent = 'PIN terdiri dari 6 angka'; return; }
      if (!name) { err.textContent = 'Isi dulu nama panggilanmu'; return; }
      S.closing = false; S.role = 'player'; S.pin = pin; S.name = name; S.pid = null;
      const saved = store.get('player');
      if (saved && saved.pin === pin && saved.name === name) S.pid = saved.pid;
      S.view = 'loading'; render(); connect();
    },
  },
  h('h2', {}, 'Gabung ke game'), pinI, nameI, err,
  h('button', { class: 'btn primary big', type: 'submit' }, 'Masuk'));
  const hostCard = h('div', { class: 'card hostcard' },
    h('div', {},
      h('h2', {}, 'Buat kuis sendiri'),
      h('p', {}, 'Unggah file Excel berformat Kahoot, atur waktu dan poin, lalu bagikan PIN ke pemain.')),
    h('div', { class: 'feat' }, ['Impor Excel', 'Atur waktu', 'Cek soal salah', 'Ekspor hasil'].map((t) => h('span', {}, t))),
    h('button', { class: 'btn primary big', onclick: () => { S.view = 'setup'; render(); } }, 'Buat kuis ▶'));
  return h('div', { class: 'home' },
    h('div', { class: 'logo' }, h('span', { class: 'gear' }, '⚙'), 'Kuis Rust'),
    h('div', { class: 'tag' }, 'Kuis interaktif ala Kahoot · server ditulis dengan Rust'),
    h('div', { class: 'cards' }, form, hostCard));
}

/* ───────────── SETUP (host) ───────────── */
function newQuestion() { return { text: '', a: ['', '', '', ''], c: [false, false, false, false], time: 20 }; }
function qProblems(q) {
  const p = [];
  const filled = q.a.filter((x) => x.trim()).length;
  if (!q.text.trim()) p.push('Pertanyaan masih kosong');
  if (filled < 2) p.push('Isi minimal 2 jawaban');
  if (!q.c.some((c, i) => c && q.a[i].trim())) p.push('Pilih minimal 1 jawaban yang benar');
  if (!(q.time >= 1 && q.time <= 600)) p.push('Waktu harus 1–600 detik');
  return p;
}
function fromParsed(pq) {
  const a = [...pq.answers]; while (a.length < 4) a.push('');
  const c = [false, false, false, false]; pq.correct.forEach((i) => { c[i] = true; });
  return { text: pq.text, a, c, time: pq.time };
}
function toPayload(q) {
  const idx = []; const answers = [];
  q.a.forEach((t, i) => { if (t.trim()) { idx[i] = answers.length; answers.push(t.trim()); } });
  const correct = [];
  q.c.forEach((on, i) => { if (on && idx[i] !== undefined) correct.push(idx[i]); });
  return { text: q.text.trim(), answers, correct, time: Math.round(q.time) };
}
function ensureDraft() {
  if (!S.draft) S.draft = store.get('draft');
  if (!S.draft || !Array.isArray(S.draft.questions)) S.draft = { title: '', questions: [], settings: {} };
  S.draft.settings = { ...DEFAULT_SETTINGS, ...(S.draft.settings || {}) };
}
function saveDraft() { clearTimeout(S.saveT); S.saveT = setTimeout(() => store.set('draft', S.draft), 400); }

function viewSetup() {
  ensureDraft();
  const d = S.draft; const set = d.settings;
  let cards = [];
  const editorBox = h('div', { class: 'stack' });
  const issuesBox = h('div');
  const barInfo = h('div', { class: 'info' });
  const goBtn = h('button', { class: 'btn primary big', onclick: createGame }, 'Buat game ▶');

  function renderBar() {
    const n = d.questions.length;
    const bad = d.questions.filter((q) => qProblems(q).length).length;
    const secs = d.questions.reduce((s, q) => s + (set.time_override || q.time) + set.read_secs, 0);
    barInfo.textContent = `${n} soal${bad ? ` · ${bad} perlu diperbaiki` : ''} · ± ${Math.max(1, Math.round(secs / 60))} menit`;
    goBtn.disabled = n === 0;
  }

  function renderIssues() {
    if (!S.issues.length && !S.issueSummary) { issuesBox.replaceChildren(); return; }
    issuesBox.replaceChildren(h('div', { class: 'card' },
      h('div', { class: 'sum' }, S.issueSummary),
      S.issues.length ? h('div', { class: 'issues' }, S.issues.map((i) =>
        h('div', { class: 'i ' + i.level }, `Baris ${i.row}: ${i.msg}${i.level === 'error' ? ' — baris ini dilewati' : ''}`))) : null));
  }

  function move(i, dir) {
    const j = i + dir; if (j < 0 || j >= d.questions.length) return;
    [d.questions[i], d.questions[j]] = [d.questions[j], d.questions[i]];
    saveDraft(); renderEditor();
  }

  function qCard(q, i) {
    const probs = h('div', { class: 'probs' });
    const cnt = h('div', { class: 'cnt' });
    const upd = () => {
      const p = qProblems(q);
      probs.textContent = p.join(' · ');
      card.classList.toggle('bad', p.length > 0 && (q.text || q.a.some((x) => x)));
      const n = q.text.length;
      cnt.textContent = `${n}/95 karakter (batas Kahoot)`;
      cnt.classList.toggle('over', n > 95);
      renderBar();
    };
    const ta = h('textarea', { class: 'inp', rows: 2, placeholder: 'Tulis pertanyaan di sini…', value: q.text,
      oninput: () => { q.text = ta.value; upd(); saveDraft(); } });
    const rows = [0, 1, 2, 3].map((k) => {
      const cb = h('input', { type: 'checkbox', checked: q.c[k],
        onchange: () => { q.c[k] = cb.checked; row.classList.toggle('on', cb.checked); upd(); saveDraft(); } });
      const inp = h('input', { class: 'inp', placeholder: k < 2 ? `Jawaban ${k + 1}` : `Jawaban ${k + 1} (opsional)`, value: q.a[k], maxlength: '150',
        oninput: () => { q.a[k] = inp.value; inp.style.borderColor = inp.value.length > 60 ? '#e0a100' : ''; upd(); saveDraft(); } });
      if (q.a[k].length > 60) inp.style.borderColor = '#e0a100';
      const row = h('div', { class: `arow c${k}` + (q.c[k] ? ' on' : '') },
        h('span', { class: 'shape' }, SHAPES[k]), inp,
        h('label', { class: 'ok', title: 'Tandai sebagai jawaban benar' }, cb, 'benar'));
      return row;
    });
    const time = h('input', { class: 'inp', type: 'number', min: '1', max: '600', list: 'stdtimes', value: q.time,
      oninput: () => { q.time = Number(time.value) || 0; upd(); saveDraft(); } });
    const card = h('div', { class: 'qcard' },
      h('div', { class: 'qhead' },
        h('b', {}, `Soal ${i + 1}`),
        h('div', { class: 'qtools' },
          h('button', { class: 'btn sm', title: 'Naikkan', onclick: () => move(i, -1) }, '↑'),
          h('button', { class: 'btn sm', title: 'Turunkan', onclick: () => move(i, 1) }, '↓'),
          h('button', { class: 'btn sm', title: 'Gandakan', onclick: () => { d.questions.splice(i + 1, 0, JSON.parse(JSON.stringify(q))); saveDraft(); renderEditor(); } }, '⧉'),
          h('button', { class: 'btn sm', title: 'Hapus', onclick: () => { d.questions.splice(i, 1); saveDraft(); renderEditor(); } }, '🗑'))),
      ta, cnt,
      h('div', { class: 'alist' }, rows),
      h('div', { class: 'qmeta' }, h('b', {}, 'Batas waktu'), time, 'detik',
        h('span', { class: 'muted' }, 'Centang beberapa jawaban jika ada lebih dari satu yang benar.')),
      probs);
    upd();
    return card;
  }

  function renderEditor() {
    cards = d.questions.map(qCard);
    editorBox.replaceChildren(
      ...cards,
      h('button', { class: 'btn line', onclick: () => { d.questions.push(newQuestion()); saveDraft(); renderEditor(); setTimeout(() => cards[cards.length - 1].scrollIntoView({ behavior: 'smooth', block: 'center' }), 30); } }, '＋ Tambah soal'));
    renderBar();
  }

  async function handleFile(file) {
    if (!file) return;
    if (file.size > 10 * 1024 * 1024) { toast('File terlalu besar (maks 10 MB)', 'err'); return; }
    if (d.questions.some((q) => q.text.trim()) && !confirm('Soal yang sekarang akan diganti dengan isi file ini. Lanjutkan?')) return;
    try {
      const buf = await file.arrayBuffer();
      const data = await api('/api/parse?name=' + encodeURIComponent(file.name), { method: 'POST', body: buf });
      d.questions = data.questions.map(fromParsed);
      d.title = data.title || d.title;
      titleI.value = d.title;
      const errs = data.issues.filter((i) => i.level === 'error').length;
      const warns = data.issues.filter((i) => i.level === 'warning').length;
      S.issues = data.issues;
      S.issueSummary = `${data.questions.length} soal berhasil dibaca dari sheet “${data.sheet}”` +
        (errs ? ` · ${errs} baris bermasalah dilewati` : '') + (warns ? ` · ${warns} peringatan` : '');
      saveDraft(); renderIssues(); renderEditor();
      toast(`${data.questions.length} soal dimuat`);
    } catch (e) { toast(e.message, 'err'); }
  }

  const fileI = h('input', { type: 'file', accept: '.xlsx,.csv', class: 'hidden', onchange: () => { handleFile(fileI.files[0]); fileI.value = ''; } });
  const drop = h('div', { class: 'drop',
    onclick: () => fileI.click(),
    ondragover: (e) => { e.preventDefault(); drop.classList.add('over'); },
    ondragleave: () => drop.classList.remove('over'),
    ondrop: (e) => { e.preventDefault(); drop.classList.remove('over'); handleFile(e.dataTransfer.files[0]); } },
  h('div', { class: 'big' }, '📄'),
  h('b', {}, 'Klik atau seret file Excel ke sini'),
  h('span', { class: 'muted' }, 'Format .xlsx sesuai template Kahoot (juga mendukung .csv)'));

  const titleI = h('input', { class: 'inp', placeholder: 'Judul kuis', maxlength: '100', value: d.title,
    oninput: () => { d.title = titleI.value; saveDraft(); } });

  /* pengaturan */
  const field = (label, control, hint) => h('div', { class: 'fld' }, h('label', { class: 't' }, label), control, hint ? h('div', { class: 'hint' }, hint) : null);
  const sel = (key, opts, num) => {
    const s = h('select', { onchange: () => { set[key] = num ? Number(s.value) : s.value; saveDraft(); renderBar(); } },
      opts.map(([v, l]) => h('option', { value: v, selected: String(set[key]) === String(v) }, l)));
    return s;
  };
  const toggle = (key, label, sub) => {
    const cb = h('input', { type: 'checkbox', checked: !!set[key], onchange: () => { set[key] = cb.checked; saveDraft(); } });
    return h('label', { class: 'tg' }, cb, h('span', {}, label, sub ? h('small', {}, sub) : null));
  };
  const useAll = h('input', { type: 'checkbox', checked: set.time_override != null });
  const allSecs = h('input', { type: 'number', min: '1', max: '600', value: set.time_override || 20, disabled: set.time_override == null, style: { width: '100px' } });
  useAll.onchange = () => {
    allSecs.disabled = !useAll.checked;
    set.time_override = useAll.checked ? (Number(allSecs.value) || 20) : null;
    saveDraft(); renderBar();
  };
  allSecs.oninput = () => { if (useAll.checked) { set.time_override = Math.min(600, Math.max(1, Number(allSecs.value) || 20)); saveDraft(); renderBar(); } };
  const settingsCard = h('div', { class: 'card' },
    h('h2', {}, '⚙ Pengaturan permainan'),
    h('div', { class: 'set' },
      field('Batas waktu soal',
        h('div', { class: 'row' }, h('label', { class: 'tg' }, useAll, h('span', {}, 'Samakan semua soal')), allSecs, 'detik'),
        'Kosongkan centang untuk memakai waktu dari tiap baris Excel.'),
      field('Waktu membaca soal', sel('read_secs', [[0, 'Langsung jawab'], [2, '2 detik'], [3, '3 detik'], [5, '5 detik'], [8, '8 detik'], [10, '10 detik']], true),
        'Soal tampil dulu sebelum pilihan jawaban muncul.'),
      field('Sistem poin', sel('points_mode', [['standard', 'Standar – makin cepat makin besar (maks 1000)'], ['double', 'Poin ganda (maks 2000)'], ['flat', 'Tetap 1000 – tanpa bonus kecepatan']])),
      field('Lanjut otomatis', sel('auto_next_secs', [[0, 'Manual – host klik Lanjut'], [3, 'Setelah 3 detik'], [5, 'Setelah 5 detik'], [8, 'Setelah 8 detik'], [10, 'Setelah 10 detik'], [15, 'Setelah 15 detik'], [30, 'Setelah 30 detik']], true),
        'Berlaku untuk layar hasil soal dan papan skor.'),
      toggle('streak_bonus', 'Bonus beruntun (streak)', 'Tambahan 100–500 poin untuk jawaban benar berturut-turut'),
      toggle('show_board', 'Tampilkan papan skor tiap soal'),
      toggle('shuffle_questions', 'Acak urutan soal'),
      toggle('shuffle_answers', 'Acak posisi jawaban'),
      toggle('allow_late_join', 'Boleh bergabung setelah game dimulai'),
      toggle('show_text_on_player', 'Tampilkan teks soal di HP pemain', 'Matikan jika soal hanya ditampilkan di layar proyektor'),
      field('Maksimal pemain', (() => { const i = h('input', { type: 'number', min: '1', max: '1000', value: set.max_players, oninput: () => { set.max_players = Math.min(1000, Math.max(1, Number(i.value) || 300)); saveDraft(); } }); return i; })())));

  async function createGame() {
    if (!d.questions.length) { toast('Tambahkan minimal 1 soal dulu', 'err'); return; }
    const bad = d.questions.findIndex((q) => qProblems(q).length);
    if (bad >= 0) {
      toast(`Soal ${bad + 1} belum lengkap`, 'err');
      cards[bad].scrollIntoView({ behavior: 'smooth', block: 'center' });
      return;
    }
    goBtn.disabled = true;
    try {
      const res = await api('/api/games', {
        method: 'POST', headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ title: d.title.trim() || 'Kuis Tanpa Judul', questions: d.questions.map(toPayload), settings: set }),
      });
      store.set('host', { pin: res.pin, token: res.token });
      store.set('draft', S.draft);
      S.role = 'host'; S.pin = res.pin; S.token = res.token; S.closing = false;
      S.view = 'loading'; render(); connect();
    } catch (e) { toast(e.message, 'err'); goBtn.disabled = false; }
  }

  const root = h('div', { class: 'page' },
    h('datalist', { id: 'stdtimes' }, STD_TIMES.map((t) => h('option', { value: t }))),
    h('div', { class: 'topline' },
      h('h1', {}, 'Buat kuis'),
      h('button', { class: 'btn line', onclick: () => { S.view = 'home'; render(); } }, '← Kembali')),
    h('div', { class: 'stack' },
      h('div', { class: 'card' },
        h('h2', {}, '1 · Impor soal dari Excel'),
        drop, fileI,
        h('div', { class: 'row', style: { marginTop: '12px' } },
          h('a', { class: 'btn', href: '/api/template.xlsx', download: 'template-kuis.xlsx', style: { textDecoration: 'none' } }, '⬇ Unduh template Excel'),
          h('span', { class: 'muted' }, 'Template memakai format Kahoot: pertanyaan, 4 jawaban, batas waktu, dan kunci jawaban (contoh 2 atau 1,3).'))),
      issuesBox,
      h('div', { class: 'card' }, h('h2', {}, '2 · Judul & soal'), titleI),
      editorBox,
      h('div', {}, settingsCard)),
    h('div', { class: 'sticky' }, barInfo, goBtn));
  renderIssues();
  renderEditor();
  return root;
}

/* ───────────── HOST: game ───────────── */
function hostBar(st) {
  return h('div', { class: 'hbar' },
    h('div', { class: 'pinmini' }, 'PIN ' + fmtPin(st.pin)),
    h('div', { class: 'ttl' }, st.title),
    st.phase !== 'lobby' && st.phase !== 'final' ? h('div', { class: 'cnt2' }, `Soal ${st.qi + 1}/${st.total}`) : null,
    st.phase === 'final' ? null : h('button', { class: 'btn sm', onclick: () => {
      if (st.phase === 'lobby') { if (confirm('Batalkan game ini? Semua pemain akan terputus.')) endHostSession(); }
      else if (confirm('Akhiri permainan sekarang dan lihat hasil?')) wsSend({ t: 'end' });
    } }, st.phase === 'lobby' ? 'Batal' : 'Akhiri'));
}
function endHostSession() { store.del('host'); leave(true); }

function viewHost() {
  const st = S.st;
  let body;
  switch (st.phase) {
    case 'lobby': body = hostLobby(st); break;
    case 'read': body = hostRead(st); break;
    case 'question': body = hostQuestion(st); break;
    case 'reveal': body = hostReveal(st); break;
    case 'board': body = hostBoard(st); break;
    default: body = hostFinal(st);
  }
  return h('div', {}, hostBar(st), body);
}

function hostLobby(st) {
  const url = `${location.origin}/?pin=${st.pin}`;
  const local = ['localhost', '127.0.0.1', '[::1]'].includes(location.hostname);
  const s = st.settings || {};
  const pts = { standard: 'poin standar', double: 'poin ganda', flat: 'poin tetap' }[s.points_mode];
  return h('div', { class: 'screen' }, h('div', { class: 'center' },
    h('div', { class: 'joinbox' },
      h('div', { class: 'u' }, 'Buka ', h('b', {}, location.host), ' lalu masukkan PIN'),
      h('div', { class: 'pincode' }, fmtPin(st.pin)),
      h('button', { class: 'btn sm', style: { marginTop: '6px', background: 'var(--bg2)', color: 'var(--ink)' },
        onclick: () => { (navigator.clipboard ? navigator.clipboard.writeText(url) : Promise.reject()).then(() => toast('Tautan disalin'), () => toast(url)); } }, '🔗 Salin tautan gabung')),
    local ? h('div', { class: 'hint2' }, 'Pemain di perangkat lain tidak bisa membuka “localhost”. Minta mereka membuka alamat IP komputer ini (tertera di jendela terminal tempat server dijalankan).') : null,
    h('div', { class: 'count' }, `👥 ${st.players.length} pemain`),
    h('div', { class: 'chips' }, st.players.length
      ? st.players.map((p) => h('button', { class: 'chip' + (p.online ? '' : ' off'), title: 'Klik untuk mengeluarkan',
        onclick: () => { if (confirm(`Keluarkan ${p.name}?`)) wsSend({ t: 'kick', id: p.id }); } }, p.name))
      : h('span', { class: 'kicker' }, 'Menunggu pemain bergabung…')),
    h('div', { class: 'ctrl' },
      h('button', { class: 'btn', onclick: () => wsSend({ t: 'lock', locked: !st.locked }) }, st.locked ? '🔓 Buka pintu' : '🔒 Kunci pintu'),
      h('button', { class: 'btn primary big', disabled: st.players.length === 0, onclick: () => wsSend({ t: 'start' }) }, 'Mulai ▶')),
    h('div', { class: 'kicker', style: { opacity: '.7' } },
      `${st.total} soal · ${s.time_override ? 'waktu ' + s.time_override + ' dtk' : 'waktu sesuai Excel'} · ${pts}${s.shuffle_questions ? ' · soal diacak' : ''}`)));
}

function hostRead(st) {
  return h('div', { class: 'screen' }, h('div', { class: 'center' },
    h('div', { class: 'kicker' }, `Soal ${st.q.index + 1} dari ${st.q.total}`),
    h('h1', { class: 'qtext' }, st.q.text),
    h('div', { class: 'bigtimer', 'data-timer': '' }, Math.ceil(st.ms_left / 1000)),
    h('div', { class: 'bar' }, h('i', { 'data-timerbar': '' })),
    h('div', { class: 'ctrl' }, h('button', { class: 'btn', onclick: () => wsSend({ t: 'skip' }) }, 'Lewati ⏭'))));
}

function hostQuestion(st) {
  const q = st.q;
  return h('div', { class: 'screen' },
    h('div', { class: 'qbox' }, q.text),
    h('div', { class: 'mid' },
      h('div', { class: 'dial', 'data-timer': '' }, Math.ceil(st.ms_left / 1000)),
      h('div', { class: 'bar', style: { flex: '1' } }, h('i', { 'data-timerbar': '' })),
      h('div', { class: 'answered' }, h('b', { id: 'ansN' }, st.answered), h('span', {}, 'dari '), h('span', { id: 'onlN' }, st.online), ' menjawab')),
    q.multi ? h('div', { class: 'kicker', style: { textAlign: 'center', margin: '4px 0 8px' } }, 'Pilih semua jawaban yang benar') : null,
    h('div', { class: 'grid host' }, q.answers.map((a, i) => h('div', { class: `tile c${i}` }, h('span', { class: 'shape' }, SHAPES[i]), h('span', { class: 'atext' }, a)))),
    h('div', { class: 'ctrl', style: { marginTop: '16px' } },
      h('button', { class: 'btn', onclick: () => wsSend({ t: 'add_time', secs: 10 }) }, '⏱ +10 detik'),
      h('button', { class: 'btn', onclick: () => wsSend({ t: 'skip' }) }, 'Selesaikan sekarang ⏭')));
}

function hostReveal(st) {
  const q = st.q; const p = pctOf(st.correct_n, st.players_total);
  const last = st.qi + 1 >= st.total;
  return h('div', { class: 'screen' },
    h('div', { class: 'qbox' }, q.text),
    h('div', { class: 'stat', style: { margin: '14px auto' } },
      `${st.correct_n} dari ${st.players_total} pemain menjawab benar (${p}%)`,
      h('div', { class: 'pbar' }, h('i', { style: { width: p + '%' } }))),
    h('div', { class: 'grid host' }, q.answers.map((a, i) => {
      const right = q.correct.includes(i);
      return h('div', { class: `tile c${i}` + (right ? '' : ' dim') },
        h('span', { class: 'shape' }, SHAPES[i]), h('span', { class: 'atext' }, a),
        h('span', { class: 'num' }, st.counts[i]), right ? h('span', { class: 'ck' }, '✓') : null);
    })),
    h('div', { class: 'ctrl', style: { marginTop: '18px', alignItems: 'center' } },
      st.auto_ms != null ? h('span', { class: 'kicker' }, 'Lanjut otomatis dalam ', h('b', { 'data-auto': '' }, '')) : null,
      h('button', { class: 'btn primary big', onclick: () => wsSend({ t: 'next' }) }, last ? 'Lihat hasil akhir ▶' : 'Lanjut ▶')));
}

function boardRows(top, me) {
  const max = Math.max(1, ...top.map((t) => t.score));
  return h('div', { class: 'board' }, top.map((t) => h('div', { class: 'brow' + (me && t.name === me ? ' me' : '') },
    h('div', { class: 'fill', style: { width: Math.max(4, (100 * t.score) / max) + '%' } }),
    h('span', { class: 'rk' }, t.rank), h('span', { class: 'nm' }, t.name),
    t.gain ? h('span', { class: 'gain' }, '+' + t.gain) : null, h('b', {}, t.score))));
}

function hostBoard(st) {
  return h('div', { class: 'screen' }, h('div', { class: 'center' },
    h('h1', {}, 'Papan skor'),
    boardRows(st.top),
    st.auto_ms != null ? h('span', { class: 'kicker' }, 'Lanjut otomatis dalam ', h('b', { 'data-auto': '' }, '')) : null,
    h('button', { class: 'btn primary big', onclick: () => wsSend({ t: 'next' }) }, 'Soal berikutnya ▶')));
}

/* komponen hasil: dipakai host & pemain */
function ansChips(answers, idxs) {
  if (!idxs || !idxs.length) return h('span', { class: 'muted' }, '—');
  return idxs.map((i) => h('span', { class: `achip c${i}` }, h('span', { class: 'shp' }, SHAPES[i]), answers[i] ?? ''));
}
function reviewItem(it, open, toggle) {
  const status = it.ok == null ? 'sk' : it.ok ? 'good' : 'low';
  const label = it.ok == null ? 'Tidak dijawab' : it.ok ? 'Benar' : 'Salah';
  return h('div', { class: 'item' },
    h('div', { class: 'hd', onclick: toggle },
      h('span', { class: 'no_' }, it.index + 1), h('span', { class: 'tx' }, it.text),
      h('span', { class: 'pill ' + status }, label + (it.points ? ` +${it.points}` : ''))),
    open ? h('div', { class: 'bd' },
      h('div', { class: 'lbl' }, 'Jawabanmu'), h('div', { class: 'mine' }, ansChips(it.answers, it.mine)),
      h('div', { class: 'lbl' }, 'Jawaban benar'), h('div', { class: 'mine' }, ansChips(it.answers, it.correct)),
      it.ms != null ? h('div', { class: 'lbl' }, `Waktu menjawab: ${(it.ms / 1000).toFixed(1)} detik`) : null) : null);
}

function podium(top) {
  const order = [top[1], top[0], top[2]].filter(Boolean);
  const med = ['🥇', '🥈', '🥉'];
  return h('div', { class: 'podium' }, order.map((p) => h('div', { class: 'pod p' + p.rank },
    h('div', { class: 'medal' }, med[p.rank - 1]), h('div', { class: 'nm' }, p.name), h('div', { class: 'sc' }, p.score + ' poin'),
    h('div', { class: 'blk' }, p.rank))));
}

function hostFinal(st) {
  const r = st.report;
  const tabBtn = (id, label) => h('button', { class: 'tab' + (S.tab === id ? ' on' : ''), onclick: () => { S.tab = id; render(); } }, label);
  let content;
  if (S.tab === 'soal') content = tabSoal(r);
  else if (S.tab === 'pemain') content = tabPemain(r);
  else content = tabRank(r);
  const url = `/api/games/${st.pin}/export.xlsx?token=${encodeURIComponent(S.token)}`;
  return h('div', { class: 'screen' },
    h('div', { class: 'center', style: { flex: 'none', gap: '6px', paddingTop: '8px' } },
      h('h1', {}, 'Hasil akhir 🎉'),
      r.players.length ? podium(r.players.slice(0, 3)) : h('div', { class: 'kicker' }, 'Tidak ada pemain')),
    h('div', { class: 'ctrl', style: { margin: '6px 0 16px' } },
      h('a', { class: 'btn primary', href: url, style: { textDecoration: 'none' } }, '⬇ Unduh hasil (Excel)'),
      h('button', { class: 'btn', onclick: () => { if (confirm('Mulai lagi dengan pemain yang masih terhubung? Skor direset.')) wsSend({ t: 'reset' }); } }, '🔁 Main lagi'),
      h('button', { class: 'btn line', onclick: endHostSession }, 'Selesai')),
    h('div', { class: 'panel' }, h('div', { class: 'tabs' }, tabBtn('rank', '🏆 Peringkat'), tabBtn('soal', '📊 Analisis soal'), tabBtn('pemain', '👤 Per pemain')), content));
}

function tabRank(r) {
  if (!r.players.length) return h('div', { class: 'empty' }, 'Belum ada data.');
  return h('table', { class: 't' },
    h('thead', {}, h('tr', {}, h('th', {}, '#'), h('th', {}, 'Nama'), h('th', { class: 'n' }, 'Skor'), h('th', { class: 'n' }, '✓ Benar'), h('th', { class: 'n' }, '✗ Salah'), h('th', { class: 'n' }, '— Kosong'))),
    h('tbody', {}, r.players.map((p) => h('tr', {}, h('td', {}, p.rank), h('td', {}, p.name), h('td', { class: 'n' }, p.score),
      h('td', { class: 'n t-ok' }, p.correct_n), h('td', { class: 'n t-no' }, p.wrong_n), h('td', { class: 'n t-sk' }, p.skipped_n)))));
}

function tabSoal(r) {
  const tot = r.total_players;
  const rows = r.questions.map((q) => ({ q, p: pctOf(q.correct_n, tot) }));
  const hardest = [...rows].sort((a, b) => a.p - b.p)[0];
  if (S.sortHard) rows.sort((a, b) => a.p - b.p || a.q.index - b.q.index);
  const list = rows.map(({ q, p }) => {
    const key = 'q' + q.index; const open = S.open.has(key);
    const maxC = Math.max(1, ...q.counts);
    const wrongTop = q.counts.map((c, i) => [c, i]).filter(([, i]) => !q.correct.includes(i)).sort((a, b) => b[0] - a[0])[0];
    return h('div', { class: 'item' },
      h('div', { class: 'hd', onclick: () => { open ? S.open.delete(key) : S.open.add(key); render(); } },
        h('span', { class: 'no_' }, q.index + 1), h('span', { class: 'tx' }, q.text),
        h('span', { class: 'pill ' + lvl(p) }, `${p}% benar`)),
      open ? h('div', { class: 'bd' },
        q.answers.map((a, i) => h('div', { class: 'drow' },
          h('span', { class: `achip c${i}` }, h('span', { class: 'shp' }, SHAPES[i])),
          h('span', { class: 'lab' }, a, q.correct.includes(i) ? ' ✓' : (wrongTop && wrongTop[1] === i && wrongTop[0] > 0 ? ' (paling banyak salah pilih)' : '')),
          h('span', { class: 'bar2' }, h('i', { style: { width: (100 * q.counts[i]) / maxC + '%', background: q.correct.includes(i) ? 'var(--good)' : 'var(--low)' } })),
          h('b', {}, q.counts[i]))),
        h('div', { class: 'lbl' }, `${q.correct_n} benar · ${tot - q.correct_n} salah/tidak menjawab (${tot - q.answered} kosong) · rata-rata ${(q.avg_ms / 1000).toFixed(1)} dtk dari batas ${q.time} dtk`)) : null);
  });
  return h('div', {},
    hardest && tot ? h('div', { class: 'insight' }, `Soal tersulit: no. ${hardest.q.index + 1} — hanya ${hardest.p}% pemain menjawab benar.`) : null,
    h('div', { class: 'tools' }, h('label', {}, h('input', { type: 'checkbox', checked: S.sortHard, onchange: (e) => { S.sortHard = e.target.checked; render(); } }), 'Urutkan dari yang tersulit')),
    list.length ? list : h('div', { class: 'empty' }, 'Belum ada data.'));
}

function tabPemain(r) {
  const list = r.players.map((p) => {
    const key = 'p' + p.name; const open = S.open.has(key);
    let body = null;
    if (open) {
      const items = r.questions.map((q) => {
        const a = p.answers[q.index];
        return { index: q.index, text: q.text, answers: q.answers, correct: q.correct,
          mine: a ? a.choices : null, ok: a ? a.ok : null, points: a ? a.points : 0, ms: a ? a.ms : null };
      }).filter((it) => !S.onlyWrong || it.ok !== true);
      body = h('div', { class: 'bd' }, items.length
        ? items.map((it) => reviewItem(it, true, () => {}))
        : h('div', { class: 'empty' }, 'Semua jawaban benar 🎉'));
    }
    return h('div', { class: 'item' },
      h('div', { class: 'hd', onclick: () => { open ? S.open.delete(key) : S.open.add(key); render(); } },
        h('span', { class: 'no_' }, '#' + p.rank), h('span', { class: 'tx' }, p.name, h('span', { class: 'muted' }, `  ·  ${p.score} poin`)),
        h('span', { class: 't-ok' }, '✓' + p.correct_n), h('span', { class: 't-no' }, '✗' + p.wrong_n), h('span', { class: 't-sk' }, '—' + p.skipped_n)),
      body);
  });
  return h('div', {},
    h('div', { class: 'tools' }, h('label', {}, h('input', { type: 'checkbox', checked: S.onlyWrong, onchange: (e) => { S.onlyWrong = e.target.checked; render(); } }), 'Tampilkan hanya soal yang salah / tidak dijawab'),
      h('span', { class: 'muted' }, 'Klik nama untuk membuka detail')),
    list.length ? list : h('div', { class: 'empty' }, 'Belum ada data.'));
}

/* ───────────── PEMAIN ───────────── */
function viewPlayer() {
  const st = S.st;
  let body;
  switch (st.phase) {
    case 'lobby': body = pLobby(st); break;
    case 'read': body = pRead(st); break;
    case 'question': body = pQuestion(st); break;
    case 'reveal': body = pReveal(st); break;
    case 'board': body = pBoard(st); break;
    default: body = pFinal(st);
  }
  const top = h('div', { class: 'ptop' }, h('span', { class: 'nm' }, st.name), h('span', { class: 'sc' }, st.score + ' poin'));
  return h('div', { class: 'pscreen' }, top, body);
}

function pLobby(st) {
  return h('div', { class: 'wait' },
    h('div', { class: 'bignum' }, '🎉'), h('h1', {}, 'Kamu sudah masuk!'),
    h('div', { class: 'kicker' }, 'Lihat namamu di layar host'),
    h('div', { class: 'pill2' }, `${st.players_total} pemain di ruangan`),
    h('button', { class: 'btn line sm', onclick: () => { store.del('player'); leave(true); } }, 'Keluar'));
}

function pRead(st) {
  return h('div', { class: 'wait' },
    h('div', { class: 'kicker' }, `Soal ${st.q.index + 1} dari ${st.q.total}`),
    h('div', { class: 'bigtimer', 'data-timer': '' }, Math.ceil(st.ms_left / 1000)),
    st.q.text ? h('div', { class: 'pq', style: { width: '100%' } }, st.q.text) : h('h2', {}, 'Bersiap…'),
    st.q.multi ? h('div', { class: 'pill2' }, 'Ada lebih dari satu jawaban benar') : null);
}

function pQuestion(st) {
  const q = st.q;
  if (st.answered) {
    const first = (st.mine && st.mine[0]) ?? 0;
    return h('div', { class: 'wait' },
      h('div', { class: `bigshape c${first}` }, SHAPES[first]),
      h('h2', {}, 'Jawaban terkirim ✓'),
      h('div', { class: 'kicker dots' }, 'Menunggu pemain lain'));
  }
  const shapeOnly = !q.answers.some((a) => a);
  const submit = () => { if (S.sel.size) wsSend({ t: 'answer', choices: [...S.sel] }); };
  const tiles = q.answers.map((a, i) => h('button', { class: `tile c${i}` + (shapeOnly ? ' shapeonly' : '') + (S.sel.has(i) ? ' sel' : ''),
    onclick: (e) => {
      if (q.multi) { S.sel.has(i) ? S.sel.delete(i) : S.sel.add(i); e.currentTarget.classList.toggle('sel', S.sel.has(i)); sendBtn.disabled = !S.sel.size; }
      else wsSend({ t: 'answer', choices: [i] });
    } }, h('span', { class: 'shape' }, SHAPES[i]), shapeOnly ? null : h('span', { class: 'atext' }, a)));
  const sendBtn = h('button', { class: 'btn primary big', disabled: !S.sel.size, style: { marginTop: '12px' }, onclick: submit }, 'Kirim jawaban');
  return h('div', { style: { display: 'flex', flexDirection: 'column', flex: '1' } },
    h('div', { class: 'tbar' }, h('i', { 'data-timerbar': '' })),
    h('div', { class: 'row sb', style: { marginBottom: '8px' } }, h('span', { class: 'kicker' }, `Soal ${q.index + 1}/${q.total}`), h('b', { class: 'bigtimer', style: { fontSize: '1.8rem' }, 'data-timer': '' }, '')),
    q.text ? h('div', { class: 'pq' }, q.text) : null,
    q.multi ? h('div', { class: 'pill2', style: { alignSelf: 'center', marginBottom: '8px' } }, 'Pilih semua yang benar, lalu kirim') : null,
    h('div', { class: 'pgrid' + (q.answers.length === 2 && !shapeOnly ? ' one' : '') }, tiles),
    q.multi ? sendBtn : null);
}

function pReveal(st) {
  const r = st.result; const q = st.q;
  const kind = !r.answered ? 'none' : r.ok ? 'ok' : 'bad';
  const title = !r.answered ? 'Waktu habis' : r.ok ? 'Benar!' : 'Salah';
  const ico = !r.answered ? '⏰' : r.ok ? '✔' : '✖';
  return h('div', { class: 'res ' + kind },
    h('div', { class: 'ico' }, ico), h('h2', {}, title),
    r.ok ? h('div', { class: 'pts' }, '+' + r.points) : null,
    r.ok && r.bonus ? h('div', { class: 'sub' }, `Termasuk bonus beruntun +${r.bonus} 🔥`) : null,
    r.ok && st.streak > 1 ? h('div', { class: 'sub' }, `Beruntun ${st.streak} soal berturut-turut`) : null,
    !r.ok ? h('div', { class: 'sub' }, 'Jawaban benar:') : null,
    !r.ok ? h('div', {}, ansChips(q.answers, q.correct)) : null,
    h('div', { class: 'chipsrow' },
      h('span', { class: 'pill2' }, `Peringkat ${st.rank} dari ${st.players_total}`),
      r.behind ? h('span', { class: 'pill2' }, `${r.behind.diff} poin di belakang ${r.behind.name}`) : (st.rank === 1 ? h('span', { class: 'pill2' }, '👑 Kamu memimpin!') : null)));
}

function pBoard(st) {
  return h('div', { class: 'wait', style: { justifyContent: 'flex-start', paddingTop: '10px' } },
    h('h1', {}, 'Papan skor'),
    boardRows(st.top, st.name),
    h('div', { class: 'chipsrow' }, h('span', { class: 'pill2' }, `Kamu peringkat ${st.rank} · ${st.score} poin`),
      st.behind ? h('span', { class: 'pill2' }, `${st.behind.diff} poin lagi untuk menyusul ${st.behind.name}`) : null),
    h('div', { class: 'kicker dots' }, 'Menunggu soal berikutnya'));
}

function pFinal(st) {
  const items = st.review.filter((it) => !S.onlyWrong || it.ok !== true);
  const med = ['🥇', '🥈', '🥉'][st.rank - 1];
  return h('div', { style: { display: 'flex', flexDirection: 'column', gap: '12px' } },
    h('div', { class: 'wait', style: { padding: '12px 0' } },
      h('div', { class: 'medal', style: { fontSize: '3rem' } }, med || '🏁'),
      h('div', { class: 'kicker' }, 'Permainan selesai'),
      h('div', { class: 'bignum' }, '#' + st.rank),
      h('div', { class: 'pill2' }, `${st.score} poin · dari ${st.players_total} pemain`),
      h('div', { class: 'chipsrow' },
        h('span', { class: 'pill2' }, `✓ ${st.correct_n} benar`), h('span', { class: 'pill2' }, `✗ ${st.wrong_n} salah`), h('span', { class: 'pill2' }, `— ${st.skipped_n} kosong`))),
    h('div', { class: 'panel review' },
      h('h3', {}, 'Tinjau jawabanmu'),
      h('div', { class: 'tools' }, h('label', {}, h('input', { type: 'checkbox', checked: S.onlyWrong, onchange: (e) => { S.onlyWrong = e.target.checked; render(); } }), 'Hanya soal yang salah / tidak dijawab')),
      items.length ? items.map((it) => reviewItem(it, S.open.has('r' + it.index), () => { const k = 'r' + it.index; S.open.has(k) ? S.open.delete(k) : S.open.add(k); render(); }))
        : h('div', { class: 'empty' }, 'Semua jawabanmu benar! 🎉')),
    h('button', { class: 'btn line', onclick: () => { store.del('player'); leave(true); } }, 'Keluar'));
}

/* ───────────── mulai ───────────── */
function boot() {
  const hs = store.get('host'); const ps = store.get('player');
  if (hs && hs.pin && hs.token) {
    S.role = 'host'; S.pin = hs.pin; S.token = hs.token; S.view = 'loading'; connect();
  } else if (ps && ps.pin && ps.pid) {
    S.role = 'player'; S.pin = ps.pin; S.name = ps.name; S.pid = ps.pid; S.view = 'loading'; connect();
  }
  render();
  requestAnimationFrame(tick);
}
boot();
