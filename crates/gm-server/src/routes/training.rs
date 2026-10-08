//! Endgame-theory practice vs the engine (`/api/training*`).
//!
//! The game itself runs in the browser against the regular engine endpoints; these routes only
//! keep score: attempts, successes and the success streak per drill (3 in a row = mastered).

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use gm_store::training::{DrillProgress, MASTERY_STREAK, MAX_ATTEMPT_MOVES};

use crate::api::store_op;
use crate::error::{ApiError, ApiJson, ApiPath, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/training", get(progress))
        .route("/training/:drill_id/attempt", post(attempt))
}

#[derive(Serialize)]
struct ProgressResponse {
    /// Successes in a row needed to master a drill.
    mastery_streak: u32,
    /// One entry per drill in the content (zeros when never attempted).
    drills: Vec<DrillProgress>,
}

async fn progress(State(st): State<AppState>) -> ApiResult<Json<ProgressResponse>> {
    let ids: Vec<String> = st.content.endgames.iter().map(|d| d.id.clone()).collect();
    let stored = store_op(&st.store, |s| s.training_progress()).await?;
    let drills = ids
        .into_iter()
        .map(|id| {
            stored
                .iter()
                .find(|p| p.drill_id == id)
                .cloned()
                .unwrap_or(DrillProgress { drill_id: id, ..Default::default() })
        })
        .collect();
    Ok(Json(ProgressResponse { mastery_streak: MASTERY_STREAK, drills }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttemptRequest {
    success: bool,
    /// Moves the user played in the attempt.
    #[serde(default)]
    moves: u32,
}

#[derive(Serialize)]
struct AttemptResponse {
    #[serde(flatten)]
    progress: DrillProgress,
    /// True when this attempt completed the mastery streak for the first time.
    just_mastered: bool,
    mastery_streak: u32,
}

async fn attempt(
    State(st): State<AppState>,
    ApiPath(drill_id): ApiPath<String>,
    ApiJson(req): ApiJson<AttemptRequest>,
) -> ApiResult<Json<AttemptResponse>> {
    if st.content.endgame(&drill_id).is_none() {
        return Err(ApiError::not_found(format!("endgame {:?} not found", crate::api::clip(&drill_id, 64))));
    }
    if req.moves > MAX_ATTEMPT_MOVES {
        return Err(ApiError::bad_request(format!("moves must be at most {MAX_ATTEMPT_MOVES}")));
    }
    let (progress, just_mastered) = store_op(&st.store, move |s| {
        let was_mastered = s.drill_progress(&drill_id)?.is_some_and(|p| p.mastered);
        let p = s.record_training_attempt(&drill_id, req.success, req.moves)?;
        let _ = s.log_activity("endgame", 1);
        let just = p.mastered && !was_mastered;
        Ok((p, just))
    })
    .await?;
    Ok(Json(AttemptResponse { progress, just_mastered, mastery_streak: MASTERY_STREAK }))
}
