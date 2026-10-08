//! Endgame-theory practice results (how reliably the user converts / holds each drill).
//!
//! One row per drill with attempt/success counters and the current success streak. A drill is
//! "mastered" once it has been solved [`MASTERY_STREAK`] times in a row; mastery is sticky (a later
//! failure resets the streak but keeps the badge).
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).

use anyhow::Context;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

/// Successes in a row needed to master a drill.
pub const MASTERY_STREAK: u32 = 3;
/// Longest accepted drill id.
pub const MAX_DRILL_ID_LEN: usize = 64;
/// Upper bound for the reported move count of one attempt.
pub const MAX_ATTEMPT_MOVES: u32 = 500;

/// Practice progress for one drill.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DrillProgress {
    pub drill_id: String,
    pub attempts: u32,
    pub successes: u32,
    /// Current run of consecutive successes.
    pub streak: u32,
    pub best_streak: u32,
    /// Fewest moves in a successful attempt.
    pub best_moves: Option<u32>,
    pub mastered: bool,
    /// When the drill was first mastered (UTC, `YYYY-MM-DDTHH:MM:SSZ`).
    pub mastered_at: Option<String>,
    /// Last attempt (UTC).
    pub last_at: Option<String>,
    pub last_success: Option<bool>,
}

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS endgame_training (
  drill_id TEXT PRIMARY KEY,
  attempts INTEGER NOT NULL DEFAULT 0,
  successes INTEGER NOT NULL DEFAULT 0,
  streak INTEGER NOT NULL DEFAULT 0,
  best_streak INTEGER NOT NULL DEFAULT 0,
  best_moves INTEGER,
  mastered_at TEXT,
  last_at TEXT,
  last_success INTEGER
) WITHOUT ROWID;
"#,
    )
}

const COLUMNS: &str =
    "drill_id, attempts, successes, streak, best_streak, best_moves, mastered_at, last_at, last_success";

fn to_u32(v: i64) -> u32 {
    v.clamp(0, u32::MAX as i64) as u32
}

fn row_to_progress(r: &rusqlite::Row<'_>) -> rusqlite::Result<DrillProgress> {
    let mastered_at: Option<String> = r.get(6)?;
    Ok(DrillProgress {
        drill_id: r.get(0)?,
        attempts: to_u32(r.get(1)?),
        successes: to_u32(r.get(2)?),
        streak: to_u32(r.get(3)?),
        best_streak: to_u32(r.get(4)?),
        best_moves: r.get::<_, Option<i64>>(5)?.map(to_u32),
        mastered: mastered_at.is_some(),
        mastered_at,
        last_at: r.get(7)?,
        last_success: r.get::<_, Option<i64>>(8)?.map(|v| v != 0),
    })
}

fn valid_drill_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DRILL_ID_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Store {
    /// Progress for every drill the user has attempted (sorted by drill id).
    pub fn training_progress(&self) -> anyhow::Result<Vec<DrillProgress>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM endgame_training ORDER BY drill_id"))?;
        let rows = stmt.query_map([], row_to_progress)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Progress for one drill (`None` if never attempted).
    pub fn drill_progress(&self, drill_id: &str) -> anyhow::Result<Option<DrillProgress>> {
        let conn = self.conn.lock();
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM endgame_training WHERE drill_id = ?1"),
            params![drill_id],
            row_to_progress,
        )
        .optional()
        .context("reading drill progress")
    }

    /// Records one practice attempt and returns the updated progress. `drill_id` must be a short
    /// slug (the caller checks it against the content); `moves` is clamped.
    pub fn record_training_attempt(&self, drill_id: &str, success: bool, moves: u32) -> anyhow::Result<DrillProgress> {
        if !valid_drill_id(drill_id) {
            anyhow::bail!("invalid drill id");
        }
        let moves = moves.min(MAX_ATTEMPT_MOVES);
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO endgame_training (drill_id) VALUES (?1) ON CONFLICT(drill_id) DO NOTHING",
            params![drill_id],
        )?;
        if success {
            tx.execute(
                "UPDATE endgame_training SET \
                   attempts = attempts + 1, successes = successes + 1, streak = streak + 1, \
                   best_streak = MAX(best_streak, streak + 1), \
                   best_moves = CASE WHEN best_moves IS NULL OR ?2 < best_moves THEN ?2 ELSE best_moves END, \
                   mastered_at = CASE WHEN mastered_at IS NULL AND streak + 1 >= ?3 \
                                      THEN strftime('%Y-%m-%dT%H:%M:%SZ','now') ELSE mastered_at END, \
                   last_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'), last_success = 1 \
                 WHERE drill_id = ?1",
                params![drill_id, moves, MASTERY_STREAK],
            )?;
        } else {
            tx.execute(
                "UPDATE endgame_training SET attempts = attempts + 1, streak = 0, \
                   last_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'), last_success = 0 \
                 WHERE drill_id = ?1",
                params![drill_id],
            )?;
        }
        let p = tx.query_row(
            &format!("SELECT {COLUMNS} FROM endgame_training WHERE drill_id = ?1"),
            params![drill_id],
            row_to_progress,
        )?;
        tx.commit()?;
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaks_and_mastery() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.training_progress().unwrap().is_empty());
        assert!(s.drill_progress("lucena").unwrap().is_none());

        let p = s.record_training_attempt("lucena", true, 12).unwrap();
        assert_eq!((p.attempts, p.successes, p.streak, p.mastered), (1, 1, 1, false));
        assert_eq!(p.best_moves, Some(12));
        let p = s.record_training_attempt("lucena", false, 30).unwrap();
        assert_eq!((p.attempts, p.successes, p.streak, p.best_streak), (2, 1, 0, 1));
        assert_eq!(p.last_success, Some(false));
        assert_eq!(p.best_moves, Some(12), "failures never set best moves");

        for (i, moves) in [15, 9, 11].into_iter().enumerate() {
            let p = s.record_training_attempt("lucena", true, moves).unwrap();
            assert_eq!(p.streak, i as u32 + 1);
            assert_eq!(p.mastered, i == 2, "mastered after {MASTERY_STREAK} in a row");
        }
        let p = s.drill_progress("lucena").unwrap().unwrap();
        assert_eq!((p.attempts, p.successes, p.best_streak, p.best_moves), (5, 4, 3, Some(9)));
        let mastered_at = p.mastered_at.clone().expect("mastered_at");

        // Mastery is sticky: a failure resets the streak only.
        let p = s.record_training_attempt("lucena", false, 3).unwrap();
        assert_eq!(p.streak, 0);
        assert!(p.mastered);
        assert_eq!(p.mastered_at.as_deref(), Some(mastered_at.as_str()));

        s.record_training_attempt("philidor", true, 999_999).unwrap();
        let all = s.training_progress().unwrap();
        assert_eq!(all.iter().map(|p| p.drill_id.as_str()).collect::<Vec<_>>(), ["lucena", "philidor"]);
        assert_eq!(all[1].best_moves, Some(MAX_ATTEMPT_MOVES));
    }

    #[test]
    fn rejects_bad_ids() {
        let s = Store::open_in_memory().unwrap();
        for id in ["", "a b", "x'; DROP TABLE endgame_training; --", &"x".repeat(MAX_DRILL_ID_LEN + 1)] {
            assert!(s.record_training_attempt(id, true, 1).is_err(), "{id:?}");
        }
        assert!(s.training_progress().unwrap().is_empty());
    }
}
