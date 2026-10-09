//! Weekly personal set: `/api/weekly*` routes.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use gm_server::{app, AppState};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn test_app() -> Router {
    let content = gm_content::Content::load(&data_dir()).expect("content");
    let web = std::env::temp_dir().join(format!("gm-weekly-web-{}", std::process::id()));
    std::fs::create_dir_all(&web).expect("web dir");
    std::fs::write(web.join("index.html"), "<!doctype html><title>GrandMentor</title>").expect("index");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(1, 4),
        gm_store::Store::open_in_memory().expect("store"),
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    app(state, &web)
}

async fn json_req(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(req.body(body).expect("request")).await.expect("response");
    let status = res.status();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn weekly_set_lifecycle() {
    let app = test_app();
    // Built on first request, stable afterwards.
    let (s, v) = json_req(&app, Method::GET, "/api/weekly", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let items = v["items"].as_array().expect("items").clone();
    assert!((10..=15).contains(&items.len()), "{} items", items.len());
    assert_eq!(v["generation"], 1);
    assert_eq!(v["progress"]["done"], 0);
    let focus = v["focus"].as_array().expect("focus");
    assert!((2..=3).contains(&focus.len()));
    assert_eq!(focus[0]["reason"], "starter", "no history yet");
    assert!(v["week"].as_str().expect("week").contains("-W"));
    for it in &items {
        assert_eq!(it["kind"], "puzzle");
        assert!(it["puzzle"]["moves"].as_array().expect("moves").len() >= 2);
        assert!(it["result"].is_null());
    }
    let set_id = v["set_id"].as_i64().expect("set id");
    let (_, again) = json_req(&app, Method::GET, "/api/weekly", None).await;
    assert_eq!(again["set_id"], set_id);
    assert_eq!(again["items"], v["items"]);

    // First attempt counts and rates the puzzle; the second doesn't.
    let url = "/api/weekly/attempt";
    let (s, a) = json_req(&app, Method::POST, url, Some(json!({"set_id": set_id, "index": 0, "solved": false, "time_ms": 5000}))).await;
    assert_eq!(s, StatusCode::OK, "{a}");
    assert_eq!((a["counted"].clone(), a["item"]["result"].clone()), (json!(true), json!("failed")));
    assert!(a["rating"]["rating"].is_number());
    assert_eq!(a["progress"]["done"], 1);
    let (_, a) = json_req(&app, Method::POST, url, Some(json!({"set_id": set_id, "index": 0, "solved": true}))).await;
    assert_eq!(a["counted"], false);
    assert!(a.get("rating").is_none());
    for i in 1..items.len() {
        let (s, a) = json_req(&app, Method::POST, url, Some(json!({"set_id": set_id, "index": i, "solved": true}))).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(a["finished"], i == items.len() - 1);
    }
    let (s, _) = json_req(&app, Method::POST, url, Some(json!({"set_id": set_id, "index": 99, "solved": true}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = json_req(&app, Method::POST, url, Some(json!({"set_id": set_id + 50, "index": 0, "solved": true}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = json_req(&app, Method::POST, url, Some(json!({"set_id": "x"}))).await;
    assert!(s.is_client_error());

    // The profile saw the rated attempts.
    let (_, p) = json_req(&app, Method::GET, "/api/profile", None).await;
    assert_eq!(p["puzzles_failed"], 1);

    // History: this week is last, with per-theme results.
    let (s, h) = json_req(&app, Method::GET, "/api/weekly/history?weeks=4", None).await;
    assert_eq!(s, StatusCode::OK);
    let weeks = h["weeks"].as_array().expect("weeks");
    assert_eq!(weeks.len(), 4);
    assert_eq!(weeks[3]["week"], v["week"]);
    assert!(weeks[0]["progress"].is_null());
    assert_eq!(weeks[3]["progress"]["done"], items.len());
    let attempted: u64 = weeks[3]["themes"].as_array().expect("themes").iter().map(|t| t["attempted"].as_u64().unwrap_or(0)).sum();
    assert_eq!(attempted as usize, items.len());
    let (s, _) = json_req(&app, Method::GET, "/api/weekly/history?weeks=99", None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // New set: next generation, fresh progress.
    let (s, n) = json_req(&app, Method::POST, "/api/weekly/regenerate", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(n["generation"], 2);
    assert_eq!(n["progress"]["done"], 0);
    assert_ne!(n["set_id"], set_id);
    let (_, cur) = json_req(&app, Method::GET, "/api/weekly", None).await;
    assert_eq!(cur["set_id"], n["set_id"]);
}
