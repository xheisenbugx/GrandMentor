//! Puzzle profile extras: Puzzle Rush personal bests per mode (`/api/puzzles/rush/bests`) and
//! resetting the puzzle rating / stats (`/api/profile/puzzles/reset`).

use std::collections::BTreeMap;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use gm_store::puzzle_profile::{valid_mode, MAX_MODES};
use gm_store::Profile;

use crate::api::store_op;
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/puzzles/rush/bests", get(get_bests).post(merge_bests))
        .route("/profile/puzzles/reset", post(reset_puzzles))
}

#[derive(Serialize)]
pub struct BestsResponse {
    /// Personal best per Rush mode (`"3"`, `"5"`, `"survival"`, ...). Missing = never played.
    pub bests: BTreeMap<String, u32>,
    /// Best Rush score over every mode (the profile's `rush_best`).
    pub overall: u32,
}

async fn bests_response(st: &AppState) -> ApiResult<BestsResponse> {
    store_op(&st.store, |s| {
        let bests = s.rush_bests()?;
        let overall = s.get_profile()?.rush_best.max(bests.values().copied().max().unwrap_or(0));
        Ok(BestsResponse { bests, overall })
    })
    .await
}

async fn get_bests(State(st): State<AppState>) -> ApiResult<Json<BestsResponse>> {
    Ok(Json(bests_response(&st).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeRequest {
    bests: BTreeMap<String, u32>,
}

/// Merge bests kept elsewhere (the browser's old local copy): the higher score wins per mode.
async fn merge_bests(
    State(st): State<AppState>,
    ApiJson(req): ApiJson<MergeRequest>,
) -> ApiResult<Json<BestsResponse>> {
    if req.bests.len() > MAX_MODES {
        return Err(ApiError::bad_request(format!("at most {MAX_MODES} modes")));
    }
    if let Some(bad) = req.bests.keys().find(|m| !valid_mode(m)) {
        return Err(ApiError::bad_request(format!("invalid Puzzle Rush mode {bad:?}")));
    }
    let bests = req.bests;
    store_op(&st.store, move |s| s.merge_rush_bests(&bests)).await?;
    Ok(Json(bests_response(&st).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetRequest {
    /// Must be `true`: a guard against accidental calls.
    confirm: bool,
}

/// Puzzle rating back to the start, solved / failed counters, attempts and rating history
/// cleared. Rush scores and everything else are kept. Returns the updated profile.
async fn reset_puzzles(
    State(st): State<AppState>,
    ApiJson(req): ApiJson<ResetRequest>,
) -> ApiResult<Json<Profile>> {
    if !req.confirm {
        return Err(ApiError::bad_request("send {\"confirm\": true} to reset puzzle stats"));
    }
    let profile = store_op(&st.store, |s| {
        s.reset_puzzle_stats()?;
        s.get_profile()
    })
    .await?;
    Ok(Json(profile))
}
