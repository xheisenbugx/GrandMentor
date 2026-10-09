//! Feature route modules. Each module owns its `/api/...` paths and returns a `Router` that is
//! merged into the main API router (see `api::router`). Paths are relative to `/api`.

use axum::Router;

use crate::state::AppState;

pub mod adaptive;
pub mod backup;
pub mod classics;
pub mod daily;
pub mod drills;
pub mod first_week;
pub mod insights;
pub mod mistakes;
pub mod puzzle_profile;
pub mod repertoire;
pub mod training;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(adaptive::router())
        .merge(backup::router())
        .merge(classics::router())
        .merge(daily::router())
        .merge(drills::router())
        .merge(first_week::router())
        .merge(insights::router())
        .merge(mistakes::router())
        .merge(puzzle_profile::router())
        .merge(repertoire::router())
        .merge(training::router())
}
