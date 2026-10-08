// Repertoire page (#/repertoire, #/repertoire/:side). Owned by the Repertoire feature; see
// docs/CONTRACT.md "Repertoire".
//
//   Build  — a move tree per side. Play moves on the board to add them (one move of yours per
//            position, any number of opponent replies), add notes, delete branches, or start
//            from a ready-made starter repertoire.
//   Drill  — spaced repetition: a client-side "bot" plays the opponent's moves along a line
//            picked by the server (due and weak branches first); you answer from memory.
//            Wrong answers show the right move with an arrow and come back later.
//   Recent games — where your saved games left your repertoire.
//
// Query: ?drill=1 starts the drill right away, ?node=<id> selects a move in the tree.
// Also exports openAddToRepertoire() used by the Openings page.

import {
  h, icon, disposables, pageHeader, emptyState, loadingBlock, skeleton, escapeHtml, formatSan,
  formatRelative, toast, modal, confirmDialog,
} from '../ui.js';
import { api, qs, isAbort } from '../api.js';
import { getSetting, pieceUrl } from '../settings.js';
import { Board } from '../components/board.js';
import {
  ensureLearnCss, START_FEN, fenKey, timerSet, confetti, sfx, setFeedback, readStore, writeStore,
} from './learn.js';
import { t } from '../i18n.js';

export const title = () => t('nav.routes.repertoire');

const SIDE_KEY = 'gm.repertoire.side.v1';
const OPP_DELAY_MS = 520;
const NEXT_DELAY_MS = 700;
const MAX_RETRY = 20;
const DAY_S = 86400;

function ensureCss() {
  if (document.querySelector('link[data-page-css="repertoire"]')) return;
  const link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = '/css/repertoire.css';
  link.dataset.pageCss = 'repertoire';
  document.head.appendChild(link);
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------
const sanOf = (san) => formatSan(san, getSetting('moveNotation'));
const otherSide = (s) => (s === 'white' ? 'black' : 'white');
const sideName = (s) => (s === 'black' ? t('repertoire.side.black') : t('repertoire.side.white'));
/** Side name used inside a sentence ("your Black repertoire" / "tu repertorio con negras"). */
const sideInline = (s) => (s === 'black' ? t('repertoire.side.blackInline') : t('repertoire.side.whiteInline'));

/** "7." for White's 7th move, "7…" for Black's. */
function moveNo(ply) {
  const n = Math.ceil(ply / 2);
  return ply % 2 === 1 ? `${n}.` : `${n}…`;
}
function moveLabel(ply, san) { return `${moveNo(ply)} ${sanOf(san)}`; }

/** "1. e4 e5 2. Nf3" from a list of {ply, san}. */
function numbered(nodes) {
  const out = [];
  nodes.forEach((n, i) => {
    if (n.ply % 2 === 1) out.push(`${Math.ceil(n.ply / 2)}.`);
    else if (i === 0) out.push(`${Math.ceil(n.ply / 2)}…`);
    out.push(sanOf(n.san));
  });
  return out.join(' ');
}

function nowS() { return Math.floor(Date.now() / 1000); }

function dueText(n) {
  if (!n || !n.mine) return '';
  if (n.reps === 0 && n.lapses === 0) return t('repertoire.card.new');
  const left = n.due - nowS();
  if (left <= 0) return t('repertoire.card.dueNow');
  if (left < DAY_S) return t('repertoire.card.dueToday');
  const days = Math.round(left / DAY_S);
  return t('repertoire.card.dueIn', { count: days });
}

function sidePill(side) {
  return h('span', { class: `rep-side-pill ${side}` }, sideName(side));
}

function isSameUci(a, b) { return String(a || '').toLowerCase() === String(b || '').toLowerCase(); }

/** Build lookup maps for a flat node list. */
function indexTree(nodes) {
  const byId = new Map();
  const kids = new Map();
  for (const n of nodes) {
    byId.set(n.id, n);
    if (!kids.has(n.parent_id)) kids.set(n.parent_id, []);
    kids.get(n.parent_id).push(n);
  }
  return { byId, kids };
}

// ===========================================================================
// Shared: "Add to my repertoire" dialog (used by the Openings page)
// ===========================================================================

/**
 * Ask which side and add a line (UCI from the initial position) to the repertoire.
 * @param {{ucis: string[], sans?: string[], name?: string, side?: 'white'|'black'}} opts
 */
export function openAddToRepertoire({ ucis, sans = [], name = '', side = 'white' } = {}) {
  ensureCss();
  const line = Array.isArray(ucis) ? ucis.slice(0, 80) : [];
  if (!line.length) { toast(t('repertoire.add.noMoves'), 'warning'); return null; }
  let chosen = side === 'black' ? 'black' : 'white';
  const seg = h('div', { class: 'segmented block', role: 'group', 'aria-label': t('repertoire.add.sideAria') });
  const renderSeg = () => seg.replaceChildren(...['white', 'black'].map((s) => h('button', {
    type: 'button', class: chosen === s ? 'active' : '', 'aria-pressed': chosen === s ? 'true' : 'false',
    onClick: () => { chosen = s; renderSeg(); },
  }, h('span', { class: `rep-dot ${s}`, 'aria-hidden': 'true' }), sideName(s))));
  renderSeg();
  const preview = sans.length
    ? h('div', { class: 'rep-line-preview' }, numbered(sans.map((s, i) => ({ ply: i + 1, san: s }))))
    : null;
  const body = h('div', { class: 'stack' },
    h('p', { class: 'muted' }, name ? t('repertoire.add.intro', { name }) : t('repertoire.add.introLine')),
    seg, preview,
    h('p', { class: 'text-sm subtle' }, t('repertoire.add.hint')));

  async function submit(replace) {
    return api.post('/api/repertoire/lines', { side: chosen, moves: line, replace });
  }
  return modal({
    title: t('repertoire.add.title'),
    body,
    actions: [
      { label: t('common.cancel'), kind: 'ghost' },
      {
        label: t('repertoire.add.confirm'), kind: 'primary', icon: 'plus',
        onClick: async (close) => {
          let res = await submit(false);
          if (res && res.conflict) {
            const c = res.conflict;
            const ok = await confirmDialog({
              title: t('repertoire.conflict.title'),
              message: t('repertoire.conflict.text', { existing: moveLabel(c.ply, c.existing_san), next: moveLabel(c.ply, c.new_san) }),
              confirmLabel: t('repertoire.conflict.replace'),
            });
            if (!ok) return false;
            res = await submit(true);
          }
          close();
          const added = res ? res.added : 0;
          toast(added > 0
            ? t('repertoire.add.done', { count: added, side: sideInline(chosen) })
            : t('repertoire.add.already', { side: sideInline(chosen) }), 'success');
          return true;
        },
      },
    ],
  });
}

// ===========================================================================
// Page
// ===========================================================================
export async function mount(root, { params = {}, query = {} } = {}) {
  ensureLearnCss();
  ensureCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const signal = ctrl.signal;
  const timers = timerSet();
  bag.add(() => timers.clear());

  const saved = readStore(SIDE_KEY, {});
  let side = params.side === 'black' || params.side === 'white' ? params.side : (saved.side === 'black' ? 'black' : 'white');
  writeStore(SIDE_KEY, { side });

  // ------------------------------------------------------------------ state
  let tree = { nodes: [], stats: {}, max_nodes: 5000 };
  let idx = indexTree([]);
  let summary = null;
  let starters = [];
  let selected = 0;
  let mode = 'build';
  let modeBag = null;        // disposables for the current mode (board, listeners)
  let board = null;
  let busy = false;
  // Build-mode view references (declared up here: mount() renders before the helpers below).
  let chipEls = new Map();
  let treeBody = null;
  let pathBar = null;
  let details = null;
  let feedback = null;
  let navBtns = {};
  let clearBtn = null;

  // ------------------------------------------------------------------ skeleton
  const page = h('div', { class: 'page rep-page' });
  root.appendChild(page);
  const drillBtn = h('button', { class: 'btn btn-primary', type: 'button', onClick: () => startDrill(false) });
  const header = pageHeader({
    title: t('repertoire.title'), icon: 'book', subtitle: t('repertoire.subtitle'),
    actions: [
      h('a', { class: 'btn btn-secondary', href: '#/openings', html: icon('openings') + `<span>${escapeHtml(t('repertoire.browseOpenings'))}</span>` }),
      drillBtn,
    ],
  });
  const sideBar = h('div', { class: 'rep-sidebar-row' });
  const statsRow = h('div', { class: 'rep-stats' });
  const startersTop = h('section', { class: 'rep-section' });
  const workspace = h('div', { class: 'rep-workspace' });
  const startersBottom = h('section', { class: 'rep-section' });
  const recent = h('section', { class: 'rep-section' });
  page.append(header, sideBar, statsRow, startersTop, workspace, recent, startersBottom);
  workspace.append(loadingBlock(t('repertoire.loading')));
  bag.add(() => { if (modeBag) modeBag.dispose(); });

  // ------------------------------------------------------------------ data
  async function loadTree() {
    tree = await api.get('/api/repertoire' + qs({ side }), { signal });
    idx = indexTree(tree.nodes || []);
    if (selected && !idx.byId.has(selected)) selected = 0;
  }
  async function loadSummary() {
    try { summary = await api.get('/api/repertoire/summary', { signal }); } catch (e) { if (isAbort(e)) throw e; summary = null; }
  }
  async function loadStarters() {
    try { starters = await api.get('/api/repertoire/starters', { signal }); } catch (e) { if (isAbort(e)) throw e; starters = []; }
  }

  try {
    await Promise.all([loadTree(), loadSummary(), loadStarters()]);
  } catch (e) {
    if (isAbort(e) || bag.disposed) return () => bag.dispose();
    workspace.replaceChildren(emptyState({ icon: 'alert', title: t('repertoire.error.title'), text: e && e.message, action: { label: t('repertoire.error.retry'), icon: 'refresh', onClick: () => location.reload() } }));
    return () => bag.dispose();
  }
  if (bag.disposed) return () => bag.dispose();

  const wanted = Number(query.node);
  if (Number.isFinite(wanted) && idx.byId.has(wanted)) selected = wanted;

  renderChrome();
  renderBuild();
  loadRecent();
  if (query.drill === '1' || query.drill === 'true') startDrill(false);

  return () => bag.dispose();

  // ==================================================================
  // Chrome: side switch, stats, starters
  // ==================================================================
  function sideStats(s) { return (summary && summary[s]) || (s === side ? tree.stats : null) || {}; }

  function renderChrome() {
    const due = sideStats(side).due || 0;
    drillBtn.innerHTML = icon('target') + `<span>${escapeHtml(due > 0 ? t('repertoire.drill.startDue', { count: due }) : t('repertoire.drill.start'))}</span>`;
    drillBtn.disabled = !(sideStats(side).cards > 0);

    sideBar.replaceChildren(h('div', { class: 'segmented rep-side-switch', role: 'tablist', 'aria-label': t('repertoire.side.aria') },
      ['white', 'black'].map((s) => {
        const st = sideStats(s);
        return h('a', {
          class: ['seg', s === side && 'active'], role: 'tab', href: `#/repertoire/${s}`,
          'aria-selected': s === side ? 'true' : 'false',
        },
        h('span', { class: `rep-dot ${s}`, 'aria-hidden': 'true' }),
        h('span', null, sideName(s)),
        h('span', { class: 'rep-seg-count' }, t('repertoire.linesCount', { count: st.lines || 0 })),
        st.due ? h('span', { class: 'rep-due-dot', title: t('repertoire.dueCount', { count: st.due }) }, String(st.due)) : null);
      })));

    const st = tree.stats || {};
    const tile = (label, value, cls, ic) => h('div', { class: ['card card-sm rep-stat', cls] },
      h('div', { class: 'rep-stat-icon', html: icon(ic) }),
      h('div', { class: 'stat' }, h('div', { class: 'stat-label' }, label), h('div', { class: 'stat-value' }, String(value ?? 0))));
    statsRow.replaceChildren(
      tile(t('repertoire.stats.lines'), st.lines, '', 'openings'),
      tile(t('repertoire.stats.moves'), st.cards, '', 'knight'),
      tile(t('repertoire.stats.due'), st.due, st.due ? 'is-due' : '', 'clock'),
      tile(t('repertoire.stats.learned'), st.learned, 'is-learned', 'check-circle'));

    const mine = starters.filter((s) => s.side === side);
    const empty = !tree.nodes.length;
    startersTop.replaceChildren();
    startersBottom.replaceChildren();
    const target = empty ? startersTop : startersBottom;
    if (mine.length) {
      target.append(
        h('h2', { class: 'section-title' }, empty ? t('repertoire.starters.titleEmpty') : t('repertoire.starters.title')),
        h('p', { class: 'muted rep-section-sub' }, empty ? t('repertoire.starters.subEmpty', { side: sideInline(side) }) : t('repertoire.starters.sub')),
        h('div', { class: 'rep-starters' }, mine.map(starterCard)));
    }
  }

  function starterCard(s) {
    const names = (s.openings || []).map((o) => o.name);
    const shown = names.slice(0, 4);
    const more = names.length - shown.length;
    const btn = h('button', { class: 'btn btn-secondary btn-sm', type: 'button', html: icon('plus') + `<span>${escapeHtml(t('repertoire.starters.add'))}</span>` });
    btn.addEventListener('click', async () => {
      if (busy) return;
      busy = true;
      btn.classList.add('loading');
      try {
        const r = await api.post(`/api/repertoire/starters/${encodeURIComponent(s.id)}`, {}, { signal });
        if (bag.disposed) return;
        toast(r.added > 0 ? t('repertoire.starters.added', { count: r.lines }) : t('repertoire.starters.nothing'), r.added > 0 ? 'success' : 'info');
        if (r.skipped > 0) toast(t('repertoire.starters.skipped', { count: r.skipped }), 'info');
        await refreshAll();
      } catch (e) {
        if (!isAbort(e)) toast(e.message, 'error');
      } finally {
        busy = false;
        btn.classList.remove('loading');
      }
    });
    return h('div', { class: `card rep-starter ${s.side}` },
      h('div', { class: 'rep-starter-head' },
        h('div', { class: 'rep-starter-emoji', 'aria-hidden': 'true' }, h('img', { src: pieceUrl(s.side === 'white' ? 'wN' : 'bN'), alt: '' })),
        h('div', { style: 'min-width:0' },
          h('div', { class: 'rep-starter-title' }, t(`repertoire.starters.packs.${s.id}.title`)),
          h('div', { class: 'text-sm muted' }, t(`repertoire.starters.packs.${s.id}.blurb`)))),
      h('div', { class: 'chip-row rep-starter-chips' },
        shown.map((n) => h('span', { class: 'badge' }, n)),
        more > 0 ? h('span', { class: 'badge badge-info' }, t('repertoire.starters.more', { count: more })) : null),
      h('div', { class: 'rep-starter-foot' },
        h('span', { class: 'text-xs subtle' }, t('repertoire.linesCount', { count: s.lines })), btn));
  }

  async function refreshAll({ keepSelection = true } = {}) {
    const prev = selected;
    await Promise.all([loadTree(), loadSummary()]);
    if (bag.disposed) return;
    if (keepSelection && idx.byId.has(prev)) selected = prev;
    renderChrome();
    if (mode !== 'build') return;
    if (board && treeBody) {
      // Keep the board (no flicker); refresh the tree and the selection.
      renderTree();
      if (clearBtn) clearBtn.hidden = !tree.nodes.length;
      select(selected, { animate: false });
    } else {
      renderBuild();
    }
  }

  // ==================================================================
  // Build mode
  // ==================================================================
  function newModeBag() {
    if (modeBag) modeBag.dispose();
    modeBag = disposables();
    timers.clear();
    board = null;
    return modeBag;
  }

  function fenOf(id) { return id && idx.byId.has(id) ? idx.byId.get(id).fen : START_FEN; }
  function childrenOf(id) { return idx.kids.get(id) || []; }
  function pathTo(id) {
    const out = [];
    let cur = idx.byId.get(id);
    let guard = 0;
    while (cur && guard++ < 200) { out.push(cur); cur = idx.byId.get(cur.parent_id); }
    return out.reverse();
  }
  /** Ply of the move to be played next at node `id` (1 at the start). */
  function nextPly(id) { const n = idx.byId.get(id); return n ? n.ply + 1 : 1; }
  function isMyPly(ply) { return (ply % 2 === 1) === (side === 'white'); }

  function renderBuild() {
    mode = 'build';
    const mb = newModeBag();
    chipEls = new Map();
    const boardSlot = h('div', { class: 'board-slot' });
    pathBar = h('div', { class: 'rep-pathbar', 'aria-live': 'polite' });
    const tb = (ic, label, onClick) => h('button', { class: 'btn btn-ghost btn-icon', type: 'button', 'aria-label': label, 'data-tooltip': label, html: icon(ic), onClick });
    navBtns = {
      first: tb('first', t('repertoire.nav.start'), () => select(0)),
      prev: tb('chevron-left', t('repertoire.nav.back'), () => { const n = idx.byId.get(selected); select(n ? n.parent_id : 0); }),
      next: tb('chevron-right', t('repertoire.nav.forward'), () => { const k = childrenOf(selected)[0]; if (k) select(k.id); }),
      flip: tb('flip', t('repertoire.nav.flip'), () => board && board.flip()),
    };
    const toolbar = h('div', { class: 'toolbar' }, navBtns.first, navBtns.prev, navBtns.next, navBtns.flip);

    treeBody = h('div', { class: 'panel-body rep-tree', role: 'tree', 'aria-label': t('repertoire.tree.aria') });
    clearBtn = h('button', {
      class: 'btn btn-ghost btn-icon btn-sm rep-clear', type: 'button', 'aria-label': t('repertoire.tree.clear'), 'data-tooltip': t('repertoire.tree.clear'), html: icon('trash'),
      onClick: clearSide,
    });
    const treePanel = h('div', { class: 'panel grow rep-tree-panel' },
      h('div', { class: 'panel-header' }, h('span', { html: icon('openings'), style: 'display:contents' }),
        h('span', null, t('repertoire.tree.title', { side: sideInline(side) })), h('span', { class: 'spacer' }),
        clearBtn),
      treeBody);
    clearBtn.hidden = !tree.nodes.length;
    details = h('div', { class: 'card card-sm rep-details' });
    feedback = h('div', { class: 'lrn-feedback' });

    const layout = h('div', { class: 'game-layout no-eval rep-layout rep-build' },
      h('div', { class: 'game-main' }, pathBar, h('div', { class: 'board-row' }, boardSlot), toolbar),
      h('aside', { class: 'game-panel' }, treePanel, details));
    workspace.replaceChildren(layout);

    board = new Board(boardSlot, {
      fen: fenOf(selected), orientation: side, interactive: true, movableColor: 'both',
      onMove: (mv) => onBuildMove(mv),
    });
    mb.add(() => { if (board) board.destroy(); board = null; });
    mb.on(window, 'keydown', (e) => {
      if (mode !== 'build' || e.defaultPrevented) return;
      const tag = (e.target && e.target.tagName) || '';
      if (/INPUT|TEXTAREA|SELECT/.test(tag) || (e.target && e.target.isContentEditable)) return;
      if (e.key === 'ArrowLeft') { e.preventDefault(); navBtns.prev.click(); }
      else if (e.key === 'ArrowRight') { e.preventDefault(); navBtns.next.click(); }
      else if (e.key === 'Home') { e.preventDefault(); select(0); }
    });

    renderTree();
    select(selected, { animate: false, scroll: true });
  }

  // ---------------------------------------------------------- tree view
  function chip(n, withNumber) {
    const el = h('button', {
      type: 'button', role: 'treeitem',
      class: ['rep-move', n.mine ? 'mine' : 'theirs', n.is_due && (n.reps > 0 || n.lapses > 0) && 'due', n.note && 'has-note', n.id === selected && 'selected'],
      'aria-selected': n.id === selected ? 'true' : null,
      'aria-label':`${moveLabel(n.ply, n.san)}${n.mine ? ' · ' + t('repertoire.tree.yourMove') : ''}`,
      onClick: () => select(n.id),
    },
    withNumber || n.ply % 2 === 1 ? h('span', { class: 'rep-no' }, moveNo(n.ply)) : null,
    h('span', { class: 'rep-san' }, sanOf(n.san)),
    n.note ? h('span', { class: 'rep-note-ic', html: icon('edit', { size: 12 }), 'aria-hidden': 'true' }) : null);
    chipEls.set(n.id, el);
    return el;
  }

  /** A chain of single moves starting with `first` (rendered with its number), then branches. */
  function renderLine(first) {
    const wrap = h('div', { class: 'rep-line' });
    const row = h('div', { class: 'rep-moves' });
    wrap.append(row);
    row.append(chip(first, true));
    let cur = first.id;
    let afterBranch = false;
    for (let guard = 0; guard < 200; guard++) {
      const kids = childrenOf(cur);
      if (!kids.length) break;
      if (kids.length === 1) {
        row.append(chip(kids[0], afterBranch));
        afterBranch = false;
        cur = kids[0].id;
        continue;
      }
      wrap.append(renderBranches(kids));
      break;
    }
    return wrap;
  }

  function renderBranches(kids) {
    const box = h('div', { class: 'rep-branches' });
    for (const k of kids) box.append(h('div', { class: ['rep-branch', k.mine ? 'mine' : 'theirs'] }, renderLine(k)));
    return box;
  }

  function renderTree() {
    chipEls = new Map();
    const roots = childrenOf(0);
    if (!roots.length) {
      treeBody.replaceChildren(h('div', { class: 'rep-tree-empty' },
        h('div', { class: 'rep-tree-empty-icon', 'aria-hidden': 'true' }, side === 'white' ? '♘' : '♞'),
        h('div', { class: 'semibold' }, t('repertoire.tree.emptyTitle')),
        h('p', { class: 'text-sm muted' }, t(side === 'white' ? 'repertoire.tree.emptyWhite' : 'repertoire.tree.emptyBlack'))));
      return;
    }
    const legend = h('div', { class: 'rep-legend text-xs subtle' },
      h('span', { class: 'rep-legend-item' }, h('i', { class: 'rep-swatch mine' }), t('repertoire.tree.legendMine')),
      h('span', { class: 'rep-legend-item' }, h('i', { class: 'rep-swatch theirs' }), t('repertoire.tree.legendTheirs')),
      h('span', { class: 'rep-legend-item' }, h('i', { class: 'rep-swatch due' }), t('repertoire.tree.legendDue')));
    const body = roots.length === 1 ? renderLine(roots[0]) : renderBranches(roots);
    treeBody.replaceChildren(legend, body);
  }

  // ---------------------------------------------------------- selection
  function select(id, { animate = true, scroll = true } = {}) {
    if (id && !idx.byId.has(id)) id = 0;
    const prev = chipEls.get(selected);
    if (prev) { prev.classList.remove('selected'); prev.removeAttribute('aria-selected'); }
    selected = id;
    const el = chipEls.get(id);
    if (el) {
      el.classList.add('selected');
      el.setAttribute('aria-selected', 'true');
      if (scroll && treeBody) {
        // Keep the chip visible inside the scrolling panel only (never scroll the page).
        const pr = treeBody.getBoundingClientRect();
        const er = el.getBoundingClientRect();
        if (er.top < pr.top || er.bottom > pr.bottom) treeBody.scrollTop += er.top - pr.top - pr.height / 3;
      }
    }
    const n = idx.byId.get(id);
    if (board) {
      board.setPosition(fenOf(id), { animate, lastMove: n ? [n.uci.slice(0, 2), n.uci.slice(2, 4)] : null, sound: false });
      board.clearArrows();
      // Show your prepared move (green) or the opponent replies you have (blue).
      const kids = childrenOf(id);
      board.setArrows(kids.slice(0, 8).map((k) => ({ from: k.uci.slice(0, 2), to: k.uci.slice(2, 4), color: k.mine ? 'green' : 'blue' })));
    }
    renderPath();
    renderDetails();
    if (navBtns.prev) {
      navBtns.prev.disabled = !id;
      navBtns.first.disabled = !id;
      navBtns.next.disabled = !childrenOf(id).length;
    }
  }

  function renderPath() {
    if (!pathBar) return;
    const path = pathTo(selected);
    const items = [h('button', { type: 'button', class: ['rep-path-item', !selected && 'current'], onClick: () => select(0) }, t('repertoire.nav.startShort'))];
    path.forEach((n, i) => {
      items.push(h('button', {
        type: 'button', class: ['rep-path-item', n.id === selected && 'current', n.mine && 'mine'], onClick: () => select(n.id),
      }, (n.ply % 2 === 1 || i === 0 ? moveNo(n.ply) + ' ' : '') + sanOf(n.san)));
    });
    pathBar.replaceChildren(...items);
    pathBar.scrollLeft = pathBar.scrollWidth;
  }

  function renderDetails() {
    if (!details) return;
    const n = idx.byId.get(selected);
    const ply = nextPly(selected);
    const kids = childrenOf(selected);
    const yourTurn = isMyPly(ply);
    const prompt = yourTurn
      ? (kids.length ? t('repertoire.details.yourChoice', { move: moveLabel(kids[0].ply, kids[0].san) }) : t('repertoire.details.playYours'))
      : (kids.length ? t('repertoire.details.replies', { count: kids.length }) : t('repertoire.details.playTheirs'));
    const promptEl = h('div', { class: ['rep-prompt', yourTurn ? 'mine' : 'theirs'] },
      h('span', { html: icon(yourTurn ? 'knight' : 'users') , style: 'display:contents' }), h('span', null, prompt));

    if (!n) {
      details.replaceChildren(
        h('div', { class: 'rep-details-head' }, h('div', { class: 'semibold' }, t('repertoire.details.start')), sidePill(side)),
        promptEl, feedback);
      return;
    }
    const note = h('textarea', { class: 'textarea rep-note', rows: 2, maxlength: 500, placeholder: t('repertoire.details.notePlaceholder'), 'aria-label': t('repertoire.details.noteAria') });
    note.value = n.note || '';
    const saveNote = async () => {
      const v = note.value.trim();
      if (v === (n.note || '')) return;
      try {
        const upd = await api.put(`/api/repertoire/nodes/${n.id}`, { note: v }, { signal });
        n.note = upd.note;
        if (bag.disposed || mode !== 'build') return;
        // Re-render so the chip gets (or loses) its note icon.
        renderTree();
        setFeedback(feedback, 'good', escapeHtml(t('repertoire.details.noteSaved')));
        timers.later(() => setFeedback(feedback, null, ''), 1600);
      } catch (e) { if (!isAbort(e)) toast(e.message, 'error'); }
    };
    note.addEventListener('blur', saveNote);
    const del = h('button', { class: 'btn btn-ghost btn-sm rep-delete', type: 'button', html: icon('trash') + `<span>${escapeHtml(t('repertoire.details.delete'))}</span>`, onClick: () => deleteNode(n) });
    const badge = n.mine
      ? h('span', { class: ['badge', n.is_due ? 'badge-warning' : 'badge-primary'] }, dueText(n))
      : h('span', { class: 'badge' }, t('repertoire.details.opponentMove'));
    details.replaceChildren(
      h('div', { class: 'rep-details-head' },
        h('div', { class: 'rep-details-move' }, moveLabel(n.ply, n.san)),
        h('span', { class: ['rep-role', n.mine ? 'mine' : 'theirs'] }, n.mine ? t('repertoire.details.yourMove') : t('repertoire.details.theirMove')),
        h('span', { class: 'spacer' }), badge),
      note,
      h('div', { class: 'row-sm rep-details-actions' }, promptEl, h('span', { class: 'spacer' }), del),
      feedback);
  }

  // ---------------------------------------------------------- editing
  async function onBuildMove(mv) {
    if (busy) return false;
    const kids = childrenOf(selected);
    const existing = kids.find((k) => isSameUci(k.uci, mv.uci) || fenKey(k.fen) === fenKey(mv.fen));
    if (existing) { timers.later(() => select(existing.id, { animate: false }), 0); return true; }
    if (tree.nodes.length >= (tree.max_nodes || 5000)) {
      toast(t('repertoire.full'), 'warning');
      return false;
    }
    return addMove(selected, mv.uci, false);
  }

  async function addMove(parentId, uci, replace) {
    busy = true;
    try {
      let res = await api.post('/api/repertoire/nodes', { side, parent_id: parentId, uci, replace }, { signal });
      if (bag.disposed) return false;
      if (res && res.conflict) {
        const c = res.conflict;
        const ok = await confirmDialog({
          title: t('repertoire.conflict.title'),
          message: t('repertoire.conflict.text', { existing: moveLabel(c.ply, c.existing_san), next: moveLabel(c.ply, c.new_san) }),
          confirmLabel: t('repertoire.conflict.replace'),
        });
        if (!ok || bag.disposed) return false;
        res = await api.post('/api/repertoire/nodes', { side, parent_id: parentId, uci, replace: true }, { signal });
      }
      const last = res.path[res.path.length - 1];
      selected = last;
      await refreshAll();
      if (res.added > 0) {
        const n = idx.byId.get(last);
        if (n) setFeedback(feedback, 'good', escapeHtml(t(n.mine ? 'repertoire.details.addedMine' : 'repertoire.details.addedTheirs', { move: moveLabel(n.ply, n.san) })));
      }
      return true;
    } catch (e) {
      if (!isAbort(e)) toast(e.message, 'error');
      return false;
    } finally {
      busy = false;
    }
  }

  async function deleteNode(n) {
    const count = countSubtree(n.id);
    const ok = await confirmDialog({
      title: t('repertoire.delete.title', { move: moveLabel(n.ply, n.san) }),
      message: count > 1 ? t('repertoire.delete.textMany', { count: count - 1 }) : t('repertoire.delete.textOne'),
      confirmLabel: t('repertoire.delete.confirm'), danger: true,
    });
    if (!ok || bag.disposed) return;
    try {
      await api.del(`/api/repertoire/nodes/${n.id}`, { signal });
      selected = n.parent_id;
      await refreshAll();
    } catch (e) { if (!isAbort(e)) toast(e.message, 'error'); }
  }

  function countSubtree(id) {
    let c = 0;
    const stack = [id];
    while (stack.length && c < 100000) { const cur = stack.pop(); c++; for (const k of childrenOf(cur)) stack.push(k.id); }
    return c;
  }

  async function clearSide() {
    const ok = await confirmDialog({
      title: t('repertoire.clear.title', { side: sideInline(side) }),
      message: t('repertoire.clear.text', { count: tree.nodes.length }),
      confirmLabel: t('repertoire.clear.confirm'), danger: true,
    });
    if (!ok || bag.disposed) return;
    try {
      await api.del('/api/repertoire' + qs({ side }), { signal });
      selected = 0;
      await refreshAll();
      loadRecent();
    } catch (e) { if (!isAbort(e)) toast(e.message, 'error'); }
  }

  // ==================================================================
  // Recent games (deviation check)
  // ==================================================================
  async function loadRecent() {
    recent.replaceChildren(h('h2', { class: 'section-title' }, t('repertoire.recent.title')), skeleton('list', 2));
    let list = [];
    try {
      list = await api.get('/api/repertoire/deviations' + qs({ limit: 8 }), { signal });
    } catch (e) {
      if (isAbort(e) || bag.disposed) return;
      recent.replaceChildren();
      return;
    }
    if (bag.disposed) return;
    const head = [h('h2', { class: 'section-title' }, t('repertoire.recent.title')), h('p', { class: 'muted rep-section-sub' }, t('repertoire.recent.sub'))];
    if (!Array.isArray(list) || !list.length) {
      recent.replaceChildren(...head, h('div', { class: 'card rep-recent-empty' },
        h('span', { html: icon('swords'), class: 'rep-recent-empty-icon', 'aria-hidden': 'true' }),
        h('div', null, h('div', { class: 'semibold' }, t('repertoire.recent.emptyTitle')), h('div', { class: 'text-sm muted' }, t('repertoire.recent.emptyText'))),
        h('a', { class: 'btn btn-secondary btn-sm', href: '#/play' }, t('repertoire.recent.play'))));
      return;
    }
    recent.replaceChildren(...head, h('div', { class: 'card card-flush list rep-recent' }, list.map(deviationRow)));
  }

  function deviationRow(d) {
    const youWhite = d.side === 'white';
    const opp = youWhite ? d.black : d.white;
    const res = d.result === '1/2-1/2' ? 'draw' : (d.result === '1-0') === youWhite ? 'win' : (d.result === '0-1' || d.result === '1-0') ? 'loss' : 'draw';
    const resLetter = { win: t('repertoire.recent.w'), loss: t('repertoire.recent.l'), draw: t('repertoire.recent.d') }[res];
    const expected = (d.expected || []).map((e) => moveLabel(d.ply, e.san));
    let text = '';
    let kind = 'info';
    const actions = [];
    const showBtn = (label, ic = 'eye') => h('button', { class: 'btn btn-secondary btn-sm', type: 'button', html: icon(ic) + `<span>${escapeHtml(label)}</span>`, onClick: () => showInTree(d) });
    if (d.status === 'deviated') {
      kind = 'warn';
      text = t('repertoire.recent.deviated', { n: Math.ceil(d.ply / 2), played: moveLabel(d.ply, d.played_san), expected: expected.join(' / ') });
      actions.push(showBtn(t('repertoire.recent.show')));
    } else if (d.status === 'unprepared') {
      kind = 'new';
      text = t('repertoire.recent.unprepared', { move: moveLabel(d.ply, d.played_san) });
      const addBtn = h('button', { class: 'btn btn-primary btn-sm', type: 'button', html: icon('plus') + `<span>${escapeHtml(t('repertoire.recent.add'))}</span>` });
      addBtn.addEventListener('click', async () => {
        addBtn.classList.add('loading');
        try {
          const r = await api.post('/api/repertoire/nodes', { side: d.side, parent_id: d.parent_id, uci: d.played_uci }, { signal });
          if (bag.disposed) return;
          const id = r.path[r.path.length - 1];
          toast(t('repertoire.recent.added'), 'success');
          if (d.side === side) { selected = id; await refreshAll(); loadRecent(); workspace.scrollIntoView({ behavior: 'smooth', block: 'start' }); }
          else location.hash = `#/repertoire/${d.side}?node=${id}`;
        } catch (e) { if (!isAbort(e)) toast(e.message, 'error'); } finally { addBtn.classList.remove('loading'); }
      });
      actions.push(addBtn);
    } else if (d.status === 'end') {
      kind = 'good';
      text = t('repertoire.recent.end', { count: Math.ceil(d.book_plies / 2) });
      actions.push(showBtn(t('repertoire.recent.extend'), 'plus'));
    } else {
      kind = 'good';
      text = t('repertoire.recent.followed');
    }
    actions.push(h('a', { class: 'btn btn-ghost btn-sm', href: `#/review/${d.game_id}` }, t('repertoire.recent.review')));
    return h('div', { class: `list-row rep-dev ${kind}` },
      h('span', { class: `result result-${res}`, 'aria-label': res }, resLetter),
      h('div', { class: 'list-row-main' },
        h('div', { class: 'list-row-title' }, t('repertoire.recent.vs', { name: opp || '?' }), ' ', sidePill(d.side)),
        h('div', { class: 'rep-dev-text' }, h('span', { class: `rep-dev-ic ${kind}`, html: icon(kind === 'warn' ? 'alert' : kind === 'new' ? 'sparkles' : 'check-circle'), 'aria-hidden': 'true' }), h('span', null, text)),
        h('div', { class: 'list-row-sub' }, [d.opening_name, d.created_at ? formatRelative(d.created_at) : ''].filter(Boolean).join(' · '))),
      h('div', { class: 'rep-dev-actions' }, actions));
  }

  async function showInTree(d) {
    if (d.side !== side) { location.hash = `#/repertoire/${d.side}?node=${d.parent_id}`; return; }
    if (mode !== 'build') await exitDrill();
    if (bag.disposed) return;
    select(d.parent_id || 0);
    if (board && d.status === 'deviated') {
      const arrows = (d.expected || []).map((e) => ({ from: e.uci.slice(0, 2), to: e.uci.slice(2, 4), color: 'green' }));
      if (d.played_uci) arrows.push({ from: d.played_uci.slice(0, 2), to: d.played_uci.slice(2, 4), color: 'red' });
      board.setArrows(arrows);
      setFeedback(feedback, 'info', escapeHtml(t('repertoire.recent.arrowsHint')));
    }
    workspace.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  // ==================================================================
  // Drill mode
  // ==================================================================
  async function exitDrill() {
    mode = 'build';
    try { await Promise.all([loadTree(), loadSummary()]); } catch (e) { if (isAbort(e)) return; }
    if (bag.disposed) return;
    renderChrome();
    renderBuild();
  }

  function startDrill(practice) {
    mode = 'drill';
    const mb = newModeBag();
    let token = 0;
    const stats = { reviewed: 0, correct: 0 };
    let line = null;
    let at = 0;
    let failedHere = false;
    let isRetry = false;
    let waiting = false;
    const retry = [];
    let dueTotal = sideStats(side).due || 0;

    const boardSlot = h('div', { class: 'board-slot' });
    const promptBar = h('div', { class: 'rep-drill-prompt', 'aria-live': 'polite' });
    const lineEl = h('div', { class: 'rep-drill-line' });
    const fb = h('div', { class: 'lrn-feedback' });
    const statsEl = h('div', { class: 'rep-drill-stats' });
    const noteEl = h('div', { class: 'rep-drill-note' });
    const showBtn = h('button', { class: 'btn btn-secondary', type: 'button', html: icon('hint') + `<span>${escapeHtml(t('repertoire.drill.show'))}</span>`, onClick: () => reveal() });
    const skipBtn = h('button', { class: 'btn btn-ghost', type: 'button', html: icon('chevron-right') + `<span>${escapeHtml(t('repertoire.drill.skip'))}</span>`, onClick: () => nextLine() });
    const exitBtn = h('button', { class: 'btn btn-ghost btn-sm', type: 'button', html: icon('close') + `<span>${escapeHtml(t('repertoire.drill.exit'))}</span>`, onClick: () => exitDrill() });
    const body = h('div', { class: 'panel-body stack' }, statsEl, fb, noteEl, h('div', null, h('div', { class: 'stat-label mb-1' }, t('repertoire.drill.lineSoFar')), lineEl));
    const panel = h('div', { class: 'panel grow' },
      h('div', { class: 'panel-header' }, h('span', { html: icon('target'), style: 'display:contents' }), h('span', null, practice ? t('repertoire.drill.practiceTitle') : t('repertoire.drill.title')),
        h('span', { class: 'spacer' }), exitBtn),
      body,
      h('div', { class: 'panel-footer rep-drill-actions' }, showBtn, skipBtn));
    const host = h('div', { class: 'rep-drill-host' });
    const layout = h('div', { class: 'game-layout no-eval rep-layout rep-drill' },
      h('div', { class: 'game-main' }, promptBar, h('div', { class: 'board-row' }, boardSlot)),
      h('aside', { class: 'game-panel' }, panel));
    host.append(layout);
    workspace.replaceChildren(host);
    workspace.scrollIntoView({ behavior: 'smooth', block: 'start' });

    board = new Board(boardSlot, { fen: START_FEN, orientation: side, interactive: false, movableColor: side, onMove: (mv) => onDrillMove(mv) });
    mb.add(() => { token++; if (board) board.destroy(); board = null; });

    function renderStats() {
      statsEl.replaceChildren(
        h('div', { class: 'rep-drill-stat' }, h('div', { class: 'stat-label' }, t('repertoire.drill.reviewed')), h('div', { class: 'stat-value' }, String(stats.reviewed))),
        h('div', { class: 'rep-drill-stat' }, h('div', { class: 'stat-label' }, t('repertoire.drill.correctCount')), h('div', { class: 'stat-value text-primary' }, String(stats.correct))),
        h('div', { class: 'rep-drill-stat' }, h('div', { class: 'stat-label' }, t('repertoire.drill.dueLeft')), h('div', { class: 'stat-value' }, String(Math.max(0, dueTotal)))));
    }
    function setPrompt(kind, text) {
      promptBar.className = `rep-drill-prompt ${kind}`;
      promptBar.replaceChildren(h('span', { class: 'rep-drill-prompt-dot', 'aria-hidden': 'true' }), h('span', null, text));
    }
    function renderLine() {
      if (!line) { lineEl.replaceChildren(); return; }
      lineEl.replaceChildren(...line.nodes.slice(0, at).map((n, i) => h('span', { class: ['rep-move static', n.mine ? 'mine' : 'theirs'] },
        (n.ply % 2 === 1 || i === 0) ? h('span', { class: 'rep-no' }, moveNo(n.ply)) : null, h('span', { class: 'rep-san' }, sanOf(n.san)))));
    }

    async function nextLine() {
      const my = ++token;
      waiting = false;
      board.setInteractive(false);
      board.clearArrows();
      noteEl.replaceChildren();
      if (retry.length) {
        line = retry.shift();
        isRetry = true;
        setFeedback(fb, 'info', escapeHtml(t('repertoire.drill.retryIntro')));
      } else {
        isRetry = false;
        let res;
        try {
          res = await api.get('/api/repertoire/drill/next' + qs({ side, any: practice ? 1 : null }), { signal });
        } catch (e) {
          if (isAbort(e) || my !== token) return;
          setFeedback(fb, 'bad', escapeHtml(e.message));
          return;
        }
        if (my !== token || bag.disposed) return;
        line = res && res.line;
        if (line) dueTotal = line.due_total;
        if (!line || !line.nodes || !line.nodes.length) { finished(); return; }
        setFeedback(fb, null, '');
      }
      at = 0;
      failedHere = false;
      board.setOrientation(line.side);
      board.setPosition(START_FEN, { animate: false });
      renderLine();
      renderStats();
      step(my);
    }

    function step(my) {
      if (my !== token || !board) return;
      if (at >= line.nodes.length) {
        setPrompt('done', t('repertoire.drill.lineDone'));
        if (!failedHere) sfx('correct');
        timers.later(() => { if (my === token) nextLine(); }, NEXT_DELAY_MS + 300);
        return;
      }
      const n = line.nodes[at];
      if (!n.mine) {
        waiting = false;
        board.setInteractive(false);
        setPrompt('theirs', t('repertoire.drill.opponentThinking'));
        timers.later(() => {
          if (my !== token || !board) return;
          board.move(n.uci);
          at++;
          renderLine();
          step(my);
        }, OPP_DELAY_MS);
        return;
      }
      failedHere = false;
      waiting = true;
      board.setInteractive(true, line.side);
      setPrompt('mine', t('repertoire.drill.yourMove', { side: sideName(line.side), n: Math.ceil(n.ply / 2) }));
      showBtn.disabled = false;
    }

    /** First miss on this card: record it (server reschedules) and queue the line for a retry. */
    function markWrong(n, uci = '') {
      if (failedHere) return;
      failedHere = true;
      stats.reviewed++;
      if (!isRetry) {
        api.post('/api/repertoire/drill/attempt', { node_id: n.id, uci }, { signal }).then((r) => {
          if (r && r.card && n.is_due) { n.is_due = false; dueTotal = Math.max(0, dueTotal - 1); renderStats(); }
        }).catch(() => {});
      }
      if (retry.length < MAX_RETRY && !isRetry) retry.push({ side: line.side, nodes: line.nodes.slice(0, at + 1) });
      renderStats();
    }

    function showAnswer(n, html) {
      board.setArrows([{ from: n.uci.slice(0, 2), to: n.uci.slice(2, 4), color: 'green' }]);
      setFeedback(fb, 'bad', html);
    }

    function reveal() {
      if (!waiting || !line) return;
      const n = line.nodes[at];
      if (!n || !n.mine) return;
      markWrong(n);
      showAnswer(n, escapeHtml(t('repertoire.drill.revealed', { move: moveLabel(n.ply, n.san) })));
    }

    function onDrillMove(mv) {
      if (!waiting || !line) return false;
      const n = line.nodes[at];
      if (!n || !n.mine) return false;
      const correct = isSameUci(mv.uci, n.uci) || fenKey(mv.fen) === fenKey(n.fen);
      if (!correct) {
        markWrong(n, mv.uci);
        sfx('wrong');
        showAnswer(n, escapeHtml(t('repertoire.drill.wrong', { played: sanOf(mv.san), move: moveLabel(n.ply, n.san) })));
        return false;
      }
      waiting = false;
      showBtn.disabled = true;
      board.clearArrows();
      if (!failedHere) {
        stats.reviewed++;
        stats.correct++;
        if (!isRetry) {
          api.post('/api/repertoire/drill/attempt', { node_id: n.id, uci: mv.uci }, { signal }).then((r) => {
            if (r && n.is_due) { n.is_due = false; dueTotal = Math.max(0, dueTotal - 1); renderStats(); }
          }).catch(() => {});
        }
        setFeedback(fb, 'good', escapeHtml(t(isRetry ? 'repertoire.drill.fixed' : 'repertoire.drill.correct', { move: moveLabel(n.ply, n.san) })));
      } else {
        setFeedback(fb, 'info', escapeHtml(t('repertoire.drill.nowYouKnow')));
      }
      noteEl.replaceChildren(...(n.note ? [h('div', { class: 'rep-note-view' }, h('span', { html: icon('edit', { size: 14 }), style: 'display:contents' }), h('span', null, n.note))] : []));
      renderStats();
      at++;
      renderLine();
      const my = token;
      timers.later(() => step(my), 380);
      return true;
    }

    function finished() {
      waiting = false;
      board.setInteractive(false);
      setPrompt('done', t('repertoire.drill.allDone'));
      const pct = stats.reviewed ? Math.round((stats.correct / stats.reviewed) * 100) : 0;
      const doneCard = h('div', { class: 'rep-drill-done' },
        h('div', { class: 'rep-drill-done-emoji', 'aria-hidden': 'true' }, stats.reviewed ? '🎉' : '✅'),
        h('div', { class: 'semibold text-lg' }, stats.reviewed ? t('repertoire.drill.sessionDone') : t('repertoire.drill.nothingDue')),
        h('p', { class: 'muted text-sm' }, stats.reviewed ? t('repertoire.drill.sessionStats', { reviewed: stats.reviewed, pct }) : t('repertoire.drill.nothingDueText')),
        h('div', { class: 'row-sm row-wrap', style: 'justify-content:center' },
          h('button', { class: 'btn btn-primary', type: 'button', html: icon('refresh') + `<span>${escapeHtml(t('repertoire.drill.practiceAnyway'))}</span>`, onClick: () => startDrill(true) }),
          h('button', { class: 'btn btn-secondary', type: 'button', onClick: () => exitDrill() }, t('repertoire.drill.backToTree'))));
      fb.className = 'lrn-feedback';
      fb.replaceChildren();
      lineEl.replaceChildren();
      body.replaceChildren(statsEl, doneCard);
      renderStats();
      showBtn.disabled = true;
      skipBtn.disabled = true;
      if (stats.reviewed) { sfx('gameEnd'); confetti(host, timers); }
      loadSummary().then(() => { if (!bag.disposed) renderChrome(); }).catch(() => {});
    }

    mb.on(window, 'keydown', (e) => {
      if (mode !== 'drill') return;
      if (e.key === 'Escape') { e.preventDefault(); exitDrill(); }
    });

    renderStats();
    nextLine();
  }
}
