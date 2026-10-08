//! Quick drills endpoints, driven through the full router.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use gm_server::{app, AppState};

fn test_app() -> (Router, gm_store::Store) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = gm_content::Content::load(&root.join("data")).expect("content");
    let store = gm_store::Store::open_in_memory().expect("store");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(1, 4),
        store.clone(),
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    (app(state, &root.join("web")), store)
}

async fn req(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut r = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            r = r.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(r.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn list_batch_and_score() {
    let (app, store) = test_app();

    let (s, v) = req(&app, Method::GET, "/api/drills", None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = v["drills"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["coordinates", "hanging", "material", "checks", "knight"]);
    assert!(v["drills"][0]["variants"][0]["best"].is_null());

    for (id, variant, key) in [
        ("hanging", "standard", "hanging"),
        ("material", "standard", "options"),
        ("checks", "checks", "answers"),
        ("checks", "captures", "answers"),
        ("knight", "basic", "path"),
        ("knight", "advanced", "blocked"),
    ] {
        let (s, v) = req(&app, Method::GET, &format!("/api/drills/{id}/batch?n=7&variant={variant}"), None).await;
        assert_eq!(s, StatusCode::OK, "{id}: {v}");
        assert_eq!(v["variant"], variant);
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 7, "{id}");
        assert!(items.iter().all(|it| it["fen"].is_string() && !it[key].is_null()));
    }
    // n is clamped.
    let (_, v) = req(&app, Method::GET, "/api/drills/material/batch?n=999", None).await;
    assert_eq!(v["items"].as_array().unwrap().len(), 50);

    let (s, _) = req(&app, Method::GET, "/api/drills/coordinates/batch", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = req(&app, Method::GET, "/api/drills/nope/batch", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = req(&app, Method::GET, "/api/drills/knight/batch?variant=zzz", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let body = json!({"variant": "find-black", "score": 14, "correct": 14, "total": 16, "duration_ms": 30000});
    let (s, v) = req(&app, Method::POST, "/api/drills/coordinates/score", Some(body)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["best"], 14);
    assert_eq!(v["is_best"], true);
    assert!(v["previous_best"].is_null());
    let body = json!({"variant": "find-black", "score": 9, "correct": 9, "total": 9, "duration_ms": 30000});
    let (_, v) = req(&app, Method::POST, "/api/drills/coordinates/score", Some(body)).await;
    assert_eq!((v["best"].as_u64(), v["is_best"].as_bool(), v["plays"].as_u64()), (Some(14), Some(false), Some(2)));

    let (s, _) = req(&app, Method::POST, "/api/drills/coordinates/score", Some(json!({"score": 1, "correct": 3, "total": 2}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = req(&app, Method::POST, "/api/drills/nope/score", Some(json!({"score": 1}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (_, v) = req(&app, Method::GET, "/api/drills", None).await;
    let coords = &v["drills"][0]["variants"];
    let fb = coords.as_array().unwrap().iter().find(|x| x["id"] == "find-black").unwrap();
    assert_eq!(fb["best"], 14);
    assert_eq!(fb["recent"], json!([14, 9]));

    // Each run logs a `drill` activity.
    let days = store.activity_days(1).unwrap();
    assert_eq!(days[0].counts.get("drill"), Some(&2));
}
