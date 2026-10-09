//! Insights: personal weakness tracker built from reviewed games (`/api/insights*`).
//!
//! * `GET /api/insights` — the aggregated report ([`gm_analysis::insights::Insights`]) plus
//!   library counts. Computed server-side from stored reviews, cached in a small bounded cache
//!   keyed by the games' ids/updated_at (any game change invalidates it) and the language.
//! * `POST /api/insights/review-next` — reviews ONE not-yet-reviewed game of the user through
//!   the regular review pipeline and stores it like `POST /api/review {game_id}` does. The page
//!   calls it in a loop (bounded on the client) and aborts the request to cancel: the review
//!   future is awaited inline, so a dropped request stops its engine searches.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use shakmaty::Color;

use gm_analysis::insights::{self, GameInput, Insights};
use gm_store::{GamePatch, GameQuery, GameSummary};

use crate::api::{blocking, store_op, validate_moves, MAX_REVIEW_PLIES};
use crate::cache::BoundedCache;
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::lang::ReqLang;
use crate::state::AppState;

/// Games scanned for the report (most recent first).
const MAX_SCAN: u32 = 500;
/// Cached reports (one per language / data version / server instance).
const CACHE_CAPACITY: usize = 8;
/// Depth used for background reviews (a touch lighter than an interactive review).
const BATCH_REVIEW_DEPTH: u8 = 12;
/// Games shorter than this are not worth reviewing for insights.
const MIN_PLIES: u32 = 6;
/// Most ids accepted in `skip`.
const MAX_SKIP: usize = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/insights", get(get_insights))
        .route("/insights/review-next", post(review_next))
}

#[derive(Serialize, Clone, Debug)]
pub struct InsightsResponse {
    #[serde(flatten)]
    pub insights: Insights,
    /// Games where we know which side the user played.
    pub total_games: u32,
    /// Of those, games without a stored review.
    pub unreviewed: u32,
    /// Unreviewed games long enough to be reviewed by `review-next`.
    pub reviewable: u32,
}

fn cache() -> &'static BoundedCache<String, Arc<InsightsResponse>> {
    static CACHE: OnceLock<BoundedCache<String, Arc<InsightsResponse>>> = OnceLock::new();
    CACHE.get_or_init(|| BoundedCache::new(CACHE_CAPACITY))
}

/// The side the user played: `user_color`, else a case-insensitive match of the profile name.
fn user_side(g: &GameSummary, name: &str) -> Option<Color> {
    match g.user_color.as_deref() {
        Some("white") => return Some(Color::White),
        Some("black") => return Some(Color::Black),
        _ => {}
    }
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("player") {
        return None;
    }
    match (g.white.trim().eq_ignore_ascii_case(name), g.black.trim().eq_ignore_ascii_case(name)) {
        (true, false) => Some(Color::White),
        (false, true) => Some(Color::Black),
        _ => None,
    }
}

fn is_reviewed(g: &GameSummary) -> bool {
    g.accuracy_white.is_some() || g.accuracy_black.is_some()
}

/// User games (most recent first) with their side, plus the profile name.
fn user_games(store: &gm_store::Store) -> anyhow::Result<Vec<(GameSummary, Color)>> {
    let name = store.get_profile()?.name;
    let list = store.list_games(&GameQuery { limit: Some(MAX_SCAN), ..Default::default() })?;
    Ok(list
        .into_iter()
        .filter_map(|g| user_side(&g, &name).map(|c| (g, c)))
        .collect())
}

async fn get_insights(State(st): State<AppState>, ReqLang(lang): ReqLang) -> ApiResult<Json<InsightsResponse>> {
    let games = store_op(&st.store, user_games).await?;
    let total_games = games.len() as u32;
    let unreviewed: Vec<&GameSummary> = games.iter().map(|(g, _)| g).filter(|g| !is_reviewed(g)).collect();
    let reviewable = unreviewed.iter().filter(|g| g.move_count >= MIN_PLIES).count() as u32;
    let unreviewed = unreviewed.len() as u32;

    // Any create/update/delete changes this key. The content pointer keeps separate server
    // instances (tests) apart.
    let mut key = format!("{}|{:p}|{}", lang.code(), Arc::as_ptr(&st.content), games.len());
    for (g, c) in &games {
        if is_reviewed(g) {
            key.push_str(&format!("|{}:{}:{}", g.id, g.updated_at, c == &Color::White));
        }
    }
    let key = format!("{}#{}", key.len(), fnv(&key));
    if let Some(hit) = cache().get(&key) {
        return Ok(Json((*hit).clone()));
    }

    let reviewed: Vec<(i64, Color)> = games
        .iter()
        .filter(|(g, _)| is_reviewed(g))
        .take(insights::MAX_GAMES)
        .map(|(g, c)| (g.id, *c))
        .collect();
    let report = store_op(&st.store, move |s| {
        let mut inputs = Vec::with_capacity(reviewed.len());
        for (id, user) in reviewed {
            let Some(g) = s.get_game(id)? else { continue };
            let Some((review, _)) = g.review_json.as_deref().and_then(gm_analysis::from_stored_json) else {
                continue;
            };
            inputs.push(GameInput {
                id: g.id,
                user,
                result: g.result,
                date: g.created_at,
                opening: g.opening_name,
                clocks: insights::parse_clocks(&g.pgn, review.moves.len()),
                review,
            });
        }
        Ok(insights::aggregate(&inputs, lang))
    })
    .await?;

    let resp = Arc::new(InsightsResponse { insights: report, total_games, unreviewed, reviewable });
    cache().insert(key, Arc::clone(&resp));
    Ok(Json((*resp).clone()))
}

/// 64-bit FNV-1a (cache keys only).
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ReviewNextReq {
    /// Game ids to skip (e.g. ones that failed earlier in this batch).
    skip: Vec<i64>,
}

#[derive(Serialize)]
struct ReviewNextResp {
    /// The game just reviewed, or null when nothing is left.
    game_id: Option<i64>,
    /// Reviewable games still waiting after this one.
    remaining: u32,
}

/// Only one background review at a time (each one already uses the whole engine pool).
static BUSY: AtomicBool = AtomicBool::new(false);

struct BusyGuard;

impl BusyGuard {
    fn acquire() -> Option<BusyGuard> {
        BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).ok().map(|_| BusyGuard)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

async fn review_next(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiJson(req): ApiJson<ReviewNextReq>,
) -> ApiResult<Json<ReviewNextResp>> {
    if req.skip.len() > MAX_SKIP {
        return Err(ApiError::bad_request(format!("too many ids in skip (max {MAX_SKIP})")));
    }
    let _guard = BusyGuard::acquire()
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "a review is already running"))?;

    let skip = req.skip;
    let pending: Vec<i64> = store_op(&st.store, move |s| {
        Ok(user_games(s)?
            .into_iter()
            .filter(|(g, _)| !is_reviewed(g) && g.move_count >= MIN_PLIES && !skip.contains(&g.id))
            .map(|(g, _)| g.id)
            .collect())
    })
    .await?;
    let Some(&id) = pending.first() else {
        return Ok(Json(ReviewNextResp { game_id: None, remaining: 0 }));
    };
    let remaining = (pending.len() - 1) as u32;

    let game = store_op(&st.store, move |s| s.get_game(id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("game {id} not found")))?;
    let start_fen = if game.start_fen.trim().is_empty() {
        gm_engine::START_FEN.to_string()
    } else {
        game.start_fen.clone()
    };
    let mut moves = game.moves;
    moves.truncate(MAX_REVIEW_PLIES);
    validate_moves(&start_fen, &moves, MAX_REVIEW_PLIES)?;

    // If the client aborts (or the server shuts down) the review's stop flag halts every
    // in-flight search.
    let review = crate::api::run_review(&st, start_fen, moves, BATCH_REVIEW_DEPTH, lang).await?;
    let patch = GamePatch {
        review_json: gm_analysis::to_stored_json(&review, lang),
        accuracy_white: Some(review.white.accuracy),
        accuracy_black: Some(review.black.accuracy),
        ..Default::default()
    };
    // Make sure the work isn't lost once it's done, even if the client left meanwhile.
    let store = st.store.clone();
    blocking(move || store.update_game(id, &patch)).await?.map_err(ApiError::from)?;
    Ok(Json(ReviewNextResp { game_id: Some(id), remaining }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(white: &str, black: &str, user_color: Option<&str>) -> GameSummary {
        GameSummary {
            white: white.into(),
            black: black.into(),
            user_color: user_color.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn side_detection() {
        assert_eq!(user_side(&summary("A", "B", Some("black")), ""), Some(Color::Black));
        assert_eq!(user_side(&summary("Ana", "Bob", None), "ana"), Some(Color::White));
        assert_eq!(user_side(&summary("Ana", "Bob", None), "Bob "), Some(Color::Black));
        assert_eq!(user_side(&summary("Ana", "Bob", None), "Player"), None);
        assert_eq!(user_side(&summary("Ana", "Ana", None), "Ana"), None);
        assert_eq!(user_side(&summary("Ana", "Bob", None), ""), None);
    }

    #[test]
    fn busy_guard_is_exclusive() {
        let g = BusyGuard::acquire();
        assert!(g.is_some());
        assert!(BusyGuard::acquire().is_none());
        drop(g);
        let again = BusyGuard::acquire();
        assert!(again.is_some());
    }

    #[test]
    fn fnv_differs() {
        assert_ne!(fnv("a"), fnv("b"));
    }
}
