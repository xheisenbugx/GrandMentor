//! Quick drills: coordinates, counting material, hanging pieces (`/api/drills*`).

use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
}
