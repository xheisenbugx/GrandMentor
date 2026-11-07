//! Local-server hardening: cross-site writes, DNS rebinding (Host), device-sync limits,
//! websocket origin / cap, PIN backoff per address, CSP and all-or-nothing restores.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use gm_server::phone::{PhoneOptions, PhoneState, Transport};
use gm_server::{app, app_with_phone, AppState};

const EVIL: &str = "https://evil.example";
const PHONE_A: &str = "192.168.1.50:40000";
const PHONE_B: &str = "192.168.1.51:40000";
const INDEX: &str = "<!doctype html><title>GrandMentor</title><script>window.boot = 1;</script><script type=\"module\" src=\"/js/app.js\"></script>";

fn tmp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("gm-hardening-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("web/js")).expect("web dir");
    std::fs::write(dir.join("web/index.html"), INDEX).expect("write");
    std::fs::write(dir.join("web/js/app.js"), "export {};").expect("write");
    dir
}

fn state() -> AppState {
    AppState::new(
        Arc::new(gm_content::Content::default()),
        gm_engine::EnginePool::new(1, 4),
        gm_store::Store::open_in_memory().expect("store"),
        Arc::new(gm_mentor::Mentor::from_env()),
    )
}

fn test_app(tag: &str) -> Router {
    app(state(), &tmp_dir(tag).join("web"))
}

async fn send(app: &Router, r: Request<Body>) -> Response {
    app.clone().oneshot(r).await.expect("response")
}

async fn body_json(res: Response) -> Value {
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn post(uri: &str) -> axum::http::request::Builder {
    Request::builder().method(Method::POST).uri(uri)
}

async fn save_game(app: &Router, moves: &[&str]) {
    let r = post("/api/games")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"white":"Me","black":"Bot","result":"*","moves":moves}).to_string()))
        .expect("req");
    assert!(send(app, r).await.status().is_success());
}

async fn game_count(app: &Router) -> usize {
    let res = send(app, Request::get("/api/games").body(Body::empty()).expect("req")).await;
    body_json(res).await.as_array().map_or(0, Vec::len)
}

async fn export(app: &Router) -> Value {
    let res = send(app, Request::get("/api/backup/export?mark=false").body(Body::empty()).expect("req")).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await
}

fn import_req(uri: &str) -> axum::http::request::Builder {
    post(uri).header(header::CONTENT_TYPE, "application/json")
}

// ---------------------------------------------------------------------------------------------
// Cross-site writes (CSRF)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn another_website_cannot_replace_the_database() {
    let app = test_app("csrf");
    save_game(&app, &["e2e4"]).await;
    save_game(&app, &["d2d4"]).await;
    let empty = json!({"format":"grandmentor-backup","format_version":1,"schema_version":2,"tables":{"games":[]}});
    let uri = "/api/backup/import?mode=replace&confirm=replace";

    // The classic no-cors attack: text/plain from a foreign origin.
    let r = post(uri)
        .header(header::ORIGIN, EVIL)
        .header(header::CONTENT_TYPE, "text/plain")
        .header("sec-fetch-site", "cross-site")
        .body(Body::from(empty.to_string()))
        .expect("req");
    let res = send(&app, r).await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(body_json(res).await["error"].is_string());

    // Foreign origin with the right content type: still refused.
    let r = import_req(uri).header(header::ORIGIN, EVIL).body(Body::from(empty.to_string())).expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::FORBIDDEN);

    // No Origin but fetch metadata says cross-site.
    let r = import_req(uri).header("sec-fetch-site", "cross-site").body(Body::from(empty.to_string())).expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::FORBIDDEN);

    // Same machine, but not JSON: 415 (a browser can't send JSON cross-site without a preflight).
    for ct in ["text/plain", "application/x-www-form-urlencoded", "multipart/form-data; boundary=x"] {
        let r = post(uri).header(header::CONTENT_TYPE, ct).body(Body::from(empty.to_string())).expect("req");
        assert_eq!(send(&app, r).await.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE, "{ct}");
        let r = post("/api/backup/preview").header(header::CONTENT_TYPE, ct).body(Body::from(empty.to_string())).expect("req");
        assert_eq!(send(&app, r).await.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE, "{ct}");
    }
    assert_eq!(game_count(&app).await, 2, "nothing was wiped");

    // The app itself (same origin / localhost) still works, with a charset parameter too.
    let r = post(uri)
        .header(header::HOST, "localhost:8080")
        .header(header::ORIGIN, "http://localhost:8080")
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(empty.to_string()))
        .expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::OK);
    assert_eq!(game_count(&app).await, 0);
}

#[tokio::test]
async fn body_less_writes_from_other_sites_are_refused() {
    let app = test_app("csrf-empty");
    for (method, uri) in [
        (Method::POST, "/api/first-week/restart"),
        (Method::POST, "/api/weekly/regenerate"),
        (Method::DELETE, "/api/games/1"),
        (Method::PUT, "/api/profile"),
    ] {
        let r = Request::builder().method(method.clone()).uri(uri).header(header::ORIGIN, EVIL).body(Body::empty()).expect("req");
        assert_eq!(send(&app, r).await.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        let r = Request::builder().method(method.clone()).uri(uri).header(header::ORIGIN, "null").body(Body::empty()).expect("req");
        assert_eq!(send(&app, r).await.status(), StatusCode::FORBIDDEN, "{method} {uri} (null origin)");
    }
    // Reads are untouched (CORS still keeps their answers away from other sites).
    let r = Request::get("/api/health").header(header::ORIGIN, EVIL).body(Body::empty()).expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::OK);
    // A local dev server on another port may write.
    let r = post("/api/first-week/restart").header(header::ORIGIN, "http://127.0.0.1:5173").body(Body::empty()).expect("req");
    assert_ne!(send(&app, r).await.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------------------------
// DNS rebinding (Host allowlist)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn unknown_host_names_are_refused() {
    let app = test_app("host");
    for host in ["evil.example", "evil.example:8080", "attacker.localhost.evil.com", "127.0.0.1.nip.io:8080"] {
        let r = Request::get("/api/health").header(header::HOST, host).body(Body::empty()).expect("req");
        let res = send(&app, r).await;
        assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST, "{host}");
        assert!(body_json(res).await["error"].is_string());
        let r = Request::get("/").header(header::HOST, host).body(Body::empty()).expect("req");
        assert_eq!(send(&app, r).await.status(), StatusCode::MISDIRECTED_REQUEST, "{host} /");
    }
    // Localized.
    let r = Request::get("/api/health?lang=es").header(header::HOST, "evil.example").body(Body::empty()).expect("req");
    let msg = body_json(send(&app, r).await).await["error"].as_str().unwrap_or_default().to_string();
    assert!(msg.contains("GrandMentor") && msg.contains("dirección"), "{msg}");

    for host in ["localhost:8080", "LOCALHOST", "127.0.0.1:8199", "[::1]:8080", "192.168.1.20:8443", "10.0.0.7", "app.localhost:8080"] {
        let r = Request::get("/api/health").header(header::HOST, host).body(Body::empty()).expect("req");
        assert_eq!(send(&app, r).await.status(), StatusCode::OK, "{host}");
    }
    // This computer's own name (as in `https://<name>.local:8443`).
    if let Some(name) = gm_server::phone::net::hostname() {
        let r = Request::get("/api/health").header(header::HOST, format!("{name}.local:8443")).body(Body::empty()).expect("req");
        assert_eq!(send(&app, r).await.status(), StatusCode::OK, "{name}.local");
    }
}

// ---------------------------------------------------------------------------------------------
// Device sync
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn sync_merge_checks_the_pairing_code_before_reading_the_body() {
    let app = test_app("merge");
    use futures_util::StreamExt as _;
    // A body that never finishes: if the server tried to read it first, this would hang.
    let endless = futures_util::stream::once(async { Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![b'{'; 64 * 1024])) })
        .chain(futures_util::stream::pending());
    let r = post("/api/sync/merge")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, "209715200")
        .body(Body::from_stream(endless))
        .expect("req");
    let res = tokio::time::timeout(Duration::from_secs(5), send(&app, r)).await.expect("answered without reading the body");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // With a valid code, an announced oversize body is refused up front (413), and a
    // non-JSON body with 415.
    let res = send(&app, post("/api/sync/pair").body(Body::empty()).expect("req")).await;
    let code = body_json(res).await["code"].as_str().expect("code").to_string();
    let r = post("/api/sync/merge")
        .header("x-gm-pair", &code)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, "999999999999")
        .body(Body::empty())
        .expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let r = post("/api/sync/merge").header("x-gm-pair", &code).header(header::CONTENT_TYPE, "text/plain").body(Body::from("{}")).expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // A real merge from another device's page (cross-origin, with the code) still works.
    save_game(&app, &["e2e4"]).await;
    let snap = export(&app).await;
    let r = post("/api/sync/merge")
        .header("x-gm-pair", &code)
        .header(header::ORIGIN, "https://192.168.1.21:8443")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(snap.to_string()))
        .expect("req");
    let res = send(&app, r).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "https://192.168.1.21:8443");
    assert_eq!(body_json(res).await["inserted"], 0);
}

/// The request as a real listener hands it over: peer address + `Host`.
fn from_peer(b: axum::http::request::Builder, peer: &str, host: &str) -> axum::http::request::Builder {
    let addr: SocketAddr = peer.parse().expect("addr");
    let mut b = b.header(header::HOST, host);
    if let Some(ext) = b.extensions_mut() {
        ext.insert(ConnectInfo(addr));
    }
    b
}

#[tokio::test]
async fn wrong_pairing_codes_are_limited_per_device_without_revoking_the_code() {
    let app = test_app("pair-limit");
    let res = send(&app, post("/api/sync/pair").body(Body::empty()).expect("req")).await;
    let code = body_json(res).await["code"].as_str().expect("code").to_string();
    let snapshot = |peer: &str, code: &str| from_peer(Request::get("/api/sync/snapshot"), peer, "192.168.1.20:8080").header("x-gm-pair", code).body(Body::empty()).expect("req");

    for _ in 0..gm_server::routes::backup::MAX_PAIR_FAILURES_PER_IP {
        assert_eq!(send(&app, snapshot(PHONE_A, "ZZZ-ZZZ")).await.status(), StatusCode::UNAUTHORIZED);
    }
    // That device now has to wait, even with the right code…
    let res = send(&app, snapshot(PHONE_A, &code)).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    // …but the code still works for everyone else.
    assert_eq!(send(&app, snapshot(PHONE_B, &code)).await.status(), StatusCode::OK);
    let res = send(&app, Request::get("/api/sync/pair").body(Body::empty()).expect("req")).await;
    assert_eq!(body_json(res).await["active"], true);
    // A new code is a fresh start.
    let res = send(&app, post("/api/sync/pair").body(Body::empty()).expect("req")).await;
    let code = body_json(res).await["code"].as_str().expect("code").to_string();
    assert_eq!(send(&app, snapshot(PHONE_A, &code)).await.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn replace_import_is_all_or_nothing() {
    let source = test_app("replace-src");
    save_game(&source, &["e2e4"]).await;
    save_game(&source, &["d2d4"]).await;
    let mut file = export(&source).await;
    // The same game id twice: the copy can't be restored.
    let dup = file["tables"]["games"][0].clone();
    file["tables"]["games"].as_array_mut().expect("games").push(dup);

    let app = test_app("replace-dst");
    save_game(&app, &["c2c4"]).await;
    let uri = "/api/backup/import?mode=replace&confirm=replace";
    let res = send(&app, import_req(uri).body(Body::from(file.to_string())).expect("req")).await;
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body_json(res).await["error"].as_str().unwrap_or_default().contains("nothing was changed"));
    assert_eq!(game_count(&app).await, 1, "rolled back: the local game is still there");

    let res = send(&app, import_req(&format!("{uri}&allow_partial=1")).body(Body::from(file.to_string())).expect("req")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let rep = body_json(res).await;
    assert_eq!(rep["failed"], 1);
    assert!(rep["warnings"].as_array().expect("warnings").iter().any(|w| w["code"] == "rows_failed"));
    assert_eq!(game_count(&app).await, 2);
}

#[tokio::test]
async fn merge_drops_references_to_games_that_are_not_in_the_backup() {
    let app = test_app("merge-refs");
    for m in ["c2c4", "g1f3", "b1c3"] {
        save_game(&app, &[m]).await;
    }
    let file = json!({
        "format": "grandmentor-backup", "format_version": 1, "schema_version": 2,
        "tables": {
            "games": [],
            "mistake_cards": [{
                "id": 1, "fen_key": "k1", "fen": "8/8/8/8/8/8/8/K6k w - - 0 1", "played_uci": "a1a2",
                "best_uci": "a1b1", "game_id": 2, "due_at": 0, "created_at": 0
            }],
            "adaptive_games": [{ "game_id": 2, "opponent_elo": 800, "score": 1.0, "rating_after": 900.0, "games_after": 1 }]
        }
    });
    let res = send(&app, import_req("/api/backup/import?mode=merge").body(Body::from(file.to_string())).expect("req")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let rep = body_json(res).await;
    let table = |name: &str| rep["tables"].as_array().expect("tables").iter().find(|t| t["name"] == name).cloned().unwrap_or(Value::Null);
    assert_eq!(table("adaptive_games")["skipped"], 1, "{rep}");
    assert_eq!(table("mistake_cards")["inserted"], 1, "{rep}");

    let after = export(&app).await;
    let cards = after["tables"]["mistake_cards"].as_array().expect("cards");
    assert_eq!(cards.len(), 1);
    assert!(cards[0]["game_id"].is_null(), "must not point at the unrelated local game 2: {}", cards[0]);
    assert!(after["tables"]["adaptive_games"].as_array().is_none_or(Vec::is_empty));
}

// ---------------------------------------------------------------------------------------------
// Access PIN
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn pin_lockout_is_per_device_and_never_signs_anyone_out() {
    let dir = tmp_dir("pin");
    let phone = Arc::new(PhoneState::new(PhoneOptions {
        dir: Some(dir.join("phone")),
        lan_running: true,
        http_network_visible: true,
        pin_required: true,
        ..PhoneOptions::default()
    }));
    let app = app_with_phone(state(), &dir.join("web"), Arc::clone(&phone), Transport::Https);
    let pin = phone.pin();
    let wrong = if pin == "000000" { "111111" } else { "000000" };
    let login = |peer: &str, pin: &str| {
        from_peer(post("/api/access/login"), peer, "192.168.1.20:8443")
            .header(header::ORIGIN, "https://192.168.1.20:8443")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "pin": pin }).to_string()))
            .expect("req")
    };

    // Phone B signs in first.
    let res = send(&app, login(PHONE_B, &pin)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res.headers()[header::SET_COOKIE].to_str().expect("cookie").split(';').next().expect("pair").to_string();

    // Phone A guesses until it is locked out; more tries don't extend anyone else's wait.
    let mut locked = false;
    for _ in 0..20 {
        let res = send(&app, login(PHONE_A, wrong)).await;
        if res.status() == StatusCode::TOO_MANY_REQUESTS {
            assert!(res.headers().get(header::RETRY_AFTER).is_some());
            locked = true;
        } else {
            assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        }
    }
    assert!(locked);
    assert_eq!(send(&app, login(PHONE_A, &pin)).await.status(), StatusCode::TOO_MANY_REQUESTS);

    // B keeps working with its cookie, and a third device can still sign in.
    let r = from_peer(Request::get("/api/health"), PHONE_B, "192.168.1.20:8443").header(header::COOKIE, &cookie).body(Body::empty()).expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::OK);
    assert_eq!(send(&app, login("192.168.1.52:40000", &pin)).await.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------------------------
// Static files: CSP, dev pages
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn html_has_a_content_security_policy_with_the_inline_script_hash() {
    let app = test_app("csp");
    let res = send(&app, Request::get("/").body(Body::empty()).expect("req")).await;
    assert_eq!(res.status(), StatusCode::OK);
    let csp = res.headers()[header::CONTENT_SECURITY_POLICY].to_str().expect("csp").to_string();
    let hash = gm_server::web::inline_script_hashes(INDEX);
    assert_eq!(hash.len(), 1);
    assert!(csp.contains(&format!("script-src 'self' 'sha256-{}'", hash[0])), "{csp}");
    for part in ["default-src 'self'", "object-src 'none'", "frame-ancestors 'none'", "base-uri 'self'", "connect-src 'self' ws: wss:"] {
        assert!(csp.contains(part), "{part} in {csp}");
    }
}

// ---------------------------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------------------------

async fn listen() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = app(state(), std::path::Path::new("/nonexistent"));
    tokio::spawn(async move { axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await });
    addr
}

fn ws_request(addr: SocketAddr, origin: Option<&str>) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let mut r = format!("ws://{addr}/api/engine/ws").into_client_request().expect("request");
    if let Some(o) = origin {
        r.headers_mut().insert(header::ORIGIN, o.parse().expect("origin"));
    }
    r
}

fn http_status(err: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match err {
        tokio_tungstenite::tungstenite::Error::Http(res) => res.status(),
        other => panic!("expected an HTTP error, got {other}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_refuses_other_sites_and_caps_open_sockets() {
    let addr = listen().await;
    let err = tokio_tungstenite::connect_async(ws_request(addr, Some(EVIL))).await.expect_err("refused");
    assert_eq!(http_status(err), StatusCode::FORBIDDEN);
    // The app's own pages connect fine.
    let (first, _) = tokio_tungstenite::connect_async(ws_request(addr, Some(&format!("http://{addr}")))).await.expect("same origin");

    let mut open = vec![first];
    for _ in 1..gm_server::ws::MAX_SOCKETS {
        open.push(tokio_tungstenite::connect_async(ws_request(addr, None)).await.expect("socket").0);
    }
    let err = tokio_tungstenite::connect_async(ws_request(addr, None)).await.expect_err("full");
    assert_eq!(http_status(err), StatusCode::SERVICE_UNAVAILABLE);

    // Closing one frees a slot.
    let mut one = open.pop().expect("socket");
    one.close(None).await.expect("close");
    drop(one);
    let mut ok = false;
    for _ in 0..50 {
        if tokio_tungstenite::connect_async(ws_request(addr, None)).await.is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "a slot was freed");
}
