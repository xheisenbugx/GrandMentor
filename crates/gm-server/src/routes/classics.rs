//! Annotated classic games library (`/api/classics*`).
//!
//! * `GET  /api/classics`              — localized list (summaries + the user's progress)
//! * `GET  /api/classics/:id`          — one localized game with moves, annotations, questions, progress
//! * `POST /api/classics/:id/progress` — record position / completion / a question answer

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use gm_content::{Classic, ClassicSummary};
use gm_store::classics::{ClassicAnswer, ClassicProgress, ClassicProgressUpdate};

use crate::api::store_op;
use crate::error::{ApiError, ApiJson, ApiPath, ApiResult};
use crate::lang::ReqLang;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/classics", get(list))
        .route("/classics/:id", get(detail))
        .route("/classics/:id/progress", post(progress))
}

#[derive(Serialize)]
struct ListItem {
    #[serde(flatten)]
    game: ClassicSummary,
    progress: Option<ClassicProgress>,
}

#[derive(Serialize)]
struct Detail {
    #[serde(flatten)]
    game: Classic,
    progress: Option<ClassicProgress>,
}

async fn list(State(st): State<AppState>, ReqLang(lang): ReqLang) -> ApiResult<Json<Vec<ListItem>>> {
    let mut all = store_op(&st.store, |s| s.classic_progress_all()).await?;
    let content = st.content.localized(lang);
    let items = content
        .classics
        .iter()
        .map(|g| {
            let progress = all.iter().position(|p| p.classic_id == g.id).map(|i| all.swap_remove(i));
            ListItem { game: g.summary(), progress }
        })
        .collect();
    Ok(Json(items))
}

async fn detail(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiPath(id): ApiPath<String>,
) -> ApiResult<Json<Detail>> {
    let game = st
        .content
        .localized(lang)
        .classic(&id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("classic game {id:?} not found")))?;
    let progress = store_op(&st.store, move |s| s.classic_progress(&id)).await?;
    Ok(Json(Detail { game, progress }))
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct ProgressReq {
    ply: Option<u32>,
    completed: Option<bool>,
    answer: Option<ClassicAnswer>,
}

async fn progress(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(req): ApiJson<ProgressReq>,
) -> ApiResult<Json<ClassicProgress>> {
    let game = st.content.classic(&id).ok_or_else(|| ApiError::not_found(format!("classic game {id:?} not found")))?;
    let update = validate_update(game, req)?;
    let p = store_op(&st.store, move |s| {
        let (p, newly_completed) = s.update_classic_progress(&id, &update)?;
        if newly_completed {
            let _ = s.log_activity("classic", 1);
        }
        Ok(p)
    })
    .await?;
    Ok(Json(p))
}

/// Check plies against the game; at least one field must be set.
fn validate_update(game: &Classic, req: ProgressReq) -> ApiResult<ClassicProgressUpdate> {
    if req.ply.is_none() && req.completed.is_none() && req.answer.is_none() {
        return Err(ApiError::bad_request("send at least one of `ply`, `completed`, `answer`"));
    }
    if let Some(p) = req.ply {
        if p as usize > game.plies {
            return Err(ApiError::bad_request(format!("`ply` must be between 0 and {}", game.plies)));
        }
    }
    if let Some(a) = &req.answer {
        if !game.questions.iter().any(|q| q.ply == a.ply as usize) {
            return Err(ApiError::bad_request("`answer.ply` is not a question of this game"));
        }
    }
    Ok(ClassicProgressUpdate { ply: req.ply, completed: req.completed, answer: req.answer })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gm_content::ClassicQuestion;

    fn game() -> Classic {
        Classic {
            id: "g".into(),
            plies: 10,
            questions: vec![ClassicQuestion { ply: 7, ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn validates_progress_requests() {
        let g = game();
        assert!(validate_update(&g, ProgressReq::default()).is_err());
        assert!(validate_update(&g, ProgressReq { ply: Some(11), ..Default::default() }).is_err());
        assert!(validate_update(&g, ProgressReq { ply: Some(10), completed: Some(true), ..Default::default() }).is_ok());
        let ans = |ply| ProgressReq { answer: Some(ClassicAnswer { ply, correct: true }), ..Default::default() };
        assert!(validate_update(&g, ans(6)).is_err());
        let up = validate_update(&g, ans(7)).unwrap();
        assert_eq!(up.answer, Some(ClassicAnswer { ply: 7, correct: true }));
    }
}
