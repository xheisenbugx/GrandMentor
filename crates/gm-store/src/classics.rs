//! Progress through the annotated classic games library: how far the user got in each game,
//! whether they finished it, and their answers to the "pause and think" questions.
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).

use anyhow::{bail, Context};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

/// Max length of a classic game id.
const MAX_ID: usize = 80;
/// Highest ply / question ply accepted (content games are far shorter).
const MAX_PLY: u32 = 1000;
/// Max stored answers per game.
const MAX_ANSWERS: usize = 16;

/// Creates this module's tables. Must be idempotent (`CREATE TABLE IF NOT EXISTS ...`).
pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS classic_progress (
  classic_id TEXT PRIMARY KEY,
  last_ply INTEGER NOT NULL DEFAULT 0,
  max_ply INTEGER NOT NULL DEFAULT 0,
  completed INTEGER NOT NULL DEFAULT 0,
  completed_at TEXT,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS classic_answers (
  classic_id TEXT NOT NULL,
  ply INTEGER NOT NULL,
  correct INTEGER NOT NULL,
  answered_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
  PRIMARY KEY (classic_id, ply)
) WITHOUT ROWID;
"#,
    )
}

/// One "pause and think" answer (the first attempt is kept).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClassicAnswer {
    pub ply: u32,
    pub correct: bool,
}

/// Progress through one classic game.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClassicProgress {
    pub classic_id: String,
    /// Ply the user was last looking at (resume point).
    pub last_ply: u32,
    /// Furthest ply reached.
    pub max_ply: u32,
    pub completed: bool,
    pub completed_at: Option<String>,
    pub answers: Vec<ClassicAnswer>,
    pub updated_at: String,
}

/// A progress update; every field is optional.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClassicProgressUpdate {
    /// Current ply (sets `last_ply`, raises `max_ply`).
    pub ply: Option<u32>,
    /// `true` marks the game as read (never un-marks it).
    pub completed: Option<bool>,
    /// A question answer; only the first answer per question ply is stored.
    pub answer: Option<ClassicAnswer>,
}

fn check_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() || id.len() > MAX_ID {
        bail!("invalid classic id");
    }
    Ok(())
}

impl Store {
    /// Progress for every classic game the user has opened (ordered by id).
    pub fn classic_progress_all(&self) -> anyhow::Result<Vec<ClassicProgress>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT classic_id, last_ply, max_ply, completed, completed_at, updated_at FROM classic_progress ORDER BY classic_id",
        )?;
        let mut out: Vec<ClassicProgress> = stmt
            .query_map([], |r| {
                Ok(ClassicProgress {
                    classic_id: r.get(0)?,
                    last_ply: r.get::<_, i64>(1)?.clamp(0, MAX_PLY as i64) as u32,
                    max_ply: r.get::<_, i64>(2)?.clamp(0, MAX_PLY as i64) as u32,
                    completed: r.get::<_, i64>(3)? != 0,
                    completed_at: r.get(4)?,
                    answers: Vec::new(),
                    updated_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = conn.prepare_cached("SELECT classic_id, ply, correct FROM classic_answers ORDER BY classic_id, ply")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        for row in rows {
            let (id, ply, correct) = row?;
            if let Some(p) = out.iter_mut().find(|p| p.classic_id == id) {
                p.answers.push(ClassicAnswer { ply: ply.clamp(0, MAX_PLY as i64) as u32, correct: correct != 0 });
            }
        }
        Ok(out)
    }

    /// Progress for one game (`None` if never opened).
    pub fn classic_progress(&self, id: &str) -> anyhow::Result<Option<ClassicProgress>> {
        check_id(id)?;
        let conn = self.conn.lock();
        let row = conn
            .prepare_cached(
                "SELECT last_ply, max_ply, completed, completed_at, updated_at FROM classic_progress WHERE classic_id = ?1",
            )?
            .query_row(params![id], |r| {
                Ok(ClassicProgress {
                    classic_id: id.to_string(),
                    last_ply: r.get::<_, i64>(0)?.clamp(0, MAX_PLY as i64) as u32,
                    max_ply: r.get::<_, i64>(1)?.clamp(0, MAX_PLY as i64) as u32,
                    completed: r.get::<_, i64>(2)? != 0,
                    completed_at: r.get(3)?,
                    answers: Vec::new(),
                    updated_at: r.get(4)?,
                })
            })
            .optional()?;
        let Some(mut p) = row else { return Ok(None) };
        let mut stmt = conn.prepare_cached("SELECT ply, correct FROM classic_answers WHERE classic_id = ?1 ORDER BY ply")?;
        p.answers = stmt
            .query_map(params![id], |r| {
                Ok(ClassicAnswer { ply: r.get::<_, i64>(0)?.clamp(0, MAX_PLY as i64) as u32, correct: r.get::<_, i64>(1)? != 0 })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(p))
    }

    /// Record progress. The caller validates `id` and the plies against the content.
    /// Returns the new progress and whether this update completed the game for the first time.
    pub fn update_classic_progress(&self, id: &str, up: &ClassicProgressUpdate) -> anyhow::Result<(ClassicProgress, bool)> {
        check_id(id)?;
        if up.ply.is_some_and(|p| p > MAX_PLY) || up.answer.as_ref().is_some_and(|a| a.ply == 0 || a.ply > MAX_PLY) {
            bail!("ply out of range");
        }
        let newly_completed = {
            let mut conn = self.conn.lock();
            let tx = conn.transaction().context("starting transaction")?;
            let was_completed: bool = tx
                .query_row("SELECT completed FROM classic_progress WHERE classic_id = ?1", params![id], |r| r.get::<_, i64>(0))
                .optional()?
                .is_some_and(|c| c != 0);
            tx.execute("INSERT OR IGNORE INTO classic_progress (classic_id) VALUES (?1)", params![id])?;
            if let Some(ply) = up.ply {
                tx.execute(
                    "UPDATE classic_progress SET last_ply = ?2, max_ply = MAX(max_ply, ?2) WHERE classic_id = ?1",
                    params![id, ply],
                )?;
            }
            let complete_now = up.completed == Some(true) && !was_completed;
            if complete_now {
                tx.execute(
                    "UPDATE classic_progress SET completed = 1, completed_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE classic_id = ?1",
                    params![id],
                )?;
            }
            if let Some(a) = &up.answer {
                let n: i64 = tx.query_row("SELECT COUNT(*) FROM classic_answers WHERE classic_id = ?1", params![id], |r| r.get(0))?;
                if (n as usize) < MAX_ANSWERS {
                    tx.execute(
                        "INSERT OR IGNORE INTO classic_answers (classic_id, ply, correct) VALUES (?1, ?2, ?3)",
                        params![id, a.ply, a.correct as i64],
                    )?;
                }
            }
            tx.execute(
                "UPDATE classic_progress SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE classic_id = ?1",
                params![id],
            )?;
            tx.commit().context("saving classic progress")?;
            complete_now
        };
        let p = self.classic_progress(id)?.context("progress row missing after update")?;
        Ok((p, newly_completed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_progress_and_answers() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.classic_progress("opera-game").unwrap().is_none());
        assert!(s.classic_progress_all().unwrap().is_empty());

        let (p, done) = s.update_classic_progress("opera-game", &ClassicProgressUpdate { ply: Some(12), ..Default::default() }).unwrap();
        assert!(!done);
        assert_eq!((p.last_ply, p.max_ply, p.completed), (12, 12, false));

        let (p, _) = s.update_classic_progress("opera-game", &ClassicProgressUpdate { ply: Some(4), ..Default::default() }).unwrap();
        assert_eq!((p.last_ply, p.max_ply), (4, 12));

        let ans = |ply, correct| ClassicProgressUpdate { answer: Some(ClassicAnswer { ply, correct }), ..Default::default() };
        s.update_classic_progress("opera-game", &ans(31, true)).unwrap();
        let (p, _) = s.update_classic_progress("opera-game", &ans(31, false)).unwrap(); // first answer kept
        assert_eq!(p.answers, vec![ClassicAnswer { ply: 31, correct: true }]);

        let fin = ClassicProgressUpdate { ply: Some(33), completed: Some(true), ..Default::default() };
        let (p, done) = s.update_classic_progress("opera-game", &fin).unwrap();
        assert!(done && p.completed && p.completed_at.is_some());
        let (_, done) = s.update_classic_progress("opera-game", &fin).unwrap();
        assert!(!done, "completion is only reported once");
        let (p, _) = s.update_classic_progress("opera-game", &ClassicProgressUpdate { completed: Some(false), ..Default::default() }).unwrap();
        assert!(p.completed, "completed is never cleared");

        s.update_classic_progress("immortal-game", &ans(5, false)).unwrap();
        let all = s.classic_progress_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].classic_id, "immortal-game");
        assert_eq!(all[0].answers.len(), 1);
        assert_eq!(all[1].answers.len(), 1);
    }

    #[test]
    fn rejects_bad_input() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.update_classic_progress("", &ClassicProgressUpdate::default()).is_err());
        assert!(s.update_classic_progress(&"x".repeat(200), &ClassicProgressUpdate::default()).is_err());
        let bad = ClassicProgressUpdate { ply: Some(5000), ..Default::default() };
        assert!(s.update_classic_progress("a", &bad).is_err());
        let bad = ClassicProgressUpdate { answer: Some(ClassicAnswer { ply: 0, correct: true }), ..Default::default() };
        assert!(s.update_classic_progress("a", &bad).is_err());
    }
}
