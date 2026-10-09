//! Guided first week: a 7-day path for new players that deep-links into existing features.
//!
//! * The plan ([`PLAN`]) is static: 7 days, each with 2–4 steps. Every step has a [`Rule`] that is
//!   checked against real activity (lessons, drills, endgame practice, games, reviews, puzzles,
//!   daily goal). Anything can also be marked done by hand.
//! * Day `n` (1-based) unlocks on `started_on + (n - 1)` (UTC calendar days, same clock as the
//!   activity log). Unlocked days stay open, so a missed day can be caught up later.
//! * Completed steps are persisted in `first_week_steps` the first time they are seen done (only
//!   for unlocked days), so progress never goes backwards and the client can celebrate exactly
//!   once (`newly_done`, `newly_completed_days`, `week_just_completed`).
//!
//! Tables are created by [`schema`], which runs inside schema migration v3 (see `lib.rs`).

use std::collections::{BTreeMap, HashSet};

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::activity::{format_day, parse_day};
use crate::Store;

/// Days in the path.
pub const DAYS: u32 = 7;
/// New players (fewer games *and* fewer completed lessons than this) are offered the path.
pub const NEWCOMER_MAX: u32 = 3;
/// Bounds on the activity rows read per evaluation.
const MAX_GAMES_SCANNED: u32 = 500;
const MAX_PUZZLES_SCANNED: u32 = 500;

/// How a step is completed automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The lesson is completed (`lesson_progress`).
    Lesson { course: &'static str, lesson: &'static str },
    /// At least one run of the quick drill.
    Drill(&'static str),
    /// At least one successful attempt at the endgame drill.
    Endgame(&'static str),
    /// A game against one of these bots, started on/after the day unlocked.
    GameVs(&'static [&'static str]),
    /// A game was reviewed (Game Review saved) since the path started.
    Review,
    /// `n` puzzles solved (with one of `themes`, when non-empty) on/after the day unlocked.
    Puzzles { n: u32, themes: &'static [&'static str] },
    /// A daily goal has been chosen.
    DailyGoal,
}

impl Rule {
    pub fn kind(&self) -> &'static str {
        match self {
            Rule::Lesson { .. } => "lesson",
            Rule::Drill(_) => "drill",
            Rule::Endgame(_) => "endgame",
            Rule::GameVs(_) => "game",
            Rule::Review => "review",
            Rule::Puzzles { .. } => "puzzles",
            Rule::DailyGoal => "goal",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StepDef {
    /// Stable id (also the i18n key `firstweek.steps.<id>`).
    pub id: &'static str,
    pub rule: Rule,
    /// Deep link (hash route). `Review` steps link to the latest game instead when there is one.
    pub href: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct DayDef {
    /// Stable id (i18n key `firstweek.days.<id>`).
    pub id: &'static str,
    pub emoji: &'static str,
    pub steps: &'static [StepDef],
}

/// Coach bots (category `coach`).
pub const COACH_BOTS: &[&str] = &["coach", "coach-leo"];
/// Beginner bots (Elo <= 900).
pub const BEGINNER_BOTS: &[&str] = &["pawnny", "lulu", "benny", "rosa"];

const fn lesson(id: &'static str, course: &'static str, lesson: &'static str, href: &'static str) -> StepDef {
    StepDef { id, rule: Rule::Lesson { course, lesson }, href }
}

/// The seven days. Ids are verified against the real content by a gm-server test.
pub static PLAN: &[DayDef] = &[
    DayDef {
        id: "pieces",
        emoji: "♟️",
        steps: &[
            lesson("board", "chess-basics", "the-board", "#/learn/chess-basics/the-board"),
            lesson("longMovers", "chess-basics", "rook-bishop-queen", "#/learn/chess-basics/rook-bishop-queen"),
            lesson("knightKingPawn", "chess-basics", "knight-king-pawn", "#/learn/chess-basics/knight-king-pawn"),
        ],
    },
    DayDef {
        id: "capture",
        emoji: "⚔️",
        steps: &[
            lesson("check", "chess-basics", "check-and-checkmate", "#/learn/chess-basics/check-and-checkmate"),
            lesson("values", "chess-basics", "piece-values", "#/learn/chess-basics/piece-values"),
            StepDef { id: "hangingDrill", rule: Rule::Drill("hanging"), href: "#/drills/hanging" },
        ],
    },
    DayDef {
        id: "mates",
        emoji: "👑",
        steps: &[
            lesson("backRank", "checkmate-patterns", "back-rank-mate", "#/learn/checkmate-patterns/back-rank-mate"),
            lesson("queenMateLesson", "checkmate-patterns", "king-queen-mate", "#/learn/checkmate-patterns/king-queen-mate"),
            StepDef { id: "queenMate", rule: Rule::Endgame("kq-vs-k"), href: "#/endgames/kq-vs-k" },
        ],
    },
    DayDef {
        id: "coach",
        emoji: "🎓",
        steps: &[
            lesson("develop", "opening-principles", "develop-your-pieces", "#/learn/opening-principles/develop-your-pieces"),
            StepDef { id: "coachGame", rule: Rule::GameVs(COACH_BOTS), href: "#/play/coach" },
        ],
    },
    DayDef {
        id: "review",
        emoji: "🔍",
        steps: &[
            StepDef { id: "reviewGame", rule: Rule::Review, href: "#/library" },
            lesson("castling", "chess-basics", "castling", "#/learn/chess-basics/castling"),
        ],
    },
    DayDef {
        id: "tactics",
        emoji: "🍴",
        steps: &[
            lesson("forks", "tactics-fundamentals", "forks", "#/learn/tactics-fundamentals/forks"),
            lesson("pins", "tactics-fundamentals", "pins", "#/learn/tactics-fundamentals/pins"),
            StepDef { id: "tacticPuzzles", rule: Rule::Puzzles { n: 3, themes: &["fork", "pin"] }, href: "#/puzzles?play=1&theme=fork" },
        ],
    },
    DayDef {
        id: "graduation",
        emoji: "🏆",
        steps: &[
            StepDef { id: "beginnerGame", rule: Rule::GameVs(BEGINNER_BOTS), href: "#/play/benny" },
            StepDef { id: "dailyGoal", rule: Rule::DailyGoal, href: "#/" },
        ],
    },
];

/// Looks up a step by id: `(day index 0-based, step)`.
pub fn find_step(id: &str) -> Option<(usize, &'static StepDef)> {
    PLAN.iter().enumerate().find_map(|(i, d)| d.steps.iter().find(|s| s.id == id).map(|s| (i, s)))
}

// ---------------------------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StepProgress {
    pub have: u32,
    pub need: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StepState {
    pub id: String,
    /// lesson | drill | endgame | game | review | puzzles | goal
    pub kind: String,
    pub href: String,
    pub done: bool,
    /// Marked done by hand (can be undone).
    pub manual: bool,
    /// For counted steps (puzzles).
    pub progress: Option<StepProgress>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DayState {
    /// 1..=7
    pub day: u32,
    pub id: String,
    pub emoji: String,
    /// `YYYY-MM-DD` (UTC) the day opens.
    pub unlock_on: String,
    pub unlocked: bool,
    pub done: bool,
    pub steps: Vec<StepState>,
}

/// Everything `GET /api/first-week` returns.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FirstWeekState {
    pub started: bool,
    /// `YYYY-MM-DD` (UTC); None until started.
    pub started_on: Option<String>,
    pub today: String,
    /// Hidden from Home ("I already know how to play").
    pub dismissed: bool,
    /// True when Home should offer the path (new player, or path in progress; not dismissed).
    pub eligible: bool,
    pub unlocked_days: u32,
    pub completed_days: u32,
    pub week_complete: bool,
    /// First unlocked day that is not done (1..=7), or None when nothing is open.
    pub current_day: Option<u32>,
    pub days: Vec<DayState>,
    /// Steps that became done during this request (for celebrations).
    pub newly_done: Vec<String>,
    pub newly_completed_days: Vec<u32>,
    pub week_just_completed: bool,
}

// ---------------------------------------------------------------------------------------------
// Pure evaluation
// ---------------------------------------------------------------------------------------------

/// A game the rules care about.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameFact {
    pub id: i64,
    pub bot_id: Option<String>,
    /// Day number the game was created.
    pub day: i64,
    pub reviewed: bool,
    /// Day number of the last update (a review saves the game).
    pub updated_day: i64,
}

/// Snapshot of the user's activity, read once per evaluation.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub lessons: HashSet<(String, String)>,
    pub drills: HashSet<String>,
    pub endgames: HashSet<String>,
    /// Newest first.
    pub games: Vec<GameFact>,
    /// `(puzzle id, day solved)`.
    pub puzzles: Vec<(String, i64)>,
    pub goal_set: bool,
}

/// Is the step satisfied by real activity? Returns `(done, progress)`.
///
/// `start` / `unlock` are day numbers; `has_theme(puzzle_id, themes)` tells whether a puzzle has
/// one of the themes.
pub fn auto_done(
    rule: &Rule,
    facts: &Facts,
    start: i64,
    unlock: i64,
    has_theme: &dyn Fn(&str, &[&str]) -> bool,
) -> (bool, Option<StepProgress>) {
    match rule {
        Rule::Lesson { course, lesson } => {
            (facts.lessons.contains(&((*course).to_string(), (*lesson).to_string())), None)
        }
        Rule::Drill(id) => (facts.drills.contains(*id), None),
        Rule::Endgame(id) => (facts.endgames.contains(*id), None),
        Rule::GameVs(bots) => (
            facts
                .games
                .iter()
                .any(|g| g.day >= unlock && g.bot_id.as_deref().is_some_and(|b| bots.contains(&b))),
            None,
        ),
        Rule::Review => (facts.games.iter().any(|g| g.reviewed && g.updated_day >= start), None),
        Rule::Puzzles { n, themes } => {
            let have = facts
                .puzzles
                .iter()
                .filter(|(id, day)| *day >= unlock && (themes.is_empty() || has_theme(id, themes)))
                .count()
                .min(*n as usize) as u32;
            (have >= *n, Some(StepProgress { have, need: *n }))
        }
        Rule::DailyGoal => (facts.goal_set, None),
    }
}

/// Persisted completion of one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoneRow {
    pub manual: bool,
}

/// Builds the state from the plan, the persisted rows and the facts. Also returns the step ids
/// that are newly auto-done (on unlocked days) and should be persisted.
pub fn evaluate(
    start: i64,
    today: i64,
    done: &BTreeMap<String, DoneRow>,
    facts: &Facts,
    has_theme: &dyn Fn(&str, &[&str]) -> bool,
) -> (Vec<DayState>, Vec<String>) {
    let mut days = Vec::with_capacity(PLAN.len());
    let mut fresh = Vec::new();
    let last_game = facts.games.first().map(|g| g.id);
    for (i, d) in PLAN.iter().enumerate() {
        let unlock = start + i as i64;
        let unlocked = unlock <= today;
        let steps: Vec<StepState> = d
            .steps
            .iter()
            .map(|s| {
                let (auto, progress) = auto_done(&s.rule, facts, start, unlock, has_theme);
                let row = done.get(s.id);
                let is_done = unlocked && (row.is_some() || auto);
                if unlocked && auto && row.is_none() {
                    fresh.push(s.id.to_string());
                }
                let href = match (s.rule, last_game) {
                    (Rule::Review, Some(id)) => format!("#/review/{id}"),
                    _ => s.href.to_string(),
                };
                StepState {
                    id: s.id.to_string(),
                    kind: s.rule.kind().to_string(),
                    href,
                    done: is_done,
                    manual: is_done && row.is_some_and(|r| r.manual),
                    progress: progress.map(|p| if is_done { StepProgress { have: p.need, need: p.need } } else { p }),
                }
            })
            .collect();
        days.push(DayState {
            day: i as u32 + 1,
            id: d.id.to_string(),
            emoji: d.emoji.to_string(),
            unlock_on: format_day(unlock),
            unlocked,
            done: unlocked && steps.iter().all(|s| s.done),
            steps,
        });
    }
    (days, fresh)
}

// ---------------------------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------------------------

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS first_week (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  started_on TEXT,
  dismissed INTEGER NOT NULL DEFAULT 0,
  completed_on TEXT,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE TABLE IF NOT EXISTS first_week_steps (
  step_id TEXT PRIMARY KEY,
  manual INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
) WITHOUT ROWID;
"#,
    )
}

fn today_num(conn: &Connection) -> anyhow::Result<i64> {
    let s: String = conn.prepare_cached("SELECT date('now')")?.query_row([], |r| r.get(0))?;
    parse_day(&s).context("bad clock")
}

struct Meta {
    started_on: Option<i64>,
    dismissed: bool,
}

fn meta(conn: &Connection) -> anyhow::Result<Meta> {
    let row: Option<(Option<String>, i64)> = conn
        .prepare_cached("SELECT started_on, dismissed FROM first_week WHERE id = 1")?
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((s, d)) => Meta { started_on: s.as_deref().and_then(parse_day), dismissed: d != 0 },
        None => Meta { started_on: None, dismissed: false },
    })
}

fn done_rows(conn: &Connection) -> anyhow::Result<BTreeMap<String, DoneRow>> {
    let mut stmt = conn.prepare_cached("SELECT step_id, manual FROM first_week_steps")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (id, manual) = row?;
        out.insert(id, DoneRow { manual: manual != 0 });
    }
    Ok(out)
}

fn day_of(ts: &str) -> i64 {
    parse_day(ts).unwrap_or(i64::MIN)
}

fn facts(conn: &Connection, start: i64) -> anyhow::Result<Facts> {
    let mut f = Facts::default();
    let mut stmt = conn.prepare_cached("SELECT course_id, lesson_id FROM lesson_progress WHERE completed = 1")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        f.lessons.insert(row?);
    }
    let mut stmt = conn.prepare_cached("SELECT DISTINCT drill FROM drill_bests WHERE plays > 0")?;
    for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
        f.drills.insert(row?);
    }
    let mut stmt = conn.prepare_cached("SELECT drill_id FROM endgame_training WHERE successes > 0")?;
    for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
        f.endgames.insert(row?);
    }
    let since = format_day(start);
    let mut stmt = conn.prepare_cached(
        "SELECT id, bot_id, created_at, review_json IS NOT NULL, updated_at FROM games \
         WHERE updated_at >= ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![since, MAX_GAMES_SCANNED], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, bool>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (id, bot_id, created, reviewed, updated) = row?;
        f.games.push(GameFact { id, bot_id, day: day_of(&created), reviewed, updated_day: day_of(&updated) });
    }
    let mut stmt = conn.prepare_cached(
        "SELECT puzzle_id, created_at FROM puzzle_attempts WHERE solved = 1 AND created_at >= ?1 \
         ORDER BY id DESC LIMIT ?2",
    )?;
    for row in stmt.query_map(params![since, MAX_PUZZLES_SCANNED], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (id, at) = row?;
        f.puzzles.push((id, day_of(&at)));
    }
    f.goal_set = conn.prepare_cached("SELECT 1 FROM daily_goal WHERE id = 1")?.exists([])?;
    Ok(f)
}

fn is_newcomer(conn: &Connection) -> anyhow::Result<bool> {
    let games: i64 = conn.prepare_cached("SELECT COUNT(*) FROM games")?.query_row([], |r| r.get(0))?;
    let lessons: i64 = conn
        .prepare_cached("SELECT COUNT(*) FROM lesson_progress WHERE completed = 1")?
        .query_row([], |r| r.get(0))?;
    Ok(games < i64::from(NEWCOMER_MAX) && lessons < i64::from(NEWCOMER_MAX))
}

fn count_done(days: &[DayState]) -> HashSet<u32> {
    days.iter().filter(|d| d.done).map(|d| d.day).collect()
}

/// Evaluates, persists newly auto-done steps and builds the response. `before` is the persisted
/// set of done days before the caller's change (None = compute it from the current rows).
fn state_at(
    conn: &Connection,
    today: i64,
    has_theme: &dyn Fn(&str, &[&str]) -> bool,
    before: Option<HashSet<u32>>,
) -> anyhow::Result<FirstWeekState> {
    let m = meta(conn)?;
    let Some(start) = m.started_on else {
        // Not started: a preview as if starting today (nothing unlocked, nothing done).
        let empty = Facts::default();
        let (mut days, _) = evaluate(today, today - 1, &BTreeMap::new(), &empty, has_theme);
        for d in &mut days {
            d.unlock_on.clear();
        }
        return Ok(FirstWeekState {
            started: false,
            today: format_day(today),
            dismissed: m.dismissed,
            eligible: !m.dismissed && is_newcomer(conn)?,
            days,
            ..Default::default()
        });
    };
    let f = facts(conn, start)?;
    let rows = done_rows(conn)?;
    let before = match before {
        Some(b) => b,
        None => count_done(&evaluate(start, today, &rows, &Facts::default(), has_theme).0),
    };
    let (mut days, fresh) = evaluate(start, today, &rows, &f, has_theme);
    if !fresh.is_empty() {
        let mut ins =
            conn.prepare_cached("INSERT OR IGNORE INTO first_week_steps (step_id, manual) VALUES (?1, 0)")?;
        for id in &fresh {
            ins.execute(params![id])?;
        }
        // Re-evaluate from the persisted rows so `done` is stable.
        days = evaluate(start, today, &done_rows(conn)?, &f, has_theme).0;
    }
    let after = count_done(&days);
    let mut newly_completed_days: Vec<u32> = after.difference(&before).copied().collect();
    newly_completed_days.sort_unstable();
    let week_complete = after.len() == PLAN.len();
    let week_just_completed = week_complete && before.len() < PLAN.len();
    if week_just_completed {
        conn.prepare_cached("UPDATE first_week SET completed_on = ?1 WHERE id = 1 AND completed_on IS NULL")?
            .execute(params![format_day(today)])?;
    }
    Ok(FirstWeekState {
        started: true,
        started_on: Some(format_day(start)),
        today: format_day(today),
        dismissed: m.dismissed,
        eligible: !m.dismissed && !week_complete,
        unlocked_days: days.iter().filter(|d| d.unlocked).count() as u32,
        completed_days: after.len() as u32,
        week_complete,
        current_day: days.iter().find(|d| d.unlocked && !d.done).map(|d| d.day),
        newly_done: fresh,
        newly_completed_days,
        week_just_completed,
        days,
    })
}

fn start_at(conn: &Connection, today: i64, restart: bool) -> anyhow::Result<()> {
    if restart {
        conn.execute("DELETE FROM first_week_steps", [])?;
        conn.execute(
            "INSERT INTO first_week (id, started_on, dismissed, completed_on) VALUES (1, ?1, 0, NULL) \
             ON CONFLICT(id) DO UPDATE SET started_on = excluded.started_on, dismissed = 0, completed_on = NULL, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            params![format_day(today)],
        )?;
    } else {
        conn.execute(
            "INSERT INTO first_week (id, started_on, dismissed) VALUES (1, ?1, 0) \
             ON CONFLICT(id) DO UPDATE SET started_on = COALESCE(started_on, excluded.started_on), dismissed = 0, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            params![format_day(today)],
        )?;
    }
    Ok(())
}

/// A request the user can fix (unknown step, locked day, not started). Maps to HTTP 400.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserError(pub &'static str);

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for UserError {}

/// True when `e` is a [`UserError`].
pub fn is_user_error(e: &anyhow::Error) -> bool {
    e.downcast_ref::<UserError>().is_some()
}

fn set_step_at(conn: &Connection, today: i64, start: i64, step_id: &str, done: bool) -> anyhow::Result<()> {
    let Some((day_idx, step)) = find_step(step_id) else { return Err(UserError("unknown step").into()) };
    if start + day_idx as i64 > today {
        return Err(UserError("this day is not unlocked yet").into());
    }
    if done {
        conn.prepare_cached(
            "INSERT INTO first_week_steps (step_id, manual) VALUES (?1, 1) ON CONFLICT(step_id) DO NOTHING",
        )?
        .execute(params![step.id])?;
    } else {
        conn.prepare_cached("DELETE FROM first_week_steps WHERE step_id = ?1 AND manual = 1")?
            .execute(params![step.id])?;
    }
    Ok(())
}

impl Store {
    /// Current state; persists steps that real activity has completed since the last call.
    pub fn first_week(&self, has_theme: &dyn Fn(&str, &[&str]) -> bool) -> anyhow::Result<FirstWeekState> {
        let conn = self.conn.lock();
        let today = today_num(&conn)?;
        state_at(&conn, today, has_theme, None)
    }

    /// Starts the path today (no-op when already started) and clears `dismissed`.
    pub fn first_week_start(&self, has_theme: &dyn Fn(&str, &[&str]) -> bool) -> anyhow::Result<FirstWeekState> {
        let conn = self.conn.lock();
        let today = today_num(&conn)?;
        start_at(&conn, today, false)?;
        state_at(&conn, today, has_theme, None)
    }

    /// Starts over from Day 1 today, forgetting every completed step.
    pub fn first_week_restart(&self, has_theme: &dyn Fn(&str, &[&str]) -> bool) -> anyhow::Result<FirstWeekState> {
        let conn = self.conn.lock();
        let today = today_num(&conn)?;
        start_at(&conn, today, true)?;
        state_at(&conn, today, has_theme, None)
    }

    /// Hides (or shows again) the path on Home.
    pub fn first_week_dismiss(
        &self,
        dismissed: bool,
        has_theme: &dyn Fn(&str, &[&str]) -> bool,
    ) -> anyhow::Result<FirstWeekState> {
        let conn = self.conn.lock();
        let today = today_num(&conn)?;
        conn.execute(
            "INSERT INTO first_week (id, dismissed) VALUES (1, ?1) \
             ON CONFLICT(id) DO UPDATE SET dismissed = excluded.dismissed, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')",
            params![dismissed],
        )?;
        state_at(&conn, today, has_theme, None)
    }

    /// Marks a step done by hand, or undoes a manual mark (`done = false`; auto-detected steps
    /// stay done). The step's day must be unlocked.
    pub fn first_week_set_step(
        &self,
        step_id: &str,
        done: bool,
        has_theme: &dyn Fn(&str, &[&str]) -> bool,
    ) -> anyhow::Result<FirstWeekState> {
        let conn = self.conn.lock();
        let today = today_num(&conn)?;
        let Some(start) = meta(&conn)?.started_on else {
            return Err(UserError("the first week has not started yet").into());
        };
        let before = count_done(&evaluate(start, today, &done_rows(&conn)?, &Facts::default(), has_theme).0);
        set_step_at(&conn, today, start, step_id, done)?;
        let mut st = state_at(&conn, today, has_theme, Some(before))?;
        if done && !st.newly_done.iter().any(|s| s == step_id) {
            st.newly_done.push(step_id.to_string());
        }
        Ok(st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewGame;

    fn no_theme(_: &str, _: &[&str]) -> bool {
        false
    }

    fn all_steps() -> usize {
        PLAN.iter().map(|d| d.steps.len()).sum()
    }

    #[test]
    fn plan_shape() {
        assert_eq!(PLAN.len(), DAYS as usize);
        let mut ids = HashSet::new();
        for d in PLAN {
            assert!((2..=4).contains(&d.steps.len()), "day {} has {} steps", d.id, d.steps.len());
            for s in d.steps {
                assert!(ids.insert(s.id), "duplicate step id {}", s.id);
                assert!(s.href.starts_with("#/"));
            }
        }
        assert!(find_step("coachGame").is_some_and(|(i, _)| i == 3));
        assert!(find_step("nope").is_none());
    }

    #[test]
    fn rules_detect_activity() {
        let start = 100;
        let mut f = Facts::default();
        let lesson = Rule::Lesson { course: "chess-basics", lesson: "the-board" };
        assert!(!auto_done(&lesson, &f, start, start, &no_theme).0);
        f.lessons.insert(("chess-basics".into(), "the-board".into()));
        assert!(auto_done(&lesson, &f, start, start, &no_theme).0);

        // A coach game counts only when played on/after the day unlocked.
        let game = Rule::GameVs(COACH_BOTS);
        f.games.push(GameFact { id: 1, bot_id: Some("coach".into()), day: start + 1, reviewed: false, updated_day: start + 1 });
        assert!(!auto_done(&game, &f, start, start + 3, &no_theme).0);
        assert!(auto_done(&game, &f, start, start + 1, &no_theme).0);
        assert!(!auto_done(&Rule::GameVs(BEGINNER_BOTS), &f, start, start, &no_theme).0);
        assert!(!auto_done(&Rule::Review, &f, start, start + 4, &no_theme).0);
        f.games[0].reviewed = true;
        assert!(auto_done(&Rule::Review, &f, start, start + 4, &no_theme).0);

        // Puzzles: counted by theme and date, with progress.
        let pz = Rule::Puzzles { n: 3, themes: &["fork"] };
        let forky = |id: &str, th: &[&str]| id.starts_with('f') && th.contains(&"fork");
        f.puzzles = vec![("f1".into(), start + 5), ("x".into(), start + 5), ("f2".into(), start + 4), ("f3".into(), start + 5)];
        let (done, p) = auto_done(&pz, &f, start, start + 5, &forky);
        assert!(!done);
        assert_eq!(p, Some(StepProgress { have: 2, need: 3 }));
        f.puzzles.push(("f4".into(), start + 6));
        assert!(auto_done(&pz, &f, start, start + 5, &forky).0);
    }

    #[test]
    fn days_unlock_one_per_day_and_complete() {
        let start = 1000;
        let mut f = Facts::default();
        let (days, fresh) = evaluate(start, start, &BTreeMap::new(), &f, &no_theme);
        assert!(days[0].unlocked && !days[1].unlocked);
        assert_eq!(days[6].unlock_on, format_day(start + 6));
        assert!(fresh.is_empty());

        // Day 2's lesson done early: not counted until day 2 unlocks, then auto-done.
        f.lessons.insert(("chess-basics".into(), "check-and-checkmate".into()));
        let (days, fresh) = evaluate(start, start, &BTreeMap::new(), &f, &no_theme);
        assert!(!days[1].steps[0].done);
        assert!(fresh.is_empty());
        let (days, fresh) = evaluate(start, start + 1, &BTreeMap::new(), &f, &no_theme);
        assert!(days[1].steps[0].done);
        assert_eq!(fresh, vec!["check".to_string()]);

        // Manual rows complete a day.
        let mut rows = BTreeMap::new();
        for s in PLAN[0].steps {
            rows.insert(s.id.to_string(), DoneRow { manual: true });
        }
        let (days, _) = evaluate(start, start + 1, &rows, &f, &no_theme);
        assert!(days[0].done && days[0].steps.iter().all(|s| s.manual));
        assert!(!days[1].done);
        // Catch-up: a week later every day is open.
        let (days, _) = evaluate(start, start + 30, &rows, &f, &no_theme);
        assert!(days.iter().all(|d| d.unlocked));
    }

    #[test]
    fn store_flow() {
        let s = Store::open_in_memory().unwrap();
        let st = s.first_week(&no_theme).unwrap();
        assert!(!st.started && st.eligible && st.days.len() == 7);
        assert!(st.days.iter().all(|d| !d.unlocked));
        assert!(s.first_week_set_step("board", true, &no_theme).is_err());

        let st = s.first_week_start(&no_theme).unwrap();
        assert!(st.started && st.eligible);
        assert_eq!(st.unlocked_days, 1);
        assert_eq!(st.current_day, Some(1));

        // Locked day / unknown step are rejected as user errors.
        let e = s.first_week_set_step("check", true, &no_theme).unwrap_err();
        assert!(is_user_error(&e));
        assert!(is_user_error(&s.first_week_set_step("zzz", true, &no_theme).unwrap_err()));

        // Auto-detection from a real lesson completion, then manual marks finish day 1.
        s.set_lesson_progress("chess-basics", "the-board", true).unwrap();
        let st = s.first_week(&no_theme).unwrap();
        assert_eq!(st.newly_done, vec!["board".to_string()]);
        assert!(!st.days[0].steps[0].manual);
        let st = s.first_week(&no_theme).unwrap();
        assert!(st.newly_done.is_empty(), "celebrate only once");
        s.first_week_set_step("longMovers", true, &no_theme).unwrap();
        let st = s.first_week_set_step("knightKingPawn", true, &no_theme).unwrap();
        assert_eq!(st.newly_completed_days, vec![1]);
        assert_eq!(st.completed_days, 1);
        assert_eq!(st.current_day, None);
        // Undo a manual mark; auto-detected steps cannot be undone.
        let st = s.first_week_set_step("knightKingPawn", false, &no_theme).unwrap();
        assert!(!st.days[0].done);
        let st = s.first_week_set_step("board", false, &no_theme).unwrap();
        assert!(st.days[0].steps[0].done);

        // Move the start back a week: everything unlocks; finish it all and celebrate once.
        {
            let conn = s.conn.lock();
            conn.execute("UPDATE first_week SET started_on = date('now', '-6 days')", []).unwrap();
        }
        s.create_game(&NewGame {
            result: "1-0".into(),
            moves: vec!["e2e4".into()],
            bot_id: Some("coach".into()),
            ..Default::default()
        })
        .unwrap();
        let st = s.first_week(&no_theme).unwrap();
        assert_eq!(st.unlocked_days, 7);
        assert!(st.newly_done.contains(&"coachGame".to_string()));
        assert!(st.days[4].steps[0].href.starts_with("#/review/"));
        let mut last = None;
        for d in PLAN {
            for step in d.steps {
                last = Some(s.first_week_set_step(step.id, true, &no_theme).unwrap());
            }
        }
        let st = last.unwrap();
        assert!(st.week_complete && st.week_just_completed && !st.eligible);
        assert_eq!(st.completed_days, 7);
        assert!(!s.first_week(&no_theme).unwrap().week_just_completed);
        let rows: i64 = s.conn.lock().query_row("SELECT COUNT(*) FROM first_week_steps", [], |r| r.get(0)).unwrap();
        assert_eq!(rows as usize, all_steps());

        // Restart forgets steps and starts today.
        let st = s.first_week_restart(&no_theme).unwrap();
        assert_eq!(st.unlocked_days, 1);
        assert!(!st.week_complete);
        // Dismiss hides it from Home.
        let st = s.first_week_dismiss(true, &no_theme).unwrap();
        assert!(st.dismissed && !st.eligible);
        let st = s.first_week_start(&no_theme).unwrap();
        assert!(!st.dismissed);
    }

    #[test]
    fn veterans_are_not_offered_the_path() {
        let s = Store::open_in_memory().unwrap();
        for _ in 0..NEWCOMER_MAX {
            s.create_game(&NewGame { result: "*".into(), ..Default::default() }).unwrap();
        }
        assert!(!s.first_week(&no_theme).unwrap().eligible);
    }
}
