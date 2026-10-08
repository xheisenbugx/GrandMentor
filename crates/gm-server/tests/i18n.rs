//! Language negotiation end to end: `?lang=` / `Accept-Language`, localized content, bot text,
//! mentor text, localized errors and text-only review re-localization.

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
    let dir = std::env::temp_dir().join(format!("gm-i18n-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let data = dir.join("data");
    let es = data.join("i18n").join("es");
    std::fs::create_dir_all(&es).unwrap();
    std::fs::create_dir_all(dir.join("web")).unwrap();
    std::fs::write(
        data.join("openings.json"),
        json!([
            {"id":"kings-pawn","eco":"B00","name":"King's Pawn Game","family":"King's Pawn","moves":"e4 e5","side":"white","popularity":8,"level":"beginner","description":"d","ideas":["Develop"],"traps":[]},
            {"id":"italian-game","eco":"C50","name":"Italian Game","family":"Italian Game","moves":"e4 e5 Nf3 Nc6 Bc4","side":"white","popularity":10,"level":"beginner","description":"Aim at f7.","ideas":["Castle"],"traps":[]}
        ])
        .to_string(),
    )
    .unwrap();
    std::fs::write(data.join("puzzles.json"), "[]").unwrap();
    std::fs::write(
        data.join("courses.json"),
        json!([{"id":"basics","title":"Chess Basics","category":"basics","level":"beginner","description":"Rules.","icon":"♟️",
            "lessons":[{"id":"l1","title":"Lesson","summary":"S","steps":[{"text":"Hello"},{"text":"Bye"}]}]}])
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        data.join("endgames.json"),
        json!([{"id":"kq","title":"Queen Mate","category":"basic","level":"beginner","fen":"8/8/8/4k3/8/8/8/3QK3 w - - 0 1","goal":"win","description":"Mate.","hint":"Box.","technique":["Box"]}])
            .to_string(),
    )
    .unwrap();
    std::fs::write(es.join("openings.json"), json!({"italian-game": {"name": "Apertura italiana", "family": "Italiana"}}).to_string()).unwrap();
    std::fs::write(
        es.join("courses.json"),
        json!({"basics": {"title": "Fundamentos", "lessons": {"l1": {"steps": [{"text": "Hola"}, null]}}}}).to_string(),
    )
    .unwrap();
    std::fs::write(es.join("endgames.json"), json!({"kq": {"title": "Mate de dama"}}).to_string()).unwrap();
    std::fs::write(dir.join("web/index.html"), "<!doctype html>").unwrap();
    dir
}

fn test_app() -> Router {
    let dir = fixture_dir();
    let content = gm_content::Content::load(&dir.join("data")).expect("content");
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(2, 4),
        gm_store::Store::open_in_memory().expect("store"),
        // No API key: the rule-based coach answers deterministically.
        Arc::new(gm_mentor::Mentor::new(None, None)),
    );
    app(state, &dir.join("web"))
}

async fn req(app: &Router, method: Method, uri: &str, lang: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut r = Request::builder().method(method).uri(uri);
    if let Some(l) = lang {
        r = r.header(header::ACCEPT_LANGUAGE, l);
    }
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
    let v = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

const ES: Option<&str> = Some("es-MX,es;q=0.9,en;q=0.8");

#[tokio::test]
async fn bots_follow_language() {
    let app = test_app();
    let (_, en) = req(&app, Method::GET, "/api/bots", None, None).await;
    let (_, es) = req(&app, Method::GET, "/api/bots", ES, None).await;
    let (_, forced_en) = req(&app, Method::GET, "/api/bots?lang=en", ES, None).await;
    let (_, query_es) = req(&app, Method::GET, "/api/bots?lang=es", None, None).await;
    assert_eq!(en[0]["name"], es[0]["name"]);
    assert_eq!(en[0]["style"], es[0]["style"], "style stays a machine key");
    assert_ne!(en[0]["greeting"], es[0]["greeting"]);
    assert!(es[0]["greeting"].as_str().unwrap().starts_with("¡Hola"));
    assert_eq!(forced_en, en);
    assert_eq!(query_es, es);
}

#[tokio::test]
async fn errors_are_localized() {
    let app = test_app();
    let (s, v) = req(&app, Method::GET, "/api/nope", ES, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"], "No existe ese endpoint de la API");
    let (s, v) = req(&app, Method::GET, "/api/games/999", ES, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["error"], "No se encontró la partida 999");
    let (s, v) = req(&app, Method::GET, "/api/openings/lookup?fen=garbage", ES, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().starts_with("FEN no válido"), "{v}");
    let (s, v) = req(&app, Method::POST, "/api/bot/move", ES, Some(json!({"bot_id": "titan", "start_fen": "", "moves": ["e2e5"]}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("ilegal"), "{v}");
    let (s, v) = req(&app, Method::POST, "/api/review", ES, Some(json!({"pgn": "1. e4 e5 2. Ke3"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(!v["error"].as_str().unwrap().is_empty(), "{v}");
    // English stays English.
    let (_, v) = req(&app, Method::GET, "/api/games/999", None, None).await;
    assert_eq!(v["error"], "game 999 not found");
}

#[tokio::test]
async fn content_overlays_and_fallback() {
    let app = test_app();
    let (_, c) = req(&app, Method::GET, "/api/courses", ES, None).await;
    assert_eq!(c[0]["title"], "Fundamentos");
    assert_eq!(c[0]["description"], "Rules.", "missing field falls back");
    let (_, c) = req(&app, Method::GET, "/api/courses/basics", ES, None).await;
    assert_eq!(c["lessons"][0]["steps"][0]["text"], "Hola");
    assert_eq!(c["lessons"][0]["steps"][1]["text"], "Bye", "null step falls back");
    let (_, e) = req(&app, Method::GET, "/api/endgames", ES, None).await;
    assert_eq!(e[0]["title"], "Mate de dama");
    assert_eq!(e[0]["hint"], "Box.");
    let (_, o) = req(&app, Method::GET, "/api/openings/italian-game", ES, None).await;
    assert_eq!(o["name"], "Apertura italiana");
    // Search matches Spanish and English names, results are localized.
    let (_, l) = req(&app, Method::GET, "/api/openings?q=italiana", ES, None).await;
    assert_eq!(l.as_array().unwrap().len(), 1);
    let (_, l) = req(&app, Method::GET, "/api/openings?q=italian%20game", ES, None).await;
    assert_eq!(l[0]["name"], "Apertura italiana");
    // Opening lookup: localized name, book continuation names, start position.
    let fen = "r1bqkbnr/pppp1ppp/2n5/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3";
    let uri = format!("/api/openings/lookup?fen={}", fen.replace(' ', "%20").replace('/', "%2F"));
    let (_, m) = req(&app, Method::GET, &uri, ES, None).await;
    let bc4 = m["continuations"].as_array().unwrap().iter().find(|b| b["san"] == "Bc4").cloned().unwrap();
    assert_eq!(bc4["name"], "Apertura italiana");
    let (_, en) = req(&app, Method::GET, &uri, None, None).await;
    let bc4 = en["continuations"].as_array().unwrap().iter().find(|b| b["san"] == "Bc4").cloned().unwrap();
    assert_eq!(bc4["name"], "Italian Game", "cache is per language");
}

#[tokio::test]
async fn mentor_speaks_spanish() {
    let app = test_app();
    let italian = "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3";
    let (s, v) = req(
        &app,
        Method::POST,
        "/api/mentor/chat",
        ES,
        Some(json!({"question": "¿Qué debo hacer?", "fen": italian, "engine_lines": ["+0.25: Bc5 c3 Nf6"]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["source"], "coach");
    assert!(v["answer"].as_str().unwrap().contains("**Bc5**"), "{v}");
    let uri = format!("/api/mentor/position?fen={}", italian.replace(' ', "%20").replace('/', "%2F"));
    let (_, en) = req(&app, Method::GET, &uri, None, None).await;
    let (_, es) = req(&app, Method::GET, &uri, ES, None).await;
    assert_eq!(en["eval"], es["eval"], "engine part shared across languages");
    assert_ne!(en["ideas"], es["ideas"]);
    assert!(es["ideas"].as_array().unwrap().iter().any(|i| i.as_str().unwrap().contains("material")), "{es}");
}

#[tokio::test]
async fn review_relocalizes_stored_text_without_engine() {
    let app = test_app();
    let (s, g) = req(&app, Method::POST, "/api/games", None, Some(json!({"result": "0-1", "moves": ["f2f3", "e7e5", "g2g4", "d8h4"]}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = g["id"].as_i64().unwrap();
    let (s, en) = req(&app, Method::POST, "/api/review", None, Some(json!({"game_id": id, "depth": 4}))).await;
    assert_eq!(s, StatusCode::OK, "{en}");

    // Plant a sentinel evaluation in the stored review: if the engine were re-run it would be
    // overwritten; a text-only re-localization keeps it.
    let mut stored = en.clone();
    stored["evals"][1] = json!({"cp": 4242});
    stored["moves"][0]["eval_after"] = json!({"cp": 4242});
    stored["lang"] = json!("en");
    let (s, _) = req(&app, Method::PUT, &format!("/api/games/{id}"), None, Some(json!({"review_json": stored.to_string()}))).await;
    assert_eq!(s, StatusCode::OK);

    let (s, es) = req(&app, Method::POST, "/api/review", ES, Some(json!({"game_id": id, "depth": 4}))).await;
    assert_eq!(s, StatusCode::OK, "{es}");
    assert_eq!(es["evals"][1], json!({"cp": 4242}), "engine must not run");
    assert_eq!(es["moves"].as_array().unwrap().len(), 4);
    for (a, b) in en["moves"].as_array().unwrap().iter().zip(es["moves"].as_array().unwrap()) {
        assert_eq!(a["classification"], b["classification"]);
        assert_eq!(a["best_move_uci"], b["best_move_uci"]);
        assert_ne!(a["explanation"], b["explanation"]);
    }
    assert!(es["summary"].as_str().unwrap().contains("Precisión"), "{}", es["summary"]);
    assert!(es.get("lang").is_none(), "response shape unchanged");

    // The latest language is cached in the game.
    let (_, g) = req(&app, Method::GET, &format!("/api/games/{id}"), None, None).await;
    let stored: Value = serde_json::from_str(g["review_json"].as_str().unwrap()).unwrap();
    assert_eq!(stored["lang"], "es");
    assert_eq!(stored["summary"], es["summary"]);

    // Ad-hoc (moves) reviews: the second language reuses the cached evaluations.
    let body = json!({"moves": ["e2e4", "e7e5", "g1f3"], "depth": 5});
    let (_, a) = req(&app, Method::POST, "/api/review", None, Some(body.clone())).await;
    let (_, b) = req(&app, Method::POST, "/api/review", ES, Some(body)).await;
    assert_eq!(a["evals"], b["evals"]);
    assert_ne!(a["summary"], b["summary"]);
    assert_eq!(b["opening"]["name"], "King's Pawn Game", "untranslated opening falls back to English");
}
