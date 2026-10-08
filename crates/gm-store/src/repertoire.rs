//! Opening repertoire (user-chosen lines for White and Black) and its spaced-repetition drill state.
//!
//! The repertoire is a move tree per side rooted at the standard initial position. Each row of
//! `repertoire_nodes` is one move (`parent_id = 0` for first moves). Moves played by the
//! repertoire's own side ("your moves") are flash cards with an SM-2 style schedule
//! (`ease`, `interval_days`, `reps`, `lapses`, `due`); for your side there is at most one move
//! per position, while opponent replies may branch freely.
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).
//! Times are unix seconds passed in by the caller (`now`) so the scheduling is testable.

use std::collections::HashMap;

use anyhow::Context;
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use shakmaty::fen::Fen;
use shakmaty::san::SanPlus;
use shakmaty::{Chess, EnPassantMode, Position};

use crate::pgn;
use crate::Store;

/// Max moves (nodes) stored per side.
pub const MAX_NODES_PER_SIDE: usize = 5000;
/// Max depth of a repertoire line, in plies.
pub const MAX_PLY: usize = 80;
/// Max note length (chars).
pub const MAX_NOTE: usize = 500;
/// Max recent games scanned by [`Store::repertoire_deviations`].
pub const MAX_DEVIATION_GAMES: u32 = 30;
/// A failed card comes back after this many seconds.
const RELEARN_SECS: i64 = 600;
const DAY_SECS: f64 = 86_400.0;
const START_EASE: f64 = 2.5;
const MIN_EASE: f64 = 1.3;
const MAX_EASE: f64 = 3.0;
const MAX_INTERVAL_DAYS: f64 = 365.0;
/// A card with an interval of at least this many days counts as "learned".
const LEARNED_DAYS: f64 = 21.0;

/// Which side a repertoire belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    White,
    Black,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::White => "white",
            Side::Black => "black",
        }
    }
    pub fn parse(s: &str) -> Option<Side> {
        match s.trim().to_ascii_lowercase().as_str() {
            "white" | "w" => Some(Side::White),
            "black" | "b" => Some(Side::Black),
            _ => None,
        }
    }
    /// Is the move at 1-based `ply` played by this side?
    pub fn owns_ply(self, ply: u32) -> bool {
        (ply % 2 == 1) == (self == Side::White)
    }
}

/// One move of the repertoire tree.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RepNode {
    pub id: i64,
    /// 0 = first move of the game.
    pub parent_id: i64,
    pub side: String,
    /// 1-based ply of this move (1 = White's first move).
    pub ply: u32,
    pub uci: String,
    pub san: String,
    /// Full FEN after the move.
    pub fen: String,
    pub note: String,
    /// `true` when this move is played by the repertoire's side (a drill card).
    pub mine: bool,
    pub ease: f64,
    pub interval_days: f64,
    pub reps: u32,
    pub lapses: u32,
    /// Unix seconds when the card is next due.
    pub due: i64,
    /// `mine && due <= now`.
    pub is_due: bool,
    pub last_review: Option<i64>,
}

/// Per-side numbers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SideStats {
    /// Moves stored.
    pub nodes: u32,
    /// Your moves (drill cards).
    pub cards: u32,
    /// Leaf count = distinct lines.
    pub lines: u32,
    /// Cards due now (including never-reviewed ones).
    pub due: u32,
    /// Cards never reviewed.
    pub new: u32,
    /// Cards with a long interval.
    pub learned: u32,
    /// Deepest ply.
    pub depth: u32,
}

/// `GET /api/repertoire?side=` payload.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RepTree {
    pub side: String,
    pub nodes: Vec<RepNode>,
    pub stats: SideStats,
    pub max_nodes: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RepSummary {
    pub due: u32,
    pub lines: u32,
    pub new: u32,
    pub cards: u32,
    pub white: SideStats,
    pub black: SideStats,
}

/// Adding a "your move" where a different one is already stored.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Conflict {
    pub ply: u32,
    pub parent_id: i64,
    pub existing_id: i64,
    pub existing_uci: String,
    pub existing_san: String,
    pub new_uci: String,
    pub new_san: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AddOutcome {
    /// Newly created moves.
    pub added: u32,
    /// Moves removed because `replace` swapped one of your moves.
    pub removed: u32,
    /// Node ids of the line, in order (empty when there is a conflict).
    pub path: Vec<i64>,
    /// Set when nothing was saved because one of your moves would be replaced.
    pub conflict: Option<Conflict>,
}

/// Why a repertoire change was refused (mapped to 4xx by the server).
#[derive(Clone, Debug, PartialEq)]
pub enum RepError {
    NotFound,
    /// Empty move list.
    NoMoves,
    /// Line deeper than [`MAX_PLY`].
    TooDeep,
    /// More than [`MAX_NODES_PER_SIDE`] moves.
    Full,
    /// 1-based index into the submitted moves, offending move.
    Illegal(usize, String),
    /// Parent belongs to the other side.
    WrongSide,
    /// Drill attempt on an opponent move.
    NotACard,
}

/// One line chosen for a drill.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DrillLine {
    pub side: String,
    /// Path from the first move; drill until the last node (always one of your moves).
    pub nodes: Vec<RepNode>,
    /// Due cards in this line.
    pub due_in_line: u32,
    /// Due cards in the whole repertoire (both sides, or the requested side).
    pub due_total: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReviewOutcome {
    pub correct: bool,
    pub expected_uci: String,
    pub expected_san: String,
    /// The card after scheduling.
    pub card: RepNode,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MoveRef {
    pub uci: String,
    pub san: String,
}

/// Where a recent game left the repertoire.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Deviation {
    pub game_id: i64,
    pub side: String,
    pub white: String,
    pub black: String,
    pub result: String,
    pub created_at: String,
    pub opening_name: Option<String>,
    /// `deviated` (you played another move), `unprepared` (opponent move you have not prepared),
    /// `end` (you followed your preparation to its end), `followed` (the game ended inside it).
    pub status: String,
    /// 1-based ply of the deviating move (or of the last move inside the repertoire).
    pub ply: u32,
    pub played_uci: String,
    pub played_san: String,
    /// What the repertoire has in that position.
    pub expected: Vec<MoveRef>,
    /// Repertoire node whose position is `fen_before` (0 = initial position) — add replies here.
    pub parent_id: i64,
    /// FEN before the deviating move.
    pub fen_before: String,
    /// UCI moves played before the deviating move.
    pub moves_before: Vec<String>,
    /// Plies the game spent inside the repertoire.
    pub book_plies: u32,
}

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS repertoire_nodes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  side TEXT NOT NULL CHECK (side IN ('white','black')),
  parent_id INTEGER NOT NULL DEFAULT 0,
  ply INTEGER NOT NULL,
  uci TEXT NOT NULL,
  san TEXT NOT NULL,
  fen TEXT NOT NULL,
  note TEXT NOT NULL DEFAULT '',
  ease REAL NOT NULL DEFAULT 2.5,
  interval_days REAL NOT NULL DEFAULT 0,
  reps INTEGER NOT NULL DEFAULT 0,
  lapses INTEGER NOT NULL DEFAULT 0,
  due INTEGER NOT NULL DEFAULT 0,
  last_review INTEGER,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  UNIQUE (side, parent_id, uci)
);
CREATE INDEX IF NOT EXISTS repertoire_nodes_parent ON repertoire_nodes (parent_id);
CREATE INDEX IF NOT EXISTS repertoire_nodes_side_due ON repertoire_nodes (side, due);
"#,
    )
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

const NODE_COLS: &str =
    "id, parent_id, side, ply, uci, san, fen, note, ease, interval_days, reps, lapses, due, last_review";

fn row_to_node(r: &Row<'_>, now: i64) -> rusqlite::Result<RepNode> {
    let side: String = r.get(2)?;
    let ply = r.get::<_, i64>(3)?.clamp(0, u32::MAX as i64) as u32;
    let mine = Side::parse(&side).is_some_and(|s| s.owns_ply(ply));
    let due: i64 = r.get(12)?;
    Ok(RepNode {
        id: r.get(0)?,
        parent_id: r.get(1)?,
        side,
        ply,
        uci: r.get(4)?,
        san: r.get(5)?,
        fen: r.get(6)?,
        note: r.get(7)?,
        mine,
        ease: r.get(8)?,
        interval_days: r.get(9)?,
        reps: r.get::<_, i64>(10)?.clamp(0, u32::MAX as i64) as u32,
        lapses: r.get::<_, i64>(11)?.clamp(0, u32::MAX as i64) as u32,
        due,
        is_due: mine && due <= now,
        last_review: r.get(13)?,
    })
}

/// Position identity: the first four FEN fields.
fn fen_key(fen: &str) -> String {
    fen.split_whitespace().take(4).collect::<Vec<_>>().join(" ")
}

fn fen_of(pos: &Chess) -> String {
    Fen::from_position(pos.clone(), EnPassantMode::Legal).to_string()
}

fn load_side(conn: &rusqlite::Connection, side: Side, now: i64) -> rusqlite::Result<Vec<RepNode>> {
    let mut stmt =
        conn.prepare_cached(&format!("SELECT {NODE_COLS} FROM repertoire_nodes WHERE side = ?1 ORDER BY id"))?;
    let rows = stmt.query_map(params![side.as_str()], |r| row_to_node(r, now))?;
    rows.collect()
}

fn load_node(conn: &rusqlite::Connection, id: i64, now: i64) -> rusqlite::Result<Option<RepNode>> {
    conn.prepare_cached(&format!("SELECT {NODE_COLS} FROM repertoire_nodes WHERE id = ?1"))?
        .query_row(params![id], |r| row_to_node(r, now))
        .optional()
}

fn side_stats(nodes: &[RepNode]) -> SideStats {
    let mut has_child = std::collections::HashSet::with_capacity(nodes.len());
    for n in nodes {
        has_child.insert(n.parent_id);
    }
    let mut s = SideStats { nodes: nodes.len() as u32, ..Default::default() };
    for n in nodes {
        if !has_child.contains(&n.id) {
            s.lines += 1;
        }
        s.depth = s.depth.max(n.ply);
        if n.mine {
            s.cards += 1;
            if n.is_due {
                s.due += 1;
            }
            if n.reps == 0 && n.lapses == 0 {
                s.new += 1;
            }
            if n.interval_days >= LEARNED_DAYS {
                s.learned += 1;
            }
        }
    }
    s
}

fn delete_subtree(tx: &Transaction<'_>, id: i64) -> rusqlite::Result<usize> {
    tx.execute(
        "WITH RECURSIVE sub(id) AS (SELECT ?1 UNION ALL \
         SELECT n.id FROM repertoire_nodes n JOIN sub ON n.parent_id = sub.id) \
         DELETE FROM repertoire_nodes WHERE id IN (SELECT id FROM sub)",
        params![id],
    )
}

/// Small deterministic PRNG (SplitMix64) so drill picks are testable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Index chosen with probability proportional to `weights` (sum must be > 0).
    fn pick(&mut self, weights: &[u64]) -> usize {
        let total: u64 = weights.iter().sum();
        if total == 0 {
            return 0;
        }
        let mut r = self.next() % total;
        for (i, w) in weights.iter().enumerate() {
            if r < *w {
                return i;
            }
            r -= w;
        }
        weights.len().saturating_sub(1)
    }
}

/// Drill priority of a card: due cards first, weak/overdue ones more often.
fn card_weight(n: &RepNode, any: bool, now: i64) -> u64 {
    if !n.mine {
        return 0;
    }
    if n.is_due {
        let overdue_days = ((now - n.due).max(0) / 86_400).min(20) as u64;
        let weak = if n.ease < 2.0 { 4 } else { 0 };
        10 + overdue_days + (n.lapses.min(5) as u64) * 2 + weak
    } else if any {
        1
    } else {
        0
    }
}

fn schedule(card: &mut RepNode, correct: bool, now: i64) {
    if correct {
        if card.due > now {
            // Early review: nothing to reschedule.
            return;
        }
        card.reps = card.reps.saturating_add(1);
        card.interval_days = match card.reps {
            1 => 1.0,
            2 => 3.0,
            _ => (card.interval_days * card.ease).max(card.interval_days + 1.0),
        }
        .min(MAX_INTERVAL_DAYS);
        card.ease = (card.ease + 0.1).min(MAX_EASE);
        card.due = now + (card.interval_days * DAY_SECS) as i64;
    } else {
        card.lapses = card.lapses.saturating_add(1);
        card.reps = 0;
        card.ease = (card.ease - 0.2).max(MIN_EASE);
        card.interval_days = 0.0;
        card.due = now + RELEARN_SECS;
    }
    card.last_review = Some(now);
    card.is_due = card.due <= now;
}

/// Positions of one side's repertoire: position key -> moves stored there, and a node that
/// reaches the position (0 = initial position).
struct PositionIndex {
    moves: HashMap<String, Vec<MoveRef>>,
    node_at: HashMap<String, i64>,
}

impl PositionIndex {
    fn build(nodes: &[RepNode]) -> PositionIndex {
        let start = fen_key(&fen_of(&Chess::default()));
        let fen_by_id: HashMap<i64, &str> = nodes.iter().map(|n| (n.id, n.fen.as_str())).collect();
        let mut moves: HashMap<String, Vec<MoveRef>> = HashMap::new();
        let mut node_at: HashMap<String, i64> = HashMap::new();
        node_at.insert(start.clone(), 0);
        for n in nodes {
            node_at.entry(fen_key(&n.fen)).or_insert(n.id);
            let before = if n.parent_id == 0 {
                start.clone()
            } else {
                match fen_by_id.get(&n.parent_id) {
                    Some(f) => fen_key(f),
                    None => continue,
                }
            };
            let list = moves.entry(before).or_default();
            if !list.iter().any(|m| m.uci == n.uci) {
                list.push(MoveRef { uci: n.uci.clone(), san: n.san.clone() });
            }
        }
        PositionIndex { moves, node_at }
    }
}

// ---------------------------------------------------------------------------------------------
// Store API
// ---------------------------------------------------------------------------------------------

impl Store {
    /// The whole tree of one side (flat, ordered by id: parents come before children).
    pub fn repertoire_tree(&self, side: Side, now: i64) -> anyhow::Result<RepTree> {
        let conn = self.conn.lock();
        let nodes = load_side(&conn, side, now).context("loading repertoire")?;
        let stats = side_stats(&nodes);
        Ok(RepTree { side: side.as_str().into(), nodes, stats, max_nodes: MAX_NODES_PER_SIDE as u32 })
    }

    /// Add a line of UCI moves below `parent_id` (0 = from the initial position). Moves that already
    /// exist are reused. If one of your moves differs from the stored one in that position, nothing
    /// is saved and `conflict` is returned — unless `replace` is set, which removes the old move
    /// (and everything after it) first.
    pub fn repertoire_add_line(
        &self,
        side: Side,
        parent_id: i64,
        moves: &[String],
        replace: bool,
        now: i64,
    ) -> anyhow::Result<Result<AddOutcome, RepError>> {
        if moves.is_empty() {
            return Ok(Err(RepError::NoMoves));
        }
        if moves.len() > MAX_PLY {
            return Ok(Err(RepError::TooDeep));
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let (mut pos, base_ply) = if parent_id == 0 {
            (Chess::default(), 0u32)
        } else {
            let Some(parent) = load_node(&tx, parent_id, now)? else {
                return Ok(Err(RepError::NotFound));
            };
            if parent.side != side.as_str() {
                return Ok(Err(RepError::WrongSide));
            }
            match pgn::parse_position(&parent.fen) {
                Ok(p) => (p, parent.ply),
                Err(_) => return Ok(Err(RepError::NotFound)),
            }
        };
        if base_ply as usize + moves.len() > MAX_PLY {
            return Ok(Err(RepError::TooDeep));
        }
        let mut count: usize = tx.query_row(
            "SELECT COUNT(*) FROM repertoire_nodes WHERE side = ?1",
            params![side.as_str()],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let mut out = AddOutcome::default();
        let mut cur = parent_id;
        for (i, raw) in moves.iter().enumerate() {
            let raw = raw.trim().to_ascii_lowercase();
            let mv = match pgn::uci_to_move(&pos, &raw) {
                Ok(m) => m,
                Err(_) => return Ok(Err(RepError::Illegal(i + 1, raw.chars().take(8).collect()))),
            };
            let uci = pgn::move_to_uci(&mv);
            let san = SanPlus::from_move_and_play_unchecked(&mut pos, &mv).to_string();
            let fen = fen_of(&pos);
            let ply = base_ply + i as u32 + 1;

            let existing: Option<i64> = tx
                .prepare_cached("SELECT id FROM repertoire_nodes WHERE side = ?1 AND parent_id = ?2 AND uci = ?3")?
                .query_row(params![side.as_str(), cur, uci], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                out.path.push(id);
                cur = id;
                continue;
            }
            if side.owns_ply(ply) {
                let sibling: Option<(i64, String, String)> = tx
                    .prepare_cached(
                        "SELECT id, uci, san FROM repertoire_nodes WHERE side = ?1 AND parent_id = ?2 LIMIT 1",
                    )?
                    .query_row(params![side.as_str(), cur], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .optional()?;
                if let Some((sid, suci, ssan)) = sibling {
                    if !replace {
                        return Ok(Ok(AddOutcome {
                            conflict: Some(Conflict {
                                ply,
                                parent_id: cur,
                                existing_id: sid,
                                existing_uci: suci,
                                existing_san: ssan,
                                new_uci: uci,
                                new_san: san,
                            }),
                            ..Default::default()
                        }));
                    }
                    let removed = delete_subtree(&tx, sid)?;
                    out.removed += removed as u32;
                    count = count.saturating_sub(removed);
                }
            }
            if count >= MAX_NODES_PER_SIDE {
                return Ok(Err(RepError::Full));
            }
            tx.prepare_cached(
                "INSERT INTO repertoire_nodes (side, parent_id, ply, uci, san, fen, ease, due) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![side.as_str(), cur, ply, uci, san, fen, START_EASE, now])?;
            cur = tx.last_insert_rowid();
            count += 1;
            out.added += 1;
            out.path.push(cur);
        }
        tx.commit()?;
        Ok(Ok(out))
    }

    /// Set the note of one move. `None` if the node does not exist.
    pub fn repertoire_set_note(&self, id: i64, note: &str, now: i64) -> anyhow::Result<Option<RepNode>> {
        let note: String = note.trim().chars().take(MAX_NOTE).collect();
        let conn = self.conn.lock();
        let n = conn.execute("UPDATE repertoire_nodes SET note = ?1 WHERE id = ?2", params![note, id])?;
        if n == 0 {
            return Ok(None);
        }
        Ok(load_node(&conn, id, now)?)
    }

    /// Delete a move and everything after it. Returns the number of moves removed.
    pub fn repertoire_delete(&self, id: i64) -> anyhow::Result<usize> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let exists: Option<i64> =
            tx.query_row("SELECT id FROM repertoire_nodes WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        if exists.is_none() {
            return Ok(0);
        }
        let n = delete_subtree(&tx, id)?;
        tx.commit()?;
        Ok(n)
    }

    /// Delete a whole side.
    pub fn repertoire_clear(&self, side: Side) -> anyhow::Result<usize> {
        let conn = self.conn.lock();
        Ok(conn.execute("DELETE FROM repertoire_nodes WHERE side = ?1", params![side.as_str()])?)
    }

    pub fn repertoire_summary(&self, now: i64) -> anyhow::Result<RepSummary> {
        let conn = self.conn.lock();
        let white = side_stats(&load_side(&conn, Side::White, now)?);
        let black = side_stats(&load_side(&conn, Side::Black, now)?);
        Ok(RepSummary {
            due: white.due + black.due,
            lines: white.lines + black.lines,
            new: white.new + black.new,
            cards: white.cards + black.cards,
            white,
            black,
        })
    }

    /// Pick the next line to drill: walks the tree preferring due and weak cards; opponent
    /// replies are chosen at random weighted by how much work their branch needs. With `any`,
    /// cards that are not due are eligible too (practice mode). `None` = nothing to drill.
    pub fn repertoire_drill_next(
        &self,
        side: Option<Side>,
        any: bool,
        seed: u64,
        now: i64,
    ) -> anyhow::Result<Option<DrillLine>> {
        let conn = self.conn.lock();
        let sides: Vec<Side> = match side {
            Some(s) => vec![s],
            None => vec![Side::White, Side::Black],
        };
        let mut rng = Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B);
        let mut candidates: Vec<(Side, Vec<RepNode>, Vec<u64>)> = Vec::new();
        let mut due_total = 0u32;
        for s in sides {
            let nodes = load_side(&conn, s, now)?;
            due_total += nodes.iter().filter(|n| n.is_due).count() as u32;
            // Subtree weights: children always have larger ids than their parents.
            let index: HashMap<i64, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
            let mut weight: Vec<u64> = nodes.iter().map(|n| card_weight(n, any, now)).collect();
            for i in (0..nodes.len()).rev() {
                if let Some(&p) = index.get(&nodes[i].parent_id) {
                    if p < i {
                        weight[p] = weight[p].saturating_add(weight[i]);
                    }
                }
            }
            candidates.push((s, nodes, weight));
        }
        let totals: Vec<u64> = candidates
            .iter()
            .map(|(_, nodes, w)| nodes.iter().zip(w).filter(|(n, _)| n.parent_id == 0).map(|(_, w)| *w).sum())
            .collect();
        if totals.iter().all(|t| *t == 0) {
            return Ok(None);
        }
        let ci = rng.pick(&totals);
        let (s, nodes, weight) = &candidates[ci];
        let mut children: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, n) in nodes.iter().enumerate() {
            children.entry(n.parent_id).or_default().push(i);
        }
        let mut line = Vec::new();
        let mut cur = 0i64;
        while line.len() < MAX_PLY {
            let Some(kids) = children.get(&cur) else { break };
            let live: Vec<usize> = kids.iter().copied().filter(|&i| weight[i] > 0).collect();
            if live.is_empty() {
                break;
            }
            let ws: Vec<u64> = live.iter().map(|&i| weight[i]).collect();
            let pick = if nodes[live[0]].mine {
                // Your side should have one move; if there are several, take the neediest.
                live.iter().zip(&ws).max_by_key(|(_, w)| **w).map(|(i, _)| *i).unwrap_or(live[0])
            } else {
                live[rng.pick(&ws)]
            };
            line.push(nodes[pick].clone());
            cur = nodes[pick].id;
        }
        // Never end on an opponent move.
        while line.last().is_some_and(|n| !n.mine) {
            line.pop();
        }
        if line.is_empty() {
            return Ok(None);
        }
        let due_in_line = line.iter().filter(|n| n.is_due).count() as u32;
        Ok(Some(DrillLine { side: s.as_str().into(), nodes: line, due_in_line, due_total }))
    }

    /// Grade an answer for card `id`. Logs a `repertoire_review` activity.
    pub fn repertoire_review(
        &self,
        id: i64,
        answer_uci: &str,
        now: i64,
    ) -> anyhow::Result<Result<ReviewOutcome, RepError>> {
        let outcome = {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            let Some(mut card) = load_node(&tx, id, now)? else {
                return Ok(Err(RepError::NotFound));
            };
            if !card.mine {
                return Ok(Err(RepError::NotACard));
            }
            let answer = answer_uci.trim().to_ascii_lowercase();
            let mut correct = answer == card.uci;
            if !correct {
                // Accept other spellings of the same move (e.g. castling as king-takes-rook).
                let before = if card.parent_id == 0 {
                    Some(Chess::default())
                } else {
                    load_node(&tx, card.parent_id, now)?.and_then(|p| pgn::parse_position(&p.fen).ok())
                };
                if let Some(mut pos) = before {
                    if let Ok(m) = pgn::uci_to_move(&pos, &answer) {
                        pos.play_unchecked(&m);
                        correct = fen_key(&fen_of(&pos)) == fen_key(&card.fen);
                    }
                }
            }
            schedule(&mut card, correct, now);
            tx.execute(
                "UPDATE repertoire_nodes SET ease = ?1, interval_days = ?2, reps = ?3, lapses = ?4, due = ?5, \
                 last_review = ?6 WHERE id = ?7",
                params![card.ease, card.interval_days, card.reps, card.lapses, card.due, card.last_review, card.id],
            )?;
            tx.commit()?;
            ReviewOutcome { correct, expected_uci: card.uci.clone(), expected_san: card.san.clone(), card }
        };
        // Best effort: the daily plan should not break a review.
        let _ = self.log_activity("repertoire_review", 1);
        Ok(Ok(outcome))
    }

    /// For the user's most recent games (newest first), where each left the repertoire.
    /// Games of a side with an empty repertoire, or not from the initial position, are skipped.
    pub fn repertoire_deviations(&self, limit: u32, now: i64) -> anyhow::Result<Vec<Deviation>> {
        let limit = limit.clamp(1, MAX_DEVIATION_GAMES);
        let conn = self.conn.lock();
        let white = load_side(&conn, Side::White, now)?;
        let black = load_side(&conn, Side::Black, now)?;
        let idx_white = PositionIndex::build(&white);
        let idx_black = PositionIndex::build(&black);
        let mut stmt = conn.prepare_cached(
            "SELECT id, white, black, result, user_color, opening_name, created_at, start_fen, moves \
             FROM games WHERE user_color IN ('white','black') ORDER BY id DESC LIMIT ?1",
        )?;
        type GameRow = (i64, String, String, String, String, Option<String>, String, String, String);
        let rows: Vec<GameRow> = stmt
            .query_map(params![limit * 3], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (game_id, w, b, result, color, opening_name, created_at, start_fen, moves) in rows {
            if out.len() >= limit as usize {
                break;
            }
            let Some(side) = Side::parse(&color) else { continue };
            let (nodes, idx) = match side {
                Side::White => (&white, &idx_white),
                Side::Black => (&black, &idx_black),
            };
            if nodes.is_empty() || !pgn::is_standard_start(&start_fen) {
                continue;
            }
            let moves = crate::moves_from_db(&moves);
            if moves.is_empty() {
                continue;
            }
            let mut d = Deviation {
                game_id,
                side: side.as_str().into(),
                white: w,
                black: b,
                result,
                created_at,
                opening_name,
                ..Default::default()
            };
            let mut pos = Chess::default();
            let mut status = "followed";
            for (i, raw) in moves.iter().take(MAX_PLY).enumerate() {
                let fen_before = fen_of(&pos);
                let key = fen_key(&fen_before);
                let Ok(m) = pgn::uci_to_move(&pos, raw) else { break };
                let uci = pgn::move_to_uci(&m);
                let ply = i as u32 + 1;
                let Some(expected) = idx.moves.get(&key) else {
                    status = "end";
                    break;
                };
                if expected.iter().any(|e| e.uci == uci) {
                    pos.play_unchecked(&m);
                    d.book_plies = ply;
                    d.ply = ply;
                    continue;
                }
                let mut p2 = pos.clone();
                let san = SanPlus::from_move_and_play_unchecked(&mut p2, &m).to_string();
                status = if side.owns_ply(ply) { "deviated" } else { "unprepared" };
                d.ply = ply;
                d.played_uci = uci;
                d.played_san = san;
                d.expected = expected.clone();
                d.parent_id = idx.node_at.get(&key).copied().unwrap_or(0);
                d.fen_before = fen_before;
                d.moves_before = moves[..i].to_vec();
                break;
            }
            if status == "end" || status == "followed" {
                d.fen_before = fen_of(&pos);
                d.moves_before = moves[..d.book_plies as usize].to_vec();
                d.parent_id = idx.node_at.get(&fen_key(&d.fen_before)).copied().unwrap_or(0);
                if status == "followed" && moves.len() > MAX_PLY {
                    status = "end";
                }
            }
            d.status = status.into();
            out.push(d);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ucis(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn add_line_builds_tree_and_reuses_moves() {
        let s = Store::open_in_memory().unwrap();
        let r = s.repertoire_add_line(Side::White, 0, &ucis("e2e4 e7e5 g1f3 b8c6 f1c4"), false, NOW).unwrap().unwrap();
        assert_eq!(r.added, 5);
        assert!(r.conflict.is_none());
        // Opponent branch: 1.e4 c5 2.c3
        let r2 = s.repertoire_add_line(Side::White, 0, &ucis("e2e4 c7c5 c2c3"), false, NOW).unwrap().unwrap();
        assert_eq!(r2.added, 2, "e4 is reused");
        assert_eq!(r2.path[0], r.path[0]);
        let tree = s.repertoire_tree(Side::White, NOW).unwrap();
        assert_eq!(tree.stats.nodes, 7);
        assert_eq!(tree.stats.lines, 2);
        assert_eq!(tree.stats.cards, 4); // e4, Nf3, Bc4, c3
        assert_eq!(tree.stats.due, 4, "new cards are due");
        let e4 = &tree.nodes[0];
        assert_eq!((e4.san.as_str(), e4.ply, e4.mine), ("e4", 1, true));
        let e5 = tree.nodes.iter().find(|n| n.san == "e5").unwrap();
        assert!(!e5.mine);
        // Black side is independent.
        assert_eq!(s.repertoire_tree(Side::Black, NOW).unwrap().nodes.len(), 0);
    }

    #[test]
    fn your_move_conflict_and_replace() {
        let s = Store::open_in_memory().unwrap();
        s.repertoire_add_line(Side::Black, 0, &ucis("e2e4 c7c6 d2d4 d7d5"), false, NOW).unwrap().unwrap();
        let r = s.repertoire_add_line(Side::Black, 0, &ucis("e2e4 e7e5"), false, NOW).unwrap().unwrap();
        let c = r.conflict.expect("conflict");
        assert_eq!((c.existing_san.as_str(), c.new_san.as_str(), c.ply), ("c6", "e5", 2));
        assert_eq!(s.repertoire_tree(Side::Black, NOW).unwrap().nodes.len(), 4, "nothing saved");
        let r = s.repertoire_add_line(Side::Black, 0, &ucis("e2e4 e7e5"), true, NOW).unwrap().unwrap();
        assert_eq!((r.added, r.removed), (1, 3));
        let sans: Vec<String> = s.repertoire_tree(Side::Black, NOW).unwrap().nodes.into_iter().map(|n| n.san).collect();
        assert_eq!(sans, vec!["e4", "e5"]);
    }

    #[test]
    fn validation_errors() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.repertoire_add_line(Side::White, 0, &[], false, NOW).unwrap(), Err(RepError::NoMoves));
        assert_eq!(
            s.repertoire_add_line(Side::White, 0, &ucis("e2e4 e2e4"), false, NOW).unwrap(),
            Err(RepError::Illegal(2, "e2e4".into()))
        );
        assert_eq!(s.repertoire_add_line(Side::White, 999, &ucis("e2e4"), false, NOW).unwrap(), Err(RepError::NotFound));
        let r = s.repertoire_add_line(Side::White, 0, &ucis("e2e4"), false, NOW).unwrap().unwrap();
        assert_eq!(
            s.repertoire_add_line(Side::Black, r.path[0], &ucis("e7e5"), false, NOW).unwrap(),
            Err(RepError::WrongSide)
        );
        let long: Vec<String> = ["g1f3", "g8f6", "f3g1", "f6g8"].iter().cycle().take(MAX_PLY + 1).map(|s| s.to_string()).collect();
        assert_eq!(s.repertoire_add_line(Side::White, 0, &long, false, NOW).unwrap(), Err(RepError::TooDeep));
        // Nothing illegal was stored.
        assert_eq!(s.repertoire_tree(Side::White, NOW).unwrap().nodes.len(), 1);
    }

    #[test]
    fn size_is_bounded() {
        let s = Store::open_in_memory().unwrap();
        {
            let conn = s.conn.lock();
            for i in 0..MAX_NODES_PER_SIDE {
                conn.execute(
                    "INSERT INTO repertoire_nodes (side, parent_id, ply, uci, san, fen) VALUES ('white', ?1, 1, 'a2a3', 'a3', '')",
                    params![100_000 + i as i64],
                )
                .unwrap();
            }
        }
        assert_eq!(s.repertoire_add_line(Side::White, 0, &ucis("e2e4"), false, NOW).unwrap(), Err(RepError::Full));
        assert!(s.repertoire_add_line(Side::Black, 0, &ucis("e2e4"), false, NOW).unwrap().is_ok());
    }

    #[test]
    fn notes_and_delete_subtree() {
        let s = Store::open_in_memory().unwrap();
        let r = s.repertoire_add_line(Side::White, 0, &ucis("d2d4 d7d5 c1f4 g8f6 e2e3"), false, NOW).unwrap().unwrap();
        let n = s.repertoire_set_note(r.path[2], "  The London!  ", NOW).unwrap().unwrap();
        assert_eq!(n.note, "The London!");
        assert!(s.repertoire_set_note(12345, "x", NOW).unwrap().is_none());
        assert_eq!(s.repertoire_delete(r.path[1]).unwrap(), 4);
        assert_eq!(s.repertoire_tree(Side::White, NOW).unwrap().nodes.len(), 1);
        assert_eq!(s.repertoire_delete(r.path[1]).unwrap(), 0);
        assert_eq!(s.repertoire_clear(Side::White).unwrap(), 1);
    }

    #[test]
    fn review_schedules_cards() {
        let s = Store::open_in_memory().unwrap();
        let r = s.repertoire_add_line(Side::White, 0, &ucis("e2e4 e7e5 e1e2"), false, NOW).unwrap().unwrap();
        let ok = s.repertoire_review(r.path[0], "e2e4", NOW).unwrap().unwrap();
        assert!(ok.correct);
        assert_eq!(ok.card.reps, 1);
        assert_eq!(ok.card.due, NOW + 86_400);
        assert!(!ok.card.is_due);
        // Not due any more: an early correct answer changes nothing.
        let again = s.repertoire_review(r.path[0], "e2e4", NOW + 10).unwrap().unwrap();
        assert_eq!(again.card.due, NOW + 86_400);
        // Second review after a day: interval 3.
        let ok2 = s.repertoire_review(r.path[0], "e2e4", NOW + 86_400).unwrap().unwrap();
        assert_eq!(ok2.card.interval_days, 3.0);
        // Wrong answer resets.
        let bad = s.repertoire_review(r.path[0], "d2d4", NOW + 5 * 86_400).unwrap().unwrap();
        assert!(!bad.correct);
        assert_eq!(bad.expected_san, "e4");
        assert_eq!((bad.card.reps, bad.card.lapses), (0, 1));
        assert_eq!(bad.card.due, NOW + 5 * 86_400 + RELEARN_SECS);
        assert!(bad.card.ease < START_EASE + 0.2);
        // Opponent moves are not cards.
        assert_eq!(s.repertoire_review(r.path[1], "e7e5", NOW).unwrap(), Err(RepError::NotACard));
        assert_eq!(s.repertoire_review(999, "e7e5", NOW).unwrap(), Err(RepError::NotFound));
        // Activity is logged.
        let days = s.activity_days(1).unwrap();
        assert_eq!(days[0].counts.get("repertoire_review"), Some(&4));
    }

    #[test]
    fn drill_prefers_due_branches_and_ends_on_your_move() {
        let s = Store::open_in_memory().unwrap();
        let a = s.repertoire_add_line(Side::White, 0, &ucis("e2e4 e7e5 g1f3 b8c6"), false, NOW).unwrap().unwrap();
        let b = s.repertoire_add_line(Side::White, 0, &ucis("e2e4 c7c5 c2c3"), false, NOW).unwrap().unwrap();
        // Learn everything in the e5 branch and e4 itself.
        for id in [a.path[0], a.path[2]] {
            s.repertoire_review(id, if id == a.path[0] { "e2e4" } else { "g1f3" }, NOW).unwrap().unwrap();
        }
        for seed in 0..20 {
            let line = s.repertoire_drill_next(None, false, seed, NOW).unwrap().expect("c3 is due");
            assert_eq!(line.side, "white");
            let sans: Vec<&str> = line.nodes.iter().map(|n| n.san.as_str()).collect();
            assert_eq!(sans, vec!["e4", "c5", "c3"]);
            assert_eq!(line.due_in_line, 1);
            assert_eq!(line.nodes.last().unwrap().id, b.path[2]);
        }
        s.repertoire_review(b.path[2], "c2c3", NOW).unwrap().unwrap();
        assert!(s.repertoire_drill_next(None, false, 1, NOW).unwrap().is_none());
        // Practice mode still drills, and never ends on an opponent move (b8c6 is a leaf).
        for seed in 0..20 {
            let line = s.repertoire_drill_next(Some(Side::White), true, seed, NOW).unwrap().unwrap();
            assert!(line.nodes.last().unwrap().mine);
        }
        assert!(s.repertoire_drill_next(Some(Side::Black), true, 1, NOW).unwrap().is_none());
    }

    #[test]
    fn summary_counts_both_sides() {
        let s = Store::open_in_memory().unwrap();
        s.repertoire_add_line(Side::White, 0, &ucis("e2e4 e7e5 g1f3"), false, NOW).unwrap().unwrap();
        s.repertoire_add_line(Side::Black, 0, &ucis("e2e4 c7c6"), false, NOW).unwrap().unwrap();
        s.repertoire_add_line(Side::Black, 0, &ucis("d2d4 d7d5"), false, NOW).unwrap().unwrap();
        let sum = s.repertoire_summary(NOW).unwrap();
        assert_eq!(sum.lines, 3);
        assert_eq!(sum.due, 4);
        assert_eq!(sum.white.cards, 2);
        assert_eq!(sum.black.lines, 2);
    }

    fn save_game(s: &Store, moves: &str, color: &str) -> i64 {
        s.create_game(&crate::NewGame {
            white: "W".into(),
            black: "B".into(),
            result: "1-0".into(),
            moves: ucis(moves),
            user_color: Some(color.into()),
            ..Default::default()
        })
        .unwrap()
        .id
    }

    #[test]
    fn deviations_find_where_games_left_the_repertoire() {
        let s = Store::open_in_memory().unwrap();
        s.repertoire_add_line(Side::Black, 0, &ucis("e2e4 c7c6 d2d4 d7d5 b1c3 d5e4"), false, NOW).unwrap().unwrap();
        s.repertoire_add_line(Side::White, 0, &ucis("e2e4 e7e5 g1f3"), false, NOW).unwrap().unwrap();
        // You (black) played ...e5 instead of ...c6.
        let g1 = save_game(&s, "e2e4 e7e5 g1f3", "black");
        // Opponent (white) played 3.e5, not prepared.
        let g2 = save_game(&s, "e2e4 c7c6 d2d4 d7d5 e4e5 c8f5", "black");
        // White game that followed the whole prep then went on.
        let g3 = save_game(&s, "e2e4 e7e5 g1f3 b8c6 f1c4", "white");
        // Imported game without a user colour is ignored.
        s.create_game(&crate::NewGame { moves: ucis("e2e4"), ..Default::default() }).unwrap();
        let devs = s.repertoire_deviations(10, NOW).unwrap();
        assert_eq!(devs.len(), 3);
        let d3 = &devs[0];
        assert_eq!((d3.game_id, d3.status.as_str(), d3.book_plies), (g3, "end", 3));
        let d2 = &devs[1];
        assert_eq!((d2.game_id, d2.status.as_str(), d2.ply), (g2, "unprepared", 5));
        assert_eq!(d2.played_san, "e5");
        assert_eq!(d2.expected[0].san, "Nc3");
        assert_eq!(d2.moves_before.len(), 4);
        // "Add it" goes below the ...d5 node.
        let tree = s.repertoire_tree(Side::Black, NOW).unwrap();
        let d5 = tree.nodes.iter().find(|n| n.san == "d5").unwrap();
        assert_eq!(d2.parent_id, d5.id);
        let d1 = &devs[2];
        assert_eq!((d1.game_id, d1.status.as_str(), d1.ply), (g1, "deviated", 2));
        assert_eq!((d1.played_san.as_str(), d1.expected[0].san.as_str()), ("e5", "c6"));
        assert_eq!(d1.parent_id, tree.nodes[0].id);
    }
}
