//! Opening repertoire builder and drills (`/api/repertoire*`).

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
