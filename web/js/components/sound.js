// GrandMentor — synthesized sound effects (WebAudio, no audio files).
// Contract: docs/CONTRACT.md §5 — playSound('move'|'capture'|'castle'|'check'|'promote'|'gameStart'|
// 'gameEnd'|'lowTime'|'illegal'|'correct'|'wrong'|'notify').
//
// Every event can be played in several styles (see ../sound-catalog.js): piece events in a
// material (wood, marble, plastic, felt, glass, 8-bit, pop, click) and UI chimes in a timbre
// (chime, bells, marimba, 8-bit, beep). The user picks one per event in Settings; 'none' mutes it.
//
// The default wood sound is modelled on a real wooden piece meeting a wooden board: a
// felt-softened contact click, a few short inharmonic wood resonances, a low board "thump" and
// a touch of room. A capture is two impacts — the hard clack of piece against piece, then the
// landing. Each voice is rendered once into an AudioBuffer with an OfflineAudioContext, so
// playing a sound is a single buffer source (lowest latency, no per-play graph building). Each
// play varies pitch and level slightly so repeated moves never sound machine-made.
//
// Memory: one shared AudioContext (created lazily after the first user gesture), a bounded
// cache of rendered voices (the selected style of every event plus a few previews), and one
// short-lived source + gain per play that are disconnected on end.

import { getSetting, onSettingsChange } from '../settings.js';
import { SOUND_EVENTS, resolveStyle, stylesFor } from '../sound-catalog.js';

/** @type {AudioContext|null} */
let ctx = null;
/** @type {GainNode|null} */
let master = null;
let gestureSeen = false;
let unlockInstalled = false;
/** @type {Map<string, AudioBuffer>} rendered voices keyed "event:style" (bounded: MAX_BUFFERS) */
const buffers = new Map();
let renderPromise = null;
const lastPlayed = new Map(); // name -> timestamp (bounded: one entry per sound name)
const MIN_GAP_MS = 35;        // de-duplicate bursts of the same sound
const MAX_VOICES = 12;        // hard cap on concurrently playing voices
let activeVoices = 0;

export const SOUND_NAMES = Object.freeze(SOUND_EVENTS.map((e) => e.id));

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
  master.gain.value = masterLevel();
  master.connect(comp).connect(ctx.destination);
  renderSelected();
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

/** A tone that slides from f0 to f1 with a fast plucked envelope (8-bit blips, bubble pops). */
function blip(c, out, t, f0, f1, { dur = 0.06, gain = 0.2, type = 'square' } = {}) {
  const osc = c.createOscillator();
  osc.type = type;
  osc.frequency.setValueAtTime(f0, t);
  osc.frequency.exponentialRampToValueAtTime(Math.max(20, f1), t + dur);
  const g = c.createGain();
  g.gain.setValueAtTime(0, t);
  g.gain.linearRampToValueAtTime(gain, t + 0.002);
  g.gain.setValueAtTime(gain, t + dur * 0.6);
  g.gain.linearRampToValueAtTime(0, t + dur);
  osc.connect(g).connect(out);
  osc.start(t);
  osc.stop(t + dur + 0.02);
}

/**
 * A struck object described as resonant modes + noise bursts.
 * modes: [freq, amp, tau, kind] where kind 'b' scales with weight (the low body) and 'h' with
 * brightness. bursts: biquad-filtered noise; `pitched` follows the pitch, kind 'h' brightness.
 */
function struck(c, out, noise, t, spec, { level = 1, weight = 1, bright = 1, pitch = 1 } = {}) {
  for (const [f, a, tau, kind] of spec.modes) {
    mode(c, out, t, f * pitch, a * level * (kind === 'b' ? weight : kind === 'h' ? bright : 1), tau);
  }
  for (const b of spec.bursts || []) {
    burst(c, out, noise, t, { type: b.type, freq: b.pitched ? b.freq * pitch : b.freq, q: b.q ?? 0.7, amp: b.amp * level * (b.kind === 'h' ? bright : 1), tau: b.tau });
  }
}

// ---------------------------------------------------------------- piece materials
// land = a piece touching the board; strike = the attacker knocking the captured piece.
// `wood` is the original GrandMentor sound: a felt-padded wooden piece on a wooden board.

const SPEC = {
  woodLand: {
    modes: [[150, 0.42, 0.030, 'b'], [310, 0.30, 0.020, 'b'], [720, 0.26, 0.013], [1180, 0.22, 0.009, 'h'], [1930, 0.13, 0.006, 'h'], [3150, 0.06, 0.0035, 'h']],
    bursts: [{ type: 'bandpass', freq: 1400, q: 0.9, amp: 0.30, tau: 0.006, pitched: true }, { type: 'highpass', freq: 4200, amp: 0.10, tau: 0.0018, kind: 'h' }],
  },
  woodStrike: {
    modes: [[1650, 0.26, 0.007], [2480, 0.22, 0.005], [3720, 0.14, 0.0035], [5300, 0.07, 0.0022]],
    bursts: [{ type: 'highpass', freq: 2600, q: 0.8, amp: 0.42, tau: 0.0028 }],
  },
  // Stone: harder contact, higher and longer-ringing modes.
  marbleLand: {
    modes: [[190, 0.36, 0.022, 'b'], [455, 0.26, 0.020, 'b'], [1240, 0.24, 0.028], [2210, 0.20, 0.024, 'h'], [3480, 0.13, 0.018, 'h'], [5150, 0.07, 0.012, 'h']],
    bursts: [{ type: 'highpass', freq: 3200, amp: 0.34, tau: 0.0022, kind: 'h' }, { type: 'bandpass', freq: 2000, q: 1.2, amp: 0.12, tau: 0.003, pitched: true }],
  },
  marbleStrike: {
    modes: [[2350, 0.28, 0.022], [3640, 0.22, 0.017], [5080, 0.13, 0.012], [6900, 0.06, 0.008]],
    bursts: [{ type: 'highpass', freq: 3800, amp: 0.40, tau: 0.0022 }],
  },
  // Hollow plastic club set on a roll-up vinyl board: short, clicky, little body.
  plasticLand: {
    modes: [[380, 0.34, 0.011, 'b'], [860, 0.32, 0.008], [1720, 0.20, 0.005, 'h'], [2900, 0.10, 0.003, 'h']],
    bursts: [{ type: 'bandpass', freq: 2300, q: 1.4, amp: 0.36, tau: 0.0032, pitched: true }, { type: 'highpass', freq: 5000, amp: 0.08, tau: 0.0012, kind: 'h' }],
  },
  plasticStrike: {
    modes: [[1900, 0.30, 0.006], [3050, 0.20, 0.004], [4400, 0.10, 0.0028]],
    bursts: [{ type: 'highpass', freq: 3000, amp: 0.45, tau: 0.002 }],
  },
  // Thick felt: a muffled thump with almost no click.
  feltLand: {
    modes: [[118, 0.50, 0.034, 'b'], [245, 0.32, 0.024, 'b'], [510, 0.14, 0.012], [860, 0.05, 0.006, 'h']],
    bursts: [{ type: 'lowpass', freq: 900, amp: 0.40, tau: 0.009, pitched: true }],
  },
  feltStrike: {
    modes: [[880, 0.16, 0.008], [1320, 0.10, 0.006]],
    bursts: [{ type: 'lowpass', freq: 1900, amp: 0.30, tau: 0.005 }],
  },
  // Glass pieces: a soft knock and a bright, ringing "tink".
  glassLand: {
    modes: [[210, 0.22, 0.014, 'b'], [1520, 0.18, 0.070], [3180, 0.13, 0.055, 'h'], [4890, 0.07, 0.040, 'h'], [6900, 0.035, 0.030, 'h']],
    bursts: [{ type: 'highpass', freq: 5500, amp: 0.14, tau: 0.0014, kind: 'h' }],
  },
  glassStrike: {
    modes: [[2640, 0.22, 0.090], [4310, 0.14, 0.065], [6120, 0.07, 0.045]],
    bursts: [{ type: 'highpass', freq: 6000, amp: 0.20, tau: 0.0012 }],
  },
  // A tiny UI tick: present but out of the way.
  clickLand: {
    modes: [[2400, 0.18, 0.003], [150, 0.18, 0.010, 'b']],
    bursts: [{ type: 'highpass', freq: 3000, amp: 0.35, tau: 0.0012 }],
  },
  clickStrike: {
    modes: [[3400, 0.18, 0.0025]],
    bursts: [{ type: 'highpass', freq: 4500, amp: 0.35, tau: 0.001 }],
  },
};

const fromSpec = (land, strike) => ({
  land: (c, out, n, t, o) => struck(c, out, n, t, SPEC[land], o),
  strike: (c, out, n, t, o) => struck(c, out, n, t, SPEC[strike], o),
});

/**
 * Piece materials. room/dur scale the event's reverb send and length; accent is the waveform
 * (and level) of the little tune that check and promotion add; level evens out loudness.
 */
const MATERIALS = {
  wood: { ...fromSpec('woodLand', 'woodStrike'), room: 1, dur: 1, accent: ['triangle', 1] },
  marble: { ...fromSpec('marbleLand', 'marbleStrike'), room: 1.4, dur: 1.3, accent: ['sine', 1.1] },
  plastic: { ...fromSpec('plasticLand', 'plasticStrike'), room: 0.8, dur: 1, accent: ['triangle', 0.8] },
  felt: { ...fromSpec('feltLand', 'feltStrike'), room: 0.7, dur: 1, accent: ['sine', 0.7] },
  glass: { ...fromSpec('glassLand', 'glassStrike'), room: 1.6, dur: 1.5, accent: ['sine', 1.1] },
  retro: {
    land: (c, out, n, t, { level = 1, weight = 1, pitch = 1 } = {}) => blip(c, out, t, 330 * pitch, 165 * pitch, { dur: 0.06, gain: 0.2 * level * Math.min(weight, 1.2) }),
    strike: (c, out, n, t, { level = 1, pitch = 1 } = {}) => {
      burst(c, out, n, t, { type: 'lowpass', freq: 3000, amp: 0.3 * level, tau: 0.01 });
      blip(c, out, t, 880 * pitch, 220 * pitch, { dur: 0.07, gain: 0.14 * level });
    },
    room: 0, dur: 1, accent: ['square', 0.5], level: 0.5, // square waves sound much louder at the same peak
  },
  pop: {
    land: (c, out, n, t, { level = 1, pitch = 1 } = {}) => {
      blip(c, out, t, 820 * pitch, 260 * pitch, { dur: 0.045, gain: 0.5 * level, type: 'sine' });
      burst(c, out, n, t, { type: 'highpass', freq: 4000, amp: 0.05 * level, tau: 0.001 });
    },
    strike: (c, out, n, t, { level = 1, pitch = 1 } = {}) => blip(c, out, t, 1300 * pitch, 500 * pitch, { dur: 0.035, gain: 0.4 * level, type: 'sine' }),
    room: 0.5, dur: 1, accent: ['sine', 0.9], level: 0.65,
  },
  click: { ...fromSpec('clickLand', 'clickStrike'), room: 0.5, dur: 1, accent: ['sine', 0.6] },
};

/** Piece events: what happens on the board, in terms of the material's land/strike. */
const PIECE_EVENTS = {
  move: { dur: 0.45, room: 0.16, peak: 0.75, play(M, c, dry, wet, n, t) { for (const out of [dry, wet]) M.land(c, out, n, t); } },
  capture: {
    dur: 0.5, room: 0.18, peak: 0.9,
    play(M, c, dry, wet, n, t) {
      // Attacker strikes the victim, then lands a little harder and brighter than a quiet move.
      for (const out of [dry, wet]) {
        M.strike(c, out, n, t, { level: 1 });
        M.land(c, out, n, t + 0.022, { level: 1.2, weight: 1.15, bright: 1.25, pitch: 1.04 });
      }
    },
  },
  castle: {
    dur: 0.6, room: 0.16, peak: 0.75,
    play(M, c, dry, wet, n, t) {
      for (const out of [dry, wet]) {
        M.land(c, out, n, t, { level: 0.95, pitch: 1.02 });                   // king
        M.land(c, out, n, t + 0.105, { level: 0.85, weight: 1.1, pitch: 0.94 }); // rook
      }
    },
  },
  check: {
    dur: 0.6, room: 0.2, peak: 0.8,
    play(M, c, dry, wet, n, t) {
      for (const out of [dry, wet]) M.land(c, out, n, t, { level: 1.1, bright: 1.35, pitch: 1.06 });
      const [type, k] = M.accent;
      tone(c, dry, t + 0.015, 988, { dur: 0.16, gain: 0.07 * k, type });
      tone(c, dry, t + 0.085, 1319, { dur: 0.22, gain: 0.06 * k, type });
    },
  },
  promote: {
    dur: 0.8, room: 0.2, peak: 0.75,
    play(M, c, dry, wet, n, t) {
      for (const out of [dry, wet]) M.land(c, out, n, t, { level: 1, bright: 1.1 });
      const [type, k] = M.accent;
      [523, 659, 784, 1047].forEach((f, i) => tone(c, dry, t + 0.06 + i * 0.06, f, { dur: 0.18, gain: 0.07 * k, type }));
    },
  },
};

// ---------------------------------------------------------------- chime timbres
// Each chime event is a short tune: [start, freq, dur, gain, waveform, slideTo?]. The waveform is
// what the default `chime` timbre uses; the other timbres play the same notes their own way.

const CHIME_EVENTS = {
  gameStart: { dur: 0.7, room: 0.2, notes: [[0, 523, 0.16, 0.09, 'triangle'], [0.1, 659, 0.16, 0.09, 'triangle'], [0.2, 784, 0.32, 0.1, 'triangle']] },
  gameEnd: {
    dur: 1.4, room: 0.25,
    notes: [[0, 392, 0.7, 0.08, 'sine'], [0, 494, 0.7, 0.08, 'sine'], [0, 587, 0.7, 0.08, 'sine'],
      [0.22, 523, 0.9, 0.07, 'sine'], [0.22, 659, 0.9, 0.07, 'sine'], [0.22, 784, 0.9, 0.07, 'sine']],
  },
  lowTime: { dur: 0.5, room: 0.08, notes: [[0, 1568, 0.07, 0.11, 'triangle'], [0.14, 1568, 0.07, 0.11, 'triangle'], [0.28, 1568, 0.1, 0.11, 'triangle']] },
  illegal: { dur: 0.25, room: 0, notes: [[0, 180, 0.12, 0.12, 'triangle', 140]] },
  correct: { dur: 0.45, room: 0.15, notes: [[0, 784, 0.14, 0.14, 'triangle'], [0.09, 1175, 0.24, 0.14, 'triangle']] },
  wrong: { dur: 0.5, room: 0.1, notes: [[0, 392, 0.16, 0.12, 'sawtooth', 330], [0.12, 311, 0.26, 0.1, 'sawtooth', 247]] },
  notify: { dur: 0.5, room: 0.15, notes: [[0, 1319, 0.22, 0.1, 'sine'], [0.08, 1760, 0.3, 0.07, 'sine']] },
};

/** A sine partial with an exponential decay (bells, marimba bars). */
function partial(c, out, t, freq, slideRatio, amp, tau, len) {
  const osc = c.createOscillator();
  osc.type = 'sine';
  osc.frequency.setValueAtTime(freq, t);
  if (slideRatio !== 1) osc.frequency.exponentialRampToValueAtTime(freq * slideRatio, t + len * 0.6);
  const g = c.createGain();
  g.gain.setValueAtTime(0, t);
  g.gain.linearRampToValueAtTime(amp, t + 0.002);
  g.gain.setTargetAtTime(0, t + 0.002, tau);
  osc.connect(g).connect(out);
  osc.start(t);
  osc.stop(t + len);
}

/** Timbres: how one note of a chime tune is played. dur scales the event's length. */
const TIMBRES = {
  chime: { dur: 1, room: 1, note: (c, out, t, [, f, d, g, type, slide]) => tone(c, out, t, f, { dur: d, gain: g, type, slideTo: slide || 0 }) },
  bell: {
    dur: 1.8, room: 1.3,
    note(c, out, t, [, f, d, g, , slide]) {
      const r = slide ? slide / f : 1;
      [[1, 1], [2.76, 0.45], [5.4, 0.22], [8.93, 0.1]].forEach(([m, a], i) => partial(c, out, t, f * m, r, g * a * 0.5, (d * 0.9 + 0.12) / (1 + i * 0.7), d * 2.4 + 0.2));
    },
  },
  marimba: {
    dur: 0.9, room: 0.9,
    note(c, out, t, [, f, d, g, , slide]) {
      const r = slide ? slide / f : 1;
      const tau = Math.min(0.16, 0.05 + 40 / f);
      partial(c, out, t, f, r, g * 0.6, tau, Math.min(d, 0.5) + 0.3);
      partial(c, out, t, f * 3.93, r, g * 0.14, tau * 0.35, 0.2);
      partial(c, out, t, f * 9.9, r, g * 0.025, tau * 0.15, 0.08);
    },
  },
  retro: { dur: 1, room: 0, note: (c, out, t, [, f, d, g, , slide]) => blip(c, out, t, f, slide || f, { dur: Math.max(0.06, d), gain: g * 0.3, type: 'square' }) },
  beep: { dur: 0.8, room: 0.3, note: (c, out, t, [, f, d, g, , slide]) => blip(c, out, t, f, slide || f, { dur: Math.min(Math.max(0.05, d), 0.14), gain: g * 0.4, type: 'sine' }) },
};

/** The recipe for an event in a style: { dur, room, peak?, play(c, dry, wet, noise, t) }, or null. */
function recipe(name, style) {
  const pe = PIECE_EVENTS[name];
  if (pe) {
    const M = MATERIALS[style];
    if (!M) return null;
    return { dur: pe.dur * M.dur, room: pe.room * M.room, peak: pe.peak * (M.level ?? 1), play: (c, dry, wet, n, t) => pe.play(M, c, dry, wet, n, t) };
  }
  const ce = CHIME_EVENTS[name];
  const T = ce && TIMBRES[style];
  if (!T) return null;
  return {
    dur: ce.dur * T.dur + (style === 'bell' ? 0.4 : 0), room: ce.room * T.room,
    play(c, dry, wet, n, t) {
      for (const note of ce.notes) {
        T.note(c, dry, t + note[0], note);
        // The classic chime stays dry (as it always was); the other timbres get a little room.
        if (style !== 'chime' && ce.room * T.room > 0) T.note(c, wet, t + note[0], note);
      }
    },
  };
}

/** Render one voice into an AudioBuffer (stereo, with its room tail). */
async function renderVoice(name, style, sampleRate) {
  const v = recipe(name, style);
  if (!v) throw new Error(`no sound ${name}:${style}`);
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

// ---------------------------------------------------------------- buffer cache

const voiceKey = (name, style) => `${name}:${style}`;
const MAX_BUFFERS = 40;     // the selected voices (12) plus recently previewed ones
const rendering = new Map(); // key -> Promise<AudioBuffer|null> (in flight only)

/** Render (once) and cache a voice. Resolves to the buffer, or null on failure. */
function ensureVoice(name, style) {
  const key = voiceKey(name, style);
  if (buffers.has(key)) return Promise.resolve(buffers.get(key));
  if (rendering.has(key)) return rendering.get(key);
  if (!ctx) return Promise.resolve(null);
  const p = renderVoice(name, style, ctx.sampleRate)
    .then((buf) => { buffers.set(key, buf); trimBuffers(); return buf; })
    .catch((e) => { console.warn('[sound] render failed', key, e); return null; })
    .finally(() => rendering.delete(key));
  rendering.set(key, p);
  return p;
}

/** Keep the cache bounded: drop the oldest voices that are not currently selected. */
function trimBuffers() {
  if (buffers.size <= MAX_BUFFERS) return;
  const keep = new Set(SOUND_NAMES.map((n) => voiceKey(n, currentStyle(n))));
  for (const key of buffers.keys()) {
    if (buffers.size <= MAX_BUFFERS) break;
    if (!keep.has(key)) buffers.delete(key);
  }
}

function currentStyle(name) {
  let picks = null;
  try { picks = getSetting('soundPicks'); } catch { /* settings unavailable */ }
  return resolveStyle(picks, name);
}

/** Pre-render the selected voice of every event, piece sounds first (heard within a second). */
function renderSelected() {
  const order = ['move', 'capture', 'check', 'castle', 'promote', 'correct', 'wrong', 'illegal', 'lowTime', 'notify', 'gameStart', 'gameEnd'];
  if (renderPromise) return renderPromise;
  renderPromise = (async () => {
    for (const name of order) {
      const style = currentStyle(name);
      if (style !== 'none') await ensureVoice(name, style);
    }
  })().finally(() => { renderPromise = null; });
  return renderPromise;
}

function masterLevel() {
  let v = 100;
  try { v = getSetting('soundVolume'); } catch { /* default */ }
  return 0.9 * (Number.isFinite(v) ? Math.min(100, Math.max(0, v)) : 100) / 100;
}

// Follow the volume and sound choices live (module-level, one listener for the app's lifetime).
try {
  onSettingsChange((s, key) => {
    if (!ctx || !master) return;
    if (key === 'soundVolume') master.gain.setTargetAtTime(masterLevel(), ctx.currentTime, 0.015);
    if (key === 'soundPicks') renderSelected();
  });
} catch { /* settings unavailable */ }

/** Fallback while a buffer is still rendering: build the voice live (no room). */
function playLive(c, v, t) {
  const g = c.createGain();
  g.connect(master);
  v.play(c, g, c.createGain(), noiseBuffer(c, 0.2), t);
  setTimeout(() => { try { g.disconnect(); } catch { /* already */ } }, (v.dur + 0.2) * 1000);
}

// ---------------------------------------------------------------- public API

/** Whether sounds are enabled in user settings. */
export function isSoundEnabled() {
  try { return getSetting('sounds') !== false; } catch { return true; }
}

/**
 * Play a named sound in the user's chosen style. No-op when sounds are disabled, the event is
 * set to 'none', before the first user gesture, when WebAudio is unavailable, or when the same
 * sound was just played.
 * @param {string} name
 * @param {{force?: boolean, style?: string}} [opts] force = ignore the on/off setting (previews);
 *   style = play this style instead of the chosen one (previews in Settings)
 */
export function playSound(name, opts = {}) {
  const styles = stylesFor(name);
  if (!styles.length) return;
  const style = opts.style && styles.includes(opts.style) ? opts.style : currentStyle(name);
  if (style === 'none') return;
  if (!opts.force && !isSoundEnabled()) return;
  if (typeof document !== 'undefined' && document.hidden) return;
  const c = getCtx();
  if (!c || !master) return;
  if (activeVoices >= MAX_VOICES) return;
  const now = performance.now();
  const key = voiceKey(name, style);
  if (now - (lastPlayed.get(name) || 0) < MIN_GAP_MS) return;
  lastPlayed.set(name, now);
  const t = c.currentTime + 0.002;
  try {
    const buf = buffers.get(key);
    if (!buf) {
      const v = recipe(name, style);
      if (v) playLive(c, v, t);
      ensureVoice(name, style);
      return;
    }
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
 * @param {string} name @param {number} [sampleRate] @param {string} [style] defaults to the chosen one
 */
export function renderSound(name, sampleRate = 48000, style) {
  const st = style || currentStyle(name);
  if (!recipe(name, st)) return Promise.reject(new Error(`unknown sound ${name}:${st}`));
  return renderVoice(name, st, sampleRate);
}

/** Call from a user-gesture handler to make sure audio is ready (optional). */
export function unlockAudio() {
  gestureSeen = true;
  getCtx();
}
