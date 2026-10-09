//! Puzzle profile extras: Puzzle Rush personal bests per mode, and resetting puzzle stats.
//!
//! * `rush_bests` stores one row per **new personal best** (mode, score, created_at). The best
//!   for a mode is `MAX(score)`, so a backup merge (which adds rows by content) naturally keeps
//!   the best of both devices. Rows below a mode's best are pruned on every write.
//! * Modes are short ids (`3`, `5`, `survival`, ...); at most [`MAX_MODES`] distinct modes.
//!
//! Tables are created by [`schema`], which runs inside schema migration v3 (see `lib.rs`).

use std::collections::BTreeMap;

use anyhow::bail;
use rusqlite::{params, Connection};

use crate::Store;

/// Distinct Rush modes kept (bounds the table).
pub const MAX_MODES: usize = 16;
/// Longest accepted mode id.
pub const MAX_MODE_LEN: usize = 16;
/// Sanity cap on a submitted score.
pub const MAX_SCORE: u32 = 10_000;

/// Creates this module's tables. Idempotent.
pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS rush_bests (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  mode TEXT NOT NULL,
  score INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS idx_rush_bests_mode ON rush_bests(mode, score);
"#,
    )
}

/// Is `mode` a valid Rush mode id (`[a-z0-9_-]`, 1..=16 chars)?
pub fn valid_mode(mode: &str) -> bool {
    !mode.is_empty()
        && mode.len() <= MAX_MODE_LEN
        && mode
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn bests_in(conn: &Connection) -> rusqlite::Result<BTreeMap<String, u32>> {
    let mut stmt =
        conn.prepare_cached("SELECT mode, MAX(score) FROM rush_bests GROUP BY mode ORDER BY mode")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (mode, best) = row?;
        out.insert(mode, best.clamp(0, MAX_SCORE as i64) as u32);
    }
    Ok(out)
}

/// Records `score` for `mode` if it beats the stored best. Returns true when it did.
fn offer_in(conn: &Connection, mode: &str, score: u32) -> anyhow::Result<bool> {
    let score = score.min(MAX_SCORE);
    if score == 0 {
        return Ok(false);
    }
    let current = bests_in(conn)?;
    match current.get(mode) {
        Some(&best) if best >= score => return Ok(false),
        None if current.len() >= MAX_MODES => bail!("too many Puzzle Rush modes"),
        _ => {}
    }
    conn.prepare_cached("INSERT INTO rush_bests (mode, score) VALUES (?1, ?2)")?
        .execute(params![mode, score])?;
    conn.prepare_cached("DELETE FROM rush_bests WHERE mode = ?1 AND score < ?2")?
        .execute(params![mode, score])?;
    Ok(true)
}

impl Store {
    /// Puzzle Rush personal best per mode.
    pub fn rush_bests(&self) -> anyhow::Result<BTreeMap<String, u32>> {
        Ok(bests_in(&self.conn.lock())?)
    }

    /// Records a finished run's score for `mode`. Returns `(previous best, new best)`.
    pub fn record_rush_best(&self, mode: &str, score: u32) -> anyhow::Result<(u32, u32)> {
        if !valid_mode(mode) {
            bail!("invalid Puzzle Rush mode");
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let prev = bests_in(&tx)?.get(mode).copied().unwrap_or(0);
        offer_in(&tx, mode, score)?;
        tx.commit()?;
        Ok((prev, prev.max(score.min(MAX_SCORE))))
    }

    /// Merges bests kept elsewhere (e.g. the browser's old local copy): per mode, the higher
    /// score wins. Invalid modes are ignored. Returns every best afterwards.
    pub fn merge_rush_bests(&self, bests: &BTreeMap<String, u32>) -> anyhow::Result<BTreeMap<String, u32>> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        for (mode, &score) in bests.iter().filter(|(m, _)| valid_mode(m)).take(MAX_MODES) {
            // A full mode table just means this extra mode is not kept.
            let _ = offer_in(&tx, mode, score);
        }
        let out = bests_in(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Resets the puzzle rating to its starting value and clears puzzle stats: solved / failed
    /// counters, every puzzle attempt and the puzzle rating history. Rush scores, lessons,
    /// games and everything else are kept.
    pub fn reset_puzzle_stats(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM rating_history WHERE kind = 'puzzle'", [])?;
        tx.execute("DELETE FROM puzzle_attempts", [])?;
        tx.execute(
            "UPDATE profile SET puzzle_rating = ?1, puzzle_rd = ?2, puzzles_solved = 0, puzzles_failed = 0 \
             WHERE id = 1",
            params![crate::rating::START_RATING, crate::rating::START_RD],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bests_per_mode_take_the_max() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.rush_bests().unwrap().is_empty());
        assert_eq!(s.record_rush_best("3", 12).unwrap(), (0, 12));
        assert_eq!(s.record_rush_best("3", 7).unwrap(), (12, 12));
        assert_eq!(s.record_rush_best("3", 15).unwrap(), (12, 15));
        assert_eq!(s.record_rush_best("survival", 30).unwrap(), (0, 30));
        assert!(s.record_rush_best("Bad Mode!", 3).is_err());
        let merged = s
            .merge_rush_bests(&BTreeMap::from([("3".into(), 10), ("5".into(), 22), ("x y".into(), 99)]))
            .unwrap();
        assert_eq!(merged, BTreeMap::from([("3".into(), 15), ("5".into(), 22), ("survival".into(), 30)]));
        // Only the best row per mode survives pruning.
        let rows: i64 = s
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM rush_bests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 3);
        for i in 0..MAX_MODES {
            let _ = s.record_rush_best(&format!("m{i}"), 1);
        }
        assert_eq!(s.rush_bests().unwrap().len(), MAX_MODES);
    }

    #[test]
    fn bests_survive_backup_merge() {
        let a = Store::open_in_memory().unwrap();
        let b = Store::open_in_memory().unwrap();
        a.record_rush_best("3", 20).unwrap();
        a.record_rush_best("5", 5).unwrap();
        b.record_rush_best("5", 9).unwrap();
        let file = crate::backup::parse_backup(&a.export_backup(false).unwrap()).unwrap();
        b.import_backup(&file, crate::backup::ImportMode::Merge, crate::backup::ImportSource::Sync)
            .unwrap();
        assert_eq!(b.rush_bests().unwrap(), BTreeMap::from([("3".into(), 20), ("5".into(), 9)]));
    }

    #[test]
    fn reset_puzzle_stats_keeps_other_progress() {
        let s = Store::open_in_memory().unwrap();
        for i in 0..5 {
            s.record_puzzle_attempt(&format!("p{i}"), 1500, i % 2 == 0, 1000).unwrap();
        }
        s.record_rush(9).unwrap();
        s.record_rush_best("3", 9).unwrap();
        assert_ne!(s.get_profile().unwrap().puzzle_rating, 1200.0);
        s.reset_puzzle_stats().unwrap();
        let p = s.get_profile().unwrap();
        assert_eq!((p.puzzle_rating, p.puzzle_rd, p.puzzles_solved, p.puzzles_failed), (1200.0, 350.0, 0, 0));
        assert_eq!(p.rush_best, 9);
        let st = s.stats().unwrap();
        assert!(st.rating_history.is_empty());
        assert_eq!(s.rush_bests().unwrap().get("3"), Some(&9));
    }
}
