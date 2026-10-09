//! Integration tests for the guided first week endpoints.

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
        "gm-firstweek-test-{}-{}",
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
async fn first_week_flow() {
    let app = test_app();
    let (s, v) = req(&app, Method::GET, "/api/first-week", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["started"], false);
    assert_eq!(v["eligible"], true);
    assert_eq!(v["days"].as_array().unwrap().len(), 7);

    let (s, v) = req(&app, Method::POST, "/api/first-week/step", Some(json!({"step_id": "board"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "not started: {v}");

    let (s, v) = req(&app, Method::POST, "/api/first-week/start", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["started"], true);
    assert_eq!(v["unlocked_days"], 1);
    assert_eq!(v["current_day"], 1);

    // Completing a lesson through the normal API is detected.
    let (s, _) = req(
        &app,
        Method::POST,
        "/api/progress",
        Some(json!({"course_id": "chess-basics", "lesson_id": "the-board"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = req(&app, Method::GET, "/api/first-week", None).await;
    assert_eq!(v["days"][0]["steps"][0]["done"], true);
    assert_eq!(v["newly_done"], json!(["board"]));

    let (s, _) = req(&app, Method::POST, "/api/first-week/step", Some(json!({"step_id": "nope"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = req(&app, Method::POST, "/api/first-week/step", Some(json!({"step_id": "coachGame"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "locked day: {v}");
    assert!(v["error"].is_string());

    req(&app, Method::POST, "/api/first-week/step", Some(json!({"step_id": "longMovers"}))).await;
    let (s, v) = req(
        &app,
        Method::POST,
        "/api/first-week/step",
        Some(json!({"step_id": "knightKingPawn", "done": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["newly_completed_days"], json!([1]));
    assert_eq!(v["days"][0]["done"], true);

    let (_, v) = req(&app, Method::POST, "/api/first-week/dismiss", Some(json!({"dismissed": true}))).await;
    assert_eq!(v["dismissed"], true);
    assert_eq!(v["eligible"], false);
    let (_, v) = req(&app, Method::POST, "/api/first-week/restart", None).await;
    assert_eq!(v["dismissed"], false);
    assert_eq!(v["completed_days"], 0);
    // The lesson is still completed, so step 1 is detected again right away.
    assert_eq!(v["days"][0]["steps"][0]["done"], true);
}
