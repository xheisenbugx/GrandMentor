// GrandMentor — tiny synthesized sound effects (WebAudio, no audio files).
// Contract: docs/CONTRACT.md §5 — playSound('move'|'capture'|'check'|'castle'|'promote'|
// 'gameEnd'|'illegal'|'correct'|'wrong'|'notify').
//
// Memory: one shared AudioContext, created lazily after the first user gesture, one cached
// noise buffer. Every voice is a handful of short-lived nodes that are disconnected when they
// end, so nothing accumulates no matter how many sounds are played.

import { getSetting } from '../settings.js';

/** @type {AudioContext|null} */
let ctx = null;
/** @type {GainNode|null} */
let master = null;
/** @type {AudioBuffer|null} */
let noiseBuf = null;
let gestureSeen = false;
let unlockInstalled = false;
const lastPlayed = new Map(); // name -> timestamp (bounded: one entry per sound name)
const MIN_GAP_MS = 35;        // de-duplicate bursts of the same sound
const MAX_VOICES = 12;        // hard cap on concurrently scheduled voices
let activeVoices = 0;

export const SOUND_NAMES = Object.freeze([
  'move', 'capture', 'check', 'castle', 'promote', 'gameEnd', 'illegal', 'correct', 'wrong', 'notify',
]);

function hasUserActivation() {
  if (gestureSeen) return true;
  try {
    // Chrome/Edge/Safari 16.4+/Firefox 120+.
    if (navigator.userActivation && navigator.userActivation.hasBeenActive) gestureSeen = true;
  } catch { /* ignore */ }
  return gestureSeen;
}

/** Install one-shot listeners that mark the first user gesture and resume audio. */
function installUnlock() {
  if (unlockInstalled || typeof window === 'undefined') return;
  unlockInstalled = true;
  const events = ['pointerdown', 'keydown', 'touchend'];
  const onGesture = () => {
    gestureSeen = true;
    for (const ev of events) window.removeEventListener(ev, onGesture, true);
    if (ctx && ctx.state === 'suspended') ctx.resume().catch(() => {});
  };
  for (const ev of events) window.addEventListener(ev, onGesture, { capture: true, passive: true });
}
installUnlock();

function getCtx() {
  if (ctx) {
    if (ctx.state === 'suspended') ctx.resume().catch(() => {});
    return ctx.state === 'closed' ? null : ctx;
  }
  if (!hasUserActivation()) return null; // browsers would block/warn before a gesture
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) return null;
  try {
    ctx = new AC({ latencyHint: 'interactive' });
  } catch {
    return null;
  }
  master = ctx.createGain();
  master.gain.value = 0.6;
  master.connect(ctx.destination);
  return ctx;
}

function getNoise(c) {
  if (noiseBuf && noiseBuf.sampleRate === c.sampleRate) return noiseBuf;
  const len = Math.floor(c.sampleRate * 0.25);
  noiseBuf = c.createBuffer(1, len, c.sampleRate);
  const data = noiseBuf.getChannelData(0);
  let seed = 1234567;
  for (let i = 0; i < len; i++) {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff; // deterministic, cheap
    data[i] = (seed / 0x3fffffff) - 1;
  }
  return noiseBuf;
}

/** Track a source node; disconnect the whole voice when it ends. */
function track(src, nodes) {
  activeVoices++;
  src.onended = () => {
    activeVoices = Math.max(0, activeVoices - 1);
    src.onended = null;
    for (const n of nodes) { try { n.disconnect(); } catch { /* already */ } }
  };
}

/** A woody "tock": filtered noise burst + low sine thump. */
function knock(c, t, { freq = 1800, q = 3, dur = 0.07, gain = 0.9, thump = 180, thumpGain = 0.5 } = {}) {
  const src = c.createBufferSource();
  src.buffer = getNoise(c);
  const bp = c.createBiquadFilter();
  bp.type = 'bandpass';
  bp.frequency.value = freq;
  bp.Q.value = q;
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(gain, t + 0.003);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  src.connect(bp).connect(g).connect(master);
  track(src, [src, bp, g]);
  src.start(t);
  src.stop(t + dur + 0.02);

  if (thump > 0) {
    const osc = c.createOscillator();
    osc.type = 'sine';
    osc.frequency.setValueAtTime(thump, t);
    osc.frequency.exponentialRampToValueAtTime(thump * 0.55, t + dur);
    const og = c.createGain();
    og.gain.setValueAtTime(0.0001, t);
    og.gain.exponentialRampToValueAtTime(thumpGain, t + 0.004);
    og.gain.exponentialRampToValueAtTime(0.0001, t + dur + 0.02);
    osc.connect(og).connect(master);
    track(osc, [osc, og]);
    osc.start(t);
    osc.stop(t + dur + 0.04);
  }
}

/** A soft tone with attack/decay envelope. */
function tone(c, t, freq, { dur = 0.15, gain = 0.25, type = 'sine', slideTo = 0 } = {}) {
  const osc = c.createOscillator();
  osc.type = type;
  osc.frequency.setValueAtTime(freq, t);
  if (slideTo) osc.frequency.exponentialRampToValueAtTime(slideTo, t + dur);
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(gain, t + 0.01);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(g).connect(master);
  track(osc, [osc, g]);
  osc.start(t);
  osc.stop(t + dur + 0.03);
}

const VOICES = {
  move(c, t) { knock(c, t, { freq: 1500, q: 2.5, dur: 0.06, gain: 0.8, thump: 170, thumpGain: 0.45 }); },
  capture(c, t) {
    knock(c, t, { freq: 2400, q: 1.6, dur: 0.05, gain: 0.9, thump: 230, thumpGain: 0.4 });
    knock(c, t + 0.045, { freq: 1300, q: 2.2, dur: 0.08, gain: 0.85, thump: 150, thumpGain: 0.55 });
  },
  castle(c, t) {
    knock(c, t, { freq: 1500, q: 2.5, dur: 0.055, gain: 0.75, thump: 170, thumpGain: 0.4 });
    knock(c, t + 0.09, { freq: 1350, q: 2.5, dur: 0.06, gain: 0.75, thump: 160, thumpGain: 0.4 });
  },
  check(c, t) {
    knock(c, t, { freq: 1600, q: 2.5, dur: 0.06, gain: 0.7, thump: 170, thumpGain: 0.35 });
    tone(c, t + 0.02, 880, { dur: 0.14, gain: 0.12, type: 'triangle' });
    tone(c, t + 0.1, 1175, { dur: 0.18, gain: 0.12, type: 'triangle' });
  },
  promote(c, t) {
    knock(c, t, { freq: 1500, q: 2.5, dur: 0.06, gain: 0.7, thump: 170, thumpGain: 0.35 });
    [523, 659, 784, 1047].forEach((f, i) => tone(c, t + 0.05 + i * 0.06, f, { dur: 0.16, gain: 0.1, type: 'triangle' }));
  },
  gameEnd(c, t) {
    [392, 494, 587].forEach((f) => tone(c, t, f, { dur: 0.7, gain: 0.09, type: 'sine' }));
    [523, 659, 784].forEach((f) => tone(c, t + 0.22, f, { dur: 0.9, gain: 0.08, type: 'sine' }));
  },
  illegal(c, t) { tone(c, t, 160, { dur: 0.14, gain: 0.18, type: 'square', slideTo: 120 }); },
  correct(c, t) {
    tone(c, t, 784, { dur: 0.14, gain: 0.16, type: 'triangle' });
    tone(c, t + 0.09, 1175, { dur: 0.24, gain: 0.16, type: 'triangle' });
  },
  wrong(c, t) {
    tone(c, t, 392, { dur: 0.16, gain: 0.16, type: 'sawtooth', slideTo: 330 });
    tone(c, t + 0.12, 311, { dur: 0.26, gain: 0.14, type: 'sawtooth', slideTo: 247 });
  },
  notify(c, t) {
    tone(c, t, 1319, { dur: 0.22, gain: 0.12, type: 'sine' });
    tone(c, t + 0.08, 1760, { dur: 0.3, gain: 0.08, type: 'sine' });
  },
};

/** Whether sounds are enabled in user settings. */
export function isSoundEnabled() {
  try { return getSetting('sounds') !== false; } catch { return true; }
}

/**
 * Play a named sound. No-op when sounds are disabled, before the first user gesture,
 * when WebAudio is unavailable, or when the same sound was just played.
 * @param {string} name
 * @param {{force?: boolean}} [opts] force = ignore the settings toggle (e.g. a "test sound" button)
 */
export function playSound(name, opts = {}) {
  const voice = VOICES[name];
  if (!voice) return;
  if (!opts.force && !isSoundEnabled()) return;
  if (typeof document !== 'undefined' && document.hidden) return;
  const c = getCtx();
  if (!c || !master) return;
  if (activeVoices >= MAX_VOICES) return;
  const now = performance.now();
  const last = lastPlayed.get(name) || 0;
  if (now - last < MIN_GAP_MS) return;
  lastPlayed.set(name, now);
  try {
    voice(c, c.currentTime + 0.005);
  } catch (e) {
    console.warn('[sound] failed', e);
  }
}

/** Call from a user-gesture handler to make sure audio is ready (optional). */
export function unlockAudio() {
  gestureSeen = true;
  getCtx();
}
