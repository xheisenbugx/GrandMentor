//! Integration tests for `/api/adaptive/*` and the adaptive bot.

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
        "gm-adaptive-test-{}-{}",
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
    let content = gm_content::Content::load(&data).expect("content");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(1, 4),
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
    let v = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
    (status, v)
}

async fn save_game(app: &Router, bot: &str, result: &str, color: &str, start_fen: &str) -> i64 {
    let (s, g) = req(
        app,
        Method::POST,
        "/api/games",
        Some(json!({
            "white": "Me", "black": "Bot", "result": result, "termination": "resignation",
            "start_fen": start_fen, "moves": if start_fen.is_empty() { json!(["e2e4", "e7e5", "g1f3"]) } else { json!(["e2e3", "e8d8"]) },
            "bot_id": bot, "user_color": color
        })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{g}");
    g["id"].as_i64().expect("id")
}

#[tokio::test]
async fn estimate_and_results() {
    let app = test_app();
    let (s, e) = req(&app, Method::GET, "/api/adaptive/estimate", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(e["rating"].is_null());
    assert_eq!(e["games"], 0);
    assert_eq!(e["provisional"], true);
    assert!(e["bot_level"].is_number());

    // A win as white against Max (1200).
    let id = save_game(&app, "max", "1-0", "white", "").await;
    let (s, r) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": id }))).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["counted"], true);
    assert!(r["previous"].is_null());
    assert!(r["delta"].as_i64().unwrap() > 0);
    assert_eq!(r["estimate"]["games"], 1);
    let rating = r["rating"].as_i64().unwrap();

    // Posting again does not count twice.
    let (_, r2) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": id }))).await;
    assert_eq!(r2["counted"], false);
    assert_eq!(r2["rating"].as_i64().unwrap(), rating);
    assert_eq!(r2["estimate"]["games"], 1);

    // Black loses: result 1-0 as black is a loss.
    let id = save_game(&app, "max", "1-0", "black", "").await;
    let (_, r) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": id }))).await;
    assert!(r["delta"].as_i64().unwrap() < 0, "{r}");

    // Custom positions and unfinished games are skipped.
    let id = save_game(&app, "max", "1-0", "white", "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1").await;
    let (s, r) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": id }))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(r["skipped"], "custom_position");
    assert!(r.get("counted").is_none());
    let id = save_game(&app, "max", "*", "white", "").await;
    let (_, r) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": id }))).await;
    assert_eq!(r["skipped"], "unfinished");
    assert_eq!(r["estimate"]["games"], 2);

    // Beating the adaptive bot raises its level, also in /api/bots.
    let (_, before) = req(&app, Method::GET, "/api/adaptive/estimate", None).await;
    let id = save_game(&app, "adaptive", "0-1", "black", "").await;
    let (_, r) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": id }))).await;
    assert!(r["bot_level_delta"].as_i64().unwrap() > 0, "{r}");
    assert_eq!(
        r["estimate"]["bot_level"].as_i64().unwrap(),
        before["bot_level"].as_i64().unwrap() + r["bot_level_delta"].as_i64().unwrap()
    );
    let (_, bots) = req(&app, Method::GET, "/api/bots", None).await;
    let sparky = bots.as_array().unwrap().iter().find(|b| b["id"] == "adaptive").expect("adaptive bot");
    assert_eq!(sparky["elo"], r["estimate"]["bot_level"]);

    // The adaptive bot answers moves.
    let (s, m) = req(
        &app,
        Method::POST,
        "/api/bot/move",
        Some(json!({ "bot_id": "adaptive", "start_fen": "", "moves": ["e2e4"] })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert!(m["uci"].is_string());
}

#[tokio::test]
async fn bad_requests() {
    let app = test_app();
    let (s, v) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());
    let (s, v) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": 999 }))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["error"].is_string());
    let (s, _) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": "x" }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // Local game (no bot): skipped.
    let (_, g) = req(
        &app,
        Method::POST,
        "/api/games",
        Some(json!({ "result": "1-0", "moves": ["e2e4", "e7e5"], "user_color": "white" })),
    )
    .await;
    let (_, r) = req(&app, Method::POST, "/api/adaptive/result", Some(json!({ "game_id": g["id"] }))).await;
    assert_eq!(r["skipped"], "not_bot");
}
