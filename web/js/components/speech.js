// GrandMentor — read moves aloud with the Web Speech API (speechSynthesis).
// Contract: docs/CONTRACT.md §5 (`speakMove`, `speak`, `cancelSpeech`, `speechSupported`).
//
//   speakMove(moveObj, { mine: false })  // "Black knight to f6" when the "Read moves aloud" setting is on
//   speak(t('a11y.speech.sample'))       // say any (translated) text now
//   cancelSpeech()                       // stop talking (page change)
//
// Opt-in only (settings `speakMoves`, `speakOwnMoves`). The voice follows the UI language: the best
// installed voice whose language matches is chosen; without one, the utterance still carries the
// language tag so the browser can pick something sensible. Every call is a no-op when the browser
// has no speech synthesis, and nothing here throws.

import { getSetting } from '../settings.js';
import { getLanguage } from '../i18n.js';
import { describeMove } from './announcer.js';

// UI language -> BCP 47 tag for the utterance (the region is only a hint).
const LANG_TAGS = { en: 'en-US', es: 'es-ES', pt: 'pt-BR', fr: 'fr-FR', de: 'de-DE' };
const MAX_PENDING = 2;   // a burst of moves never builds a long backlog
const MAX_LEN = 300;

let voiceCache = { lang: '', voice: null, count: -1 };
let pending = 0;

function synth() {
  try {
    if (typeof window === 'undefined' || !('speechSynthesis' in window) || typeof window.SpeechSynthesisUtterance !== 'function') return null;
    return window.speechSynthesis;
  } catch { return null; }
}

/** True when this browser can speak. */
export function speechSupported() {
  return !!synth();
}

function uiLang() {
  try { return getLanguage() || 'en'; } catch { return 'en'; }
}

/** Best installed voice for a language code ('es'), or null. Cached until the voice list changes. */
function pickVoice(s, lang) {
  let voices = [];
  try { voices = s.getVoices() || []; } catch { voices = []; }
  if (voiceCache.lang === lang && voiceCache.count === voices.length) return voiceCache.voice;
  const tag = (LANG_TAGS[lang] || lang).toLowerCase();
  const norm = (v) => String(v.lang || '').replace('_', '-').toLowerCase();
  const same = voices.filter((v) => norm(v) === tag || norm(v).startsWith(`${lang}-`) || norm(v) === lang);
  const score = (v) => (norm(v) === tag ? 4 : 0) + (v.localService ? 2 : 0) + (v.default ? 1 : 0);
  const voice = same.sort((a, b) => score(b) - score(a))[0] || null;
  voiceCache = { lang, voice, count: voices.length };
  return voice;
}

/**
 * Whether an installed voice matches the UI language: true / false, or null while the browser
 * hasn't listed its voices yet (or can't speak at all).
 */
export function voiceAvailable() {
  const s = synth();
  if (!s) return null;
  let count = 0;
  try { count = (s.getVoices() || []).length; } catch { count = 0; }
  if (!count) return null;
  return !!pickVoice(s, uiLang());
}

/** Call fn() when the browser's voice list changes. Returns an unsubscribe function. */
export function onVoicesChanged(fn) {
  const s = synth();
  if (!s || typeof s.addEventListener !== 'function') return () => {};
  const handler = () => { try { fn(); } catch (e) { console.error(e); } };
  s.addEventListener('voiceschanged', handler);
  return () => { try { s.removeEventListener('voiceschanged', handler); } catch { /* ignore */ } };
}

/** Say `text` in the UI language. Returns true when it was queued. */
export function speak(text) {
  const s = synth();
  const msg = String(text ?? '').replace(/\s+/g, ' ').trim().slice(0, MAX_LEN);
  if (!s || !msg) return false;
  try {
    // Keep up with the game: drop what hasn't been said yet instead of queuing behind it.
    if (pending >= MAX_PENDING) { s.cancel(); pending = 0; }
    const lang = uiLang();
    const u = new window.SpeechSynthesisUtterance(msg);
    u.lang = LANG_TAGS[lang] || lang;
    const voice = pickVoice(s, lang);
    if (voice) u.voice = voice;
    u.rate = 1;
    const done = () => { pending = Math.max(0, pending - 1); };
    u.onend = done;
    u.onerror = done;
    pending++;
    s.speak(u);
    return true;
  } catch {
    return false;
  }
}

/** Stop speaking and drop anything queued. */
export function cancelSpeech() {
  pending = 0;
  const s = synth();
  if (!s) return;
  try { if (s.speaking || s.pending) s.cancel(); } catch { /* ignore */ }
}

/**
 * Read a move aloud if the user turned it on. `mine` = the user's own move (needs `speakOwnMoves`).
 * @param {object} mv a Board move object or a verbose chess.js move
 */
export function speakMove(mv, { mine = false } = {}) {
  let on = false;
  try { on = getSetting('speakMoves') === true && (!mine || getSetting('speakOwnMoves') === true); } catch { on = false; }
  if (!on || !mv) return false;
  const text = describeMove(mv);
  if (!text) return false;
  return speak(text[0].toLocaleUpperCase() + text.slice(1));
}

// Some browsers load voices asynchronously: forget the cached pick when the list changes.
try {
  const s = synth();
  if (s && typeof s.addEventListener === 'function') s.addEventListener('voiceschanged', () => { voiceCache = { lang: '', voice: null, count: -1 }; });
} catch { /* ignore */ }
