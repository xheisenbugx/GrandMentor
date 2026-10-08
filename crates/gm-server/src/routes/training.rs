//! Endgame-theory practice vs the engine (`/api/training*`).

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
