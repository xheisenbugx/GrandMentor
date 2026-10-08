//! Backup export / import and device sync (`/api/backup*`, `/api/sync*`).

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
