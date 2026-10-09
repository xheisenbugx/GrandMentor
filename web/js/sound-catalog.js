// GrandMentor — sound catalogue (pure data, no audio code).
// Shared by settings.js (validation), components/sound.js (synthesis) and the Settings page (pickers).
// Contract: docs/CONTRACT.md §5 "Sound".
//
// Every sound event belongs to a family. Piece events (a piece touching the board) pick a
// material; chime events (UI cues) pick a timbre. The user's choices are stored in the
// `soundPicks` setting as { event: styleId } and only hold overrides of the defaults below.

/** Piece materials, in menu order. The first one is the default. */
export const PIECE_STYLES = Object.freeze(['wood', 'marble', 'plastic', 'felt', 'glass', 'retro', 'pop', 'click', 'none']);

/** Chime timbres, in menu order. The first one is the default. */
export const CHIME_STYLES = Object.freeze(['chime', 'bell', 'marimba', 'retro', 'beep', 'none']);

/** Sound events in menu order, with their family. */
export const SOUND_EVENTS = Object.freeze([
  { id: 'move', family: 'piece' },
  { id: 'capture', family: 'piece' },
  { id: 'castle', family: 'piece' },
  { id: 'check', family: 'piece' },
  { id: 'promote', family: 'piece' },
  { id: 'gameStart', family: 'chime' },
  { id: 'gameEnd', family: 'chime' },
  { id: 'lowTime', family: 'chime' },
  { id: 'illegal', family: 'chime' },
  { id: 'correct', family: 'chime' },
  { id: 'wrong', family: 'chime' },
  { id: 'notify', family: 'chime' },
]);

const FAMILY = new Map(SOUND_EVENTS.map((e) => [e.id, e.family]));

/** Styles available for an event (empty array for an unknown event). */
export function stylesFor(event) {
  const f = FAMILY.get(event);
  return f === 'piece' ? PIECE_STYLES : f === 'chime' ? CHIME_STYLES : [];
}

/** Default style of an event. */
export function defaultStyle(event) {
  return stylesFor(event)[0] || null;
}

/**
 * Ready-made themes: one piece material + one chime timbre for every event.
 * 'classic' is the out-of-the-box sound (no overrides).
 */
export const SOUND_PRESETS = Object.freeze({
  classic: { piece: 'wood', chime: 'chime' },
  marble: { piece: 'marble', chime: 'bell' },
  club: { piece: 'plastic', chime: 'beep' },
  cozy: { piece: 'felt', chime: 'marimba' },
  crystal: { piece: 'glass', chime: 'bell' },
  arcade: { piece: 'retro', chime: 'retro' },
  bubbly: { piece: 'pop', chime: 'marimba' },
  minimal: { piece: 'click', chime: 'beep' },
});

/** The `soundPicks` overrides a preset stands for (defaults are left out). */
export function presetPicks(presetId) {
  const p = SOUND_PRESETS[presetId];
  if (!p) return null;
  const picks = {};
  for (const e of SOUND_EVENTS) {
    const style = p[e.family];
    if (style !== defaultStyle(e.id)) picks[e.id] = style;
  }
  return picks;
}

/** The preset matching a `soundPicks` value, or null when the choices are custom. */
export function matchPreset(picks) {
  for (const id of Object.keys(SOUND_PRESETS)) {
    const p = SOUND_PRESETS[id];
    if (SOUND_EVENTS.every((e) => resolveStyle(picks, e.id) === p[e.family])) return id;
  }
  return null;
}

/** Effective style of an event for a `soundPicks` value (falls back to the default). */
export function resolveStyle(picks, event) {
  const v = picks && typeof picks === 'object' ? picks[event] : undefined;
  return typeof v === 'string' && stylesFor(event).includes(v) ? v : defaultStyle(event);
}

/** Validator for the `soundPicks` setting: a plain object of known event → allowed style. */
export function isValidSoundPicks(v) {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return false;
  return Object.entries(v).every(([k, s]) => typeof s === 'string' && stylesFor(k).includes(s));
}
