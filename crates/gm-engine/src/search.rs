//! Search: iterative deepening + aspiration windows + PVS alpha-beta with a transposition table,
//! null-move pruning, late move reductions/pruning, (reverse) futility pruning, razoring,
//! killer + history move ordering, SEE-based capture ordering and pruning, check extensions,
//! quiescence search, draw detection and MultiPV.
//!
//! Scores inside the search are side-to-move relative; they are converted to white POV at the
//! API boundary (`SearchInfo`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use shakmaty::{Chess, Move, Position, Role};

use crate::eval::evaluate;
use crate::moves::{encode, full_hash, hash_after, hash_null, mvv_lva, see_ge, value};
use crate::tt::{Tt, BOUND_EXACT, BOUND_LOWER, BOUND_UPPER};
use crate::{move_to_san, move_to_uci, PvLine, Score, SearchInfo, SearchLimits};

pub const MAX_PLY: usize = 128;
const INF: i32 = 32_000;
const MATE: i32 = 31_000;
/// Scores with absolute value >= MATE_BOUND are mate scores.
const MATE_BOUND: i32 = MATE - 2 * MAX_PLY as i32;
const MAX_DEPTH: i32 = 100;
const CHECK_INTERVAL: u64 = 2048;
const MAX_MULTIPV: usize = 64;
/// Cap on externally supplied game history (only the last 100 plies can matter).
const MAX_GAME_HISTORY: usize = 256;
const HISTORY_MAX: i32 = 16_384;

type PvTable = [[u16; MAX_PLY + 1]; MAX_PLY + 1];

/// Long-lived per-engine tables (allocated once, reused across searches).
struct Tables {
    tt: Tt,
    /// history[color][from|to<<6]
    history: Box<[[i32; 4096]; 2]>,
    killers: Box<[[u16; 2]; MAX_PLY + 2]>,
    pv: Box<PvTable>,
    pv_len: Box<[usize; MAX_PLY + 2]>,
    evals: Box<[i32; MAX_PLY + 2]>,
    /// Hashes of positions on the path (game history + search stack), parents only.
    path: Vec<u64>,
    lmr: Box<[[i32; 64]; 64]>,
}

impl Tables {
    fn new(tt_mb: usize) -> Tables {
        let mut lmr = Box::new([[0i32; 64]; 64]);
        for (d, row) in lmr.iter_mut().enumerate().skip(1) {
            for (m, r) in row.iter_mut().enumerate().skip(1) {
                *r = (0.8 + (d as f64).ln() * (m as f64).ln() / 2.4) as i32;
            }
        }
        Tables {
            tt: Tt::new(tt_mb),
            history: Box::new([[0; 4096]; 2]),
            killers: Box::new([[0; 2]; MAX_PLY + 2]),
            pv: Box::new([[0; MAX_PLY + 1]; MAX_PLY + 1]),
            pv_len: Box::new([0; MAX_PLY + 2]),
            evals: Box::new([0; MAX_PLY + 2]),
            path: Vec::with_capacity(MAX_GAME_HISTORY + MAX_PLY + 8),
            lmr,
        }
    }

    fn clear(&mut self) {
        self.tt.clear();
        self.history.iter_mut().for_each(|h| h.fill(0));
        self.killers.iter_mut().for_each(|k| *k = [0; 2]);
    }
}

/// A single-threaded search engine. Owns a fixed-size transposition table.
pub struct Engine {
    tables: Tables,
    tt_mb: usize,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("tt_mb", &self.tt_mb)
            .finish()
    }
}

impl Engine {
    /// Create an engine with a transposition table of `tt_mb` MiB (clamped to 1..=4096,
    /// rounded down to a power of two).
    pub fn new(tt_mb: usize) -> Self {
        let tt_mb = tt_mb.clamp(1, 4096);
        Engine {
            tables: Tables::new(tt_mb),
            tt_mb,
        }
    }

    /// Clear TT / history tables.
    pub fn new_game(&mut self) {
        self.tables.clear();
    }

    /// Transposition table size in bytes.
    pub fn tt_bytes(&self) -> usize {
        self.tables.tt.size_bytes()
    }

    /// Search `pos`. Calls `on_info` after every completed iteration; returns the last one.
    /// If the position has no legal moves the result has `depth == 0` and no lines.
    pub fn search(
        &mut self,
        pos: &Chess,
        limits: &SearchLimits,
        stop: &AtomicBool,
        on_info: &mut dyn FnMut(&SearchInfo),
    ) -> SearchInfo {
        self.search_with_history(pos, &[], limits, stop, on_info)
    }

    /// Like [`Engine::search`], but with the game's earlier positions (oldest first, excluding
    /// `pos` itself) so that repetitions of positions played before the root are detected.
    pub fn search_with_history(
        &mut self,
        pos: &Chess,
        history: &[Chess],
        limits: &SearchLimits,
        stop: &AtomicBool,
        on_info: &mut dyn FnMut(&SearchInfo),
    ) -> SearchInfo {
        let start = Instant::now();
        let t = &mut self.tables;
        t.tt.new_search();
        for side in t.history.iter_mut() {
            side.iter_mut().for_each(|h| *h /= 2);
        }
        t.killers.iter_mut().for_each(|k| *k = [0; 2]);
        t.path.clear();
        let skip = history.len().saturating_sub(MAX_GAME_HISTORY);
        t.path.extend(history[skip..].iter().map(full_hash));

        let root_moves = pos.legal_moves();
        if root_moves.is_empty() {
            return SearchInfo::default();
        }

        let mut s = Searcher {
            t,
            stop,
            start,
            deadline: limits
                .movetime_ms
                .map(|ms| start + Duration::from_millis(ms)),
            node_limit: limits.nodes,
            nodes: 0,
            next_check: CHECK_INTERVAL,
            stopped: false,
            can_stop: false,
            seldepth: 0,
            pv_idx: 0,
            excluded: Vec::with_capacity(MAX_MULTIPV),
            null_floor: 0,
        };
        s.next_check = s.next_check_from(0);
        s.iterate(pos, &root_moves, limits, on_info)
    }
}

struct Searcher<'a> {
    t: &'a mut Tables,
    stop: &'a AtomicBool,
    start: Instant,
    deadline: Option<Instant>,
    node_limit: Option<u64>,
    nodes: u64,
    next_check: u64,
    stopped: bool,
    /// The first iteration always completes so a move is always returned.
    can_stop: bool,
    seldepth: usize,
    pv_idx: usize,
    /// Root moves already reported in earlier MultiPV lines of this iteration.
    excluded: Vec<u16>,
    /// Index into `path` below which repetition checks must not look (null move boundary).
    null_floor: usize,
}

#[inline]
fn score_to_tt(s: i32, ply: usize) -> i32 {
    if s >= MATE_BOUND {
        s + ply as i32
    } else if s <= -MATE_BOUND {
        s - ply as i32
    } else {
        s
    }
}

#[inline]
fn score_from_tt(s: i32, ply: usize) -> i32 {
    if s >= MATE_BOUND {
        s - ply as i32
    } else if s <= -MATE_BOUND {
        s + ply as i32
    } else {
        s
    }
}

/// Internal side-to-move score -> API `Score` (still side-to-move POV).
fn stm_score(s: i32) -> Score {
    if s.abs() >= MATE_BOUND {
        let plies = MATE - s.abs();
        let moves = (plies + 1) / 2;
        Score::Mate(if s > 0 { moves } else { -moves })
    } else {
        Score::Cp(s)
    }
}

#[inline]
fn has_non_pawn_material(pos: &Chess) -> bool {
    let b = pos.board();
    let us = b.by_color(pos.turn());
    (us & !(b.pawns() | b.kings())).any()
}

#[inline]
fn hist_index(code: u16) -> usize {
    (code & 0x0FFF) as usize
}

#[inline]
fn color_index(pos: &Chess) -> usize {
    match pos.turn() {
        shakmaty::Color::White => 0,
        shakmaty::Color::Black => 1,
    }
}

#[inline]
fn history_update(h: &mut i32, bonus: i32) {
    *h += bonus - *h * bonus.abs() / HISTORY_MAX;
}

/// A pawn move to the 6th or 7th rank (relative): dangerous, never pruned or reduced.
#[inline]
fn is_advanced_pawn_push(pos: &Chess, m: &Move) -> bool {
    if m.role() != Role::Pawn || m.is_promotion() {
        return false;
    }
    let rank = u32::from(m.to().rank());
    match pos.turn() {
        shakmaty::Color::White => rank >= 5,
        shakmaty::Color::Black => rank <= 2,
    }
}

/// Find the legal move with this code.
fn decode(pos: &Chess, code: u16) -> Option<Move> {
    if code == 0 {
        return None;
    }
    pos.legal_moves().into_iter().find(|m| encode(m) == code)
}

impl Searcher<'_> {
    fn next_check_from(&self, nodes: u64) -> u64 {
        let mut n = nodes + CHECK_INTERVAL;
        if let Some(l) = self.node_limit {
            if l > nodes {
                n = n.min(l);
            }
        }
        n
    }

    #[inline]
    fn check_stop(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.nodes >= self.next_check {
            self.next_check = self.next_check_from(self.nodes);
            if self.can_stop {
                let out_of_nodes = self.node_limit.is_some_and(|l| self.nodes >= l);
                let out_of_time = self.deadline.is_some_and(|d| Instant::now() >= d);
                if out_of_nodes || out_of_time || self.stop.load(Ordering::Relaxed) {
                    self.stopped = true;
                }
            }
        }
        self.stopped
    }

    fn is_repetition(&self, hash: u64, halfmoves: u32) -> bool {
        let path = &self.t.path;
        let n = path.len();
        let lim = (halfmoves as usize).min(n.saturating_sub(self.null_floor));
        let mut i = 2;
        while i <= lim {
            if path[n - i] == hash {
                return true;
            }
            i += 2;
        }
        false
    }

    fn update_pv(&mut self, ply: usize, code: u16) {
        let child_len = self.t.pv_len[ply + 1].min(MAX_PLY - 1 - ply.min(MAX_PLY - 1));
        let (head, tail) = self.t.pv.split_at_mut(ply + 1);
        let row = &mut head[ply];
        row[0] = code;
        if let Some(child) = tail.first() {
            let len = child_len.min(row.len() - 1).min(child.len());
            row[1..=len].copy_from_slice(&child[..len]);
            self.t.pv_len[ply] = len + 1;
        } else {
            self.t.pv_len[ply] = 1;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Iterative deepening driver
    // -----------------------------------------------------------------------------------------

    fn iterate(
        &mut self,
        pos: &Chess,
        root_moves: &shakmaty::MoveList,
        limits: &SearchLimits,
        on_info: &mut dyn FnMut(&SearchInfo),
    ) -> SearchInfo {
        let multipv = limits.multipv.clamp(1, MAX_MULTIPV).min(root_moves.len());
        let max_depth = limits
            .depth
            .map(|d| (d as i32).clamp(1, MAX_DEPTH))
            .unwrap_or(MAX_DEPTH);
        let root_hash = full_hash(pos);
        let soft_deadline = limits
            .movetime_ms
            .map(|ms| self.start + Duration::from_millis(ms.saturating_mul(55) / 100));

        let mut result = SearchInfo::default();
        let mut prev_scores: Vec<i32> = Vec::with_capacity(multipv);

        for depth in 1..=max_depth {
            let mut lines: Vec<(i32, Vec<u16>)> = Vec::with_capacity(multipv);
            self.excluded.clear();
            self.seldepth = 0;
            for pv_idx in 0..multipv {
                self.pv_idx = pv_idx;
                let prev = prev_scores.get(pv_idx).copied();
                let mut delta = 18;
                let (mut alpha, mut beta) = match prev {
                    Some(p) if depth >= 5 && p.abs() < MATE_BOUND => {
                        ((p - delta).max(-INF), (p + delta).min(INF))
                    }
                    _ => (-INF, INF),
                };
                let score = loop {
                    let sc = self.negamax(pos, root_hash, depth, alpha, beta, 0, false);
                    if self.stopped {
                        break sc;
                    }
                    if sc <= alpha && alpha > -INF {
                        beta = (alpha + beta) / 2;
                        alpha = (sc - delta).max(-INF);
                    } else if sc >= beta && beta < INF {
                        beta = (sc + delta).min(INF);
                    } else {
                        break sc;
                    }
                    delta += delta / 2 + 8;
                    if delta > 600 {
                        alpha = -INF;
                        beta = INF;
                    }
                };
                if self.stopped {
                    break;
                }
                let len = self.t.pv_len[0];
                let mut pv: Vec<u16> = self.t.pv[0][..len].to_vec();
                if pv.is_empty() {
                    // Should not happen with a full window; fall back to any non-excluded move.
                    if let Some(m) = root_moves
                        .iter()
                        .find(|m| !self.excluded.contains(&encode(m)))
                    {
                        pv.push(encode(m));
                    } else {
                        break;
                    }
                }
                self.excluded.push(pv[0]);
                lines.push((score, pv));
            }
            if self.stopped && depth > 1 {
                break;
            }
            // Stable sort: best first.
            lines.sort_by_key(|l| std::cmp::Reverse(l.0));
            prev_scores.clear();
            prev_scores.extend(lines.iter().map(|l| l.0));

            let elapsed = self.start.elapsed();
            let ms = elapsed.as_millis() as u64;
            let info = SearchInfo {
                depth: depth.clamp(0, 255) as u8,
                seldepth: self.seldepth.max(depth as usize).min(255) as u8,
                nodes: self.nodes,
                nps: self
                    .nodes
                    .saturating_mul(1000)
                    .checked_div(ms)
                    .unwrap_or(self.nodes.saturating_mul(1000)),
                time_ms: ms,
                lines: lines
                    .iter()
                    .map(|(sc, pv)| self.build_line(pos, *sc, pv, depth as usize))
                    .collect(),
            };
            on_info(&info);
            self.can_stop = true;
            let best = lines.first().map(|l| l.0).unwrap_or(0);
            result = info;

            // Termination conditions between iterations.
            if self.stopped || self.stop.load(Ordering::Relaxed) {
                break;
            }
            if self.node_limit.is_some_and(|l| self.nodes >= l) {
                break;
            }
            let now = Instant::now();
            if soft_deadline.is_some_and(|d| now >= d) || self.deadline.is_some_and(|d| now >= d) {
                break;
            }
            // A forced mate has been found and fully proven: deeper search adds nothing.
            if multipv == 1 && best.abs() >= MATE_BOUND && depth > (MATE - best.abs()) + 4 {
                break;
            }
            // Only one legal move with a time limit: no need to think.
            if root_moves.len() == 1 && limits.movetime_ms.is_some() && depth >= 6 {
                break;
            }
        }
        result
    }

    /// Convert an internal PV (codes) into a `PvLine`, extending it from the TT if short.
    fn build_line(&self, root: &Chess, score: i32, pv: &[u16], depth: usize) -> PvLine {
        let mut pos = root.clone();
        let mut hash = full_hash(root);
        let mut seen: Vec<u64> = Vec::with_capacity(depth + 2);
        seen.push(hash);
        let mut moves = Vec::with_capacity(depth.max(pv.len()));
        let mut san = Vec::with_capacity(depth.max(pv.len()));
        let mut play = |pos: &mut Chess, hash: &mut u64, m: &Move| {
            moves.push(move_to_uci(m));
            san.push(move_to_san(pos, m));
            let before = pos.clone();
            pos.play_unchecked(m);
            *hash = hash_after(*hash, &before, m, pos);
        };
        for &code in pv {
            match decode(&pos, code) {
                Some(m) => {
                    play(&mut pos, &mut hash, &m);
                    seen.push(hash);
                }
                None => break,
            }
        }
        // Extend from the transposition table (bounded, no cycles).
        let want = depth.min(MAX_PLY);
        let mut count = pv.len();
        while count < want {
            let Some(e) = self.t.tt.probe(hash) else {
                break;
            };
            let Some(m) = decode(&pos, e.mv) else { break };
            play(&mut pos, &mut hash, &m);
            if seen.contains(&hash) {
                break;
            }
            seen.push(hash);
            count += 1;
        }
        PvLine {
            score: stm_score(score).to_white_pov(root.turn()),
            moves,
            san,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Alpha-beta
    // -----------------------------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn negamax(
        &mut self,
        pos: &Chess,
        hash: u64,
        mut depth: i32,
        mut alpha: i32,
        mut beta: i32,
        ply: usize,
        allow_null: bool,
    ) -> i32 {
        let pv_node = beta - alpha > 1;
        let root = ply == 0;
        self.t.pv_len[ply] = 0;
        if self.check_stop() {
            return 0;
        }
        let in_check = pos.is_check();

        if !root {
            if pos.halfmoves() >= 100 && (!in_check || !pos.legal_moves().is_empty()) {
                return 0;
            }
            if self.is_repetition(hash, pos.halfmoves()) || pos.is_insufficient_material() {
                return 0;
            }
            if ply >= MAX_PLY - 2 {
                return if in_check { 0 } else { evaluate(pos) };
            }
            // Mate distance pruning
            alpha = alpha.max(-MATE + ply as i32);
            beta = beta.min(MATE - ply as i32 - 1);
            if alpha >= beta {
                return alpha;
            }
        }

        if in_check {
            depth += 1;
        }
        if depth <= 0 {
            return self.qsearch(pos, hash, alpha, beta, ply);
        }
        self.nodes += 1;
        self.seldepth = self.seldepth.max(ply);

        // Transposition table
        let tt = self.t.tt.probe(hash);
        let mut tt_move = 0u16;
        if let Some(e) = tt {
            tt_move = e.mv;
            if !pv_node && e.depth as i32 >= depth {
                let s = score_from_tt(e.score as i32, ply);
                let b = e.bound();
                if b == BOUND_EXACT
                    || (b == BOUND_LOWER && s >= beta)
                    || (b == BOUND_UPPER && s <= alpha)
                {
                    return s;
                }
            }
        }

        // Static evaluation
        let static_eval = if in_check {
            -INF
        } else if let Some(e) = tt {
            e.eval as i32
        } else {
            evaluate(pos)
        };
        let mut eval = static_eval;
        if let (Some(e), false) = (tt, in_check) {
            let s = score_from_tt(e.score as i32, ply);
            let b = e.bound();
            if s.abs() < MATE_BOUND
                && (b == BOUND_EXACT
                    || (b == BOUND_LOWER && s > eval)
                    || (b == BOUND_UPPER && s < eval))
            {
                eval = s;
            }
        }
        self.t.evals[ply] = static_eval;
        let improving = !in_check && ply >= 2 && static_eval > self.t.evals[ply - 2];
        self.t.killers[ply + 1] = [0; 2];

        if !pv_node && !in_check {
            // Reverse futility pruning
            if depth <= 7 && eval.abs() < MATE_BOUND && beta.abs() < MATE_BOUND {
                let margin = 75 * (depth - improving as i32);
                if eval - margin >= beta {
                    return (eval + beta) / 2;
                }
            }
            // Razoring
            if depth <= 3 && eval + 220 * depth < alpha {
                let q = self.qsearch(pos, hash, alpha, alpha + 1, ply);
                if self.stopped {
                    return 0;
                }
                if q <= alpha {
                    return q;
                }
            }
            // Null-move pruning
            if allow_null
                && depth >= 3
                && eval >= beta
                && static_eval >= beta - 20 * depth + 140
                && beta.abs() < MATE_BOUND
                && has_non_pawn_material(pos)
            {
                let r = 3 + depth / 4 + ((eval - beta) / 200).min(3);
                if let Ok(null) = pos.clone().swap_turn() {
                    let nh = hash_null(hash, pos);
                    self.t.path.push(hash);
                    let saved_floor = self.null_floor;
                    self.null_floor = self.t.path.len();
                    let s =
                        -self.negamax(&null, nh, depth - 1 - r, -beta, -beta + 1, ply + 1, false);
                    self.null_floor = saved_floor;
                    self.t.path.pop();
                    if self.stopped {
                        return 0;
                    }
                    if s >= beta {
                        return if s >= MATE_BOUND { beta } else { s };
                    }
                }
            }
        }

        // Internal iterative reduction: no TT move means ordering will be poor.
        if tt_move == 0 && depth >= 4 && !root {
            depth -= 1;
        }

        let mut moves = pos.legal_moves();
        if moves.is_empty() {
            return if in_check { -MATE + ply as i32 } else { 0 };
        }

        // Move ordering scores
        let us = color_index(pos);
        let killers = self.t.killers[ply];
        let n = moves.len();
        let mut scores = [0i32; 256];
        for (i, m) in moves.iter().enumerate() {
            let code = encode(m);
            scores[i] = if code == tt_move {
                4_000_000
            } else if m.is_capture() || m.promotion() == Some(Role::Queen) {
                let k = mvv_lva(m);
                if see_ge(pos, m, 0) {
                    2_000_000 + k
                } else {
                    -2_000_000 + k
                }
            } else if m.is_promotion() {
                -3_000_000
            } else if code == killers[0] {
                1_000_002
            } else if code == killers[1] {
                1_000_001
            } else {
                self.t.history[us][hist_index(code)]
            };
        }

        let original_alpha = alpha;
        let mut best = -INF;
        let mut best_move = 0u16;
        let mut searched = 0usize;
        let mut quiets: [u16; 64] = [0; 64];
        let mut quiet_count = 0usize;
        let mut quiets_seen = 0i32;
        let lmp_limit = (3 + depth * depth) / if improving { 1 } else { 2 };

        for i in 0..n {
            // selection sort step
            let mut bi = i;
            for j in i + 1..n {
                if scores[j] > scores[bi] {
                    bi = j;
                }
            }
            moves.swap(i, bi);
            scores.swap(i, bi);
            let m = &moves[i];
            let code = encode(m);
            if root && self.excluded.contains(&code) {
                continue;
            }
            let is_quiet = !m.is_capture() && !m.is_promotion();
            if is_quiet {
                quiets_seen += 1;
            }

            let mut child = pos.clone();
            child.play_unchecked(m);
            let gives_check = child.is_check();
            let advanced_pawn = is_advanced_pawn_push(pos, m);

            // Pruning of late / hopeless moves (never at the root, never when mated-in-N threat
            // has not yet been refuted by at least one move).
            if !root && !in_check && best > -MATE_BOUND && !gives_check && !advanced_pawn {
                if is_quiet {
                    if !pv_node && depth <= 8 && quiets_seen > lmp_limit {
                        continue;
                    }
                    if depth <= 8 && eval + 100 + 90 * depth <= alpha {
                        continue;
                    }
                    if depth <= 8 && scores[i] < 1_000_000 && !see_ge(pos, m, -60 * depth) {
                        continue;
                    }
                } else if depth <= 6 && !see_ge(pos, m, -100 * depth) {
                    continue;
                }
            }

            let child_hash = hash_after(hash, pos, m, &child);
            self.t.path.push(hash);
            searched += 1;
            let new_depth = depth - 1;
            let mut score;
            if searched == 1 {
                score = -self.negamax(&child, child_hash, new_depth, -beta, -alpha, ply + 1, true);
            } else {
                let mut r = 0;
                if depth >= 3
                    && searched > 1 + pv_node as usize
                    && !advanced_pawn
                    && (is_quiet || scores[i] < 0)
                {
                    r = self.t.lmr[(depth as usize).min(63)][searched.min(63)];
                    if pv_node {
                        r -= 1;
                    }
                    if !improving {
                        r += 1;
                    }
                    if gives_check {
                        r -= 1;
                    }
                    if code == killers[0] || code == killers[1] {
                        r -= 1;
                    }
                    if is_quiet {
                        r -= self.t.history[us][hist_index(code)] / 6000;
                    }
                    r = r.clamp(0, (new_depth - 1).max(0));
                }
                score = -self.negamax(
                    &child,
                    child_hash,
                    new_depth - r,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    true,
                );
                if score > alpha && r > 0 {
                    score = -self.negamax(
                        &child,
                        child_hash,
                        new_depth,
                        -alpha - 1,
                        -alpha,
                        ply + 1,
                        true,
                    );
                }
                if score > alpha && score < beta {
                    score =
                        -self.negamax(&child, child_hash, new_depth, -beta, -alpha, ply + 1, true);
                }
            }
            self.t.path.pop();
            if self.stopped {
                return 0;
            }

            if score > best {
                best = score;
                if score > alpha {
                    best_move = code;
                    alpha = score;
                    self.update_pv(ply, code);
                    if score >= beta {
                        if is_quiet {
                            let k = &mut self.t.killers[ply];
                            if k[0] != code {
                                k[1] = k[0];
                                k[0] = code;
                            }
                            let bonus = (depth * depth * 16).min(1600);
                            history_update(&mut self.t.history[us][hist_index(code)], bonus);
                            for &q in &quiets[..quiet_count] {
                                history_update(&mut self.t.history[us][hist_index(q)], -bonus);
                            }
                        }
                        break;
                    }
                }
            }
            if is_quiet && quiet_count < quiets.len() {
                quiets[quiet_count] = code;
                quiet_count += 1;
            }
        }

        if searched == 0 {
            // Every move was excluded (MultiPV root) or pruned.
            return if root { -INF } else { alpha };
        }

        let bound = if best >= beta {
            BOUND_LOWER
        } else if best > original_alpha {
            BOUND_EXACT
        } else {
            BOUND_UPPER
        };
        if !(root && self.pv_idx > 0) {
            self.t.tt.store(
                hash,
                best_move,
                score_to_tt(best, ply),
                static_eval,
                depth,
                bound,
            );
        }
        best
    }

    // -----------------------------------------------------------------------------------------
    // Quiescence
    // -----------------------------------------------------------------------------------------

    fn qsearch(&mut self, pos: &Chess, hash: u64, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.t.pv_len[ply] = 0;
        if self.check_stop() {
            return 0;
        }
        self.nodes += 1;
        self.seldepth = self.seldepth.max(ply);
        if pos.is_insufficient_material() {
            return 0;
        }
        let in_check = pos.is_check();
        if ply >= MAX_PLY - 2 {
            return if in_check { 0 } else { evaluate(pos) };
        }

        let tt = self.t.tt.probe(hash);
        let mut tt_move = 0u16;
        if let Some(e) = tt {
            tt_move = e.mv;
            let s = score_from_tt(e.score as i32, ply);
            let b = e.bound();
            if b == BOUND_EXACT
                || (b == BOUND_LOWER && s >= beta)
                || (b == BOUND_UPPER && s <= alpha)
            {
                return s;
            }
        }

        let original_alpha = alpha;
        let mut best;
        let stand;
        if in_check {
            stand = -INF;
            best = -INF;
        } else {
            stand = match tt {
                Some(e) => e.eval as i32,
                None => evaluate(pos),
            };
            if stand >= beta {
                // Guard against standing pat in a stalemate when only king + pawns are left.
                let b = pos.board();
                let us = b.by_color(pos.turn());
                if (us & !(b.kings() | b.pawns())).any() || !pos.legal_moves().is_empty() {
                    return stand;
                }
                return 0;
            }
            best = stand;
            if stand > alpha {
                alpha = stand;
            }
        }

        let mut moves = pos.legal_moves();
        if moves.is_empty() {
            return if in_check { -MATE + ply as i32 } else { 0 };
        }
        if !in_check {
            moves.retain(|m| m.is_capture() || m.promotion() == Some(Role::Queen));
        }
        let n = moves.len();
        let mut scores = [0i32; 256];
        for (i, m) in moves.iter().enumerate() {
            scores[i] = if encode(m) == tt_move {
                1_000_000
            } else {
                mvv_lva(m)
            };
        }

        let mut best_move = 0u16;
        for i in 0..n {
            let mut bi = i;
            for j in i + 1..n {
                if scores[j] > scores[bi] {
                    bi = j;
                }
            }
            moves.swap(i, bi);
            scores.swap(i, bi);
            let m = &moves[i];
            if !in_check {
                // Delta pruning
                if m.promotion().is_none() {
                    let gain = match m {
                        Move::EnPassant { .. } => value(Role::Pawn),
                        _ => m.capture().map(value).unwrap_or(0),
                    };
                    if stand + gain + 200 <= alpha {
                        continue;
                    }
                }
                // SEE pruning: skip losing captures
                if !see_ge(pos, m, 0) {
                    continue;
                }
            }
            let mut child = pos.clone();
            child.play_unchecked(m);
            let child_hash = hash_after(hash, pos, m, &child);
            let score = -self.qsearch(&child, child_hash, -beta, -alpha, ply + 1);
            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                    best_move = encode(m);
                    self.update_pv(ply, best_move);
                    if score >= beta {
                        break;
                    }
                }
            }
        }

        let bound = if best >= beta {
            BOUND_LOWER
        } else if best > original_alpha {
            BOUND_EXACT
        } else {
            BOUND_UPPER
        };
        self.t
            .tt
            .store(hash, best_move, score_to_tt(best, ply), stand, 0, bound);
        best
    }
}
