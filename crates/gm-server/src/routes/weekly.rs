//! Your weekly set: a personal puzzle set per ISO week built from the user's weakest tactic
//! themes (`/api/weekly*`). See docs/CONTRACT.md "Weekly personal set".
//!
//! * `GET  /api/weekly`            — this week's set (built and stored on first request)
//! * `POST /api/weekly/attempt`    — `{set_id, index, solved, time_ms}`; the first attempt counts
//! * `POST /api/weekly/regenerate` — "New set": a fresh set for this week
//! * `GET  /api/weekly/history?weeks=N` — per-theme solve rate of the last N weeks (1..=12)
//!
//! Themes are ranked from the rated puzzle history (fail rate per theme, recency weighted) and
//! from the mistakes found in reviewed games (each mistake card classified by tactic theme).
//! All weeks are UTC.

pub mod plan;

use std::collections::{HashMap, HashSet};

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use gm_content::Content;
use gm_store::weekly::{WeekThemeStat, WeeklyItem, WeeklyProgress, WeeklySet};
use gm_store::{PuzzleResult, Store};

use crate::api::{blocking, store_op};
use crate::error::{ApiError, ApiJson, ApiQuery, ApiResult};
use crate::state::AppState;
use plan::{FocusTheme, GameObs, IsoWeek, OwnCandidate, Pick, PuzzleObs, SetInput};

/// Mistake cards scanned when building a set.
const MAX_OWN_SCAN: u32 = 400;
/// Puzzle attempts scanned when ranking.
const MAX_PUZZLE_LOG: u32 = 3_000;
/// Puzzles attempted this recently are avoided in a new set.
const AVOID_DAYS: u32 = 30;
/// Most weeks returned by the history.
const MAX_HISTORY_WEEKS: u32 = 12;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/weekly", get(current))
        .route("/weekly/attempt", post(attempt))
        .route("/weekly/regenerate", post(regenerate))
        .route("/weekly/history", get(history))
}

/// Serializes set creation so two concurrent first requests don't build two sets.
fn build_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn this_week() -> IsoWeek {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    IsoWeek::of_day(secs.div_euclid(86_400))
}

// ---------------------------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------------------------

/// The solver-ready puzzle of an item (same shape as a pack puzzle, `userFirst` for own-game
/// positions without the opponent's previous move).
#[derive(Serialize)]
struct ItemPuzzle {
    id: String,
    fen: String,
    moves: Vec<String>,
    rating: u32,
    themes: Vec<String>,
    #[serde(rename = "userFirst", skip_serializing_if = "std::ops::Not::not")]
    user_first: bool,
}

#[derive(Serialize)]
struct ItemView {
    index: u32,
    /// "puzzle" | "mistake"
    kind: String,
    /// Focus theme the item trains ("" when none).
    theme: String,
    puzzle: ItemPuzzle,
    /// Own-game context (mistake items): card_id, game_id, opponent, move_number, played_san, best_san.
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    game: serde_json::Value,
    /// null until attempted, then "solved" | "failed".
    result: Option<&'static str>,
    time_ms: Option<u64>,
}

#[derive(Serialize)]
struct SetView {
    week: String,
    week_start: String,
    week_end: String,
    set_id: i64,
    generation: u32,
    created_at: String,
    rating: u32,
    focus: serde_json::Value,
    items: Vec<ItemView>,
    progress: WeeklyProgress,
    finished: bool,
}

fn result_of(solved: Option<bool>) -> Option<&'static str> {
    solved.map(|s| if s { "solved" } else { "failed" })
}

fn item_view(content: &Content, it: &WeeklyItem) -> ItemView {
    let themes = if it.kind == "puzzle" {
        content.puzzle(&it.ref_id).map(|p| p.themes.clone()).unwrap_or_default()
    } else if it.theme.is_empty() {
        Vec::new()
    } else {
        vec![it.theme.clone()]
    };
    ItemView {
        index: it.index,
        kind: it.kind.clone(),
        theme: it.theme.clone(),
        puzzle: ItemPuzzle {
            id: if it.kind == "puzzle" { it.ref_id.clone() } else { format!("mistake-{}", it.ref_id) },
            fen: it.fen.clone(),
            moves: it.moves.clone(),
            rating: it.rating,
            themes,
            user_first: it.user_first,
        },
        game: if it.kind == "mistake" { it.meta.clone() } else { serde_json::Value::Null },
        result: result_of(it.solved),
        time_ms: it.time_ms,
    }
}

fn set_view(content: &Content, set: &WeeklySet, week: &IsoWeek) -> SetView {
    let progress = set.progress();
    SetView {
        week: set.week.clone(),
        week_start: week.start(),
        week_end: week.end(),
        set_id: set.id,
        generation: set.generation,
        created_at: set.created_at.clone(),
        rating: set.rating,
        focus: set.focus.clone(),
        items: set.items.iter().map(|i| item_view(content, i)).collect(),
        finished: progress.total > 0 && progress.done >= progress.total,
        progress,
    }
}

// ---------------------------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------------------------

/// Ranks the themes and builds + stores a new set for `week` (blocking: SQLite + chess work).
fn build_and_store(store: &Store, content: &Content, week: &IsoWeek) -> anyhow::Result<WeeklySet> {
    let profile = store.get_profile()?;
    let rating = if profile.puzzle_rating.is_finite() && profile.puzzle_rating > 0.0 {
        profile.puzzle_rating.round().clamp(400.0, 3000.0) as u16
    } else {
        1200
    };
    let log = store.weekly_puzzle_log(plan::WINDOW_DAYS as u32, MAX_PUZZLE_LOG)?;
    let puzzle_obs: Vec<PuzzleObs> = log
        .iter()
        .filter_map(|e| {
            content.puzzle(&e.puzzle_id).map(|p| PuzzleObs { themes: p.themes.clone(), solved: e.solved, age_days: e.age_days })
        })
        .collect();
    let avoid: HashSet<String> = log
        .iter()
        .filter(|e| e.age_days <= f64::from(AVOID_DAYS))
        .map(|e| e.puzzle_id.clone())
        .collect();

    let cards = store.weekly_mistake_positions(MAX_OWN_SCAN)?;
    let classified: Vec<(usize, Option<&'static str>)> =
        cards.iter().enumerate().map(|(i, c)| (i, plan::classify_mistake(c))).collect();
    let game_obs: Vec<GameObs> = classified
        .iter()
        .filter_map(|(i, t)| t.map(|t| GameObs { theme: t.to_string(), age_days: cards[*i].age_days }))
        .collect();

    // Themes with enough pack puzzles to train.
    let mut theme_counts: HashMap<&str, usize> = HashMap::new();
    for p in &content.puzzles {
        for t in &p.themes {
            if let Some(&tt) = plan::TRACKED.iter().find(|x| **x == t.as_str()) {
                *theme_counts.entry(tt).or_default() += 1;
            }
        }
    }
    let ranked = plan::rank_themes(&puzzle_obs, &game_obs, |t| {
        theme_counts.get(t).copied().unwrap_or(0) >= plan::MIN_THEME_PUZZLES
    });
    let focus: Vec<FocusTheme> = plan::pick_focus(&ranked);

    let own: Vec<OwnCandidate> = classified
        .iter()
        .filter_map(|(i, t)| {
            let c = &cards[*i];
            // Must be solvable: a legal best move and a non-empty solution.
            let pos = gm_engine::parse_fen(&c.fen).ok()?;
            gm_engine::uci_to_move(&pos, &c.best_uci).ok()?;
            Some(OwnCandidate {
                card_id: c.id,
                theme: t.map(str::to_string),
                age_days: c.age_days,
                win_chance_loss: c.win_chance_loss,
                graduated: c.graduated,
            })
        })
        .collect();

    let generation = store.weekly_current(&week.key)?.map(|s| s.generation).unwrap_or(0) + 1;
    let picks = plan::build_set(&SetInput {
        puzzles: &content.puzzles,
        rating,
        focus: &focus,
        avoid: &avoid,
        own: &own,
        seed: plan::seed_for(&week.key, generation),
    });

    let by_id: HashMap<i64, &gm_store::weekly::MistakePosition> = cards.iter().map(|c| (c.id, c)).collect();
    let items: Vec<gm_store::weekly::NewWeeklyItem> = picks
        .iter()
        .filter_map(|p| match p {
            Pick::Puzzle { index, theme } => {
                let pz = content.puzzles.get(*index)?;
                Some(gm_store::weekly::NewWeeklyItem {
                    kind: "puzzle".into(),
                    ref_id: pz.id.clone(),
                    theme: theme.clone(),
                    rating: u32::from(pz.rating),
                    fen: pz.fen.clone(),
                    moves: pz.moves.clone(),
                    user_first: false,
                    meta: serde_json::Value::Null,
                })
            }
            Pick::Own { card_id, theme } => {
                let c = by_id.get(card_id)?;
                let solution = if c.solution.first() == Some(&c.best_uci) { c.solution.clone() } else { vec![c.best_uci.clone()] };
                let (fen, moves, user_first) = match (&c.prev_fen, &c.prev_uci) {
                    (Some(pf), Some(pu)) => {
                        let mut m = vec![pu.clone()];
                        m.extend(solution);
                        (pf.clone(), m, false)
                    }
                    _ => (c.fen.clone(), solution, true),
                };
                Some(gm_store::weekly::NewWeeklyItem {
                    kind: "mistake".into(),
                    ref_id: c.id.to_string(),
                    theme: theme.clone().unwrap_or_default(),
                    rating: 0,
                    fen,
                    moves,
                    user_first,
                    meta: json!({
                        "card_id": c.id,
                        "game_id": c.game_id,
                        "opponent": c.opponent.chars().take(60).collect::<String>(),
                        "move_number": c.move_number,
                        "played_san": c.played_san,
                        "best_san": c.best_san,
                        "classification": c.classification,
                    }),
                })
            }
        })
        .collect();

    let focus_json = serde_json::to_value(&focus)?;
    store.weekly_create(&week.key, u32::from(rating), &focus_json, &items)
}

async fn current_or_build(st: &AppState, week: &IsoWeek, force_new: bool) -> ApiResult<WeeklySet> {
    let _guard = build_lock().lock().await;
    let (store, content, w) = (st.store.clone(), st.content.clone(), week.clone());
    blocking(move || -> anyhow::Result<WeeklySet> {
        if !force_new {
            if let Some(set) = store.weekly_current(&w.key)? {
                return Ok(set);
            }
        }
        build_and_store(&store, &content, &w)
    })
    .await?
    .map_err(ApiError::from)
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

async fn current(State(st): State<AppState>) -> ApiResult<Json<SetView>> {
    let week = this_week();
    let set = current_or_build(&st, &week, false).await?;
    Ok(Json(set_view(&st.content, &set, &week)))
}

async fn regenerate(State(st): State<AppState>) -> ApiResult<Json<SetView>> {
    let week = this_week();
    let set = current_or_build(&st, &week, true).await?;
    Ok(Json(set_view(&st.content, &set, &week)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptRequest {
    set_id: i64,
    index: u32,
    solved: bool,
    #[serde(default)]
    time_ms: u64,
}

#[derive(Serialize)]
struct AttemptResponse {
    item: ItemView,
    /// False when the item already had a result (only the first attempt counts).
    counted: bool,
    progress: WeeklyProgress,
    finished: bool,
    /// New puzzle rating for pack puzzles (first attempt only).
    #[serde(skip_serializing_if = "Option::is_none")]
    rating: Option<PuzzleResult>,
}

async fn attempt(State(st): State<AppState>, ApiJson(req): ApiJson<AttemptRequest>) -> ApiResult<Json<AttemptResponse>> {
    if req.index as usize >= gm_store::weekly::MAX_ITEMS {
        return Err(ApiError::bad_request("index out of range"));
    }
    let content = st.content.clone();
    let time_ms = req.time_ms.min(24 * 3600 * 1000);
    let res = store_op(&st.store, move |s| {
        let Some(a) = s.weekly_record(req.set_id, req.index, req.solved, time_ms)? else {
            return Ok(None);
        };
        let mut rating = None;
        if a.counted {
            if a.item.kind == "puzzle" {
                if let Some(p) = content.puzzle(&a.item.ref_id) {
                    rating = Some(s.record_puzzle_attempt(&p.id, p.rating, req.solved, time_ms)?);
                    let _ = s.log_activity("puzzle", 1);
                }
            } else if let Ok(card_id) = a.item.ref_id.parse::<i64>() {
                // Keep the mistakes deck in step (best effort; the card may have been removed).
                let _ = s.attempt_mistake(card_id, req.solved, time_ms);
                let _ = s.log_activity("mistake_review", 1);
            }
        }
        Ok(Some((a, rating)))
    })
    .await?;
    let Some((a, rating)) = res else {
        return Err(ApiError::not_found("no such weekly set item"));
    };
    let finished = a.progress.total > 0 && a.progress.done >= a.progress.total;
    Ok(Json(AttemptResponse {
        item: item_view(&st.content, &a.item),
        counted: a.counted,
        progress: a.progress,
        finished,
        rating,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct HistoryQuery {
    weeks: Option<u32>,
}

#[derive(Serialize)]
struct ThemeStat {
    theme: String,
    attempted: u32,
    solved: u32,
}

#[derive(Serialize)]
struct WeekView {
    week: String,
    week_start: String,
    /// Progress of the week's current set (null when no set was built that week).
    progress: Option<WeeklyProgress>,
    themes: Vec<ThemeStat>,
}

#[derive(Serialize)]
struct HistoryResponse {
    /// Oldest first; the last entry is this week.
    weeks: Vec<WeekView>,
}

async fn history(State(st): State<AppState>, ApiQuery(q): ApiQuery<HistoryQuery>) -> ApiResult<Json<HistoryResponse>> {
    let n = q.weeks.unwrap_or(6);
    if n == 0 || n > MAX_HISTORY_WEEKS {
        return Err(ApiError::bad_request(format!("weeks must be between 1 and {MAX_HISTORY_WEEKS}")));
    }
    let now = this_week();
    let weeks: Vec<IsoWeek> = (0..i64::from(n)).rev().map(|k| now.back(k)).collect();
    let keys: Vec<String> = weeks.iter().map(|w| w.key.clone()).collect();
    let (stats, progress) = store_op(&st.store, move |s| {
        let stats: Vec<WeekThemeStat> = s.weekly_theme_stats(&keys)?;
        let progress = keys.iter().map(|k| s.weekly_progress(k)).collect::<anyhow::Result<Vec<_>>>()?;
        Ok((stats, progress))
    })
    .await?;
    let weeks = weeks
        .into_iter()
        .zip(progress)
        .map(|(w, progress)| WeekView {
            themes: stats
                .iter()
                .filter(|s| s.week == w.key)
                .map(|s| ThemeStat { theme: s.theme.clone(), attempted: s.attempted, solved: s.solved })
                .collect(),
            week_start: w.start(),
            week: w.key,
            progress,
        })
        .collect();
    Ok(Json(HistoryResponse { weeks }))
}
