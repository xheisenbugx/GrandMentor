// GrandMentor user settings — persisted in localStorage, applied to <html>.
// Contract: docs/CONTRACT.md §5.

import { DEFAULT_LANGUAGE, isLanguage } from './languages.js';

const STORAGE_KEY = 'grandmentor.settings.v1';

/** Board color themes. Exported so the Settings page can render previews.
 *  `labelKey` is an i18n key resolved at render time (this module must not import i18n.js). */
export const BOARD_THEMES = Object.freeze({
  green: { labelKey: 'settings.boardThemes.green', light: '#ebecd0', dark: '#739552' },
  brown: { labelKey: 'settings.boardThemes.brown', light: '#f0d9b5', dark: '#b58863' },
  blue: { labelKey: 'settings.boardThemes.blue', light: '#dee3e6', dark: '#8ca2ad' },
  purple: { labelKey: 'settings.boardThemes.purple', light: '#efeaf6', dark: '#8877b7' },
  gray: { labelKey: 'settings.boardThemes.gray', light: '#e2e2e0', dark: '#9a9c9e' },
  // High contrast: near-white vs. a deep slate blue (4.6:1 between squares), bold coordinates.
  contrast: { labelKey: 'a11y.settings.boardContrast', light: '#ffffff', dark: '#56708f', coordLight: '#1d2a3a', coordDark: '#ffffff' },
});

/** UI text scale presets (percent of the browser's base font size). */
export const UI_SCALES = Object.freeze([100, 115, 130]);

export const PIECE_SETS = Object.freeze({
  cburnett: { labelKey: 'settings.pieceSets.cburnett' },
  merida: { labelKey: 'settings.pieceSets.merida' },
  alpha: { labelKey: 'settings.pieceSets.alpha' },
});

export const DEFAULTS = Object.freeze({
  boardTheme: 'green',
  pieceSet: 'cburnett',
  sounds: true,
  showCoords: true,
  showLegal: true,
  animationMs: 200,
  showEvalBar: true,
  autoQueen: false,
  theme: 'dark',
  moveNotation: 'san',
  sidebarCollapsed: false,
  language: DEFAULT_LANGUAGE,
  // Accessibility
  highContrast: false,     // stronger borders and text (WCAG AAA body text)
  cbPalette: false,        // colour-blind-safe move classification colours
  motion: 'system',        // 'system' (follow prefers-reduced-motion) | 'reduce' | 'full'
  announceMoves: true,     // screen-reader announcements of moves
  squareNames: false,      // show the square name on hover / keyboard focus
  uiScale: 100,            // UI text scale in percent (UI_SCALES)
});

// Validators keep corrupted storage from breaking the app.
const VALIDATE = {
  boardTheme: (v) => Object.hasOwn(BOARD_THEMES, v),
  pieceSet: (v) => Object.hasOwn(PIECE_SETS, v),
  sounds: (v) => typeof v === 'boolean',
  showCoords: (v) => typeof v === 'boolean',
  showLegal: (v) => typeof v === 'boolean',
  animationMs: (v) => typeof v === 'number' && Number.isFinite(v) && v >= 0 && v <= 1000,
  showEvalBar: (v) => typeof v === 'boolean',
  autoQueen: (v) => typeof v === 'boolean',
  theme: (v) => v === 'dark' || v === 'light',
  moveNotation: (v) => v === 'san' || v === 'figurine',
  sidebarCollapsed: (v) => typeof v === 'boolean',
  language: (v) => isLanguage(v),
  highContrast: (v) => typeof v === 'boolean',
  cbPalette: (v) => typeof v === 'boolean',
  motion: (v) => v === 'system' || v === 'reduce' || v === 'full',
  announceMoves: (v) => typeof v === 'boolean',
  squareNames: (v) => typeof v === 'boolean',
  uiScale: (v) => UI_SCALES.includes(v),
};

const listeners = new Set();
let current = load();

function load() {
  const s = { ...DEFAULTS };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === 'object') {
        for (const [k, v] of Object.entries(parsed)) {
          if (VALIDATE[k] ? VALIDATE[k](v) : false) s[k] = v;
        }
      }
    }
  } catch { /* storage blocked or corrupted: use defaults */ }
  return s;
}

function save() {
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(current)); } catch { /* quota / private mode */ }
}

/** Returns a (frozen) snapshot of all settings. */
export function getSettings() {
  return Object.freeze({ ...current });
}

/** Read one setting. */
export function getSetting(key) {
  return current[key];
}

/**
 * Update a setting; persists, re-applies to <html>, and notifies subscribers.
 * Invalid values are ignored (returns false).
 */
export function setSetting(key, value) {
  if (key === 'animationMs' || key === 'uiScale') value = Number(value);
  const valid = VALIDATE[key] ? VALIDATE[key](value) : true;
  if (!valid) { console.warn(`[settings] invalid value for ${key}:`, value); return false; }
  if (Object.is(current[key], value)) return true;
  current = { ...current, [key]: value };
  save();
  applySettings();
  const snapshot = getSettings();
  for (const fn of Array.from(listeners)) {
    try { fn(snapshot, key, value); } catch (e) { console.error('[settings] listener error', e); }
  }
  return true;
}

/** Restore all defaults. */
export function resetSettings() {
  for (const [k, v] of Object.entries(DEFAULTS)) setSetting(k, v);
}

/**
 * Subscribe to changes: fn(settings, changedKey, value). Returns an unsubscribe function.
 */
export function onSettingsChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Apply theme + board CSS variables to <html>. Called on import and on every change. */
export function applySettings(root = document.documentElement) {
  const s = current;
  root.dataset.theme = s.theme;
  root.dataset.boardTheme = s.boardTheme;
  root.dataset.pieceSet = s.pieceSet;
  root.dataset.coords = s.showCoords ? 'on' : 'off';
  root.dataset.contrast = s.highContrast ? 'high' : 'normal';
  root.dataset.clsPalette = s.cbPalette ? 'cb' : 'default';
  root.dataset.motion = reducedMotion() ? 'reduce' : 'full';
  root.dataset.squareNames = s.squareNames ? 'on' : 'off';
  root.dataset.uiScale = String(s.uiScale);
  root.style.fontSize = s.uiScale === 100 ? '' : `${s.uiScale}%`;
  const bt = BOARD_THEMES[s.boardTheme] || BOARD_THEMES.green;
  root.style.setProperty('--board-light', bt.light);
  root.style.setProperty('--board-dark', bt.dark);
  root.style.setProperty('--board-coord-light', bt.coordLight || bt.dark);
  root.style.setProperty('--board-coord-dark', bt.coordDark || bt.light);
  root.style.setProperty('--anim-ms', `${reducedMotion() ? 0 : s.animationMs}ms`);
  root.style.setProperty('--piece-set-url', `url("/img/pieces/${s.pieceSet}/")`);
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.setAttribute('content', s.theme === 'light' ? '#ffffff' : '#1b1e21');
}

/** The system "reduce motion" preference (prefers-reduced-motion). */
export function systemPrefersReducedMotion() {
  try { return typeof window !== 'undefined' && !!window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches; } catch { return false; }
}

/**
 * True when motion should be reduced: the `motion` setting is 'reduce', or it is 'system' and the
 * OS asks for reduced motion. Pages use this for confetti, smooth scrolling and pop-ins; the board
 * uses it to set its animation time to 0.
 */
export function reducedMotion() {
  const m = current.motion;
  if (m === 'reduce') return true;
  if (m === 'full') return false;
  return systemPrefersReducedMotion();
}

/** `behavior` for scrollIntoView / scrollTo that respects reduced motion. */
export function scrollBehavior() {
  return reducedMotion() ? 'auto' : 'smooth';
}

/** URL of a piece image for the current (or given) set, e.g. pieceUrl('wK'). */
export function pieceUrl(code, set = current.pieceSet) {
  return `/img/pieces/${set}/${code}.svg`;
}

// Keep tabs in sync when another tab changes settings.
if (typeof window !== 'undefined') {
  window.addEventListener('storage', (e) => {
    if (e.key !== STORAGE_KEY) return;
    const next = load();
    const changed = Object.keys(next).filter((k) => !Object.is(next[k], current[k]));
    if (!changed.length) return;
    current = next;
    applySettings();
    const snap = getSettings();
    for (const k of changed) for (const fn of Array.from(listeners)) {
      try { fn(snap, k, snap[k]); } catch (err) { console.error(err); }
    }
  });
  // Follow the OS reduced-motion preference live while `motion` is 'system'.
  try {
    const mq = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)');
    if (mq && mq.addEventListener) {
      mq.addEventListener('change', () => {
        if (current.motion !== 'system') return;
        applySettings();
        const snap = getSettings();
        for (const fn of Array.from(listeners)) { try { fn(snap, 'motion', snap.motion); } catch (err) { console.error(err); } }
      });
    }
  } catch { /* old browsers */ }
  applySettings();
}
