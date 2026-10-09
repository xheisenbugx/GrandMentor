//! Weekly personal training set ("Your weekly set"): a small, stable set of puzzles per ISO week,
//! built by the server from the user's weakest tactic themes, plus the per-item results.
//!
//! * `weekly_sets` — one row per generated set. The current set of a week is the newest row for
//!   that week; "New set" adds another row (older rows keep their results for the history).
//! * `weekly_items` — the items of a set, snapshotted (FEN + solution line) so a set never changes
//!   under the user, plus the first-attempt result of each item.
//!
//! The store only persists; choosing the themes and the puzzles lives in the server
//! (`gm-server/src/routes/weekly`). It also exposes the raw inputs that ranking needs
//! ([`Store::weekly_puzzle_log`], [`Store::weekly_mistake_positions`]).
//!
//! Tables are created by [`schema`], which runs inside schema migration v3 (see `lib.rs`).

use anyhow::Context;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

/// Most items accepted in one set.
pub const MAX_ITEMS: usize = 30;
/// Sets kept in the database (older ones are pruned when a new one is created).
pub const MAX_SETS: i64 = 120;
/// Max rows returned by [`Store::weekly_puzzle_log`] / [`Store::weekly_mistake_positions`].
pub const MAX_LOG: u32 = 5_000;
/// Longest accepted week key (`YYYY-Www`).
const MAX_WEEK_LEN: usize = 8;
const MAX_TEXT: usize = 200;
const MAX_FEN: usize = 120;
const MAX_MOVES: usize = 32;

/// One item to store in a new set.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NewWeeklyItem {
    /// "puzzle" (pack puzzle) | "mistake" (position from the user's own game).
    pub kind: String,
    /// Pack puzzle id, or the mistake card id as text.
    pub ref_id: String,
    /// The focus theme this item trains ("" when it has none).
    pub theme: String,
    /// Puzzle rating (0 for own-game positions).
    pub rating: u32,
    /// Start FEN for the solver.
    pub fen: String,
    /// UCI moves. Pack puzzles start with the opponent's set-up move unless `user_first`.
    pub moves: Vec<String>,
    pub user_first: bool,
    /// Free-form context for the UI (own-game items: opponent, move number, played / best SAN).
    pub meta: serde_json::Value,
}

/// A stored item with its result.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WeeklyItem {
    pub index: u32,
    pub kind: String,
    pub ref_id: String,
    pub theme: String,
    pub rating: u32,
    pub fen: String,
    pub moves: Vec<String>,
    pub user_first: bool,
    pub meta: serde_json::Value,
    /// `None` until attempted; then whether the first attempt was clean.
    pub solved: Option<bool>,
    pub time_ms: Option<u64>,
    pub attempted_at: Option<String>,
}

/// A stored set (header + items).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WeeklySet {
    pub id: i64,
    /// ISO week key, e.g. `2026-W41`.
    pub week: String,
    /// 1 for the first set of the week, +1 for each "New set".
    pub generation: u32,
    /// Puzzle rating the set was built around.
    pub rating: u32,
    /// Why the themes were chosen (opaque JSON written by the server).
    pub focus: serde_json::Value,
    pub created_at: String,
    pub items: Vec<WeeklyItem>,
}

/// Counters of one set.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeeklyProgress {
    pub total: u32,
    pub done: u32,
    pub solved: u32,
}

impl WeeklySet {
    pub fn progress(&self) -> WeeklyProgress {
        let done = self.items.iter().filter(|i| i.solved.is_some()).count() as u32;
        let solved = self.items.iter().filter(|i| i.solved == Some(true)).count() as u32;
        WeeklyProgress { total: self.items.len() as u32, done, solved }
    }
}

/// Result of recording an attempt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WeeklyAttempt {
    pub item: WeeklyItem,
    /// False when the item already had a result (only the first attempt counts).
    pub counted: bool,
    pub progress: WeeklyProgress,
}

/// Attempts and solves of one theme in one week (all sets of that week).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeekThemeStat {
    pub week: String,
    pub theme: String,
    pub attempted: u32,
    pub solved: u32,
}

/// One rated puzzle attempt (input for theme ranking).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PuzzleLogEntry {
    pub puzzle_id: String,
    pub solved: bool,
    /// Days since the attempt (>= 0).
    pub age_days: f64,
}

/// A live mistake card, reduced to what the weekly set needs.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MistakePosition {
    pub id: i64,
    pub fen: String,
    pub prev_fen: Option<String>,
    pub prev_uci: Option<String>,
    pub played_uci: String,
    pub played_san: String,
    pub best_uci: String,
    pub best_san: String,
    pub solution: Vec<String>,
    pub game_id: Option<i64>,
    pub move_number: u32,
    pub opponent: String,
    pub classification: String,
    pub win_chance_loss: f32,
    pub graduated: bool,
    /// Days since the card was created (>= 0).
    pub age_days: f64,
}

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS weekly_sets (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  week TEXT NOT NULL,
  generation INTEGER NOT NULL DEFAULT 1,
  rating INTEGER NOT NULL DEFAULT 0,
  focus_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS weekly_sets_week ON weekly_sets (week, id);
CREATE TABLE IF NOT EXISTS weekly_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  set_id INTEGER NOT NULL REFERENCES weekly_sets(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL,
  kind TEXT NOT NULL DEFAULT 'puzzle',
  ref_id TEXT NOT NULL DEFAULT '',
  theme TEXT NOT NULL DEFAULT '',
  rating INTEGER NOT NULL DEFAULT 0,
  fen TEXT NOT NULL,
  moves TEXT NOT NULL DEFAULT '[]',
  user_first INTEGER NOT NULL DEFAULT 0,
  meta_json TEXT NOT NULL DEFAULT '{}',
  solved INTEGER,
  time_ms INTEGER,
  attempted_at TEXT,
  UNIQUE (set_id, idx)
);
"#,
    )
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn u32_of(v: i64) -> u32 {
    v.clamp(0, u32::MAX as i64) as u32
}

/// `YYYY-Www` with a plausible year and week.
pub fn valid_week(week: &str) -> bool {
    let b = week.as_bytes();
    if b.len() != MAX_WEEK_LEN || b[4] != b'-' || b[5] != b'W' {
        return false;
    }
    if !b[..4].iter().chain(&b[6..]).all(u8::is_ascii_digit) {
        return false;
    }
    let w: u32 = week[6..].parse().unwrap_or(0);
    (1..=53).contains(&w)
}

const SET_COLS: &str = "id, week, generation, rating, focus_json, created_at";
const ITEM_COLS: &str =
    "idx, kind, ref_id, theme, rating, fen, moves, user_first, meta_json, solved, time_ms, attempted_at";

fn row_to_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<WeeklyItem> {
    let moves: String = r.get(6)?;
    let meta: String = r.get(8)?;
    Ok(WeeklyItem {
        index: u32_of(r.get(0)?),
        kind: r.get(1)?,
        ref_id: r.get(2)?,
        theme: r.get(3)?,
        rating: u32_of(r.get(4)?),
        fen: r.get(5)?,
        moves: serde_json::from_str(&moves).unwrap_or_default(),
        user_first: r.get::<_, i64>(7)? != 0,
        meta: serde_json::from_str(&meta).unwrap_or(serde_json::Value::Null),
        solved: r.get::<_, Option<i64>>(9)?.map(|v| v != 0),
        time_ms: r.get::<_, Option<i64>>(10)?.map(|v| v.max(0) as u64),
        attempted_at: r.get(11)?,
    })
}

fn set_by_id(conn: &rusqlite::Connection, id: i64) -> anyhow::Result<Option<WeeklySet>> {
    let head = conn
        .query_row(&format!("SELECT {SET_COLS} FROM weekly_sets WHERE id = ?1"), params![id], |r| {
            let focus: String = r.get(4)?;
            Ok(WeeklySet {
                id: r.get(0)?,
                week: r.get(1)?,
                generation: u32_of(r.get(2)?),
                rating: u32_of(r.get(3)?),
                focus: serde_json::from_str(&focus).unwrap_or(serde_json::Value::Null),
                created_at: r.get(5)?,
                items: Vec::new(),
            })
        })
        .optional()
        .context("reading weekly set")?;
    let Some(mut set) = head else { return Ok(None) };
    let mut stmt = conn.prepare(&format!("SELECT {ITEM_COLS} FROM weekly_items WHERE set_id = ?1 ORDER BY idx"))?;
    set.items = stmt.query_map(params![id], row_to_item)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(set))
}

impl Store {
    /// The current (newest) set of `week`, if one was generated.
    pub fn weekly_current(&self, week: &str) -> anyhow::Result<Option<WeeklySet>> {
        let conn = self.conn.lock();
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM weekly_sets WHERE week = ?1 ORDER BY id DESC LIMIT 1",
                params![week],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => set_by_id(&conn, id),
            None => Ok(None),
        }
    }

    /// A set by id.
    pub fn weekly_set(&self, id: i64) -> anyhow::Result<Option<WeeklySet>> {
        let conn = self.conn.lock();
        set_by_id(&conn, id)
    }

    /// Stores a new set for `week` (it becomes the week's current set) and prunes old sets.
    pub fn weekly_create(
        &self,
        week: &str,
        rating: u32,
        focus: &serde_json::Value,
        items: &[NewWeeklyItem],
    ) -> anyhow::Result<WeeklySet> {
        if !valid_week(week) {
            anyhow::bail!("invalid week key");
        }
        let focus_json = serde_json::to_string(focus)?;
        if focus_json.len() > 16 * 1024 {
            anyhow::bail!("focus too large");
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let generation: i64 = tx.query_row(
            "SELECT COALESCE(MAX(generation), 0) + 1 FROM weekly_sets WHERE week = ?1",
            params![week],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO weekly_sets (week, generation, rating, focus_json) VALUES (?1, ?2, ?3, ?4)",
            params![week, generation, rating.min(10_000), focus_json],
        )?;
        let set_id = tx.last_insert_rowid();
        {
            let mut ins = tx.prepare(
                "INSERT INTO weekly_items (set_id, idx, kind, ref_id, theme, rating, fen, moves, user_first, meta_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for (i, it) in items.iter().take(MAX_ITEMS).enumerate() {
                let kind = if it.kind == "mistake" { "mistake" } else { "puzzle" };
                let moves: Vec<String> = it.moves.iter().take(MAX_MOVES).map(|m| clip(m, 6)).collect();
                let meta = serde_json::to_string(&it.meta).unwrap_or_else(|_| "{}".into());
                ins.execute(params![
                    set_id,
                    i as i64,
                    kind,
                    clip(&it.ref_id, 64),
                    clip(&it.theme, 40),
                    it.rating.min(10_000),
                    clip(&it.fen, MAX_FEN),
                    serde_json::to_string(&moves)?,
                    it.user_first as i64,
                    clip(&meta, MAX_TEXT * 8),
                ])?;
            }
        }
        // Keep the newest MAX_SETS sets.
        tx.execute(
            "DELETE FROM weekly_items WHERE set_id <= (SELECT MAX(id) FROM weekly_sets) - ?1",
            params![MAX_SETS],
        )?;
        tx.execute("DELETE FROM weekly_sets WHERE id <= (SELECT MAX(id) FROM weekly_sets) - ?1", params![MAX_SETS])?;
        tx.commit()?;
        drop(conn);
        self.weekly_set(set_id)?.context("weekly set vanished")
    }

    /// Records the result of item `index` of set `set_id`. Only the first attempt counts.
    /// `None` when the set or item does not exist.
    pub fn weekly_record(
        &self,
        set_id: i64,
        index: u32,
        solved: bool,
        time_ms: u64,
    ) -> anyhow::Result<Option<WeeklyAttempt>> {
        let time_ms = time_ms.min(24 * 3600 * 1000) as i64;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE weekly_items SET solved = ?3, time_ms = ?4, \
               attempted_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') \
             WHERE set_id = ?1 AND idx = ?2 AND solved IS NULL",
            params![set_id, index, solved as i64, time_ms],
        )?;
        tx.commit()?;
        let Some(set) = set_by_id(&conn, set_id)? else { return Ok(None) };
        let Some(item) = set.items.iter().find(|i| i.index == index).cloned() else { return Ok(None) };
        Ok(Some(WeeklyAttempt { item, counted: changed > 0, progress: set.progress() }))
    }

    /// Per-week, per-theme attempt counters for the given weeks (all sets of each week).
    pub fn weekly_theme_stats(&self, weeks: &[String]) -> anyhow::Result<Vec<WeekThemeStat>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT s.week, i.theme, COUNT(i.solved), COALESCE(SUM(i.solved), 0) \
             FROM weekly_items i JOIN weekly_sets s ON s.id = i.set_id \
             WHERE s.week = ?1 AND i.theme <> '' AND i.solved IS NOT NULL \
             GROUP BY i.theme ORDER BY i.theme",
        )?;
        let mut out = Vec::new();
        for w in weeks.iter().take(60) {
            let rows = stmt.query_map(params![w], |r| {
                Ok(WeekThemeStat {
                    week: r.get(0)?,
                    theme: r.get(1)?,
                    attempted: u32_of(r.get(2)?),
                    solved: u32_of(r.get(3)?),
                })
            })?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok(out)
    }

    /// Progress of the current set of `week` (`None` when no set was generated).
    pub fn weekly_progress(&self, week: &str) -> anyhow::Result<Option<WeeklyProgress>> {
        let conn = self.conn.lock();
        let id: Option<i64> = conn
            .query_row(
                "SELECT id FROM weekly_sets WHERE week = ?1 ORDER BY id DESC LIMIT 1",
                params![week],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        let (total, done, solved): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COUNT(solved), COALESCE(SUM(solved), 0) FROM weekly_items WHERE set_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(Some(WeeklyProgress { total: u32_of(total), done: u32_of(done), solved: u32_of(solved) }))
    }

    /// Rated puzzle attempts of the last `days` days, newest first (bounded).
    pub fn weekly_puzzle_log(&self, days: u32, limit: u32) -> anyhow::Result<Vec<PuzzleLogEntry>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT puzzle_id, solved, MAX(0.0, julianday('now') - julianday(created_at)) AS age \
             FROM puzzle_attempts WHERE julianday('now') - julianday(created_at) <= ?1 \
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![f64::from(days.min(3650)), limit.clamp(1, MAX_LOG)], |r| {
            Ok(PuzzleLogEntry {
                puzzle_id: r.get(0)?,
                solved: r.get::<_, i64>(1)? != 0,
                age_days: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Live (not removed) mistake cards, newest first (bounded).
    pub fn weekly_mistake_positions(&self, limit: u32) -> anyhow::Result<Vec<MistakePosition>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, fen, prev_fen, prev_uci, played_uci, played_san, best_uci, best_san, solution, game_id, \
               move_number, opponent, classification, win_chance_loss, graduated, \
               MAX(0.0, (CAST(strftime('%s','now') AS REAL) - created_at) / 86400.0) \
             FROM mistake_cards WHERE removed = 0 ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.clamp(1, MAX_LOG)], |r| {
            let solution: String = r.get(8)?;
            Ok(MistakePosition {
                id: r.get(0)?,
                fen: r.get(1)?,
                prev_fen: r.get(2)?,
                prev_uci: r.get(3)?,
                played_uci: r.get(4)?,
                played_san: r.get(5)?,
                best_uci: r.get(6)?,
                best_san: r.get(7)?,
                solution: serde_json::from_str(&solution).unwrap_or_default(),
                game_id: r.get(9)?,
                move_number: u32_of(r.get(10)?),
                opponent: r.get(11)?,
                classification: r.get(12)?,
                win_chance_loss: r.get::<_, f64>(13)? as f32,
                graduated: r.get::<_, i64>(14)? != 0,
                age_days: r.get::<_, Option<f64>>(15)?.unwrap_or(0.0),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str, theme: &str) -> NewWeeklyItem {
        NewWeeklyItem {
            kind: "puzzle".into(),
            ref_id: id.into(),
            theme: theme.into(),
            rating: 1200,
            fen: "8/8/8/8/8/8/8/K6k w - - 0 1".into(),
            moves: vec!["a1a2".into(), "h1h2".into()],
            ..Default::default()
        }
    }

    #[test]
    fn week_keys() {
        assert!(valid_week("2026-W41"));
        assert!(valid_week("2020-W53"));
        for bad in ["", "2026-W00", "2026-W54", "2026W41", "2026-w41", "abcd-W01", "2026-W411"] {
            assert!(!valid_week(bad), "{bad}");
        }
    }

    #[test]
    fn create_record_and_stats() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.weekly_current("2026-W41").unwrap().is_none());
        assert!(s.weekly_create("bad", 1200, &json!([]), &[]).is_err());

        let focus = json!([{ "theme": "fork" }]);
        let set = s
            .weekly_create("2026-W41", 1250, &focus, &[item("a", "fork"), item("b", "pin"), item("c", "fork")])
            .unwrap();
        assert_eq!((set.generation, set.rating, set.items.len()), (1, 1250, 3));
        assert_eq!(set.focus, focus);
        assert_eq!(set.items[1].ref_id, "b");
        assert_eq!(set.items[0].moves, vec!["a1a2", "h1h2"]);
        assert_eq!(set.progress(), WeeklyProgress { total: 3, done: 0, solved: 0 });

        let a = s.weekly_record(set.id, 0, true, 1000).unwrap().unwrap();
        assert!(a.counted);
        assert_eq!(a.item.solved, Some(true));
        // Only the first attempt counts.
        let again = s.weekly_record(set.id, 0, false, 1000).unwrap().unwrap();
        assert!(!again.counted);
        assert_eq!(again.item.solved, Some(true));
        let b = s.weekly_record(set.id, 2, false, 99_999_999_999).unwrap().unwrap();
        assert_eq!(b.progress, WeeklyProgress { total: 3, done: 2, solved: 1 });
        assert_eq!(b.item.time_ms, Some(24 * 3600 * 1000));
        assert!(s.weekly_record(set.id, 7, true, 0).unwrap().is_none());
        assert!(s.weekly_record(set.id + 100, 0, true, 0).unwrap().is_none());

        // A new set for the same week becomes current; the old results stay in the stats.
        let set2 = s.weekly_create("2026-W41", 1260, &focus, &[item("d", "fork")]).unwrap();
        assert_eq!(set2.generation, 2);
        assert_eq!(s.weekly_current("2026-W41").unwrap().unwrap().id, set2.id);
        s.weekly_record(set2.id, 0, true, 10).unwrap();
        let stats = s.weekly_theme_stats(&["2026-W40".into(), "2026-W41".into()]).unwrap();
        assert_eq!(stats, vec![WeekThemeStat { week: "2026-W41".into(), theme: "fork".into(), attempted: 3, solved: 2 }]);
        assert_eq!(s.weekly_progress("2026-W41").unwrap(), Some(WeeklyProgress { total: 1, done: 1, solved: 1 }));
        assert_eq!(s.weekly_progress("2026-W40").unwrap(), None);
    }

    #[test]
    fn bounded() {
        let s = Store::open_in_memory().unwrap();
        let many: Vec<NewWeeklyItem> = (0..MAX_ITEMS + 5).map(|i| item(&format!("p{i}"), "fork")).collect();
        let set = s.weekly_create("2026-W01", 1200, &json!([]), &many).unwrap();
        assert_eq!(set.items.len(), MAX_ITEMS);
        for _ in 0..(MAX_SETS + 3) {
            s.weekly_create("2026-W02", 1200, &json!([]), &[item("x", "pin")]).unwrap();
        }
        assert!(s.weekly_set(set.id).unwrap().is_none(), "oldest set pruned");
        let conn = s.conn.lock();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM weekly_sets", [], |r| r.get(0)).unwrap();
        assert_eq!(n, MAX_SETS);
    }

    #[test]
    fn inputs() {
        let s = Store::open_in_memory().unwrap();
        s.record_puzzle_attempt("p1", 1200, true, 100).unwrap();
        s.record_puzzle_attempt("p2", 1200, false, 100).unwrap();
        let log = s.weekly_puzzle_log(30, 100).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].puzzle_id, "p2");
        assert!(!log[0].solved);
        assert!(log[0].age_days < 1.0);
        assert!(s.weekly_mistake_positions(10).unwrap().is_empty());
    }
}
