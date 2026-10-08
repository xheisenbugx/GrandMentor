//! Integration tests for `/api/insights` and `/api/insights/review-next`.

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

fn test_app() -> Router {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "gm-insights-test-{}-{}",
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
    std::fs::write(web.join("index.html"), "<!doctype html><title>t</title>").unwrap();
    let content = gm_content::Content::load(&data).expect("content");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(2, 4),
        gm_store::Store::open_in_memory().expect("store"),
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    app(state, &web)
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

fn moves(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insights_empty_then_review_and_aggregate() {
    let app = test_app();
    let (s, v) = req(&app, Method::GET, "/api/insights", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ready"], false);
    assert_eq!(v["games_analyzed"], 0);
    assert_eq!(v["min_games"], 3);
    assert!(v["time_trouble"].is_null());
    assert_eq!(v["phases"].as_array().map(Vec::len), Some(3));

    // Three games where the user (white) hangs the queen, plus a too-short one and a game
    // where we can't tell the user's side.
    let blunder = "e2e4 e7e5 d1h5 g8f6 h5f7 e8f7 g1f3 d7d5";
    for _ in 0..3 {
        let (s, v) = req(
            &app,
            Method::POST,
            "/api/games",
            Some(json!({"white":"You","black":"Bot","result":"0-1","moves":moves(blunder),"user_color":"white","bot_id":"martin"})),
        )
        .await;
        assert!(s.is_success(), "{v}");
    }
    req(&app, Method::POST, "/api/games", Some(json!({"result":"*","moves":["e2e4","e7e5"],"user_color":"white"}))).await;
    req(&app, Method::POST, "/api/games", Some(json!({"white":"X","black":"Y","result":"1-0","moves":moves(blunder)}))).await;

    let (_, v) = req(&app, Method::GET, "/api/insights", None).await;
    assert_eq!(v["total_games"], 4);
    assert_eq!(v["unreviewed"], 4);
    assert_eq!(v["reviewable"], 3);

    // Review them one by one.
    let mut reviewed = 0;
    loop {
        let (s, v) = req(&app, Method::POST, "/api/insights/review-next", Some(json!({"skip": []}))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        if v["game_id"].is_null() {
            assert_eq!(v["remaining"], 0);
            break;
        }
        reviewed += 1;
        assert!(reviewed <= 3, "loop must end");
    }
    assert_eq!(reviewed, 3);

    let (s, v) = req(&app, Method::GET, "/api/insights?lang=es", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ready"], true, "{v}");
    assert_eq!(v["games_analyzed"], 3);
    assert_eq!(v["unreviewed"], 1);
    assert_eq!(v["reviewable"], 0);
    let weak = v["weaknesses"].as_array().cloned().unwrap_or_default();
    assert!(!weak.is_empty() && weak.len() <= 3, "{v}");
    for w in &weak {
        assert!(w["title"].as_str().is_some_and(|t| !t.is_empty()));
        assert!(w["drill"]["href"].as_str().is_some_and(|h| h.starts_with("#/")));
    }
    assert_eq!(v["by_color"]["white"]["losses"], 3);
    assert_eq!(v["accuracy_trend"].as_array().map(Vec::len), Some(3));

    // Cached answer is identical; the other language has its own text.
    let (_, again) = req(&app, Method::GET, "/api/insights?lang=es", None).await;
    assert_eq!(again, v);
    let (_, en) = req(&app, Method::GET, "/api/insights?lang=en", None).await;
    assert_ne!(en["weaknesses"], v["weaknesses"]);
}

#[tokio::test]
async fn review_next_validates_input() {
    let app = test_app();
    let skip: Vec<i64> = (0..500).collect();
    let (s, _) = req(&app, Method::POST, "/api/insights/review-next", Some(json!({"skip": skip}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, v) = req(&app, Method::POST, "/api/insights/review-next", Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["game_id"].is_null());
}
