//! Daily plan, streaks and goals (`/api/daily*`, `/api/activity*`).
//!
//! The server owns the activity log, streaks and the daily goal; the task list of the daily plan
//! is composed client-side (it needs data from several feature endpoints). All days are UTC.

use axum::extract::State;
use axum::routing::{get, put};
use axum::{Json, Router};
use gm_store::activity::{ActivityDay, DailyGoal, DailySummary, KINDS, MAX_DAYS};
use serde::{Deserialize, Serialize};

use crate::api::store_op;
use crate::error::{ApiError, ApiJson, ApiQuery, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/daily", get(get_daily))
        .route("/daily/goal", put(put_goal))
        .route("/activity", get(get_activity).post(post_activity))
}

/// `GET /api/daily` — today's goal progress, streak and the last 7 days.
async fn get_daily(State(st): State<AppState>) -> ApiResult<Json<DailySummary>> {
    Ok(Json(store_op(&st.store, |s| s.daily_summary()).await?))
}

/// `PUT /api/daily/goal` — `{ "kind": "minutes"|"activities", "target": n }`; returns the summary.
async fn put_goal(
    State(st): State<AppState>,
    ApiJson(goal): ApiJson<DailyGoal>,
) -> ApiResult<Json<DailySummary>> {
    let goal = goal.validated().map_err(|e| ApiError::bad_request(e.to_string()))?;
    let summary = store_op(&st.store, move |s| {
        s.set_daily_goal(&goal)?;
        s.daily_summary()
    })
    .await?;
    Ok(Json(summary))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ActivityQuery {
    days: Option<u32>,
}

#[derive(Serialize)]
struct ActivityResponse {
    days: u32,
    /// Only days with activity, most recent first.
    items: Vec<ActivityDay>,
}

/// `GET /api/activity?days=N` (1..=400, default 84).
async fn get_activity(
    State(st): State<AppState>,
    ApiQuery(q): ApiQuery<ActivityQuery>,
) -> ApiResult<Json<ActivityResponse>> {
    let days = q.days.unwrap_or(84).clamp(1, MAX_DAYS);
    let items = store_op(&st.store, move |s| s.activity_days(days)).await?;
    Ok(Json(ActivityResponse { days, items }))
}

#[derive(Deserialize)]
struct LogActivity {
    kind: String,
    #[serde(default = "one")]
    n: u32,
}

fn one() -> u32 {
    1
}

/// `POST /api/activity` — `{ "kind": "...", "n": 1 }` for client-only activities (e.g. reading a
/// classic game). `n` is 1..=20. Returns the updated summary.
async fn post_activity(
    State(st): State<AppState>,
    ApiJson(body): ApiJson<LogActivity>,
) -> ApiResult<Json<DailySummary>> {
    if !KINDS.contains(&body.kind.as_str()) {
        return Err(ApiError::bad_request(format!("unknown activity kind (expected one of: {})", KINDS.join(", "))));
    }
    if body.n == 0 || body.n > 20 {
        return Err(ApiError::bad_request("n must be between 1 and 20"));
    }
    let summary = store_op(&st.store, move |s| {
        s.log_activity(&body.kind, body.n)?;
        s.daily_summary()
    })
    .await?;
    Ok(Json(summary))
}
