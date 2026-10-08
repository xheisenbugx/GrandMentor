//! Spaced-repetition deck of positions taken from the user's own mistakes ("Learn from your mistakes").
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).
//!
//! Scheduling is a small Leitner/SM-2 hybrid:
//! * a new card is due immediately;
//! * a correct answer on a due card grows the interval (1 day, then ~3 days, ...) and bumps the
//!   streak; after [`GRADUATE_STREAK`] correct answers in a row the card graduates (is retired);
//! * a wrong answer resets the streak, lowers the ease and brings the card back in
//!   [`RELEARN_SECS`] seconds;
//! * answering a card early (before it is due) is practice: success leaves the schedule alone,
//!   failure still resets it.
//!
//! Positions are deduplicated by the first four FEN fields (placement, side, castling, en passant).
//! Removed cards are kept as tombstones so a re-scan never brings them back.

use anyhow::Context;
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::Store;

/// Max live (not removed) cards in the deck; ingestion stops adding beyond this.
pub const MAX_CARDS: i64 = 5_000;
/// Max cards returned by one list page.
pub const MAX_PAGE: u32 = 100;
/// Correct answers in a row (on due cards) needed to retire a card.
pub const GRADUATE_STREAK: u32 = 3;
/// A failed card comes back after this many seconds.
pub const RELEARN_SECS: i64 = 10 * 60;
/// Max plies kept in a card's solution line.
pub const MAX_SOLUTION_PLIES: usize = 9;
const DAY: f64 = 86_400.0;
const MIN_EASE: f64 = 1.3;
const MAX_EASE: f64 = 3.0;
const START_EASE: f64 = 2.5;
const MAX_TEXT: usize = 2_000;
const MAX_SHORT: usize = 100;

const CARD_COLS: &str = "id, fen, prev_fen, prev_uci, played_uci, played_san, best_uci, best_san, solution, \
    game_id, ply, move_number, color, classification, phase, opponent, bot_id, explanation, lang, \
    win_chance_loss, due_at, interval_days, ease, streak, reps, lapses, graduated, last_reviewed_at, created_at";

/// A card to add to the deck (built by the server from a game review).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NewMistake {
    /// Position before the user's move (full FEN, user to move).
    pub fen: String,
    /// Position before the opponent's previous move, when there is one (for the set-up animation).
    pub prev_fen: Option<String>,
    /// The opponent's previous move (UCI) leading from `prev_fen` to `fen`.
    pub prev_uci: Option<String>,
    pub played_uci: String,
    pub played_san: String,
    pub best_uci: String,
    pub best_san: String,
    /// UCI line starting with the best move; odd length (ends on a user move).
    pub solution: Vec<String>,
    pub game_id: Option<i64>,
    /// 1-based ply of the user's move in the game.
    pub ply: u32,
    pub move_number: u32,
    /// "white" | "black" — the user's side.
    pub color: String,
    /// "mistake" | "miss" | "blunder"
    pub classification: String,
    /// "opening" | "middlegame" | "endgame"
    pub phase: String,
    pub opponent: String,
    pub bot_id: Option<String>,
    /// Coach's explanation of the played move (in `lang`).
    pub explanation: String,
    pub lang: String,
    pub win_chance_loss: f32,
}

/// A card in the deck with its schedule.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MistakeCard {
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
    pub ply: u32,
    pub move_number: u32,
    pub color: String,
    pub classification: String,
    pub phase: String,
    pub opponent: String,
    pub bot_id: Option<String>,
    pub explanation: String,
    pub lang: String,
    pub win_chance_loss: f32,
    /// ISO-8601 UTC.
    pub due_at: String,
    /// Seconds until due (<= 0 when due).
    pub due_in_secs: i64,
    pub interval_days: f64,
    pub ease: f64,
    pub streak: u32,
    pub reps: u32,
    pub lapses: u32,
    pub graduated: bool,
    pub last_reviewed_at: Option<String>,
    pub created_at: String,
}

/// Deck counters.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct MistakeSummary {
    /// Cards due now.
    pub due: u32,
    /// Live cards (including graduated ones).
    pub total: u32,
    /// Retired cards.
    pub graduated: u32,
    /// Cards still being learned (`total - graduated`).
    pub learning: u32,
    /// ISO time of the next not-yet-due card, if any.
    pub next_due_at: Option<String>,
    /// Seconds until `next_due_at`.
    pub next_due_in_secs: Option<i64>,
}

/// Result of an attempt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AttemptOutcome {
    pub card: MistakeCard,
    /// False when the card was not due (early practice): a success then leaves the schedule alone.
    pub counted: bool,
    /// The card just graduated with this attempt.
    pub graduated_now: bool,
    pub summary: MistakeSummary,
}

/// Dedup key: placement, side to move, castling, en passant.
pub fn fen_key(fen: &str) -> String {
    fen.split_whitespace().take(4).collect::<Vec<_>>().join(" ")
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

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS mistake_cards (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  fen_key TEXT NOT NULL UNIQUE,
  fen TEXT NOT NULL,
  prev_fen TEXT, prev_uci TEXT,
  played_uci TEXT NOT NULL, played_san TEXT NOT NULL DEFAULT '',
  best_uci TEXT NOT NULL, best_san TEXT NOT NULL DEFAULT '',
  solution TEXT NOT NULL DEFAULT '[]',
  game_id INTEGER, ply INTEGER NOT NULL DEFAULT 0, move_number INTEGER NOT NULL DEFAULT 0,
  color TEXT NOT NULL DEFAULT 'white', classification TEXT NOT NULL DEFAULT 'mistake',
  phase TEXT NOT NULL DEFAULT 'middlegame', opponent TEXT NOT NULL DEFAULT '', bot_id TEXT,
  explanation TEXT NOT NULL DEFAULT '', lang TEXT NOT NULL DEFAULT 'en',
  win_chance_loss REAL NOT NULL DEFAULT 0,
  due_at INTEGER NOT NULL,
  interval_days REAL NOT NULL DEFAULT 0,
  ease REAL NOT NULL DEFAULT 2.5,
  streak INTEGER NOT NULL DEFAULT 0,
  reps INTEGER NOT NULL DEFAULT 0,
  lapses INTEGER NOT NULL DEFAULT 0,
  graduated INTEGER NOT NULL DEFAULT 0,
  removed INTEGER NOT NULL DEFAULT 0,
  last_reviewed_at INTEGER,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS mistake_cards_due ON mistake_cards (removed, graduated, due_at);
CREATE INDEX IF NOT EXISTS mistake_cards_game ON mistake_cards (game_id);
CREATE TABLE IF NOT EXISTS mistake_scanned_games (
  game_id INTEGER PRIMARY KEY,
  scanned_at INTEGER NOT NULL
);
"#,
    )
}

fn iso(secs: i64) -> String {
    // Civil-from-days (Howard Hinnant), UTC.
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn row_to_card(r: &Row<'_>, now: i64) -> rusqlite::Result<MistakeCard> {
    let solution: String = r.get(8)?;
    let due_at: i64 = r.get(20)?;
    let last: Option<i64> = r.get(27)?;
    let created: i64 = r.get(28)?;
    let u = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    Ok(MistakeCard {
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
        ply: u(r.get(10)?),
        move_number: u(r.get(11)?),
        color: r.get(12)?,
        classification: r.get(13)?,
        phase: r.get(14)?,
        opponent: r.get(15)?,
        bot_id: r.get(16)?,
        explanation: r.get(17)?,
        lang: r.get(18)?,
        win_chance_loss: r.get::<_, f64>(19)? as f32,
        due_at: iso(due_at),
        due_in_secs: due_at - now,
        interval_days: r.get(21)?,
        ease: r.get(22)?,
        streak: u(r.get(23)?),
        reps: u(r.get(24)?),
        lapses: u(r.get(25)?),
        graduated: r.get::<_, i64>(26)? != 0,
        last_reviewed_at: last.map(iso),
        created_at: iso(created),
    })
}

fn summary_in(conn: &rusqlite::Connection, now: i64) -> anyhow::Result<MistakeSummary> {
    let (total, graduated, due): (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(graduated), 0), \
         COALESCE(SUM(CASE WHEN graduated = 0 AND due_at <= ?1 THEN 1 ELSE 0 END), 0) \
         FROM mistake_cards WHERE removed = 0",
        params![now],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let next: Option<i64> = conn.query_row(
        "SELECT MIN(due_at) FROM mistake_cards WHERE removed = 0 AND graduated = 0 AND due_at > ?1",
        params![now],
        |r| r.get(0),
    )?;
    let u = |v: i64| v.clamp(0, u32::MAX as i64) as u32;
    Ok(MistakeSummary {
        due: u(due),
        total: u(total),
        graduated: u(graduated),
        learning: u(total - graduated),
        next_due_at: next.map(iso),
        next_due_in_secs: next.map(|n| n - now),
    })
}

fn get_card_in(conn: &rusqlite::Connection, id: i64, now: i64) -> anyhow::Result<Option<MistakeCard>> {
    let sql = format!("SELECT {CARD_COLS} FROM mistake_cards WHERE id = ?1 AND removed = 0");
    Ok(conn.query_row(&sql, params![id], |r| row_to_card(r, now)).optional()?)
}

/// New schedule after an attempt: (due_at, interval_days, ease, streak, graduated, counted).
fn schedule(card: &MistakeCard, solved: bool, time_ms: u64, now: i64) -> (i64, f64, f64, u32, bool, bool) {
    let is_due = card.due_in_secs <= 0;
    let ease = card.ease.clamp(MIN_EASE, MAX_EASE);
    if !solved {
        let ease = (ease - 0.2).max(MIN_EASE);
        return (now + RELEARN_SECS, 0.0, ease, 0, false, true);
    }
    if !is_due || card.graduated {
        // Early practice: keep the schedule.
        let due = now + card.due_in_secs.max(0);
        return (due, card.interval_days, ease, card.streak, card.graduated, false);
    }
    // Quick, confident answers make the next interval a little longer.
    let ease = if time_ms > 0 && time_ms <= 15_000 { (ease + 0.1).min(MAX_EASE) } else { ease };
    let streak = card.streak.saturating_add(1);
    let interval = match streak {
        1 => 1.0,
        2 => (card.interval_days.max(1.0) * ease).max(3.0),
        _ => (card.interval_days.max(3.0) * ease).max(7.0),
    }
    .min(365.0);
    let graduated = streak >= GRADUATE_STREAK;
    let due = now + (interval * DAY) as i64;
    (due, interval, ease, streak, graduated, true)
}

impl Store {
    /// Adds cards to the deck, skipping positions already present (including removed ones) and
    /// stopping at [`MAX_CARDS`]. Returns how many were added.
    pub fn add_mistakes(&self, cards: &[NewMistake]) -> anyhow::Result<usize> {
        self.add_mistakes_at(cards, now_secs())
    }

    pub fn add_mistakes_at(&self, cards: &[NewMistake], now: i64) -> anyhow::Result<usize> {
        if cards.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let mut live: i64 =
            tx.query_row("SELECT COUNT(*) FROM mistake_cards WHERE removed = 0", [], |r| r.get(0))?;
        let mut added = 0;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO mistake_cards (fen_key, fen, prev_fen, prev_uci, played_uci, played_san, \
                 best_uci, best_san, solution, game_id, ply, move_number, color, classification, phase, opponent, \
                 bot_id, explanation, lang, win_chance_loss, due_at, ease, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?21)",
            )?;
            for c in cards {
                if live >= MAX_CARDS {
                    break;
                }
                let key = fen_key(&c.fen);
                if key.is_empty() || c.best_uci.is_empty() || c.played_uci.is_empty() {
                    continue;
                }
                let mut solution: Vec<String> =
                    c.solution.iter().take(MAX_SOLUTION_PLIES).map(|m| clip(m, 6)).collect();
                if solution.is_empty() {
                    solution.push(clip(&c.best_uci, 6));
                }
                if solution.len().is_multiple_of(2) {
                    solution.pop();
                }
                let solution = serde_json::to_string(&solution).context("encoding solution")?;
                let n = stmt.execute(params![
                    key,
                    clip(&c.fen, 120),
                    c.prev_fen.as_deref().map(|s| clip(s, 120)),
                    c.prev_uci.as_deref().map(|s| clip(s, 6)),
                    clip(&c.played_uci, 6),
                    clip(&c.played_san, 16),
                    clip(&c.best_uci, 6),
                    clip(&c.best_san, 16),
                    solution,
                    c.game_id,
                    c.ply,
                    c.move_number,
                    clip(&c.color, 8),
                    clip(&c.classification, 16),
                    clip(&c.phase, 16),
                    clip(&c.opponent, MAX_SHORT),
                    c.bot_id.as_deref().map(|s| clip(s, MAX_SHORT)),
                    clip(&c.explanation, MAX_TEXT),
                    clip(&c.lang, 8),
                    f64::from(c.win_chance_loss),
                    now,
                    START_EASE,
                ])?;
                if n > 0 {
                    added += 1;
                    live += 1;
                }
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Deck counters.
    pub fn mistake_summary(&self) -> anyhow::Result<MistakeSummary> {
        self.mistake_summary_at(now_secs())
    }

    pub fn mistake_summary_at(&self, now: i64) -> anyhow::Result<MistakeSummary> {
        let conn = self.conn.lock();
        summary_in(&conn, now)
    }

    /// The most overdue card; if nothing is due, the soonest upcoming one (`due_in_secs > 0`).
    /// Graduated cards are never returned. `exclude` skips one card id (e.g. the one just answered).
    pub fn next_mistake(&self, exclude: Option<i64>) -> anyhow::Result<Option<MistakeCard>> {
        self.next_mistake_at(exclude, now_secs())
    }

    pub fn next_mistake_at(&self, exclude: Option<i64>, now: i64) -> anyhow::Result<Option<MistakeCard>> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT {CARD_COLS} FROM mistake_cards WHERE removed = 0 AND graduated = 0 AND id != ?1 \
             ORDER BY due_at, id LIMIT 1"
        );
        let found = conn
            .query_row(&sql, params![exclude.unwrap_or(-1)], |r| row_to_card(r, now))
            .optional()?;
        if found.is_some() || exclude.is_none() {
            return Ok(found);
        }
        // Only the excluded card is left: hand it back rather than nothing.
        get_card_in(&conn, exclude.unwrap_or(-1), now).map(|c| c.filter(|c| !c.graduated))
    }

    pub fn get_mistake(&self, id: i64) -> anyhow::Result<Option<MistakeCard>> {
        let conn = self.conn.lock();
        get_card_in(&conn, id, now_secs())
    }

    /// Live cards, newest first. `filter`: "due" | "learning" | "graduated" | anything else = all.
    pub fn list_mistakes(&self, filter: &str, limit: u32, offset: u32) -> anyhow::Result<Vec<MistakeCard>> {
        let now = now_secs();
        let cond = match filter {
            "due" => "AND graduated = 0 AND due_at <= ?3",
            "learning" => "AND graduated = 0 AND ?3 = ?3",
            "graduated" => "AND graduated = 1 AND ?3 = ?3",
            _ => "AND ?3 = ?3",
        };
        let sql = format!(
            "SELECT {CARD_COLS} FROM mistake_cards WHERE removed = 0 {cond} ORDER BY id DESC LIMIT ?1 OFFSET ?2"
        );
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![limit.clamp(1, MAX_PAGE), offset.min(MAX_CARDS as u32 * 2), now],
            |r| row_to_card(r, now),
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Records an attempt and reschedules the card. `None` if the card does not exist.
    pub fn attempt_mistake(&self, id: i64, solved: bool, time_ms: u64) -> anyhow::Result<Option<AttemptOutcome>> {
        self.attempt_mistake_at(id, solved, time_ms, now_secs())
    }

    pub fn attempt_mistake_at(
        &self,
        id: i64,
        solved: bool,
        time_ms: u64,
        now: i64,
    ) -> anyhow::Result<Option<AttemptOutcome>> {
        let conn = self.conn.lock();
        let Some(card) = get_card_in(&conn, id, now)? else {
            return Ok(None);
        };
        let was_graduated = card.graduated;
        let (due, interval, ease, streak, graduated, counted) = schedule(&card, solved, time_ms, now);
        conn.execute(
            "UPDATE mistake_cards SET due_at = ?2, interval_days = ?3, ease = ?4, streak = ?5, graduated = ?6, \
             reps = reps + 1, lapses = lapses + ?7, last_reviewed_at = ?8 WHERE id = ?1",
            params![id, due, interval, ease, streak, graduated as i64, (!solved) as i64, now],
        )?;
        let card = get_card_in(&conn, id, now)?.context("card vanished")?;
        let summary = summary_in(&conn, now)?;
        Ok(Some(AttemptOutcome {
            graduated_now: graduated && !was_graduated,
            card,
            counted,
            summary,
        }))
    }

    /// Removes a card from the deck (kept as a tombstone so it is never re-added). False if missing.
    pub fn remove_mistake(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.lock();
        let n = conn.execute("UPDATE mistake_cards SET removed = 1 WHERE id = ?1 AND removed = 0", params![id])?;
        Ok(n > 0)
    }

    /// Reviewed games with a known user side that have not been scanned yet (newest first).
    pub fn unscanned_reviewed_games(&self, limit: u32) -> anyhow::Result<Vec<i64>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT g.id FROM games g WHERE g.review_json IS NOT NULL AND g.user_color IN ('white','black') \
             AND NOT EXISTS (SELECT 1 FROM mistake_scanned_games s WHERE s.game_id = g.id) \
             ORDER BY g.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.clamp(1, 1000)], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Marks a game as scanned for mistakes (so the backfill skips it).
    pub fn mark_game_scanned(&self, game_id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO mistake_scanned_games (game_id, scanned_at) VALUES (?1, ?2)",
            params![game_id, now_secs()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(fen: &str) -> NewMistake {
        NewMistake {
            fen: fen.into(),
            played_uci: "e2e4".into(),
            played_san: "e4".into(),
            best_uci: "d2d4".into(),
            best_san: "d4".into(),
            solution: vec!["d2d4".into(), "d7d5".into()],
            game_id: Some(1),
            ply: 1,
            move_number: 1,
            color: "white".into(),
            classification: "blunder".into(),
            phase: "opening".into(),
            opponent: "Bot".into(),
            lang: "en".into(),
            ..Default::default()
        }
    }

    const F1: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
    const F2: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";

    #[test]
    fn iso_formats_epoch() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn dedupes_by_fen_and_trims_solution() {
        let s = Store::open_in_memory().unwrap();
        let mut dup = card(F1);
        dup.fen = F1.replace(" 0 1", " 3 9"); // same position, other counters
        assert_eq!(s.add_mistakes_at(&[card(F1), dup, card(F2)], 1000).unwrap(), 2);
        assert_eq!(s.add_mistakes_at(&[card(F1)], 1000).unwrap(), 0);
        let c = s.next_mistake_at(None, 1000).unwrap().unwrap();
        assert_eq!(c.solution.len(), 1, "even-length solution is trimmed to end on a user move");
        let sum = s.mistake_summary_at(1000).unwrap();
        assert_eq!((sum.due, sum.total, sum.graduated), (2, 2, 0));
    }

    #[test]
    fn graduates_after_three_correct_due_answers() {
        let s = Store::open_in_memory().unwrap();
        s.add_mistakes_at(&[card(F1)], 0).unwrap();
        let id = s.next_mistake_at(None, 0).unwrap().unwrap().id;
        let mut now = 10;
        let o = s.attempt_mistake_at(id, true, 5000, now).unwrap().unwrap();
        assert!(o.counted);
        assert_eq!(o.card.streak, 1);
        assert!((o.card.interval_days - 1.0).abs() < 1e-9);
        // Early practice doesn't count.
        let early = s.attempt_mistake_at(id, true, 5000, now + 60).unwrap().unwrap();
        assert!(!early.counted);
        assert_eq!(early.card.streak, 1);
        now += 86_400 + 1;
        let o2 = s.attempt_mistake_at(id, true, 30_000, now).unwrap().unwrap();
        assert_eq!(o2.card.streak, 2);
        assert!(o2.card.interval_days >= 3.0);
        assert!(!o2.card.graduated);
        now += (o2.card.interval_days * DAY) as i64 + 1;
        let o3 = s.attempt_mistake_at(id, true, 30_000, now).unwrap().unwrap();
        assert!(o3.card.graduated && o3.graduated_now);
        assert!(o3.card.interval_days > o2.card.interval_days);
        let sum = s.mistake_summary_at(now).unwrap();
        assert_eq!((sum.due, sum.total, sum.graduated, sum.learning), (0, 1, 1, 0));
        assert!(s.next_mistake_at(None, now).unwrap().is_none());
    }

    #[test]
    fn failure_resets_and_relearns_soon() {
        let s = Store::open_in_memory().unwrap();
        s.add_mistakes_at(&[card(F1)], 0).unwrap();
        let id = s.next_mistake_at(None, 0).unwrap().unwrap().id;
        s.attempt_mistake_at(id, true, 1000, 1).unwrap();
        let o = s.attempt_mistake_at(id, false, 1000, 100).unwrap().unwrap();
        assert_eq!(o.card.streak, 0);
        assert_eq!(o.card.lapses, 1);
        assert_eq!(o.card.due_in_secs, RELEARN_SECS);
        assert!(o.card.ease < START_EASE + 0.2);
        let sum = s.mistake_summary_at(100).unwrap();
        assert_eq!(sum.due, 0);
        assert_eq!(sum.next_due_in_secs, Some(RELEARN_SECS));
        // Nothing due: next returns the soonest upcoming card.
        let n = s.next_mistake_at(None, 100).unwrap().unwrap();
        assert!(n.due_in_secs > 0);
    }

    #[test]
    fn removed_cards_stay_gone() {
        let s = Store::open_in_memory().unwrap();
        s.add_mistakes_at(&[card(F1)], 0).unwrap();
        let id = s.next_mistake_at(None, 0).unwrap().unwrap().id;
        assert!(s.remove_mistake(id).unwrap());
        assert!(!s.remove_mistake(id).unwrap());
        assert_eq!(s.add_mistakes_at(&[card(F1)], 0).unwrap(), 0);
        assert_eq!(s.mistake_summary_at(0).unwrap().total, 0);
        assert!(s.attempt_mistake_at(id, true, 0, 0).unwrap().is_none());
        assert!(s.list_mistakes("all", 10, 0).unwrap().is_empty());
    }

    #[test]
    fn next_skips_excluded_card() {
        let s = Store::open_in_memory().unwrap();
        s.add_mistakes_at(&[card(F1), card(F2)], 0).unwrap();
        let a = s.next_mistake_at(None, 0).unwrap().unwrap();
        let b = s.next_mistake_at(Some(a.id), 0).unwrap().unwrap();
        assert_ne!(a.id, b.id);
        s.remove_mistake(b.id).unwrap();
        // Only the excluded one is left: it comes back.
        assert_eq!(s.next_mistake_at(Some(a.id), 0).unwrap().unwrap().id, a.id);
        assert_eq!(s.list_mistakes("due", 10, 0).unwrap().len(), 1);
    }
}
