// GrandMentor — synthesized sound effects (WebAudio, no audio files).
// Contract: docs/CONTRACT.md §5 — playSound('move'|'capture'|'check'|'castle'|'promote'|
// 'gameEnd'|'illegal'|'correct'|'wrong'|'notify').
//
// Piece sounds are modelled on a real wooden piece meeting a wooden board: a felt-softened
// contact click, a few short inharmonic wood resonances, a low board "thump" and a touch of
// room. A capture is two impacts — the hard clack of piece against piece, then the landing.
// Every voice is rendered once into an AudioBuffer with an OfflineAudioContext, so playing a
// sound is a single buffer source (lowest latency, no per-play graph building). Each play
// varies pitch and level slightly so repeated moves never sound machine-made.
//
// Memory: one shared AudioContext (created lazily after the first user gesture), one cached
// buffer per sound, and one short-lived source + gain per play that are disconnected on end.

import { getSetting } from '../settings.js';

/** @type {AudioContext|null} */
let ctx = null;
/** @type {GainNode|null} */
let master = null;
let gestureSeen = false;
let unlockInstalled = false;
/** @type {Map<string, AudioBuffer>} rendered voices (bounded: one per sound name) */
const buffers = new Map();
let renderPromise = null;
const lastPlayed = new Map(); // name -> timestamp (bounded: one entry per sound name)
const MIN_GAP_MS = 35;        // de-duplicate bursts of the same sound
const MAX_VOICES = 12;        // hard cap on concurrently playing voices
let activeVoices = 0;

export const SOUND_NAMES = Object.freeze([
  'move', 'capture', 'check', 'castle', 'promote', 'gameEnd', 'illegal', 'correct', 'wrong', 'notify',
]);

// Piece sounds vary in pitch per play; UI chimes stay exactly in tune.
const VARIED = new Set(['move', 'capture', 'castle', 'check', 'promote']);

function hasUserActivation() {
  if (gestureSeen) return true;
  try {
    if (navigator.userActivation && navigator.userActivation.hasBeenActive) gestureSeen = true;
  } catch { /* ignore */ }
  return gestureSeen;
}

/** One-shot listeners: mark the first user gesture, resume audio and pre-render the voices. */
function installUnlock() {
  if (unlockInstalled || typeof window === 'undefined') return;
  unlockInstalled = true;
  const events = ['pointerdown', 'keydown', 'touchend'];
  const onGesture = () => {
    gestureSeen = true;
    for (const ev of events) window.removeEventListener(ev, onGesture, true);
    getCtx(); // creates the context and starts rendering before the first move lands
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
  const AC = window.AudioContext || /** @type {any} */ (window).webkitAudioContext;
  if (!AC) return null;
  try {
    ctx = new AC({ latencyHint: 'interactive' });
  } catch {
    return null;
  }
  // Gentle glue so stacked sounds never clip.
  const comp = ctx.createDynamicsCompressor();
  comp.threshold.value = -10;
  comp.knee.value = 8;
  comp.ratio.value = 4;
  comp.attack.value = 0.002;
  comp.release.value = 0.12;
  master = ctx.createGain();
  master.gain.value = 0.9;
  master.connect(comp).connect(ctx.destination);
  renderAll(ctx.sampleRate);
  return ctx;
}

// ---------------------------------------------------------------- synthesis

/** Deterministic white noise (same buffer every time: renders are reproducible). */
function noiseBuffer(c, seconds) {
  const len = Math.max(1, Math.floor(c.sampleRate * seconds));
  const buf = c.createBuffer(1, len, c.sampleRate);
  const data = buf.getChannelData(0);
  let seed = 22222;
  for (let i = 0; i < len; i++) {
    seed = (seed * 1103515245 + 12345) & 0x7fffffff;
    data[i] = (seed / 0x3fffffff) - 1;
  }
  return buf;
}

/** A small, warm room: decaying stereo noise, darkened with a one-pole low-pass. */
function roomImpulse(c) {
  const len = Math.floor(c.sampleRate * 0.32);
  const ir = c.createBuffer(2, len, c.sampleRate);
  for (let ch = 0; ch < 2; ch++) {
    const d = ir.getChannelData(ch);
    let seed = 9001 + ch * 7919;
    let lp = 0;
    for (let i = 0; i < len; i++) {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      const n = (seed / 0x3fffffff) - 1;
      lp += 0.35 * (n - lp);
      const t = i / c.sampleRate;
      d[i] = lp * Math.exp(-t / 0.055) * (t < 0.004 ? t / 0.004 : 1);
    }
  }
  return ir;
}

/** A damped sinusoid: one resonant mode of the wood. tau = decay time constant (s). */
function mode(c, out, t, freq, amp, tau) {
  const osc = c.createOscillator();
  osc.type = 'sine';
  osc.frequency.setValueAtTime(freq, t);
  // Real wood drops a hair in pitch as the impact energy dissipates.
  osc.frequency.exponentialRampToValueAtTime(freq * 0.985, t + tau * 4);
  const g = c.createGain();
  g.gain.setValueAtTime(0, t);
  g.gain.linearRampToValueAtTime(amp, t + 0.0012);
  g.gain.setTargetAtTime(0, t + 0.0012, tau);
  osc.connect(g).connect(out);
  osc.start(t);
  osc.stop(t + 0.0012 + tau * 9);
}

/** A filtered noise burst: the contact "click" / felt scuff of an impact. */
function burst(c, out, noise, t, { type = 'highpass', freq = 2000, q = 0.7, amp = 0.4, tau = 0.004 } = {}) {
  const src = c.createBufferSource();
  src.buffer = noise;
  const f = c.createBiquadFilter();
  f.type = type;
  f.frequency.value = freq;
  f.Q.value = q;
  const g = c.createGain();
  g.gain.setValueAtTime(0, t);
  g.gain.linearRampToValueAtTime(amp, t + 0.0006);
  g.gain.setTargetAtTime(0, t + 0.0006, tau);
  src.connect(f).connect(g).connect(out);
  src.start(t);
  src.stop(t + tau * 10 + 0.01);
}

/**
 * A piece landing on the board.
 * weight scales the low body; bright scales the high wood modes and contact click.
 */
function placePiece(c, out, noise, t, { level = 1, weight = 1, bright = 1, pitch = 1 } = {}) {
  const L = level;
  // Board thump (the whole board resonating) and the piece body.
  mode(c, out, t, 150 * pitch, 0.42 * L * weight, 0.030);
  mode(c, out, t, 310 * pitch, 0.30 * L * weight, 0.020);
  // Wood modes: inharmonic, short, mid-heavy — the "tock".
  mode(c, out, t, 720 * pitch, 0.26 * L, 0.013);
  mode(c, out, t, 1180 * pitch, 0.22 * L * bright, 0.009);
  mode(c, out, t, 1930 * pitch, 0.13 * L * bright, 0.006);
  mode(c, out, t, 3150 * pitch, 0.06 * L * bright, 0.0035);
  // Felt-padded contact: soft band-limited scuff plus a tiny bright tick.
  burst(c, out, noise, t, { type: 'bandpass', freq: 1400 * pitch, q: 0.9, amp: 0.30 * L, tau: 0.006 });
  burst(c, out, noise, t, { type: 'highpass', freq: 4200, q: 0.7, amp: 0.10 * L * bright, tau: 0.0018 });
}

/** Two hard pieces knocking together (the captured piece being struck). */
function clack(c, out, noise, t, { level = 1, pitch = 1 } = {}) {
  const L = level;
  mode(c, out, t, 1650 * pitch, 0.26 * L, 0.007);
  mode(c, out, t, 2480 * pitch, 0.22 * L, 0.005);
  mode(c, out, t, 3720 * pitch, 0.14 * L, 0.0035);
  mode(c, out, t, 5300 * pitch, 0.07 * L, 0.0022);
  burst(c, out, noise, t, { type: 'highpass', freq: 2600, q: 0.8, amp: 0.42 * L, tau: 0.0028 });
}

/** A soft tone with attack/decay (UI chimes). */
function tone(c, out, t, freq, { dur = 0.15, gain = 0.25, type = 'sine', slideTo = 0 } = {}) {
  const osc = c.createOscillator();
  osc.type = type;
  osc.frequency.setValueAtTime(freq, t);
  if (slideTo) osc.frequency.exponentialRampToValueAtTime(slideTo, t + dur);
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(gain, t + 0.01);
  g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
  osc.connect(g).connect(out);
  osc.start(t);
  osc.stop(t + dur + 0.03);
}

/** Voice recipes: (context, dry bus, room bus, noise, start time). Durations in seconds. */
const VOICES = {
  move: { dur: 0.45, room: 0.16, peak: 0.75, play(c, dry, wet, n, t) { placePiece(c, dry, n, t); placePiece(c, wet, n, t); } },
  capture: {
    dur: 0.5, room: 0.18, peak: 0.9,
    play(c, dry, wet, n, t) {
      // Attacker strikes the victim, then lands a little harder and brighter than a quiet move.
      for (const out of [dry, wet]) {
        clack(c, out, n, t, { level: 1 });
        placePiece(c, out, n, t + 0.022, { level: 1.2, weight: 1.15, bright: 1.25, pitch: 1.04 });
      }
    },
  },
  castle: {
    dur: 0.6, room: 0.16, peak: 0.75,
    play(c, dry, wet, n, t) {
      for (const out of [dry, wet]) {
        placePiece(c, out, n, t, { level: 0.95, pitch: 1.02 });            // king
        placePiece(c, out, n, t + 0.105, { level: 0.85, weight: 1.1, pitch: 0.94 }); // rook
      }
    },
  },
  check: {
    dur: 0.6, room: 0.2, peak: 0.8,
    play(c, dry, wet, n, t) {
      for (const out of [dry, wet]) placePiece(c, out, n, t, { level: 1.1, bright: 1.35, pitch: 1.06 });
      tone(c, dry, t + 0.015, 988, { dur: 0.16, gain: 0.07, type: 'triangle' });
      tone(c, dry, t + 0.085, 1319, { dur: 0.22, gain: 0.06, type: 'triangle' });
    },
  },
  promote: {
    dur: 0.8, room: 0.2, peak: 0.75,
    play(c, dry, wet, n, t) {
      for (const out of [dry, wet]) placePiece(c, out, n, t, { level: 1, bright: 1.1 });
      [523, 659, 784, 1047].forEach((f, i) => tone(c, dry, t + 0.06 + i * 0.06, f, { dur: 0.18, gain: 0.07, type: 'triangle' }));
    },
  },
  gameEnd: {
    dur: 1.4, room: 0.25,
    play(c, dry, wet, n, t) {
      [392, 494, 587].forEach((f) => tone(c, dry, t, f, { dur: 0.7, gain: 0.08 }));
      [523, 659, 784].forEach((f) => tone(c, dry, t + 0.22, f, { dur: 0.9, gain: 0.07 }));
    },
  },
  illegal: { dur: 0.25, room: 0, play(c, dry, wet, n, t) { tone(c, dry, t, 180, { dur: 0.12, gain: 0.12, type: 'triangle', slideTo: 140 }); } },
  correct: {
    dur: 0.45, room: 0.15,
    play(c, dry, wet, n, t) {
      tone(c, dry, t, 784, { dur: 0.14, gain: 0.14, type: 'triangle' });
      tone(c, dry, t + 0.09, 1175, { dur: 0.24, gain: 0.14, type: 'triangle' });
    },
  },
  wrong: {
    dur: 0.5, room: 0.1,
    play(c, dry, wet, n, t) {
      tone(c, dry, t, 392, { dur: 0.16, gain: 0.12, type: 'sawtooth', slideTo: 330 });
      tone(c, dry, t + 0.12, 311, { dur: 0.26, gain: 0.1, type: 'sawtooth', slideTo: 247 });
    },
  },
  notify: {
    dur: 0.5, room: 0.15,
    play(c, dry, wet, n, t) {
      tone(c, dry, t, 1319, { dur: 0.22, gain: 0.1 });
      tone(c, dry, t + 0.08, 1760, { dur: 0.3, gain: 0.07 });
    },
  },
};

/** Render one voice into an AudioBuffer (stereo, with its room tail). */
async function renderVoice(name, sampleRate) {
  const v = VOICES[name];
  const OAC = window.OfflineAudioContext || /** @type {any} */ (window).webkitOfflineAudioContext;
  const c = new OAC(2, Math.ceil(sampleRate * v.dur), sampleRate);
  const dry = c.createGain();
  dry.connect(c.destination);
  const wetIn = c.createGain();
  if (v.room > 0) {
    const verb = c.createConvolver();
    verb.buffer = roomImpulse(c);
    const wetOut = c.createGain();
    wetOut.gain.value = v.room;
    wetIn.connect(verb).connect(wetOut).connect(c.destination);
  }
  v.play(c, dry, wetIn, noiseBuffer(c, 0.2), 0.002);
  const buf = await c.startRendering();
  if (v.peak) normalize(buf, v.peak);
  return buf;
}

/** Scale a rendered buffer so its loudest sample hits `target` (consistent levels, no clipping). */
function normalize(buf, target) {
  let peak = 0;
  for (let ch = 0; ch < buf.numberOfChannels; ch++) {
    for (const x of buf.getChannelData(ch)) peak = Math.max(peak, Math.abs(x));
  }
  if (!peak) return;
  const k = target / peak;
  for (let ch = 0; ch < buf.numberOfChannels; ch++) {
    const d = buf.getChannelData(ch);
    for (let i = 0; i < d.length; i++) d[i] *= k;
  }
}

function renderAll(sampleRate) {
  if (renderPromise) return renderPromise;
  // Piece sounds first: they are what the user hears within the first second.
  const order = ['move', 'capture', 'check', 'castle', 'promote', 'correct', 'wrong', 'illegal', 'notify', 'gameEnd'];
  renderPromise = (async () => {
    for (const name of order) {
      try { buffers.set(name, await renderVoice(name, sampleRate)); } catch (e) { console.warn('[sound] render failed', name, e); }
    }
  })();
  return renderPromise;
}

/** Fallback while buffers are still rendering (first ~50ms after the first gesture). */
function playLive(c, name, t) {
  const v = VOICES[name];
  const g = c.createGain();
  g.connect(master);
  v.play(c, g, g.context.createGain(), noiseBuffer(c, 0.2), t);
  setTimeout(() => { try { g.disconnect(); } catch { /* already */ } }, (v.dur + 0.2) * 1000);
}

// ---------------------------------------------------------------- public API

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
  if (!VOICES[name]) return;
  if (!opts.force && !isSoundEnabled()) return;
  if (typeof document !== 'undefined' && document.hidden) return;
  const c = getCtx();
  if (!c || !master) return;
  if (activeVoices >= MAX_VOICES) return;
  const now = performance.now();
  if (now - (lastPlayed.get(name) || 0) < MIN_GAP_MS) return;
  lastPlayed.set(name, now);
  const t = c.currentTime + 0.002;
  try {
    const buf = buffers.get(name);
    if (!buf) { playLive(c, name, t); return; }
    const src = c.createBufferSource();
    src.buffer = buf;
    const g = c.createGain();
    if (VARIED.has(name)) {
      src.playbackRate.value = 0.96 + Math.random() * 0.08; // ±4% pitch, like different pieces/squares
      g.gain.value = 0.88 + Math.random() * 0.12;
    }
    src.connect(g).connect(master);
    activeVoices++;
    src.onended = () => {
      activeVoices = Math.max(0, activeVoices - 1);
      src.onended = null;
      try { src.disconnect(); g.disconnect(); } catch { /* already */ }
    };
    src.start(t);
  } catch (e) {
    console.warn('[sound] failed', e);
  }
}

/**
 * Render a sound to an AudioBuffer without playing it (previews, waveform checks, tests).
 * @param {string} name @param {number} [sampleRate]
 */
export function renderSound(name, sampleRate = 48000) {
  if (!VOICES[name]) return Promise.reject(new Error('unknown sound ' + name));
  return renderVoice(name, sampleRate);
}

/** Call from a user-gesture handler to make sure audio is ready (optional). */
export function unlockAudio() {
  gestureSeen = true;
  getCtx();
}
