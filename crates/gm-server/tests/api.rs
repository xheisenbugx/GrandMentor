//! Integration tests: drive the full router (layers included) with `tower::ServiceExt::oneshot`.

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

const ITALIAN_FEN: &str = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";

/// Fresh temp dir with a tiny content set and a tiny web root.
fn fixture_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "gm-server-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let data = dir.join("data");
    let web = dir.join("web");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(web.join("vendor")).unwrap();
    std::fs::create_dir_all(web.join("js")).unwrap();
    std::fs::write(
        data.join("openings.json"),
        json!([
            {"id":"kings-pawn","eco":"B00","name":"King's Pawn Game","family":"King's Pawn","moves":"e4 e5","side":"white","popularity":8,"level":"beginner","description":"d","ideas":[],"traps":[]},
            {"id":"italian-game","eco":"C50","name":"Italian Game","family":"Italian Game","moves":"e4 e5 Nf3 Nc6 Bc4","side":"white","popularity":10,"level":"beginner","description":"d","ideas":["Castle"],"traps":[]},
            {"id":"ruy-lopez","eco":"C60","name":"Ruy Lopez","family":"Ruy Lopez","moves":"e4 e5 Nf3 Nc6 Bb5","side":"white","popularity":9,"level":"intermediate","description":"d","ideas":[],"traps":[]},
            {"id":"sicilian","eco":"B20","name":"Sicilian Defense","family":"Sicilian","moves":"e4 c5","side":"black","popularity":10,"level":"beginner","description":"d","ideas":[],"traps":[]}
        ])
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        data.join("puzzles.json"),
        json!([
            {"id":"p1","fen":"6k1/5ppp/8/8/8/8/5PPP/R5K1 b - - 0 1","moves":["h7h6","a1a8"],"rating":900,"themes":["mateIn1","backRankMate"],"popularity":90},
            {"id":"p2","fen":"6k1/5ppp/8/8/8/8/5PPP/R5K1 b - - 0 1","moves":["g7g6","a1a8"],"rating":1400,"themes":["mateIn1"],"popularity":80}
        ])
        .to_string(),
    )
    .unwrap();
    std::fs::write(data.join("courses.json"), "[]").unwrap();
    std::fs::write(data.join("endgames.json"), "[]").unwrap();
    std::fs::write(
        web.join("index.html"),
        "<!doctype html><title>GrandMentor</title>",
    )
    .unwrap();
    std::fs::write(web.join("vendor/lib.js"), "export const x = 1;".repeat(100)).unwrap();
    std::fs::write(web.join("js/app.js"), "console.log('app');").unwrap();
    dir
}

fn test_app() -> Router {
    let dir = fixture_dir();
    let content = gm_content::Content::load(&dir.join("data")).expect("content");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(2, 4),
        gm_store::Store::open_in_memory().expect("store"),
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    app(state, &dir.join("web"))
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, bytes)
}

async fn json_req(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (s, _, b) = send(app, method, uri, body).await;
    let v = if b.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&b)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&b).into()))
    };
    (s, v)
}

#[tokio::test]
async fn health_reports_engines() {
    let app = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/health", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["ok"], true);
    assert_eq!(v["engines"], 2);
    assert!(v["version"].is_string());
    assert!(v["llm_enabled"].is_boolean());
}

#[tokio::test]
async fn bots_list() {
    let app = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/bots", None).await;
    assert_eq!(s, StatusCode::OK);
    let bots = v.as_array().expect("array");
    assert!(!bots.is_empty());
    assert!(bots
        .iter()
        .all(|b| b["id"].is_string() && b["elo"].is_number()));
}

#[tokio::test]
async fn errors_are_json() {
    let app = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["error"].is_string());

    // malformed JSON
    let res = app
        .clone()
        .oneshot(
            Request::post("/api/games")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let v: Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(v["error"].as_str().unwrap().contains("JSON"));

    // body too large (> 1MB)
    let big = json!({ "pgn": "x".repeat(1_100_000) });
    let (s, v) = json_req(&app, Method::POST, "/api/games/import", Some(big)).await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(v["error"].is_string());

    // bad fen
    let (s, v) = json_req(&app, Method::GET, "/api/openings/lookup?fen=garbage", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());

    // bad id
    let (s, v) = json_req(&app, Method::GET, "/api/games/abc", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn games_crud() {
    let app = test_app();
    let new_game = json!({
        "white": "Me", "black": "Max", "result": "1-0", "termination": "checkmate",
        "start_fen": "", "moves": ["e2e4","e7e5","g1f3","b8c6","f1c4"],
        "bot_id": "max", "user_color": "white", "notes": "", "tags": ["test"]
    });
    let (s, g) = json_req(&app, Method::POST, "/api/games", Some(new_game)).await;
    assert_eq!(s, StatusCode::CREATED, "{g}");
    let id = g["id"].as_i64().expect("id");
    assert_eq!(g["moves"].as_array().unwrap().len(), 5);
    assert_eq!(g["opening_name"], "Italian Game");

    // illegal moves rejected
    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/games",
        Some(json!({"moves": ["e2e5"]})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());

    let (s, list) = json_req(&app, Method::GET, "/api/games?limit=10&favorite=", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(list.as_array().unwrap().iter().any(|x| x["id"] == id));

    let (s, got) = json_req(&app, Method::GET, &format!("/api/games/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(got["white"], "Me");

    let (s, upd) = json_req(
        &app,
        Method::PUT,
        &format!("/api/games/{id}"),
        Some(json!({"favorite": true, "notes": "nice"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{upd}");
    assert_eq!(upd["favorite"], true);
    assert_eq!(upd["notes"], "nice");

    let (s, h, body) = send(&app, Method::GET, &format!("/api/games/{id}/pgn"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/plain"));
    assert!(h[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .contains("attachment"));
    let pgn = String::from_utf8(body).unwrap();
    assert!(pgn.contains("e4") && pgn.contains("Bc4"), "{pgn}");

    let (s, v) = json_req(&app, Method::DELETE, &format!("/api/games/{id}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["deleted"], true);
    let (s, _) = json_req(&app, Method::GET, &format!("/api/games/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = json_req(&app, Method::DELETE, &format!("/api/games/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn games_import() {
    let app = test_app();
    let pgn = "[Event \"x\"]\n[White \"A\"]\n[Black \"B\"]\n[Result \"0-1\"]\n\n1. f3 e5 2. g4 Qh4# 0-1\n";
    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/games/import",
        Some(json!({ "pgn": pgn })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let games = v.as_array().unwrap();
    assert_eq!(games.len(), 1);
    assert_eq!(games[0]["result"], "0-1");
    assert_eq!(games[0]["moves"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn openings_list_get_lookup() {
    let app = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/openings", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 4);

    let (s, v) = json_req(&app, Method::GET, "/api/openings?side=black", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["id"], "sicilian");

    let (s, v) = json_req(&app, Method::GET, "/api/openings?q=italian", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v[0]["id"], "italian-game");

    let (s, v) = json_req(&app, Method::GET, "/api/openings/italian-game", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["uci"].as_array().unwrap().len(), 5);
    let (s, _) = json_req(&app, Method::GET, "/api/openings/nope", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let uri = format!("/api/openings/lookup?fen={}", urlencode(ITALIAN_FEN));
    let (s, v) = json_req(&app, Method::GET, &uri, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["opening"]["name"], "Italian Game");
    // cached path returns the same
    let (s, v2) = json_req(&app, Method::GET, &uri, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v, v2);

    // after 1.e4 e5: King's Pawn, continuation Nf3
    let fen = "rnbqkbnr/pppp1ppp/8/4p3/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2";
    let (s, v) = json_req(
        &app,
        Method::GET,
        &format!("/api/openings/lookup?fen={}", urlencode(fen)),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["opening"]["id"], "kings-pawn");
    assert!(v["continuations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["uci"] == "g1f3"));

    // unknown position -> null
    let fen = "8/8/8/4k3/8/8/8/4K2R w K - 0 1";
    let (s, v) = json_req(
        &app,
        Method::GET,
        &format!("/api/openings/lookup?fen={}", urlencode(fen)),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(v.is_null());
}

#[tokio::test]
async fn puzzles_endpoints() {
    let app = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/puzzles/themes", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v[0]["theme"], "mateIn1");
    assert_eq!(v[0]["count"], 2);

    let (s, a) = json_req(&app, Method::GET, "/api/puzzles/daily", None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, b) = json_req(&app, Method::GET, "/api/puzzles/daily", None).await;
    assert_eq!(a["id"], b["id"]);

    let (s, v) = json_req(
        &app,
        Method::GET,
        "/api/puzzles/next?theme=backRankMate",
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["id"], "p1");
    let (s, _) = json_req(&app, Method::GET, "/api/puzzles/next?theme=nothing", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = json_req(&app, Method::GET, "/api/puzzles/next?min=abc", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v) = json_req(&app, Method::GET, "/api/puzzles/rush?count=40", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v.as_array().unwrap().len(), 2);
    assert!(v[0]["rating"].as_u64() <= v[1]["rating"].as_u64());

    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/puzzles/p1/attempt",
        Some(json!({"solved": true, "time_ms": 5000})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["rating"].is_number() && v["delta"].is_number());
    let (s, _) = json_req(
        &app,
        Method::POST,
        "/api/puzzles/zzz/attempt",
        Some(json!({"solved": true})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/puzzles/rush",
        Some(json!({"score": 12})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["best"], 12);
}

#[tokio::test]
async fn profile_progress_stats() {
    let app = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/profile", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = json_req(
        &app,
        Method::PUT,
        "/api/profile",
        Some(json!({"name": "Ozzy"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["name"], "Ozzy");
    let (s, _) = json_req(
        &app,
        Method::PUT,
        "/api/profile",
        Some(json!({"settings_json": "{bad"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/progress",
        Some(json!({"course_id": "c", "lesson_id": "l", "completed": true})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v[0]["lesson_id"], "l");
    let (s, v) = json_req(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = json_req(&app, Method::GET, "/api/courses", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v.is_array());
    let (s, v) = json_req(&app, Method::GET, "/api/endgames", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v.is_array());
}

#[tokio::test]
async fn engine_and_bot() {
    let app = test_app();
    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/engine/analyze",
        Some(json!({"fen": "startpos", "depth": 3, "multipv": 2})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!v["lines"].as_array().unwrap().is_empty());
    assert!(v["lines"][0]["score"].is_object());

    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/engine/analyze",
        Some(json!({"fen": "bad fen"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].is_string());

    let (s, bots) = json_req(&app, Method::GET, "/api/bots", None).await;
    assert_eq!(s, StatusCode::OK);
    let bot_id = bots[0]["id"].as_str().unwrap().to_string();
    let (s, v) = json_req(
        &app,
        Method::POST,
        "/api/bot/move",
        Some(json!({"bot_id": bot_id, "start_fen": "", "moves": ["e2e4"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["uci"].as_str().unwrap().len() >= 4);
    let (s, _) = json_req(
        &app,
        Method::POST,
        "/api/bot/move",
        Some(json!({"bot_id": "nobody", "moves": []})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn review_small_game_and_cache_into_game() {
    let app = test_app();
    let (s, g) = json_req(
        &app,
        Method::POST,
        "/api/games",
        Some(json!({"result": "0-1", "moves": ["f2f3","e7e5","g2g4","d8h4"]})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let id = g["id"].as_i64().unwrap();
    let (s, r) = json_req(
        &app,
        Method::POST,
        "/api/review",
        Some(json!({"game_id": id, "depth": 4})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["moves"].as_array().unwrap().len(), 4);
    assert_eq!(r["evals"].as_array().unwrap().len(), 5);
    let (_, g) = json_req(&app, Method::GET, &format!("/api/games/{id}"), None).await;
    assert!(g["review_json"].is_string());
    assert!(g["accuracy_white"].is_number());

    let (s, r) = json_req(
        &app,
        Method::POST,
        "/api/review",
        Some(json!({"moves": ["e2e4", "e7e5"], "depth": 3})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["moves"].as_array().unwrap().len(), 2);
    let (s, _) = json_req(&app, Method::POST, "/api/review", Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn static_files_and_spa_fallback() {
    let app = test_app();
    let (s, h, body) = send(&app, Method::GET, "/", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CACHE_CONTROL], "no-cache");
    assert!(String::from_utf8_lossy(&body).contains("GrandMentor"));

    let (s, h, body) = send(&app, Method::GET, "/play/max", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CACHE_CONTROL], "no-cache");
    assert!(String::from_utf8_lossy(&body).contains("GrandMentor"));

    let (s, h, _) = send(&app, Method::GET, "/vendor/lib.js", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("immutable"));

    let (s, h, _) = send(&app, Method::GET, "/js/app.js", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CACHE_CONTROL], "no-cache");

    let (s, _, _) = send(&app, Method::GET, "/js/missing.js", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // gzip negotiated
    let res = app
        .clone()
        .oneshot(
            Request::get("/vendor/lib.js")
                .header(header::ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.headers()[header::CONTENT_ENCODING], "gzip");
}

#[tokio::test]
async fn cors_allows_localhost_only() {
    let app = test_app();
    let res = app
        .clone()
        .oneshot(
            Request::get("/api/health")
                .header(header::ORIGIN, "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "http://localhost:5173"
    );
    let res = app
        .clone()
        .oneshot(
            Request::get("/api/health")
                .header(header::ORIGIN, "https://evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
