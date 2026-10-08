//! Quick-drill scores (coordinate vision, hanging pieces, counting material, checks, knight routes).
//!
//! * `drill_scores` keeps a bounded history ([`MAX_HISTORY`] runs per drill + variant).
//! * `drill_bests` keeps the personal best and play count, so trimming history never loses them.
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).

use anyhow::{bail, Context};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

/// Every drill and its variants (the first variant is the default). Scores are "higher is better".
pub const DRILLS: &[(&str, &[&str])] = &[
    ("coordinates", &["find-white", "find-black", "name-white", "name-black"]),
    ("hanging", &["standard"]),
    ("material", &["standard"]),
    ("checks", &["checks", "captures"]),
    ("knight", &["basic", "advanced"]),
];

/// Runs kept per drill + variant (older runs are deleted; the best survives in `drill_bests`).
pub const MAX_HISTORY: usize = 50;
/// Recent scores returned per variant by [`Store::drill_stats`].
pub const RECENT: usize = 10;
/// Sanity caps on submitted numbers.
pub const MAX_SCORE: u32 = 10_000;
pub const MAX_DURATION_MS: u32 = 60 * 60 * 1000;

/// One finished run, as submitted by the client.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct DrillRun {
    pub score: u32,
    /// Correct answers.
    pub correct: u32,
    /// Answers given (correct + wrong).
    pub total: u32,
    pub duration_ms: u32,
}

/// Result of [`Store::record_drill`].
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DrillRecordResult {
    pub score: u32,
    /// Personal best after this run.
    pub best: u32,
    /// Best before this run (None on the first run).
    pub previous_best: Option<u32>,
    /// True when this run set a new best (strictly higher than the previous one, or the first run).
    pub is_best: bool,
    /// Runs played for this drill + variant.
    pub plays: u32,
}

/// Stats for one variant of a drill.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DrillVariantStats {
    pub id: String,
    pub best: Option<u32>,
    pub best_at: Option<String>,
    pub plays: u32,
    /// Last scores, oldest first (at most [`RECENT`]).
    pub recent: Vec<u32>,
    /// Accuracy (0..100) of the last run, when it had answers.
    pub last_accuracy: Option<f32>,
}

/// Stats for one drill.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct DrillStats {
    pub id: String,
    pub variants: Vec<DrillVariantStats>,
}

/// True when `drill` / `variant` are known.
pub fn is_known(drill: &str, variant: &str) -> bool {
    DRILLS.iter().any(|(d, vs)| *d == drill && vs.contains(&variant))
}

/// Variants of `drill`, or None if the drill is unknown.
pub fn variants_of(drill: &str) -> Option<&'static [&'static str]> {
    DRILLS.iter().find(|(d, _)| *d == drill).map(|(_, vs)| *vs)
}

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS drill_scores (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  drill TEXT NOT NULL,
  variant TEXT NOT NULL,
  score INTEGER NOT NULL,
  correct INTEGER NOT NULL,
  total INTEGER NOT NULL,
  duration_ms INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX IF NOT EXISTS drill_scores_dv ON drill_scores(drill, variant, id);
CREATE TABLE IF NOT EXISTS drill_bests (
  drill TEXT NOT NULL,
  variant TEXT NOT NULL,
  best INTEGER NOT NULL,
  best_at TEXT NOT NULL,
  plays INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (drill, variant)
) WITHOUT ROWID;
"#,
    )
}

impl Store {
    /// Records a finished run and returns the (possibly new) personal best.
    pub fn record_drill(&self, drill: &str, variant: &str, run: &DrillRun) -> anyhow::Result<DrillRecordResult> {
        if !is_known(drill, variant) {
            bail!("unknown drill: {drill}/{variant}");
        }
        if run.score > MAX_SCORE || run.total > MAX_SCORE || run.correct > run.total {
            bail!("invalid drill score");
        }
        let duration = run.duration_ms.min(MAX_DURATION_MS);
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let prev: Option<(u32, u32)> = tx
            .query_row(
                "SELECT best, plays FROM drill_bests WHERE drill = ?1 AND variant = ?2",
                params![drill, variant],
                |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)),
            )
            .optional()?;
        tx.execute(
            "INSERT INTO drill_scores (drill, variant, score, correct, total, duration_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![drill, variant, run.score, run.correct, run.total, duration],
        )
        .context("saving drill score")?;
        tx.execute(
            "DELETE FROM drill_scores WHERE drill = ?1 AND variant = ?2 AND id NOT IN \
             (SELECT id FROM drill_scores WHERE drill = ?1 AND variant = ?2 ORDER BY id DESC LIMIT ?3)",
            params![drill, variant, MAX_HISTORY as i64],
        )?;
        let previous_best = prev.map(|p| p.0);
        let plays = prev.map(|p| p.1).unwrap_or(0).saturating_add(1);
        let is_best = previous_best.map(|b| run.score > b).unwrap_or(true);
        let best = previous_best.map(|b| b.max(run.score)).unwrap_or(run.score);
        if is_best {
            tx.execute(
                "INSERT INTO drill_bests (drill, variant, best, best_at, plays) \
                 VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ','now'), ?4) \
                 ON CONFLICT(drill, variant) DO UPDATE SET best = excluded.best, best_at = excluded.best_at, plays = excluded.plays",
                params![drill, variant, best, plays],
            )?;
        } else {
            tx.execute(
                "UPDATE drill_bests SET plays = ?3 WHERE drill = ?1 AND variant = ?2",
                params![drill, variant, plays],
            )?;
        }
        tx.commit()?;
        Ok(DrillRecordResult { score: run.score, best, previous_best, is_best, plays })
    }

    /// Bests, play counts and recent scores for every drill and variant (in [`DRILLS`] order).
    pub fn drill_stats(&self) -> anyhow::Result<Vec<DrillStats>> {
        let conn = self.conn.lock();
        let mut bests = conn.prepare_cached("SELECT best, best_at, plays FROM drill_bests WHERE drill = ?1 AND variant = ?2")?;
        let mut recent = conn.prepare_cached(
            "SELECT score, correct, total FROM drill_scores WHERE drill = ?1 AND variant = ?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let mut out = Vec::with_capacity(DRILLS.len());
        for (drill, variants) in DRILLS {
            let mut stats = DrillStats { id: (*drill).to_string(), variants: Vec::with_capacity(variants.len()) };
            for variant in *variants {
                let best: Option<(u32, String, u32)> = bests
                    .query_row(params![drill, variant], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .optional()?;
                let rows: Vec<(u32, u32, u32)> = recent
                    .query_map(params![drill, variant, RECENT as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<_, _>>()?;
                let last_accuracy = rows
                    .first()
                    .filter(|(_, _, total)| *total > 0)
                    .map(|(_, correct, total)| (*correct as f32 * 100.0 / *total as f32).clamp(0.0, 100.0));
                let mut recent_scores: Vec<u32> = rows.iter().map(|r| r.0).collect();
                recent_scores.reverse();
                stats.variants.push(DrillVariantStats {
                    id: (*variant).to_string(),
                    best: best.as_ref().map(|b| b.0),
                    best_at: best.as_ref().map(|b| b.1.clone()),
                    plays: best.as_ref().map(|b| b.2).unwrap_or(0),
                    recent: recent_scores,
                    last_accuracy,
                });
            }
            out.push(stats);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(score: u32) -> DrillRun {
        DrillRun { score, correct: score, total: score + 1, duration_ms: 30_000 }
    }

    #[test]
    fn records_bests_and_plays() {
        let s = Store::open_in_memory().unwrap();
        let r = s.record_drill("coordinates", "find-white", &run(10)).unwrap();
        assert!(r.is_best);
        assert_eq!((r.best, r.previous_best, r.plays), (10, None, 1));
        let r = s.record_drill("coordinates", "find-white", &run(7)).unwrap();
        assert!(!r.is_best);
        assert_eq!((r.best, r.previous_best, r.plays), (10, Some(10), 2));
        let r = s.record_drill("coordinates", "find-white", &run(12)).unwrap();
        assert!(r.is_best);
        assert_eq!((r.best, r.previous_best, r.plays), (12, Some(10), 3));
        // Equal to the best is not a new best.
        assert!(!s.record_drill("coordinates", "find-white", &run(12)).unwrap().is_best);

        let stats = s.drill_stats().unwrap();
        assert_eq!(stats.len(), DRILLS.len());
        let coords = &stats[0];
        assert_eq!(coords.id, "coordinates");
        let v = &coords.variants[0];
        assert_eq!((v.best, v.plays), (Some(12), 4));
        assert_eq!(v.recent, vec![10, 7, 12, 12]);
        assert!(v.last_accuracy.is_some());
        // Other variants are independent.
        assert_eq!(coords.variants[1].best, None);
        assert_eq!(coords.variants[1].plays, 0);
    }

    #[test]
    fn rejects_unknown_and_invalid() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.record_drill("nope", "standard", &run(1)).is_err());
        assert!(s.record_drill("hanging", "nope", &run(1)).is_err());
        let bad = DrillRun { score: 1, correct: 5, total: 2, duration_ms: 0 };
        assert!(s.record_drill("hanging", "standard", &bad).is_err());
        let huge = DrillRun { score: MAX_SCORE + 1, correct: 0, total: 0, duration_ms: 0 };
        assert!(s.record_drill("hanging", "standard", &huge).is_err());
    }

    #[test]
    fn history_is_bounded_but_best_survives() {
        let s = Store::open_in_memory().unwrap();
        s.record_drill("material", "standard", &run(99)).unwrap();
        for _ in 0..(MAX_HISTORY + 20) {
            s.record_drill("material", "standard", &run(1)).unwrap();
        }
        let n: i64 = s
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM drill_scores WHERE drill = 'material'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n as usize, MAX_HISTORY);
        let stats = s.drill_stats().unwrap();
        let v = &stats.iter().find(|d| d.id == "material").unwrap().variants[0];
        assert_eq!(v.best, Some(99));
        assert_eq!(v.plays as usize, MAX_HISTORY + 21);
        assert_eq!(v.recent.len(), RECENT);
    }
}
