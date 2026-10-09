// Game Review (#/review/:gameId) — chess.com-style coach review.
// Flow: load game → POST /api/review {game_id} (animated coach loading state) → report
// (summary, accuracy, estimated Elo, classification table, eval graph, key moments) →
// "Start Review" walk-through (badges, best-move arrows, Show line, Retry, synced eval bar).
// Explore: move any piece on the board to try your own line from that point; the engine
// and coach evaluate it live, and ← / "Back to game" return to the real game.
// Every listener, timer, request and component is released by the returned cleanup.

import { Chess } from '../../vendor/chess.js';
import { api, isAbort, EngineClient } from '../api.js';
import {
  h, icon, formatScore, disposables, classificationMeta, classificationBadge, mdLite, emptyState, formatSan,
} from '../ui.js';
import { userAvatar } from '../ui.js';
import { getSettings, onSettingsChange } from '../settings.js';
import { Board } from '../components/board.js';
import { EvalBar } from '../components/evalbar.js';
import { MoveList } from '../components/movelist.js';
import { EvalGraph, ensureAnalysisCss } from '../components/evalgraph.js';
import { MentorPanel } from '../components/mentor.js';
import { playSound } from '../components/sound.js';
import { WhyPanel, hasReason } from '../components/whyline.js';
import { START_FEN, uciSquares, fenPly, numberedLine } from './analysis.js';
import { t, formatNumber } from '../i18n.js';

export function title() { return t('review.title'); }

const TABLE_ORDER = ['brilliant', 'great', 'best', 'excellent', 'good', 'book', 'inaccuracy', 'mistake', 'miss', 'blunder'];
const RETRY_CLASSES = new Set(['inaccuracy', 'mistake', 'miss', 'blunder']);
const GOOD_RETRY = new Set(['brilliant', 'great', 'best', 'excellent']);
const SHOW_BEST = new Set(['inaccuracy', 'mistake', 'miss', 'blunder', 'good', 'excellent']);
const PHRASE_CLASSES = new Set(['brilliant', 'great', 'best', 'excellent', 'good', 'book', 'inaccuracy', 'mistake', 'miss', 'blunder', 'forced']);

/** Whole-sentence verdict for a classification, e.g. "Nf3 is a blunder"; `fallback` when unknown. */
function phrase(cls, move, fallback = null) {
  if (PHRASE_CLASSES.has(cls)) return t(`review.phrase.${cls}`, { move });
  return fallback ? t(`review.phrase.${fallback}`, { move }) : '';
}

/** Same sentence as DOM nodes, with the move rendered by `moveNode` where {move} sits. */
function phraseNodes(cls, moveNode) {
  if (!PHRASE_CLASSES.has(cls)) return [moveNode];
  return withNode(t(`review.phrase.${cls}`), 'move', moveNode);
}

/** Split a message on {name} and put `node` in its place (keeps translated word order). */
function withNode(msg, name, node) {
  const parts = String(msg).split(`{${name}}`);
  const out = [];
  parts.forEach((p, i) => { if (i) out.push(node.cloneNode ? (i === 1 ? node : node.cloneNode(true)) : node); if (p) out.push(p); });
  return out;
}

// Resolved at render time so a language switch is picked up (never at import).
const loadingSteps = () => t('review.loading.steps');
const loadingTips = () => t('review.loading.tips');

function coachAvatar(size = '') {
  return h('div', { class: `avatar ${size} rv-coach-avatar` }, '🎓');
}

// ---------------------------------------------------------------------------
export async function mount(root, { params = {}, query = {} } = {}) {
  ensureAnalysisCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const page = h('div', { class: 'review-page' });
  root.appendChild(page);
  bag.add(() => page.remove());

  const gameId = String(params.gameId || '').trim();
  if (!/^\d{1,18}$/.test(gameId)) {
    page.appendChild(errorCard(t('review.error.badLinkTitle'), t('review.error.badLinkText')));
    return bag.dispose;
  }

  // 1) Game record ---------------------------------------------------------
  let game;
  page.appendChild(h('div', { class: 'page' }, h('div', { class: 'loading-center' }, h('div', { class: 'spinner spinner-lg' }), t('review.loading.game'))));
  try {
    game = await api.get(`/api/games/${gameId}`, { signal: ctrl.signal });
  } catch (e) {
    if (isAbort(e) || bag.disposed) return bag.dispose;
    page.replaceChildren(errorCard(t(e.status === 404 ? 'review.error.notFoundTitle' : 'review.error.loadTitle'), e.status === 404 ? t('review.error.notFoundText') : e.message));
    return bag.dispose;
  }
  if (bag.disposed) return bag.dispose;
  if (!Array.isArray(game.moves) || !game.moves.length) {
    page.replaceChildren(h('div', { class: 'page' }, emptyState({
      emoji: '♟', title: t('review.error.emptyTitle'), text: t('review.error.emptyText'),
      action: { label: t('review.error.playBot'), href: '#/play', icon: 'play' },
    })));
    return bag.dispose;
  }

  // The player's own avatar (same as on Play and Profile) — best effort, non-blocking.
  let myAvatar = userAvatar(null);
  api.get('/api/profile', { signal: ctrl.signal }).then((p) => {
    myAvatar = userAvatar(p);
    page.querySelectorAll('[data-user-avatar]').forEach((el) => { el.textContent = myAvatar; });
  }).catch(() => {});

  // Bot avatar (best effort, non-blocking)
  let botAvatar = '🤖';
  if (game.bot_id) {
    api.get('/api/bots', { signal: ctrl.signal }).then((bots) => {
      const b = Array.isArray(bots) ? bots.find((x) => x.id === game.bot_id) : null;
      if (b?.avatar) { botAvatar = b.avatar; page.querySelectorAll('[data-bot-avatar]').forEach((el) => { el.textContent = b.avatar; }); }
    }).catch(() => {});
  }

  // 2) Review ----------------------------------------------------------------
  let review = null;
  if (game.review_json) {
    try { review = normalizeReview(JSON.parse(game.review_json)); } catch { review = null; }
  }
  if (!review) review = await runReview();
  if (!review || bag.disposed) return bag.dispose;

  renderReview(review);
  return bag.dispose;

  // -------------------------------------------------------------------------
  async function runReview() {
    const plies = game.moves.length;
    const estMs = 2500 + plies * 260;
    const bar = h('div', { class: 'progress-bar rv-progress-bar', style: 'width:2%' });
    const pct = h('div', { class: 'rv-load-pct tabular' }, '0%');
    const STEPS = loadingSteps();
    const TIPS = loadingTips();
    const stepText = h('div', { class: 'rv-load-step' }, STEPS[0]);
    let tipIdx = Math.floor(Math.random() * TIPS.length);
    const tip = h('div', { class: 'rv-load-tip' }, h('span', { html: icon('hint') }), h('span', null, TIPS[tipIdx]));
    const cancelBtn = h('a', { class: 'btn btn-ghost btn-sm', href: '#/library' }, t('review.loading.cancel'));
    const minis = h('div', { class: 'rv-load-dots' }, game.moves.slice(0, 60).map((_, i) => h('span', { style: { '--i': i } })));
    const view = h('div', { class: 'page rv-loading' },
      h('div', { class: 'card rv-load-card pop-in' },
        h('div', { class: 'rv-load-coach' }, h('div', { class: 'rv-load-ring' }), coachAvatar('avatar-xl')),
        h('h2', { class: 'rv-load-title' }, t('review.loading.title')),
        h('div', { class: 'muted' }, t('review.loading.subtitle', { white: game.white, black: game.black, moves: t('review.loading.moves', { count: Math.ceil(plies / 2) }) })),
        minis,
        h('div', { class: 'rv-load-progress' }, h('div', { class: 'progress progress-lg' }, bar), pct),
        stepText, tip, cancelBtn));
    page.replaceChildren(view);

    const t0 = performance.now();
    const iv = setInterval(() => {
      const elapsed = (performance.now() - t0) / estMs;
      const p = Math.min(0.96, 1 - Math.exp(-2.2 * elapsed));
      bar.style.width = `${(p * 100).toFixed(1)}%`;
      pct.textContent = `${Math.round(p * 100)}%`;
      stepText.textContent = STEPS[Math.min(STEPS.length - 1, Math.floor(p * STEPS.length))];
      const lit = Math.floor(p * minis.childElementCount);
      for (let i = 0; i < minis.childElementCount; i++) minis.children[i].classList.toggle('lit', i < lit);
    }, 180);
    const tipIv = setInterval(() => { tipIdx = (tipIdx + 1) % TIPS.length; tip.lastChild.textContent = TIPS[tipIdx]; }, 5000);
    const stop = () => { clearInterval(iv); clearInterval(tipIv); };
    bag.add(stop);

    try {
      const res = await api.post('/api/review', { game_id: Number(gameId) }, { signal: ctrl.signal, timeout: 600000 });
      stop();
      if (bag.disposed) return null;
      bar.style.width = '100%';
      pct.textContent = '100%';
      stepText.textContent = t('review.loading.done');
      for (const d of minis.children) d.classList.add('lit');
      const r = normalizeReview(res);
      if (!r) throw new Error(t('review.error.reviewEmpty'));
      await new Promise((resolve) => bag.timeout(resolve, 350));
      return bag.disposed ? null : r;
    } catch (e) {
      stop();
      if (isAbort(e) || bag.disposed) return null;
      const retry = h('button', { class: 'btn btn-primary', type: 'button', html: icon('refresh') + spanHtml(t('review.error.tryAgain')) });
      page.replaceChildren(h('div', { class: 'page' }, h('div', { class: 'card placeholder-card error-card' },
        h('div', { class: 'empty-state-icon', html: icon('alert') }),
        h('h2', { class: 'mb-2' }, t('review.error.failedTitle')),
        h('p', { class: 'muted' }, e.message || t('review.error.generic')),
        h('div', { class: 'row', style: 'justify-content:center;margin-top:var(--sp-5)' }, retry,
          h('a', { class: 'btn btn-ghost', href: `#/analysis?game=${gameId}`, html: icon('analysis') + spanHtml(t('review.error.openAnalysis')) })))));
      const onRetry = async () => {
        retry.removeEventListener('click', onRetry);
        const r = await runReview();
        if (r && !bag.disposed) renderReview(r);
      };
      retry.addEventListener('click', onRetry);
      return null;
    }
  }

  // -------------------------------------------------------------------------
  function renderReview(rv) {
    const settings = getSettings();
    const moves = rv.moves;
    const n = moves.length;
    const startFen = rv.start_fen || game.start_fen || START_FEN;
    const ply0 = fenPly(startFen);
    const orientation = game.user_color === 'black' ? 'black' : 'white';
    const keySet = new Set(rv.key_moments);
    const moveLabel = (p, san) => {
      const abs = ply0 + p - 1;
      return `${Math.floor(abs / 2) + 1}${abs % 2 === 0 ? '.' : '…'} ${san}`;
    };
    const st = {
      ply: 0, mode: 'report', tab: 'review', orientation,
      lineTimer: 0, lineActive: false, retry: null,
      explore: null, // { basePly, line: [{uci, san, from, to, fen, before, cls, expl, best}], idx, info }
    };

    // ---- player bars ----
    const userIsWhite = game.user_color === 'white';
    const userIsBlack = game.user_color === 'black';
    const avatarFor = (color) => {
      const isUser = color === 'white' ? userIsWhite : userIsBlack;
      if (isUser) return h('div', { class: 'avatar', dataset: { userAvatar: '1' }, 'aria-hidden': 'true' }, myAvatar);
      if (game.bot_id) return h('div', { class: 'avatar', dataset: { botAvatar: '1' } }, botAvatar);
      return h('div', { class: 'avatar' }, color === 'white' ? '♔' : '♚');
    };
    const playerBar = (color) => {
      const s = color === 'white' ? rv.white : rv.black;
      return h('div', { class: 'player-bar' }, avatarFor(color),
        h('div', { style: 'min-width:0' },
          h('div', { class: 'player-name' }, color === 'white' ? game.white : game.black,
            h('span', { class: 'player-rating' }, ` · ${t(color === 'white' ? 'review.side.white' : 'review.side.black')}`))),
        h('div', { class: 'clock-slot' }, h('span', { class: 'badge rv-acc-chip', style: { '--acc': accColor(s.accuracy) } }, `${fmtAcc(s.accuracy)}%`)));
    };
    const topSlot = h('div', { class: 'rv-bar-slot' });
    const bottomSlot = h('div', { class: 'rv-bar-slot' });
    const renderBars = () => {
      topSlot.replaceChildren(playerBar(st.orientation === 'white' ? 'black' : 'white'));
      bottomSlot.replaceChildren(playerBar(st.orientation));
    };

    const evalSlot = h('div', { class: 'evalbar-slot' });
    const boardSlot = h('div', { class: 'board-slot' });
    const lineBanner = h('div', { class: 'rv-line-banner', hidden: true });
    boardSlot.appendChild(lineBanner);
    const main = h('div', { class: 'game-main' }, topSlot, h('div', { class: 'board-row' }, evalSlot, boardSlot), bottomSlot);

    // ---- panel ----
    const tabR = h('button', { class: 'tab active', role: 'tab', type: 'button', 'aria-selected': 'true', html: icon('chart') + spanHtml(t('review.tabs.review')) });
    const tabC = h('button', { class: 'tab', role: 'tab', type: 'button', 'aria-selected': 'false', html: icon('chat') + spanHtml(t('review.tabs.coach')) });
    const graphHost = h('div', { class: 'rv-graph' });
    const reportView = h('div', { class: 'rv-report' });
    const walkCard = h('div', { class: 'rv-walk-card' });
    const moveListHost = h('div', { class: 'rv-movelist' });
    const walkView = h('div', { class: 'rv-walk', hidden: true }, walkCard, moveListHost);
    const reviewBody = h('div', { class: 'rv-body' }, reportView, walkView);
    const footer = h('div', { class: 'panel-footer rv-footer' });
    const reviewTab = h('div', { class: 'rv-tab' }, graphHost, reviewBody, footer);
    const chatHost = h('div', { class: 'rv-chat' });
    const chatTab = h('div', { class: 'rv-tab', hidden: true }, chatHost);
    const panel = h('div', { class: 'panel grow rv-panel' },
      h('div', { class: 'tabs rv-tabs', role: 'tablist' }, tabR, tabC), reviewTab, chatTab);

    const nav = (ic, label) => h('button', { class: 'btn btn-ghost btn-icon', type: 'button', 'aria-label': label, 'data-tooltip': label, html: icon(ic) });
    const bFirst = nav('first', t('review.nav.first'));
    const bPrev = nav('chevron-left', t('review.nav.prev'));
    const bNext = nav('chevron-right', t('review.nav.next'));
    const bLast = nav('last', t('review.nav.last'));
    const bFlip = nav('flip', t('review.nav.flip'));
    const bAnalyze = h('a', { class: 'btn btn-ghost btn-icon', 'aria-label': t('review.nav.analyse'), 'data-tooltip': t('review.nav.analyseTip'), html: icon('analysis') });
    // "Take over from here": play this position against the same bot (CONTRACT: #/play?fen=).
    const bPlayHere = h('a', { class: 'btn btn-ghost btn-icon', 'aria-label': t('review.nav.playFromHere'), 'data-tooltip': t('review.nav.playFromHereTip'), html: icon('robot') });
    const setPlayHref = (fen) => {
      const q = new URLSearchParams({ fen });
      if (game.bot_id) q.set('bot', game.bot_id);
      if (game.user_color === 'white' || game.user_color === 'black') q.set('color', game.user_color === 'black' ? 'b' : 'w');
      bPlayHere.href = `#/play?${q}`;
    };
    setPlayHref(startFen);
    const toolbar = h('div', { class: 'toolbar' }, bFirst, bPrev, bNext, bLast, bFlip, bAnalyze, bPlayHere);
    const aside = h('aside', { class: 'game-panel' }, panel, toolbar);

    const layout = h('div', { class: 'game-layout review-layout' + (settings.showEvalBar === false ? ' no-eval' : ''), style: { '--panel-w': '420px' } }, main, aside);
    page.replaceChildren(layout);
    renderBars();

    // ---- components ----
    const board = new Board(boardSlot, {
      fen: startFen, orientation, interactive: false, movableColor: null,
      showCoords: settings.showCoords, showLegal: settings.showLegal, animationMs: settings.animationMs, sounds: settings.sounds,
      onMove: (m) => (st.retry ? onRetryMove(m) : onExploreMove(m)),
    });
    bag.add(() => board.destroy());
    const evalBar = new EvalBar(evalSlot, { orientation });
    bag.add(() => evalBar.destroy());
    const graph = new EvalGraph(graphHost, { onSelect: (p) => { if (st.mode === 'report') setMode('walk'); goTo(p); }, height: 96 });
    bag.add(() => graph.destroy());
    graph.setData(rv.evals, moves.map((m) => m.classification), moves.map((m) => m.san));
    const moveList = new MoveList(moveListHost, { onSelect: (p) => goTo(p) });
    bag.add(() => moveList.destroy());
    moveList.setMoves(moves.map((m) => ({ san: m.san, classification: m.classification })),
      { startColor: ply0 % 2 ? 'black' : 'white', startMoveNumber: Math.floor(ply0 / 2) + 1 });
    const mentor = new MentorPanel(chatHost, {
      getContext: () => {
        const ex = st.explore;
        if (ex) {
          const cur = ex.line[ex.idx - 1];
          const lines = [];
          const top = ex.info?.lines?.[0];
          if (top) lines.push(`${formatScore(top.score)}: ${(top.san || []).slice(0, 8).join(' ')}`);
          return {
            fen: cur ? cur.fen : exploreBaseFen(ex),
            moves_san: moves.slice(0, ex.basePly).map((x) => x.san).concat(ex.line.slice(0, ex.idx).map((x) => x.san)),
            engine_lines: lines,
          };
        }
        const p = st.ply;
        const fen = p === 0 ? startFen : moves[p - 1].fen_after;
        const m = p > 0 ? moves[p - 1] : null;
        const lines = [];
        if (m) lines.push(`${formatScore(m.eval_after)}: played ${m.san} (${m.classification}); best was ${m.best_move_san} — ${m.best_line_san.slice(0, 8).join(' ')}`);
        return { fen, moves_san: moves.slice(0, p).map((x) => x.san), engine_lines: lines };
      },
      greeting: t('review.mentor.greeting'),
      suggestions: t('review.mentor.suggestions'),
    });
    bag.add(() => mentor.destroy());
    bag.add(onSettingsChange((s, key) => {
      if (key === 'showEvalBar') layout.classList.toggle('no-eval', s.showEvalBar === false);
    }));

    // ---- report view ----
    renderReport();

    function renderReport() {
      const counts = (side, k) => Number(side?.counts?.[k] || 0);
      const order = TABLE_ORDER.concat(counts(rv.white, 'forced') + counts(rv.black, 'forced') ? ['forced'] : []);
      const accCard = (color) => {
        const s = color === 'white' ? rv.white : rv.black;
        return h('div', { class: `rv-acc-card ${color}` },
          h('div', { class: 'rv-acc-name truncate' }, color === 'white' ? game.white : game.black),
          h('div', { class: 'rv-acc-ring', style: { '--value': Math.round(s.accuracy), '--acc': accColor(s.accuracy) } },
            h('span', { class: 'rv-acc-value tabular' }, fmtAcc(s.accuracy))),
          h('div', { class: 'rv-acc-label' }, t('review.report.accuracy')),
          s.estimated_elo ? h('div', { class: 'rv-elo' }, h('span', { class: 'subtle text-xs' }, t('review.report.gameRating')), h('strong', { class: 'tabular' }, String(s.estimated_elo))) : null);
      };
      const opening = rv.opening?.name || game.opening_name;
      const keyChips = rv.key_moments.filter((p) => p >= 1 && p <= n).slice(0, 12).map((p) => {
        const m = moves[p - 1];
        return h('button', { class: 'chip rv-key-chip', type: 'button', dataset: { ply: p, cls: m.classification } },
          classificationBadge(m.classification), ` ${moveLabel(p, m.san)}`);
      });
      reportView.replaceChildren(...[
        h('div', { class: 'mentor-row rv-summary' }, coachAvatar('avatar-lg'),
          h('div', { class: 'bubble bubble-mentor md', html: mdLite(rv.summary || defaultSummary()) })),
        h('div', { class: 'rv-acc-row' }, accCard('white'), h('div', { class: 'rv-vs' }, t('review.report.vs')), accCard('black')),
        opening ? h('div', { class: 'rv-opening', html: icon('book') + `<span></span>` }) : null,
        h('div', { class: 'rv-cls-table', role: 'table', 'aria-label': t('review.report.classifications') },
          h('div', { class: 'rv-cls-row head', role: 'row' },
            h('span', { role: 'columnheader' }, game.white), h('span', { role: 'columnheader' }), h('span', { role: 'columnheader' }, game.black)),
          order.map((k) => {
            const meta = classificationMeta(k);
            const w = counts(rv.white, k); const b = counts(rv.black, k);
            return h('div', { class: 'rv-cls-row', role: 'row', dataset: { cls: k }, title: meta.description },
              h('span', { class: 'rv-cls-count' + (w ? '' : ' zero'), role: 'cell' }, String(w)),
              h('span', { class: 'rv-cls-mid', role: 'cell' }, classificationBadge(k), h('span', { class: 'rv-cls-label' }, meta.label)),
              h('span', { class: 'rv-cls-count' + (b ? '' : ' zero'), role: 'cell' }, String(b)));
          })),
        keyChips.length ? h('div', { class: 'rv-keys' },
          h('div', { class: 'rv-section-title', html: icon('target') + spanHtml(t('review.report.keyMoments')) }),
          h('div', { class: 'chip-row' }, keyChips)) : null,
      ].filter(Boolean));
      const op = reportView.querySelector('.rv-opening span');
      if (op) op.textContent = opening;
    }
    bag.on(reportView, 'click', (e) => {
      const chip = e.target.closest('.rv-key-chip');
      if (chip) { setMode('walk'); goTo(Number(chip.dataset.ply)); }
    });

    function defaultSummary() {
      const ua = game.user_color === 'black' ? rv.black.accuracy : rv.white.accuracy;
      return t('review.report.defaultSummary', { accuracy: fmtAcc(ua) });
    }

    // ---- mode / tabs ----
    function setMode(mode) {
      st.mode = mode;
      reportView.hidden = mode !== 'report';
      walkView.hidden = mode !== 'walk';
      reviewBody.scrollTop = 0;
      renderFooter();
      if (mode === 'walk') { renderWalk(); moveList.setCurrent(st.ply); }
    }

    function setTab(which) {
      st.tab = which;
      tabR.classList.toggle('active', which === 'review');
      tabC.classList.toggle('active', which === 'chat');
      tabR.setAttribute('aria-selected', String(which === 'review'));
      tabC.setAttribute('aria-selected', String(which === 'chat'));
      reviewTab.hidden = which !== 'review';
      chatTab.hidden = which !== 'chat';
    }
    bag.on(tabR, 'click', () => setTab('review'));
    bag.on(tabC, 'click', () => setTab('chat'));

    function renderFooter() {
      if (st.mode === 'report') {
        footer.replaceChildren(h('button', { class: 'btn btn-primary btn-lg btn-block rv-start', type: 'button', dataset: { act: 'start' }, html: icon('play') + spanHtml(t('review.footer.start')) }));
      } else {
        footer.replaceChildren(
          st.explore
            ? h('button', { class: 'btn btn-ghost btn-icon rv-back-game', type: 'button', dataset: { act: 'explore-exit' }, 'aria-label': t('review.footer.backToGame'), 'data-tooltip': t('review.footer.backToGame'), html: icon('undo') })
            : h('button', { class: 'btn btn-ghost btn-icon', type: 'button', dataset: { act: 'report' }, 'aria-label': t('review.footer.backToReport'), 'data-tooltip': t('review.footer.backToReport'), html: icon('chart') }),
          h('button', { class: 'btn btn-secondary rv-nav-btn', type: 'button', dataset: { act: 'prev' }, disabled: !st.explore && st.ply <= 0, html: icon('chevron-left') + spanHtml(t('review.footer.prev')) }),
          h('button', { class: 'btn btn-primary rv-nav-btn', type: 'button', dataset: { act: 'next' }, disabled: st.explore ? st.explore.idx >= st.explore.line.length : st.ply >= n, html: spanHtml(t('review.footer.next')) + icon('chevron-right') }));
      }
    }
    bag.on(footer, 'click', (e) => {
      const b = e.target.closest('[data-act]');
      if (!b || b.disabled) return;
      const a = b.dataset.act;
      if (a === 'start') { setMode('walk'); goTo(st.ply === n ? 0 : st.ply); }
      else if (a === 'report') setMode('report');
      else if (a === 'explore-exit') goTo(st.explore ? st.explore.basePly : st.ply);
      else if (a === 'prev') stepBack();
      else if (a === 'next') stepForward();
    });

    // ---- navigation ----
    function arrowsFor(p) {
      if (p <= 0) return [];
      const m = moves[p - 1];
      const out = [];
      if (m.best_move_uci && m.best_move_uci !== m.uci && SHOW_BEST.has(m.classification)) {
        const b = uciSquares(m.best_move_uci);
        if (b) out.push({ from: b[0], to: b[1], color: 'green' });
        const pl = uciSquares(m.uci);
        if (pl && (m.classification === 'mistake' || m.classification === 'blunder' || m.classification === 'miss')) {
          out.push({ from: pl[0], to: pl[1], color: 'red' });
        } else if (pl && m.classification === 'inaccuracy') {
          out.push({ from: pl[0], to: pl[1], color: 'yellow' });
        }
      }
      return out;
    }

    function showPly(p, animate) {
      const m = p > 0 ? moves[p - 1] : null;
      // The board is always "live": moving a piece starts exploring a new line from here.
      board.setInteractive(true, 'both');
      board.setPosition(m ? m.fen_after : startFen, { animate, lastMove: m ? uciSquares(m.uci) : null });
      board.clearBadges();
      board.clearHighlights();
      if (m) { const sq = uciSquares(m.uci); if (sq) board.setBadge(sq[1], m.classification); }
      const arrows = arrowsFor(p);
      if (arrows.length) board.setArrows(arrows); else board.clearArrows();
      evalBar.set(rv.evals[p] || { cp: 0 }, { fen: m ? m.fen_after : startFen });
    }

    function goTo(p) {
      p = Math.max(0, Math.min(n, Number(p) || 0));
      stopLine();
      endRetry();
      endExplore();
      const animate = Math.abs(p - st.ply) === 1;
      st.ply = p;
      showPly(p, animate);
      graph.setCurrent(p);
      moveList.setCurrent(p);
      bAnalyze.href = `#/analysis?game=${gameId}&ply=${p}`;
      setPlayHref(p > 0 ? moves[p - 1].fen_after : startFen);
      if (st.mode === 'walk') { renderWalk(); renderFooter(); }
    }

    // ---- walk-through card ----
    let why = null; // WhyPanel for the current move ("Why was that a mistake?")
    bag.add(() => { why?.destroy(); why = null; });

    function renderWalk() {
      why?.destroy();
      why = null;
      const p = st.ply;
      if (p === 0) {
        const opening = rv.opening?.name || game.opening_name;
        walkCard.replaceChildren(h('div', { class: 'mentor-row rv-coach' }, coachAvatar('avatar-lg'),
          h('div', { class: 'bubble bubble-mentor' },
            h('div', { class: 'rv-walk-head' }, h('strong', null, t('review.walk.introTitle'))),
            h('div', { class: 'md', html: mdLite(t('review.walk.intro') + (opening ? `\n\n${t('review.walk.opening', { name: escapeMd(opening) })}` : '')) }))));
        return;
      }
      const m = moves[p - 1];
      const meta = classificationMeta(m.classification);
      const isKey = keySet.has(p);
      const showBest = m.best_move_san && m.best_move_uci !== m.uci && SHOW_BEST.has(m.classification);
      const actions = [];
      if (Array.isArray(m.best_line_san) && m.best_line_san.length && m.best_move_uci !== m.uci && m.classification !== 'book' && m.classification !== 'forced') {
        actions.push(h('button', { class: 'btn btn-secondary btn-sm', type: 'button', dataset: { act: 'line' }, html: icon('play-circle') + spanHtml(t('review.walk.showLine')) }));
      }
      if (RETRY_CLASSES.has(m.classification) && m.best_move_uci && m.best_move_uci !== m.uci) {
        actions.push(h('button', { class: 'btn btn-primary btn-sm', type: 'button', dataset: { act: 'retry' }, html: icon('refresh') + spanHtml(t('review.walk.retry')) }));
      }
      if (keySet.size) {
        const nextKey = rv.key_moments.find((k) => k > p);
        if (nextKey) actions.push(h('button', { class: 'btn btn-ghost btn-sm', type: 'button', dataset: { act: 'key', ply: nextKey }, html: icon('target') + spanHtml(t('review.walk.nextKey')) }));
      }
      walkCard.replaceChildren(
        h('div', { class: 'mentor-row rv-coach pop-in', dataset: { cls: m.classification } }, coachAvatar('avatar-lg'),
          h('div', { class: 'bubble bubble-mentor rv-bubble' },
            h('div', { class: 'rv-walk-head' },
              classificationBadge(m.classification, { large: true }),
              h('span', { class: 'rv-walk-title' }, ...phraseNodes(m.classification, h('strong', null, moveLabel(p, formatSan(m.san, getSettings().moveNotation))))),
              h('span', { class: 'spacer' }),
              h('span', { class: 'engine-score' + (isNegScore(m.eval_after) ? ' neg' : '') }, formatScore(m.eval_after))),
            isKey ? h('div', { class: 'badge badge-warning rv-key-tag', html: icon('target') + spanHtml(t('review.walk.keyMoment')) }) : null,
            h('div', { class: 'md rv-expl', html: mdLite(m.explanation || meta.description) }),
            m.classification === 'book' && m.opening_name ? h('div', { class: 'rv-book', html: icon('book') + '<span></span>' }) : null,
            showBest ? h('div', { class: 'rv-best' }, h('span', { class: 'rv-best-dot' }), ...withNode(t('review.walk.bestWas'), 'move', h('strong', null, formatSan(m.best_move_san, getSettings().moveNotation))),
              h('span', { class: 'subtle' }, ` ${t('review.walk.evalBefore', { eval: formatScore(rv.evals[p - 1]) })}`)) : null,
            actions.length ? h('div', { class: 'row-wrap rv-actions' }, actions) : null,
            h('div', { class: 'rv-try-tip subtle text-xs', html: icon('hint') + spanHtml(t('review.walk.tryTip')) }))));
      const bk = walkCard.querySelector('.rv-book span');
      if (bk) bk.textContent = m.opening_name;
      const expl = walkCard.querySelector('.rv-expl');
      if (expl && RETRY_CLASSES.has(m.classification) && hasReason(m)) {
        const whyHost = h('div', { class: 'rv-why' });
        expl.after(whyHost);
        why = new WhyPanel(whyHost, {
          move: m, ply0, board,
          onStart: () => { stopLine(); endRetry(); },
          onExit: () => showPly(st.ply, false),
        });
      }
    }
    bag.on(walkCard, 'click', (e) => {
      const b = e.target.closest('[data-act]');
      if (!b) return;
      const a = b.dataset.act;
      if (a === 'line') playLine();
      else if (a === 'retry') startRetry();
      else if (a === 'key') goTo(Number(b.dataset.ply));
      else if (a === 'hint') retryHint();
      else if (a === 'answer') retryAnswer();
      else if (a === 'stop-retry') goTo(st.ply);
      else if (a === 'stop-line') { stopLine(); showPly(st.ply, false); }
      else if (a === 'explore-go') exploreGo(Number(b.dataset.i));
      else if (a === 'explore-undo') exploreUndo();
      else if (a === 'explore-exit') goTo(st.explore ? st.explore.basePly : st.ply);
    });

    // ---- show line ----
    function stopLine() {
      why?.stop(false);
      if (st.lineTimer) { clearTimeout(st.lineTimer); st.lineTimer = 0; }
      if (st.lineActive) { st.lineActive = false; lineBanner.hidden = true; }
    }
    bag.add(stopLine);

    function playLine() {
      const p = st.ply;
      if (p < 1) return;
      endRetry();
      stopLine();
      const m = moves[p - 1];
      const c = new Chess(m.fen_before);
      const sans = m.best_line_san.slice(0, 10);
      st.lineActive = true;
      board.setInteractive(false, null);
      board.clearBadges();
      board.clearArrows();
      board.setPosition(m.fen_before, { animate: false, lastMove: null });
      const startPly = ply0 + p - 1;
      const tokens = [];
      const notation = getSettings().moveNotation;
      sans.forEach((s, i) => {
        const abs = startPly + i;
        if (abs % 2 === 0 || i === 0) tokens.push(h('span', { class: 'rv-line-num' }, `${Math.floor(abs / 2) + 1}${abs % 2 === 0 ? '.' : '…'}`));
        tokens.push(h('span', { class: 'rv-line-move', dataset: { i } }, formatSan(s, notation)));
      });
      const stopBtn = h('button', { class: 'btn btn-ghost btn-icon btn-sm', type: 'button', 'aria-label': t('review.walk.stop'), html: icon('close') });
      stopBtn.addEventListener('click', () => { stopLine(); showPly(st.ply, false); });
      lineBanner.replaceChildren(h('span', { class: 'rv-line-label', html: icon('play-circle') + spanHtml(t('review.walk.bestLine')) }), h('span', { class: 'rv-line-moves' }, tokens), stopBtn);
      lineBanner.hidden = false;
      let i = 0;
      const stepFn = () => {
        st.lineTimer = 0;
        if (bag.disposed || !st.lineActive) return;
        if (i >= sans.length) return;
        let mv = null;
        try { mv = c.move(sans[i]); } catch { mv = null; }
        if (!mv) return;
        board.setPosition(c.fen(), { animate: true, lastMove: [mv.from, mv.to] });
        lineBanner.querySelectorAll('.rv-line-move').forEach((el) => el.classList.toggle('on', Number(el.dataset.i) === i));
        i++;
        st.lineTimer = setTimeout(stepFn, 900);
      };
      st.lineTimer = setTimeout(stepFn, 350);
    }

    // ---- retry ----
    function endRetry() {
      if (!st.retry) return;
      st.retry.ctrl?.abort();
      if (st.retry.timer) clearTimeout(st.retry.timer);
      st.retry = null;
      board.setInteractive(false, null);
    }
    bag.add(endRetry);

    function startRetry() {
      const p = st.ply;
      if (p < 1) return;
      stopLine();
      endRetry();
      const m = moves[p - 1];
      const color = m.color === 'black' ? 'black' : 'white';
      st.retry = { ply: p, move: m, color, ctrl: null, timer: 0, tries: 0, busy: false, solved: false };
      board.clearBadges();
      board.clearArrows();
      board.clearHighlights();
      board.setPosition(m.fen_before, { animate: false, lastMove: null });
      board.setInteractive(true, color);
      evalBar.set(rv.evals[p - 1] || { cp: 0 });
      renderRetryCard(t(color === 'white' ? 'review.retry.promptWhite' : 'review.retry.promptBlack', { played: phrase(m.classification, escapeMd(m.san), 'played') }), 'info');
    }

    function renderRetryCard(text, kind) {
      const r = st.retry;
      walkCard.replaceChildren(h('div', { class: `mentor-row rv-coach pop-in rv-retry kind-${kind}` }, coachAvatar('avatar-lg'),
        h('div', { class: 'bubble bubble-mentor rv-bubble' },
          h('div', { class: 'rv-walk-head' }, h('span', { class: 'rv-retry-icon', html: icon(kind === 'success' ? 'check-circle' : kind === 'error' ? 'x-circle' : 'target') }),
            h('strong', null, t(kind === 'success' ? 'review.retry.titleSuccess' : kind === 'error' ? 'review.retry.titleError' : 'review.retry.titleTurn'))),
          h('div', { class: 'md rv-expl', html: mdLite(text) }),
          h('div', { class: 'row-wrap rv-actions' },
            r && !r.solved ? h('button', { class: 'btn btn-secondary btn-sm', type: 'button', dataset: { act: 'hint' }, html: icon('hint') + spanHtml(t('review.retry.hint')) }) : null,
            r && !r.solved ? h('button', { class: 'btn btn-ghost btn-sm', type: 'button', dataset: { act: 'answer' }, html: icon('eye') + spanHtml(t('review.retry.showAnswer')) }) : null,
            h('button', { class: 'btn btn-ghost btn-sm', type: 'button', dataset: { act: 'stop-retry' }, html: icon('undo') + spanHtml(t('review.retry.backToGame')) }),
            r?.solved && st.ply < n ? h('button', { class: 'btn btn-primary btn-sm', type: 'button', dataset: { act: 'key', ply: st.ply + 1 }, html: spanHtml(t('review.retry.continue')) + icon('chevron-right') }) : null))));
    }

    function retryHint() {
      const r = st.retry;
      if (!r) return;
      const sq = uciSquares(r.move.best_move_uci);
      if (sq) board.setHighlights([{ square: sq[0], kind: 'hint' }]);
      renderRetryCard(t('review.retry.hintText'), 'info');
    }

    function retryAnswer() {
      const r = st.retry;
      if (!r) return;
      const sq = uciSquares(r.move.best_move_uci);
      if (sq) board.setArrows([{ from: sq[0], to: sq[1], color: 'green' }]);
      r.solved = true;
      board.setInteractive(false, null);
      renderRetryCard(t('review.retry.answer', { move: escapeMd(r.move.best_move_san), line: escapeMd(numberedLine(r.move.best_line_san, ply0 + r.ply - 1, getSettings().moveNotation, 8)) }), 'info');
    }

    function onRetryMove(mv) {
      const r = st.retry;
      if (!r || r.busy || r.solved) return false;
      const uci = mv.uci || (mv.from + mv.to + (mv.promotion || ''));
      r.tries++;
      board.clearHighlights();
      if (uci === r.move.best_move_uci) {
        retrySuccess(mv, 'best', t(r.tries === 1 ? 'review.retry.bestFirstTry' : 'review.retry.bestLater', { move: escapeMd(mv.san) }));
        return true;
      }
      if (uci === r.move.uci) {
        retryFail(mv, r.move.classification, t('review.retry.sameAsGame', { move: escapeMd(mv.san) }));
        return true;
      }
      // Ask the server how good the alternative is.
      r.busy = true;
      board.setInteractive(false, null);
      r.ctrl = new AbortController();
      api.post('/api/mentor/explain', { fen: r.move.fen_before, move_uci: uci }, { signal: r.ctrl.signal, timeout: 30000 })
        .then((res) => {
          if (bag.disposed || st.retry !== r) return;
          r.busy = false;
          const cls = String(res?.classification || '');
          if (GOOD_RETRY.has(cls)) retrySuccess(mv, cls, t('review.retry.alsoGood', { verdict: phrase(cls, `**${escapeMd(mv.san)}**`, 'works'), explanation: res?.explanation ? escapeMdKeep(res.explanation) : '' }).trim());
          else retryFail(mv, cls || 'inaccuracy', t('review.retry.notGood', { verdict: phrase(cls, escapeMd(mv.san), 'notBest'), explanation: res?.explanation ? escapeMdKeep(res.explanation) : t('review.retry.tryAgain') }));
        })
        .catch((e) => {
          if (isAbort(e) || bag.disposed || st.retry !== r) return;
          r.busy = false;
          retryFail(mv, null, t('review.retry.checkFailed'));
        });
      return true;
    }

    function retrySuccess(mv, cls, text) {
      const r = st.retry;
      r.solved = true;
      board.setInteractive(false, null);
      board.setBadge(mv.to, cls);
      try { if (getSettings().sounds) playSound('correct'); } catch { /* ignore */ }
      renderRetryCard(text, 'success');
    }

    function retryFail(mv, cls, text) {
      const r = st.retry;
      if (cls) board.setBadge(mv.to, cls);
      try { if (getSettings().sounds) playSound('wrong'); } catch { /* ignore */ }
      renderRetryCard(text, 'error');
      board.setInteractive(false, null);
      r.timer = setTimeout(() => {
        if (bag.disposed || st.retry !== r) return;
        r.timer = 0;
        board.clearBadges();
        board.setPosition(r.move.fen_before, { animate: true, lastMove: null });
        board.setInteractive(true, r.color);
      }, 1100);
    }


    // ---- explore your own lines ----
    let engine = null;
    bag.add(() => { engine?.close(); engine = null; });
    let exploreCtrl = null;
    bag.add(() => exploreCtrl?.abort());

    function exploreBaseFen(ex) { return ex.basePly === 0 ? startFen : moves[ex.basePly - 1].fen_after; }

    function stepBack() {
      const ex = st.explore;
      if (!ex) { goTo(st.ply - 1); return; }
      if (ex.idx > 1) exploreGo(ex.idx - 1);
      else goTo(ex.basePly); // first move of the line undone: back on the real game
    }

    function stepForward() {
      const ex = st.explore;
      if (!ex) { if (st.mode === 'report') setMode('walk'); goTo(st.ply + 1); return; }
      if (ex.idx < ex.line.length) exploreGo(ex.idx + 1);
    }

    function endExplore() {
      if (!st.explore) return;
      st.explore = null;
      exploreCtrl?.abort();
      exploreCtrl = null;
      engine?.stop();
      walkView.classList.remove('exploring');
      renderFooter();
    }

    function onExploreMove(mv) {
      const uci = mv.uci || (mv.from + mv.to + (mv.promotion || ''));
      if (st.mode === 'report') setMode('walk');
      stopLine();
      if (!st.explore) {
        // Playing the game's own next move simply follows the game.
        const next = moves[st.ply];
        if (next && next.uci === uci) {
          queueMicrotask(() => { if (!bag.disposed) goTo(st.ply + 1); });
          return true;
        }
        st.explore = { basePly: st.ply, line: [], idx: 0, info: null };
        walkView.classList.add('exploring');
      }
      const ex = st.explore;
      if (ex.idx < ex.line.length) {
        if (ex.line[ex.idx].uci === uci) { queueMicrotask(() => exploreGo(ex.idx + 1, false)); return true; }
        ex.line = ex.line.slice(0, ex.idx); // a new move replaces the rest of the line
      }
      const entry = { uci, san: mv.san, from: mv.from, to: mv.to, fen: mv.fen, before: mv.before, cls: null, expl: '', best: '', pending: true };
      ex.line.push(entry);
      ex.idx = ex.line.length;
      queueMicrotask(() => { if (st.explore === ex) exploreShow(false); });
      judge(entry, ex);
      return true;
    }

    /** Ask the coach how good an explored move is (badge + explanation). */
    function judge(entry, ex) {
      exploreCtrl?.abort();
      exploreCtrl = new AbortController();
      api.post('/api/mentor/explain', { fen: entry.before, move_uci: entry.uci }, { signal: exploreCtrl.signal, timeout: 30000 })
        .then((res) => {
          entry.pending = false;
          entry.cls = String(res?.classification || '') || null;
          entry.expl = String(res?.explanation || '');
          entry.best = res?.best_move_san && res.best_move_san !== entry.san ? String(res.best_move_san) : '';
          if (bag.disposed || st.explore !== ex || ex.line[ex.idx - 1] !== entry) return;
          if (entry.cls) board.setBadge(entry.to, entry.cls);
          renderExploreCard();
        })
        .catch((e) => {
          entry.pending = false;
          if (isAbort(e) || bag.disposed || st.explore !== ex) return;
          renderExploreCard();
        });
    }

    function exploreGo(i, animate = true) {
      const ex = st.explore;
      if (!ex) return;
      ex.idx = Math.max(1, Math.min(ex.line.length, i));
      exploreShow(animate);
      const cur = ex.line[ex.idx - 1];
      if (cur.cls === null && !cur.pending) { cur.pending = true; judge(cur, ex); }
    }

    function exploreShow(animate) {
      const ex = st.explore;
      const cur = ex.line[ex.idx - 1];
      board.setInteractive(true, 'both');
      board.setPosition(cur.fen, { animate, lastMove: [cur.from, cur.to] });
      board.clearBadges();
      board.clearArrows();
      board.clearHighlights();
      if (cur.cls) board.setBadge(cur.to, cur.cls);
      moveList.setCurrent(ex.basePly);
      graph.setCurrent(ex.basePly);
      bAnalyze.href = `#/analysis?fen=${encodeURIComponent(cur.fen)}`;
      setPlayHref(cur.fen);
      ex.info = null;
      renderExploreCard();
      renderFooter();
      analyse(cur.fen, ex);
    }

    /** Live engine evaluation of the explored position: eval bar, best-move arrow, top line. */
    function analyse(fen, ex) {
      let over = false;
      try { over = new Chess(fen).isGameOver(); } catch { over = true; }
      if (over) { engine?.stop(); evalBar.set({ mate: 0 }, { fen }); return; }
      if (!engine) engine = new EngineClient();
      engine.analyze(fen, { multipv: 1, movetime_ms: 2500 }, (info) => {
        if (bag.disposed || st.explore !== ex || ex.line[ex.idx - 1]?.fen !== fen) return;
        const top = info?.lines?.[0];
        if (!top) return;
        ex.info = info;
        evalBar.set(top.score, { fen });
        const sq = uciSquares(top.moves?.[0]);
        if (sq && !st.retry) board.setArrows([{ from: sq[0], to: sq[1], color: 'blue' }]);
        const eng = walkCard.querySelector('.rv-explore-engine');
        if (eng) eng.replaceChildren(...engineLineNodes(info, ex));
      }, () => {});
    }

    function engineLineNodes(info, ex) {
      const top = info?.lines?.[0];
      if (!top) return [h('span', { class: 'spinner spinner-sm' }), h('span', { class: 'subtle' }, ` ${t('review.explore.thinking')}`)];
      const absPly = ply0 + ex.basePly + ex.idx;
      return [
        h('span', { class: 'engine-score' + (isNegScore(top.score) ? ' neg' : '') }, formatScore(top.score)),
        h('span', { class: 'rv-explore-pv' }, numberedLine(top.san || [], absPly, getSettings().moveNotation, 8)),
        h('span', { class: 'subtle text-xs' }, ` ${t('review.explore.depth', { depth: info.depth })}`),
      ];
    }

    function renderExploreCard() {
      const ex = st.explore;
      if (!ex) return;
      const cur = ex.line[ex.idx - 1];
      const notation = getSettings().moveNotation;
      const chips = [];
      ex.line.forEach((m, i) => {
        const abs = ply0 + ex.basePly + i;
        if (abs % 2 === 0 || i === 0) chips.push(h('span', { class: 'rv-line-num' }, `${Math.floor(abs / 2) + 1}${abs % 2 === 0 ? '.' : '…'}`));
        chips.push(h('button', { class: 'rv-ex-move' + (i === ex.idx - 1 ? ' on' : ''), type: 'button', dataset: { act: 'explore-go', i: i + 1, cls: m.cls || '' } },
          m.cls ? classificationBadge(m.cls) : null, formatSan(m.san, notation)));
      });
      const verdict = cur.pending
        ? h('div', { class: 'rv-expl subtle' }, h('span', { class: 'spinner spinner-sm' }), ` ${t('review.explore.checking')}`)
        : cur.cls
          ? h('div', null,
            h('div', { class: 'rv-walk-head' }, classificationBadge(cur.cls, { large: true }),
              h('span', { class: 'rv-walk-title' }, ...phraseNodes(cur.cls, h('strong', null, formatSan(cur.san, notation))))),
            cur.expl ? h('div', { class: 'md rv-expl', html: mdLite(escapeMdKeep(cur.expl)) }) : null,
            cur.best && !GOOD_RETRY.has(cur.cls) && cur.cls !== 'book' && cur.cls !== 'forced'
              ? h('div', { class: 'rv-best' }, h('span', { class: 'rv-best-dot' }), ...withNode(t('review.walk.bestWas'), 'move', h('strong', null, formatSan(cur.best, notation)))) : null)
          : null;
      const fromText = ex.basePly === 0 ? t('review.explore.fromStart')
        : t('review.explore.fromMove', { move: moveLabel(ex.basePly, formatSan(moves[ex.basePly - 1].san, notation)) });
      walkCard.replaceChildren(h('div', { class: 'mentor-row rv-coach rv-explore pop-in' }, coachAvatar('avatar-lg'),
        h('div', { class: 'bubble bubble-mentor rv-bubble' },
          h('div', { class: 'rv-walk-head' }, h('span', { class: 'rv-explore-icon', html: icon('analysis') }),
            h('strong', null, t('review.explore.title')), h('span', { class: 'spacer' }),
            h('span', { class: 'badge rv-explore-tag' }, t('review.explore.notInGame'))),
          h('div', { class: 'subtle text-xs rv-explore-from' }, fromText),
          h('div', { class: 'rv-explore-line' }, chips),
          verdict,
          h('div', { class: 'rv-explore-engine' }, ...engineLineNodes(ex.info, ex)),
          h('div', { class: 'row-wrap rv-actions' },
            h('button', { class: 'btn btn-secondary btn-sm', type: 'button', dataset: { act: 'explore-undo' }, html: icon('undo') + spanHtml(t('review.explore.undo')) }),
            h('button', { class: 'btn btn-primary btn-sm', type: 'button', dataset: { act: 'explore-exit' }, html: icon('arrow-left') + spanHtml(t('review.explore.backToGame')) })))));
    }

    function exploreUndo() {
      const ex = st.explore;
      if (!ex) return;
      ex.line = ex.line.slice(0, ex.idx - 1);
      if (!ex.line.length) { goTo(ex.basePly); return; }
      exploreGo(ex.line.length, true);
    }

    // ---- toolbar & keyboard ----
    const flip = () => {
      st.orientation = st.orientation === 'white' ? 'black' : 'white';
      board.setOrientation(st.orientation);
      evalBar.setOrientation(st.orientation);
      renderBars();
    };
    bag.on(bFirst, 'click', () => goTo(0));
    bag.on(bPrev, 'click', () => stepBack());
    bag.on(bNext, 'click', () => stepForward());
    bag.on(bLast, 'click', () => goTo(n));
    bag.on(bFlip, 'click', flip);
    bag.on(window, 'keydown', (e) => {
      if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
      const tgt = e.target;
      if (tgt && (tgt.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(tgt.tagName))) return;
      if (document.querySelector('.modal-backdrop')) return;
      if (e.key === 'ArrowLeft') { e.preventDefault(); stepBack(); }
      else if (e.key === 'ArrowRight') { e.preventDefault(); stepForward(); }
      else if (e.key === 'Home') { e.preventDefault(); goTo(0); }
      else if (e.key === 'End') { e.preventDefault(); goTo(n); }
      else if (e.key === 'f' || e.key === 'F') flip();
    });

    // ---- start ----
    // `?ply=<n>` (e.g. from Insights) opens that move in the walkthrough.
    const startPly = Number.parseInt(query.ply, 10);
    if (startPly >= 1 && startPly <= n) { setMode('walk'); goTo(startPly); } else { setMode('report'); goTo(0); }
  }

  function errorCard(titleText, text) {
    return h('div', { class: 'page' }, emptyState({
      icon: 'alert', title: titleText, text,
      action: { label: t('review.error.goLibrary'), href: '#/library', icon: 'library' },
    }));
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
function normalizeReview(r) {
  if (!r || typeof r !== 'object' || !Array.isArray(r.moves) || !r.moves.length) return null;
  const moves = r.moves.filter((m) => m && typeof m.fen_after === 'string' && typeof m.uci === 'string').map((m) => ({
    ...m,
    san: String(m.san || m.uci),
    classification: String(m.classification || 'good'),
    best_line_san: Array.isArray(m.best_line_san) ? m.best_line_san.map(String) : [],
    best_move_uci: String(m.best_move_uci || ''),
    best_move_san: String(m.best_move_san || ''),
    explanation: String(m.explanation || ''),
    color: m.color === 'black' ? 'black' : 'white',
  }));
  if (!moves.length) return null;
  const evals = Array.isArray(r.evals) ? r.evals.slice(0, moves.length + 1) : [];
  if (!evals.length) evals.push(moves[0].eval_before || { cp: 0 });
  while (evals.length < moves.length + 1) evals.push(moves[evals.length - 1]?.eval_after || evals[evals.length - 1] || { cp: 0 });
  const side = (s) => ({
    accuracy: Number.isFinite(Number(s?.accuracy)) ? Number(s.accuracy) : 0,
    estimated_elo: Number(s?.estimated_elo) || 0,
    counts: (s && typeof s.counts === 'object' && s.counts) || {},
  });
  return {
    start_fen: typeof r.start_fen === 'string' && r.start_fen ? r.start_fen : START_FEN,
    moves, evals,
    white: side(r.white), black: side(r.black),
    opening: r.opening && typeof r.opening === 'object' ? r.opening : null,
    key_moments: Array.isArray(r.key_moments) ? r.key_moments.map(Number).filter(Number.isFinite).sort((a, b) => a - b) : [],
    summary: typeof r.summary === 'string' ? r.summary : '',
  };
}

function fmtAcc(a) {
  const v = Number(a) || 0;
  return v >= 99.95 ? formatNumber(100) : formatNumber(v, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
}

/** Text as a <span> HTML string (escaped), for buttons built with `html: icon(..) + ...`. */
function spanHtml(text) {
  return h('span', null, text).outerHTML;
}


function accColor(a) {
  const v = Number(a) || 0;
  if (v >= 90) return 'var(--cls-best)';
  if (v >= 75) return 'var(--cls-excellent)';
  if (v >= 60) return 'var(--cls-inaccuracy)';
  if (v >= 45) return 'var(--cls-mistake)';
  return 'var(--cls-blunder)';
}

function isNegScore(s) {
  return typeof s?.mate === 'number' ? s.mate < 0 : (s?.cp || 0) < 0;
}

/** Strip characters mdLite would interpret from engine/user-provided fragments (it is HTML-escaped anyway). */
function escapeMd(s) {
  return String(s ?? '').replace(/[*`]/g, '');
}
/** Keep server explanation formatting (it is markdown-lite already). */
function escapeMdKeep(s) {
  return String(s ?? '');
}
