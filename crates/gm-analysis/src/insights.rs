//! Personal weakness tracker: aggregates patterns across a player's reviewed games.
//!
//! Everything here is pure (no engine, no I/O): it reads stored [`GameReview`]s and the board
//! motifs from `gm_mentor::tactics`, and returns an [`Insights`] report — top weaknesses with
//! evidence and a one-click drill, plus breakdowns (errors by phase, pieces hung, missed tactics
//! by theme, accuracy trend, results by colour / opening, converting winning positions and,
//! when clock data exists, time trouble). Human text is written in the requested [`Lang`].

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;
use shakmaty::{attacks, Chess, Color, Move, Position, Role};

use gm_content::Lang;
use gm_engine::{fen_key, parse_fen, uci_to_move, Score};
use gm_mentor::tactics::{self, LineKind};

use crate::{Classification, GameReview, MoveReview};

/// Fewer reviewed games than this and the page shows its encouraging empty state.
pub const MIN_GAMES: usize = 3;
/// Most games aggregated in one report (most recent first).
pub const MAX_GAMES: usize = 200;
/// Evidence positions kept per weakness.
const MAX_EXAMPLES: usize = 3;
/// Accuracy-trend points returned (most recent).
const MAX_TREND: usize = 40;
/// Openings listed in the results-by-opening breakdown.
const MAX_OPENINGS: usize = 6;
/// Failed conversions listed.
const MAX_FAILED: usize = 5;
/// User-POV centipawns a position must reach to count as "clearly winning" (≈ +3).
const WINNING_CP: i32 = 300;
/// Seconds left on the clock below which the player is "in time trouble".
pub const LOW_TIME_SECS: u32 = 30;
/// Last full move number counted as the opening.
const OPENING_LAST_MOVE: u32 = 10;
/// Total non-pawn material (both sides, points) at or below which we are in an endgame.
const ENDGAME_MATERIAL: i32 = 20;

// ---------------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------------

/// One reviewed game, as seen from the user's side.
#[derive(Clone, Debug)]
pub struct GameInput {
    pub id: i64,
    /// The side the user played.
    pub user: Color,
    /// "1-0" | "0-1" | "1/2-1/2" | "*"
    pub result: String,
    /// ISO timestamp (used for ordering and the trend chart).
    pub date: String,
    pub opening: Option<String>,
    pub review: GameReview,
    /// Seconds left after each ply (index = ply - 1), from PGN `[%clk]` comments. Empty when
    /// the game has no clock data.
    pub clocks: Vec<Option<u32>>,
}

// ---------------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Example {
    pub game_id: i64,
    /// 1-based ply of the user's move (open Game Review at this ply).
    pub ply: usize,
    /// Position before the move.
    pub fen: String,
    pub move_uci: String,
    pub move_san: String,
    pub best_uci: String,
    pub best_san: String,
    /// "white" | "black" — the user's side (board orientation).
    pub color: String,
    pub classification: Classification,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Drill {
    pub href: String,
    pub label: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Weakness {
    /// hanging_pieces | missed_tactics | endgame | opening | conversion | repeated | time_trouble
    pub id: String,
    pub title: String,
    pub explanation: String,
    /// How many times it happened.
    pub count: u32,
    /// In how many games.
    pub games: u32,
    /// Relative severity (higher = more urgent).
    pub score: f32,
    /// Puzzle theme for `missed_tactics`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub examples: Vec<Example>,
    pub drill: Drill,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct PhaseStats {
    /// opening | middlegame | endgame
    pub phase: String,
    /// User moves played in this phase.
    pub moves: u32,
    pub inaccuracies: u32,
    pub mistakes: u32,
    pub blunders: u32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct PieceCount {
    /// pawn | knight | bishop | rook | queen
    pub piece: String,
    pub count: u32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct ThemeCount {
    /// Puzzle theme id (fork, pin, skewer, discoveredAttack, backRankMate, hangingPiece, mateIn1..3)
    pub theme: String,
    pub count: u32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct TrendPoint {
    pub game_id: i64,
    pub date: String,
    pub accuracy: f32,
    /// win | loss | draw | ongoing
    pub outcome: String,
    pub color: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Record {
    pub games: u32,
    pub wins: u32,
    pub losses: u32,
    pub draws: u32,
    /// Average user accuracy (None without data).
    pub accuracy: Option<f32>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct OpeningRecord {
    pub name: String,
    #[serde(flatten)]
    pub record: Record,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct FailedConversion {
    pub game_id: i64,
    /// Ply where the advantage started slipping (or where it was reached).
    pub ply: usize,
    pub fen: String,
    pub color: String,
    /// draw | loss
    pub outcome: String,
    /// Best user-POV centipawn advantage reached (mate = 10000).
    pub best_cp: i32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Conversion {
    /// Finished games where the user reached ≥ +3.
    pub winning_games: u32,
    pub converted: u32,
    pub failed: Vec<FailedConversion>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct TimeTrouble {
    pub games_with_clock: u32,
    pub low_time_secs: u32,
    /// User moves made with little time left / their errors.
    pub low_time_moves: u32,
    pub low_time_errors: u32,
    /// Other user moves (with clock data) / their errors.
    pub normal_moves: u32,
    pub normal_errors: u32,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Insights {
    pub games_analyzed: u32,
    pub min_games: u32,
    /// Enough reviewed games for a meaningful report.
    pub ready: bool,
    /// Top weaknesses, most urgent first (at most 3).
    pub weaknesses: Vec<Weakness>,
    pub phases: Vec<PhaseStats>,
    pub hanging: Vec<PieceCount>,
    pub tactics: Vec<ThemeCount>,
    pub accuracy_trend: Vec<TrendPoint>,
    pub average_accuracy: Option<f32>,
    pub by_color: BTreeMap<String, Record>,
    pub by_opening: Vec<OpeningRecord>,
    pub conversion: Conversion,
    /// None when no game has clock data.
    pub time_trouble: Option<TimeTrouble>,
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    Opening,
    Middlegame,
    Endgame,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Opening => "opening",
            Phase::Middlegame => "middlegame",
            Phase::Endgame => "endgame",
        }
    }
}

/// Game phase of a position: endgame by remaining material, opening by move number.
pub fn phase_of(pos: &Chess) -> Phase {
    let board = pos.board();
    let mut npm = 0;
    for role in [Role::Knight, Role::Bishop, Role::Rook, Role::Queen] {
        npm += tactics::points(role) * board.by_role(role).count() as i32;
    }
    let queens = board.by_role(Role::Queen).count();
    if npm <= ENDGAME_MATERIAL || (queens == 0 && npm <= 26) {
        Phase::Endgame
    } else if pos.fullmoves().get() <= OPENING_LAST_MOVE {
        Phase::Opening
    } else {
        Phase::Middlegame
    }
}

fn color_str(c: Color) -> &'static str {
    match c {
        Color::White => "white",
        Color::Black => "black",
    }
}

fn is_error(c: Classification) -> bool {
    matches!(c, Classification::Mistake | Classification::Miss | Classification::Blunder)
}

/// win | loss | draw | ongoing from the user's point of view.
fn outcome(result: &str, user: Color) -> &'static str {
    match (result, user) {
        ("1-0", Color::White) | ("0-1", Color::Black) => "win",
        ("1-0", Color::Black) | ("0-1", Color::White) => "loss",
        ("1/2-1/2", _) => "draw",
        _ => "ongoing",
    }
}

/// User-POV centipawns (mate = ±10000).
fn user_cp(score: Score, user: Color) -> i32 {
    let white = match score {
        Score::Cp(c) => c,
        Score::Mate(m) if m >= 0 => 10_000,
        Score::Mate(_) => -10_000,
    };
    if user == Color::White {
        white
    } else {
        -white
    }
}

fn example(game: &GameInput, m: &MoveReview) -> Example {
    Example {
        game_id: game.id,
        ply: m.ply,
        fen: m.fen_before.clone(),
        move_uci: m.uci.clone(),
        move_san: m.san.clone(),
        best_uci: m.best_move_uci.clone(),
        best_san: m.best_move_san.clone(),
        color: color_str(game.user).to_string(),
        classification: m.classification,
    }
}

/// Games where a position went wrong, and a few weighted examples from it.
type RepeatedSpot = (HashSet<i64>, Vec<(f32, Example)>);

/// Collects evidence for one weakness: counts and the most instructive examples.
#[derive(Default)]
struct Evidence {
    count: u32,
    games: HashSet<i64>,
    /// (severity, example) — the biggest win% losses are kept.
    examples: Vec<(f32, Example)>,
}

impl Evidence {
    fn add(&mut self, weight: f32, ex: Example) {
        self.count += 1;
        self.games.insert(ex.game_id);
        self.examples.push((weight, ex));
        // Keep memory bounded: trim to the best few per game set.
        if self.examples.len() > 64 {
            self.trim();
        }
    }

    fn trim(&mut self) {
        self.examples.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        let mut rest = Vec::new();
        for e in self.examples.drain(..) {
            if seen.insert(e.1.game_id) {
                kept.push(e);
            } else {
                rest.push(e);
            }
        }
        kept.extend(rest);
        kept.truncate(16);
        self.examples = kept;
    }

    /// Best examples, preferring distinct games.
    fn top(mut self) -> (u32, u32, Vec<Example>) {
        self.trim();
        let ex = self.examples.into_iter().take(MAX_EXAMPLES).map(|(_, e)| e).collect();
        (self.count, self.games.len() as u32, ex)
    }
}

/// The puzzle theme of the tactic the best move `m` would have executed, if it is one.
pub fn tactic_theme(pos: &Chess, m: &Move, eval_before: Score) -> Option<&'static str> {
    let facts = tactics::move_facts(pos, m);
    if facts.is_mate {
        return Some(if tactics::is_back_rank_mate(pos, m) { "backRankMate" } else { "mateIn1" });
    }
    // A forced mate in 2-3 for the mover.
    if let Score::Mate(n) = eval_before {
        let mover_mates = (n > 0 && pos.turn() == Color::White) || (n < 0 && pos.turn() == Color::Black);
        match n.unsigned_abs() {
            2 if mover_mates => return Some("mateIn2"),
            3 if mover_mates => return Some("mateIn3"),
            _ => {}
        }
    }
    if !facts.fork.is_empty() {
        return Some("fork");
    }
    if facts.lines.iter().any(|l| l.kind == LineKind::Pin && l.behind.1 == Role::King) {
        return Some("pin");
    }
    if facts.lines.iter().any(|l| l.kind == LineKind::Skewer) {
        return Some("skewer");
    }
    if is_discovered_attack(pos, m) {
        return Some("discoveredAttack");
    }
    if facts.lines.iter().any(|l| l.kind == LineKind::Pin) {
        return Some("pin");
    }
    if facts.captured.is_some() && facts.capture_net >= 250 {
        return Some("hangingPiece");
    }
    None
}

/// Moving a piece off a line uncovers an attack by a friendly slider on a king or on
/// something that can then be won.
fn is_discovered_attack(pos: &Chess, m: &Move) -> bool {
    let Some(from) = m.from() else { return false };
    if m.is_castle() {
        return false;
    }
    let mover = pos.turn();
    let mut after = pos.clone();
    after.play_unchecked(m);
    let before = pos.board();
    let board = after.board();
    let sliders = board.by_color(mover) & (board.bishops() | board.rooks() | board.queens());
    for s in sliders {
        if s == m.to() {
            continue;
        }
        let them_before = before.by_color(!mover);
        let them_after = board.by_color(!mover);
        let new = (board.attacks_from(s) & them_after) & !(before.attacks_from(s) & them_before);
        for t in new {
            if !attacks::between(s, t).contains(from) {
                continue;
            }
            let role = board.role_at(t);
            if role == Some(Role::King) || tactics::see(board, t, mover) >= 300 {
                return true;
            }
        }
    }
    false
}

/// The piece a bad move left en prise (biggest first), if any.
pub fn hung_piece(pos: &Chess, m: &Move) -> Option<Role> {
    let facts = tactics::move_facts(pos, m);
    if facts.is_mate {
        return None;
    }
    if let Some((h, _, _)) = facts.hangs.first() {
        if h.gain >= 100 {
            return Some(h.role);
        }
    }
    if facts.capture_net <= -200 {
        return Some(facts.role);
    }
    None
}

/// Seconds left after each ply, from `[%clk h:mm:ss]` comments in PGN movetext. Returns an
/// empty vector when the PGN has no clock comments. Bounded by `max_plies`.
pub fn parse_clocks(pgn: &str, max_plies: usize) -> Vec<Option<u32>> {
    if !pgn.contains("[%clk") {
        return Vec::new();
    }
    // Movetext only: skip tag-pair lines.
    let text: String = pgn
        .lines()
        .filter(|l| !l.trim_start().starts_with('['))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: Vec<Option<u32>> = Vec::new();
    let mut rest = text.as_str();
    // Walk tokens: a SAN-ish move pushes a slot; a following clk comment fills it.
    while !rest.is_empty() && out.len() <= max_plies {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(r) = rest.strip_prefix('{') {
            let end = r.find('}').unwrap_or(r.len());
            let comment = &r[..end];
            if let Some(i) = comment.find("[%clk") {
                let v = comment[i + 5..].trim_start();
                let v = &v[..v.find(']').unwrap_or(v.len())];
                if let (Some(secs), Some(last)) = (parse_hms(v.trim()), out.last_mut()) {
                    *last = Some(secs);
                }
            }
            rest = r.get(end + 1..).unwrap_or("");
            continue;
        }
        let end = rest.find(|c: char| c.is_whitespace() || c == '{').unwrap_or(rest.len());
        let tok = &rest[..end];
        rest = &rest[end..];
        let tok = tok.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        if tok.is_empty() || tok.starts_with('$') || matches!(tok, "1-0" | "0-1" | "1/2-1/2" | "*") {
            continue;
        }
        if tok.starts_with(|c: char| c.is_ascii_alphabetic()) {
            out.push(None);
        }
    }
    out.truncate(max_plies);
    if out.iter().all(Option::is_none) {
        Vec::new()
    } else {
        out
    }
}

fn parse_hms(s: &str) -> Option<u32> {
    let mut total: u32 = 0;
    let mut parts = 0;
    for p in s.split(':') {
        parts += 1;
        if parts > 3 {
            return None;
        }
        let whole = p.split('.').next()?;
        let n: u32 = whole.parse().ok()?;
        total = total.checked_mul(60)?.checked_add(n)?;
    }
    (parts >= 2).then_some(total)
}

fn opening_family(name: &str) -> String {
    let n = name.split(':').next().unwrap_or(name).trim();
    n.chars().take(60).collect()
}

fn average(xs: &[f32]) -> Option<f32> {
    if xs.is_empty() {
        None
    } else {
        Some((xs.iter().sum::<f32>() / xs.len() as f32 * 10.0).round() / 10.0)
    }
}

fn user_accuracy(g: &GameInput) -> f32 {
    match g.user {
        Color::White => g.review.white.accuracy,
        Color::Black => g.review.black.accuracy,
    }
}

// ---------------------------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------------------------

/// Build the insights report. `games` may be in any order; at most [`MAX_GAMES`] most recent
/// are used.
pub fn aggregate(games: &[GameInput], lang: Lang) -> Insights {
    let mut games: Vec<&GameInput> = games.iter().filter(|g| !g.review.moves.is_empty()).collect();
    games.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
    games.truncate(MAX_GAMES);
    let n = games.len();

    let mut out = Insights {
        games_analyzed: n as u32,
        min_games: MIN_GAMES as u32,
        ready: n >= MIN_GAMES,
        ..Default::default()
    };

    let mut phases: BTreeMap<Phase, PhaseStats> = BTreeMap::new();
    for p in [Phase::Opening, Phase::Middlegame, Phase::Endgame] {
        phases.insert(p, PhaseStats { phase: p.as_str().into(), ..Default::default() });
    }
    let mut hanging: BTreeMap<Role, u32> = BTreeMap::new();
    let mut themes: HashMap<&'static str, Evidence> = HashMap::new();
    let mut ev_hanging = Evidence::default();
    let mut ev_endgame = Evidence::default();
    let mut ev_opening = Evidence::default();
    let mut ev_conversion = Evidence::default();
    let mut ev_time = Evidence::default();
    let mut endgame_games: HashSet<i64> = HashSet::new();
    // fen key -> (game ids, examples)
    let mut repeated: HashMap<String, RepeatedSpot> = HashMap::new();
    let mut tt = TimeTrouble { low_time_secs: LOW_TIME_SECS, ..Default::default() };
    let mut accs: Vec<f32> = Vec::new();
    let mut by_color: BTreeMap<String, (Record, Vec<f32>)> = BTreeMap::new();
    let mut by_opening: HashMap<String, (Record, Vec<f32>)> = HashMap::new();

    for g in &games {
        let user = g.user;
        let oc = outcome(&g.result, user);
        let acc = user_accuracy(g);
        let has_acc = acc.is_finite() && acc > 0.0;
        if has_acc {
            accs.push(acc);
        }

        // Results by colour / opening.
        let bump = |rec: &mut (Record, Vec<f32>)| {
            rec.0.games += 1;
            match oc {
                "win" => rec.0.wins += 1,
                "loss" => rec.0.losses += 1,
                "draw" => rec.0.draws += 1,
                _ => {}
            }
            if has_acc {
                rec.1.push(acc);
            }
        };
        bump(by_color.entry(color_str(user).to_string()).or_default());
        let opening = g
            .opening
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| g.review.opening.as_ref().map(|o| o.name.clone()).filter(|s| !s.is_empty()));
        if let Some(name) = opening {
            bump(by_opening.entry(opening_family(&name)).or_default());
        }

        let has_clock = g.clocks.iter().any(Option::is_some);
        if has_clock {
            tt.games_with_clock += 1;
        }

        for (i, m) in g.review.moves.iter().enumerate() {
            if m.color != color_str(user) {
                continue;
            }
            let Ok(pos) = parse_fen(&m.fen_before) else { continue };
            let phase = phase_of(&pos);
            let cls = m.classification;
            {
                let ps = phases.entry(phase).or_default();
                ps.moves += 1;
                match cls {
                    Classification::Inaccuracy => ps.inaccuracies += 1,
                    Classification::Mistake | Classification::Miss => ps.mistakes += 1,
                    Classification::Blunder => ps.blunders += 1,
                    _ => {}
                }
            }
            if phase == Phase::Endgame {
                endgame_games.insert(g.id);
            }

            // Clock.
            if has_clock {
                if let Some(Some(left)) = g.clocks.get(i) {
                    // Time left *before* the move is the clock after the user's previous move.
                    let before = i.checked_sub(2).and_then(|j| g.clocks.get(j).copied().flatten()).unwrap_or(*left);
                    if before < LOW_TIME_SECS {
                        tt.low_time_moves += 1;
                        if is_error(cls) {
                            tt.low_time_errors += 1;
                            ev_time.add(m.win_chance_loss, example(g, m));
                        }
                    } else {
                        tt.normal_moves += 1;
                        if is_error(cls) {
                            tt.normal_errors += 1;
                        }
                    }
                }
            }

            if !is_error(cls) {
                continue;
            }
            let ex = example(g, m);
            let w = m.win_chance_loss;

            match phase {
                Phase::Endgame => ev_endgame.add(w, ex.clone()),
                Phase::Opening => ev_opening.add(w, ex.clone()),
                Phase::Middlegame => {}
            }

            // Hung a piece?
            if cls != Classification::Miss {
                if let Ok(played) = uci_to_move(&pos, &m.uci) {
                    if let Some(role) = hung_piece(&pos, &played) {
                        *hanging.entry(role).or_default() += 1;
                        ev_hanging.add(w, ex.clone());
                    }
                }
            }

            // Missed a tactic?
            if !m.best_move_uci.is_empty() && m.best_move_uci != m.uci {
                if let Ok(best) = uci_to_move(&pos, &m.best_move_uci) {
                    if let Some(theme) = tactic_theme(&pos, &best, m.eval_before) {
                        themes.entry(theme).or_default().add(w, ex.clone());
                    }
                }
            }

            // Same position, same kind of slip, in different games.
            let entry = repeated.entry(fen_key(&m.fen_before)).or_default();
            entry.0.insert(g.id);
            if entry.1.len() < 4 {
                entry.1.push((w, ex));
            }
        }

        // Converting winning positions (finished games only).
        if oc != "ongoing" {
            let evals = &g.review.evals;
            // Skip the final position (a finished game's terminal score is not a "lead").
            let upto = evals.len().saturating_sub(1);
            let reached = evals[..upto]
                .iter()
                .position(|s| user_cp(*s, user) >= WINNING_CP);
            if let Some(k) = reached {
                out.conversion.winning_games += 1;
                if oc == "win" {
                    out.conversion.converted += 1;
                } else {
                    let best_cp = evals[..upto].iter().map(|s| user_cp(*s, user)).max().unwrap_or(0);
                    // The user's worst move after the lead appeared.
                    let slip = g
                        .review
                        .moves
                        .iter()
                        .filter(|m| m.ply > k && m.color == color_str(user))
                        .max_by(|a, b| a.win_chance_loss.total_cmp(&b.win_chance_loss));
                    let (ply, fen) = match slip {
                        Some(m) => (m.ply, m.fen_before.clone()),
                        None => {
                            let m = g.review.moves.get(k.min(g.review.moves.len().saturating_sub(1)));
                            (m.map(|m| m.ply).unwrap_or(1), m.map(|m| m.fen_before.clone()).unwrap_or_default())
                        }
                    };
                    if let Some(m) = slip {
                        ev_conversion.add(m.win_chance_loss + 100.0, example(g, m));
                    } else if let Some(m) = g.review.moves.get(k) {
                        ev_conversion.add(0.0, example(g, m));
                    }
                    if out.conversion.failed.len() < MAX_FAILED {
                        out.conversion.failed.push(FailedConversion {
                            game_id: g.id,
                            ply,
                            fen,
                            color: color_str(user).into(),
                            outcome: oc.into(),
                            best_cp: best_cp.min(10_000),
                        });
                    }
                }
            }
        }
    }

    // ---- breakdowns ----
    out.phases = phases.into_values().collect();
    out.hanging = [Role::Pawn, Role::Knight, Role::Bishop, Role::Rook, Role::Queen]
        .into_iter()
        .map(|r| PieceCount { piece: tactics::role_name(r).into(), count: hanging.get(&r).copied().unwrap_or(0) })
        .collect();
    let mut theme_list: Vec<ThemeCount> =
        themes.iter().map(|(t, e)| ThemeCount { theme: (*t).into(), count: e.count }).collect();
    theme_list.sort_by(|a, b| b.count.cmp(&a.count).then(a.theme.cmp(&b.theme)));
    out.tactics = theme_list;

    let mut trend: Vec<TrendPoint> = games
        .iter()
        .take(MAX_TREND)
        .filter(|g| user_accuracy(g) > 0.0)
        .map(|g| TrendPoint {
            game_id: g.id,
            date: g.date.clone(),
            accuracy: (user_accuracy(g) * 10.0).round() / 10.0,
            outcome: outcome(&g.result, g.user).into(),
            color: color_str(g.user).into(),
        })
        .collect();
    trend.reverse();
    out.accuracy_trend = trend;
    out.average_accuracy = average(&accs);
    out.by_color = by_color
        .into_iter()
        .map(|(k, (mut r, a))| {
            r.accuracy = average(&a);
            (k, r)
        })
        .collect();
    let mut openings: Vec<OpeningRecord> = by_opening
        .into_iter()
        .map(|(name, (mut r, a))| {
            r.accuracy = average(&a);
            OpeningRecord { name, record: r }
        })
        .collect();
    openings.sort_by(|a, b| b.record.games.cmp(&a.record.games).then(a.name.cmp(&b.name)));
    openings.truncate(MAX_OPENINGS);
    out.by_opening = openings;
    if tt.games_with_clock > 0 {
        out.time_trouble = Some(tt.clone());
    }

    // ---- weaknesses ----
    let per_game = |c: u32| c as f32 / n.max(1) as f32;
    let mut cands: Vec<Weakness> = Vec::new();
    let mut push = |id: &str, ev: Evidence, score_of: &dyn Fn(u32, u32) -> f32, min: u32, theme: Option<&str>, href: String| {
        let (count, games_n, examples) = ev.top();
        if count < min {
            return;
        }
        let score = score_of(count, games_n);
        let (title, explanation, label) = texts(lang, id, count, games_n, theme);
        cands.push(Weakness {
            id: id.into(),
            title,
            explanation,
            count,
            games: games_n,
            score: (score * 100.0).round() / 100.0,
            theme: theme.map(str::to_string),
            examples,
            drill: Drill { href, label },
        });
    };

    push("hanging_pieces", ev_hanging, &|c, _| per_game(c) * 1.3, 2, None, "#/drills/hanging".into());

    // Missed tactics: report the most frequent theme (all themes count toward the score).
    let total_missed: u32 = themes.values().map(|e| e.count).sum();
    if let Some(top) = out.tactics.first().map(|t| t.theme.clone()) {
        let theme: &'static str = themes.keys().copied().find(|k| *k == top).unwrap_or("fork");
        if let Some(ev) = themes.remove(theme) {
            let href = format!("#/puzzles?theme={theme}");
            push("missed_tactics", ev, &|_, _| per_game(total_missed) * 1.1, 2, Some(theme), href);
        }
    }

    let eg_games = endgame_games.len() as u32;
    push(
        "endgame",
        ev_endgame,
        &|c, _| c as f32 / eg_games.max(1) as f32 * 1.0 * (eg_games as f32 / n.max(1) as f32).sqrt(),
        2,
        None,
        "#/endgames".into(),
    );
    push("opening", ev_opening, &|c, _| per_game(c) * 0.9, 2, None, "#/repertoire".into());

    let conv_href = out
        .conversion
        .failed
        .first()
        .map(|f| {
            let side = if f.color == "black" { "b" } else { "w" };
            format!("#/play?fen={}&color={side}", encode_uri_component(&f.fen))
        })
        .unwrap_or_else(|| "#/play".into());
    push("conversion", ev_conversion, &|c, _| per_game(c) * 2.0, 1, None, conv_href);

    let mut ev_repeated = Evidence::default();
    for (_, (ids, exs)) in repeated {
        if ids.len() >= 2 {
            for (w, ex) in exs {
                ev_repeated.add(w, ex);
            }
        }
    }
    push("repeated", ev_repeated, &|c, _| per_game(c) * 1.0, 2, None, "#/puzzles/mistakes".into());

    if tt.low_time_errors >= 2 {
        let low_rate = tt.low_time_errors as f32 / tt.low_time_moves.max(1) as f32;
        let normal_rate = tt.normal_errors as f32 / tt.normal_moves.max(1) as f32;
        if low_rate > normal_rate * 1.5 {
            push("time_trouble", ev_time, &|c, _| per_game(c) * 1.2, 2, None, "#/puzzles/rush".into());
        }
    }

    cands.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.count.cmp(&a.count)));
    cands.truncate(3);
    out.weaknesses = cands;
    out
}

/// `encodeURIComponent` for FENs in drill links.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------------------------

fn theme_name(lang: Lang, theme: &str) -> &'static str {
    match (lang, theme) {
        (Lang::En, "fork") => "forks",
        (Lang::En, "pin") => "pins",
        (Lang::En, "skewer") => "skewers",
        (Lang::En, "discoveredAttack") => "discovered attacks",
        (Lang::En, "backRankMate") => "back-rank mates",
        (Lang::En, "hangingPiece") => "free pieces",
        (Lang::En, "mateIn1") => "mates in one",
        (Lang::En, "mateIn2") => "mates in two",
        (Lang::En, "mateIn3") => "mates in three",
        (Lang::En, _) => "tactics",
        (Lang::Es, "fork") => "ataques dobles",
        (Lang::Es, "pin") => "clavadas",
        (Lang::Es, "skewer") => "enfiladas",
        (Lang::Es, "discoveredAttack") => "ataques a la descubierta",
        (Lang::Es, "backRankMate") => "mates del pasillo",
        (Lang::Es, "hangingPiece") => "piezas colgadas",
        (Lang::Es, "mateIn1") => "mates en una",
        (Lang::Es, "mateIn2") => "mates en dos",
        (Lang::Es, "mateIn3") => "mates en tres",
        (Lang::Es, _) => "tácticas",
    }
}

fn times(lang: Lang, n: u32) -> String {
    match (lang, n) {
        (Lang::En, 1) => "once".into(),
        (Lang::En, 2) => "twice".into(),
        (Lang::En, n) => format!("{n} times"),
        (Lang::Es, 1) => "una vez".into(),
        (Lang::Es, n) => format!("{n} veces"),
    }
}

fn games_word(lang: Lang, n: u32) -> String {
    match (lang, n) {
        (Lang::En, 1) => "1 game".into(),
        (Lang::En, n) => format!("{n} games"),
        (Lang::Es, 1) => "1 partida".into(),
        (Lang::Es, n) => format!("{n} partidas"),
    }
}

/// (title, explanation, drill label) for a weakness.
fn texts(lang: Lang, id: &str, count: u32, games: u32, theme: Option<&str>) -> (String, String, String) {
    let t = times(lang, count);
    let g = games_word(lang, games);
    let th = theme_name(lang, theme.unwrap_or(""));
    let (a, b, c) = match lang {
        Lang::En => match id {
            "hanging_pieces" => (
                "Leaving pieces unprotected".to_string(),
                format!("You left a piece where it could be taken for free {t} across {g}. Before every move, ask: \"Is everything I own still protected?\""),
                "Practice spotting hanging pieces".to_string(),
            ),
            "missed_tactics" => (
                format!("Missing {th}"),
                format!("You missed winning {th} {t} across {g}. Look for checks, captures and attacks first: the tactic is often already there."),
                format!("Practice spotting {th}"),
            ),
            "endgame" => (
                "Slipping in the endgame".to_string(),
                format!("When only a few pieces are left, you made {count} costly mistakes in {g}. Endgames reward calm technique: activate your king and push passed pawns."),
                "Train endgames".to_string(),
            ),
            "opening" => (
                "Trouble in the opening".to_string(),
                format!("You made {count} costly mistakes in the first ten moves of {g}. A small, well-known repertoire gets you to a good middlegame safely."),
                "Build your repertoire".to_string(),
            ),
            "conversion" => (
                "Letting winning games slip".to_string(),
                format!("You were clearly winning (+3 or more) and didn't win {t}. When you're ahead, trade pieces, keep your king safe and avoid giving counterplay."),
                "Practice winning a won position".to_string(),
            ),
            "repeated" => (
                "Repeating the same mistakes".to_string(),
                format!("The same position went wrong {t} in different games. Reviewing these moments once fixes them for good."),
                "Review your mistakes".to_string(),
            ),
            _ => (
                "Mistakes when the clock runs low".to_string(),
                format!("With under {LOW_TIME_SECS} seconds left you made {count} costly mistakes in {g}. Save time early with familiar openings and quick, safe moves."),
                "Train speed with Puzzle Rush".to_string(),
            ),
        },
        Lang::Es => match id {
            "hanging_pieces" => (
                "Dejar piezas sin protección".to_string(),
                format!("Dejaste una pieza que se podía capturar gratis {t} en {g}. Antes de cada jugada, pregúntate: «¿Sigue todo protegido?»"),
                "Practica detectar piezas colgadas".to_string(),
            ),
            "missed_tactics" => (
                format!("Tácticas que se te escapan: {th}"),
                format!("Tuviste una táctica ganadora ({th}) {t} en {g} y no la jugaste. Busca primero jaques, capturas y amenazas: muchas veces la táctica ya está ahí."),
                format!("Practica detectar {th}"),
            ),
            "endgame" => (
                "Tropiezos en el final".to_string(),
                format!("Con pocas piezas en el tablero cometiste {count} errores costosos en {g}. Los finales premian la técnica tranquila: activa tu rey y avanza los peones pasados."),
                "Entrena finales".to_string(),
            ),
            "opening" => (
                "Problemas en la apertura".to_string(),
                format!("Cometiste {count} errores costosos en las diez primeras jugadas de {g}. Un repertorio pequeño y conocido te lleva a un buen medio juego con seguridad."),
                "Construye tu repertorio".to_string(),
            ),
            "conversion" => (
                "Partidas ganadas que se escapan".to_string(),
                format!("Ibas claramente ganando (+3 o más) y no ganaste {t}. Cuando vas por delante, cambia piezas, cuida a tu rey y no des contrajuego."),
                "Practica ganar una posición ganada".to_string(),
            ),
            "repeated" => (
                "Repetir los mismos errores".to_string(),
                format!("La misma posición te salió mal {t} en partidas distintas. Repasar estos momentos una vez los corrige para siempre."),
                "Repasa tus errores".to_string(),
            ),
            _ => (
                "Errores con poco tiempo en el reloj".to_string(),
                format!("Con menos de {LOW_TIME_SECS} segundos cometiste {count} errores costosos en {g}. Ahorra tiempo al principio con aperturas conocidas y jugadas seguras."),
                "Entrena la velocidad con Puzzle Rush".to_string(),
            ),
        },
    };
    (a, b, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SideStats;

    fn mv(ply: usize, color: &str, fen: &str, uci: &str, best: &str, cls: Classification, loss: f32) -> MoveReview {
        MoveReview {
            ply,
            san: uci.into(),
            uci: uci.into(),
            color: color.into(),
            fen_before: fen.into(),
            best_move_uci: best.into(),
            best_move_san: best.into(),
            classification: cls,
            win_chance_loss: loss,
            ..Default::default()
        }
    }

    fn game(id: i64, user: Color, result: &str, moves: Vec<MoveReview>, evals: Vec<Score>) -> GameInput {
        GameInput {
            id,
            user,
            result: result.into(),
            date: format!("2026-01-{:02}T10:00:00Z", id.clamp(1, 28)),
            opening: Some("Italian Game: Giuoco Piano".into()),
            review: GameReview {
                moves,
                evals,
                white: SideStats { accuracy: 70.0 + id as f32, ..Default::default() },
                black: SideStats { accuracy: 60.0, ..Default::default() },
                ..Default::default()
            },
            clocks: Vec::new(),
        }
    }

    // White to move; Nc3 is attacked by nothing... white plays Qd1-h5?? where the queen can be
    // taken by the knight on f6.
    const HANG_FEN: &str = "rnbqkb1r/pppp1ppp/5n2/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 2 3";
    // White knight on e5 can fork king e8... use a classic: Nc7+ forks king and rook.
    const FORK_FEN: &str = "r3k3/8/8/1N6/8/8/8/4K3 w - - 0 40";

    #[test]
    fn phases() {
        let start = parse_fen(gm_engine::START_FEN).expect("fen");
        assert_eq!(phase_of(&start), Phase::Opening);
        let mid = parse_fen("r1bq1rk1/pp2bppp/2n1pn2/3p4/3P4/2NBPN2/PP3PPP/R2QK2R w KQ - 0 12").expect("fen");
        assert_eq!(phase_of(&mid), Phase::Middlegame);
        let end = parse_fen("8/5pk1/6p1/8/3R4/6P1/5PK1/3r4 w - - 0 45").expect("fen");
        assert_eq!(phase_of(&end), Phase::Endgame);
    }

    #[test]
    fn detects_hanging_queen() {
        let pos = parse_fen(HANG_FEN).expect("fen");
        let m = uci_to_move(&pos, "d1h5").expect("move");
        assert_eq!(hung_piece(&pos, &m), Some(Role::Queen));
        let ok = uci_to_move(&pos, "d2d3").expect("move");
        assert_eq!(hung_piece(&pos, &ok), None);
    }

    #[test]
    fn detects_missed_fork_and_mate() {
        let pos = parse_fen(FORK_FEN).expect("fen");
        let m = uci_to_move(&pos, "b5c7").expect("move");
        assert_eq!(tactic_theme(&pos, &m, Score::Cp(500)), Some("fork"));
        let br = parse_fen("6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 30").expect("fen");
        let mate = uci_to_move(&br, "a1a8").expect("move");
        assert_eq!(tactic_theme(&br, &mate, Score::Mate(1)), Some("backRankMate"));
    }

    #[test]
    fn empty_report_is_not_ready() {
        let r = aggregate(&[], Lang::En);
        assert!(!r.ready);
        assert!(r.weaknesses.is_empty());
        assert_eq!(r.phases.len(), 3);
        assert!(r.time_trouble.is_none());
    }

    #[test]
    fn aggregates_weaknesses_and_breakdowns() {
        let mut games = Vec::new();
        for id in 1..=4 {
            let moves = vec![
                mv(5, "white", HANG_FEN, "d1h5", "g1f3", Classification::Blunder, 40.0),
                mv(6, "black", HANG_FEN, "f6h5", "f6h5", Classification::Best, 0.0),
                mv(79, "white", FORK_FEN, "e1d2", "b5c7", Classification::Miss, 30.0),
            ];
            // Reached +4 then lost.
            let evals = vec![Score::Cp(20), Score::Cp(400), Score::Cp(-300), Score::Cp(-900)];
            games.push(game(id, Color::White, "0-1", moves, evals));
        }
        let r = aggregate(&games, Lang::En);
        assert!(r.ready);
        assert_eq!(r.games_analyzed, 4);
        let ids: Vec<&str> = r.weaknesses.iter().map(|w| w.id.as_str()).collect();
        assert!(ids.contains(&"hanging_pieces"), "{ids:?}");
        assert!(ids.contains(&"conversion"), "{ids:?}");
        assert!(r.weaknesses.len() <= 3);
        assert_eq!(r.hanging.iter().find(|p| p.piece == "queen").map(|p| p.count), Some(4));
        assert_eq!(r.tactics.first().map(|t| t.theme.as_str()), Some("fork"));
        assert_eq!(r.conversion.winning_games, 4);
        assert_eq!(r.conversion.converted, 0);
        assert!(!r.conversion.failed.is_empty());
        let opening = r.phases.iter().find(|p| p.phase == "opening").expect("phase");
        assert_eq!(opening.blunders, 4);
        let end = r.phases.iter().find(|p| p.phase == "endgame").expect("phase");
        assert_eq!(end.mistakes, 4);
        assert_eq!(r.by_color.get("white").map(|c| c.losses), Some(4));
        assert_eq!(r.by_opening.first().map(|o| o.name.as_str()), Some("Italian Game"));
        assert_eq!(r.accuracy_trend.len(), 4);
        // Oldest first.
        assert!(r.accuracy_trend[0].date < r.accuracy_trend[3].date);
        for w in &r.weaknesses {
            assert!(!w.examples.is_empty());
            assert!(w.examples.len() <= MAX_EXAMPLES);
            assert!(!w.title.is_empty() && !w.drill.href.is_empty());
        }
        // Repeated mistakes: same position in four games.
        let es = aggregate(&games, Lang::Es);
        assert!(es.weaknesses.iter().all(|w| !w.explanation.contains("times")));
    }

    #[test]
    fn black_side_and_draws() {
        let evals = vec![Score::Cp(0), Score::Cp(-500), Score::Cp(0)];
        let g = game(1, Color::Black, "1/2-1/2", vec![], evals.clone());
        // No moves: ignored entirely.
        assert_eq!(aggregate(&[g], Lang::En).games_analyzed, 0);
        let m = mv(2, "black", gm_engine::START_FEN, "e7e5", "e7e5", Classification::Best, 0.0);
        let g = game(2, Color::Black, "1/2-1/2", vec![m], evals);
        let r = aggregate(&[g], Lang::En);
        assert_eq!(r.conversion.winning_games, 1);
        assert_eq!(r.conversion.failed.first().map(|f| f.outcome.as_str()), Some("draw"));
    }

    #[test]
    fn clocks() {
        let pgn = "[Event \"x\"]\n[TimeControl \"300\"]\n\n1. e4 { [%clk 0:05:00] } 1... e5 { [%clk 0:04:58.5] } 2. Nf3 {[%clk 0:00:12]} Nc6 1-0";
        let c = parse_clocks(pgn, 100);
        assert_eq!(c, vec![Some(300), Some(298), Some(12), None]);
        assert!(parse_clocks("1. e4 e5 2. Nf3 *", 100).is_empty());
        assert_eq!(parse_hms("1:02:03"), Some(3723));
        assert_eq!(parse_hms("abc"), None);
    }

    #[test]
    fn uri_encoding() {
        assert_eq!(encode_uri_component("8/8 w - - 0 1"), "8%2F8%20w%20-%20-%200%201");
    }
}
