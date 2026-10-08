//! Quick drills: coordinates, hanging pieces, counting material, checks/captures, knight routes
//! (`/api/drills*`). See docs/CONTRACT.md "Quick drills".
//!
//! * `GET  /api/drills`                 — every drill with its variants, bests and recent scores
//! * `GET  /api/drills/:id/batch?n=&variant=` — server-generated questions with answers
//! * `POST /api/drills/:id/score`       — record a finished run, returns the personal best

pub mod generate;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use gm_store::drills::{self as store_drills, DrillRecordResult, DrillRun, DrillStats};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{blocking, store_op};
use crate::error::{ApiError, ApiJson, ApiPath, ApiQuery, ApiResult};
use crate::state::AppState;

/// Default number of items per batch.
const DEFAULT_BATCH: usize = 20;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/drills", get(list))
        .route("/drills/:id/batch", get(batch))
        .route("/drills/:id/score", post(score))
}

#[derive(Serialize)]
struct DrillList {
    drills: Vec<DrillStats>,
}

async fn list(State(st): State<AppState>) -> ApiResult<Json<DrillList>> {
    let drills = store_op(&st.store, |s| s.drill_stats()).await?;
    Ok(Json(DrillList { drills }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BatchQuery {
    n: Option<usize>,
    variant: Option<String>,
}

/// Resolve `variant` (default: the drill's first variant) or fail with 400/404.
fn resolve_variant(id: &str, variant: Option<&str>) -> ApiResult<&'static str> {
    let variants = store_drills::variants_of(id).ok_or_else(|| ApiError::not_found(format!("drill {id} not found")))?;
    match variant.map(str::trim).filter(|v| !v.is_empty()) {
        None => variants.first().copied().ok_or_else(|| ApiError::internal("drill has no variants")),
        Some(v) => variants
            .iter()
            .copied()
            .find(|known| *known == v)
            .ok_or_else(|| ApiError::bad_request(format!("unknown variant: {}", crate::api::clip(v, 32)))),
    }
}

async fn batch(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiQuery(q): ApiQuery<BatchQuery>,
) -> ApiResult<Json<Value>> {
    let variant = resolve_variant(&id, q.variant.as_deref())?;
    let n = q.n.unwrap_or(DEFAULT_BATCH).clamp(1, generate::MAX_BATCH);
    let content = st.content.clone();
    let id_owned = id.clone();
    let items = blocking(move || -> ApiResult<Value> {
        let mut rng = rand::thread_rng();
        let puzzles = &content.puzzles;
        let v = match id_owned.as_str() {
            "hanging" => serde_json::to_value(generate::hanging_batch(puzzles, n, &mut rng)),
            "material" => serde_json::to_value(generate::material_batch(puzzles, n, &mut rng)),
            "checks" => serde_json::to_value(generate::checks_batch(puzzles, n, variant == "captures", &mut rng)),
            "knight" => serde_json::to_value(generate::knight_batch(n, variant == "advanced", &mut rng)),
            _ => return Err(ApiError::bad_request("this drill has no server batch")),
        };
        v.map_err(|e| ApiError::internal(format!("serializing batch: {e}")))
    })
    .await??;
    Ok(Json(json!({ "drill": id, "variant": variant, "items": items })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ScoreBody {
    variant: Option<String>,
    score: u32,
    correct: u32,
    total: u32,
    duration_ms: u32,
}

async fn score(
    State(st): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<ScoreBody>,
) -> ApiResult<Json<DrillRecordResult>> {
    let variant = resolve_variant(&id, body.variant.as_deref())?;
    if body.score > store_drills::MAX_SCORE || body.total > store_drills::MAX_SCORE {
        return Err(ApiError::bad_request("score out of range"));
    }
    if body.correct > body.total {
        return Err(ApiError::bad_request("correct cannot exceed total"));
    }
    let run = DrillRun { score: body.score, correct: body.correct, total: body.total, duration_ms: body.duration_ms };
    let res = store_op(&st.store, move |s| {
        let res = s.record_drill(&id, variant, &run)?;
        if let Err(e) = s.log_activity("drill", 1) {
            tracing::warn!("logging drill activity: {e:#}");
        }
        Ok(res)
    })
    .await?;
    Ok(Json(res))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_resolve() {
        assert_eq!(resolve_variant("checks", None).unwrap(), "checks");
        assert_eq!(resolve_variant("checks", Some("captures")).unwrap(), "captures");
        assert_eq!(resolve_variant("knight", Some(" ")).unwrap(), "basic");
        assert_eq!(resolve_variant("nope", None).unwrap_err().status.as_u16(), 404);
        assert_eq!(resolve_variant("knight", Some("x")).unwrap_err().status.as_u16(), 400);
    }
}
