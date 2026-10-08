//! Integration tests for the daily plan / streak / goal endpoints.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use gm_server::{app, AppState};

fn fixture_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "gm-daily-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let data = dir.join("data");
    let web = dir.join("web");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&web).unwrap();
    for f in ["openings.json", "puzzles.json", "courses.json", "endgames.json"] {
        std::fs::write(data.join(f), "[]").unwrap();
    }
    std::fs::write(web.join("index.html"), "<!doctype html><title>GrandMentor</title>").unwrap();
    dir
}

fn test_app() -> Router {
    let dir = fixture_dir();
    let content = gm_content::Content::load(&dir.join("data")).expect("content");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(1, 4),
        gm_store::Store::open_in_memory().expect("store"),
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    app(state, &dir.join("web"))
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
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

#[tokio::test]
async fn daily_summary_goal_and_activity() {
    let app = test_app();
    let (s, v) = req(&app, Method::GET, "/api/daily", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["goal"], json!({"kind": "minutes", "target": 10}));
    assert_eq!(v["streak"]["current"], 0);
    assert_eq!(v["last7"].as_array().unwrap().len(), 7);
    assert_eq!(v["date"].as_str().unwrap().len(), 10);

    let (s, v) = req(&app, Method::PUT, "/api/daily/goal", Some(json!({"kind": "activities", "target": 2}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["goal"]["kind"], "activities");
    let (s, v) = req(&app, Method::PUT, "/api/daily/goal", Some(json!({"kind": "minutes", "target": 0}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());
    let (s, _) = req(&app, Method::PUT, "/api/daily/goal", Some(json!({"kind": "weeks", "target": 2}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, _) = req(&app, Method::POST, "/api/activity", Some(json!({"kind": "nope"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = req(&app, Method::POST, "/api/activity", Some(json!({"kind": "classic", "n": 99}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = req(&app, Method::POST, "/api/activity", Some(json!({"kind": "classic"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["today"]["classic"], 1);
    assert_eq!(v["progress"]["met"], false);
    let (_, v) = req(&app, Method::POST, "/api/activity", Some(json!({"kind": "drill", "n": 1}))).await;
    assert_eq!(v["progress"]["met"], true);
    assert_eq!(v["streak"]["current"], 1);
    assert_eq!(v["streak"]["today_active"], true);

    // The profile reads the same streak.
    let (_, p) = req(&app, Method::GET, "/api/profile", None).await;
    assert_eq!(p["streak_days"], 1);

    let (s, v) = req(&app, Method::GET, "/api/activity?days=7", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["days"], 7);
    assert_eq!(v["items"][0]["total"], 2);
    let (_, v) = req(&app, Method::GET, "/api/activity?days=99999", None).await;
    assert_eq!(v["days"], 400);
}
