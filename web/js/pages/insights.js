// Insights page (#/insights): personal weakness tracker built from the user's reviewed games.
// Data: GET /api/insights (see docs/CONTRACT.md "Insights"). Unreviewed games can be reviewed
// in a bounded, cancellable batch through POST /api/insights/review-next (one game per request).
// Charts are plain SVG/CSS using design tokens; no libraries.

import { api, isAbort } from '../api.js';
import { h, icon, pageHeader, disposables, emptyState, skeleton, toast, escapeHtml } from '../ui.js';
import { t, hasKey, formatNumber, formatDateIntl } from '../i18n.js';
import { pieceUrl } from '../settings.js';
import { weeklyInsightsCard } from '../components/weekly-card.js';

export const title = () => t('insights.title');

/** Most games reviewed per click of "Review my unreviewed games". */
export const MAX_BATCH = 10;
/** A background review can take a while on slow machines (deep games). */
const REVIEW_TIMEOUT_MS = 180000;

const span = (key, params) => `<span>${escapeHtml(t(key, params))}</span>`;
const num = (v) => (Number.isFinite(Number(v)) ? Number(v) : 0);
const pct1 = (v) => `${formatNumber(Math.round(num(v) * 10) / 10)}%`;

/** Inject /css/insights.css once (also linked from index.html; idempotent). */
export function ensureInsightsCss() {
  if (document.querySelector('link[data-page-css="insights"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/css/insights.css';
  link.dataset.pageCss = 'insights';
  document.head.appendChild(link);
}

/** Label for a puzzle theme id (falls back to the raw id). */
export function themeLabel(id) {
  const s = String(id || '');
  return s && hasKey(`themes.${s}`) ? t(`themes.${s}`) : s;
}

/** Game Review link for one moment of a game. */
export function momentHref(gameId, ply) {
  return `#/review/${encodeURIComponent(gameId)}?ply=${encodeURIComponent(ply)}`;
}

/** Move label like "12." / "12…" for a FEN's side to move. */
function moveNumber(fen) {
  const parts = String(fen || '').split(' ');
  const n = Number(parts[5]) || 1;
  return parts[1] === 'b' ? `${n}…` : `${n}.`;
}

/** SVG element for tags `h()` doesn't know as SVG (a, title). */
function svgNode(tag, attrs, text) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs || {})) if (v != null) el.setAttribute(k, String(v));
  if (text != null) el.textContent = String(text);
  return el;
}

// ---------------------------------------------------------------------------
// Mini board (static SVG) with the played move highlighted and the better move as an arrow.
// ---------------------------------------------------------------------------
const FILES = 'abcdefgh';
function sqXY(sq, flip) {
  const f = FILES.indexOf(sq[0]); const r = Number(sq[1]);
  if (f < 0 || !(r >= 1 && r <= 8)) return null;
  const x = flip ? 7 - f : f; const y = flip ? r - 1 : 8 - r;
  return { x, y };
}

export function miniBoard(fen, { orientation = 'white', played = '', best = '' } = {}) {
  const NS = 'http://www.w3.org/2000/svg';
  const flip = orientation === 'black';
  const rows = String(fen || '').split(' ')[0].split('/');
  let out = '<rect class="l" width="8" height="8"/>';
  for (let r = 0; r < 8; r++) for (let f = 0; f < 8; f++) {
    if ((r + f) % 2 === 1) out += `<rect class="d" x="${f}" y="${r}" width="1" height="1"/>`;
  }
  const mark = (uci, cls) => {
    for (const sq of [uci.slice(0, 2), uci.slice(2, 4)]) {
      const p = sqXY(sq, flip);
      if (p) out += `<rect class="${cls}" x="${p.x}" y="${p.y}" width="1" height="1"/>`;
    }
  };
  if (/^[a-h][1-8][a-h][1-8]/.test(played)) mark(played, 'mv-played');
  for (let r = 0; r < 8 && r < rows.length; r++) {
    let f = 0;
    for (const ch of rows[r]) {
      if (f > 7) break;
      if (/[1-8]/.test(ch)) { f += Number(ch); continue; }
      if (!/[prnbqkPRNBQK]/.test(ch)) { f++; continue; }
      const code = (ch === ch.toUpperCase() ? 'w' : 'b') + ch.toUpperCase();
      const x = flip ? 7 - f : f; const y = flip ? 7 - r : r;
      out += `<image href="${escapeHtml(pieceUrl(code))}" x="${x}" y="${y}" width="1" height="1"/>`;
      f++;
    }
  }
  if (/^[a-h][1-8][a-h][1-8]/.test(best) && best !== played) {
    const a = sqXY(best.slice(0, 2), flip); const b = sqXY(best.slice(2, 4), flip);
    if (a && b) {
      const ax = a.x + 0.5; const ay = a.y + 0.5; const bx = b.x + 0.5; const by = b.y + 0.5;
      const len = Math.hypot(bx - ax, by - ay) || 1;
      const ux = (bx - ax) / len; const uy = (by - ay) / len;
      const ex = bx - ux * 0.32; const ey = by - uy * 0.32;
      const hx = bx - ux * 0.05; const hy = by - uy * 0.05;
      const px = -uy * 0.24; const py = ux * 0.24;
      out += `<g class="mv-best"><line x1="${ax}" y1="${ay}" x2="${ex}" y2="${ey}"/>`
        + `<polygon points="${hx},${hy} ${ex + px},${ey + py} ${ex - px},${ey - py}"/></g>`;
    }
  }
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', '0 0 8 8');
  svg.setAttribute('class', 'ins-mini-board');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('shape-rendering', 'crispEdges');
  svg.innerHTML = out;
  return svg;
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------
export async function mount(root) {
  ensureInsightsCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  // The running review batch (one at a time); aborted on unmount.
  let batch = null;
  bag.add(() => batch?.ctrl.abort());
  let data = null;
  // "Your weekly set" card (own fetch); rebuilt on every render.
  let weekly = null;
  bag.add(() => weekly?.destroy());
  const weeklyCard = () => {
    weekly?.destroy();
    weekly = weeklyInsightsCard();
    return weekly.el;
  };

  const reviewBtn = h('button', { class: 'btn btn-secondary', type: 'button', hidden: true, html: icon('sparkles') + span('insights.review.start') });
  bag.on(reviewBtn, 'click', () => runBatch());
  const banner = h('div', { class: 'ins-banner-slot' });
  const content = h('div', { class: 'stack-lg' }, skeleton('card', 3), skeleton('text', 6));
  const page = h('div', { class: 'page ins-page' },
    pageHeader({ title: t('insights.title'), subtitle: t('insights.subtitle'), icon: 'chart', actions: [reviewBtn] }),
    banner,
    content);
  root.appendChild(page);

  load();
  return bag.dispose;

  async function load({ quiet = false } = {}) {
    if (!quiet) content.replaceChildren(skeleton('card', 3), skeleton('text', 6));
    try {
      data = await api.get('/api/insights', { signal });
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      content.replaceChildren(h('div', { class: 'card' }, emptyState({
        icon: 'wifi-off', title: t('insights.errors.loadTitle'), text: e.message || t('insights.errors.server'),
        action: { label: t('insights.errors.retry'), icon: 'refresh', onClick: () => load() },
      })));
      return;
    }
    if (bag.disposed) return;
    render();
  }

  // ---- review batch -------------------------------------------------------
  function renderBanner() {
    const reviewable = num(data?.reviewable);
    reviewBtn.hidden = !!batch || reviewable === 0 || !data?.ready;
    if (batch) {
      const pctDone = Math.round((batch.done / Math.max(1, batch.total)) * 100);
      const stop = h('button', { class: 'btn btn-ghost btn-sm', type: 'button', html: icon('x') + span('insights.review.cancel') });
      stop.addEventListener('click', () => batch?.ctrl.abort(), { once: true });
      banner.replaceChildren(h('div', { class: 'card ins-banner', role: 'status', 'aria-live': 'polite' },
        h('div', { class: 'spinner' }),
        h('div', { class: 'ins-banner-main' },
          h('div', { class: 'semibold' }, t('insights.review.progress', { done: Math.min(batch.done + 1, batch.total), total: batch.total })),
          h('div', { class: 'progress progress-sm mt-2' }, h('div', { class: 'progress-bar', style: { width: `${Math.max(4, pctDone)}%` } }))),
        stop));
      return;
    }
    if (reviewable > 0 && data?.ready) {
      const go = h('button', { class: 'btn btn-primary btn-sm', type: 'button', html: icon('sparkles') + span('insights.review.start') });
      go.addEventListener('click', () => runBatch(), { once: true });
      banner.replaceChildren(h('div', { class: 'card ins-banner' },
        h('div', { class: 'ins-banner-icon', html: icon('info') }),
        h('div', { class: 'ins-banner-main' },
          h('div', { class: 'semibold' }, t('insights.review.title', { count: reviewable })),
          h('div', { class: 'muted text-sm' }, t('insights.review.text', { max: MAX_BATCH }))),
        go));
      reviewBtn.hidden = true;
      return;
    }
    banner.replaceChildren();
  }

  async function runBatch() {
    if (batch || bag.disposed) return;
    const total = Math.min(MAX_BATCH, num(data?.reviewable));
    if (!total) return;
    batch = { ctrl: new AbortController(), done: 0, total };
    const my = batch;
    renderBanner();
    if (!data?.ready) render();
    let stopped = false;
    try {
      while (my.done < my.total) {
        const r = await api.post('/api/insights/review-next', { skip: [] }, { signal: my.ctrl.signal, timeout: REVIEW_TIMEOUT_MS });
        if (!r || r.game_id == null) break;
        my.done++;
        if (bag.disposed) return;
        renderBanner();
      }
    } catch (e) {
      if (isAbort(e)) stopped = true;
      else if (e.status === 409) toast(t('insights.review.busy'), 'warning');
      else toast(t('insights.review.failed', { message: e.message }), 'error');
    } finally {
      if (batch === my) batch = null;
    }
    if (bag.disposed) return;
    if (stopped) toast(t('insights.review.stopped'), 'info');
    else if (my.done) toast(t('insights.review.finished', { count: my.done }), 'success');
    await load({ quiet: true });
  }

  // ---- render ---------------------------------------------------------------
  function render() {
    renderBanner();
    if (!data.ready) {
      content.replaceChildren(renderEmpty(), weeklyCard());
      return;
    }
    const sections = [
      renderSummary(),
      renderWeaknesses(),
      h('div', { class: 'ins-grid' },
        renderPhases(),
        renderTactics(),
        weeklyCard(),
        renderHanging(),
        renderTrend(),
        renderColors(),
        renderOpenings(),
        renderConversion(),
        data.time_trouble ? renderTime(data.time_trouble) : null),
    ];
    content.replaceChildren(...sections.filter(Boolean));
  }

  function renderEmpty() {
    const min = num(data.min_games) || 3;
    const have = num(data.games_analyzed);
    const reviewable = num(data.reviewable);
    const actions = h('div', { class: 'row row-wrap ins-empty-actions' },
      h('a', { class: 'btn btn-primary btn-lg', href: '#/play', html: icon('play') + span('insights.empty.play') }));
    if (batch) {
      // Progress shows in the banner above.
    } else if (reviewable > 0) {
      const go = h('button', { class: 'btn btn-secondary btn-lg', type: 'button', html: icon('sparkles') + span('insights.review.start') });
      go.addEventListener('click', () => runBatch(), { once: true });
      actions.append(go);
    } else if (num(data.total_games) > 0) {
      actions.append(h('a', { class: 'btn btn-ghost btn-lg', href: '#/library', html: icon('library') + span('insights.empty.library') }));
    }
    const steps = Array.from({ length: min }, (_, i) => h('span', { class: ['ins-step', i < have && 'done'], 'aria-hidden': 'true' }, i < have ? '✓' : String(i + 1)));
    return h('section', { class: 'card ins-empty' },
      h('div', { class: 'ins-empty-art', 'aria-hidden': 'true' }, '🔍'),
      h('h2', { class: 'ins-empty-title' }, t('insights.empty.title')),
      h('p', { class: 'muted ins-empty-text' }, t('insights.empty.text', { min })),
      h('div', { class: 'ins-steps' }, steps),
      h('div', { class: 'subtle text-sm' }, t('insights.empty.progress', { count: Math.min(have, min), min })),
      reviewable > 0 && !batch ? h('p', { class: 'text-sm muted' }, t('insights.review.title', { count: reviewable })) : null,
      actions);
  }

  function renderSummary() {
    const phases = Array.isArray(data.phases) ? data.phases : [];
    const costly = phases.reduce((s, p) => s + num(p.mistakes) + num(p.blunders), 0);
    const games = num(data.games_analyzed);
    const conv = data.conversion || {};
    const tile = (label, value, ic) => h('div', { class: 'stat ins-stat' },
      h('div', { class: 'ins-stat-icon', html: icon(ic) }),
      h('div', null, h('div', { class: 'stat-label' }, label), h('div', { class: 'stat-value tabular' }, value)));
    return h('div', { class: 'ins-stats' },
      tile(t('insights.summary.games'), formatNumber(games), 'board'),
      tile(t('insights.summary.accuracy'), data.average_accuracy != null ? pct1(data.average_accuracy) : '—', 'target'),
      tile(t('insights.summary.errorsPerGame'), games ? formatNumber(Math.round((costly / games) * 10) / 10) : '—', 'alert'),
      tile(t('insights.summary.conversion'), num(conv.winning_games) ? `${num(conv.converted)}/${num(conv.winning_games)}` : '—', 'trophy'));
  }

  // ---- weaknesses -----------------------------------------------------------
  function renderWeaknesses() {
    const list = Array.isArray(data.weaknesses) ? data.weaknesses : [];
    const head = h('div', { class: 'ins-section-head' },
      h('h2', { class: 'section-title' }, t('insights.weak.title')),
      h('p', { class: 'muted' }, t('insights.weak.subtitle')));
    if (!list.length) {
      return h('section', { class: 'stack' }, head,
        h('div', { class: 'card' }, emptyState({ emoji: '🌟', title: t('insights.weak.noneTitle'), text: t('insights.weak.noneText') })));
    }
    return h('section', { class: 'stack' }, head,
      h('div', { class: 'ins-weak-grid' }, list.map((w, i) => weaknessCard(w, i))));
  }

  function weaknessCard(w, i) {
    const examples = Array.isArray(w.examples) ? w.examples : [];
    const gamesTxt = t('insights.weak.games', { count: num(w.games) });
    return h('article', { class: 'card ins-weak', dataset: { kind: w.id, rank: i + 1 } },
      h('div', { class: 'ins-weak-top' },
        h('span', { class: 'ins-rank' }, t('insights.weak.rank', { rank: i + 1 })),
        h('span', { class: 'badge ins-evidence' }, t('insights.weak.evidence', { count: num(w.count), games: gamesTxt }))),
      h('h3', { class: 'ins-weak-title' }, w.title || ''),
      h('p', { class: 'ins-weak-text' }, w.explanation || ''),
      examples.length ? h('div', { class: 'ins-examples-label subtle text-xs' }, t('insights.weak.examples')) : null,
      examples.length ? h('div', { class: 'ins-examples' }, examples.map(exampleTile)) : null,
      h('div', { class: 'ins-weak-foot' },
        h('a', { class: 'btn btn-primary btn-block', href: w.drill?.href || '#/puzzles', html: icon('target') + `<span>${escapeHtml(w.drill?.label || t('insights.hanging.drill'))}</span>` })));
  }

  function exampleTile(ex) {
    const mv = `${moveNumber(ex.fen)} ${ex.move_san || ''}`.trim();
    const a = h('a', { class: 'ins-example', href: momentHref(ex.game_id, ex.ply), dataset: { cls: ex.classification }, 'aria-label': t('insights.weak.exampleAria', { move: mv }) },
      miniBoard(ex.fen, { orientation: ex.color, played: ex.move_uci, best: ex.best_uci }),
      h('span', { class: 'ins-example-move' }, h('span', { class: 'ins-dot', 'aria-hidden': 'true' }), mv),
      ex.best_san && ex.best_san !== ex.move_san ? h('span', { class: 'ins-example-best subtle' }, t('insights.weak.best', { san: ex.best_san })) : null);
    return a;
  }

  // ---- breakdown cards --------------------------------------------------------
  function card(titleKey, iconName, subtitle, ...body) {
    return h('section', { class: 'card ins-card' },
      h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon(iconName) + span(titleKey) })),
      subtitle ? h('p', { class: 'muted text-sm ins-card-sub' }, subtitle) : null,
      ...body);
  }

  function renderPhases() {
    const phases = Array.isArray(data.phases) ? data.phases : [];
    const kinds = [['inaccuracies', 'inaccuracy'], ['mistakes', 'mistake'], ['blunders', 'blunder']];
    const rate = (p) => (num(p.moves) ? ((num(p.mistakes) + num(p.blunders) + num(p.inaccuracies)) / num(p.moves)) * 100 : 0);
    const maxRate = Math.max(1, ...phases.map(rate));
    const rows = phases.map((p) => {
      const total = num(p.inaccuracies) + num(p.mistakes) + num(p.blunders);
      const width = (rate(p) / maxRate) * 100;
      const bar = h('div', { class: 'ins-stack', style: { width: `${Math.max(total ? 3 : 0, width)}%` } },
        kinds.map(([k, cls]) => (num(p[k]) ? h('span', { class: 'ins-seg', dataset: { cls }, style: { flexGrow: num(p[k]) }, title: `${t(`insights.phase.${cls}`)}: ${num(p[k])}` }) : null)));
      return h('div', { class: 'ins-phase-row' },
        h('div', { class: 'ins-phase-head' },
          h('span', { class: 'semibold' }, t(`insights.phase.${p.phase}`)),
          h('span', { class: 'subtle text-xs tabular' }, num(p.moves)
            ? `${t('insights.phase.moves', { count: num(p.moves) })} · ${t('insights.phase.rate', { rate: formatNumber(Math.round(rate(p) * 10) / 10) })}`
            : t('insights.phase.none'))),
        h('div', { class: 'ins-track' }, bar),
        h('div', { class: 'ins-phase-counts text-xs tabular' }, kinds.map(([k, cls]) =>
          h('span', { class: 'ins-count', dataset: { cls } }, h('span', { class: 'ins-dot', 'aria-hidden': 'true' }), `${num(p[k])} ${t(`insights.phase.${cls}`)}`))));
    });
    return card('insights.phase.title', 'chart', t('insights.phase.subtitle'), h('div', { class: 'stack' }, rows));
  }

  function renderHanging() {
    const list = Array.isArray(data.hanging) ? data.hanging : [];
    const total = list.reduce((s, p) => s + num(p.count), 0);
    if (!total) return card('insights.hanging.title', 'shield', t('insights.hanging.subtitle'), h('p', { class: 'ins-none' }, t('insights.hanging.none')));
    const max = Math.max(1, ...list.map((p) => num(p.count)));
    const codes = { pawn: 'P', knight: 'N', bishop: 'B', rook: 'R', queen: 'Q' };
    return card('insights.hanging.title', 'shield', t('insights.hanging.subtitle'),
      h('div', { class: 'ins-columns', role: 'list' }, list.map((p) => {
        const c = num(p.count);
        const name = t(`insights.hanging.pieces.${p.piece}`);
        return h('div', { class: ['ins-col', c === max && 'top'], role: 'listitem', 'aria-label': `${name}: ${c}` },
          h('span', { class: 'ins-col-value tabular' }, formatNumber(c)),
          h('div', { class: 'ins-col-track' }, h('div', { class: 'ins-col-bar', style: { height: `${c ? Math.max(6, (c / max) * 100) : 0}%` } })),
          h('img', { class: 'ins-col-piece', src: pieceUrl(`w${codes[p.piece] || 'P'}`), alt: '', width: 32, height: 32 }),
          h('span', { class: 'ins-col-label text-xs' }, name));
      })),
      h('a', { class: 'btn btn-ghost btn-sm ins-card-link', href: '#/drills/hanging', html: icon('target') + span('insights.hanging.drill') }));
  }

  function renderTactics() {
    const list = (Array.isArray(data.tactics) ? data.tactics : []).slice(0, 7);
    if (!list.length) return card('insights.tactics.title', 'bolt', t('insights.tactics.subtitle'), h('p', { class: 'ins-none' }, t('insights.tactics.none')));
    const max = Math.max(1, ...list.map((x) => num(x.count)));
    return card('insights.tactics.title', 'bolt', t('insights.tactics.subtitle'),
      h('div', { class: 'ins-hbars' }, list.map((x) => {
        const label = themeLabel(x.theme);
        return h('a', { class: 'ins-hbar', href: `#/puzzles?theme=${encodeURIComponent(x.theme)}`, 'aria-label': t('insights.tactics.practiceAria', { theme: label }) },
          h('span', { class: 'ins-hbar-label' }, label),
          h('span', { class: 'ins-track' }, h('span', { class: 'ins-hbar-fill', style: { width: `${Math.max(4, (num(x.count) / max) * 100)}%` } })),
          h('span', { class: 'ins-hbar-value tabular' }, formatNumber(num(x.count))),
          h('span', { class: 'ins-hbar-go', html: icon('chevron-right', { size: 16 }) }));
      })));
  }

  function renderTrend() {
    const pts = Array.isArray(data.accuracy_trend) ? data.accuracy_trend : [];
    if (pts.length < 2) return card('insights.trend.title', 'target', null, h('p', { class: 'ins-none' }, t('insights.trend.none')));
    const W = 560; const H = 200; const P = { l: 34, r: 12, t: 12, b: 22 };
    const x = (i) => P.l + (i * (W - P.l - P.r)) / Math.max(1, pts.length - 1);
    const y = (v) => P.t + (1 - num(v) / 100) * (H - P.t - P.b);
    const avg = pts.reduce((s, p) => s + num(p.accuracy), 0) / pts.length;
    const line = pts.map((p, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)},${y(p.accuracy).toFixed(1)}`).join(' ');
    const area = `${line} L${x(pts.length - 1).toFixed(1)},${y(0)} L${x(0).toFixed(1)},${y(0)} Z`;
    const grid = [0, 25, 50, 75, 100].flatMap((v) => [
      h('line', { class: 'ins-grid-line', x1: P.l, x2: W - P.r, y1: y(v), y2: y(v) }),
      h('text', { class: 'ins-axis', x: P.l - 6, y: y(v) + 4, 'text-anchor': 'end' }, String(v)),
    ]);
    const dots = pts.map((p, i) => {
      const label = t('insights.trend.point', { date: formatDateIntl(p.date, { month: 'short', day: 'numeric' }), value: pct1(p.accuracy) });
      const link = svgNode('a', { href: `#/review/${encodeURIComponent(p.game_id)}`, 'aria-label': label });
      const dot = h('circle', { class: `ins-pt ${p.outcome}`, cx: x(i), cy: y(p.accuracy), r: 4.5 });
      dot.appendChild(svgNode('title', null, label));
      link.appendChild(dot);
      return link;
    });
    const svg = h('svg', { class: 'ins-trend', viewBox: `0 0 ${W} ${H}`, role: 'img', 'aria-label': t('insights.trend.title') },
      grid,
      h('path', { class: 'ins-trend-area', d: area }),
      h('line', { class: 'ins-trend-avg', x1: P.l, x2: W - P.r, y1: y(avg), y2: y(avg) }),
      h('path', { class: 'ins-trend-line', d: line }),
      dots);
    const legend = h('div', { class: 'row row-wrap ins-legend text-xs' },
      ['win', 'draw', 'loss'].map((o) => h('span', { class: `ins-legend-item ${o}` }, h('span', { class: 'ins-legend-dot', 'aria-hidden': 'true' }), t(`insights.trend.legend${o[0].toUpperCase()}${o.slice(1)}`))),
      h('span', { class: 'spacer' }),
      h('span', { class: 'subtle' }, t('insights.trend.average', { value: pct1(avg) })));
    return card('insights.trend.title', 'target', t('insights.trend.subtitle', { count: pts.length }), svg, legend);
  }

  function wdlBar(r) {
    const total = Math.max(1, num(r.wins) + num(r.draws) + num(r.losses));
    return h('div', { class: 'ins-wdl', role: 'img', 'aria-label': t('insights.color.record', { wins: num(r.wins), draws: num(r.draws), losses: num(r.losses) }) },
      ['wins', 'draws', 'losses'].map((k) => (num(r[k]) ? h('span', { class: `ins-wdl-seg ${k}`, style: { width: `${(num(r[k]) / total) * 100}%` } }) : null)));
  }

  function renderColors() {
    const byColor = data.by_color || {};
    const rows = ['white', 'black'].map((c) => {
      const r = byColor[c];
      return h('div', { class: 'ins-color-row' },
        h('div', { class: 'ins-color-head' },
          h('span', { class: `ins-king ${c}`, 'aria-hidden': 'true' }, c === 'white' ? '♔' : '♚'),
          h('span', { class: 'semibold' }, t(`insights.color.${c}`)),
          h('span', { class: 'spacer' }),
          r ? h('span', { class: 'subtle text-xs tabular' }, t('insights.weak.games', { count: num(r.games) })) : null),
        r ? wdlBar(r) : h('div', { class: 'subtle text-sm' }, t('insights.color.none')),
        r ? h('div', { class: 'row text-xs tabular' },
          h('span', null, t('insights.color.record', { wins: num(r.wins), draws: num(r.draws), losses: num(r.losses) })),
          h('span', { class: 'spacer' }),
          r.accuracy != null ? h('span', { class: 'subtle' }, t('insights.color.accuracy', { value: pct1(r.accuracy) })) : null) : null);
    });
    return card('insights.color.title', 'flag', null, h('div', { class: 'stack' }, rows));
  }

  function renderOpenings() {
    const list = Array.isArray(data.by_opening) ? data.by_opening : [];
    if (!list.length) return card('insights.openings.title', 'openings', null, h('p', { class: 'ins-none' }, t('insights.openings.none')));
    return card('insights.openings.title', 'openings', null,
      h('div', { class: 'stack-sm' }, list.map((o) => h('div', { class: 'ins-open-row' },
        h('div', { class: 'row' },
          h('span', { class: 'semibold truncate ins-open-name', title: o.name }, o.name),
          h('span', { class: 'spacer' }),
          h('span', { class: 'subtle text-xs tabular nowrap' }, `${t('insights.openings.games', { count: num(o.games) })}${o.accuracy != null ? ` · ${pct1(o.accuracy)}` : ''}`)),
        wdlBar(o),
        h('div', { class: 'subtle text-xs tabular' }, t('insights.color.record', { wins: num(o.wins), draws: num(o.draws), losses: num(o.losses) }))))));
  }

  function renderConversion() {
    const c = data.conversion || {};
    const total = num(c.winning_games);
    if (!total) return card('insights.conversion.title', 'trophy', t('insights.conversion.subtitle'), h('p', { class: 'ins-none' }, t('insights.conversion.none')));
    const conv = num(c.converted);
    const ratio = conv / total;
    const ring = h('div', { class: 'ins-ring', style: { '--value': Math.round(ratio * 100) }, 'aria-hidden': 'true' }, h('span', null, `${Math.round(ratio * 100)}%`));
    const failed = Array.isArray(c.failed) ? c.failed : [];
    const fmtAdv = (cp) => (num(cp) >= 10000 ? 'M' : `+${formatNumber(Math.round(num(cp) / 10) / 10)}`);
    return card('insights.conversion.title', 'trophy', t('insights.conversion.subtitle'),
      h('div', { class: 'row ins-conv-head' }, ring,
        h('div', null, h('div', { class: 'semibold text-lg' }, t('insights.conversion.rate', { converted: conv, total })))),
      failed.length ? h('div', { class: 'subtle text-xs mt-3 mb-1' }, t('insights.conversion.failed')) : null,
      failed.length ? h('div', { class: 'ins-failed' }, failed.map((f) => h('a', { class: 'ins-failed-row', href: momentHref(f.game_id, f.ply), title: t('insights.conversion.open') },
        miniBoard(f.fen, { orientation: f.color }),
        h('div', { class: 'stack-sm', style: 'min-width:0' },
          h('span', { class: `badge ${f.outcome === 'loss' ? 'badge-danger' : 'badge-warning'}` }, t(`insights.conversion.outcome.${f.outcome === 'loss' ? 'loss' : 'draw'}`)),
          h('span', { class: 'text-xs subtle tabular' }, t('insights.conversion.best', { value: fmtAdv(f.best_cp) }))),
        h('span', { class: 'ins-hbar-go', html: icon('chevron-right', { size: 16 }) })))) : null);
  }

  function renderTime(tt) {
    const lowRate = num(tt.low_time_moves) ? (num(tt.low_time_errors) / num(tt.low_time_moves)) * 100 : 0;
    const normRate = num(tt.normal_moves) ? (num(tt.normal_errors) / num(tt.normal_moves)) * 100 : 0;
    const max = Math.max(1, lowRate, normRate);
    const row = (labelKey, rate, cls) => h('div', { class: 'ins-phase-row' },
      h('div', { class: 'ins-phase-head' }, h('span', { class: 'semibold' }, t(labelKey)),
        h('span', { class: 'subtle text-xs tabular' }, t('insights.time.rate', { rate: formatNumber(Math.round(rate)) }))),
      h('div', { class: 'ins-track' }, h('span', { class: `ins-hbar-fill ${cls}`, style: { width: `${Math.max(3, (rate / max) * 100)}%` } })));
    return card('insights.time.title', 'clock', t('insights.time.subtitle', { secs: num(tt.low_time_secs) }),
      h('div', { class: 'stack' }, row('insights.time.low', lowRate, 'danger'), row('insights.time.normal', normRate, '')),
      h('p', { class: 'subtle text-xs mt-2' }, t('insights.time.games', { count: num(tt.games_with_clock) })));
  }
}

// ---------------------------------------------------------------------------
// Compact card for the Profile page. Returns { el, destroy }.
// ---------------------------------------------------------------------------
export function topWeaknessesCard() {
  ensureInsightsCss();
  const ctrl = new AbortController();
  const body = h('div', { class: 'stack-sm' }, h('div', { class: 'skeleton skeleton-text' }), h('div', { class: 'skeleton skeleton-text', style: 'width:70%' }));
  const el = h('section', { class: 'card ins-profile-card' },
    h('div', { class: 'card-header' },
      h('div', { class: 'card-title', html: icon('chart') + span('insights.profile.title') }),
      h('a', { class: 'btn btn-ghost btn-sm', href: '#/insights', html: span('insights.profile.all') + icon('chevron-right') })),
    body);
  api.get('/api/insights', { signal: ctrl.signal }).then((d) => {
    const list = d && d.ready && Array.isArray(d.weaknesses) ? d.weaknesses.slice(0, 3) : [];
    if (!d?.ready) {
      const min = num(d?.min_games) || 3;
      body.replaceChildren(h('div', { class: 'row ins-profile-empty' },
        h('span', { class: 'ins-profile-emoji', 'aria-hidden': 'true' }, '🔍'),
        h('div', { class: 'stack-sm', style: 'min-width:0' },
          h('div', { class: 'semibold' }, t('insights.empty.title')),
          h('div', { class: 'muted text-sm' }, t('insights.empty.progress', { count: Math.min(num(d?.games_analyzed), min), min })))));
      return;
    }
    if (!list.length) {
      body.replaceChildren(h('p', { class: 'muted text-sm' }, t('insights.weak.noneText')));
      return;
    }
    body.replaceChildren(h('div', { class: 'ins-profile-list' }, list.map((w, i) =>
      h('a', { class: 'ins-profile-row', href: w.drill?.href || '#/insights' },
        h('span', { class: 'ins-rank sm' }, String(i + 1)),
        h('div', { class: 'ins-profile-main' },
          h('div', { class: 'semibold truncate' }, w.title || ''),
          h('div', { class: 'subtle text-xs' }, t('insights.weak.evidence', { count: num(w.count), games: t('insights.weak.games', { count: num(w.games) }) }))),
        h('span', { class: 'ins-profile-drill text-sm', html: escapeHtml(w.drill?.label || '') + icon('chevron-right', { size: 16 }) })))));
  }).catch((e) => {
    if (isAbort(e)) return;
    el.remove();
  });
  return { el, destroy() { ctrl.abort(); } };
}
