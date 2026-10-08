//! Insights: personal weakness tracker built from reviewed games (`/api/insights*`).

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
