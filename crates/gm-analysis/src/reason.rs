//! "Why was that a mistake?" — explanations grounded in the engine's own lines.
//!
//! For every inaccuracy, mistake, miss and blunder the review already has two engine lines
//! for free: the best line from the position *before* the move (what should have happened)
//! and the best line from the position *after* it (the opponent's refutation). Both come from
//! the review's bounded, stoppable searches, so deriving a reason costs no extra engine time.
//!
//! [`derive`] replays both lines on a board and turns them into a concrete, structured
//! [`MoveReason`]: the move allows mate in N, loses material (and how: a hanging piece, a fork,
//! a pin, a skewer, or simply the line that wins it), misses a mate, misses winning material
//! or a fork, or — when nothing concrete happens — the opponent's best answer. [`render`]
//! writes the warm, beginner-friendly text for it in any [`Lang`]; the structured fields are
//! stored with the review so the text can be rewritten in another language without the engine.
//!
//! Everything here is pure, bounded (lines are capped at [`MAX_LINE`] plies) and panic-free on
//! arbitrary input: illegal or garbage line moves simply end the line.

use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Color, Move, Position, Role};

use gm_content::words::{de_piece, fill, fr_piece, join_list, kv, piece_name, piece_plural, plural, pt_piece, PieceRef};
use gm_content::Lang;
use gm_engine::{uci_line_to_san, uci_to_move, Score};
use gm_mentor::tactics::{self, LineKind};

use crate::Classification;

/// Longest line we keep or replay (plies).
pub const MAX_LINE: usize = 12;
/// Plies of a line normally inspected for material changes (extended through captures/checks).
const BASE_WINDOW: usize = 6;

/// What kind of reason a move got. Serialized snake_case on the wire.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasonKind {
    /// The opponent can now force checkmate.
    AllowsMate,
    /// A piece is left where the opponent's very next move simply takes it.
    HangsPiece,
    /// The opponent's reply is a fork that wins material.
    AllowsFork,
    /// The opponent's reply is a pin that wins material.
    AllowsPin,
    /// The opponent's reply is a skewer that wins material.
    AllowsSkewer,
    /// The refutation line wins material (e.g. a capture that doesn't add up).
    LosesMaterial,
    /// The mover had a forced mate and let it go.
    MissedMate,
    /// The best move was a fork that wins material.
    MissedFork,
    /// The best move wins material.
    MissedMaterial,
    /// Nothing concrete: the opponent's best answer simply gives them the easier game.
    #[default]
    Positional,
}

impl ReasonKind {
    /// True when the reason is concrete enough to replace the rule-based explanation.
    pub fn is_concrete(self) -> bool {
        !matches!(self, ReasonKind::Positional)
    }
}

/// A grounded reason for an error, plus the lines that show it on the board.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct MoveReason {
    pub kind: ReasonKind,
    /// Friendly text in the review's language.
    pub text: String,
    /// The opponent's best line after the played move (starts from `fen_after`).
    pub refutation_uci: Vec<String>,
    pub refutation_san: Vec<String>,
    /// Index in `refutation_*` of the move that matters (the capture, the fork, the mate).
    pub refutation_key: Option<usize>,
    /// The engine's best line instead of the played move (starts from `fen_before`).
    pub better_uci: Vec<String>,
    pub better_san: Vec<String>,
    /// Index in `better_*` of the move that matters.
    pub better_key: Option<usize>,
    /// Mate distance in moves for `allows_mate` / `missed_mate`.
    pub mate_in: Option<u32>,
    /// Pieces the mover loses (role names: "queen", "pawn"...), most valuable first.
    pub lost: Vec<String>,
    /// Pieces the mover gets back in the same line (or wins, for the `missed_*` kinds).
    pub won: Vec<String>,
    /// Pieces hit by the fork / pin / skewer / attacking reply (role names).
    pub targets: Vec<String>,
    /// Net material change for the mover in points (1/3/3/5/9), negative = lost.
    pub material: i32,
}

/// Everything [`derive`] needs about one move.
pub struct ReasonInput<'a> {
    pub before: &'a Chess,
    pub played: &'a Move,
    /// Engine's best line from the position after the move (UCI), i.e. the refutation.
    pub refutation: &'a [String],
    /// Engine's best line from the position before the move (UCI).
    pub best: &'a [String],
    /// White-POV evaluations before and after the move.
    pub eval_before: Score,
    pub eval_after: Score,
    pub classification: Classification,
}

fn role_str(r: Role) -> &'static str {
    tactics::role_name(r)
}

fn parse_role(s: &str) -> Option<Role> {
    match s {
        "pawn" => Some(Role::Pawn),
        "knight" => Some(Role::Knight),
        "bishop" => Some(Role::Bishop),
        "rook" => Some(Role::Rook),
        "queen" => Some(Role::Queen),
        "king" => Some(Role::King),
        _ => None,
    }
}

/// Mate distance from `side`'s point of view (positive = `side` mates).
fn pov_mate(score: Score, side: Color) -> Option<i32> {
    match score {
        Score::Mate(m) if m != 0 => Some(if side == Color::White { m } else { -m }),
        _ => None,
    }
}

/// Replays up to [`MAX_LINE`] UCI moves from `start`, stopping at the first illegal one.
fn replay(start: &Chess, line: &[String]) -> Vec<Move> {
    let mut pos = start.clone();
    let mut out = Vec::new();
    for u in line.iter().take(MAX_LINE) {
        let Ok(m) = uci_to_move(&pos, u.trim()) else { break };
        pos.play_unchecked(&m);
        out.push(m);
        if pos.is_game_over() {
            break;
        }
    }
    out
}

/// Material outcome of a line for `mover`.
#[derive(Debug, Default)]
struct Outcome {
    /// Net change in points for `mover` over the inspected window.
    net: i32,
    /// (index in the line, role) of the mover's pieces captured, after cancelling equal trades.
    lost: Vec<(usize, Role)>,
    /// (index in the line, role) of the opponent's pieces the mover captured, after cancelling.
    won: Vec<(usize, Role)>,
}

fn same_value(a: Role, b: Role) -> bool {
    tactics::points(a) == tactics::points(b)
}

/// Inspect the first plies of `moves` (played from `start`) and report the material swing.
/// The window is [`BASE_WINDOW`] plies, extended while the last inspected move is a capture,
/// promotion or check so an exchange is never cut in half.
fn outcome(start: &Chess, moves: &[Move], mover: Color) -> Outcome {
    let mut end = moves.len().min(BASE_WINDOW);
    while end < moves.len() && end < MAX_LINE {
        let last = &moves[end - 1];
        if last.is_capture() || last.is_promotion() {
            end += 1;
        } else {
            break;
        }
    }
    let balance = |p: &Chess| tactics::material_points(p.board(), mover) - tactics::material_points(p.board(), !mover);
    let mut pos = start.clone();
    let b0 = balance(&pos);
    let mut lost: Vec<(usize, Role)> = Vec::new();
    let mut won: Vec<(usize, Role)> = Vec::new();
    for (i, m) in moves.iter().take(end).enumerate() {
        let by = pos.turn();
        if let Some(r) = m.capture() {
            if by == mover {
                won.push((i, r));
            } else {
                lost.push((i, r));
            }
        }
        pos.play_unchecked(m);
    }
    // Cancel equal trades (knight for bishop counts as equal) so "you lose your queen" isn't
    // drowned out by the pieces that were simply exchanged on the way.
    let mut i = 0;
    while i < lost.len() {
        if let Some(j) = won.iter().position(|w| same_value(w.1, lost[i].1)) {
            won.remove(j);
            lost.remove(i);
        } else {
            i += 1;
        }
    }
    lost.sort_by_key(|(i, r)| (-tactics::points(*r), *i));
    won.sort_by_key(|(i, r)| (-tactics::points(*r), *i));
    Outcome { net: balance(&pos) - b0, lost, won }
}

fn trim_line(line: &[String], keep: usize) -> Vec<String> {
    line.iter().take(keep.clamp(1, MAX_LINE)).cloned().collect()
}

/// Derive a grounded reason for an error. Returns `None` for moves that are not errors.
pub fn derive(inp: &ReasonInput<'_>) -> Option<MoveReason> {
    if !matches!(
        inp.classification,
        Classification::Inaccuracy | Classification::Mistake | Classification::Miss | Classification::Blunder
    ) {
        return None;
    }
    let mover = inp.before.turn();
    let mut after = inp.before.clone();
    after.play_unchecked(inp.played);

    let refutation_moves = if after.is_game_over() { Vec::new() } else { replay(&after, inp.refutation) };
    let best_moves = replay(inp.before, inp.best);
    let refutation_uci: Vec<String> = refutation_moves.iter().map(gm_engine::move_to_uci).collect();
    let best_uci: Vec<String> = best_moves.iter().map(gm_engine::move_to_uci).collect();
    // The best line only makes sense when it differs from the played move.
    let best_differs = best_moves.first().map(|b| b != inp.played).unwrap_or(false);

    let mut r = MoveReason::default();
    let set_ref = |r: &mut MoveReason, keep: usize| {
        r.refutation_uci = trim_line(&refutation_uci, keep);
        r.refutation_san = uci_line_to_san(&after, &r.refutation_uci);
        if r.refutation_uci.is_empty() {
            r.refutation_san.clear();
        }
    };
    let set_best = |r: &mut MoveReason, keep: usize| {
        if best_differs {
            r.better_uci = trim_line(&best_uci, keep);
            r.better_san = uci_line_to_san(inp.before, &r.better_uci);
        }
    };

    let mate_after = pov_mate(inp.eval_after, mover);
    let mate_before = pov_mate(inp.eval_before, mover);

    // 1. Allows a forced mate (unless the mover was already being mated anyway).
    let static_mate = tactics::mate_in_one(&after);
    let allows_mate = match mate_after {
        Some(a) if a < 0 => mate_before.map(|b| b > 0).unwrap_or(true),
        _ => static_mate.is_some() && mate_before.map(|b| b > 0).unwrap_or(true),
    };
    if allows_mate && !after.is_game_over() {
        let n = match mate_after {
            Some(a) if a < 0 => (-a) as u32,
            _ => 1,
        };
        r.kind = ReasonKind::AllowsMate;
        r.mate_in = Some(n);
        let plies = (2 * n as usize).saturating_sub(1).clamp(1, MAX_LINE);
        if refutation_uci.is_empty() || n == 1 {
            // Use the guaranteed mating move when the engine line is missing (or for mate in 1,
            // where the static probe is exact).
            if let Some(m) = &static_mate {
                r.refutation_uci = vec![gm_engine::move_to_uci(m)];
                r.refutation_san = uci_line_to_san(&after, &r.refutation_uci);
            } else {
                set_ref(&mut r, plies);
            }
        } else {
            set_ref(&mut r, plies);
        }
        if !r.refutation_uci.is_empty() {
            r.refutation_key = Some(r.refutation_uci.len() - 1);
        }
        set_best(&mut r, 6);
        return Some(r);
    }

    // 2. Missed a forced mate.
    if let Some(b) = mate_before.filter(|b| *b > 0) {
        if mate_after.map(|a| a <= 0).unwrap_or(true) && best_differs {
            r.kind = ReasonKind::MissedMate;
            r.mate_in = Some(b as u32);
            set_best(&mut r, (2 * b as usize).saturating_sub(1).clamp(1, MAX_LINE));
            if !r.better_uci.is_empty() {
                r.better_key = Some(r.better_uci.len() - 1);
            }
            set_ref(&mut r, 4);
            return Some(r);
        }
    }

    // 3. Material: compare what the played line and the best line do to the balance.
    let mut played_line = Vec::with_capacity(refutation_moves.len() + 1);
    played_line.push(inp.played.clone());
    played_line.extend(refutation_moves.iter().cloned());
    let played_out = outcome(inp.before, &played_line, mover);
    let best_out = if best_differs { outcome(inp.before, &best_moves, mover) } else { Outcome::default() };
    let diff = best_out.net - played_out.net;

    // When the better move wins more than the played line loses (e.g. the opponent's queen was
    // there for the taking and we dropped a pawn instead), the missed win is the real story.
    let missed_win = diff >= 1 && best_out.net >= 1 && !best_out.won.is_empty() && best_out.net > -played_out.net;
    if diff >= 1 && played_out.net <= -1 && !played_out.lost.is_empty() && !missed_win {
        r.material = played_out.net;
        r.lost = played_out.lost.iter().map(|(_, ro)| role_str(*ro).to_string()).collect();
        r.won = played_out.won.iter().map(|(_, ro)| role_str(*ro).to_string()).collect();
        // Index 0 of the played line is the played move itself.
        let key_line_idx = played_out.lost.first().map(|(i, _)| *i).unwrap_or(1);
        let key = key_line_idx.saturating_sub(1);
        r.refutation_key = Some(key);
        set_ref(&mut r, (key + 3).max(4));
        set_best(&mut r, 6);
        let lost_main = played_out.lost.first().map(|(_, ro)| *ro);

        // How was it won? Look at the opponent's first reply.
        r.kind = if key == 0 && played_out.won.is_empty() {
            ReasonKind::HangsPiece
        } else {
            ReasonKind::LosesMaterial
        };
        if let (Some(reply), Some(lost_role)) = (refutation_moves.first(), lost_main) {
            if key > 0 {
                let facts = tactics::move_facts(&after, reply);
                let fork_roles: Vec<Role> = facts.fork.iter().map(|(_, ro)| *ro).collect();
                if fork_roles.len() >= 2 && fork_roles.contains(&lost_role) {
                    r.kind = ReasonKind::AllowsFork;
                    r.targets = fork_roles.iter().take(3).map(|ro| role_str(*ro).to_string()).collect();
                } else if let Some(lt) = facts.lines.iter().find(|l| l.front.1 == lost_role || l.behind.1 == lost_role) {
                    r.kind = if lt.kind == LineKind::Pin { ReasonKind::AllowsPin } else { ReasonKind::AllowsSkewer };
                    r.targets = vec![role_str(lt.front.1).to_string(), role_str(lt.behind.1).to_string()];
                }
            }
        }
        return Some(r);
    }

    if missed_win && best_differs {
        r.material = best_out.net;
        r.won = best_out.won.iter().map(|(_, ro)| role_str(*ro).to_string()).collect();
        r.lost = best_out.lost.iter().map(|(_, ro)| role_str(*ro).to_string()).collect();
        let key = best_out.won.first().map(|(i, _)| *i).unwrap_or(0);
        r.better_key = Some(key);
        set_best(&mut r, (key + 2).max(4));
        set_ref(&mut r, 4);
        r.kind = ReasonKind::MissedMaterial;
        if let Some(first) = best_moves.first() {
            let facts = tactics::move_facts(inp.before, first);
            if facts.fork.len() >= 2 {
                r.kind = ReasonKind::MissedFork;
                r.targets = facts.fork.iter().take(3).map(|(_, ro)| role_str(*ro).to_string()).collect();
            }
        }
        return Some(r);
    }

    // 4. Nothing concrete: show the opponent's best answer (and whether it attacks something).
    r.kind = ReasonKind::Positional;
    r.material = played_out.net;
    set_ref(&mut r, 4);
    set_best(&mut r, 6);
    if let Some(reply) = refutation_moves.first() {
        let mut p = after.clone();
        p.play_unchecked(reply);
        let landed = reply.to();
        if reply.castling_side().is_none() {
            let attacked: Vec<Role> = tactics::fork_targets(p.board(), landed)
                .into_iter()
                .map(|(_, ro)| ro)
                .filter(|ro| *ro != Role::Pawn)
                .collect();
            if let Some(first) = attacked.first() {
                r.targets = vec![role_str(*first).to_string()];
                r.refutation_key = Some(0);
            }
        }
    }
    Some(r)
}

// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

/// Pick the string for `lang` from `[en, es, pt, fr, de]`.
fn l5(lang: Lang, s: [&'static str; 5]) -> &'static str {
    match lang {
        Lang::En => s[0],
        Lang::Es => s[1],
        Lang::Pt => s[2],
        Lang::Fr => s[3],
        Lang::De => s[4],
    }
}

/// "your queen" as a direct object ("tu dama", "sua dama", "ta dame", "deine Dame"/"deinen Turm").
fn your(role: Role, lang: Lang) -> String {
    match lang {
        Lang::En => format!("your {}", piece_name(role, lang)),
        Lang::Es => format!("tu {}", piece_name(role, lang)),
        Lang::Pt => {
            let n = pt_piece(role);
            format!("{} {}", if n.fem { "sua" } else { "seu" }, n.sing)
        }
        Lang::Fr => {
            let n = fr_piece(role);
            format!("{} {}", if n.fem { "ta" } else { "ton" }, n.sing)
        }
        Lang::De => {
            let n = de_piece(role);
            if n.fem {
                format!("deine {}", n.sing)
            } else {
                format!("deinen {}", n.obl)
            }
        }
    }
}

/// Group roles into (role, count), keeping first-seen order (already most valuable first).
fn group(roles: &[Role]) -> Vec<(Role, usize)> {
    let mut out: Vec<(Role, usize)> = Vec::new();
    for r in roles {
        match out.iter_mut().find(|(x, _)| x == r) {
            Some(e) => e.1 += 1,
            None => out.push((*r, 1)),
        }
    }
    out
}

fn parse_roles(v: &[String]) -> Vec<Role> {
    v.iter().filter_map(|s| parse_role(s)).collect()
}

/// "your queen", "a pawn", "your rook and 2 pawns".
fn lost_phrase(roles: &[Role], lang: Lang) -> String {
    let items: Vec<String> = group(roles)
        .into_iter()
        .take(3)
        .map(|(r, n)| match (n, r) {
            (1, Role::Pawn) => fill("{x_un_acc}", &PieceRef::new(r).vars("x", lang)),
            (1, _) => your(r, lang),
            _ => format!("{n} {}", piece_plural(r, lang)),
        })
        .collect();
    join_list(&items, lang)
}

/// "a pawn", "a knight and 2 pawns" (accusative in German).
fn some_phrase(roles: &[Role], lang: Lang) -> String {
    let items: Vec<String> = group(roles)
        .into_iter()
        .take(3)
        .map(|(r, n)| {
            if n == 1 {
                let v = PieceRef::new(r).vars("x", lang);
                fill("{x_un_acc}", &v)
            } else {
                format!("{n} {}", piece_plural(r, lang))
            }
        })
        .collect();
    join_list(&items, lang)
}

/// What a better move wins: "the queen", "a pawn", "the rook and 2 pawns" (accusative in German).
/// Pieces take the definite article, pawns the indefinite one.
fn won_phrase(roles: &[Role], lang: Lang) -> String {
    let items: Vec<String> = group(roles)
        .into_iter()
        .take(3)
        .map(|(r, n)| {
            let v = PieceRef::new(r).vars("x", lang);
            match (n, r) {
                (1, Role::Pawn) => fill("{x_un_acc}", &v),
                (1, _) => fill("{x_acc}", &v),
                _ => format!("{n} {}", piece_plural(r, lang)),
            }
        })
        .collect();
    join_list(&items, lang)
}

/// " for a pawn" / " for nothing".
fn for_phrase(won: &[Role], lang: Lang) -> String {
    if won.is_empty() {
        return l5(lang, [" for nothing", " a cambio de nada", " sem nada em troca", " pour rien", " ohne Gegenwert"]).to_string();
    }
    let w = some_phrase(won, lang);
    fill(l5(lang, [" for {w}", " a cambio de {w}", " em troca de {w}", " contre {w}", " für {w}"]), &kv(&[("w", &w)]))
}

/// "the queen and the rook" for the opponent's pieces (accusative in German).
fn their_phrase(roles: &[Role], lang: Lang) -> String {
    let items: Vec<String> = roles
        .iter()
        .take(3)
        .map(|r| {
            let v = PieceRef::new(*r).vars("x", lang);
            fill(if lang == Lang::Es { "{x_al}" } else { "{x_acc}" }, &v)
        })
        .collect();
    join_list(&items, lang)
}

/// A short SAN sequence ("Nxe5 dxe5 Qd5"), at most 3 moves; longer ones show only the key move.
fn seq(line: &[String], key: usize) -> String {
    let upto = (key + 1).min(line.len());
    if upto == 0 {
        return String::new();
    }
    if upto <= 3 {
        line[..upto].join(" ")
    } else {
        line[upto - 1].clone()
    }
}

/// The friendly text for `r` in `lang`. `played_san` is the move that was played.
pub fn render(r: &MoveReason, lang: Lang) -> String {
    let lost = parse_roles(&r.lost);
    let won = parse_roles(&r.won);
    let targets = parse_roles(&r.targets);
    let ref_key = r.refutation_key.unwrap_or(0);
    let key_move = r.refutation_san.get(ref_key).cloned().unwrap_or_default();
    let reply = r.refutation_san.first().cloned().unwrap_or_default();
    let best = r.better_san.first().cloned().unwrap_or_default();
    let better = if best.is_empty() {
        String::new()
    } else {
        fill(l5(lang, [" {b} was better.", " Era mejor {b}.", " {b} era melhor.", " {b} était meilleur.", " Besser war {b}."]), &kv(&[("b", &best)]))
    };
    let n = r.mate_in.unwrap_or(1).max(1);
    let n_s = n.to_string();
    let moves_word = plural(
        n as i64,
        l5(lang, ["move", "jugada", "lance", "coup", "Zug"]),
        l5(lang, ["moves", "jugadas", "lances", "coups", "Zügen"]),
        lang,
    );

    let text = match r.kind {
        ReasonKind::AllowsMate if key_move.is_empty() => l5(
            lang,
            [
                "This lets your opponent force checkmate.",
                "Esta jugada permite que tu rival fuerce el jaque mate.",
                "Este lance permite que o adversário force o xeque-mate.",
                "Ce coup permet à ton adversaire de forcer le mat.",
                "Dieser Zug erlaubt deinem Gegner ein erzwungenes Matt.",
            ],
        )
        .to_string(),
        ReasonKind::AllowsMate if n == 1 => fill(
            l5(
                lang,
                [
                    "This allows {m}, which is checkmate!",
                    "Esta jugada permite {m}, ¡que es jaque mate!",
                    "Este lance permite {m}, que é xeque-mate!",
                    "Ce coup permet {m}, qui fait échec et mat !",
                    "Dieser Zug erlaubt {m} – und das ist Schachmatt!",
                ],
            ),
            &kv(&[("m", &key_move)]),
        ),
        ReasonKind::AllowsMate => fill(
            l5(
                lang,
                [
                    "This lets your opponent force checkmate in {n} {w}, starting with {m}.",
                    "Esta jugada permite que tu rival fuerce el mate en {n} {w}, empezando por {m}.",
                    "Este lance permite que o adversário force o mate em {n} {w}, começando com {m}.",
                    "Ce coup permet à ton adversaire de forcer le mat en {n} {w}, en commençant par {m}.",
                    "Dieser Zug erlaubt deinem Gegner ein erzwungenes Matt in {n} {w}, beginnend mit {m}.",
                ],
            ),
            &kv(&[("n", &n_s), ("w", moves_word), ("m", &reply)]),
        ),
        ReasonKind::MissedMate if n == 1 => fill(
            l5(
                lang,
                [
                    "You had checkmate in one with {b}!",
                    "¡Tenías mate en una con {b}!",
                    "Você tinha mate em um com {b}!",
                    "Tu avais un mat en un coup avec {b} !",
                    "Du hattest Matt in einem Zug mit {b}!",
                ],
            ),
            &kv(&[("b", &best)]),
        ),
        ReasonKind::MissedMate => fill(
            l5(
                lang,
                [
                    "You had a forced checkmate in {n} {w}, starting with {b}.",
                    "Tenías un mate forzado en {n} {w}, empezando por {b}.",
                    "Você tinha um mate forçado em {n} {w}, começando com {b}.",
                    "Tu avais un mat forcé en {n} {w}, en commençant par {b}.",
                    "Du hattest ein erzwungenes Matt in {n} {w}, beginnend mit {b}.",
                ],
            ),
            &kv(&[("n", &n_s), ("w", moves_word), ("b", &best)]),
        ),
        ReasonKind::HangsPiece => {
            let role = lost.first().copied().unwrap_or(Role::Pawn);
            let mut v = PieceRef::new(role).vars("p", lang);
            v.extend(kv(&[("y", &your(role, lang)), ("m", &key_move)]));
            let s = fill(
                l5(
                    lang,
                    [
                        "This leaves {y} unprotected: {m} simply takes it.",
                        "Esta jugada deja {y} sin protección: {m} {p_lo} captura gratis.",
                        "Este lance deixa {y} sem proteção: {m} {p_lo} captura de graça.",
                        "Ce coup laisse {y} sans protection : {m} {p_lo} prend tout simplement.",
                        "Dieser Zug lässt {y} ungeschützt: {m} schlägt {p_lo} einfach.",
                    ],
                ),
                &v,
            );
            format!("{s}{better}")
        }
        ReasonKind::LosesMaterial => {
            let s = fill(
                l5(
                    lang,
                    [
                        "After {s}, you lose {l}{f}.",
                        "Tras {s}, pierdes {l}{f}.",
                        "Depois de {s}, você perde {l}{f}.",
                        "Après {s}, tu perds {l}{f}.",
                        "Nach {s} verlierst du {l}{f}.",
                    ],
                ),
                &kv(&[("s", &seq(&r.refutation_san, ref_key)), ("l", &lost_phrase(&lost, lang)), ("f", &for_phrase(&won, lang))]),
            );
            format!("{s}{better}")
        }
        ReasonKind::AllowsFork => {
            let t: Vec<String> = targets.iter().map(|ro| your(*ro, lang)).collect();
            let s = fill(
                l5(
                    lang,
                    [
                        "This allows {m}, a fork that attacks {t} at once — you end up losing {l}{f}.",
                        "Esta jugada permite {m}, un ataque doble contra {t}: acabas perdiendo {l}{f}.",
                        "Este lance permite {m}, um garfo em {t}: você acaba perdendo {l}{f}.",
                        "Ce coup permet {m}, une fourchette sur {t} : tu finis par perdre {l}{f}.",
                        "Dieser Zug erlaubt {m}, eine Gabel gegen {t} – am Ende verlierst du {l}{f}.",
                    ],
                ),
                &kv(&[("m", &reply), ("t", &join_list(&t, lang)), ("l", &lost_phrase(&lost, lang)), ("f", &for_phrase(&won, lang))]),
            );
            format!("{s}{better}")
        }
        ReasonKind::AllowsPin | ReasonKind::AllowsSkewer => {
            let front = targets.first().copied().unwrap_or(Role::Pawn);
            let behind = targets.get(1).copied().unwrap_or(Role::King);
            let tpl = if r.kind == ReasonKind::AllowsPin {
                l5(
                    lang,
                    [
                        "This allows {m}, which pins {a} to {b} — you end up losing {l}{f}.",
                        "Esta jugada permite {m}, que clava {a} contra {b}: acabas perdiendo {l}{f}.",
                        "Este lance permite {m}, que crava {a} contra {b}: você acaba perdendo {l}{f}.",
                        "Ce coup permet {m}, qui cloue {a} sur {b} : tu finis par perdre {l}{f}.",
                        "Dieser Zug erlaubt {m}: Das fesselt {a} an {b} – am Ende verlierst du {l}{f}.",
                    ],
                )
            } else {
                l5(
                    lang,
                    [
                        "This allows {m}, a skewer through {a} and {b} — you end up losing {l}{f}.",
                        "Esta jugada permite {m}, una enfilada contra {a} y {b}: acabas perdiendo {l}{f}.",
                        "Este lance permite {m}, um espeto em {a} e {b}: você acaba perdendo {l}{f}.",
                        "Ce coup permet {m}, une enfilade sur {a} et {b} : tu finis par perdre {l}{f}.",
                        "Dieser Zug erlaubt {m}, einen Spieß gegen {a} und {b} – am Ende verlierst du {l}{f}.",
                    ],
                )
            };
            let s = fill(
                tpl,
                &kv(&[
                    ("m", &reply),
                    ("a", &your(front, lang)),
                    ("b", &your(behind, lang)),
                    ("l", &lost_phrase(&lost, lang)),
                    ("f", &for_phrase(&won, lang)),
                ]),
            );
            format!("{s}{better}")
        }
        ReasonKind::MissedFork => fill(
            l5(
                lang,
                [
                    "You missed a fork: {b} attacks {t} at once and wins {w}.",
                    "Se te escapó un ataque doble: {b} ataca a la vez {t} y gana {w}.",
                    "Você deixou passar um garfo: {b} ataca {t} ao mesmo tempo e ganha {w}.",
                    "Tu as raté une fourchette : {b} attaque à la fois {t} et gagne {w}.",
                    "Hier gab es eine Gabel: {b} greift gleichzeitig {t} an und gewinnt {w}.",
                ],
            ),
            &kv(&[("b", &best), ("t", &their_phrase(&targets, lang)), ("w", &won_phrase(&won, lang))]),
        ),
        ReasonKind::MissedMaterial => fill(
            l5(
                lang,
                [
                    "You missed a chance: {b} wins {w}.",
                    "Se te escapó una oportunidad: {b} gana {w}.",
                    "Você deixou passar uma oportunidade: {b} ganha {w}.",
                    "Tu as laissé passer une occasion : {b} gagne {w}.",
                    "Hier gab es eine Chance: {b} gewinnt {w}.",
                ],
            ),
            &kv(&[("b", &best), ("w", &won_phrase(&won, lang))]),
        ),
        ReasonKind::Positional => {
            if reply.is_empty() {
                better.trim().to_string()
            } else if let Some(t) = targets.first() {
                let s = fill(
                    l5(
                        lang,
                        [
                            "Your opponent can answer {m}, attacking {y} and gaining time.",
                            "Tu rival puede responder {m}, atacando {y} y ganando tiempo.",
                            "Seu adversário pode responder {m}, atacando {y} e ganhando tempo.",
                            "Ton adversaire peut répondre {m}, qui attaque {y} et gagne du temps.",
                            "Dein Gegner kann mit {m} antworten, {y} angreifen und so Zeit gewinnen.",
                        ],
                    ),
                    &kv(&[("m", &reply), ("y", &your(*t, lang))]),
                );
                format!("{s}{better}")
            } else {
                let s = fill(
                    l5(
                        lang,
                        [
                            "You don't lose material right away, but after {m} your opponent gets the easier game.",
                            "No pierdes material enseguida, pero tras {m} tu rival tiene la partida más cómoda.",
                            "Você não perde material de imediato, mas depois de {m} o adversário fica com o jogo mais confortável.",
                            "Tu ne perds pas de matériel tout de suite, mais après {m} ton adversaire a la partie plus facile.",
                            "Du verlierst nicht sofort Material, aber nach {m} hat dein Gegner das angenehmere Spiel.",
                        ],
                    ),
                    &kv(&[("m", &reply)]),
                );
                format!("{s}{better}")
            }
        }
    };
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gm_engine::parse_fen;

    fn ucis(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    struct Case<'a> {
        fen: &'a str,
        played: &'a str,
        refutation: &'a str,
        best: &'a str,
        before: Score,
        after: Score,
        cls: Classification,
    }

    fn run(c: &Case<'_>) -> MoveReason {
        let pos = parse_fen(c.fen).expect("fen");
        let m = uci_to_move(&pos, c.played).expect("played move");
        let refutation = ucis(c.refutation);
        let best = ucis(c.best);
        let mut r = derive(&ReasonInput {
            before: &pos,
            played: &m,
            refutation: &refutation,
            best: &best,
            eval_before: c.before,
            eval_after: c.after,
            classification: c.cls,
        })
        .expect("a reason for an error");
        r.text = render(&r, Lang::En);
        r
    }

    fn all_langs_clean(r: &MoveReason) {
        for lang in Lang::ALL {
            let t = render(r, lang);
            assert!(!t.is_empty(), "{lang}");
            assert!(!t.contains('{') && !t.contains('}'), "{lang}: {t}");
        }
    }

    /// 1.e4 e5 2.Qh5 Nc6 3.Qxf7+?? Kxf7: the queen is lost for a pawn — not "the e4 pawn".
    #[test]
    fn queen_for_a_pawn() {
        let r = run(&Case {
            fen: "r1bqkbnr/pppp1ppp/2n5/4p2Q/4P3/8/PPPP1PPP/RNB1KBNR w KQkq - 2 3",
            played: "h5f7",
            refutation: "e8f7 f1c4 g8f6 b1c3",
            best: "f1c4 g7g6 h5f3 g8f6",
            before: Score::Cp(-20),
            after: Score::Cp(-850),
            cls: Classification::Blunder,
        });
        assert_eq!(r.kind, ReasonKind::LosesMaterial, "{r:?}");
        assert_eq!(r.lost, vec!["queen"]);
        assert_eq!(r.won, vec!["pawn"]);
        assert_eq!(r.refutation_key, Some(0));
        assert_eq!(r.text, "After Kxf7, you lose your queen for a pawn. Bc4 was better.");
        assert!(!r.text.contains("e4"));
        assert_eq!(render(&r, Lang::Es), "Tras Kxf7, pierdes tu dama a cambio de un peón. Era mejor Bc4.");
        assert_eq!(render(&r, Lang::De), "Nach Kxf7 verlierst du deine Dame für einen Bauern. Besser war Bc4.");
        all_langs_clean(&r);
    }

    /// A queen simply left en prise.
    #[test]
    fn hung_queen() {
        // White plays Qd1-d5?? and the pawn on e6 takes it.
        let r = run(&Case {
            fen: "4k3/8/4p3/8/8/8/8/3QK3 w - - 0 1",
            played: "d1d5",
            refutation: "e6d5 e1d2",
            best: "d1d4",
            before: Score::Cp(900),
            after: Score::Cp(-100),
            cls: Classification::Blunder,
        });
        assert_eq!(r.kind, ReasonKind::HangsPiece, "{r:?}");
        assert_eq!(r.lost, vec!["queen"]);
        assert_eq!(r.refutation_san.first().map(String::as_str), Some("exd5"));
        assert!(r.text.contains("your queen") && r.text.contains("exd5"), "{}", r.text);
        assert!(render(&r, Lang::Fr).contains("ta dame"), "{}", render(&r, Lang::Fr));
        assert!(render(&r, Lang::Pt).contains("sua dama"), "{}", render(&r, Lang::Pt));
        all_langs_clean(&r);
    }

    /// Allowing a back-rank mate in one.
    #[test]
    fn allowed_mate_in_one() {
        // Black to move; ...Kf8?? doesn't matter — any non-luft move allows Re8#. Black plays a6.
        let r = run(&Case {
            fen: "6k1/5ppp/p7/8/8/8/5PPP/4R1K1 b - - 0 1",
            played: "a6a5",
            refutation: "e1e8",
            best: "g7g6",
            before: Score::Cp(500),
            after: Score::Mate(1),
            cls: Classification::Blunder,
        });
        assert_eq!(r.kind, ReasonKind::AllowsMate, "{r:?}");
        assert_eq!(r.mate_in, Some(1));
        assert_eq!(r.refutation_san, vec!["Re8#"]);
        assert_eq!(r.refutation_key, Some(0));
        assert!(r.text.contains("Re8#") && r.text.contains("checkmate"), "{}", r.text);
        all_langs_clean(&r);
        // Even without an engine line, the static probe finds the mate.
        let r2 = run(&Case {
            fen: "6k1/5ppp/p7/8/8/8/5PPP/4R1K1 b - - 0 1",
            played: "a6a5",
            refutation: "",
            best: "",
            before: Score::Cp(0),
            after: Score::Cp(0),
            cls: Classification::Blunder,
        });
        assert_eq!(r2.kind, ReasonKind::AllowsMate);
        assert_eq!(r2.refutation_san, vec!["Re8#"]);
    }

    /// Missing a knight fork of king and rook.
    #[test]
    fn missed_fork() {
        // White knight on e5; Nf7+ forks the king on h8 and the rook on d8.
        let r = run(&Case {
            fen: "3r3k/6pp/8/4N3/8/8/6PP/6K1 w - - 0 1",
            played: "g2g3",
            refutation: "d8d2",
            best: "e5f7 h8g8 f7d8",
            before: Score::Cp(450),
            after: Score::Cp(20),
            cls: Classification::Miss,
        });
        assert_eq!(r.kind, ReasonKind::MissedFork, "{r:?}");
        assert_eq!(r.won, vec!["rook"]);
        assert!(r.targets.contains(&"king".to_string()) && r.targets.contains(&"rook".to_string()), "{r:?}");
        assert_eq!(r.better_key, Some(2));
        assert!(r.text.contains("fork") && r.text.contains("Nf7+") && r.text.contains("wins the rook"), "{}", r.text);
        assert!(render(&r, Lang::Es).contains("gana la torre"), "{}", render(&r, Lang::Es));
        assert!(render(&r, Lang::De).contains("gewinnt den Turm"), "{}", render(&r, Lang::De));
        all_langs_clean(&r);
    }

    /// Allowing a fork that wins the rook.
    #[test]
    fn allowed_fork() {
        // Black king g8, rook d8; White knight e5 can fork with Nf7 after Black plays ...Kh8?
        // Black's Kg8-h8?? walks into Nf7+ forking king and rook.
        let r = run(&Case {
            fen: "3r2k1/6pp/8/4N3/8/8/6PP/6K1 b - - 0 1",
            played: "g8h8",
            refutation: "e5f7 h8g8 f7d8",
            best: "d8d2",
            before: Score::Cp(0),
            after: Score::Cp(450),
            cls: Classification::Blunder,
        });
        assert_eq!(r.kind, ReasonKind::AllowsFork, "{r:?}");
        assert_eq!(r.lost, vec!["rook"]);
        assert!(r.text.contains("Nf7+") && r.text.contains("your rook"), "{}", r.text);
        all_langs_clean(&r);
    }

    /// Dropping a pawn while the opponent's queen was en prise: the missed queen is the story.
    #[test]
    fn missed_queen_beats_lost_pawn() {
        let r = run(&Case {
            fen: "4k3/8/8/3q4/4P3/8/2P5/4K3 w - - 0 1",
            played: "e1f1",
            refutation: "d5c4 f1e1 c4c2",
            best: "e4d5 e8d7",
            before: Score::Cp(900),
            after: Score::Cp(-900),
            cls: Classification::Blunder,
        });
        assert_eq!(r.kind, ReasonKind::MissedMaterial, "{r:?}");
        assert_eq!(r.text, "You missed a chance: exd5 wins the queen.");
        // The same played line without the hanging queen is a plain pawn loss, phrased "a pawn".
        let r = run(&Case {
            fen: "4k3/8/8/2q5/4P3/8/2P5/4K3 w - - 0 1",
            played: "e1f1",
            refutation: "c5c4 f1e1 c4c2",
            best: "e1d2",
            before: Score::Cp(0),
            after: Score::Cp(-900),
            cls: Classification::Mistake,
        });
        assert_eq!(r.kind, ReasonKind::LosesMaterial, "{r:?}");
        assert!(r.text.contains("you lose a pawn for nothing"), "{}", r.text);
        all_langs_clean(&r);
    }

    /// Missing a forced mate.
    #[test]
    fn missed_mate() {
        let r = run(&Case {
            fen: "6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1",
            played: "g2g3",
            refutation: "g8f8",
            best: "a1a8",
            before: Score::Mate(1),
            after: Score::Cp(300),
            cls: Classification::Miss,
        });
        assert_eq!(r.kind, ReasonKind::MissedMate, "{r:?}");
        assert_eq!(r.better_san, vec!["Ra8#"]);
        assert!(r.text.contains("Ra8#"), "{}", r.text);
        all_langs_clean(&r);
    }

    /// No material changes hands: the reason names the opponent's best reply and the threat.
    #[test]
    fn positional_with_tempo() {
        // 1.e4 e6 2.Bc4?! d5: the bishop is hit with tempo.
        let r = run(&Case {
            fen: "rnbqkbnr/pppp1ppp/4p3/8/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2",
            played: "f1c4",
            refutation: "d7d5 e4d5 e6d5 c4b3",
            best: "d2d4 d7d5 b1c3",
            before: Score::Cp(30),
            after: Score::Cp(-30),
            cls: Classification::Inaccuracy,
        });
        assert_eq!(r.kind, ReasonKind::Positional, "{r:?}");
        assert_eq!(r.targets, vec!["bishop"]);
        assert!(r.text.contains("d5") && r.text.contains("your bishop") && r.text.contains("d4"), "{}", r.text);
        all_langs_clean(&r);
    }

    #[test]
    fn non_errors_and_garbage() {
        let pos = parse_fen(gm_engine::START_FEN).expect("fen");
        let m = uci_to_move(&pos, "e2e4").expect("move");
        let junk = ucis("zz99 e7e5 a1a8");
        let inp = |cls| ReasonInput {
            before: &pos,
            played: &m,
            refutation: &junk,
            best: &junk,
            eval_before: Score::Cp(0),
            eval_after: Score::Cp(0),
            classification: cls,
        };
        assert!(derive(&inp(Classification::Best)).is_none());
        let r = derive(&inp(Classification::Mistake)).expect("reason");
        assert_eq!(r.kind, ReasonKind::Positional);
        assert!(r.refutation_uci.is_empty());
        for lang in Lang::ALL {
            assert!(!render(&r, lang).contains('{'));
        }
    }
}
