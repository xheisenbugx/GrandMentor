//! Daily activity log used by the daily plan, streaks and goals.
//!
//! Every feature records what the user did with [`Store::log_activity`]; the daily plan reads
//! the per-day totals back with [`Store::activity_days`] and [`Store::daily_summary`].
//!
//! ## Days and time zones
//! All days are **UTC calendar days** (`YYYY-MM-DD`, SQLite `date('now')`). The activity log,
//! the profile's `last_active` column and every streak computation use the same clock, so a day
//! never "moves" between tables. (A user far from UTC sees the day roll over at UTC midnight.)
//!
//! ## Streaks: one source of truth
//! The streak is always computed by [`streak_in`] from the set of active days, which is the union
//! of
//! - every day with a non-zero count in the `activity` table, and
//! - the run recorded in `profile.last_active` / `profile.streak_days` (the pre-activity-log
//!   streak, still bumped by store writes such as games and puzzle attempts; this keeps streaks
//!   earned before the activity log existed).
//!
//! `Profile::streak_days`, `Stats::streak_days` and `GET /api/daily` all read it from here.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

/// Known activity kinds. Unknown kinds are rejected so the table stays tidy.
pub const KINDS: &[&str] = &[
    "game", "puzzle", "lesson", "mistake_review", "repertoire_review", "endgame", "drill", "classic", "local_game",
];

/// Max days returned by [`Store::activity_days`].
pub const MAX_DAYS: u32 = 400;

/// How far back (days) streaks are computed. Bounds the work per request.
pub const STREAK_HORIZON_DAYS: i64 = 3650;

/// Allowed goal kinds and their target ranges.
pub const GOAL_MINUTES: &str = "minutes";
pub const GOAL_ACTIVITIES: &str = "activities";
pub const MAX_GOAL_MINUTES: u32 = 240;
pub const MAX_GOAL_ACTIVITIES: u32 = 100;

/// Rough minutes one unit of each activity takes; used to turn counts into "minutes practised".
pub fn minutes_per(kind: &str) -> u32 {
    match kind {
        "game" | "local_game" => 10,
        "classic" => 5,
        "lesson" => 4,
        "endgame" => 3,
        "puzzle" | "drill" => 2,
        "mistake_review" | "repertoire_review" => 1,
        _ => 1,
    }
}

/// Totals for one UTC day.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ActivityDay {
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
    /// kind -> count
    pub counts: BTreeMap<String, u32>,
    /// Sum of all counts.
    pub total: u32,
}

/// The user's daily goal.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DailyGoal {
    /// `"minutes"` or `"activities"`.
    pub kind: String,
    pub target: u32,
}

impl Default for DailyGoal {
    fn default() -> Self {
        DailyGoal { kind: GOAL_MINUTES.into(), target: 10 }
    }
}

impl DailyGoal {
    /// Validates and normalises a goal coming from the user.
    pub fn validated(&self) -> anyhow::Result<DailyGoal> {
        let max = match self.kind.as_str() {
            GOAL_MINUTES => MAX_GOAL_MINUTES,
            GOAL_ACTIVITIES => MAX_GOAL_ACTIVITIES,
            _ => anyhow::bail!("goal kind must be \"minutes\" or \"activities\""),
        };
        if self.target == 0 || self.target > max {
            anyhow::bail!("goal target must be between 1 and {max}");
        }
        Ok(self.clone())
    }
}

/// Current and best streak, in days.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Streak {
    /// Consecutive active days ending today, or ending yesterday when today has no activity yet
    /// (the streak is still alive until a whole day is missed).
    pub current: u32,
    pub best: u32,
    /// True when today already counts.
    pub today_active: bool,
}

/// Progress towards today's goal.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct GoalProgress {
    /// Estimated minutes practised today.
    pub minutes: u32,
    /// Activities completed today.
    pub activities: u32,
    /// Value measured in the goal's unit.
    pub value: u32,
    pub target: u32,
    /// 0..=1
    pub ratio: f32,
    pub met: bool,
}

/// One day of the 7-day strip.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DayDot {
    pub day: String,
    pub total: u32,
    pub active: bool,
}

/// Everything `GET /api/daily` returns.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DailySummary {
    /// Today (UTC, `YYYY-MM-DD`).
    pub date: String,
    pub goal: DailyGoal,
    pub progress: GoalProgress,
    pub streak: Streak,
    /// Oldest first, ending today.
    pub last7: Vec<DayDot>,
    /// Today's counts per kind.
    pub today: BTreeMap<String, u32>,
}

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS activity (
  day TEXT NOT NULL,
  kind TEXT NOT NULL,
  count INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, kind)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS daily_goal (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  kind TEXT NOT NULL DEFAULT 'minutes',
  target INTEGER NOT NULL DEFAULT 10
);
"#,
    )
}

// ---------------------------------------------------------------------------------------------
// Day arithmetic (proleptic Gregorian, days since 1970-01-01).
// ---------------------------------------------------------------------------------------------

/// Parses `YYYY-MM-DD` into a day number. Returns `None` for anything malformed.
pub fn parse_day(s: &str) -> Option<i64> {
    let s = s.get(..10)?;
    let mut it = s.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Formats a day number as `YYYY-MM-DD`.
pub fn format_day(n: i64) -> String {
    let z = n + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Pure streak computation over a set of active day numbers.
pub fn compute_streak(active: &BTreeSet<i64>, today: i64) -> Streak {
    let today_active = active.contains(&today);
    let mut current = 0u32;
    let start = if today_active {
        Some(today)
    } else if active.contains(&(today - 1)) {
        Some(today - 1)
    } else {
        None
    };
    if let Some(mut d) = start {
        while active.contains(&d) {
            current += 1;
            d -= 1;
        }
    }
    let mut best = 0u32;
    let mut run = 0u32;
    let mut prev: Option<i64> = None;
    for &d in active.iter().filter(|&&d| d <= today) {
        run = if prev == Some(d - 1) { run + 1 } else { 1 };
        best = best.max(run);
        prev = Some(d);
    }
    Streak { current, best: best.max(current), today_active }
}

fn today_in(conn: &Connection) -> anyhow::Result<String> {
    Ok(conn.prepare_cached("SELECT date('now')")?.query_row([], |r| r.get(0))?)
}

/// Active days (activity table ∪ the profile's recorded run) within the horizon before `today`.
fn active_days_in(conn: &Connection, today: i64) -> anyhow::Result<BTreeSet<i64>> {
    let from = today - STREAK_HORIZON_DAYS;
    let mut set = BTreeSet::new();
    let mut stmt = conn.prepare_cached("SELECT DISTINCT day FROM activity WHERE count > 0 AND day >= ?1")?;
    for day in stmt.query_map(params![format_day(from)], |r| r.get::<_, String>(0))? {
        if let Some(d) = parse_day(&day?) {
            if d <= today {
                set.insert(d);
            }
        }
    }
    let legacy: Option<(String, i64)> = conn
        .prepare_cached("SELECT last_active, streak_days FROM profile WHERE id = 1")?
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    if let Some((last, len)) = legacy {
        if let Some(last) = parse_day(&last) {
            let len = len.clamp(0, STREAK_HORIZON_DAYS);
            for d in (last - len + 1)..=last {
                if d >= from && d <= today {
                    set.insert(d);
                }
            }
        }
    }
    Ok(set)
}

/// The single streak computation used everywhere (profile, stats, daily summary).
pub(crate) fn streak_in(conn: &Connection) -> anyhow::Result<Streak> {
    let today = parse_day(&today_in(conn)?).context("bad clock")?;
    Ok(compute_streak(&active_days_in(conn, today)?, today))
}

/// Records today (UTC) as active on the profile row: `last_active = today` and `streak_days` =
/// the current run computed from all sources (so the row is a cache of [`streak_in`] and never
/// drops days the activity table knows about). Called by store writes and [`Store::log_activity`].
pub(crate) fn touch_profile(conn: &Connection) -> anyhow::Result<()> {
    let today_s = today_in(conn)?;
    let today = parse_day(&today_s).context("bad clock")?;
    let mut active = active_days_in(conn, today)?;
    active.insert(today);
    let current = i64::from(compute_streak(&active, today).current);
    conn.prepare_cached(
        "UPDATE profile SET last_active = ?1, streak_days = ?2 \
         WHERE id = 1 AND (last_active != ?1 OR streak_days != ?2)",
    )?
    .execute(params![today_s, current])?;
    Ok(())
}

fn goal_in(conn: &Connection) -> anyhow::Result<DailyGoal> {
    let g: Option<(String, i64)> = conn
        .prepare_cached("SELECT kind, target FROM daily_goal WHERE id = 1")?
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match g {
        Some((kind, target)) => DailyGoal { kind, target: target.clamp(1, MAX_GOAL_MINUTES as i64) as u32 }
            .validated()
            .unwrap_or_default(),
        None => DailyGoal::default(),
    })
}

fn counts_between(conn: &Connection, from: &str, to: &str) -> anyhow::Result<BTreeMap<String, BTreeMap<String, u32>>> {
    let mut stmt =
        conn.prepare_cached("SELECT day, kind, count FROM activity WHERE day >= ?1 AND day <= ?2")?;
    let mut out: BTreeMap<String, BTreeMap<String, u32>> = BTreeMap::new();
    let rows = stmt.query_map(params![from, to], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
    })?;
    for row in rows {
        let (day, kind, count) = row?;
        out.entry(day).or_default().insert(kind, count.clamp(0, u32::MAX as i64) as u32);
    }
    Ok(out)
}

/// Builds the goal progress for one day's counts.
pub fn goal_progress(goal: &DailyGoal, counts: &BTreeMap<String, u32>) -> GoalProgress {
    let minutes = counts
        .iter()
        .fold(0u32, |acc, (k, &n)| acc.saturating_add(n.saturating_mul(minutes_per(k))));
    let activities = counts.values().fold(0u32, |acc, &n| acc.saturating_add(n));
    let value = if goal.kind == GOAL_ACTIVITIES { activities } else { minutes };
    let target = goal.target.max(1);
    GoalProgress {
        minutes,
        activities,
        value,
        target,
        ratio: (value as f32 / target as f32).min(1.0),
        met: value >= target,
    }
}

impl Store {
    /// Adds `n` to today's (UTC) counter for `kind`. `kind` must be one of [`KINDS`].
    pub fn log_activity(&self, kind: &str, n: u32) -> anyhow::Result<()> {
        if !KINDS.contains(&kind) {
            anyhow::bail!("unknown activity kind: {kind}");
        }
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO activity (day, kind, count) VALUES (strftime('%Y-%m-%d','now'), ?1, ?2) \
             ON CONFLICT(day, kind) DO UPDATE SET count = min(count + excluded.count, 1000000)",
            params![kind, n.min(10_000)],
        )
        .context("logging activity")?;
        if n > 0 {
            touch_profile(&conn)?;
        }
        Ok(())
    }

    /// Per-day totals for the last `days` days (most recent first), only days with activity.
    pub fn activity_days(&self, days: u32) -> anyhow::Result<Vec<ActivityDay>> {
        let days = days.clamp(1, MAX_DAYS);
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT day, kind, count FROM activity \
             WHERE day >= strftime('%Y-%m-%d','now', ?1) ORDER BY day DESC, kind",
        )?;
        let offset = format!("-{} days", days - 1);
        let mut out: Vec<ActivityDay> = Vec::new();
        let rows = stmt.query_map(params![offset], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?;
        for row in rows {
            let (day, kind, count) = row?;
            let count = count.clamp(0, u32::MAX as i64) as u32;
            if out.last().map(|d| d.day != day).unwrap_or(true) {
                out.push(ActivityDay { day: day.clone(), ..Default::default() });
            }
            if let Some(d) = out.last_mut() {
                d.total = d.total.saturating_add(count);
                d.counts.insert(kind, count);
            }
        }
        Ok(out)
    }

    /// Current and best streak (UTC days).
    pub fn streak(&self) -> anyhow::Result<Streak> {
        let conn = self.conn.lock();
        streak_in(&conn)
    }

    /// The user's daily goal (default: 10 minutes).
    pub fn daily_goal(&self) -> anyhow::Result<DailyGoal> {
        let conn = self.conn.lock();
        goal_in(&conn)
    }

    /// Validates and saves the daily goal.
    pub fn set_daily_goal(&self, goal: &DailyGoal) -> anyhow::Result<DailyGoal> {
        let goal = goal.validated()?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO daily_goal (id, kind, target) VALUES (1, ?1, ?2) \
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, target = excluded.target",
            params![goal.kind, goal.target],
        )
        .context("saving daily goal")?;
        Ok(goal)
    }

    /// Today's goal progress, streak and the last 7 days.
    pub fn daily_summary(&self) -> anyhow::Result<DailySummary> {
        let conn = self.conn.lock();
        let date = today_in(&conn)?;
        let today = parse_day(&date).context("bad clock")?;
        let goal = goal_in(&conn)?;
        let active = active_days_in(&conn, today)?;
        let streak = compute_streak(&active, today);
        let counts = counts_between(&conn, &format_day(today - 6), &date)?;
        let last7 = (today - 6..=today)
            .map(|d| {
                let day = format_day(d);
                let total = counts.get(&day).map(|c| c.values().sum::<u32>()).unwrap_or(0);
                DayDot { active: total > 0 || active.contains(&d), day, total }
            })
            .collect();
        let today_counts = counts.get(&date).cloned().unwrap_or_default();
        let progress = goal_progress(&goal, &today_counts);
        Ok(DailySummary { date, goal, progress, streak, last7, today: today_counts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(days: &[&str]) -> BTreeSet<i64> {
        days.iter().map(|d| parse_day(d).unwrap()).collect()
    }

    #[test]
    fn logs_and_reads_activity() {
        let s = Store::open_in_memory().unwrap();
        s.log_activity("puzzle", 2).unwrap();
        s.log_activity("puzzle", 1).unwrap();
        s.log_activity("lesson", 1).unwrap();
        assert!(s.log_activity("nope", 1).is_err());
        let days = s.activity_days(7).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].total, 4);
        assert_eq!(days[0].counts.get("puzzle"), Some(&3));
    }

    #[test]
    fn day_roundtrip() {
        assert_eq!(parse_day("1970-01-01"), Some(0));
        assert_eq!(parse_day("2000-03-01"), Some(11_017));
        for n in [-1000, 0, 59, 60, 11_016, 19_000, 20_000, 30_000] {
            assert_eq!(parse_day(&format_day(n)), Some(n), "{n}");
        }
        assert_eq!(format_day(parse_day("2024-02-29").unwrap() + 1), "2024-03-01");
        assert_eq!(format_day(parse_day("2023-12-31").unwrap() + 1), "2024-01-01");
        assert_eq!(parse_day("2024-13-01"), None);
        assert_eq!(parse_day("nope"), None);
        assert_eq!(parse_day(""), None);
    }

    #[test]
    fn streak_rules() {
        let today = parse_day("2026-03-10").unwrap();
        // Nothing yet.
        assert_eq!(compute_streak(&BTreeSet::new(), today), Streak::default());
        // Active today and the two days before.
        let s = compute_streak(&set(&["2026-03-08", "2026-03-09", "2026-03-10"]), today);
        assert_eq!((s.current, s.best, s.today_active), (3, 3, true));
        // Not yet active today: yesterday's run is still alive.
        let s = compute_streak(&set(&["2026-03-08", "2026-03-09"]), today);
        assert_eq!((s.current, s.best, s.today_active), (2, 2, false));
        // A whole missed day breaks it, but best remembers.
        let s = compute_streak(&set(&["2026-03-01", "2026-03-02", "2026-03-03", "2026-03-04", "2026-03-08"]), today);
        assert_eq!((s.current, s.best), (0, 4));
        // Across month and year boundaries.
        let t = parse_day("2026-01-01").unwrap();
        let s = compute_streak(&set(&["2025-12-30", "2025-12-31", "2026-01-01"]), t);
        assert_eq!((s.current, s.best), (3, 3));
        // Future days (clock skew) are ignored for best.
        let s = compute_streak(&set(&["2026-03-11", "2026-03-12", "2026-03-13", "2026-03-14"]), today);
        assert_eq!((s.current, s.best), (0, 0));
    }

    #[test]
    fn streak_uses_activity_table_and_legacy_profile() {
        let s = Store::open_in_memory().unwrap();
        {
            let conn = s.conn.lock();
            // Activity on the 3 previous days (UTC), nothing today yet.
            for off in 1..=3 {
                conn.execute(
                    "INSERT INTO activity (day, kind, count) VALUES (date('now', ?1), 'puzzle', 1)",
                    params![format!("-{off} days")],
                )
                .unwrap();
            }
            // Legacy profile run: ended 4 days ago, 2 days long -> joins up into a 5-day run.
            conn.execute(
                "UPDATE profile SET last_active = date('now','-4 days'), streak_days = 2 WHERE id = 1",
                [],
            )
            .unwrap();
        }
        let st = s.streak().unwrap();
        assert_eq!((st.current, st.best, st.today_active), (5, 5, false));
        // Logging today extends it and syncs the profile row.
        s.log_activity("drill", 1).unwrap();
        let st = s.streak().unwrap();
        assert_eq!((st.current, st.best, st.today_active), (6, 6, true));
        assert_eq!(s.get_profile().unwrap().streak_days, 6);
        let d = s.daily_summary().unwrap();
        assert_eq!(d.last7.len(), 7);
        assert!(d.last7.iter().skip(1).all(|x| x.active));
        assert_eq!(d.last7.last().unwrap().day, d.date);
        assert_eq!(d.today.get("drill"), Some(&1));
    }

    #[test]
    fn goals_and_progress() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(s.daily_goal().unwrap(), DailyGoal::default());
        assert!(s.set_daily_goal(&DailyGoal { kind: "hours".into(), target: 1 }).is_err());
        assert!(s.set_daily_goal(&DailyGoal { kind: "minutes".into(), target: 0 }).is_err());
        assert!(s.set_daily_goal(&DailyGoal { kind: "minutes".into(), target: 999 }).is_err());
        let g = s.set_daily_goal(&DailyGoal { kind: "activities".into(), target: 3 }).unwrap();
        assert_eq!(s.daily_goal().unwrap(), g);

        let d = s.daily_summary().unwrap();
        assert!(!d.progress.met);
        assert_eq!(d.streak.current, 0);
        s.log_activity("puzzle", 2).unwrap();
        let d = s.daily_summary().unwrap();
        assert_eq!((d.progress.value, d.progress.met), (2, false));
        s.log_activity("lesson", 1).unwrap();
        let d = s.daily_summary().unwrap();
        assert_eq!((d.progress.activities, d.progress.minutes, d.progress.met), (3, 8, true));
        assert_eq!(d.progress.ratio, 1.0);
        assert_eq!(d.streak.current, 1);

        s.set_daily_goal(&DailyGoal { kind: "minutes".into(), target: 20 }).unwrap();
        let d = s.daily_summary().unwrap();
        assert_eq!((d.progress.value, d.progress.target, d.progress.met), (8, 20, false));
    }
}
