//! Guided first week (`/api/first-week*`): a 7-day path for new players.
//!
//! The plan and the completion rules live in `gm_store::first_week`; this module only adds the
//! puzzle-theme lookup (the store does not know the content) and maps errors to HTTP.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use gm_content::Content;
use gm_store::first_week::{self as fw, FirstWeekState};
use gm_store::Store;
use serde::Deserialize;

use crate::api::store_op;
use crate::error::{ApiError, ApiJson, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/first-week", get(get_state))
        .route("/first-week/start", post(start))
        .route("/first-week/restart", post(restart))
        .route("/first-week/dismiss", post(dismiss))
        .route("/first-week/step", post(set_step))
}

/// Runs `f` with a puzzle-theme matcher backed by the content.
async fn with_themes<F>(st: &AppState, f: F) -> ApiResult<Json<FirstWeekState>>
where
    F: FnOnce(&Store, &dyn Fn(&str, &[&str]) -> bool) -> anyhow::Result<FirstWeekState> + Send + 'static,
{
    let content: Arc<Content> = st.content.clone();
    store_op(&st.store, move |s| f(s, &|id: &str, themes: &[&str]| has_theme(&content, id, themes)))
        .await
        .map(Json)
}

fn has_theme(content: &Content, puzzle_id: &str, themes: &[&str]) -> bool {
    content.puzzle(puzzle_id).is_some_and(|p| p.themes.iter().any(|t| themes.contains(&t.as_str())))
}

/// `GET /api/first-week` — the 7-day path with progress (persists newly detected steps).
async fn get_state(State(st): State<AppState>) -> ApiResult<Json<FirstWeekState>> {
    with_themes(&st, |s, th| s.first_week(th)).await
}

/// `POST /api/first-week/start` — starts the path today (idempotent) and un-dismisses it.
async fn start(State(st): State<AppState>) -> ApiResult<Json<FirstWeekState>> {
    with_themes(&st, |s, th| s.first_week_start(th)).await
}

/// `POST /api/first-week/restart` — starts over from Day 1 today.
async fn restart(State(st): State<AppState>) -> ApiResult<Json<FirstWeekState>> {
    with_themes(&st, |s, th| s.first_week_restart(th)).await
}

#[derive(Deserialize)]
struct DismissBody {
    #[serde(default = "yes")]
    dismissed: bool,
}

fn yes() -> bool {
    true
}

/// `POST /api/first-week/dismiss` — `{ "dismissed": true|false }`.
async fn dismiss(State(st): State<AppState>, ApiJson(body): ApiJson<DismissBody>) -> ApiResult<Json<FirstWeekState>> {
    with_themes(&st, move |s, th| s.first_week_dismiss(body.dismissed, th)).await
}

#[derive(Deserialize)]
struct StepBody {
    step_id: String,
    #[serde(default = "yes")]
    done: bool,
}

/// `POST /api/first-week/step` — `{ "step_id": "...", "done": true|false }` marks a step done by
/// hand (or undoes a manual mark). 400 for unknown steps, locked days or a path not started.
async fn set_step(State(st): State<AppState>, ApiJson(body): ApiJson<StepBody>) -> ApiResult<Json<FirstWeekState>> {
    let id = body.step_id.trim().to_string();
    if id.is_empty() || id.len() > 64 || fw::find_step(&id).is_none() {
        return Err(ApiError::bad_request("unknown step_id"));
    }
    let content = st.content.clone();
    let res = store_op(&st.store, move |s| {
        let matcher = |pid: &str, themes: &[&str]| has_theme(&content, pid, themes);
        Ok(s.first_week_set_step(&id, body.done, &matcher).map_err(|e| {
            if fw::is_user_error(&e) {
                ApiError::bad_request(e.to_string())
            } else {
                ApiError::from(e)
            }
        }))
    })
    .await?;
    Ok(Json(res?))
}

#[cfg(test)]
mod tests {
    use gm_store::first_week::{Rule, BEGINNER_BOTS, COACH_BOTS, PLAN};

    /// Every deep link and id in the plan exists in the real content / bot list.
    #[test]
    fn plan_links_resolve() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let content = gm_content::Content::load(&data).expect("content");
        for day in PLAN {
            for s in day.steps {
                match s.rule {
                    Rule::Lesson { course, lesson } => {
                        let c = content.course(course).unwrap_or_else(|| panic!("course {course}"));
                        assert!(c.lessons.iter().any(|l| l.id == lesson), "lesson {course}/{lesson}");
                        assert_eq!(s.href, format!("#/learn/{course}/{lesson}"));
                    }
                    Rule::Drill(id) => {
                        assert!(gm_store::drills::variants_of(id).is_some(), "drill {id}");
                        assert_eq!(s.href, format!("#/drills/{id}"));
                    }
                    Rule::Endgame(id) => {
                        assert!(content.endgame(id).is_some(), "endgame {id}");
                        assert_eq!(s.href, format!("#/endgames/{id}"));
                    }
                    Rule::GameVs(bots) => {
                        for b in bots {
                            assert!(gm_bots::exists(b), "bot {b}");
                        }
                        let linked = s.href.trim_start_matches("#/play/");
                        assert!(bots.contains(&linked), "{} links to a bot the rule accepts", s.id);
                    }
                    Rule::Puzzles { n, themes } => {
                        for th in themes {
                            let count = content.puzzles.iter().filter(|p| p.themes.iter().any(|t| t == th)).count();
                            assert!(count >= n as usize * 10, "theme {th} has {count} puzzles");
                        }
                    }
                    Rule::Review | Rule::DailyGoal => {}
                }
            }
        }
        for b in COACH_BOTS {
            assert_eq!(gm_bots::get(b, gm_content::Lang::En).map(|p| p.category), Some("coach".to_string()));
        }
        for b in BEGINNER_BOTS {
            assert_eq!(gm_bots::get(b, gm_content::Lang::En).map(|p| p.category), Some("beginner".to_string()));
        }
    }
}
