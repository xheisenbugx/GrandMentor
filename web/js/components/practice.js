// Opening practice vs a bot: shared helpers for the play page (#/play?opening=&line=&color=) and the
// pages that link to it (openings, repertoire). A practice game is a normal game from the standard
// start whose first moves are already on the board. Contract: docs/CONTRACT.md §5 "Opening practice".

import { Chess } from '/vendor/chess.js';

export const MAX_LINE_PLIES = 40;
const MAX_LINE_CHARS = 400;
const UCI_RE = /^[a-h][1-8][a-h][1-8][qrbn]?$/;

/**
 * Validate a UCI line from the URL ("e2e4 e7e5 g1f3", commas or '+' also separate moves).
 * Returns null for an empty line, `{ error: 'invalid'|'tooLong'|'finished' }`, or
 * `{ moves, sans, fen }` with every move legal from the standard start.
 */
export function parseLine(raw) {
  const text = String(raw ?? '').trim();
  if (!text) return null;
  if (text.length > MAX_LINE_CHARS) return { error: 'tooLong' };
  const parts = text.toLowerCase().split(/[\s,+]+/).filter(Boolean);
  if (parts.length > MAX_LINE_PLIES) return { error: 'tooLong' };
  const chess = new Chess();
  const moves = [];
  const sans = [];
  for (const uci of parts) {
    if (!UCI_RE.test(uci)) return { error: 'invalid' };
    let mv = null;
    try { mv = chess.move({ from: uci.slice(0, 2), to: uci.slice(2, 4), promotion: uci[4] || undefined }); } catch { mv = null; }
    if (!mv) return { error: 'invalid' };
    moves.push(mv.from + mv.to + (mv.promotion || ''));
    sans.push(mv.san);
  }
  if (chess.isGameOver()) return { error: 'finished' };
  return { moves, sans, fen: chess.fen() };
}

/** "1. e4 e5 2. Nf3" from SAN moves played from the standard start. */
export function numberedSans(sans) {
  const out = [];
  sans.forEach((s, i) => {
    if (i % 2 === 0) out.push(`${i / 2 + 1}.`);
    out.push(s);
  });
  return out.join(' ');
}

/**
 * Link to the play page that starts a practice game.
 * `color` is 'w'|'b' (the user's side); `line` is a list of UCI moves (omit to use the opening's
 * main line); `botId` preselects a bot on the setup screen.
 */
export function practiceHref({ botId = '', opening = '', line = null, color = '' } = {}) {
  const q = new URLSearchParams();
  if (opening) q.set('opening', opening);
  if (Array.isArray(line) && line.length) q.set('line', line.slice(0, MAX_LINE_PLIES).join(' '));
  if (color === 'w' || color === 'b') q.set('color', color);
  const path = botId ? `#/play/${encodeURIComponent(botId)}` : '#/play';
  const qs = q.toString().replace(/\+/g, '%20');
  return qs ? `${path}?${qs}` : path;
}
