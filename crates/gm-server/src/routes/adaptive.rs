//! Adaptive bot and estimated playing rating (`/api/adaptive*`).

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
