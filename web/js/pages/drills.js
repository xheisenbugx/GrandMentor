// Quick drills (#/drills hub, #/drills/:drillId game). Contract: docs/CONTRACT.md "Quick drills".
//
// Five short, timed board-vision games with personal bests:
//   coordinates — find / name squares (client-generated)
//   hanging     — tap every hanging piece        (GET /api/drills/hanging/batch)
//   material    — who is ahead and by how much   (GET /api/drills/material/batch)
//   checks      — find every check (or capture)  (GET /api/drills/checks/batch)
//   knight      — shortest knight route          (GET /api/drills/knight/batch)
// Finished runs are saved with POST /api/drills/:id/score (server keeps bests + bounded history).
//
// Memory: every listener, timer, animation frame, fetch and the Board are released by the cleanup.

import { h, icon, disposables, pageHeader, emptyState, skeleton } from '../ui.js';
import { api, qs, isAbort } from '../api.js';
import { t, formatNumber } from '../i18n.js';
import { Board } from '../components/board.js';
import { reducedMotion as settingsReducedMotion } from '../settings.js';
import { playSound } from '../components/sound.js';

export const title = (params) => {
  const id = params && params.drillId;
  return id && DRILL_IDS.includes(id) ? t(`drills.${id}.title`) : t('nav.routes.drills');
};

const DRILL_IDS = ['coordinates', 'hanging', 'material', 'checks', 'knight'];
const META = {
  coordinates: { emoji: '🎯', tone: 'green', seconds: 30, server: false },
  hanging: { emoji: '🎁', tone: 'gold', seconds: 60, server: true },
  material: { emoji: '⚖️', tone: 'blue', seconds: 60, server: true },
  checks: { emoji: '⚡', tone: 'red', seconds: 60, server: true },
  knight: { emoji: '🐴', tone: 'teal', seconds: 60, server: true },
};
const PREFS_KEY = 'gm.drills.v1';
const BATCH_SIZE = 20;
const REFILL_AT = 6;
const MAX_QUEUE = 60;
const MAX_MISTAKES = 40;
const FILES = 'abcdefgh';
const EMPTY_FEN = '8/8/8/8/8/8/8/8 w - - 0 1';
const COUNTDOWN = 3;

// ----------------------------------------------------------------------------- small helpers

function reducedMotion() {
  try { return settingsReducedMotion(); } catch { return false; }
}

function readPrefs() {
  try {
    const v = JSON.parse(localStorage.getItem(PREFS_KEY) || '{}');
    return v && typeof v === 'object' ? v : {};
  } catch { return {}; }
}

function writePrefs(all) {
  try { localStorage.setItem(PREFS_KEY, JSON.stringify(all)); } catch { /* private mode */ }
}

function defaultPrefs(id) {
  switch (id) {
    case 'coordinates': return { mode: 'find', color: 'white', coords: true };
    case 'checks': return { variant: 'checks' };
    case 'knight': return { variant: 'basic' };
    default: return {};
  }
}

function prefsFor(id) {
  const saved = readPrefs()[id];
  const d = defaultPrefs(id);
  const p = { ...d, ...(saved && typeof saved === 'object' ? saved : {}) };
  // Sanitize.
  if (id === 'coordinates') {
    if (!['find', 'name'].includes(p.mode)) p.mode = 'find';
    if (!['white', 'black'].includes(p.color)) p.color = 'white';
    p.coords = p.coords !== false;
  }
  if (id === 'checks' && !['checks', 'captures'].includes(p.variant)) p.variant = 'checks';
  if (id === 'knight' && !['basic', 'advanced'].includes(p.variant)) p.variant = 'basic';
  return p;
}

function savePrefs(id, p) {
  const all = readPrefs();
  all[id] = p;
  writePrefs(all);
}

function variantOf(id, p) {
  if (id === 'coordinates') return `${p.mode}-${p.color}`;
  return p.variant || 'standard';
}

function randomSquare(except) {
  let sq;
  do { sq = FILES[Math.floor(Math.random() * 8)] + (1 + Math.floor(Math.random() * 8)); } while (sq === except);
  return sq;
}

function shuffle(arr) {
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [arr[i], arr[j]] = [arr[j], arr[i]];
  }
  return arr;
}

/** Four distinct square names including `sq`, with tricky neighbours as distractors. */
function squareOptions(sq) {
  const f = sq.charCodeAt(0) - 97;
  const r = Number(sq[1]) - 1;
  const near = [];
  for (const [df, dr] of [[0, 1], [0, -1], [1, 0], [-1, 0], [1, 1], [-1, -1], [7 - 2 * f, 0], [0, 7 - 2 * r], [2, 0], [0, 2]]) {
    const nf = f + df; const nr = r + dr;
    if (nf >= 0 && nf < 8 && nr >= 0 && nr < 8 && (df || dr)) near.push(FILES[nf] + (nr + 1));
  }
  const opts = new Set([sq]);
  for (const s of shuffle(near)) { if (opts.size >= 4) break; opts.add(s); }
  while (opts.size < 4) opts.add(randomSquare(sq));
  return shuffle([...opts]);
}

function isKnightHop(a, b) {
  const df = Math.abs(a.charCodeAt(0) - b.charCodeAt(0));
  const dr = Math.abs(Number(a[1]) - Number(b[1]));
  return (df === 1 && dr === 2) || (df === 2 && dr === 1);
}

/** 64-entry grid (a8..h1 row-major) from a FEN placement. */
function fenGrid(fen) {
  const rows = String(fen).split(' ')[0].split('/');
  const grid = [];
  for (const row of rows) {
    for (const c of row) {
      if (/\d/.test(c)) for (let i = 0; i < Number(c); i++) grid.push(null);
      else grid.push(c);
    }
  }
  return grid.length === 64 ? grid : new Array(64).fill(null);
}

function gridFen(grid, rest = 'w - - 0 1') {
  const rows = [];
  for (let r = 0; r < 8; r++) {
    let s = ''; let empty = 0;
    for (let c = 0; c < 8; c++) {
      const p = grid[r * 8 + c];
      if (p) { if (empty) { s += empty; empty = 0; } s += p; } else empty++;
    }
    if (empty) s += empty;
    rows.push(s);
  }
  return `${rows.join('/')} ${rest}`;
}

const sqIndex = (sq) => (8 - Number(sq[1])) * 8 + (sq.charCodeAt(0) - 97);

function moveKnight(fen, from, to) {
  const g = fenGrid(fen);
  g[sqIndex(to)] = g[sqIndex(from)] || 'N';
  g[sqIndex(from)] = null;
  return gridFen(g);
}

function sideToMove(fen) { return String(fen).split(' ')[1] === 'b' ? 'black' : 'white'; }

function diffLabel(d) {
  if (d > 0) return t('drills.material.whitePlus', { n: d });
  if (d < 0) return t('drills.material.blackPlus', { n: -d });
  return t('drills.material.equal');
}

function pieceName(code) {
  const role = String(code || '').slice(1).toLowerCase();
  return t(`drills.pieces.${['p', 'n', 'b', 'r', 'q', 'k'].includes(role) ? role : 'p'}`);
}

function accuracyOf(run) {
  const total = run.correct + run.wrong;
  return total ? Math.round((run.correct * 100) / total) : null;
}

/** Children list without null/false (native replaceChildren would print "null"). */
function kids(...list) {
  return list.flat().filter((x) => x != null && x !== false);
}

function durationLabel(seconds) {
  return t('drills.seconds', { n: seconds });
}

/** Tiny bar sparkline of recent scores. */
function sparkline(values) {
  if (!values || values.length < 2 || Math.max(...values) <= 0) return null;
  const max = Math.max(1, ...values);
  return h('div', { class: 'drl-spark', 'aria-hidden': 'true' },
    values.map((v, i) => h('span', {
      class: i === values.length - 1 ? 'last' : null,
      style: { '--h': `${Math.max(8, Math.round((v / max) * 100))}%` },
    })));
}

function bestOf(drillStats) {
  if (!drillStats) return null;
  let best = null; let plays = 0;
  for (const v of drillStats.variants || []) {
    plays += v.plays || 0;
    if (v.best != null && (best == null || v.best > best)) best = v.best;
  }
  return { best, plays };
}

/** Burst of CSS confetti inside `host` (skipped with reduced motion). */
function confetti(host, later) {
  if (reducedMotion() || !host) return;
  const layer = h('div', { class: 'drl-confetti', 'aria-hidden': 'true' });
  const tones = ['var(--primary)', 'var(--gold)', 'var(--info)', 'var(--accent)', 'var(--danger)'];
  for (let i = 0; i < 28; i++) {
    layer.appendChild(h('i', {
      style: {
        '--x': `${Math.round(Math.random() * 240 - 120)}px`,
        '--y': `${Math.round(-80 - Math.random() * 160)}px`,
        '--r': `${Math.round(Math.random() * 720 - 360)}deg`,
        '--d': `${Math.round(Math.random() * 120)}ms`,
        '--c': tones[i % tones.length],
      },
    }));
  }
  host.appendChild(layer);
  later(() => layer.remove(), 1600);
}

// ============================================================================ mount

export async function mount(root, { params = {} } = {}) {
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const id = params.drillId;
  if (id && !DRILL_IDS.includes(id)) {
    root.append(h('div', { class: 'page' }, emptyState({
      icon: 'help', title: t('drills.notFoundTitle'), text: t('drills.notFoundText'),
      action: { label: t('drills.allDrills'), href: '#/drills' },
    })));
    return () => bag.dispose();
  }
  if (id) mountDrill(root, id, bag, ctrl.signal);
  else mountHub(root, bag, ctrl.signal);
  return () => bag.dispose();
}

// ============================================================================ hub

function mountHub(root, bag, signal) {
  const grid = h('div', { class: 'drl-grid' });
  const page = h('div', { class: 'page drl-hub' },
    pageHeader({ title: t('drills.title'), subtitle: t('drills.subtitle'), icon: 'bolt' }),
    h('p', { class: 'drl-hub-intro muted' }, t('drills.hubIntro')),
    grid);
  root.append(page);

  const render = (stats) => {
    const byId = new Map((stats || []).map((d) => [d.id, d]));
    grid.replaceChildren(...kids(...DRILL_IDS.map((id, i) => {
      const meta = META[id];
      const s = bestOf(byId.get(id));
      const recent = (() => {
        const ds = byId.get(id);
        if (!ds) return null;
        const v = [...(ds.variants || [])].sort((a, b) => (b.plays || 0) - (a.plays || 0))[0];
        return v && v.recent;
      })();
      return h('a', {
        class: 'card card-link drl-card', href: `#/drills/${id}`, dataset: { tone: meta.tone },
        style: { '--i': i },
      },
        h('div', { class: 'drl-card-top' },
          h('div', { class: 'drl-card-emoji', 'aria-hidden': 'true' }, meta.emoji),
          h('span', { class: 'badge drl-time', html: icon('timer', { size: 14 }) }, durationLabel(meta.seconds))),
        h('div', { class: 'drl-card-title' }, t(`drills.${id}.title`)),
        h('p', { class: 'drl-card-blurb' }, t(`drills.${id}.blurb`)),
        h('div', { class: 'drl-card-foot' },
          s && s.best != null
            ? h('span', { class: 'drl-best', html: icon('trophy', { size: 16 }) }, t('drills.bestShort', { n: formatNumber(s.best) }))
            : h('span', { class: 'drl-best new' }, t('drills.notPlayed')),
          sparkline(recent),
          h('span', { class: 'drl-play' }, t('drills.play'), h('span', { html: icon('chevron-right', { size: 16 }) }))));
    })));
  };

  grid.append(skeleton('card', 5));
  const load = () => {
    api.get('/api/drills', { signal })
      .then((data) => { if (!bag.disposed) render(data && data.drills); })
      .catch((e) => {
        if (isAbort(e) || bag.disposed) return;
        // Still show the drills; bests are just unavailable.
        render([]);
      });
  };
  load();
}

// ============================================================================ drill

function mountDrill(root, id, bag, signal) {
  const meta = META[id];
  let prefs = prefsFor(id);
  let stats = null;            // DrillStats for this drill (from GET /api/drills)
  let phase = 'intro';         // intro | countdown | play | over
  let run = null;
  let board = null;
  let pool = [];               // prefetched batch for the next run
  let poolVariant = null;
  let poolPromise = null;
  let rafId = 0;
  let typed = '';              // keyboard square entry buffer
  let lastResult = null;
  const timers = new Set();
  const later = (fn, ms) => {
    const tid = setTimeout(() => { timers.delete(tid); if (!bag.disposed) fn(); }, ms);
    timers.add(tid);
    return tid;
  };
  bag.add(() => { for (const tid of timers) clearTimeout(tid); timers.clear(); });
  bag.add(() => cancelAnimationFrame(rafId));
  bag.add(() => { if (board) { board.destroy(); board = null; } });

  // ------------------------------------------------------------------ DOM
  const live = h('div', { class: 'sr-only', 'aria-live': 'polite', role: 'status' });
  const timerFill = h('div', { class: 'drl-timer-fill' });
  const timerText = h('span', { class: 'drl-timer-text tabular' }, String(meta.seconds));
  const scoreText = h('span', { class: 'drl-score-num tabular' }, '0');
  const promptEl = h('div', { class: 'drl-prompt' });
  const hud = h('div', { class: 'drl-hud' },
    h('div', { class: 'drl-timer', 'aria-hidden': 'true' },
      h('span', { class: 'drl-timer-icon', html: icon('timer', { size: 16 }) }), timerText,
      h('div', { class: 'drl-timer-track' }, timerFill)),
    promptEl,
    h('div', { class: 'drl-score', 'aria-label': t('drills.score') },
      h('span', { class: 'drl-score-label' }, t('drills.score')), scoreText));
  const boardSlot = h('div', { class: 'board-slot drl-board' });
  const overlay = h('div', { class: 'drl-overlay' });
  boardSlot.append(overlay);
  const answers = h('div', { class: 'drl-answers' });
  const panelBody = h('div', { class: 'panel-body drl-panel-body' });
  const panel = h('div', { class: 'panel grow drl-panel' },
    h('div', { class: 'panel-header drl-panel-header' },
      h('a', { class: 'btn btn-ghost btn-sm btn-icon', href: '#/drills', 'aria-label': t('drills.allDrills'), html: icon('chevron-left') }),
      h('span', { class: 'drl-panel-emoji', 'aria-hidden': 'true' }, meta.emoji),
      h('span', { class: 'truncate' }, t(`drills.${id}.title`))),
    panelBody);
  const layout = h('div', { class: `game-layout no-eval drl-layout drl-${id}`, dataset: { tone: meta.tone, phase } },
    h('div', { class: 'game-main' }, hud, h('div', { class: 'board-row' }, boardSlot), answers),
    h('aside', { class: 'game-panel' }, panel),
    live);
  root.append(layout);

  const setPhase = (p) => { phase = p; layout.dataset.phase = p; };
  const announce = (msg) => { live.textContent = ''; later(() => { live.textContent = msg; }, 30); };

  // ------------------------------------------------------------------ board
  function boardConfig() {
    const base = { interactive: false, movableColor: null, sounds: true, onSquareClick: onSquareClick, onMove: onBoardMove };
    if (id === 'coordinates') return { ...base, fen: EMPTY_FEN, orientation: prefs.color, showCoords: prefs.coords };
    return { ...base, fen: EMPTY_FEN, orientation: 'white' };
  }

  function createBoard() {
    if (board) board.destroy();
    board = new Board(boardSlot, boardConfig());
    // Keep the overlay above the board.
    boardSlot.append(overlay);
    showSample();
  }

  /** What the board shows on the start screen. */
  function showSample() {
    if (!board) return;
    board.setHighlights([]);
    board.setArrows([]);
    board.setCircles([]);
    if (id === 'coordinates') { board.setPosition(EMPTY_FEN, { animate: false }); return; }
    const sample = pool[0];
    if (sample && sample.fen) {
      board.setOrientation(id === 'checks' ? sideToMove(sample.fen) : 'white');
      board.setPosition(sample.fen, { animate: false });
    } else {
      board.setPosition('start', { animate: false });
    }
  }

  // ------------------------------------------------------------------ data
  function loadStats() {
    api.get('/api/drills', { signal })
      .then((data) => {
        if (bag.disposed) return;
        stats = ((data && data.drills) || []).find((d) => d.id === id) || null;
        if (phase === 'intro') renderIntro();
      })
      .catch((e) => { if (!isAbort(e) && !bag.disposed && phase === 'intro') renderIntro(); });
  }

  function variantStats(variant = variantOf(id, prefs)) {
    return stats && (stats.variants || []).find((v) => v.id === variant);
  }

  function fetchBatch(variant) {
    return api.get(`/api/drills/${id}/batch` + qs({ n: BATCH_SIZE, variant }), { signal })
      .then((data) => (data && Array.isArray(data.items) ? data.items : []));
  }

  /** Prefetch a batch for the selected variant (used by the start screen and the first questions). */
  function preparePool() {
    if (!meta.server) return Promise.resolve();
    const variant = variantOf(id, prefs);
    if (poolVariant === variant && (pool.length || poolPromise)) return poolPromise || Promise.resolve();
    poolVariant = variant;
    pool = [];
    const p = fetchBatch(variant).then((items) => {
      if (bag.disposed || poolVariant !== variant) return;
      pool = items;
      if (phase === 'intro') showSample();
    }).catch((e) => {
      if (isAbort(e) || bag.disposed) return;
      if (poolVariant === variant) poolVariant = null;
      throw e;
    }).finally(() => { if (poolPromise === p) poolPromise = null; });
    poolPromise = p;
    return p;
  }

  /** Keep the run queue topped up in the background. */
  function refill() {
    if (!run || run.refilling || !meta.server || run.queue.length >= REFILL_AT) return;
    run.refilling = true;
    const r = run;
    fetchBatch(r.variant).then((items) => {
      if (bag.disposed || run !== r) return;
      r.queue.push(...items);
      if (r.queue.length > MAX_QUEUE) r.queue.length = MAX_QUEUE;
      if (r.waiting) { r.waiting = false; nextQuestion(); }
    }).catch(() => { /* retried on the next question */ })
      .finally(() => { r.refilling = false; });
  }

  // ------------------------------------------------------------------ intro
  function optionGroup(label, name, choices, current, onPick) {
    return h('div', { class: 'drl-opt' },
      h('div', { class: 'drl-opt-label', id: `drl-opt-${name}` }, label),
      h('div', { class: 'segmented block', role: 'radiogroup', 'aria-labelledby': `drl-opt-${name}` },
        choices.map(([value, text]) => h('button', {
          type: 'button', role: 'radio', class: value === current ? 'active' : null,
          'aria-checked': value === current ? 'true' : 'false',
          onClick: () => { if (value !== current) onPick(value); },
        }, text))));
  }

  function updatePrefs(patch) {
    prefs = { ...prefs, ...patch };
    savePrefs(id, prefs);
    if (id === 'coordinates') createBoard();
    preparePool().catch(() => {});
    renderIntro();
  }

  function optionsBlock() {
    if (id === 'coordinates') {
      return h('div', { class: 'stack-sm' },
        optionGroup(t('drills.options.mode'), 'mode', [['find', t('drills.coordinates.find')], ['name', t('drills.coordinates.name')]], prefs.mode, (v) => updatePrefs({ mode: v })),
        optionGroup(t('drills.options.playAs'), 'color', [['white', t('common.white')], ['black', t('common.black')]], prefs.color, (v) => updatePrefs({ color: v })),
        h('label', { class: 'switch drl-switch' },
          h('input', { type: 'checkbox', checked: prefs.coords, onChange: (e) => updatePrefs({ coords: e.target.checked }) }),
          h('span', { class: 'switch-track' }), t('drills.options.showCoords')));
    }
    if (id === 'checks') {
      return optionGroup(t('drills.options.find'), 'variant', [['checks', t('drills.checks.variantChecks')], ['captures', t('drills.checks.variantCaptures')]], prefs.variant, (v) => updatePrefs({ variant: v }));
    }
    if (id === 'knight') {
      return optionGroup(t('drills.options.level'), 'variant', [['basic', t('drills.knight.basic')], ['advanced', t('drills.knight.advanced')]], prefs.variant, (v) => updatePrefs({ variant: v }));
    }
    return null;
  }

  function rulesKey() {
    if (id === 'coordinates') return `drills.coordinates.rules.${prefs.mode}`;
    if (id === 'checks') return `drills.checks.rules.${prefs.variant}`;
    if (id === 'knight') return `drills.knight.rules.${prefs.variant}`;
    return `drills.${id}.rules`;
  }

  function bestBlock() {
    const v = variantStats();
    return h('div', { class: 'drl-bests' },
      h('div', { class: 'stat drl-stat' },
        h('div', { class: 'stat-label' }, t('drills.personalBest')),
        h('div', { class: 'stat-value tabular' }, v && v.best != null ? formatNumber(v.best) : '–')),
      h('div', { class: 'stat drl-stat' },
        h('div', { class: 'stat-label' }, t('drills.plays')),
        h('div', { class: 'stat-value tabular' }, formatNumber((v && v.plays) || 0))),
      v && v.recent && v.recent.length > 1
        ? h('div', { class: 'drl-recent' }, h('div', { class: 'stat-label' }, t('drills.recent')), sparkline(v.recent))
        : null);
  }

  function renderIntro() {
    setPhase('intro');
    renderHud();
    answers.replaceChildren(...kids());
    const rules = t(rulesKey());
    panelBody.replaceChildren(...kids(
      h('p', { class: 'drl-desc' }, t(`drills.${id}.description`)),
      h('ul', { class: 'drl-rules' }, (Array.isArray(rules) ? rules : [rules]).map((r) => h('li', null, r))),
      optionsBlock(),
      bestBlock(),
      h('button', {
        class: 'btn btn-primary btn-lg btn-block drl-start', type: 'button',
        html: icon('play-circle') + `<span>${t('drills.start')}</span>`, onClick: startRun,
      }),
      h('p', { class: 'drl-kbd-hint subtle text-xs' }, t(`drills.kbd.${kbdKind()}`))));
    overlay.replaceChildren(...kids(
      h('div', { class: 'drl-ov-card drl-ov-intro' },
        h('div', { class: 'drl-ov-emoji', 'aria-hidden': 'true' }, meta.emoji),
        h('div', { class: 'drl-ov-title' }, t(`drills.${id}.title`)),
        h('button', { class: 'btn btn-primary btn-lg', type: 'button', html: icon('play-circle') + `<span>${t('drills.start')}</span>`, onClick: startRun }),
        h('div', { class: 'drl-ov-sub' }, durationLabel(meta.seconds)))));
    overlay.hidden = false;
  }

  function kbdKind() {
    if (id === 'material' || (id === 'coordinates' && prefs.mode === 'name')) return 'options';
    if (id === 'checks') return 'moves';
    return 'squares';
  }

  // ------------------------------------------------------------------ HUD
  function renderHud() {
    const over = phase === 'over';
    const secs = run && phase === 'play' ? Math.max(0, Math.ceil((run.endsAt - performance.now()) / 1000)) : (over ? 0 : meta.seconds);
    timerText.textContent = String(secs);
    const frac = run && phase === 'play' ? Math.max(0, (run.endsAt - performance.now()) / (meta.seconds * 1000)) : (over ? 0 : 1);
    timerFill.style.transform = `scaleX(${frac})`;
    hud.classList.toggle('low', phase === 'play' && secs <= 5);
    scoreText.textContent = formatNumber(run ? run.score : 0);
    if (phase !== 'play') promptEl.replaceChildren(...kids(h('span', { class: 'muted' }, phase === 'over' ? t('drills.timeUp') : t('drills.ready'))));
  }

  function bumpScore() {
    scoreText.textContent = formatNumber(run.score);
    scoreText.classList.remove('bump');
    void scoreText.offsetWidth;
    scoreText.classList.add('bump');
  }

  function floatText(text, kind = 'good') {
    if (reducedMotion()) return;
    const el = h('div', { class: `drl-float ${kind}`, 'aria-hidden': 'true' }, text);
    boardSlot.appendChild(el);
    later(() => el.remove(), 900);
  }

  function shakeBoard() {
    boardSlot.classList.remove('shake');
    void boardSlot.offsetWidth;
    boardSlot.classList.add('shake');
    later(() => boardSlot.classList.remove('shake'), 400);
  }

  function tick() {
    if (!run || phase !== 'play') return;
    renderHud();
    if (performance.now() >= run.endsAt) { endRun(); return; }
    rafId = requestAnimationFrame(tick);
  }

  // ------------------------------------------------------------------ run lifecycle
  async function startRun() {
    if (phase === 'countdown' || phase === 'play') return;
    const variant = variantOf(id, prefs);
    run = {
      variant, score: 0, correct: 0, wrong: 0, streak: 0, bestStreak: 0, mistakes: [],
      queue: [], item: null, st: null, busy: false, waiting: false, refilling: false,
      startedAt: 0, endsAt: 0, lastSquare: null,
    };
    setPhase('countdown');
    typed = '';
    answers.replaceChildren(...kids());
    board.setInteractive(false);
    showSample();
    panelBody.replaceChildren(...kids(liveStats()));
    renderHud();
    // Make sure the first questions are here before "Go!".
    const ready = meta.server ? preparePool().catch((e) => e) : Promise.resolve();
    for (let n = COUNTDOWN; n > 0; n--) {
      overlay.replaceChildren(...kids(h('div', { class: 'drl-count' }, String(n))));
      overlay.hidden = false;
      announce(String(n));
      playSound('move');
      await new Promise((res) => later(res, 650));
      if (bag.disposed || phase !== 'countdown') return;
    }
    const err = await ready;
    if (bag.disposed || phase !== 'countdown') return;
    if (meta.server) {
      if (err instanceof Error || !pool.length) {
        setPhase('intro');
        renderIntro();
        panelBody.prepend(h('div', { class: 'callout callout-danger' }, h('div', null, t('drills.loadFailed'))));
        poolVariant = null;
        return;
      }
      run.queue = pool.slice();
      pool = [];
      poolVariant = null;
    }
    overlay.replaceChildren(...kids(h('div', { class: 'drl-count go' }, t('drills.go'))));
    later(() => { if (phase === 'play' && !(id === 'coordinates' && prefs.mode === 'find')) overlay.hidden = true; }, 450);
    playSound('notify');
    setPhase('play');
    run.startedAt = performance.now();
    run.endsAt = run.startedAt + meta.seconds * 1000;
    rafId = requestAnimationFrame(tick);
    nextQuestion();
    refill();
  }

  function liveStats() {
    return h('div', { class: 'drl-live stack' },
      h('p', { class: 'drl-desc' }, t(`drills.${id}.description`)),
      h('div', { class: 'drl-live-grid' },
        h('div', { class: 'stat drl-stat' }, h('div', { class: 'stat-label' }, t('drills.correct')), h('div', { class: 'stat-value tabular drl-live-correct' }, '0')),
        h('div', { class: 'stat drl-stat' }, h('div', { class: 'stat-label' }, t('drills.mistakes')), h('div', { class: 'stat-value tabular drl-live-wrong' }, '0')),
        h('div', { class: 'stat drl-stat' }, h('div', { class: 'stat-label' }, t('drills.streak')), h('div', { class: 'stat-value tabular drl-live-streak' }, '0'))),
      h('button', { class: 'btn btn-ghost btn-sm drl-end', type: 'button', onClick: () => { if (phase === 'play') endRun(); else if (phase === 'countdown') renderIntro(); } }, t('drills.endNow')));
  }

  function updateLive() {
    const q = (sel) => panelBody.querySelector(sel);
    const c = q('.drl-live-correct'); if (c) c.textContent = formatNumber(run.correct);
    const w = q('.drl-live-wrong'); if (w) w.textContent = formatNumber(run.wrong);
    const s = q('.drl-live-streak'); if (s) s.textContent = formatNumber(run.streak);
  }

  function addMistake(m) {
    if (run.mistakes.length < MAX_MISTAKES) run.mistakes.push(m);
  }

  function markCorrect(points = 1, { sound = true } = {}) {
    run.score += points;
    run.correct += 1;
    run.streak += 1;
    run.bestStreak = Math.max(run.bestStreak, run.streak);
    if (sound) playSound('correct');
    bumpScore();
    updateLive();
  }

  function markWrong() {
    run.wrong += 1;
    run.streak = 0;
    playSound('wrong');
    shakeBoard();
    updateLive();
  }

  function setPrompt(...children) {
    promptEl.replaceChildren(...kids(...children));
  }

  function nextQuestion() {
    if (!run || phase !== 'play') return;
    run.busy = false;
    typed = '';
    board.setArrows([]);
    board.setCircles([]);
    board.setHighlights([]);
    if (id === 'coordinates') { nextCoordinate(); return; }
    const item = run.queue.shift();
    refill();
    if (!item) {
      run.waiting = true;
      setPrompt(h('span', { class: 'muted' }, t('common.loading')));
      return;
    }
    run.item = item;
    if (id === 'hanging') startHanging(item);
    else if (id === 'material') startMaterial(item);
    else if (id === 'checks') startChecks(item);
    else if (id === 'knight') startKnight(item);
  }

  async function endRun() {
    if (!run || phase !== 'play') return;
    cancelAnimationFrame(rafId);
    if (id === 'checks') finishChecksPosition(true); // time ran out: only wrong moves count as mistakes
    const elapsed = Math.min(meta.seconds * 1000, Math.round(performance.now() - run.startedAt));
    setPhase('over');
    board.setInteractive(false);
    board.setHighlights([]);
    board.setArrows([]);
    board.setCircles([]);
    answers.replaceChildren(...kids());
    renderHud();
    playSound('gameEnd');
    const r = run;
    const prev = variantStats(r.variant);
    lastResult = { pending: true, best: prev && prev.best, is_best: false, previous_best: prev ? prev.best : null };
    renderOver();
    preparePool().catch(() => {});
    try {
      const res = await api.post(`/api/drills/${id}/score`, {
        variant: r.variant, score: r.score, correct: r.correct, total: r.correct + r.wrong, duration_ms: elapsed,
      }, { signal });
      if (bag.disposed || run !== r) return;
      lastResult = { ...res, pending: false };
      if (stats) {
        const v = (stats.variants || []).find((x) => x.id === r.variant);
        if (v) {
          v.best = res.best; v.plays = res.plays;
          v.recent = [...(v.recent || []), r.score].slice(-10);
        }
      }
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      lastResult = { pending: false, failed: true, best: prev && prev.best, is_best: false };
    }
    if (phase === 'over' && run === r) {
      renderOver();
      if (lastResult.is_best && r.score > 0) {
        playSound('promote');
        confetti(boardSlot, later);
      }
    }
  }

  function renderOver() {
    const r = run;
    const res = lastResult || {};
    const acc = accuracyOf(r);
    const newBest = !res.pending && res.is_best && r.score > 0;
    const firstBest = newBest && res.previous_best == null;
    overlay.replaceChildren(...kids(
      h('div', { class: `drl-ov-card drl-ov-over${newBest ? ' best' : ''}` },
        h('div', { class: 'drl-ov-kicker' }, newBest ? (firstBest ? t('drills.firstScore') : t('drills.newBest')) : t('drills.timeUp')),
        h('div', { class: 'drl-ov-score tabular' }, formatNumber(r.score)),
        h('div', { class: 'drl-ov-sub' }, res.best != null && !newBest
          ? t('drills.bestIs', { n: formatNumber(res.best) })
          : (acc == null ? t('drills.points', { count: r.score }) : t('drills.accuracyLine', { n: acc }))),
        h('button', { class: 'btn btn-primary btn-lg', type: 'button', html: icon('refresh') + `<span>${t('drills.again')}</span>`, onClick: startRun }))));
    overlay.hidden = false;

    const mistakesList = r.mistakes.length
      ? h('div', { class: 'drl-mistakes' }, r.mistakes.map((m, i) => h('button', {
        type: 'button', class: 'list-row drl-mistake', onClick: (e) => showMistake(m, e.currentTarget),
      },
        h('span', { class: 'drl-mistake-n tabular' }, String(i + 1)),
        h('span', { class: 'list-row-main' }, h('span', { class: 'list-row-title' }, mistakeTitle(m)), h('span', { class: 'list-row-sub' }, mistakeSub(m))),
        h('span', { html: icon('eye', { size: 16 }), class: 'subtle' }))))
      : h('div', { class: 'callout callout-success' }, h('div', null, r.correct ? t('drills.noMistakes') : t('drills.noAnswers')));

    panelBody.replaceChildren(...kids(
      h('div', { class: `result-hero drl-result${newBest ? ' best' : ''}` },
        h('div', { class: 'result-hero-title' }, newBest ? t('drills.newBestTitle') : t(`drills.cheer.${cheerKey(r, res)}`)),
        h('div', { class: 'result-hero-sub' }, t('drills.scoreLine', { n: formatNumber(r.score) }))),
      h('div', { class: 'drl-live-grid' },
        h('div', { class: 'stat drl-stat' }, h('div', { class: 'stat-label' }, t('drills.personalBest')),
          h('div', { class: 'stat-value tabular' }, res.pending ? '…' : (res.best != null ? formatNumber(res.best) : '–'))),
        h('div', { class: 'stat drl-stat' }, h('div', { class: 'stat-label' }, t('drills.accuracy')),
          h('div', { class: 'stat-value tabular' }, acc == null ? '–' : `${acc}%`)),
        h('div', { class: 'stat drl-stat' }, h('div', { class: 'stat-label' }, t('drills.bestStreak')),
          h('div', { class: 'stat-value tabular' }, formatNumber(r.bestStreak)))),
      res.failed ? h('div', { class: 'callout callout-warning' }, h('div', null, t('drills.saveFailed'))) : null,
      h('div', { class: 'row-sm drl-over-actions' },
        h('button', { class: 'btn btn-primary grow-btn', type: 'button', html: icon('refresh') + `<span>${t('drills.again')}</span>`, onClick: startRun }),
        h('button', { class: 'btn btn-secondary', type: 'button', html: icon('settings') + `<span>${t('drills.changeOptions')}</span>`, onClick: () => { run = null; createBoard(); renderIntro(); } })),
      h('h3', { class: 'drl-h3' }, t('drills.reviewTitle'), r.mistakes.length ? h('span', { class: 'badge' }, String(r.mistakes.length)) : null),
      r.mistakes.length ? h('p', { class: 'subtle text-sm drl-review-hint' }, t('drills.reviewHint')) : null,
      mistakesList,
      h('a', { class: 'btn btn-ghost btn-sm drl-all', href: '#/drills', html: icon('grid') + `<span>${t('drills.allDrills')}</span>` })));
  }

  function cheerKey(r, res) {
    if (!r.correct) return 'zero';
    const acc = accuracyOf(r) || 0;
    if (res.best != null && r.score >= res.best * 0.8) return 'close';
    return acc >= 90 ? 'sharp' : 'good';
  }

  // ------------------------------------------------------------------ mistakes review
  function mistakeTitle(m) {
    switch (m.kind) {
      case 'coord': return t('drills.review.coord', { target: m.target, got: m.got });
      case 'hanging': return m.got ? t('drills.review.hangingWrong', { got: m.got }) : t('drills.review.hangingSkipped');
      case 'material': return t('drills.review.material', { got: diffLabel(m.got), answer: diffLabel(m.answer) });
      case 'checks': return t(`drills.review.${run && run.variant === 'captures' ? 'captures' : 'checks'}`, { missed: m.missed.length, wrong: m.wrongs.length });
      case 'knight': return m.failed ? t('drills.review.knightFailed', { min: m.min }) : t('drills.review.knight', { hops: m.hops, min: m.min });
      default: return '';
    }
  }

  function mistakeSub(m) {
    switch (m.kind) {
      case 'coord': return t('drills.review.coordSub');
      case 'hanging': return t('drills.review.hangingSub', { list: m.hanging.map((p) => `${pieceName(p.piece)} ${p.square}`).join(', ') });
      case 'material': return t('drills.review.materialSub', { white: m.white, black: m.black });
      case 'checks': return m.missed.length ? t('drills.review.missedList', { list: m.missed.join(', ') }) : t('drills.review.wrongList', { list: m.wrongs.join(', ') });
      case 'knight': return t('drills.review.knightSub', { route: [m.start, ...m.path].join(' → ') });
      default: return '';
    }
  }

  function showMistake(m, btn) {
    if (phase !== 'over') return;
    for (const el of panelBody.querySelectorAll('.drl-mistake.active')) el.classList.remove('active');
    if (btn) btn.classList.add('active');
    overlay.hidden = true;
    board.setArrows([]);
    board.setCircles([]);
    if (m.kind === 'coord') {
      board.setPosition(EMPTY_FEN, { animate: false });
      board.setHighlights([{ square: m.target, kind: 'good' }, ...(m.got && m.got !== m.target ? [{ square: m.got, kind: 'bad' }] : [])]);
    } else if (m.kind === 'hanging') {
      board.setOrientation('white');
      board.setPosition(m.fen, { animate: false });
      board.setHighlights([...m.hanging.map((p) => ({ square: p.square, kind: 'good' })), ...(m.got ? [{ square: m.got, kind: 'bad' }] : [])]);
    } else if (m.kind === 'material') {
      board.setOrientation('white');
      board.setPosition(m.fen, { animate: false });
      board.setHighlights([]);
    } else if (m.kind === 'checks') {
      board.setOrientation(sideToMove(m.fen));
      board.setPosition(m.fen, { animate: false });
      board.setHighlights([]);
      board.setArrows(m.answers.map((a) => ({ from: a.uci.slice(0, 2), to: a.uci.slice(2, 4), color: m.found.includes(a.uci) ? 'green' : 'yellow' })));
    } else if (m.kind === 'knight') {
      board.setOrientation('white');
      board.setPosition(m.fen, { animate: false });
      board.setHighlights([...m.blocked.map((s) => ({ square: s, kind: 'bad' })), { square: m.target, kind: 'target' }]);
      const hops = [m.start, ...m.path];
      board.setArrows(hops.slice(1).map((to, i) => ({ from: hops[i], to, color: 'green' })));
    }
    // On phones the board sits above the panel: bring it into view.
    try { boardSlot.scrollIntoView({ block: 'nearest', behavior: reducedMotion() ? 'auto' : 'smooth' }); } catch { /* old browsers */ }
    announce(mistakeTitle(m));
  }

  // ------------------------------------------------------------------ input routing
  function onSquareClick(sq) {
    if (phase !== 'play' || !run || run.busy) return;
    if (id === 'coordinates' && prefs.mode === 'find') answerCoordinate(sq);
    else if (id === 'hanging') tapHanging(sq);
    else if (id === 'knight') tapKnight(sq);
  }

  function onBoardMove(mv) {
    if (phase !== 'play' || !run || id !== 'checks') return false;
    tryChecksMove(mv.from, mv.to, mv.san);
    return false; // always snap back: the position stays the same
  }

  function onKey(e) {
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
    const tag = (e.target && e.target.tagName) || '';
    if (/^(INPUT|TEXTAREA|SELECT)$/.test(tag) || (e.target && e.target.isContentEditable)) return;
    const k = e.key;
    if (phase === 'intro' || phase === 'over') {
      if ((k === 'Enter' || k === ' ') && !/^(BUTTON|A)$/.test(tag)) { e.preventDefault(); startRun(); }
      return;
    }
    if (phase !== 'play' || !run) return;
    if (k === 'Escape') { typed = ''; renderTyped(); return; }
    const optionsMode = kbdKind() === 'options';
    if (optionsMode && /^[1-4]$/.test(k)) {
      const btn = answers.querySelectorAll('button.drl-answer')[Number(k) - 1];
      if (btn) { e.preventDefault(); btn.click(); }
      return;
    }
    if (optionsMode) return;
    if (id === 'checks' && (k === 'Enter' || k === 'n' || k === 'N')) {
      if (k === 'Enter' || !typed) { e.preventDefault(); skipChecks(); return; }
    }
    const lower = k.length === 1 ? k.toLowerCase() : '';
    if (lower && FILES.includes(lower)) { typed = (typed.length % 2 === 0 ? typed : typed.slice(0, -1)) + lower; renderTyped(); e.preventDefault(); return; }
    if (/^[1-8]$/.test(k) && typed.length % 2 === 1) {
      typed += k;
      e.preventDefault();
      const sq = typed.slice(-2);
      if (id === 'checks') {
        if (typed.length >= 4) {
          const from = typed.slice(0, 2); typed = '';
          renderTyped();
          tryChecksMove(from, sq, null);
        } else {
          renderTyped();
          board.setHighlights([{ square: sq, kind: 'selected' }]);
        }
      } else {
        typed = '';
        renderTyped();
        onSquareClick(sq);
      }
      return;
    }
    if (k === 'Backspace' && typed) { typed = typed.slice(0, -1); renderTyped(); e.preventDefault(); }
  }
  bag.on(window, 'keydown', onKey);

  function renderTyped() {
    let el = hud.querySelector('.drl-typed');
    if (!typed) { if (el) el.remove(); if (id === 'checks' && board) board.setHighlights([]); return; }
    if (!el) { el = h('kbd', { class: 'drl-typed' }); hud.append(el); }
    el.textContent = typed;
  }

  // ------------------------------------------------------------------ coordinates
  function nextCoordinate() {
    const sq = randomSquare(run.lastSquare);
    run.lastSquare = sq;
    run.item = { square: sq };
    if (prefs.mode === 'find') {
      setPrompt(h('span', null, t('drills.coordinates.findPrompt')), h('strong', { class: 'drl-target' }, sq));
      overlay.hidden = false;
      overlay.replaceChildren(...kids(h('div', { class: 'drl-coord-target' }, sq)));
      announce(t('drills.coordinates.findSpoken', { square: sq }));
    } else {
      overlay.hidden = true;
      board.setHighlights([{ square: sq, kind: 'target' }]);
      board.setCircles([{ square: sq, color: 'blue' }]);
      setPrompt(h('span', null, t('drills.coordinates.namePrompt')));
      answers.replaceChildren(...kids(...squareOptions(sq).map((opt, i) => h('button', {
        type: 'button', class: 'btn btn-secondary btn-lg drl-answer', onClick: (e) => answerCoordinate(opt, e.currentTarget),
      }, h('kbd', { class: 'drl-key' }, String(i + 1)), h('span', { class: 'mono' }, opt)))));
      announce(t('drills.coordinates.nameSpoken'));
    }
  }

  function answerCoordinate(sq, btn) {
    if (run.busy) return;
    const target = run.item.square;
    if (sq === target) {
      markCorrect(1, { sound: true });
      board.setHighlights([{ square: sq, kind: 'good' }]);
      if (btn) btn.classList.add('right');
      run.busy = true;
      later(() => { if (phase === 'play') nextQuestion(); }, prefs.mode === 'find' ? 140 : 260);
    } else {
      markWrong();
      addMistake({ kind: 'coord', target, got: sq });
      board.setHighlights([{ square: sq, kind: 'bad' }, { square: target, kind: 'good' }]);
      if (btn) btn.classList.add('wrong');
      const right = [...answers.querySelectorAll('.drl-answer')].find((b) => b.textContent.endsWith(target));
      if (right) right.classList.add('right');
      run.busy = true;
      later(() => { if (phase === 'play') nextQuestion(); }, 650);
    }
  }

  // ------------------------------------------------------------------ hanging
  function startHanging(item) {
    run.st = { found: new Set() };
    board.setOrientation('white');
    board.setPosition(item.fen, { animate: false });
    renderHangingPrompt();
    announce(t('drills.hanging.spoken', { count: item.hanging.length }));
    answers.replaceChildren(...kids(h('div', { class: 'drl-pips', 'aria-hidden': 'true' },
      item.hanging.map(() => h('span', { class: 'drl-pip' }))),
    h('span', { class: 'subtle text-sm' }, t('drills.hanging.legend'))));
  }

  function renderHangingPrompt() {
    const n = run.item.hanging.length;
    setPrompt(h('span', null, t('drills.hanging.prompt', { count: n })), h('span', { class: 'drl-count-chip tabular' }, `${run.st.found.size}/${n}`));
    const pips = answers.querySelectorAll('.drl-pip');
    pips.forEach((p, i) => p.classList.toggle('on', i < run.st.found.size));
  }

  function tapHanging(sq) {
    const item = run.item;
    const st = run.st;
    if (!item || st.found.has(sq)) return;
    const hit = item.hanging.find((p) => p.square === sq);
    if (hit) {
      st.found.add(sq);
      markCorrect(1);
      board.setHighlights([...st.found].map((s) => ({ square: s, kind: 'good' })));
      renderHangingPrompt();
      if (st.found.size === item.hanging.length) {
        floatText(t('drills.nice'));
        run.busy = true;
        later(() => { if (phase === 'play') nextQuestion(); }, 450);
      }
      return;
    }
    // Only pieces count as answers; empty squares are ignored.
    const grid = fenGrid(item.fen);
    if (!grid[sqIndex(sq)]) return;
    markWrong();
    addMistake({ kind: 'hanging', fen: item.fen, hanging: item.hanging, got: sq });
    board.setHighlights([
      ...item.hanging.map((p) => ({ square: p.square, kind: st.found.has(p.square) ? 'good' : 'hint' })),
      { square: sq, kind: 'bad' },
    ]);
    run.busy = true;
    later(() => { if (phase === 'play') nextQuestion(); }, 1100);
  }

  // ------------------------------------------------------------------ material
  function startMaterial(item) {
    board.setOrientation('white');
    board.setPosition(item.fen, { animate: false });
    setPrompt(h('span', null, t('drills.material.prompt')));
    announce(t('drills.material.prompt'));
    answers.replaceChildren(...kids(...item.options.map((d, i) => h('button', {
      type: 'button', class: 'btn btn-secondary drl-answer', dataset: { diff: d },
      onClick: (e) => answerMaterial(d, e.currentTarget),
    }, h('kbd', { class: 'drl-key' }, String(i + 1)), h('span', null, diffLabel(d))))));
  }

  function answerMaterial(d, btn) {
    if (run.busy) return;
    const item = run.item;
    run.busy = true;
    if (d === item.diff) {
      markCorrect(1);
      btn.classList.add('right');
      later(() => { if (phase === 'play') nextQuestion(); }, 280);
    } else {
      markWrong();
      btn.classList.add('wrong');
      const right = answers.querySelector(`.drl-answer[data-diff="${item.diff}"]`);
      if (right) right.classList.add('right');
      addMistake({ kind: 'material', fen: item.fen, got: d, answer: item.diff, white: item.white, black: item.black });
      later(() => { if (phase === 'play') nextQuestion(); }, 900);
    }
  }

  // ------------------------------------------------------------------ checks / captures
  function startChecks(item) {
    run.st = { found: [], wrongs: [] };
    const side = sideToMove(item.fen);
    board.setOrientation(side);
    board.setPosition(item.fen, { animate: false });
    board.setInteractive(true, side);
    renderChecksPrompt();
    announce(t(`drills.checks.spoken.${run.variant}`, { side: t(side === 'white' ? 'common.whiteLower' : 'common.blackLower') }));
    answers.replaceChildren(...kids(
      h('div', { class: 'drl-side' }, h('span', { class: `drl-side-dot ${side}` }), t(side === 'white' ? 'common.whiteToMove' : 'common.blackToMove')),
      h('button', { type: 'button', class: 'btn btn-secondary drl-skip', html: `<span>${t('drills.checks.doneNext')}</span>` + icon('chevron-right'), onClick: skipChecks })));
  }

  function renderChecksPrompt() {
    const n = run.item.answers.length;
    setPrompt(h('span', null, t(`drills.checks.prompt.${run.variant}`, { count: n })), h('span', { class: 'drl-count-chip tabular' }, `${run.st.found.length}/${n}`));
  }

  function tryChecksMove(from, to, san) {
    if (!run || run.busy || phase !== 'play') return;
    const item = run.item;
    const st = run.st;
    const ans = item.answers.find((a) => a.uci.slice(0, 4) === from + to);
    if (ans) {
      if (st.found.includes(ans.uci)) { playSound('illegal'); return; }
      st.found.push(ans.uci);
      markCorrect(1);
      board.setArrows(st.found.map((u) => ({ from: u.slice(0, 2), to: u.slice(2, 4), color: 'green' })));
      renderChecksPrompt();
      if (st.found.length === item.answers.length) {
        run.score += 1; // completeness bonus
        bumpScore();
        floatText(t('drills.checks.complete'));
        finishChecksPosition(true);
        run.busy = true;
        later(() => { if (phase === 'play') nextQuestion(); }, 600);
      }
      return;
    }
    // Typed moves must at least be legal to count.
    let label = san;
    if (!label) {
      const legal = board.legalMoves().find((m) => m.from === from && m.to === to);
      if (!legal) { playSound('illegal'); return; }
      label = legal.san;
    }
    st.wrongs.push(label);
    markWrong();
    board.setHighlights([{ square: to, kind: 'bad' }]);
    later(() => { if (phase === 'play' && run && run.item === item) board.setHighlights([]); }, 500);
  }

  /** Record what was missed in the current position (when leaving it). */
  function finishChecksPosition(complete) {
    const item = run && run.item;
    const st = run && run.st;
    if (!item || !st || st.recorded) return;
    st.recorded = true;
    const missed = complete ? [] : item.answers.filter((a) => !st.found.includes(a.uci)).map((a) => a.san);
    if (missed.length || st.wrongs.length) {
      addMistake({ kind: 'checks', fen: item.fen, answers: item.answers, found: [...st.found], missed, wrongs: [...st.wrongs] });
    }
  }

  function skipChecks() {
    if (!run || run.busy || phase !== 'play') return;
    const item = run.item;
    const st = run.st;
    const missing = item.answers.filter((a) => !st.found.includes(a.uci));
    if (missing.length) {
      run.wrong += 1;
      run.streak = 0;
      updateLive();
      playSound('wrong');
      board.setArrows(item.answers.map((a) => ({ from: a.uci.slice(0, 2), to: a.uci.slice(2, 4), color: st.found.includes(a.uci) ? 'green' : 'yellow' })));
      floatText(t('drills.checks.missed', { count: missing.length }), 'bad');
    }
    finishChecksPosition(false);
    board.setInteractive(false);
    run.busy = true;
    later(() => { if (phase === 'play') nextQuestion(); }, missing.length ? 1200 : 200);
  }

  // ------------------------------------------------------------------ knight
  function startKnight(item) {
    run.st = { cur: item.start, hops: 0, fen: item.fen, route: [] };
    board.setOrientation('white');
    board.setPosition(item.fen, { animate: false });
    paintKnight();
    setPrompt(h('span', null, t('drills.knight.prompt', { count: item.min_moves })), h('strong', { class: 'drl-target' }, item.target));
    announce(t('drills.knight.spoken', { from: item.start, to: item.target, count: item.min_moves }));
    answers.replaceChildren(...kids(
      h('span', { class: 'drl-hops' }, t('drills.knight.hops', { n: 0 })),
      run.variant === 'advanced' ? h('span', { class: 'subtle text-sm' }, t('drills.knight.legend')) : null));
  }

  function paintKnight() {
    const item = run.item;
    board.setHighlights([
      ...item.blocked.map((s) => ({ square: s, kind: 'bad' })),
      { square: item.target, kind: 'target' },
    ]);
    board.setCircles([{ square: item.target, color: 'green' }]);
    const hops = [item.start, ...run.st.route];
    board.setArrows(hops.slice(1).map((to, i) => ({ from: hops[i], to, color: 'blue' })));
  }

  function tapKnight(sq) {
    const item = run.item;
    const st = run.st;
    if (sq === st.cur) return;
    if (!isKnightHop(st.cur, sq) || item.blocked.includes(sq)) {
      markWrong();
      board.setHighlights([...item.blocked.map((s) => ({ square: s, kind: 'bad' })), { square: item.target, kind: 'target' }, { square: sq, kind: 'bad' }]);
      later(() => { if (phase === 'play' && run && run.item === item) paintKnight(); }, 350);
      if (item.blocked.includes(sq)) floatText(t('drills.knight.blocked'), 'bad');
      return;
    }
    st.fen = moveKnight(st.fen, st.cur, sq);
    st.cur = sq;
    st.hops += 1;
    st.route.push(sq);
    board.setPosition(st.fen, { animate: true, lastMove: null, sound: false });
    playSound('move');
    paintKnight();
    const hopsEl = answers.querySelector('.drl-hops');
    if (hopsEl) hopsEl.textContent = t('drills.knight.hops', { n: st.hops });
    if (sq === item.target) {
      const perfect = st.hops === item.min_moves;
      markCorrect(perfect ? 2 : 1);
      floatText(perfect ? t('drills.knight.perfect') : t('drills.knight.reached'));
      if (!perfect) addMistake({ kind: 'knight', fen: item.fen, start: item.start, target: item.target, path: item.path, blocked: item.blocked, hops: st.hops, min: item.min_moves });
      run.busy = true;
      later(() => { if (phase === 'play') nextQuestion(); }, 500);
      return;
    }
    if (st.hops >= item.min_moves + 3) {
      markWrong();
      addMistake({ kind: 'knight', failed: true, fen: item.fen, start: item.start, target: item.target, path: item.path, blocked: item.blocked, hops: st.hops, min: item.min_moves });
      board.setPosition(item.fen, { animate: false });
      board.setArrows([item.start, ...item.path].slice(1).map((to, i) => ({ from: [item.start, ...item.path][i], to, color: 'yellow' })));
      floatText(t('drills.knight.tooLong'), 'bad');
      run.busy = true;
      later(() => { if (phase === 'play') nextQuestion(); }, 1300);
    }
  }

  // ------------------------------------------------------------------ boot
  createBoard();
  renderIntro();
  loadStats();
  preparePool().catch(() => {});
}
