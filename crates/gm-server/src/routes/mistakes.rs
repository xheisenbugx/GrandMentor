//! Learn from your mistakes: spaced-repetition puzzles from the user's own games (`/api/mistakes*`).

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
