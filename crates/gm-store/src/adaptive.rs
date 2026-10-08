//! Adaptive bot state and the user's estimated playing rating from games against bots.
//!
//! * **Estimated rating** — an Elo-style estimate updated after every finished, saved game
//!   against a bot (from the standard starting position). It starts from a prior of
//!   [`PRIOR_RATING`] with a large K-factor so the first few games calibrate it quickly; it is
//!   "provisional" until [`PROVISIONAL_GAMES`] games have been counted.
//! * **Adaptive bot level** — the strength the adaptive bot (`gm_bots::ADAPTIVE_ID`) plays at.
//!   It moves up after the user beats it and down after a loss (a shrinking staircase), which
//!   keeps the user's score against it near 50%.
//!
//! Every game is counted at most once (keyed by game id), so retries never double count.
//!
//! Tables are created by [`schema`], which runs inside schema migration v2 (see `lib.rs`).

use anyhow::Context;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

/// Rating the estimate starts from before the first counted game.
pub const PRIOR_RATING: f64 = 800.0;
/// The estimate is shown as provisional until this many games were counted.
pub const PROVISIONAL_GAMES: u32 = 5;
pub const MIN_RATING: f64 = 100.0;
pub const MAX_RATING: f64 = 3000.0;
/// Level of the adaptive bot before its first game when the user has no estimate yet.
pub const START_LEVEL: f64 = 600.0;
pub const MIN_LEVEL: f64 = 250.0;
pub const MAX_LEVEL: f64 = 2800.0;

/// The user's estimated playing strength plus the adaptive bot's level.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AdaptiveEstimate {
    /// Rounded estimate, `None` before the first counted game.
    pub rating: Option<i32>,
    /// Games counted towards the estimate.
    pub games: u32,
    pub provisional: bool,
    /// Elo the adaptive bot plays at next.
    pub bot_level: i32,
    /// Games played against the adaptive bot.
    pub bot_games: u32,
}

/// Outcome of [`Store::adaptive_record_game`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AdaptiveUpdate {
    /// False when the game was already counted (the stored change is returned again).
    pub counted: bool,
    /// Estimate before the game (`None` for the very first counted game).
    pub previous: Option<i32>,
    pub rating: i32,
    /// Rounded change (`rating - previous`, or vs the prior for the first game).
    pub delta: i32,
    pub games: u32,
    pub provisional: bool,
    /// Adaptive bot level after this game.
    pub bot_level: i32,
    /// Change of the adaptive bot's level (0 when the game was against another bot).
    pub bot_level_delta: i32,
}

pub(crate) fn schema(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS adaptive_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  rating REAL,
  games INTEGER NOT NULL DEFAULT 0,
  bot_level REAL,
  bot_games INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS adaptive_games (
  game_id INTEGER PRIMARY KEY,
  opponent_elo INTEGER NOT NULL,
  score REAL NOT NULL,
  rating_before REAL,
  rating_after REAL NOT NULL,
  games_after INTEGER NOT NULL,
  level_before REAL,
  level_after REAL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
"#,
    )
}

/// K-factor for the `games`-th counted game (0-based): fast while provisional, then calmer.
pub fn k_factor(games: u32) -> f64 {
    match games {
        0..=4 => 80.0,
        5..=14 => 48.0,
        _ => 32.0,
    }
}

/// Expected score of a player rated `r` against `opp`.
pub fn expected(r: f64, opp: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((opp - r) / 400.0))
}

/// New rating after scoring `score` (1, 0.5 or 0) against `opp`, given `games` counted so far.
pub fn update_rating(rating: Option<f64>, games: u32, opp: f64, score: f64) -> f64 {
    let r = rating.filter(|r| r.is_finite()).unwrap_or(PRIOR_RATING).clamp(MIN_RATING, MAX_RATING);
    let opp = if opp.is_finite() { opp.clamp(MIN_RATING, MAX_RATING) } else { PRIOR_RATING };
    let s = if score.is_finite() { score.clamp(0.0, 1.0) } else { 0.5 };
    (r + k_factor(games) * (s - expected(r, opp))).clamp(MIN_RATING, MAX_RATING)
}

/// Level step after the `bot_games`-th game against the adaptive bot (0-based): big steps first
/// so it finds the user's level fast, then smaller ones.
pub fn level_step(bot_games: u32) -> f64 {
    (160.0 - 20.0 * f64::from(bot_games.min(100))).max(50.0)
}

/// Adaptive bot level after the user scored `score` against it.
pub fn update_level(level: f64, bot_games: u32, score: f64) -> f64 {
    let s = if score.is_finite() { score.clamp(0.0, 1.0) } else { 0.5 };
    (level + level_step(bot_games) * (s - 0.5) * 2.0).clamp(MIN_LEVEL, MAX_LEVEL)
}

/// Level the adaptive bot starts at: near the user's estimate when there is one.
fn initial_level(rating: Option<f64>) -> f64 {
    rating.filter(|r| r.is_finite()).unwrap_or(START_LEVEL).clamp(MIN_LEVEL, MAX_LEVEL)
}

/// A counted game: (rating_before, rating_after, games_after, level_before, level_after).
type StoredGame = (Option<f64>, f64, u32, Option<f64>, Option<f64>);

struct State {
    rating: Option<f64>,
    games: u32,
    bot_level: Option<f64>,
    bot_games: u32,
}

fn to_u32(n: i64) -> u32 {
    n.clamp(0, i64::from(u32::MAX)) as u32
}

fn load_state(conn: &rusqlite::Connection) -> rusqlite::Result<State> {
    let row = conn
        .query_row(
            "SELECT rating, games, bot_level, bot_games FROM adaptive_state WHERE id = 1",
            [],
            |r| {
                Ok(State {
                    rating: r.get::<_, Option<f64>>(0)?,
                    games: to_u32(r.get(1)?),
                    bot_level: r.get::<_, Option<f64>>(2)?,
                    bot_games: to_u32(r.get(3)?),
                })
            },
        )
        .optional()?;
    Ok(row.unwrap_or(State { rating: None, games: 0, bot_level: None, bot_games: 0 }))
}

fn estimate_of(st: &State) -> AdaptiveEstimate {
    let level = st.bot_level.unwrap_or_else(|| initial_level(st.rating));
    AdaptiveEstimate {
        rating: st.rating.map(|r| r.round() as i32),
        games: st.games,
        provisional: st.games < PROVISIONAL_GAMES,
        bot_level: level.round() as i32,
        bot_games: st.bot_games,
    }
}

impl Store {
    /// The user's estimated rating and the adaptive bot's current level.
    pub fn adaptive_estimate(&self) -> anyhow::Result<AdaptiveEstimate> {
        let conn = self.conn.lock();
        let st = load_state(&conn).context("reading adaptive state")?;
        Ok(estimate_of(&st))
    }

    /// Count a finished game against a bot rated `opponent_elo` in which the user scored `score`
    /// (1 win, 0.5 draw, 0 loss). `vs_adaptive` also moves the adaptive bot's level.
    ///
    /// Idempotent per `game_id`: a game that was already counted changes nothing and returns
    /// the stored result with `counted = false`.
    pub fn adaptive_record_game(
        &self,
        game_id: i64,
        opponent_elo: i32,
        score: f64,
        vs_adaptive: bool,
    ) -> anyhow::Result<AdaptiveUpdate> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let st = load_state(&tx)?;
        let prior: Option<StoredGame> = tx
            .query_row(
                "SELECT rating_before, rating_after, games_after, level_before, level_after \
                 FROM adaptive_games WHERE game_id = ?1",
                params![game_id],
                |r| Ok((r.get(0)?, r.get(1)?, to_u32(r.get(2)?), r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((before, after, games_after, lb, la)) = prior {
            let est = estimate_of(&st);
            return Ok(AdaptiveUpdate {
                counted: false,
                previous: before.map(|r| r.round() as i32),
                rating: after.round() as i32,
                delta: (after - before.unwrap_or(PRIOR_RATING)).round() as i32,
                games: games_after,
                provisional: games_after < PROVISIONAL_GAMES,
                bot_level: est.bot_level,
                bot_level_delta: match (lb, la) {
                    (Some(b), Some(a)) => (a - b).round() as i32,
                    _ => 0,
                },
            });
        }

        let opp = f64::from(opponent_elo);
        let new_rating = update_rating(st.rating, st.games, opp, score);
        let games = st.games.saturating_add(1);
        let level_before = st.bot_level.unwrap_or_else(|| initial_level(st.rating));
        let (level_after, bot_games) = if vs_adaptive {
            (update_level(level_before, st.bot_games, score), st.bot_games.saturating_add(1))
        } else {
            (level_before, st.bot_games)
        };
        // Only pin the level once the adaptive bot has been played; until then it follows the
        // user's estimate.
        let stored_level = if vs_adaptive || st.bot_level.is_some() { Some(level_after) } else { None };

        tx.execute(
            "INSERT INTO adaptive_state (id, rating, games, bot_level, bot_games) VALUES (1, ?1, ?2, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET rating = excluded.rating, games = excluded.games, \
             bot_level = excluded.bot_level, bot_games = excluded.bot_games",
            params![new_rating, games, stored_level, bot_games],
        )?;
        tx.execute(
            "INSERT INTO adaptive_games (game_id, opponent_elo, score, rating_before, rating_after, games_after, level_before, level_after) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                game_id,
                opponent_elo,
                score,
                st.rating,
                new_rating,
                games,
                vs_adaptive.then_some(level_before),
                vs_adaptive.then_some(level_after),
            ],
        )?;
        tx.commit().context("saving adaptive result")?;

        let final_level = stored_level.unwrap_or_else(|| initial_level(Some(new_rating)));
        Ok(AdaptiveUpdate {
            counted: true,
            previous: st.rating.map(|r| r.round() as i32),
            rating: new_rating.round() as i32,
            delta: (new_rating.round() - st.rating.unwrap_or(PRIOR_RATING).round()) as i32,
            games,
            provisional: games < PROVISIONAL_GAMES,
            bot_level: final_level.round() as i32,
            bot_level_delta: if vs_adaptive { (level_after.round() - level_before.round()) as i32 } else { 0 },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_math_is_sane() {
        // Beating a much stronger bot raises the estimate a lot; losing to it barely moves it.
        let up = update_rating(None, 0, 2000.0, 1.0);
        assert!(up > PRIOR_RATING + 70.0, "{up}");
        let down = update_rating(None, 0, 2000.0, 0.0);
        assert!((PRIOR_RATING - down) < 2.0, "{down}");
        // Losing to a weak bot drops it.
        assert!(update_rating(Some(800.0), 3, 300.0, 0.0) < 730.0);
        // Equal opponent, draw: no change.
        assert!((update_rating(Some(1000.0), 20, 1000.0, 0.5) - 1000.0).abs() < 1e-9);
        // Bounded and finite even for garbage input.
        for v in [f64::NAN, f64::INFINITY, -1e9, 1e9] {
            let r = update_rating(Some(v), 0, v, v);
            assert!(r.is_finite() && (MIN_RATING..=MAX_RATING).contains(&r));
        }
        assert!(k_factor(0) > k_factor(10) && k_factor(10) > k_factor(100));
    }

    #[test]
    fn level_staircase_converges() {
        // A user who truly plays at 1100: simulate expected results; the level should settle near.
        let mut level = START_LEVEL;
        for g in 0..40 {
            let s = if expected(1100.0, level) > 0.5 { 1.0 } else { 0.0 };
            level = update_level(level, g, s);
        }
        assert!((level - 1100.0).abs() <= 100.0, "{level}");
        assert_eq!(update_level(MIN_LEVEL, 0, 0.0), MIN_LEVEL);
        assert_eq!(update_level(MAX_LEVEL, 0, 1.0), MAX_LEVEL);
        assert_eq!(update_level(900.0, 0, 0.5), 900.0);
    }

    #[test]
    fn records_games_once_and_tracks_level() {
        let s = Store::open_in_memory().unwrap();
        let e = s.adaptive_estimate().unwrap();
        assert_eq!(e.rating, None);
        assert_eq!(e.games, 0);
        assert!(e.provisional);
        assert_eq!(e.bot_level, START_LEVEL as i32);

        // Win against a 1000 bot.
        let u = s.adaptive_record_game(1, 1000, 1.0, false).unwrap();
        assert!(u.counted);
        assert_eq!(u.previous, None);
        assert!(u.delta > 0);
        assert_eq!(u.games, 1);
        assert_eq!(u.bot_level_delta, 0);
        // The untouched adaptive level follows the estimate.
        assert_eq!(s.adaptive_estimate().unwrap().bot_level, u.rating);

        // Same game again: nothing changes.
        let again = s.adaptive_record_game(1, 1000, 1.0, false).unwrap();
        assert!(!again.counted);
        assert_eq!(again.rating, u.rating);
        assert_eq!(again.delta, u.delta);
        assert_eq!(s.adaptive_estimate().unwrap().games, 1);

        // Beat the adaptive bot: its level goes up, then down after a loss.
        let lvl0 = s.adaptive_estimate().unwrap().bot_level;
        let w = s.adaptive_record_game(2, lvl0, 1.0, true).unwrap();
        assert!(w.bot_level_delta > 0);
        assert_eq!(w.bot_level, lvl0 + w.bot_level_delta);
        let l = s.adaptive_record_game(3, w.bot_level, 0.0, true).unwrap();
        assert!(l.bot_level_delta < 0);
        assert_eq!(l.previous, Some(w.rating));
        let e = s.adaptive_estimate().unwrap();
        assert_eq!(e.bot_games, 2);
        assert_eq!(e.games, 3);
        assert_eq!(e.bot_level, l.bot_level);
        assert_eq!(e.rating, Some(l.rating));
        let replay = s.adaptive_record_game(3, 2000, 1.0, true).unwrap();
        assert!(!replay.counted);
        assert_eq!(replay.bot_level_delta, l.bot_level_delta);

        for id in 4..=6 {
            s.adaptive_record_game(id, 900, 0.5, false).unwrap();
        }
        assert!(!s.adaptive_estimate().unwrap().provisional);
    }
}
