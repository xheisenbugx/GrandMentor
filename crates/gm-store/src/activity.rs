//! Daily activity log used by the daily plan, streaks and goals.
//!
//! Every feature records what the user did with [`Store::log_activity`]; the daily plan reads
//! the per-day totals back with [`Store::activity_days`].

use anyhow::Context;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::Store;

/// Known activity kinds. Unknown kinds are rejected so the table stays tidy.
pub const KINDS: &[&str] = &[
    "game", "puzzle", "lesson", "mistake_review", "repertoire_review", "endgame", "drill", "classic", "local_game",
];

/// Max days returned by [`Store::activity_days`].
pub const MAX_DAYS: u32 = 400;

/// Totals for one UTC day.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ActivityDay {
    /// `YYYY-MM-DD` (UTC).
    pub day: String,
    /// kind -> count
    pub counts: std::collections::BTreeMap<String, u32>,
    /// Sum of all counts.
    pub total: u32,
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
"#,
    )
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
             ON CONFLICT(day, kind) DO UPDATE SET count = count + excluded.count",
            params![kind, n.min(10_000)],
        )
        .context("logging activity")?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
