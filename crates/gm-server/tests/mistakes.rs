//! Integration tests for `/api/mistakes*`: drive the full router (layers included) with `tower::ServiceExt::oneshot`.

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
async fn mistakes_deck_from_reviewed_games() {
    let app = test_app();
    let (s, r) = json_req(&app, Method::GET, "/api/mistakes/summary", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((r["due"].as_u64(), r["total"].as_u64()), (Some(0), Some(0)));
    let (_, r) = json_req(&app, Method::GET, "/api/mistakes/next", None).await;
    assert!(r["card"].is_null());
    assert_eq!(r["due"], json!(false));

    // Fool's mate: the user (white) blunders with g4.
    let moves = json!(["f2f3", "e7e5", "g2g4", "d8h4"]);
    let new_game = json!({"white": "Me", "black": "Bot Bob", "result": "0-1", "moves": moves, "user_color": "white", "bot_id": "bob"});
    let (s, g) = json_req(&app, Method::POST, "/api/games", Some(new_game.clone())).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = g["id"].as_i64().unwrap();
    let (s, r) = json_req(&app, Method::POST, "/api/review", Some(json!({"game_id": id, "depth": 8}))).await;
    assert_eq!(s, StatusCode::OK, "{r}");

    let (_, sum) = json_req(&app, Method::GET, "/api/mistakes/summary", None).await;
    let total = sum["total"].as_u64().unwrap();
    assert!(total >= 1, "the g4 blunder becomes a card: {sum}");
    assert_eq!(sum["due"].as_u64(), Some(total));

    let (s, n) = json_req(&app, Method::GET, "/api/mistakes/next", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(n["due"], json!(true));
    let card = &n["card"];
    assert_eq!(card["game_id"].as_i64(), Some(id));
    assert_eq!(card["color"], json!("white"));
    assert_eq!(card["opponent"], json!("Bot Bob"));
    assert!(card["solution"].as_array().unwrap().len() % 2 == 1);
    assert_ne!(card["best_uci"], card["played_uci"]);
    let cid = card["id"].as_i64().unwrap();

    // Attempt: solved -> streak 1, scheduled about a day out.
    let (s, a) = json_req(&app, Method::POST, &format!("/api/mistakes/{cid}/attempt"), Some(json!({"solved": true, "time_ms": 4000}))).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    assert_eq!(a["counted"], json!(true));
    assert_eq!(a["card"]["streak"].as_u64(), Some(1));
    assert!(a["card"]["due_in_secs"].as_i64().unwrap() > 80_000);
    let (s, _) = json_req(&app, Method::POST, "/api/mistakes/999999/attempt", Some(json!({"solved": true}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Backfill: a second reviewed game (same positions) is scanned once; nothing is duplicated.
    let (_, g1) = json_req(&app, Method::GET, &format!("/api/games/{id}"), None).await;
    let (_, g2) = json_req(&app, Method::POST, "/api/games", Some(new_game)).await;
    let id2 = g2["id"].as_i64().unwrap();
    let (s, _) = json_req(&app, Method::PUT, &format!("/api/games/{id2}"), Some(json!({"review_json": g1["review_json"]}))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, sy) = json_req(&app, Method::POST, "/api/mistakes/sync", None).await;
    assert_eq!(s, StatusCode::OK, "{sy}");
    assert_eq!((sy["scanned"].as_u64(), sy["added"].as_u64(), sy["more"].as_bool()), (Some(1), Some(0), Some(false)));
    let (_, sy) = json_req(&app, Method::POST, "/api/mistakes/sync", None).await;
    assert_eq!(sy["scanned"].as_u64(), Some(0));

    // List + delete.
    let (s, l) = json_req(&app, Method::GET, "/api/mistakes?limit=5", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(l["items"].as_array().unwrap().len() as u64, total.min(5));
    let (s, _) = json_req(&app, Method::GET, "/api/mistakes?filter=nope", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, d) = json_req(&app, Method::DELETE, &format!("/api/mistakes/{cid}"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(d["summary"]["total"].as_u64(), Some(total - 1));
    let (s, _) = json_req(&app, Method::DELETE, &format!("/api/mistakes/{cid}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

