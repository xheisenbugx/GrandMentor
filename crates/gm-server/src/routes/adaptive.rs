//! Adaptive bot and estimated playing rating (`/api/adaptive*`).
//!
//! * `GET /api/adaptive/estimate` — the user's estimated rating and the adaptive bot's level.
//! * `POST /api/adaptive/result` `{game_id}` — count a saved, finished game against a bot.
//!   Idempotent per game, so a retry never counts a game twice.
//!
//! The adaptive bot itself is played through `POST /api/bot/move` with `bot_id: "adaptive"`;
//! that handler reads the stored level (see `adaptive_level`).

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use gm_store::adaptive::{AdaptiveEstimate, AdaptiveUpdate};

use crate::api::{parse_position, store_op};
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/adaptive/estimate", get(estimate))
        .route("/adaptive/result", post(result))
}

/// Level the adaptive bot should play at right now (falls back to its starting level when the
/// store is unavailable, so a bot move never fails because of it).
pub(crate) async fn adaptive_level(st: &AppState) -> u16 {
    match store_op(&st.store, |s| s.adaptive_estimate()).await {
        Ok(e) => e.bot_level.clamp(i32::from(gm_bots::ADAPTIVE_MIN_ELO), i32::from(gm_bots::ADAPTIVE_MAX_ELO)) as u16,
        Err(_) => gm_bots::ADAPTIVE_START_ELO,
    }
}

async fn estimate(State(st): State<AppState>) -> ApiResult<Json<AdaptiveEstimate>> {
    Ok(Json(store_op(&st.store, |s| s.adaptive_estimate()).await?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ResultReq {
    game_id: Option<i64>,
}

#[derive(Serialize)]
struct ResultResp {
    /// Why the game does not count (`None` when it does). One of `unfinished`, `not_bot`,
    /// `custom_position`, `too_short`.
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped: Option<&'static str>,
    #[serde(flatten)]
    update: Option<AdaptiveUpdate>,
    estimate: AdaptiveEstimate,
}

/// Minimum plies for a game to count (very short games are usually abandoned).
const MIN_PLIES: usize = 2;

/// Same piece placement, side, castling and en passant as the standard start.
fn is_standard_start(fen: &str) -> bool {
    let fen = fen.trim();
    if fen.is_empty() {
        return true;
    }
    let Ok(pos) = parse_position(fen) else { return false };
    let norm = gm_engine::to_fen(&pos);
    let key = |f: &str| f.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
    key(&norm) == key(gm_engine::START_FEN)
}

async fn result(State(st): State<AppState>, ApiJson(req): ApiJson<ResultReq>) -> ApiResult<Json<ResultResp>> {
    let id = req.game_id.ok_or_else(|| ApiError::bad_request("`game_id` is required"))?;
    let game = store_op(&st.store, move |s| s.get_game(id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("game {id} not found")))?;

    let user_white = match game.user_color.as_deref() {
        Some("white") => Some(true),
        Some("black") => Some(false),
        _ => None,
    };
    let bot_id = game.bot_id.clone().filter(|b| gm_bots::exists(b));
    let score_white = match game.result.as_str() {
        "1-0" => Some(1.0),
        "0-1" => Some(0.0),
        "1/2-1/2" => Some(0.5),
        _ => None,
    };
    let skipped = if score_white.is_none() {
        Some("unfinished")
    } else if bot_id.is_none() || user_white.is_none() {
        Some("not_bot")
    } else if !is_standard_start(&game.start_fen) {
        Some("custom_position")
    } else if game.moves.len() < MIN_PLIES {
        Some("too_short")
    } else {
        None
    };

    let update = match (skipped, bot_id, user_white, score_white) {
        (None, Some(bot_id), Some(white), Some(sw)) => {
            let vs_adaptive = bot_id == gm_bots::ADAPTIVE_ID;
            let elo = if vs_adaptive {
                i32::from(adaptive_level(&st).await)
            } else {
                gm_bots::get(&bot_id, gm_content::Lang::En).map(|b| i32::from(b.elo)).unwrap_or(1000)
            };
            let score = if white { sw } else { 1.0 - sw };
            Some(store_op(&st.store, move |s| s.adaptive_record_game(id, elo, score, vs_adaptive)).await?)
        }
        _ => None,
    };
    let estimate = store_op(&st.store, |s| s.adaptive_estimate()).await?;
    Ok(Json(ResultResp { skipped, update, estimate }))
}
