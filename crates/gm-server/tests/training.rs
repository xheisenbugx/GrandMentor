//! Endgame training: `/api/training*` routes and an engine sanity check of every drill position.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use gm_engine::{parse_fen, Engine, Score, SearchLimits};
use gm_server::{app, AppState};
use shakmaty::{Color, Position};

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn test_app() -> Router {
    let content = gm_content::Content::load(&data_dir()).expect("content");
    let web = std::env::temp_dir().join(format!("gm-training-web-{}", std::process::id()));
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
async fn training_progress_and_mastery() {
    let app = test_app();
    let (s, v) = json_req(&app, Method::GET, "/api/training", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["mastery_streak"], 3);
    let drills = v["drills"].as_array().expect("drills");
    assert!(drills.len() >= 40, "every content drill is listed");
    let lucena = drills.iter().find(|d| d["drill_id"] == "lucena").expect("lucena");
    assert_eq!(lucena["attempts"], 0);
    assert_eq!(lucena["mastered"], false);

    let url = "/api/training/lucena/attempt";
    let (s, v) = json_req(&app, Method::POST, url, Some(json!({"success": false, "moves": 7}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((v["attempts"].clone(), v["streak"].clone()), (json!(1), json!(0)));
    for i in 1..=3 {
        let (s, v) = json_req(&app, Method::POST, url, Some(json!({"success": true, "moves": 10 + i}))).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["streak"], i);
        assert_eq!(v["just_mastered"], i == 3);
        assert_eq!(v["mastered"], i == 3);
    }
    let (_, v) = json_req(&app, Method::POST, url, Some(json!({"success": true, "moves": 9}))).await;
    assert_eq!(v["just_mastered"], false, "only the first time");
    assert_eq!(v["best_moves"], 9);

    let (_, v) = json_req(&app, Method::GET, "/api/training", None).await;
    let lucena = v["drills"].as_array().expect("drills").iter().find(|d| d["drill_id"] == "lucena").cloned().expect("lucena");
    assert_eq!((lucena["attempts"].clone(), lucena["successes"].clone()), (json!(5), json!(4)));
    assert_eq!(lucena["mastered"], true);

    // Errors: unknown drill, bad body, out-of-range moves.
    let (s, v) = json_req(&app, Method::POST, "/api/training/nope/attempt", Some(json!({"success": true}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["error"].is_string());
    let (s, _) = json_req(&app, Method::POST, url, Some(json!({"moves": 3}))).await;
    assert!(s.is_client_error());
    let (s, _) = json_req(&app, Method::POST, url, Some(json!({"success": true, "moves": 100000}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = json_req(&app, Method::POST, url, Some(json!({"success": "yes"}))).await;
    assert!(s.is_client_error());
}

/// Engine sanity check (the authoritative check for <= 7 pieces is the Syzygy tablebase, see
/// docs/CONTRACT.md "Endgame training"): every "win" drill must look clearly winning for the side
/// to move, and no "draw" drill may be a forced mate against the defender.
#[test]
fn drill_positions_pass_engine_sanity_check() {
    let content = gm_content::Content::load(&data_dir()).expect("content");
    let mut jobs: Vec<(String, String, String)> = Vec::new();
    for d in &content.endgames {
        for f in std::iter::once(&d.fen).chain(&d.variants) {
            jobs.push((d.id.clone(), d.goal.clone(), f.clone()));
        }
    }
    let chunks: Vec<Vec<(String, String, String)>> = jobs.chunks(jobs.len().div_ceil(4)).map(<[_]>::to_vec).collect();
    let failures: Vec<String> = std::thread::scope(|sc| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                sc.spawn(move || {
                    let mut engine = Engine::new(16);
                    let stop = AtomicBool::new(false);
                    let mut bad = Vec::new();
                    for (id, goal, fen) in chunk {
                        let pos = parse_fen(&fen).expect("fen");
                        engine.new_game();
                        let limits = SearchLimits { movetime_ms: Some(250), ..Default::default() };
                        let info = engine.search(&pos, &limits, &stop, &mut |_| {});
                        // `score()` is white POV; flip to the side to move (the user).
                        let side = pos.turn();
                        let white = info.score();
                        let user = if side == Color::White { white } else { white.negate() };
                        let ok = match (goal.as_str(), user) {
                            ("win", Score::Mate(n)) => n > 0,
                            ("win", Score::Cp(cp)) => cp >= 150,
                            (_, Score::Mate(n)) => n > 0,
                            (_, Score::Cp(_)) => true,
                        };
                        if !ok {
                            bad.push(format!("{id} ({goal}) {fen}: {user:?}"));
                        }
                    }
                    bad
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("thread")).collect()
    });
    assert!(failures.is_empty(), "engine disagrees with drill goals:\n{}", failures.join("\n"));
}
