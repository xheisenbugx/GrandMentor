//! gm-analysis: game review — move classification, accuracy, key moments.
//!
//! Public API follows `docs/CONTRACT.md` §3. The review pipeline:
//!
//! 1. Replay and validate the moves (bad input is an `Err`, never a panic).
//! 2. Evaluate every distinct non-terminal position with MultiPV 2 at the requested depth,
//!    concurrently across the [`EnginePool`]. Transpositions/repetitions are searched once.
//!    Dropping the returned future stops all in-flight searches (no orphaned CPU work).
//! 3. Classify every move chess.com style (book, forced, best, brilliant, great, excellent,
//!    good, inaccuracy, mistake, miss, blunder) from the mover's expected-points (win%) loss.
//! 4. Lichess accuracy per side, an Elo estimate, key moments and a friendly summary.

mod accuracy;
mod see;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use shakmaty::{Chess, Color, Move, Position};

use gm_content::OpeningRef;
use gm_engine::{
    fen_key, move_to_san, parse_fen, to_fen, uci_line_to_san, uci_to_move, EnginePool, PvLine, Score, SearchInfo,
    SearchLimits,
};

pub use accuracy::{estimate_elo, game_accuracy, move_accuracy};

/// Longest game we accept for review (plies).
pub const MAX_PLIES: usize = 1200;
/// Safety cap per position so a pathological position can't stall a review.
const PER_POSITION_MOVETIME_MS: u64 = 5_000;
/// Only look up opening names/book status within this many plies.
const BOOK_MAX_PLY: usize = 40;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Brilliant,
    Great,
    Best,
    Excellent,
    #[default]
    Good,
    Book,
    Inaccuracy,
    Mistake,
    Miss,
    Blunder,
    Forced,
}

impl Classification {
    /// snake_case name as used on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Classification::Brilliant => "brilliant",
            Classification::Great => "great",
            Classification::Best => "best",
            Classification::Excellent => "excellent",
            Classification::Good => "good",
            Classification::Book => "book",
            Classification::Inaccuracy => "inaccuracy",
            Classification::Mistake => "mistake",
            Classification::Miss => "miss",
            Classification::Blunder => "blunder",
            Classification::Forced => "forced",
        }
    }

    /// All variants, in display order.
    pub const ALL: [Classification; 11] = [
        Classification::Brilliant,
        Classification::Great,
        Classification::Best,
        Classification::Excellent,
        Classification::Good,
        Classification::Book,
        Classification::Inaccuracy,
        Classification::Mistake,
        Classification::Miss,
        Classification::Blunder,
        Classification::Forced,
    ];

    /// Annotation symbol appended to SAN in summaries.
    fn suffix(self) -> &'static str {
        match self {
            Classification::Brilliant => "!!",
            Classification::Great => "!",
            Classification::Inaccuracy => "?!",
            Classification::Mistake | Classification::Miss => "?",
            Classification::Blunder => "??",
            _ => "",
        }
    }

    fn is_error(self) -> bool {
        matches!(
            self,
            Classification::Inaccuracy | Classification::Mistake | Classification::Miss | Classification::Blunder
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct MoveReview {
    /// 1-based
    pub ply: usize,
    pub san: String,
    pub uci: String,
    pub color: String,
    pub fen_before: String,
    pub fen_after: String,
    pub eval_before: Score,
    pub eval_after: Score,
    pub best_move_uci: String,
    pub best_move_san: String,
    pub best_line_san: Vec<String>,
    pub classification: Classification,
    pub win_chance_loss: f32,
    pub explanation: String,
    pub opening_name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SideStats {
    pub accuracy: f32,
    pub estimated_elo: u16,
    /// classification -> count
    pub counts: BTreeMap<String, u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct GameReview {
    pub start_fen: String,
    pub moves: Vec<MoveReview>,
    /// len = moves+1, evals[0] = start position
    pub evals: Vec<Score>,
    pub white: SideStats,
    pub black: SideStats,
    pub opening: Option<OpeningRef>,
    /// plies
    pub key_moments: Vec<usize>,
    pub summary: String,
}

fn win_from_cp(cp: f32) -> f32 {
    let cp = cp.clamp(-1000.0, 1000.0);
    50.0 + 50.0 * (2.0 / (1.0 + (-0.003_682_08 * cp).exp()) - 1.0)
}

/// Lichess win% formula, white POV, 0..100. Centipawns are clamped to ±1000; a forced mate
/// counts as ±1000. `Mate(0)` (a finished checkmate, see [`GameReview::evals`]) is 100 for
/// white, matching the frontend's convention that `mate >= 0` favours white.
pub fn win_percent(score: Score) -> f32 {
    match score {
        Score::Cp(c) => win_from_cp(c as f32),
        Score::Mate(0) => 100.0,
        Score::Mate(m) if m > 0 => win_from_cp(1000.0),
        Score::Mate(_) => win_from_cp(-1000.0),
    }
}

/// Mover-POV win% from white-POV win%.
fn for_mover(white_win: f32, mover: Color) -> f32 {
    match mover {
        Color::White => white_win,
        Color::Black => 100.0 - white_win,
    }
}

fn color_name(c: Color) -> &'static str {
    match c {
        Color::White => "white",
        Color::Black => "black",
    }
}

fn capitalized(c: Color) -> &'static str {
    match c {
        Color::White => "White",
        Color::Black => "Black",
    }
}

/// Sets the shared stop flag when the review future is dropped, so blocking searches that are
/// still running on the pool return promptly.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// What we know about one position of the game.
struct PosData {
    pos: Chess,
    fen: String,
    /// Eval, white POV (terminal positions are exact).
    score: Score,
    /// Exact white win% for terminal positions, else `win_percent(score)`.
    white_win: f32,
    /// Engine lines (white POV), best first; empty for terminal positions.
    lines: Vec<PvLine>,
}

/// Evaluate all positions concurrently across the pool, deduplicating identical positions.
async fn evaluate_positions(
    pool: &EnginePool,
    positions: &[Chess],
    depth: u8,
    progress: Option<&tokio::sync::mpsc::UnboundedSender<f32>>,
) -> Result<Vec<Option<SearchInfo>>, String> {
    // Map each non-terminal position to a unique search job.
    let mut job_of: Vec<Option<usize>> = Vec::with_capacity(positions.len());
    let mut jobs: Vec<Chess> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in positions {
        if p.is_game_over() {
            job_of.push(None);
            continue;
        }
        let key = fen_key(&to_fen(p));
        let idx = *seen.entry(key).or_insert_with(|| {
            jobs.push(p.clone());
            jobs.len() - 1
        });
        job_of.push(Some(idx));
    }
    drop(seen);

    let stop = Arc::new(AtomicBool::new(false));
    let _guard = StopOnDrop(Arc::clone(&stop));
    let limits = SearchLimits {
        depth: Some(depth),
        movetime_ms: Some(PER_POSITION_MOVETIME_MS),
        nodes: None,
        multipv: 2,
    };
    let total = jobs.len().max(1);
    let mut results: Vec<Option<SearchInfo>> = vec![None; jobs.len()];
    // JoinSet aborts outstanding tasks when dropped (e.g. client went away).
    let mut set = tokio::task::JoinSet::new();
    for (idx, pos) in jobs.into_iter().enumerate() {
        let pool = pool.clone();
        let stop = Arc::clone(&stop);
        let limits = limits.clone();
        set.spawn(async move {
            let info = pool
                .with_engine(move |engine| {
                    if stop.load(Ordering::Relaxed) {
                        return SearchInfo::default();
                    }
                    engine.search(&pos, &limits, &stop, &mut |_| {})
                })
                .await;
            (idx, info)
        });
    }
    let mut done = 0usize;
    while let Some(joined) = set.join_next().await {
        let (idx, info) = joined.map_err(|e| {
            if e.is_panic() {
                "engine failed while analysing the game".to_string()
            } else {
                "analysis was cancelled".to_string()
            }
        })?;
        if let Some(slot) = results.get_mut(idx) {
            *slot = Some(info);
        }
        done += 1;
        if let Some(tx) = progress {
            // Leave the last few percent for classification / summary.
            let _ = tx.send(0.97 * done as f32 / total as f32);
        }
    }
    Ok(job_of.into_iter().map(|j| j.and_then(|i| results.get(i).cloned().flatten())).collect())
}

/// Wire score for a finished position.
fn terminal_score(pos: &Chess) -> (Score, f32) {
    if pos.is_checkmate() {
        // Side to move is mated. `Mate(0)` reads as "white has mated" in the UI (mate >= 0
        // favours white); for a black win we report -M1 so the sign is unambiguous.
        match pos.turn() {
            Color::Black => (Score::Mate(0), 100.0),
            Color::White => (Score::Mate(-1), 0.0),
        }
    } else {
        (Score::Cp(0), 50.0)
    }
}

/// Facts about a single move needed for classification.
struct MoveFacts<'a> {
    played_uci: &'a str,
    mv: &'a Move,
    before: &'a PosData,
    after: &'a PosData,
    mover: Color,
    legal_count: usize,
    is_book: bool,
    /// Mover-POV win% before and after.
    win_before: f32,
    win_after: f32,
    loss: f32,
    /// Win% the opponent's previous move threw away (0 if none).
    prev_opponent_loss: f32,
    /// Square the opponent's previous move landed on, if it was a capture.
    prev_capture_square: Option<shakmaty::Square>,
}

fn classify(f: &MoveFacts<'_>) -> Classification {
    if f.legal_count <= 1 {
        return Classification::Forced;
    }
    if f.is_book {
        return Classification::Book;
    }
    let best = f.before.lines.first();
    let is_best = best.and_then(|l| l.moves.first()).map(String::as_str) == Some(f.played_uci)
        || (f.after.pos.is_checkmate());
    let loss = f.loss;

    let mut cls = if is_best {
        Classification::Best
    } else if loss <= 2.0 {
        Classification::Excellent
    } else if loss <= 5.0 {
        Classification::Good
    } else if loss <= 10.0 {
        Classification::Inaccuracy
    } else if loss <= 20.0 {
        Classification::Mistake
    } else {
        Classification::Blunder
    };

    // Brilliant: a (near-)best piece sacrifice that keeps a good position, when not already
    // completely winning.
    if (is_best || loss <= 2.0) && is_sacrifice(f) && f.win_after >= 50.0 && f.win_before < 92.0 {
        return Classification::Brilliant;
    }

    // Great: the only good move — the second-best option is much worse.
    if is_best {
        if let Some(second) = f.before.lines.get(1) {
            let second_win = for_mover(win_percent(second.score), f.mover);
            let best_win = f.win_before.max(f.win_after);
            let is_recapture = f.prev_capture_square.is_some() && f.prev_capture_square == Some(f.mv.to());
            if best_win - second_win >= 15.0 && second_win < 75.0 && !is_recapture && best_win >= 40.0 {
                cls = Classification::Great;
            }
        }
        return cls;
    }

    // Miss: failed to punish — the opponent just erred (or we had a forced mate) and we let the
    // advantage slip without actually ending up worse.
    if loss > 10.0 {
        let had_mate = matches!(f.before.score.for_side(f.mover), Score::Mate(m) if m > 0);
        let opportunity = f.win_before >= 65.0 && (f.prev_opponent_loss >= 10.0 || had_mate);
        if opportunity && f.win_after >= 40.0 {
            cls = Classification::Miss;
        }
    }
    cls
}

/// True if the move leaves material that the opponent can win by SEE (net of anything we
/// just captured), i.e. we are offering at least a minor piece / the exchange.
fn is_sacrifice(f: &MoveFacts<'_>) -> bool {
    if f.after.pos.is_game_over() {
        return false;
    }
    let offered = see::max_capture_gain(&f.after.pos);
    if offered < 200 {
        return false;
    }
    // Was the same material already hanging before our move? Then it's not a fresh sacrifice
    // unless we increased what is offered.
    let gained = see::immediate_gain(f.mv);
    offered - gained >= 200
}

fn move_label(m: &MoveReview, start_fullmove: u32, white_first: bool) -> String {
    let offset = if white_first { 0 } else { 1 };
    let n = start_fullmove as usize + (m.ply - 1 + offset) / 2;
    if m.color == "white" {
        format!("{n}. {}{}", m.san, m.classification.suffix())
    } else {
        format!("{n}... {}{}", m.san, m.classification.suffix())
    }
}

fn build_summary(
    review: &GameReview,
    final_pos: &Chess,
    start_fullmove: u32,
    white_first: bool,
) -> String {
    let mut sentences: Vec<String> = Vec::with_capacity(3);
    let label = |m: &MoveReview| move_label(m, start_fullmove, white_first);

    // 1. Opening + turning point.
    let opening = review.opening.as_ref().map(|o| o.name.clone());
    let worst = review
        .moves
        .iter()
        .filter(|m| m.classification.is_error() && m.win_chance_loss >= 10.0)
        .max_by(|a, b| a.win_chance_loss.total_cmp(&b.win_chance_loss));
    let opener = match &opening {
        Some(name) => format!("After a {name} opening, "),
        None => String::new(),
    };
    match worst {
        Some(m) => {
            let who = if m.color == "white" { "White" } else { "Black" };
            let verb = if m.classification == Classification::Miss { "let a big chance slip with" } else { "went wrong with" };
            let better = if m.best_move_san.is_empty() {
                String::new()
            } else {
                format!(" — {} was the move", m.best_move_san)
            };
            let s = format!("{opener}the game turned when {who} {verb} {}{better}.", label(m));
            sentences.push(capitalize_first(&s));
        }
        None if !review.moves.is_empty() => {
            sentences.push(capitalize_first(&format!(
                "{opener}both sides played a clean game without any serious mistakes."
            )));
        }
        None => sentences.push("No moves were played yet — make some moves and review again!".to_string()),
    }

    // 2. Highlight (brilliant/great) or the ending.
    if let Some(m) = review.moves.iter().find(|m| m.classification == Classification::Brilliant) {
        let who = if m.color == "white" { "White" } else { "Black" };
        sentences.push(format!("Don't miss {who}'s brilliant {} — a real sacrifice that works!", label(m)));
    } else if final_pos.is_checkmate() {
        let winner = capitalized(final_pos.turn().other());
        if let Some(last) = review.moves.last() {
            sentences.push(format!("{winner} finished it in style with checkmate on {}.", label(last)));
        }
    } else if final_pos.is_stalemate() {
        sentences.push("The game ended in stalemate — always check your opponent has a move!".to_string());
    } else if let Some(m) = review.moves.iter().find(|m| m.classification == Classification::Great) {
        let who = if m.color == "white" { "White" } else { "Black" };
        sentences.push(format!("{who} found a great move with {}.", label(m)));
    }

    // 3. Accuracy comparison.
    if !review.moves.is_empty() {
        let (w, b) = (review.white.accuracy, review.black.accuracy);
        let verdict = if (w - b).abs() < 3.0 {
            "an evenly matched performance".to_string()
        } else if w > b {
            "White was the more precise side".to_string()
        } else {
            "Black was the more precise side".to_string()
        };
        sentences.push(format!("Accuracy: White {w:.1}%, Black {b:.1}% — {verdict}."));
    }
    sentences.join(" ")
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Pick the most instructive plies: big swings, blunders, misses and brilliancies.
fn key_moments(moves: &[MoveReview], white_wins: &[f32]) -> Vec<usize> {
    let mut scored: Vec<(f32, usize)> = moves
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let swing = match (white_wins.get(i), white_wins.get(i + 1)) {
                (Some(a), Some(b)) => (b - a).abs(),
                _ => 0.0,
            };
            let bonus = match m.classification {
                Classification::Brilliant => 40.0,
                Classification::Great => 20.0,
                Classification::Blunder => 15.0,
                Classification::Miss => 12.0,
                Classification::Mistake => 8.0,
                _ => 0.0,
            };
            let notable = bonus > 0.0 || swing >= 15.0;
            notable.then_some((swing + bonus, m.ply))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(6);
    let mut plies: Vec<usize> = scored.into_iter().map(|(_, p)| p).collect();
    plies.sort_unstable();
    plies
}

/// Full game review. See module docs.
pub async fn review_game(
    pool: &EnginePool,
    content: Arc<gm_content::Content>,
    start_fen: &str,
    moves: &[String],
    depth: u8,
    progress: Option<tokio::sync::mpsc::UnboundedSender<f32>>,
) -> Result<GameReview, String> {
    if moves.len() > MAX_PLIES {
        return Err(format!("game too long to review ({} plies, max {MAX_PLIES})", moves.len()));
    }
    let start_fen = if start_fen.trim().is_empty() || start_fen.trim() == "start" {
        gm_engine::START_FEN
    } else {
        start_fen.trim()
    };
    let start = parse_fen(start_fen)?;
    let white_first = start.turn() == Color::White;
    let start_fullmove = start.fullmoves().get();

    // 1. Replay & validate.
    let mut positions: Vec<Chess> = Vec::with_capacity(moves.len() + 1);
    let mut played: Vec<Move> = Vec::with_capacity(moves.len());
    positions.push(start.clone());
    let mut pos = start.clone();
    for (i, u) in moves.iter().enumerate() {
        let m = uci_to_move(&pos, u.trim()).map_err(|e| format!("illegal move at ply {}: {e}", i + 1))?;
        pos.play_unchecked(&m);
        played.push(m);
        positions.push(pos.clone());
    }
    let depth = depth.clamp(1, 30);

    // 2. Evaluate.
    let infos = evaluate_positions(pool, &positions, depth, progress.as_ref()).await?;
    let data: Vec<PosData> = positions
        .into_iter()
        .zip(infos)
        .map(|(p, info)| {
            let fen = to_fen(&p);
            let (score, white_win, lines) = if p.is_game_over() {
                let (s, w) = terminal_score(&p);
                (s, w, Vec::new())
            } else {
                let mut lines = info.map(|i| i.lines).unwrap_or_default();
                for l in &mut lines {
                    if l.san.len() != l.moves.len() {
                        l.san = uci_line_to_san(&p, &l.moves);
                    }
                }
                let s = lines.first().map(|l| l.score).unwrap_or_default();
                (s, win_percent(s), lines)
            };
            PosData { pos: p, fen, score, white_win, lines }
        })
        .collect();

    // 3. Classify.
    let mut reviews: Vec<MoveReview> = Vec::with_capacity(played.len());
    let mut losses: Vec<f32> = Vec::with_capacity(played.len());
    let mut move_acc: Vec<f32> = Vec::with_capacity(played.len());
    let mut book_misses = 0usize;
    let mut opening: Option<OpeningRef> = None;
    for (i, m) in played.iter().enumerate() {
        let (before, after) = (&data[i], &data[i + 1]);
        let mover = before.pos.turn();
        let uci = moves[i].trim();
        let win_before = for_mover(before.white_win, mover);
        let win_after = for_mover(after.white_win, mover);
        let best_uci = before.lines.first().and_then(|l| l.moves.first()).cloned().unwrap_or_default();
        // Playing the engine's top move never "loses" (search noise between plies).
        // Playing the engine's top move never "loses" (search noise between plies). If the
        // played move is one of the MultiPV lines, compare scores from the same search, which
        // avoids odd/even-depth noise between consecutive positions.
        let loss = if best_uci == uci || after.pos.is_checkmate() {
            0.0
        } else if let Some(line) = before.lines.iter().skip(1).find(|l| l.moves.first().map(String::as_str) == Some(uci)) {
            (win_before - for_mover(win_percent(line.score), mover)).max(0.0)
        } else {
            (win_before - win_after).max(0.0)
        };

        let mut opening_name = None;
        let mut is_book = false;
        if i < BOOK_MAX_PLY && book_misses < 4 {
            is_book = content.is_book_position(&after.fen);
            if is_book {
                book_misses = 0;
                if let Some(om) = content.lookup_opening(&after.fen) {
                    opening_name = Some(om.opening.name.clone());
                    opening = Some(om.opening);
                }
            } else {
                book_misses += 1;
            }
        }
        // A "book" move that is actually a serious error isn't book (bad data / traps).
        if is_book && loss > 10.0 {
            is_book = false;
        }

        let prev_capture_square = if i > 0 && played[i - 1].is_capture() { Some(played[i - 1].to()) } else { None };
        let facts = MoveFacts {
            played_uci: uci,
            mv: m,
            before,
            after,
            mover,
            legal_count: before.pos.legal_moves().len(),
            is_book,
            win_before,
            win_after,
            loss,
            prev_opponent_loss: if i > 0 { losses[i - 1] } else { 0.0 },
            prev_capture_square,
        };
        let classification = classify(&facts);
        losses.push(loss);
        move_acc.push(if loss <= 0.0 { 100.0 } else { move_accuracy(win_before, win_before - loss) });

        let san = move_to_san(&before.pos, m);
        let best = before.lines.first();
        let best_move_san = best.and_then(|l| l.san.first()).cloned().unwrap_or_default();
        let best_line_san: Vec<String> = best.map(|l| l.san.iter().take(10).cloned().collect()).unwrap_or_default();
        let explanation = gm_mentor::explain_move(&gm_mentor::MoveContext {
            fen_before: before.fen.clone(),
            played_uci: uci.to_string(),
            played_san: san.clone(),
            best_uci: best_uci.clone(),
            best_san: best_move_san.clone(),
            best_line_san: best_line_san.clone(),
            eval_before: before.score,
            eval_after: after.score,
            classification: classification.as_str().to_string(),
        });
        reviews.push(MoveReview {
            ply: i + 1,
            san,
            uci: uci.to_string(),
            color: color_name(mover).to_string(),
            fen_before: before.fen.clone(),
            fen_after: after.fen.clone(),
            eval_before: before.score,
            eval_after: after.score,
            best_move_uci: best_uci,
            best_move_san,
            best_line_san,
            classification,
            win_chance_loss: (loss * 100.0).round() / 100.0,
            explanation,
            opening_name,
        });
    }

    // 4. Stats.
    let white_wins: Vec<f32> = data.iter().map(|d| d.white_win).collect();
    let (acc_w, acc_b) = game_accuracy(&white_wins, &move_acc, white_first);
    let side_stats = |color: &str, accuracy: f32| {
        let mut counts: BTreeMap<String, u32> = Classification::ALL.iter().map(|c| (c.as_str().to_string(), 0)).collect();
        let mut n = 0usize;
        for r in reviews.iter().filter(|r| r.color == color) {
            *counts.entry(r.classification.as_str().to_string()).or_insert(0) += 1;
            n += 1;
        }
        let accuracy = (accuracy * 10.0).round() / 10.0;
        SideStats { accuracy, estimated_elo: estimate_elo(accuracy, n), counts }
    };
    let white = side_stats("white", acc_w);
    let black = side_stats("black", acc_b);
    let key = key_moments(&reviews, &white_wins);
    let final_pos = data.last().map(|d| d.pos.clone()).unwrap_or_default();

    let mut review = GameReview {
        start_fen: to_fen(&start),
        evals: data.iter().map(|d| d.score).collect(),
        moves: reviews,
        white,
        black,
        opening,
        key_moments: key,
        summary: String::new(),
    };
    review.summary = build_summary(&review, &final_pos, start_fullmove, white_first);
    if let Some(tx) = &progress {
        let _ = tx.send(1.0);
    }
    Ok(review)
}

#[cfg(test)]
mod tests;
