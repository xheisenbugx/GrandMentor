//! Review cancellation, games-list thumbnails (`final_fen`), Puzzle Rush bests per mode and
//! the puzzle stats reset.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use gm_server::{app, AppState};

/// 16 plies of a Ruy Lopez: at depth 22 this review takes far longer than any test timeout.
const LONG_GAME: &str = "e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6 e1g1 f8e7 f1e1 b7b5 a4b3 d7d6 c2c3 e8g8";

fn fixture_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "gm-server-polish-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let data = dir.join("data");
    let web = dir.join("web");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(data.join("openings.json"), "[]").unwrap();
    std::fs::write(
        data.join("puzzles.json"),
        json!([
            {"id":"p1","fen":"6k1/5ppp/8/8/8/8/5PPP/R5K1 b - - 0 1","moves":["h7h6","a1a8"],"rating":900,"themes":["mateIn1"],"popularity":90}
        ])
        .to_string(),
    )
    .unwrap();
    std::fs::write(data.join("courses.json"), "[]").unwrap();
    std::fs::write(data.join("endgames.json"), "[]").unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>GrandMentor</title>").unwrap();
    dir
}

fn test_app() -> (Router, AppState) {
    let dir = fixture_dir();
    let content = gm_content::Content::load(&dir.join("data")).expect("content");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(2, 4),
        gm_store::Store::open_in_memory().expect("store"),
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    (app(state.clone(), &dir.join("web")), state)
}

fn request(method: Method, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    req.body(body).unwrap()
}

async fn json_req(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(request(method, uri, body)).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn wait_pool_idle(st: &AppState, within: Duration) -> Duration {
    let t0 = Instant::now();
    while st.pool.available() < st.pool.size() {
        assert!(t0.elapsed() < within, "engine pool still busy after {:?}", t0.elapsed());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    t0.elapsed()
}

fn long_moves() -> Value {
    json!(LONG_GAME.split_whitespace().collect::<Vec<_>>())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_is_cancelled_when_client_disconnects() {
    let (app, st) = test_app();
    let fut = app.clone().oneshot(request(
        Method::POST,
        "/api/review",
        Some(json!({ "moves": long_moves(), "depth": 22 })),
    ));
    // The client gives up after 300 ms: the handler future is dropped.
    assert!(tokio::time::timeout(Duration::from_millis(300), fut).await.is_err());
    assert!(st.pool.available() < st.pool.size(), "the review should have been running");
    let idle = wait_pool_idle(&st, Duration::from_secs(2)).await;
    eprintln!("pool idle {idle:?} after the client disconnected");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review_is_cancelled_on_shutdown() {
    let (app, st) = test_app();
    let shutdown = Arc::clone(&st.shutdown);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        shutdown.send_replace(true);
    });
    let t0 = Instant::now();
    let (status, body) =
        json_req(&app, Method::POST, "/api/review", Some(json!({ "moves": long_moves(), "depth": 22 }))).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(t0.elapsed() < Duration::from_secs(3), "took {:?}", t0.elapsed());
    wait_pool_idle(&st, Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn games_list_has_final_fen() {
    let (app, _) = test_app();
    let (s, _) = json_req(
        &app,
        Method::POST,
        "/api/games",
        Some(json!({ "moves": ["e2e4", "e7e5", "g1f3"], "result": "*" })),
    )
    .await;
    assert!(s.is_success());
    let (s, list) = json_req(&app, Method::GET, "/api/games", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        list[0]["final_fen"],
        "rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rush_bests_per_mode() {
    let (app, _) = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/puzzles/rush/bests", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({ "bests": {}, "overall": 0 }));

    let (s, v) = json_req(&app, Method::POST, "/api/puzzles/rush", Some(json!({ "score": 12, "mode": "3" }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({ "best": 12, "mode_best": 12, "previous_mode_best": 0 }));
    let (_, v) = json_req(&app, Method::POST, "/api/puzzles/rush", Some(json!({ "score": 8, "mode": "3" }))).await;
    assert_eq!(v, json!({ "best": 12, "mode_best": 12, "previous_mode_best": 12 }));
    // Old clients (no mode) still work.
    let (_, v) = json_req(&app, Method::POST, "/api/puzzles/rush", Some(json!({ "score": 4 }))).await;
    assert_eq!(v, json!({ "best": 12 }));
    let (s, _) = json_req(&app, Method::POST, "/api/puzzles/rush", Some(json!({ "score": 4, "mode": "<x>" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Migrating the browser's old local bests: the max wins.
    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/puzzles/rush/bests",
        Some(json!({ "bests": { "3": 9, "5": 21, "survival": 30 } })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, json!({ "bests": { "3": 12, "5": 21, "survival": 30 }, "overall": 30 }));
    let (s, _) = json_req(&app, Method::POST, "/api/puzzles/rush/bests", Some(json!({ "bests": { "BAD": 1 } }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_puzzle_stats() {
    let (app, _) = test_app();
    let (s, _) = json_req(&app, Method::POST, "/api/puzzles/p1/attempt", Some(json!({ "solved": true, "time_ms": 1000 }))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, stats) = json_req(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(stats["puzzles_solved"], 1);
    assert_eq!(stats["rating_history"].as_array().map(Vec::len), Some(1));

    let (s, _) = json_req(&app, Method::POST, "/api/profile/puzzles/reset", Some(json!({ "confirm": false }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, p) = json_req(&app, Method::POST, "/api/profile/puzzles/reset", Some(json!({ "confirm": true }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(p["puzzle_rating"], 1200.0);
    assert_eq!(p["puzzles_solved"], 0);
    let (_, stats) = json_req(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(stats["puzzles_solved"], 0);
    assert_eq!(stats["rating_history"], json!([]));
}
