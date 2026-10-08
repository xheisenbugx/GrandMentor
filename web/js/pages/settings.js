// Settings page (#/settings): appearance (theme, board theme, piece set), live preview board,
// board behaviour (coords, legal dots, animation, auto-queen, notation), game (eval bar, sounds)
// and a "danger zone". Everything goes through ../settings.js.

import { api, qs, isAbort } from '../api.js';
import { h, icon, pageHeader, disposables, debounce, confirmDialog, toast, formatSan } from '../ui.js';
import { getSettings, setSetting, onSettingsChange, resetSettings, BOARD_THEMES, PIECE_SETS, pieceUrl } from '../settings.js';
import { ensureHubCss, fenBoardSvg } from './library.js';

export const title = 'Settings';

const PREVIEW_FEN = 'r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4';
const SPEEDS = [[0, 'Off'], [100, 'Fast'], [200, 'Normal'], [400, 'Slow']];
const RECREATE_KEYS = new Set(['pieceSet', 'showCoords', 'showLegal', 'animationMs', 'autoQueen', 'sounds']);

export async function mount(root) {
  await ensureHubCss();
  const bag = disposables();
  const ctrl = new AbortController();
  bag.add(() => ctrl.abort());
  const syncers = []; // functions that refresh controls from the current settings

  // ---- Controls -----------------------------------------------------------------
  const segmented = (key, options, label) => {
    const el = h('div', { class: 'segmented', role: 'radiogroup', 'aria-label': label },
      options.map(([v, l]) => h('button', { type: 'button', role: 'radio', dataset: { v: String(v) } }, l)));
    bag.on(el, 'click', (e) => {
      const b = e.target.closest('button[data-v]');
      if (!b) return;
      const opt = options.find(([v]) => String(v) === b.dataset.v);
      if (opt) setSetting(key, opt[0]);
    });
    syncers.push((s) => {
      for (const b of el.children) {
        const on = b.dataset.v === String(s[key]);
        b.classList.toggle('active', on);
        b.setAttribute('aria-checked', String(on));
      }
    });
    return el;
  };

  const toggle = (key, label) => {
    const input = h('input', { type: 'checkbox', 'aria-label': label });
    bag.on(input, 'change', () => setSetting(key, input.checked));
    syncers.push((s) => { input.checked = !!s[key]; });
    return h('label', { class: 'switch' }, input, h('span', { class: 'switch-track' }));
  };

  const row = (titleText, desc, control) => h('div', { class: 'setting-row' },
    h('div', { class: 'setting-row-text' }, h('div', { class: 'setting-row-title' }, titleText), desc ? h('div', { class: 'setting-row-desc' }, desc) : null),
    control);

  // Board themes: swatches with a tiny 4x4 checkerboard.
  const themePicker = h('div', { class: 'hub-swatches', role: 'radiogroup', 'aria-label': 'Board theme' },
    Object.entries(BOARD_THEMES).map(([k, t]) => h('button', { type: 'button', class: 'hub-swatch-btn', role: 'radio', dataset: { v: k }, 'aria-label': t.label },
      h('span', { class: 'hub-swatch-board', style: { '--l': t.light, '--d': t.dark } }),
      h('span', { class: 'hub-swatch-label' }, t.label))));
  bag.on(themePicker, 'click', (e) => { const b = e.target.closest('button[data-v]'); if (b) setSetting('boardTheme', b.dataset.v); });
  syncers.push((s) => markActive(themePicker, s.boardTheme));

  // Piece sets: preview K Q R B N P.
  const piecePicker = h('div', { class: 'hub-piecesets', role: 'radiogroup', 'aria-label': 'Piece set' },
    Object.entries(PIECE_SETS).map(([k, p]) => h('button', { type: 'button', class: 'hub-pieceset', role: 'radio', dataset: { v: k }, 'aria-label': p.label },
      h('span', { class: 'hub-pieceset-row' }, ['wK', 'wQ', 'wN', 'bB', 'bR', 'bP'].map((c) => h('img', { src: pieceUrl(c, k), alt: '', width: '34', height: '34', loading: 'lazy', decoding: 'async' }))),
      h('span', { class: 'hub-swatch-label' }, p.label))));
  bag.on(piecePicker, 'click', (e) => { const b = e.target.closest('button[data-v]'); if (b) setSetting('pieceSet', b.dataset.v); });
  syncers.push((s) => markActive(piecePicker, s.pieceSet));

  // Animation speed: presets + fine slider.
  const speedSeg = segmented('animationMs', SPEEDS, 'Animation speed');
  const range = h('input', { type: 'range', class: 'range', min: '0', max: '1000', step: '20', 'aria-label': 'Animation duration in milliseconds' });
  const rangeVal = h('span', { class: 'muted text-sm tabular hub-range-val' });
  const setAnim = debounce((v) => setSetting('animationMs', v), 120);
  bag.add(setAnim.cancel);
  bag.on(range, 'input', () => { rangeVal.textContent = `${range.value} ms`; setAnim(Number(range.value)); });
  syncers.push((s) => { range.value = String(s.animationMs); rangeVal.textContent = `${s.animationMs} ms`; });

  const notationExample = h('span', { class: 'muted text-sm mono' });
  syncers.push((s) => { notationExample.textContent = ['Nf3', 'Bb5', 'O-O', 'Qxd8+'].map((m) => formatSan(m, s.moveNotation)).join('  '); });

  const testSoundBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('volume') + '<span>Test</span>' });
  bag.on(testSoundBtn, 'click', async () => {
    try {
      const m = await import('../components/sound.js');
      m.playSound?.('move');
      bag.timeout(() => { try { m.playSound?.('capture'); } catch { /* ignore */ } }, 260);
    } catch { toast('Sounds aren’t available yet', 'warning'); }
  });

  // ---- Preview board -----------------------------------------------------------
  const previewSlot = h('div', { class: 'hub-preview-board' });
  const resetPreviewBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('refresh', { size: 16 }) + '<span>Reset</span>' });
  let board = null;
  let BoardCls = null;
  let previewFen = PREVIEW_FEN;
  bag.add(() => { board?.destroy(); board = null; });

  const buildBoard = () => {
    if (bag.disposed) return;
    const s = getSettings();
    if (board) { try { previewFen = board.getFen() || previewFen; } catch { /* ignore */ } board.destroy(); board = null; }
    previewSlot.replaceChildren();
    if (!BoardCls) { previewSlot.innerHTML = fenBoardSvg(previewFen, { label: 'Board preview' }); return; }
    const holder = h('div', { class: 'hub-preview-inner' });
    previewSlot.appendChild(holder);
    try {
      board = new BoardCls(holder, {
        fen: previewFen, orientation: 'white', interactive: true, movableColor: 'both',
        showCoords: s.showCoords, showLegal: s.showLegal, animationMs: s.animationMs, sounds: s.sounds, autoQueen: s.autoQueen,
        onMove: () => true,
      });
    } catch (e) {
      console.error('[settings] preview board failed', e);
      previewSlot.innerHTML = fenBoardSvg(previewFen, { label: 'Board preview' });
    }
  };
  const rebuildSoon = debounce(buildBoard, 150);
  bag.add(rebuildSoon.cancel);
  bag.on(resetPreviewBtn, 'click', () => { previewFen = PREVIEW_FEN; if (board) board.setPosition(PREVIEW_FEN, { animate: true }); else buildBoard(); });
  previewSlot.innerHTML = fenBoardSvg(previewFen, { label: 'Board preview' });
  import('../components/board.js').then((m) => { if (bag.disposed) return; BoardCls = m.Board || null; buildBoard(); })
    .catch(() => { /* static preview stays */ });

  // ---- Danger zone -----------------------------------------------------------------
  const resetSettingsBtn = h('button', { type: 'button', class: 'btn btn-secondary', html: icon('refresh') + '<span>Reset settings</span>' });
  bag.on(resetSettingsBtn, 'click', async () => {
    const ok = await confirmDialog({ title: 'Reset all settings?', message: 'Board theme, pieces, sounds and every other preference will go back to the defaults. Your games and progress are kept.', confirmLabel: 'Reset settings', danger: true });
    if (!ok || bag.disposed) return;
    resetSettings();
    toast('Settings restored to defaults', 'success');
  });

  const deleteGamesBtn = h('button', { type: 'button', class: 'btn btn-danger', html: icon('trash') + '<span>Delete all games</span>' });
  bag.on(deleteGamesBtn, 'click', async () => {
    const ok = await confirmDialog({ title: 'Delete every saved game?', message: 'All games in your library, including their reviews, notes and tags, will be permanently deleted. This can’t be undone.', confirmLabel: 'Delete all games', danger: true });
    if (!ok || bag.disposed) return;
    deleteGamesBtn.classList.add('loading');
    let deleted = 0;
    try {
      for (let guard = 0; guard < 100; guard++) {
        const batch = await api.get('/api/games' + qs({ limit: 200, offset: 0 }), { signal: ctrl.signal });
        if (!Array.isArray(batch) || !batch.length) break;
        for (const g of batch) {
          await api.del(`/api/games/${encodeURIComponent(g.id)}`, { signal: ctrl.signal });
          deleted++;
        }
      }
      toast(deleted ? `Deleted ${deleted} ${deleted === 1 ? 'game' : 'games'}` : 'Your library was already empty', 'success');
    } catch (e) {
      if (!isAbort(e)) toast(`${e?.message || 'Could not delete games'}${deleted ? ` (${deleted} deleted)` : ''}`, 'error');
    } finally {
      deleteGamesBtn.classList.remove('loading');
    }
  });

  const clearLocalBtn = h('button', { type: 'button', class: 'btn btn-danger', html: icon('x-circle') + '<span>Clear browser data</span>' });
  bag.on(clearLocalBtn, 'click', async () => {
    const ok = await confirmDialog({ title: 'Clear data stored in this browser?', message: 'This removes GrandMentor preferences and any in-progress state saved in this browser, then reloads the app. Games stored on the server are not affected.', confirmLabel: 'Clear and reload', danger: true });
    if (!ok || bag.disposed) return;
    try {
      for (const store of [localStorage, sessionStorage]) {
        const keys = [];
        for (let i = 0; i < store.length; i++) { const k = store.key(i); if (k && /^(grandmentor|gm[._-])/i.test(k)) keys.push(k); }
        for (const k of keys) store.removeItem(k);
      }
    } catch { /* storage blocked */ }
    location.reload();
  });

  // ---- Layout ---------------------------------------------------------------------
  const page = h('div', { class: 'page hub-page hub-settings' },
    pageHeader({ title: 'Settings', subtitle: 'Make the board look and feel just right', icon: 'settings' }),
    h('div', { class: 'hub-settings-layout' },
      h('div', { class: 'stack-lg hub-settings-main' },
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('palette') + '<span>Appearance</span>' })),
          row('Theme', 'Dark is easy on the eyes; light is great in bright rooms.', segmented('theme', [['dark', 'Dark'], ['light', 'Light']], 'Color theme')),
          h('div', { class: 'hub-setting-block' }, h('div', { class: 'setting-row-title' }, 'Board colors'), themePicker),
          h('div', { class: 'hub-setting-block' }, h('div', { class: 'setting-row-title' }, 'Pieces'), piecePicker)),
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('board') + '<span>Board</span>' })),
          row('Show coordinates', 'Letters (a–h) and numbers (1–8) on the board edge help you read moves.', toggle('showCoords', 'Show coordinates')),
          row('Show legal moves', 'Dots show where the piece you picked up can go.', toggle('showLegal', 'Show legal moves')),
          row('Always promote to a queen', 'Skip the promotion menu when a pawn reaches the last rank.', toggle('autoQueen', 'Auto-queen')),
          h('div', { class: 'setting-row hub-setting-stack' },
            h('div', { class: 'setting-row-text' }, h('div', { class: 'setting-row-title' }, 'Piece animation'), h('div', { class: 'setting-row-desc' }, 'How fast pieces slide across the board.')),
            h('div', { class: 'stack-sm hub-anim-ctl' }, speedSeg, h('div', { class: 'row-sm' }, range, rangeVal))),
          row('Move notation', h('span', null, 'How moves are written. Example: ', notationExample), segmented('moveNotation', [['san', 'Letters'], ['figurine', 'Figurines']], 'Move notation'))),
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('play') + '<span>Playing</span>' })),
          row('Evaluation bar', 'The bar beside the board that shows who is winning.', toggle('showEvalBar', 'Show evaluation bar')),
          row('Sounds', 'Move, capture and check sounds.', h('div', { class: 'row-sm' }, testSoundBtn, toggle('sounds', 'Sounds')))),
        h('section', { class: 'card hub-danger' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('alert') + '<span>Danger zone</span>' })),
          row('Reset settings', 'Go back to the default look and behaviour.', resetSettingsBtn),
          row('Delete all games', 'Permanently remove every game in your library.', deleteGamesBtn),
          row('Clear browser data', 'Forget preferences saved in this browser and reload.', clearLocalBtn))),
      h('aside', { class: 'hub-settings-aside' },
        h('div', { class: 'card hub-preview-card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('eye') + '<span>Live preview</span>' }), resetPreviewBtn),
          previewSlot,
          h('p', { class: 'subtle text-xs mt-2' }, 'Try moving a piece — changes apply instantly.')))));
  root.appendChild(page);

  const syncAll = () => { const s = getSettings(); for (const fn of syncers) fn(s); };
  syncAll();
  bag.add(onSettingsChange((s, key) => {
    syncAll();
    if (RECREATE_KEYS.has(key)) {
      if (key === 'pieceSet' && !board) previewSlot.innerHTML = fenBoardSvg(previewFen, { label: 'Board preview', pieceSet: s.pieceSet });
      else rebuildSoon();
    }
  }));
  return bag.dispose;
}

function markActive(container, value) {
  for (const b of container.children) {
    const on = b.dataset.v === value;
    b.classList.toggle('active', on);
    b.setAttribute('aria-checked', String(on));
  }
}
