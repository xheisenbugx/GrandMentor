// Settings page (#/settings): appearance (theme, board theme, piece set), live preview board,
// board behaviour (coords, legal dots, animation, auto-queen, notation), game (eval bar, sounds),
// "Your data" (backup / restore / device sync, components/backup.js) and a "danger zone".
// Everything goes through ../settings.js. The language picker sits at the top of Appearance;
// switching language makes app.js remount this page.

import { api, qs, isAbort } from '../api.js';
import { h, icon, pageHeader, disposables, debounce, confirmDialog, toast, formatSan } from '../ui.js';
import { getSettings, setSetting, onSettingsChange, resetSettings, BOARD_THEMES, PIECE_SETS, UI_SCALES, pieceUrl } from '../settings.js';
import { ensureHubCss, fenBoardSvg } from './library.js';
import { t, getLanguage, setLanguage, LANGUAGES } from '../i18n.js';
import { createBackupSection } from '../components/backup.js';
import { createFirstWeekSection } from '../components/firstweek.js';
import { createPhoneSection } from '../components/phone.js';
import { speak, speechSupported, cancelSpeech, voiceAvailable, onVoicesChanged } from '../components/speech.js';
import { describeMove } from '../components/announcer.js';
import { SOUND_EVENTS, SOUND_PRESETS, stylesFor, defaultStyle, resolveStyle, presetPicks, matchPreset } from '../sound-catalog.js';

export const title = () => t('nav.routes.settings');

const PREVIEW_FEN = 'r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4';
// Animation presets: [ms, i18n key] (resolved at render time).
const SPEEDS = [[0, 'settings.speed.off'], [100, 'settings.speed.fast'], [200, 'settings.speed.normal'], [400, 'settings.speed.slow']];
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

  // Like row(), but on narrow screens the control drops below the text instead of squeezing it.
  const stackRow = (titleText, desc, control) => {
    const el = row(titleText, desc, control);
    el.classList.add('hub-setting-stack');
    return el;
  };

  // Board themes: swatches with a tiny 4x4 checkerboard.
  const themePicker = h('div', { class: 'hub-swatches', role: 'radiogroup', 'aria-label': t('settings.boardTheme') },
    Object.entries(BOARD_THEMES).map(([k, bt]) => h('button', { type: 'button', class: 'hub-swatch-btn', role: 'radio', dataset: { v: k }, 'aria-label': t(bt.labelKey) },
      h('span', { class: 'hub-swatch-board', style: { '--l': bt.light, '--d': bt.dark } }),
      h('span', { class: 'hub-swatch-label' }, t(bt.labelKey)))));
  bag.on(themePicker, 'click', (e) => { const b = e.target.closest('button[data-v]'); if (b) setSetting('boardTheme', b.dataset.v); });
  syncers.push((s) => markActive(themePicker, s.boardTheme));

  // Piece sets: preview K Q R B N P.
  const piecePicker = h('div', { class: 'hub-piecesets', role: 'radiogroup', 'aria-label': t('settings.pieceSet') },
    Object.entries(PIECE_SETS).map(([k, p]) => h('button', { type: 'button', class: 'hub-pieceset', role: 'radio', dataset: { v: k }, 'aria-label': t(p.labelKey) },
      h('span', { class: 'hub-pieceset-row' }, ['wK', 'wQ', 'wN', 'bB', 'bR', 'bP'].map((c) => h('img', { src: pieceUrl(c, k), alt: '', width: '34', height: '34', loading: 'lazy', decoding: 'async' }))),
      h('span', { class: 'hub-swatch-label' }, t(p.labelKey)))));
  bag.on(piecePicker, 'click', (e) => { const b = e.target.closest('button[data-v]'); if (b) setSetting('pieceSet', b.dataset.v); });
  syncers.push((s) => markActive(piecePicker, s.pieceSet));

  // Animation speed: presets + fine slider.
  // Language: one button per supported language, labelled with its native name.
  const langPicker = h('div', { class: 'segmented lang-picker', role: 'radiogroup', 'aria-label': t('settings.language.title') },
    Object.entries(LANGUAGES).map(([code, l]) => h('button', { type: 'button', role: 'radio', lang: code, dataset: { v: code }, 'aria-checked': String(code === getLanguage()), class: code === getLanguage() ? 'active' : null }, l.nativeName)));
  bag.on(langPicker, 'click', (e) => {
    const b = e.target.closest('button[data-v]');
    if (b && b.dataset.v !== getLanguage()) setLanguage(b.dataset.v);
  });
  syncers.push((s) => markActive(langPicker, s.language));

  const speedSeg = segmented('animationMs', SPEEDS.map(([v, k]) => [v, t(k)]), t('settings.animationSpeed'));
  const range = h('input', { type: 'range', class: 'range', min: '0', max: '1000', step: '20', 'aria-label': t('settings.animationDuration') });
  const rangeVal = h('span', { class: 'muted text-sm tabular hub-range-val' });
  const setAnim = debounce((v) => setSetting('animationMs', v), 120);
  bag.add(setAnim.cancel);
  bag.on(range, 'input', () => { rangeVal.textContent = t('settings.ms', { ms: range.value }); setAnim(Number(range.value)); });
  syncers.push((s) => { range.value = String(s.animationMs); rangeVal.textContent = t('settings.ms', { ms: s.animationMs }); });

  const notationExample = h('span', { class: 'muted text-sm mono' });
  syncers.push((s) => { notationExample.textContent = ['Nf3', 'Bb5', 'O-O', 'Qxd8+'].map((m) => formatSan(m, s.moveNotation)).join('  '); });

  const testSoundBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('volume') + `<span>${t('settings.testSound')}</span>` });

  // ---- Sounds: volume, theme and one choice per event ---------------------------------
  let soundMod = null;
  const loadSound = () => (soundMod ? Promise.resolve(soundMod) : import('../components/sound.js').then((m) => (soundMod = m)));
  /** Play one event (or a short sequence) regardless of the on/off switch: the user asked to hear it. */
  const preview = (events, style) => {
    loadSound().then((m) => {
      if (bag.disposed) return;
      events.forEach((ev, i) => {
        const go = () => { try { m.playSound(ev, { force: true, style }); } catch { /* ignore */ } };
        if (i === 0) go(); else bag.timeout(go, i * 320);
      });
    }).catch(() => toast(t('settings.soundsUnavailable'), 'warning'));
  };

  const volume = h('input', { type: 'range', class: 'range', min: '0', max: '100', step: '5', 'aria-label': t('settings.soundSection.volume.title') });
  const volumeVal = h('span', { class: 'muted text-sm tabular hub-range-val' });
  const setVolume = debounce((v) => setSetting('soundVolume', v), 120);
  const previewVolume = debounce(() => preview(['move']), 160);
  bag.add(setVolume.cancel);
  bag.add(previewVolume.cancel);
  bag.on(volume, 'input', () => {
    volumeVal.textContent = t('settings.soundSection.volume.value', { percent: volume.value });
    setVolume(Number(volume.value));
    previewVolume();
  });
  syncers.push((s) => { volume.value = String(s.soundVolume); volumeVal.textContent = t('settings.soundSection.volume.value', { percent: s.soundVolume }); });

  bag.on(testSoundBtn, 'click', () => preview(['move', 'capture', 'check']));

  const presetSelect = h('select', { class: 'select hub-sound-select', 'aria-label': t('settings.soundSection.theme.title') },
    Object.keys(SOUND_PRESETS).map((id) => h('option', { value: id }, t(`settings.soundSection.presets.${id}`))),
    h('option', { value: 'custom', disabled: true }, t('settings.soundSection.presets.custom')));
  bag.on(presetSelect, 'change', () => {
    const picks = presetPicks(presetSelect.value);
    if (!picks) return;
    setSetting('soundPicks', picks);
    preview(['move', 'capture', 'check']);
  });
  syncers.push((s) => { presetSelect.value = matchPreset(s.soundPicks) || 'custom'; });

  const styleLabel = (ev, st) => t(`settings.soundSection.styles.${SOUND_EVENTS.find((e) => e.id === ev).family}.${st}`);
  const eventRows = SOUND_EVENTS.map(({ id }) => {
    const label = t(`settings.soundSection.events.${id}`);
    const sel = h('select', { class: 'select hub-sound-select', 'aria-label': label },
      stylesFor(id).map((st) => h('option', { value: st }, styleLabel(id, st))));
    bag.on(sel, 'change', () => {
      const picks = { ...getSettings().soundPicks };
      if (sel.value === defaultStyle(id)) delete picks[id]; else picks[id] = sel.value;
      setSetting('soundPicks', picks);
      preview([id]);
    });
    syncers.push((s) => { sel.value = resolveStyle(s.soundPicks, id); });
    const play = h('button', {
      type: 'button', class: 'btn btn-ghost btn-icon btn-sm hub-sound-play', html: icon('play-circle'),
      'aria-label': t('settings.soundSection.previewOne', { sound: label }), 'data-tooltip': t('settings.soundSection.preview'),
    });
    bag.on(play, 'click', () => preview([id]));
    return h('div', { class: 'hub-sound-row' }, h('span', { class: 'hub-sound-label' }, label), sel, play);
  });
  const soundBody = h('div', { class: 'stack-sm' },
    stackRow(t('settings.soundSection.volume.title'), t('settings.soundSection.volume.desc'), h('div', { class: 'row-sm hub-anim-ctl' }, volume, volumeVal)),
    stackRow(t('settings.soundSection.theme.title'), t('settings.soundSection.theme.desc'), h('div', { class: 'row-sm hub-sound-theme-ctl' }, presetSelect, testSoundBtn)),
    h('div', { class: 'hub-setting-block hub-sound-block' },
      h('div', { class: 'setting-row-title' }, t('settings.soundSection.each.title')),
      h('div', { class: 'setting-row-desc' }, t('settings.soundSection.each.desc')),
      h('div', { class: 'hub-sound-grid' }, eventRows)));
  syncers.push((s) => { soundBody.classList.toggle('hub-sounds-off', !s.sounds); });
  const soundSection = h('section', { class: 'card', id: 'sounds', 'aria-labelledby': 'settings-sounds-title' },
    h('div', { class: 'card-header' }, h('h2', { class: 'card-title', id: 'settings-sounds-title', html: icon('volume') + `<span>${t('settings.soundSection.title')}</span>` })),
    row(t('settings.sounds.title'), t('settings.sounds.desc'), toggle('sounds', t('settings.sounds.title'))),
    soundBody);

  // ---- Preview board -----------------------------------------------------------
  const previewSlot = h('div', { class: 'hub-preview-board' });
  const resetPreviewBtn = h('button', { type: 'button', class: 'btn btn-ghost btn-sm', html: icon('refresh', { size: 16 }) + `<span>${t('common.reset')}</span>` });
  let board = null;
  let BoardCls = null;
  let previewFen = PREVIEW_FEN;
  bag.add(() => { board?.destroy(); board = null; });

  const buildBoard = () => {
    if (bag.disposed) return;
    const s = getSettings();
    if (board) { try { previewFen = board.getFen() || previewFen; } catch { /* ignore */ } board.destroy(); board = null; }
    previewSlot.replaceChildren();
    if (!BoardCls) { previewSlot.innerHTML = fenBoardSvg(previewFen, { label: t('settings.boardPreview') }); return; }
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
      previewSlot.innerHTML = fenBoardSvg(previewFen, { label: t('settings.boardPreview') });
    }
  };
  const rebuildSoon = debounce(buildBoard, 150);
  bag.add(rebuildSoon.cancel);
  bag.on(resetPreviewBtn, 'click', () => { previewFen = PREVIEW_FEN; if (board) board.setPosition(PREVIEW_FEN, { animate: true }); else buildBoard(); });
  previewSlot.innerHTML = fenBoardSvg(previewFen, { label: t('settings.boardPreview') });
  import('../components/board.js').then((m) => { if (bag.disposed) return; BoardCls = m.Board || null; buildBoard(); })
    .catch(() => { /* static preview stays */ });

  // ---- Danger zone -----------------------------------------------------------------
  const resetSettingsBtn = h('button', { type: 'button', class: 'btn btn-secondary', html: icon('refresh') + `<span>${t('settings.reset.button')}</span>` });
  bag.on(resetSettingsBtn, 'click', async () => {
    const ok = await confirmDialog({ title: t('settings.reset.confirmTitle'), message: t('settings.reset.confirmMessage'), confirmLabel: t('settings.reset.button'), danger: true });
    if (!ok || bag.disposed) return;
    resetSettings();
    toast(t('settings.reset.done'), 'success');
  });

  const deleteGamesBtn = h('button', { type: 'button', class: 'btn btn-danger', html: icon('trash') + `<span>${t('settings.deleteGames.button')}</span>` });
  bag.on(deleteGamesBtn, 'click', async () => {
    const ok = await confirmDialog({ title: t('settings.deleteGames.confirmTitle'), message: t('settings.deleteGames.confirmMessage'), confirmLabel: t('settings.deleteGames.button'), danger: true });
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
      toast(deleted ? t('settings.deleteGames.done', { count: deleted }) : t('settings.deleteGames.alreadyEmpty'), 'success');
    } catch (e) {
      if (!isAbort(e)) {
        const msg = e?.message || t('settings.deleteGames.failed');
        toast(deleted ? t('settings.deleteGames.partial', { message: msg, count: deleted }) : msg, 'error');
      }
    } finally {
      deleteGamesBtn.classList.remove('loading');
    }
  });

  const resetPuzzlesBtn = h('button', { type: 'button', class: 'btn btn-danger', html: icon('refresh') + `<span>${t('settings.resetPuzzles.button')}</span>` });
  bag.on(resetPuzzlesBtn, 'click', async () => {
    const ok = await confirmDialog({ title: t('settings.resetPuzzles.confirmTitle'), message: t('settings.resetPuzzles.confirmMessage'), confirmLabel: t('settings.resetPuzzles.button'), danger: true });
    if (!ok || bag.disposed) return;
    resetPuzzlesBtn.classList.add('loading');
    try {
      await api.post('/api/profile/puzzles/reset', { confirm: true }, { signal: ctrl.signal });
      toast(t('settings.resetPuzzles.done'), 'success');
    } catch (e) {
      if (!isAbort(e)) toast(e?.message || t('settings.resetPuzzles.failed'), 'error');
    } finally {
      resetPuzzlesBtn.classList.remove('loading');
    }
  });

  const clearLocalBtn = h('button', { type: 'button', class: 'btn btn-danger', html: icon('x-circle') + `<span>${t('settings.clearData.button')}</span>` });
  bag.on(clearLocalBtn, 'click', async () => {
    const ok = await confirmDialog({ title: t('settings.clearData.confirmTitle'), message: t('settings.clearData.confirmMessage'), confirmLabel: t('settings.clearData.confirmButton'), danger: true });
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

  // ---- Accessibility ----------------------------------------------------------------
  // High-contrast board is a quick switch over the board theme; remember the theme to go back to.
  let lastBoardTheme = getSettings().boardTheme !== 'contrast' ? getSettings().boardTheme : 'green';
  const boardContrastInput = h('input', { type: 'checkbox', 'aria-label': t('a11y.settings.boardContrastRow.title') });
  bag.on(boardContrastInput, 'change', () => {
    const cur = getSettings().boardTheme;
    if (boardContrastInput.checked) { if (cur !== 'contrast') lastBoardTheme = cur; setSetting('boardTheme', 'contrast'); }
    else setSetting('boardTheme', lastBoardTheme === 'contrast' ? 'green' : lastBoardTheme);
  });
  syncers.push((s) => { boardContrastInput.checked = s.boardTheme === 'contrast'; if (s.boardTheme !== 'contrast') lastBoardTheme = s.boardTheme; });
  // Read moves aloud (Web Speech API): main switch + "Try it", and "my moves too" under it.
  const canSpeak = speechSupported();
  const speakToggle = toggle('speakMoves', t('a11y.speech.title'));
  const speakOwnToggle = toggle('speakOwnMoves', t('a11y.speech.own.title'));
  const speakTestBtn = h('button', { type: 'button', class: 'btn btn-secondary btn-sm', 'aria-label': t('a11y.speech.testLabel'), html: icon('volume') + `<span>${t('a11y.speech.test')}</span>` });
  bag.on(speakTestBtn, 'click', () => speak(describeMove({ color: 'w', piece: 'n', from: 'g1', to: 'f3', san: 'Nf3', flags: 'n' })));
  bag.add(cancelSpeech);
  const speakOwnRow = row(t('a11y.speech.own.title'), t('a11y.speech.own.desc'), speakOwnToggle);
  speakOwnRow.classList.add('hub-setting-sub');
  syncers.push((s) => {
    const off = !canSpeak || !s.speakMoves;
    speakOwnToggle.querySelector('input').disabled = off;
    speakOwnRow.classList.toggle('is-disabled', off);
    speakTestBtn.disabled = !canSpeak;
  });
  if (!canSpeak) speakToggle.querySelector('input').disabled = true;
  const speakRow = row(t('a11y.speech.title'), canSpeak ? t('a11y.speech.desc') : t('a11y.speech.unsupported'),
    h('div', { class: 'row-sm hub-speech-controls' }, speakTestBtn, speakToggle));
  const noVoiceNote = h('div', { class: 'setting-row-desc subtle text-xs', hidden: true }, t('a11y.speech.noVoice'));
  speakRow.querySelector('.setting-row-text')?.appendChild(noVoiceNote);
  const syncVoiceNote = () => { noVoiceNote.hidden = voiceAvailable() !== false; };
  syncVoiceNote();
  bag.add(onVoicesChanged(syncVoiceNote));

  const a11ySection = h('section', { class: 'card', id: 'accessibility', 'aria-labelledby': 'settings-a11y-title' },
    h('div', { class: 'card-header' }, h('h2', { class: 'card-title', id: 'settings-a11y-title', html: icon('eye') + `<span>${t('a11y.settings.section')}</span>` })),
    h('p', { class: 'muted text-sm' }, t('a11y.settings.intro')),
    row(t('a11y.settings.highContrast.title'), t('a11y.settings.highContrast.desc'), toggle('highContrast', t('a11y.settings.highContrast.title'))),
    row(t('a11y.settings.boardContrastRow.title'), t('a11y.settings.boardContrastRow.desc'), h('label', { class: 'switch' }, boardContrastInput, h('span', { class: 'switch-track' }))),
    row(t('a11y.settings.cbPalette.title'), t('a11y.settings.cbPalette.desc'), toggle('cbPalette', t('a11y.settings.cbPalette.title'))),
    row(t('a11y.settings.motion.title'), t('a11y.settings.motion.desc'),
      segmented('motion', [['system', t('a11y.settings.motion.system')], ['reduce', t('a11y.settings.motion.reduce')], ['full', t('a11y.settings.motion.full')]], t('a11y.settings.motion.title'))),
    row(t('a11y.settings.announce.title'), t('a11y.settings.announce.desc'), toggle('announceMoves', t('a11y.settings.announce.title'))),
    speakRow,
    speakOwnRow,
    row(t('a11y.settings.squareNames.title'), t('a11y.settings.squareNames.desc'), toggle('squareNames', t('a11y.settings.squareNames.title'))),
    row(t('a11y.settings.scale.title'), t('a11y.settings.scale.desc'),
      segmented('uiScale', UI_SCALES.map((v) => [v, t('a11y.settings.scale.option', { percent: v })]), t('a11y.settings.scale.title'))),
    h('div', { class: 'setting-row hub-setting-stack' },
      h('div', { class: 'setting-row-text' },
        h('h3', { class: 'setting-row-title' }, t('a11y.settings.keyboardHelp.title')),
        h('p', { class: 'setting-row-desc' }, t('a11y.settings.keyboardHelp.text')))));

  // ---- Your data (backup, restore, device sync) — self-contained component -----------
  const dataSection = createBackupSection();
  bag.add(dataSection.destroy);
  const firstWeekSection = createFirstWeekSection();
  bag.add(firstWeekSection.destroy);
  const phoneSection = createPhoneSection(); // "Use on your phone" — components/phone.js
  bag.add(phoneSection.destroy);

  // ---- Layout ---------------------------------------------------------------------
  const page = h('div', { class: 'page hub-page hub-settings' },
    pageHeader({ title: t('settings.title'), subtitle: t('settings.subtitle'), icon: 'settings' }),
    h('div', { class: 'hub-settings-layout' },
      h('div', { class: 'stack-lg hub-settings-main' },
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('palette') + `<span>${t('settings.sections.appearance')}</span>` })),
          h('div', { class: 'setting-row hub-language-row' },
            h('div', { class: 'setting-row-text' },
              h('div', { class: 'setting-row-title', html: icon('globe', { size: 18 }) + `<span>${t('settings.language.title')}</span>`, style: 'display:flex;align-items:center;gap:var(--sp-2)' }),
              h('div', { class: 'setting-row-desc' }, t('settings.language.desc')),
              h('div', { class: 'setting-row-desc subtle text-xs' }, t('settings.language.moreSoon'))),
            langPicker),
          row(t('settings.theme.title'), t('settings.theme.desc'), segmented('theme', [['dark', t('settings.theme.dark')], ['light', t('settings.theme.light')]], t('settings.theme.aria'))),
          h('div', { class: 'hub-setting-block' }, h('div', { class: 'setting-row-title' }, t('settings.boardColors')), themePicker),
          h('div', { class: 'hub-setting-block' }, h('div', { class: 'setting-row-title' }, t('settings.pieces')), piecePicker)),
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('board') + `<span>${t('settings.sections.board')}</span>` })),
          row(t('settings.coords.title'), t('settings.coords.desc'), toggle('showCoords', t('settings.coords.title'))),
          row(t('settings.legal.title'), t('settings.legal.desc'), toggle('showLegal', t('settings.legal.title'))),
          row(t('settings.autoQueen.title'), t('settings.autoQueen.desc'), toggle('autoQueen', t('settings.autoQueen.aria'))),
          h('div', { class: 'setting-row hub-setting-stack' },
            h('div', { class: 'setting-row-text' }, h('div', { class: 'setting-row-title' }, t('settings.animation.title')), h('div', { class: 'setting-row-desc' }, t('settings.animation.desc'))),
            h('div', { class: 'stack-sm hub-anim-ctl' }, speedSeg, h('div', { class: 'row-sm' }, range, rangeVal))),
          row(t('settings.notation.title'), h('span', null, t('settings.notation.desc'), ' ', notationExample), segmented('moveNotation', [['san', t('settings.notation.letters')], ['figurine', t('settings.notation.figurines')]], t('settings.notation.title')))),
        h('section', { class: 'card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('play') + `<span>${t('settings.sections.playing')}</span>` })),
          row(t('settings.evalBar.title'), t('settings.evalBar.desc'), toggle('showEvalBar', t('settings.evalBar.aria')))),
        soundSection,
        a11ySection,
        firstWeekSection.el,
        dataSection.el,
        phoneSection.el,
        h('section', { class: 'card hub-danger' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('alert') + `<span>${t('settings.sections.danger')}</span>` })),
          row(t('settings.reset.button'), t('settings.reset.desc'), resetSettingsBtn),
          row(t('settings.deleteGames.button'), t('settings.deleteGames.desc'), deleteGamesBtn),
          row(t('settings.resetPuzzles.button'), t('settings.resetPuzzles.desc'), resetPuzzlesBtn),
          row(t('settings.clearData.button'), t('settings.clearData.desc'), clearLocalBtn))),
      h('aside', { class: 'hub-settings-aside' },
        h('div', { class: 'card hub-preview-card' },
          h('div', { class: 'card-header' }, h('div', { class: 'card-title', html: icon('eye') + `<span>${t('settings.preview.title')}</span>` }), resetPreviewBtn),
          previewSlot,
          h('p', { class: 'subtle text-xs mt-2' }, t('settings.preview.hint'))))));
  root.appendChild(page);

  const syncAll = () => { const s = getSettings(); for (const fn of syncers) fn(s); };
  syncAll();
  bag.add(onSettingsChange((s, key) => {
    syncAll();
    if (RECREATE_KEYS.has(key)) {
      if (key === 'pieceSet' && !board) previewSlot.innerHTML = fenBoardSvg(previewFen, { label: t('settings.boardPreview'), pieceSet: s.pieceSet });
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
