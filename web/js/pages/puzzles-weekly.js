// GrandMentor — "Your weekly set" (#/puzzles/weekly, params.mode = 'weekly'), loaded by puzzles.js.
// A personal set of ~12 puzzles per ISO week: pack puzzles from the user's weakest themes plus
// positions from their own games. API: GET /api/weekly, POST /api/weekly/attempt,
// POST /api/weekly/regenerate (docs/CONTRACT.md "Weekly personal set").
//
// Views: overview (why these themes, progress, start) → solver (reuses PuzzleRunner) → summary.
// Every view owns a disposables() bag; fetches share the page AbortController.

import { h, icon, pageHeader, disposables, loadingBlock, emptyState, toast, confirmDialog } from '../ui.js';
import { api, isAbort } from '../api.js';
import { createMoveInput } from '../components/moveinput.js';
import { t, formatDateIntl, formatNumber } from '../i18n.js';
import { PuzzleRunner, solverKit } from './puzzles.js';
import { ensureWeeklyCss } from '../components/weekly-card.js';

const { createBoard, btn, Stopwatch, sound, sideLabel, themeLabel, themeEmoji } = solverKit;

/** "Week of Oct 5" from a `YYYY-MM-DD` Monday. */
function weekOf(start) {
  const d = new Date(`${start}T12:00:00Z`);
  if (Number.isNaN(d.getTime())) return '';
  return t('weekly.weekOf', { date: formatDateIntl(d, { month: 'long', day: 'numeric' }) });
}

/** Plain-words reason a focus theme was chosen. */
export function reasonText(f) {
  if (f?.reason === 'games' && f.game_misses > 0) return t('weekly.reason.games', { count: f.game_misses });
  if (f?.reason === 'puzzles' && f.puzzle_attempts > 0) return t('weekly.reason.puzzles', { fails: f.puzzle_fails, attempts: f.puzzle_attempts });
  return t('weekly.reason.starter');
}

function firstOpen(set) {
  const i = set.items.findIndex((it) => !it.result);
  return i < 0 ? null : i;
}

export function mountWeekly(root, { bag, signal }) {
  ensureWeeklyCss();
  const page = h('div', { class: 'page page-wide pz-page wk-page' });
  root.appendChild(page);
  let viewBag = null;
  const swap = () => {
    if (viewBag) viewBag.dispose();
    viewBag = disposables();
    page.replaceChildren();
    return viewBag;
  };
  bag.add(() => { if (viewBag) viewBag.dispose(); viewBag = null; });

  let set = null;

  const header = () => pageHeader({
    title: t('weekly.title'), icon: 'calendar',
    subtitle: set?.week_start ? weekOf(set.week_start) : t('weekly.subtitle'),
    breadcrumbs: [{ label: t('puzzles.title'), href: '#/puzzles' }, { label: t('weekly.title') }],
  });

  async function boot() {
    swap();
    page.append(h('div', { class: 'page' }, header(), loadingBlock(t('weekly.loading'))));
    try {
      set = await api.get('/api/weekly', { signal });
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      showError(e.message);
      return;
    }
    if (bag.disposed) return;
    showOverview();
  }

  function showError(msg) {
    swap();
    page.append(h('div', { class: 'page' }, header(), h('div', { class: 'card' },
      emptyState({ icon: 'alert', title: t('weekly.errorTitle'), text: msg, action: { label: t('weekly.retry'), icon: 'refresh', onClick: () => boot() } }))));
  }

  async function newSet() {
    const ok = await confirmDialog({ title: t('weekly.newSetTitle'), message: t('weekly.newSetText'), confirmLabel: t('weekly.newSetConfirm') });
    if (!ok || bag.disposed) return;
    try {
      set = await api.post('/api/weekly/regenerate', {}, { signal });
      if (bag.disposed) return;
      toast(t('weekly.newSetDone'), 'success');
      showOverview();
    } catch (e) {
      if (!isAbort(e) && !bag.disposed) toast(e.message, 'error');
    }
  }

  // ----- Shared bits -----
  function progressBlock() {
    const p = set.progress || { total: 0, done: 0, solved: 0 };
    const pct = p.total ? Math.round((p.done / p.total) * 100) : 0;
    return h('div', { class: 'wk-progress stack-sm' },
      h('div', { class: 'row-sm wk-progress-head' },
        h('span', { class: 'semibold tabular' }, t('weekly.progress', { done: formatNumber(p.done), total: formatNumber(p.total) })),
        h('span', { class: 'dot-sep' }),
        h('span', { class: 'subtle' }, t('weekly.solvedCount', { count: p.solved }))),
      h('div', { class: 'progress', role: 'progressbar', 'aria-valuemin': '0', 'aria-valuemax': String(p.total), 'aria-valuenow': String(p.done) },
        h('div', { class: 'progress-bar', style: { width: `${pct}%` } })));
  }

  function dots(current = -1) {
    return h('ol', { class: 'wk-dots', 'aria-label': t('weekly.title') }, set.items.map((it, i) => {
      const state = i === current ? 'current' : it.result === 'solved' ? 'solved' : it.result === 'failed' ? 'failed' : 'todo';
      return h('li', {
        class: `wk-dot is-${state}${it.kind === 'mistake' ? ' is-own' : ''}`,
        title: t('weekly.itemDot', { n: i + 1, state: t(`weekly.state.${state}`) }),
        'aria-label': t('weekly.itemDot', { n: i + 1, state: t(`weekly.state.${state}`) }),
      });
    }));
  }

  // ----- Overview -----
  function showOverview() {
    if (!set || !Array.isArray(set.items)) { showError(t('weekly.empty')); return; }
    if (!set.items.length) { showError(t('weekly.empty')); return; }
    if (set.finished) { showSummary(); return; }
    swap();
    const focus = Array.isArray(set.focus) ? set.focus : [];
    const next = firstOpen(set);
    const started = (set.progress?.done | 0) > 0;
    const startBtn = btn(started ? t('weekly.continue') : t('weekly.start'), 'play', 'primary', () => showItem(next ?? 0), { cls: 'btn-lg' });
    const newBtn = btn(t('weekly.newSet'), 'refresh', 'ghost', () => newSet());
    page.append(h('div', { class: 'page stack-lg' },
      header(),
      h('section', { class: 'card wk-hero stack' },
        progressBlock(),
        dots(),
        h('div', { class: 'row row-wrap wk-hero-actions' }, startBtn, newBtn)),
      h('section', { class: 'stack' },
        h('div', null,
          h('h2', { class: 'section-title' }, t('weekly.focusTitle')),
          h('p', { class: 'muted' }, t('weekly.focusText'))),
        h('div', { class: 'wk-focus-grid' }, focus.map((f) => {
          const items = set.items.filter((it) => it.theme === f.theme);
          const done = items.filter((it) => it.result).length;
          return h('div', { class: `card wk-focus is-${f.reason || 'starter'}` },
            h('div', { class: 'wk-focus-emoji', 'aria-hidden': 'true' }, themeEmoji(f.theme)),
            h('div', { class: 'wk-focus-main' },
              h('div', { class: 'semibold' }, themeLabel(f.theme)),
              h('p', { class: 'muted text-sm' }, reasonText(f)),
              items.length ? h('div', { class: 'subtle text-sm tabular' }, t('weekly.progress', { done, total: items.length })) : null));
        })))));
    try { startBtn.focus({ preventScroll: true }); } catch { /* focus unsupported */ }
  }

  // ----- Summary -----
  function showSummary() {
    swap();
    const p = set.progress || { total: 0, solved: 0 };
    const byTheme = new Map();
    let own = { total: 0, solved: 0 };
    for (const it of set.items) {
      if (it.kind === 'mistake') {
        own = { total: own.total + 1, solved: own.solved + (it.result === 'solved' ? 1 : 0) };
        continue;
      }
      const key = it.theme || '';
      const cur = byTheme.get(key) || { total: 0, solved: 0 };
      byTheme.set(key, { total: cur.total + 1, solved: cur.solved + (it.result === 'solved' ? 1 : 0) });
    }
    const row = (label, emoji, s) => {
      const pct = s.total ? Math.round((s.solved / s.total) * 100) : 0;
      return h('div', { class: 'wk-sum-row' },
        h('span', { class: 'wk-sum-emoji', 'aria-hidden': 'true' }, emoji),
        h('span', { class: 'wk-sum-label' }, label),
        h('div', { class: 'progress progress-sm wk-sum-bar' }, h('div', { class: 'progress-bar', style: { width: `${pct}%` } })),
        h('span', { class: 'tabular semibold wk-sum-num' }, `${s.solved}/${s.total}`));
    };
    const rows = [...byTheme.entries()].filter(([k]) => k).map(([k, s]) => row(themeLabel(k), themeEmoji(k), s));
    const other = byTheme.get('');
    if (other) rows.push(row(t('weekly.summary.mixed'), '♟', other));
    if (own.total) rows.push(row(t('weekly.summary.ownGames'), '🎯', own));
    page.append(h('div', { class: 'page stack-lg' },
      header(),
      h('section', { class: 'card wk-summary stack' },
        h('div', { class: 'wk-summary-art', 'aria-hidden': 'true' }, '🏆'),
        h('h2', { class: 'wk-summary-title' }, t('weekly.summary.title')),
        h('p', { class: 'muted' }, t('weekly.summary.text', { count: p.solved, total: p.total })),
        dots(),
        h('div', { class: 'stack-sm wk-sum-list' }, h('div', { class: 'stat-label' }, t('weekly.summary.byTheme')), rows),
        h('p', { class: 'subtle text-sm' }, t('weekly.summary.again')),
        h('div', { class: 'row row-wrap wk-hero-actions' },
          btn(t('weekly.newSet'), 'refresh', 'primary', () => newSet()),
          h('a', { class: 'btn btn-secondary', href: '#/insights', html: icon('chart') + `<span>${t('weekly.summary.insights')}</span>` }),
          h('a', { class: 'btn btn-ghost', href: '#/puzzles', html: icon('grid') + `<span>${t('weekly.summary.backToPuzzles')}</span>` })))));
  }

  // ----- Solver -----
  function showItem(index) {
    const item = set.items[index];
    if (!item) { showOverview(); return; }
    const vb = swap();
    const st = { feedback: 'loading', failed: false, recorded: !!item.result, hintMsg: '', saving: false, ratingMsg: '' };

    const slot = h('div', { class: 'board-slot' });
    const turnBar = h('div', { class: 'pz-turnbar' });
    const flipBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-icon', 'aria-label': t('puzzles.solver.flipBoard'), 'data-tooltip': t('puzzles.solver.flipBoard'), html: icon('flip') });
    const timeEl = h('span', { class: 'tabular' }, '0:00');
    const stopwatch = new Stopwatch(timeEl);
    vb.add(() => stopwatch.destroy());
    const toolbar = h('div', { class: 'toolbar pz-toolbar' }, flipBtn, h('div', { class: 'spacer' }),
      h('span', { class: 'row-sm subtle text-sm' }, h('span', { html: icon('timer') }), timeEl));

    const head = h('div', { class: 'stack-sm' });
    const status = h('div', { class: 'pz-status', 'aria-live': 'polite' });
    let wkBoard = null;
    const moveInput = createMoveInput({ board: () => wkBoard });
    vb.add(() => moveInput.destroy());
    const after = h('div', { class: 'stack-sm' });
    const actions = h('div', { class: 'pz-actions' });

    const panel = h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header', html: icon('calendar') + `<span>${t('weekly.title')}</span>` },
        h('div', { class: 'spacer' }),
        h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('grid') + `<span>${t('weekly.overview')}</span>`, onClick: () => showOverview() })),
      h('div', { class: 'panel-body stack' }, head, status, moveInput.el, after),
      h('div', { class: 'panel-footer' }, actions));

    page.append(h('div', { class: 'game-layout no-eval pz-layout', style: '--board-chrome: 124px' },
      h('div', { class: 'game-main' }, turnBar, h('div', { class: 'board-row' }, slot), toolbar),
      h('aside', { class: 'game-panel' }, panel)));

    const board = createBoard(slot, (mv) => runner.handleMove(mv));
    wkBoard = board;
    vb.add(() => board.destroy());
    const runner = new PuzzleRunner(board, {
      onReady() {
        stopwatch.run();
        if (st.feedback === 'loading') st.feedback = 'ready';
        render();
      },
      onCorrect() { st.feedback = 'correct'; st.hintMsg = ''; render(); },
      onWrong() {
        st.feedback = 'wrong';
        st.hintMsg = '';
        markFailed();
        render();
        slot.classList.remove('shake'); void slot.offsetWidth; slot.classList.add('shake');
      },
      onSolved() {
        stopwatch.stop();
        st.feedback = st.failed ? 'solvedLate' : 'solved';
        if (!st.failed) record(true);
        render();
      },
      onSolutionDone() { st.feedback = 'solution'; render(); },
      onError(msg) { stopwatch.stop(); st.feedback = 'error'; st.hintMsg = msg; markFailed(); render(); },
    });
    vb.add(() => runner.destroy());

    function markFailed() {
      if (st.failed) return;
      st.failed = true;
      record(false);
    }

    async function record(solved) {
      if (st.recorded) return;
      st.recorded = true;
      st.saving = true;
      try {
        const res = await api.post('/api/weekly/attempt', { set_id: set.set_id, index: item.index, solved, time_ms: Math.round(stopwatch.ms) }, { signal });
        if (bag.disposed || !res) return;
        if (res.item) set.items[index] = res.item;
        if (res.progress) set.progress = res.progress;
        set.finished = !!res.finished;
        if (res.rating && Number.isFinite(res.rating.rating)) {
          const d = Math.round(res.rating.delta || 0);
          st.ratingMsg = t('weekly.ratingChange', { rating: formatNumber(Math.round(res.rating.rating)), delta: d > 0 ? `+${d}` : String(d) });
        }
        if (res.finished) sound('gameEnd');
      } catch (e) {
        if (isAbort(e) || bag.disposed) return;
        toast(t('puzzles.errors.saveFailed', { message: e.message }), 'warning');
      } finally {
        st.saving = false;
      }
      if (!vb.disposed) render();
    }

    function nextIndex() {
      for (let k = 1; k <= set.items.length; k++) {
        const j = (index + k) % set.items.length;
        if (!set.items[j].result) return j;
      }
      return null;
    }

    function goNext() {
      const j = nextIndex();
      if (j == null) { set.finished = true; showSummary(); } else showItem(j);
    }

    function renderHead() {
      const game = item.game || null;
      const opp = String(game?.opponent || '').trim();
      head.replaceChildren(...[
        h('div', { class: 'row-sm wk-item-head' },
          h('span', { class: 'semibold tabular' }, t('weekly.itemOf', { n: index + 1, total: set.items.length })),
          h('div', { class: 'spacer' }),
          item.kind === 'mistake' ? h('span', { class: 'badge badge-warning' }, t('weekly.ownTag')) : null,
          item.theme ? h('span', { class: 'badge badge-info' }, `${themeEmoji(item.theme)} ${themeLabel(item.theme)}`) : null),
        dots(index),
        item.kind === 'mistake'
          ? h('div', { class: 'wk-own subtle text-sm' },
            h('span', { html: icon(game?.game_id ? 'swords' : 'target') }),
            h('span', null, opp ? t('weekly.fromGameVs', { opponent: opp }) : t('weekly.fromYourGame')))
          : null,
      ].filter(Boolean));
    }

    function render() {
      const color = runner.userColor;
      const done = ['solved', 'solvedLate', 'solution', 'error'].includes(st.feedback);
      renderHead();

      let bar;
      if (st.feedback === 'loading') bar = [h('span', { class: 'spinner' }), h('span', null, t('weekly.bar.loading'))];
      else if (st.feedback === 'error') bar = [h('span', { html: icon('alert') }), h('span', null, t('puzzles.solver.bar.error'))];
      else if (st.feedback === 'solved' || st.feedback === 'solvedLate') bar = [h('span', { class: 'pz-turn-ok', html: icon('check') }), h('span', null, t('weekly.bar.solved'))];
      else if (st.feedback === 'solution') bar = [h('span', { html: icon('eye') }), h('span', null, t('puzzles.solver.bar.solution'))];
      else bar = [h('span', { class: `pz-side pz-side-${color}` }), h('span', null, t('weekly.bar.find', { side: sideLabel(color) }))];
      turnBar.className = `pz-turnbar state-${st.feedback}`;
      turnBar.replaceChildren(...bar);

      let kind = 'neutral', ic = 'target', hd = '', sub = '';
      const readySub = item.kind === 'mistake' && item.game?.played_san
        ? t('weekly.ownHint', { played: item.game.played_san })
        : item.theme ? t('weekly.status.readySub', { theme: themeLabel(item.theme) }) : t('weekly.status.readySubAny');
      switch (st.feedback) {
        case 'loading': ic = 'clock'; hd = t('puzzles.solver.status.loadingHead'); sub = t('weekly.loading'); break;
        case 'ready': hd = t('weekly.status.readyHead'); sub = readySub; break;
        case 'correct': kind = 'good'; ic = 'check-circle'; hd = t('puzzles.solver.status.correctHead'); sub = t('puzzles.solver.status.correctSub'); break;
        case 'wrong': kind = 'bad'; ic = 'x-circle'; hd = t('puzzles.solver.status.wrongHead'); sub = t('weekly.status.wrongSub'); break;
        case 'solved': kind = 'good'; ic = 'trophy'; hd = t('weekly.status.solvedHead'); sub = t('weekly.status.solvedSub'); break;
        case 'solvedLate': kind = 'good'; ic = 'check-circle'; hd = t('weekly.status.lateHead'); sub = t('weekly.status.lateSub'); break;
        case 'solution': ic = 'eye'; hd = t('weekly.status.solutionHead'); sub = t('weekly.status.solutionSub'); break;
        case 'error': kind = 'bad'; ic = 'alert'; hd = t('puzzles.solver.status.errorHead'); sub = st.hintMsg || t('puzzles.solver.status.errorSub'); break;
        default: break;
      }
      if (st.hintMsg && st.feedback === 'ready') sub = st.hintMsg;
      status.className = `pz-status pz-status-${kind}`;
      const key = `${ic}|${hd}|${sub}`;
      if (status.dataset.key !== key) {
        status.dataset.key = key;
        status.replaceChildren(
          h('div', { class: 'pz-status-icon', html: icon(ic) }),
          h('div', { class: 'pz-status-text' }, h('div', { class: 'pz-status-head' }, hd), h('div', { class: 'pz-status-sub' }, sub)));
      }

      const extra = [];
      if (done && item.kind === 'mistake' && item.game?.best_san) extra.push(h('div', { class: 'wk-best' }, h('span', { html: icon('star') }), h('span', null, t('weekly.bestWas', { best: item.game.best_san }))));
      if (done && item.kind === 'mistake' && item.game?.game_id) extra.push(h('a', { class: 'btn btn-ghost btn-sm', href: `#/review/${encodeURIComponent(item.game.game_id)}`, html: icon('analysis') + `<span>${t('puzzles.mistakes.openReview')}</span>` }));
      if (st.ratingMsg) extra.push(h('div', { class: 'subtle text-sm tabular' }, st.ratingMsg));
      if (done || st.saving) extra.push(progressBlock());
      after.replaceChildren(...extra);

      const list = [];
      if (st.feedback === 'loading') {
        list.push(btn(t('puzzles.solver.buttons.hint'), 'hint', 'secondary', null, { cls: 'pz-grow' }), btn(t('puzzles.solver.buttons.solution'), 'eye', 'ghost', null));
        list.forEach((b) => { b.disabled = true; });
      } else if (done) {
        if (st.feedback !== 'error') list.push(btn(t('puzzles.solver.buttons.retry'), 'refresh', 'ghost', () => retry(), { key: 'r' }));
        const last = nextIndex() == null;
        list.push(btn(last ? t('weekly.finish') : t('weekly.next'), last ? 'trophy' : 'arrow-right', 'primary', () => goNext(), { key: 'n', cls: 'pz-grow' }));
      } else {
        list.push(btn(t('puzzles.solver.buttons.hint'), 'hint', 'secondary', () => doHint(), { key: 'h', cls: 'pz-grow' }));
        list.push(btn(t('puzzles.solver.buttons.solution'), 'eye', 'ghost', () => doSolution()));
      }
      actions.replaceChildren(...list);
    }

    function doHint() {
      const stage = runner.hint();
      if (!stage) return;
      markFailed();
      st.hintMsg = stage === 1 ? t('puzzles.solver.hintPiece', { piece: runner.hintPieceName() }) : t('puzzles.solver.hintArrow');
      render();
    }

    function doSolution() {
      markFailed();
      stopwatch.stop();
      st.hintMsg = '';
      runner.showSolution();
      st.feedback = 'solution';
      render();
    }

    function retry() {
      st.feedback = 'loading';
      st.hintMsg = '';
      st.failed = true; // a retry never counts
      render();
      runner.retry();
    }

    vb.on(flipBtn, 'click', () => board.flip());
    vb.on(window, 'keydown', (e) => {
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
      const tg = e.target;
      if (tg && (tg.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(tg.tagName))) return;
      if (document.querySelector('.modal-backdrop')) return;
      const k = e.key.toLowerCase();
      const done = ['solved', 'solvedLate', 'solution', 'error'].includes(st.feedback);
      if (k === 'f') { board.flip(); e.preventDefault(); }
      else if (k === 'h' && !done && st.feedback !== 'loading') { doHint(); e.preventDefault(); }
      else if (k === 'r' && done && st.feedback !== 'error') { retry(); e.preventDefault(); }
      else if ((k === 'n' || k === 'arrowright') && done) { e.preventDefault(); goNext(); }
    });

    // An item already answered (e.g. re-opened from the overview) is practice only.
    if (item.result) st.failed = true;
    render();
    if (!runner.load(item.puzzle)) render();
  }

  boot();
}
