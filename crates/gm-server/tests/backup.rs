//! Integration tests for `/api/backup/*` and `/api/sync/*` (full router, layers included).

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
        "gm-backup-test-{}-{}",
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


async fn raw(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, bytes)
}

fn post_bytes(uri: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn as_json(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or(Value::Null)
}

async fn save_game(app: &Router, moves: &[&str]) {
    let (s, _, _) = send(app, Method::POST, "/api/games", Some(json!({"white":"Me","black":"Bot","result":"*","moves":moves}))).await;
    assert!(s.is_success(), "{s}");
}

#[tokio::test]
async fn export_preview_import_round_trip() {
    let a = test_app();
    save_game(&a, &["e2e4", "e7e5"]).await;
    save_game(&a, &["d2d4"]).await;
    let (s, h, file) = send(&a, Method::GET, "/api/backup/export", None).await;
    assert_eq!(s, StatusCode::OK);
    let cd = h.get(header::CONTENT_DISPOSITION).unwrap().to_str().unwrap();
    assert!(cd.contains("grandmentor-backup-"), "{cd}");
    let v = as_json(&file);
    assert_eq!(v["format"], "grandmentor-backup");
    assert_eq!(v["tables"]["games"].as_array().unwrap().len(), 2);

    let (s, _, b) = send(&a, Method::GET, "/api/backup/status", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(as_json(&b)["last_backup_at"].is_string());

    let b_app = test_app();
    save_game(&b_app, &["c2c4"]).await;
    let (s, _, p) = raw(&b_app, post_bytes("/api/backup/preview", file.clone())).await;
    assert_eq!(s, StatusCode::OK);
    let p = as_json(&p);
    let games = p["tables"].as_array().unwrap().iter().find(|t| t["name"] == "games").unwrap().clone();
    assert_eq!(games["rows"], 2);
    assert_eq!(games["current_rows"], 1);

    // Replace needs an explicit confirmation.
    let (s, _, _) = raw(&b_app, post_bytes("/api/backup/import?mode=replace", file.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = raw(&b_app, post_bytes("/api/backup/import?mode=sideways", file.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, _, r) = raw(&b_app, post_bytes("/api/backup/import?mode=merge", file.clone())).await;
    assert_eq!(s, StatusCode::OK);
    assert!(as_json(&r)["inserted"].as_u64().unwrap() >= 2);
    let (_, list) = json_req(&b_app, Method::GET, "/api/games", None).await;
    assert_eq!(list.as_array().unwrap().len(), 3);

    let (s, _, _) = raw(&b_app, post_bytes("/api/backup/import?mode=replace&confirm=replace", file)).await;
    assert_eq!(s, StatusCode::OK);
    let (_, list) = json_req(&b_app, Method::GET, "/api/games", None).await;
    assert_eq!(list.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn backup_routes_accept_large_bodies_and_reject_junk() {
    let app = test_app();
    // Bigger than the 1 MiB default limit of other endpoints.
    let big = json!({
        "format": "grandmentor-backup", "format_version": 1, "schema_version": 2,
        "tables": {}, "padding": "x".repeat(3 * 1024 * 1024)
    });
    let (s, _, _) = raw(&app, post_bytes("/api/backup/preview", big.to_string().into_bytes())).await;
    assert_eq!(s, StatusCode::OK);

    let (s, _, b) = raw(&app, post_bytes("/api/backup/preview", b"hello".to_vec())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(as_json(&b)["error"].is_string());

    let mut req = post_bytes("/api/backup/preview", br#"{"format":"x"}"#.to_vec());
    req.headers_mut().insert(header::ACCEPT_LANGUAGE, "es".parse().unwrap());
    let (s, _, b) = raw(&app, req).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(as_json(&b)["error"].as_str().unwrap().contains("GrandMentor"));

    let future = br#"{"format":"grandmentor-backup","format_version":7,"tables":{}}"#.to_vec();
    let (s, _, _) = raw(&app, post_bytes("/api/backup/import?mode=merge", future)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sync_requires_pairing_and_gates_cors() {
    let app = test_app();
    save_game(&app, &["e2e4"]).await;
    let remote = "http://192.168.1.20:8080";

    // No code -> 401, and no CORS for a foreign origin.
    let req = Request::builder().uri("/api/sync/snapshot").header(header::ORIGIN, remote).body(Body::empty()).unwrap();
    let (s, h, _) = raw(&app, req).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());

    // Pairing codes can't be created from another origin.
    let req = Request::builder().method(Method::POST).uri("/api/sync/pair").header(header::ORIGIN, remote).body(Body::empty()).unwrap();
    let (s, _, _) = raw(&app, req).await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let (s, v) = json_req(&app, Method::POST, "/api/sync/pair", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["active"], true);
    let code = v["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 7);

    // Preflight announcing the pairing header gets CORS.
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/sync/merge")
        .header(header::ORIGIN, remote)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type,x-gm-pair")
        .body(Body::empty())
        .unwrap();
    let (_, h, _) = raw(&app, req).await;
    assert_eq!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(), remote);

    // Other endpoints never get CORS for foreign origins.
    let req = Request::builder().uri("/api/games").header(header::ORIGIN, remote).header("x-gm-pair", &code).body(Body::empty()).unwrap();
    let (_, h, _) = raw(&app, req).await;
    assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());

    let req = Request::builder().uri("/api/sync/snapshot").header(header::ORIGIN, remote).header("x-gm-pair", code.to_lowercase()).body(Body::empty()).unwrap();
    let (s, h, snap) = raw(&app, req).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(), remote);
    assert_eq!(as_json(&snap)["tables"]["games"].as_array().unwrap().len(), 1);

    // Pushing the same snapshot back is a no-op merge.
    let mut req = post_bytes("/api/sync/merge", snap);
    req.headers_mut().insert("x-gm-pair", code.parse().unwrap());
    let (s, _, r) = raw(&app, req).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(as_json(&r)["inserted"], 0);

    // Revoked codes stop working.
    let (s, _) = json_req(&app, Method::DELETE, "/api/sync/pair", None).await;
    assert_eq!(s, StatusCode::OK);
    let req = Request::builder().uri("/api/sync/snapshot").header("x-gm-pair", &code).body(Body::empty()).unwrap();
    let (s, _, _) = raw(&app, req).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
