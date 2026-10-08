//! gm-bots: bot personalities and human-like move selection.
//!
//! How a bot picks a move (see `docs/CONTRACT.md` §3):
//! 1. **Opening book** — continuations from `gm_content` openings (matched by Zobrist hash, so
//!    transpositions work), weighted by popularity and the bot's style. Weak bots leave the book
//!    early and pick flatter.
//! 2. **Engine search** with per-Elo depth / node / time limits and MultiPV.
//! 3. **Human-like selection** — softmax over the MultiPV candidates with an Elo-dependent
//!    temperature (in centipawns) plus style preferences (aggressive: checks/captures/king
//!    attacks; positional: quiet improving moves; defensive: castling and trades; trappy:
//!    threat-making moves). Strong bots never stray more than `max_loss_cp` from the best line.
//! 4. **Oversights** — low-rated bots sometimes choose by a naive one-ply look that ignores the
//!    opponent's replies (hanging pieces, missed threats), or simply don't notice a capture.
//! 5. **Chat** — personality lines on captures, checks, opponent mistakes, mate; coaches give a
//!    concrete tip (loose pieces, threats, development) after every move.
//!
//! All work is bounded: no caches, no allocation that grows with anything but the (capped)
//! move list. No panics on bad input: everything returns `Err(String)`.

mod book;
mod personas;
mod strength;

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use shakmaty::zobrist::{Zobrist64, ZobristHash};
use shakmaty::{Chess, Color, EnPassantMode, Move, Position, Role, Square};

use gm_content::words::{fill, kv, PieceRef};
use gm_content::Lang;
use gm_engine::{move_to_san, move_to_uci, parse_fen, uci_to_move, Engine, Score, SearchLimits};

pub use personas::{Style, ADAPTIVE_ID, ADAPTIVE_START_ELO};

/// Range the adaptive bot's level is kept in.
pub const ADAPTIVE_MIN_ELO: u16 = 250;
pub const ADAPTIVE_MAX_ELO: u16 = 2800;
use personas::{Persona, PERSONAS};
pub use strength::Strength;

/// Maximum number of moves accepted in a game (bounded input).
const MAX_MOVES: usize = 2_000;
/// Mate scores map to +-(MATE_BASE - 10*n) centipawns from the mover's point of view.
const MATE_BASE: i32 = 30_000;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct BotProfile {
    pub id: String,
    pub name: String,
    pub elo: u16,
    /// emoji
    pub avatar: String,
    /// e.g. "aggressive", "positional", "beginner", "trappy"
    pub style: String,
    pub description: String,
    pub greeting: String,
    /// beginner | intermediate | advanced | master | coach
    pub category: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct BotMove {
    pub uci: String,
    pub san: String,
    pub chat: Option<String>,
    /// Suggested total "thinking" time in ms (300–2500), for realism. This already includes the
    /// time the server spent searching: the UI should wait `max(0, think_ms - request_elapsed)`.
    pub think_ms: u64,
}

fn profile_of(p: &Persona, lang: Lang) -> BotProfile {
    let text = p.lines(lang);
    BotProfile {
        id: p.id.into(),
        name: p.name.into(),
        elo: p.elo,
        avatar: p.avatar.into(),
        // A machine key (the UI translates the label); never localized.
        style: p.style.as_str().into(),
        description: text.description.into(),
        greeting: text.greeting.into(),
        category: p.category().into(),
    }
}

/// All bots, weakest first; coach bots last. Descriptions and greetings are in `lang`.
pub fn list(lang: Lang) -> Vec<BotProfile> {
    PERSONAS.iter().map(|p| profile_of(p, lang)).collect()
}

/// A single bot's profile.
pub fn get(bot_id: &str, lang: Lang) -> Option<BotProfile> {
    personas::find(bot_id).map(|p| profile_of(p, lang))
}

/// True if `bot_id` names a bot.
pub fn exists(bot_id: &str) -> bool {
    personas::find(bot_id).is_some()
}

/// Choose a move for `bot_id` in the position reached from `start_fen` after `moves` (UCI).
/// `content` should be the English source content (style preferences match English opening
/// names); opening names in chat come from `content.localized(lang)`. Chat is in `lang`.
///
/// Errors: unknown bot, invalid FEN/move, or the game is already over.
pub fn choose_move(
    engine: &mut Engine,
    content: &gm_content::Content,
    bot_id: &str,
    start_fen: &str,
    moves: &[String],
    lang: Lang,
) -> Result<BotMove, String> {
    let mut rng = StdRng::from_entropy();
    choose_move_with(engine, content, bot_id, start_fen, moves, &mut rng, None, lang)
}

/// Like [`choose_move`], but plays at `elo` instead of the persona's own rating (used for the
/// adaptive bot, whose level is stored per user). `elo` is clamped to
/// `ADAPTIVE_MIN_ELO..=ADAPTIVE_MAX_ELO`; `None` behaves exactly like [`choose_move`].
pub fn choose_move_at(
    engine: &mut Engine,
    content: &gm_content::Content,
    bot_id: &str,
    start_fen: &str,
    moves: &[String],
    elo: Option<u16>,
    lang: Lang,
) -> Result<BotMove, String> {
    let mut rng = StdRng::from_entropy();
    choose_move_inner(engine, content, bot_id, start_fen, moves, &mut rng, None, elo, lang)
}

// ---------------------------------------------------------------------------------------------
// Core
// ---------------------------------------------------------------------------------------------

struct Game {
    pos: Chess,
    /// Zobrist hashes of every position reached so far (including the current one).
    history: Vec<u64>,
    /// The position before the opponent's last move, and that move.
    last: Option<(Chess, Move)>,
}

fn zobrist(pos: &Chess) -> u64 {
    pos.zobrist_hash::<Zobrist64>(EnPassantMode::Legal).0
}

fn replay(start_fen: &str, moves: &[String]) -> Result<Game, String> {
    if moves.len() > MAX_MOVES {
        return Err(format!("too many moves (max {MAX_MOVES})"));
    }
    let mut pos = parse_fen(start_fen)?;
    let mut history = Vec::with_capacity(moves.len() + 1);
    history.push(zobrist(&pos));
    let mut last = None;
    for (i, u) in moves.iter().enumerate() {
        if pos.is_game_over() {
            return Err(format!("ply {}: game is already over", i + 1));
        }
        let m = uci_to_move(&pos, u).map_err(|e| format!("ply {}: {e}", i + 1))?;
        let before = pos.clone();
        pos.play_unchecked(&m);
        history.push(zobrist(&pos));
        if i + 1 == moves.len() {
            last = Some((before, m));
        }
    }
    Ok(Game { pos, history, last })
}

#[derive(Clone, Debug)]
struct Candidate {
    mv: Move,
    /// Engine score, mover's POV, centipawns (mates mapped to +-MATE_BASE).
    score: i32,
}

fn score_for_mover(s: Score, mover: Color) -> i32 {
    let pov = s.for_side(mover);
    match pov {
        Score::Cp(c) => c.clamp(-MATE_BASE + 1_000, MATE_BASE - 1_000),
        Score::Mate(n) if n > 0 => MATE_BASE - 10 * n.min(99),
        Score::Mate(n) if n < 0 => -MATE_BASE + 10 * (-n).min(99),
        Score::Mate(_) => 0,
    }
}

fn value(role: Role) -> i32 {
    match role {
        Role::Pawn => 100,
        Role::Knight => 320,
        Role::Bishop => 330,
        Role::Rook => 500,
        Role::Queen => 900,
        Role::King => 0,
    }
}

fn after(pos: &Chess, m: &Move) -> Chess {
    let mut p = pos.clone();
    p.play_unchecked(m);
    p
}

/// Pick an index with probability proportional to `weights` (all >= 0). Falls back to 0.
fn weighted_pick<R: Rng>(rng: &mut R, weights: &[f64]) -> usize {
    let total: f64 = weights.iter().filter(|w| w.is_finite() && **w > 0.0).sum();
    if total.is_nan() || total <= 0.0 {
        return 0;
    }
    let mut r = rng.gen::<f64>() * total;
    for (i, w) in weights.iter().enumerate() {
        if w.is_finite() && *w > 0.0 {
            if r < *w {
                return i;
            }
            r -= *w;
        }
    }
    weights.iter().rposition(|w| w.is_finite() && *w > 0.0).unwrap_or(0)
}

/// Softmax pick over scores (higher is better) at temperature `t` (centipawns).
fn softmax_pick<R: Rng>(rng: &mut R, scores: &[f64], t: f64) -> usize {
    let max = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let t = t.max(0.5);
    let w: Vec<f64> = scores.iter().map(|s| ((s - max) / t).exp()).collect();
    weighted_pick(rng, &w)
}

fn chebyshev(a: Square, b: Square) -> u32 {
    a.distance(b)
}

/// Pieces of `color` that are attacked and either undefended or attacked by something cheaper.
fn loose_pieces(pos: &Chess, color: Color) -> Vec<(Role, Square)> {
    let board = pos.board();
    let occ = board.occupied();
    let mut out: Vec<(Role, Square)> = Vec::new();
    for sq in board.by_color(color) {
        let Some(piece) = board.piece_at(sq) else { continue };
        if piece.role == Role::King {
            continue;
        }
        let attackers = board.attacks_to(sq, !color, occ);
        if attackers.is_empty() {
            continue;
        }
        let defenders = board.attacks_to(sq, color, occ);
        let cheapest = attackers
            .into_iter()
            .filter_map(|a| board.role_at(a))
            .map(|r| if r == Role::King { 10_000 } else { value(r) })
            .min()
            .unwrap_or(10_000);
        if cheapest == 10_000 {
            // Only the king attacks: it can take the piece only if it is undefended.
            if defenders.is_empty() {
                out.push((piece.role, sq));
            }
            continue;
        }
        if defenders.is_empty() || cheapest < value(piece.role) {
            out.push((piece.role, sq));
        }
    }
    out.sort_by_key(|(r, _)| -value(*r));
    out
}

/// Captured role of a move (en passant counts as pawn).
fn captured(m: &Move) -> Option<Role> {
    m.capture()
}

/// Style preference bonus in centipawns (mover's POV) for `m` in `pos`.
fn style_bonus(style: Style, pos: &Chess, m: &Move, after_pos: &Chess, ply: usize) -> f64 {
    let us = pos.turn();
    let them = !us;
    let gives_check = after_pos.is_check();
    let cap = captured(m);
    let to = m.to();
    let enemy_king = pos.board().king_of(them);
    let near_king = enemy_king.map(|k| chebyshev(k, to) <= 2).unwrap_or(false);
    let opening = ply < 20;
    let central = matches!(to, Square::D4 | Square::E4 | Square::D5 | Square::E5);
    let develops = matches!(m.role(), Role::Knight | Role::Bishop)
        && m.from().map(|f| {
            let back = if us == Color::White { 0 } else { 7 };
            u32::from(f.rank()) == back
        }) == Some(true);
    // Does the move attack an enemy piece worth more than the mover?
    let threatens = {
        let attacks = after_pos.board().attacks_from(to) & after_pos.board().by_color(them);
        let mover_val = value(m.promotion().unwrap_or(m.role()));
        attacks
            .into_iter()
            .filter_map(|s| after_pos.board().role_at(s))
            .any(|r| r != Role::King && value(r) > mover_val)
    };
    let trade = cap.map(|c| c == m.role()).unwrap_or(false);
    let mut b = 0.0;
    match style {
        Style::Beginner => {
            if cap.is_some() {
                b += 30.0;
            }
            if opening && m.role() == Role::Queen {
                b += 20.0;
            }
            if gives_check {
                b += 15.0;
            }
        }
        Style::Aggressive => {
            if gives_check {
                b += 25.0;
            }
            if cap.is_some() {
                b += 12.0;
            }
            if near_king {
                b += 12.0;
            }
            if m.role() == Role::Pawn && !opening {
                if let Some(k) = enemy_king {
                    if u32::from(k.file()).abs_diff(u32::from(to.file())) <= 1 {
                        b += 8.0; // pawn storm
                    }
                }
            }
            if trade && m.role() == Role::Queen {
                b -= 20.0; // keep the queens on
            }
        }
        Style::Positional => {
            if cap.is_none() && !gives_check {
                b += 8.0;
            }
            if develops {
                b += 12.0;
            }
            if opening && central && m.role() == Role::Pawn {
                b += 8.0;
            }
            if m.is_castle() {
                b += 18.0;
            }
        }
        Style::Defensive => {
            if m.is_castle() {
                b += 25.0;
            }
            if trade {
                b += 14.0;
            }
            if let Some(k) = pos.board().king_of(us) {
                if m.role() != Role::King && m.role() != Role::Pawn && chebyshev(k, to) <= 2 {
                    b += 6.0;
                }
            }
            if gives_check {
                b -= 4.0;
            }
        }
        Style::Trappy => {
            if threatens {
                b += 15.0;
            }
            if gives_check {
                b += 10.0;
            }
            if ply < 30 && cap.is_none() && threatens {
                b += 8.0;
            }
        }
        Style::Coach => {
            if develops || m.is_castle() {
                b += 8.0;
            }
        }
        Style::Universal => {}
    }
    b
}

/// One-ply "what's in front of me" evaluation used for oversights: greedy material, checks and
/// development with no regard for the opponent's replies.
fn naive_score<R: Rng>(rng: &mut R, pos: &Chess, m: &Move, ply: usize) -> f64 {
    let a = after(pos, m);
    let mut s = 0.0;
    if let Some(c) = captured(m) {
        s += f64::from(value(c));
    }
    if let Some(p) = m.promotion() {
        s += f64::from(value(p) - 100);
    }
    if a.is_checkmate() {
        s += 400.0; // even beginners like checkmate, but don't always see it
    } else if a.is_check() {
        s += 40.0;
    }
    if m.is_castle() {
        s += 30.0;
    }
    if ply < 16 && matches!(m.role(), Role::Knight | Role::Bishop) {
        s += 20.0;
    }
    if matches!(m.to(), Square::D4 | Square::E4 | Square::D5 | Square::E5) {
        s += 12.0;
    }
    if m.role() == Role::King && !m.is_castle() && ply < 40 {
        s -= 30.0;
    }
    s + rng.gen_range(-45.0..45.0)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn choose_move_with<R: Rng>(
    engine: &mut Engine,
    content: &gm_content::Content,
    bot_id: &str,
    start_fen: &str,
    moves: &[String],
    rng: &mut R,
    movetime_cap_ms: Option<u64>,
    lang: Lang,
) -> Result<BotMove, String> {
    choose_move_inner(engine, content, bot_id, start_fen, moves, rng, movetime_cap_ms, None, lang)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn choose_move_inner<R: Rng>(
    engine: &mut Engine,
    content: &gm_content::Content,
    bot_id: &str,
    start_fen: &str,
    moves: &[String],
    rng: &mut R,
    movetime_cap_ms: Option<u64>,
    elo_override: Option<u16>,
    lang: Lang,
) -> Result<BotMove, String> {
    let started = Instant::now();
    let persona = personas::find(bot_id).ok_or_else(|| format!("unknown bot: {bot_id}"))?;
    let elo = elo_override
        .map(|e| e.clamp(ADAPTIVE_MIN_ELO, ADAPTIVE_MAX_ELO))
        .unwrap_or(persona.elo);
    let game = replay(start_fen, moves)?;
    let pos = &game.pos;
    if pos.is_game_over() {
        return Err("game is over".into());
    }
    let legal = pos.legal_moves();
    if legal.is_empty() {
        return Err("game is over".into());
    }
    let st = Strength::for_elo(elo);
    let us = pos.turn();
    let ply = moves.len();

    // Forced move: no need to think.
    if legal.len() == 1 {
        let m = legal[0].clone();
        let chat = make_chat(rng, persona, &game, &m, None, None, lang);
        return Ok(finish(pos, &m, chat, rng.gen_range(300..=550)));
    }

    // ---- 1. Opening book ----------------------------------------------------------------------
    if ply < st.book_plies && rng.gen::<f64>() >= st.book_exit {
        let cands = book::candidates(content, pos, persona.style);
        if !cands.is_empty() {
            // Strong players follow the main lines; weak ones pick flatter.
            let sharp = 0.5 + f64::from(elo.min(3000)) / 3000.0;
            let w: Vec<f64> = cands.iter().map(|c| c.weight.powf(sharp)).collect();
            let c = &cands[weighted_pick(rng, &w)];
            if let Ok(m) = uci_to_move(pos, &c.uci) {
                let after_pos = after(pos, &m);
                let id = c.opening_id.clone().or_else(|| book::opening_id_at(content, &after_pos));
                let lines = persona.lines(lang);
                let name = id.and_then(|id| content.localized(lang).opening(&id).map(|o| o.name.clone()));
                let chat = match name {
                    Some(n) if rng.gen::<f64>() < 0.55 && !lines.opening.is_empty() => {
                        Some(pick_line(rng, lines.opening).replace("{opening}", &n))
                    }
                    _ => make_chat(rng, persona, &game, &m, None, None, lang),
                };
                return Ok(finish(pos, &m, chat, rng.gen_range(300..=750)));
            }
        }
    }

    // ---- 2. Engine search ---------------------------------------------------------------------
    let movetime = movetime_cap_ms.map_or(st.movetime_ms, |c| st.movetime_ms.min(c.max(1)));
    let limits = SearchLimits {
        depth: Some(st.depth),
        movetime_ms: Some(movetime),
        nodes: Some(st.nodes),
        multipv: st.multipv.min(legal.len()).max(1),
    };
    let stop = AtomicBool::new(false);
    let info = engine.search(pos, &limits, &stop, &mut |_| {});
    let mut cands: Vec<Candidate> = Vec::with_capacity(info.lines.len());
    for line in &info.lines {
        let Some(u) = line.moves.first() else { continue };
        let Ok(mv) = uci_to_move(pos, u) else { continue };
        if cands.iter().any(|c| c.mv == mv) {
            continue;
        }
        cands.push(Candidate { mv, score: score_for_mover(line.score, us) });
    }
    cands.sort_by_key(|c| std::cmp::Reverse(c.score));

    // ---- 3. Selection -------------------------------------------------------------------------
    let oversight = cands.is_empty() || rng.gen::<f64>() < st.oversight;
    let (chosen, chosen_score) = if oversight {
        // Naive one-ply choice over all legal moves (may hang pieces / miss threats).
        let naive: Vec<f64> = legal.iter().map(|m| naive_score(rng, pos, m, ply)).collect();
        let i = softmax_pick(rng, &naive, 40.0);
        let m = legal[i].clone();
        let sc = cands.iter().find(|c| c.mv == m).map(|c| c.score);
        (m, sc)
    } else {
        let best = cands[0].score;
        let mut pool: Vec<&Candidate> =
            cands.iter().filter(|c| c.score >= best.saturating_sub(st.max_loss_cp)).collect();
        // "Didn't notice that capture."
        if rng.gen::<f64>() < st.miss_capture && pool.iter().any(|c| !c.mv.is_capture()) {
            pool.retain(|c| !c.mv.is_capture());
        }
        let current_hash = game.history.last().copied();
        let adjusted: Vec<f64> = pool
            .iter()
            .map(|c| {
                let a = after(pos, &c.mv);
                let mut s = f64::from(c.score.clamp(-st.score_cap, st.score_cap));
                s += style_bonus(persona.style, pos, &c.mv, &a, ply) * st.style_weight;
                // Repetition awareness: avoid draws when better, seek them when worse.
                let h = zobrist(&a);
                if Some(h) != current_hash && game.history.contains(&h) {
                    if best > 50 {
                        s -= 150.0;
                    } else if best < -150 && elo >= 1200 {
                        s += 120.0;
                    }
                }
                s
            })
            .collect();
        let i = softmax_pick(rng, &adjusted, st.temperature_cp);
        (pool[i].mv.clone(), Some(pool[i].score))
    };

    // ---- 4. Chat + think time -----------------------------------------------------------------
    let best_score = cands.first().map(|c| c.score);
    let chat = make_chat(rng, persona, &game, &chosen, chosen_score.or(best_score), best_score, lang);
    let think = think_time(rng, elo, legal.len(), &cands, pos.is_check(), started);
    Ok(finish(pos, &chosen, chat, think))
}

fn finish(pos: &Chess, m: &Move, chat: Option<String>, think_ms: u64) -> BotMove {
    BotMove { uci: move_to_uci(m), san: move_to_san(pos, m), chat, think_ms: think_ms.clamp(300, 2500) }
}

/// Realistic total thinking time: stronger bots and complex positions take longer.
fn think_time<R: Rng>(rng: &mut R, elo: u16, n_legal: usize, cands: &[Candidate], in_check: bool, started: Instant) -> u64 {
    let mut t = 450.0 + f64::from(elo) * 0.35;
    t *= (n_legal as f64 / 30.0).clamp(0.6, 1.4);
    if cands.len() >= 2 {
        let gap = cands[0].score.saturating_sub(cands[1].score);
        if gap < 40 {
            t *= 1.3; // several good options: harder decision
        } else if gap > 300 {
            t *= 0.6; // only one sensible move (e.g. recapture)
        }
    }
    if in_check {
        t *= 0.75;
    }
    t *= rng.gen_range(0.75..1.25);
    let spent = started.elapsed().as_millis() as f64;
    // Never suggest less than what we already spent (the UI subtracts elapsed time).
    t.max(spent).clamp(300.0, 2500.0) as u64
}

fn pick_line<R: Rng>(rng: &mut R, lines: &[&'static str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    lines[rng.gen_range(0..lines.len())].to_string()
}

fn chance<R: Rng>(rng: &mut R, p: f64) -> bool {
    rng.gen::<f64>() < p
}

/// Personality chat for the move the bot is about to play. `score` is the bot's (approximate)
/// evaluation after its move, mover POV; `best` the best engine score.
fn make_chat<R: Rng>(
    rng: &mut R,
    persona: &Persona,
    game: &Game,
    m: &Move,
    score: Option<i32>,
    best: Option<i32>,
    lang: Lang,
) -> Option<String> {
    let pos = &game.pos;
    let a = after(pos, m);
    let lines = persona.lines(lang);
    if a.is_checkmate() {
        return Some(pick_line(rng, lines.win));
    }
    let opp_captured = game.last.as_ref().and_then(|(_, lm)| lm.capture().map(|r| (r, lm.to())));
    let our_cap = captured(m);
    let recapture = matches!(opp_captured, Some((_, sq)) if sq == m.to());
    let score = score.unwrap_or(0);
    let best = best.unwrap_or(score);

    if persona.coach {
        return Some(coach_tip(rng, persona, game, m, &a, score, lang));
    }

    if let Some(c) = our_cap {
        if value(c) >= 300 && best >= 250 && !recapture && chance(rng, 0.7) {
            return Some(pick_line(rng, lines.blunder));
        }
    }
    if opp_captured.is_some() && !recapture && chance(rng, 0.5) {
        return Some(pick_line(rng, lines.captured));
    }
    if our_cap.is_some() && chance(rng, 0.3) {
        return Some(pick_line(rng, lines.capture));
    }
    if a.is_check() && chance(rng, 0.35) {
        return Some(pick_line(rng, lines.check));
    }
    if score >= 600 && chance(rng, 0.12) {
        return Some(pick_line(rng, lines.winning));
    }
    if score <= -600 && chance(rng, 0.2) {
        return Some(pick_line(rng, lines.losing));
    }
    None
}

/// Coach-tip templates (see `gm_content::words` for the `{p_*}` placeholders).
struct TipText {
    took_loose: &'static str,
    loose: &'static str,
    attacks: &'static str,
    uncastled: &'static str,
    undeveloped: &'static str,
    general: [&'static str; 6],
}

fn tip_text(lang: Lang) -> &'static TipText {
    static EN: TipText = TipText {
        took_loose: "Your {p_n} on {sq} was not protected, so I took it. Before every move, check that each of your pieces is safe!",
        loose: "Heads up: your {p_n} on {sq} is under attack and not well defended.",
        attacks: "My {m_n} now attacks your {t_n}. What will you do about it?",
        uncastled: "Your king is still in the center. Castling soon will keep it safe and connect your rooks.",
        undeveloped: "You still have {n} knights/bishops on their starting squares. Bring them out before starting an attack!",
        general: [
            "Before each move, look for checks, captures and threats — for both sides.",
            "Try to put your pieces on squares where they control the center.",
            "Rooks love open files. Is there a file without pawns for your rook?",
            "Ask yourself: what is my worst-placed piece, and how can I improve it?",
            "In the endgame, your king becomes a strong piece. Bring it toward the center!",
            "Passed pawns must be pushed! A pawn with no enemy pawns in front is very dangerous.",
        ],
    };
    static ES: TipText = TipText {
        took_loose: "Tu {p_n} en {sq} no estaba protegid{p_o}, así que me {p_lo} llevé. Antes de cada jugada, ¡comprueba que todas tus piezas estén a salvo!",
        loose: "Atención: tu {p_n} en {sq} está atacad{p_o} y mal defendid{p_o}.",
        attacks: "Mi {m_n} ahora ataca a tu {t_n}. ¿Qué vas a hacer?",
        uncastled: "Tu rey sigue en el centro. Enrocar pronto lo pondrá a salvo y conectará tus torres.",
        undeveloped: "Todavía tienes {n} caballos o alfiles en sus casillas iniciales. ¡Sácalos antes de lanzarte al ataque!",
        general: [
            "Antes de cada jugada, busca jaques, capturas y amenazas, para ambos bandos.",
            "Intenta colocar tus piezas en casillas desde donde controlen el centro.",
            "A las torres les encantan las columnas abiertas. ¿Hay alguna columna sin peones para tu torre?",
            "Pregúntate: ¿cuál es mi pieza peor colocada y cómo puedo mejorarla?",
            "En el final, tu rey se convierte en una pieza fuerte. ¡Llévalo hacia el centro!",
            "¡Los peones pasados hay que avanzarlos! Un peón sin peones rivales delante es muy peligroso.",
        ],
    };
    match lang {
        Lang::En => &EN,
        Lang::Es => &ES,
    }
}

/// A concrete, beginner-friendly tip from a coach bot after its move.
fn coach_tip<R: Rng>(rng: &mut R, persona: &Persona, game: &Game, m: &Move, a: &Chess, score: i32, lang: Lang) -> String {
    let pos = &game.pos;
    let lines = persona.lines(lang);
    let tt = tip_text(lang);
    let user = !pos.turn();
    let our_cap = captured(m);
    let opp_last = game.last.as_ref();

    // 1. We just took a piece the user left hanging.
    if let Some(c) = our_cap {
        let was_loose = loose_pieces(pos, user).iter().any(|(_, sq)| *sq == m.to());
        if was_loose && value(c) >= 300 {
            let mut vars = PieceRef::new(c).vars("p", lang);
            vars.extend(kv(&[("sq", &m.to().to_string())]));
            return fill(tt.took_loose, &vars);
        }
    }
    // 2. Check.
    if a.is_check() {
        return pick_line(rng, lines.check);
    }
    // 3. User has a loose piece right now.
    if let Some((role, sq)) = loose_pieces(a, user).into_iter().find(|(r, _)| *r != Role::Pawn) {
        let mut vars = PieceRef::new(role).vars("p", lang);
        vars.extend(kv(&[("sq", &sq.to_string())]));
        return fill(tt.loose, &vars);
    }
    // 4. My move creates a threat on a valuable piece.
    let attacked: Vec<Role> = (a.board().attacks_from(m.to()) & a.board().by_color(user))
        .into_iter()
        .filter_map(|s| a.board().role_at(s))
        .filter(|r| *r != Role::King && *r != Role::Pawn)
        .collect();
    if let Some(r) = attacked.iter().max_by_key(|r| value(**r)) {
        let mut vars = PieceRef::new(m.role()).vars("m", lang);
        vars.extend(PieceRef::new(*r).vars("t", lang));
        return fill(tt.attacks, &vars);
    }
    // 5. User just won material.
    if let Some((_, lm)) = opp_last {
        if lm.capture().map(value).unwrap_or(0) >= 300 && m.to() != lm.to() {
            return pick_line(rng, lines.captured);
        }
    }
    // 6. Big evaluation swings.
    if score >= 500 {
        return pick_line(rng, lines.winning);
    }
    if score <= -500 {
        return pick_line(rng, lines.losing);
    }
    // 7. Opening principles.
    let ply = game.history.len().saturating_sub(1);
    if (14..=30).contains(&ply) && user_king_uncastled(a, user) {
        return tt.uncastled.to_string();
    }
    if ply < 16 {
        let undeveloped = undeveloped_minors(a, user);
        if undeveloped >= 2 && ply >= 6 {
            return fill(tt.undeveloped, &kv(&[("n", &undeveloped.to_string())]));
        }
    }
    tt.general[rng.gen_range(0..tt.general.len())].to_string()
}

fn user_king_uncastled(pos: &Chess, user: Color) -> bool {
    let home = if user == Color::White { Square::E1 } else { Square::E8 };
    pos.board().king_of(user) == Some(home) && pos.board().by_color(!user).count() > 10
}

fn undeveloped_minors(pos: &Chess, user: Color) -> usize {
    let (squares, color) = if user == Color::White {
        ([Square::B1, Square::G1, Square::C1, Square::F1], Color::White)
    } else {
        ([Square::B8, Square::G8, Square::C8, Square::F8], Color::Black)
    };
    let board = pos.board();
    squares
        .iter()
        .filter(|sq| {
            board
                .piece_at(**sq)
                .map(|p| p.color == color && matches!(p.role, Role::Knight | Role::Bishop))
                .unwrap_or(false)
        })
        .count()
}

#[cfg(test)]
mod tests;
