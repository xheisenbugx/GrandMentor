//! Integration tests for the access gate, `/login`, `/api/access/*` and `/api/phone/*`
//! (full router, layers included; the peer address is injected like the real listeners do).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use gm_server::phone::{PhoneOptions, PhoneState, TlsInfo, Transport};
use gm_server::{app_with_phone, AppState};

const LOOPBACK: &str = "127.0.0.1:50000";
const PHONE: &str = "192.168.1.50:40000";
const OTHER_PHONE: &str = "192.168.1.51:40000";

fn tmp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("gm-phone-it-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("data")).expect("data dir");
    std::fs::create_dir_all(dir.join("web/img/icons")).expect("web dir");
    for f in ["openings.json", "puzzles.json", "courses.json", "endgames.json"] {
        std::fs::write(dir.join("data").join(f), "[]").expect("write");
    }
    std::fs::write(dir.join("web/index.html"), "<!doctype html><title>GrandMentor</title>").expect("write");
    std::fs::write(dir.join("web/img/icons/icon-192.png"), b"png").expect("write");
    dir
}

struct Fixture {
    dir: PathBuf,
    state: AppState,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = tmp_dir(tag);
        let content = gm_content::Content::load(&dir.join("data")).expect("content");
        let state = AppState::new(
            Arc::new(content),
            gm_engine::EnginePool::new(1, 4),
            gm_store::Store::open_in_memory().expect("store"),
            Arc::new(gm_mentor::Mentor::from_env()),
        );
        Fixture { dir, state }
    }

    fn phone(&self, lan_running: bool, pin_required: bool) -> Arc<PhoneState> {
        Arc::new(PhoneState::new(PhoneOptions {
            dir: Some(self.dir.join("phone")),
            http_port: 8080,
            https_port: 8443,
            lan_running,
            lan_env: None,
            http_network_visible: true,
            pin_required,
        }))
    }

    fn app(&self, phone: &Arc<PhoneState>, transport: Transport) -> Router {
        app_with_phone(self.state.clone(), &self.dir.join("web"), Arc::clone(phone), transport)
    }
}

fn req(method: Method, uri: &str, peer: &str) -> axum::http::request::Builder {
    let addr: SocketAddr = peer.parse().expect("addr");
    let mut b = Request::builder().method(method).uri(uri).header(header::HOST, "192.168.1.20:8443");
    if let Some(ext) = b.extensions_mut() {
        ext.insert(ConnectInfo(addr));
    }
    b
}

async fn send(app: &Router, r: Request<Body>) -> Response {
    app.clone().oneshot(r).await.expect("response")
}

async fn body_json(res: Response) -> Value {
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn get(app: &Router, uri: &str, peer: &str, cookie: Option<&str>) -> Response {
    let mut b = req(Method::GET, uri, peer);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    send(app, b.body(Body::empty()).expect("req")).await
}

async fn login(app: &Router, pin: &str, peer: &str) -> Response {
    let body = format!("pin={pin}&next=%2F%23%2Fplay");
    let r = req(Method::POST, "/api/access/login", peer)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ORIGIN, "https://192.168.1.20:8443")
        .body(Body::from(body))
        .expect("req");
    send(app, r).await
}

async fn current_pin(app: &Router) -> String {
    let res = get(app, "/api/phone/status", LOOPBACK, None).await;
    assert_eq!(res.status(), StatusCode::OK);
    body_json(res).await["pin"].as_str().expect("pin").to_string()
}

fn cookie_pair(res: &Response) -> String {
    let set = res.headers().get(header::SET_COOKIE).expect("set-cookie").to_str().expect("ascii");
    set.split(';').next().expect("pair").to_string()
}

#[tokio::test]
async fn loopback_never_needs_a_pin() {
    let f = Fixture::new("loop");
    let phone = f.phone(true, true);
    for transport in [Transport::Http, Transport::Https] {
        let app = f.app(&phone, transport);
        assert_eq!(get(&app, "/api/health", LOOPBACK, None).await.status(), StatusCode::OK);
        assert_eq!(get(&app, "/", LOOPBACK, None).await.status(), StatusCode::OK);
        let ipv6 = get(&app, "/api/health", "[::1]:5000", None).await;
        assert_eq!(ipv6.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn other_devices_need_the_pin() {
    let f = Fixture::new("pin");
    let phone = f.phone(true, true);
    let app = f.app(&phone, Transport::Https);

    // API: 401 with a pointer to the login page.
    let res = get(&app, "/api/health", PHONE, None).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(body_json(res).await["login"], "/login");
    // Websocket endpoint too.
    assert_eq!(get(&app, "/api/engine/ws", PHONE, None).await.status(), StatusCode::UNAUTHORIZED);
    // Pages: redirected to the login page.
    let mut b = req(Method::GET, "/", PHONE);
    b = b.header(header::ACCEPT, "text/html");
    let res = send(&app, b.body(Body::empty()).expect("req")).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers()[header::LOCATION], "/login");
    // The login page itself and its icon are reachable.
    let res = get(&app, "/login", PHONE, None).await;
    assert_eq!(res.status(), StatusCode::OK);
    let html = String::from_utf8(res.into_body().collect().await.expect("body").to_bytes().to_vec()).expect("utf8");
    assert!(html.contains("name=\"pin\""));
    assert_eq!(get(&app, "/img/icons/icon-192.png", PHONE, None).await.status(), StatusCode::OK);
    // The manifest is fetched by browsers without cookies: it must stay public for installs.
    std::fs::write(f.dir.join("web/manifest.webmanifest"), "{}").expect("write");
    assert_eq!(get(&app, "/manifest.webmanifest", PHONE, None).await.status(), StatusCode::OK);
    assert_eq!(get(&app, "/sw.js", PHONE, None).await.status(), StatusCode::UNAUTHORIZED);
    // A forged cookie does not help.
    let fake = format!("gm_access={}", "a".repeat(64));
    assert_eq!(get(&app, "/api/health", PHONE, Some(&fake)).await.status(), StatusCode::UNAUTHORIZED);
    // Loopback-only endpoints stay closed to other devices.
    assert_eq!(get(&app, "/api/phone/status", PHONE, None).await.status(), StatusCode::UNAUTHORIZED);

    // Wrong PIN.
    let res = login(&app, "000000x", PHONE).await;
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(res.headers().get(header::SET_COOKIE).is_none());

    // Right PIN (spaces are fine): cookie + redirect to `next`.
    let pin = current_pin(&app).await;
    let res = login(&app, &format!("{}+{}", &pin[..3], &pin[3..]), PHONE).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers()[header::LOCATION], "/#/play");
    let set = res.headers()[header::SET_COOKIE].to_str().expect("ascii").to_string();
    for attr in ["HttpOnly", "SameSite=Strict", "Secure", "Path=/", "Max-Age="] {
        assert!(set.contains(attr), "{attr} missing in {set}");
    }
    let c = cookie_pair(&res);
    assert_eq!(get(&app, "/api/health", PHONE, Some(&c)).await.status(), StatusCode::OK);
    assert_eq!(get(&app, "/", PHONE, Some(&c)).await.status(), StatusCode::OK);
    let st = body_json(get(&app, "/api/access/status", PHONE, Some(&c)).await).await;
    assert_eq!(st["signed_in"], true);
    assert_eq!(st["local"], false);
    // Still loopback-only even when signed in.
    assert_eq!(get(&app, "/api/phone/status", PHONE, Some(&c)).await.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(get(&app, "/api/phone/status", LOOPBACK, None).await).await["devices"], 1);

    // Sign out all devices (from the computer): the cookie stops working.
    let r = req(Method::DELETE, "/api/phone/devices", LOOPBACK).body(Body::empty()).expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::OK);
    assert_eq!(get(&app, "/api/health", PHONE, Some(&c)).await.status(), StatusCode::UNAUTHORIZED);

    // A new PIN replaces the old one.
    let r = req(Method::POST, "/api/phone/pin", LOOPBACK).body(Body::empty()).expect("req");
    let new_pin = body_json(send(&app, r).await).await["pin"].as_str().expect("pin").to_string();
    if new_pin != pin {
        assert_eq!(login(&app, &pin, PHONE).await.status(), StatusCode::UNAUTHORIZED);
    }
    assert_eq!(login(&app, &new_pin, PHONE).await.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn cookie_is_not_secure_over_plain_http() {
    let f = Fixture::new("http");
    // GM_HOST=0.0.0.0 without phone mode: plain HTTP on the LAN, still PIN-protected.
    let phone = f.phone(false, true);
    let app = f.app(&phone, Transport::Http);
    assert_eq!(get(&app, "/api/health", PHONE, None).await.status(), StatusCode::UNAUTHORIZED);
    let pin = current_pin(&app).await;
    let res = login(&app, &pin, PHONE).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let set = res.headers()[header::SET_COOKIE].to_str().expect("ascii").to_string();
    assert!(!set.contains("Secure"));
    assert_eq!(get(&app, "/api/health", PHONE, Some(&cookie_pair(&res))).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn json_login_and_logout() {
    let f = Fixture::new("json");
    let phone = f.phone(true, true);
    let app = f.app(&phone, Transport::Https);
    let pin = current_pin(&app).await;
    let r = req(Method::POST, "/api/access/login", PHONE)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "pin": pin }).to_string()))
        .expect("req");
    let res = send(&app, r).await;
    assert_eq!(res.status(), StatusCode::OK);
    let c = cookie_pair(&res);
    assert_eq!(get(&app, "/api/health", PHONE, Some(&c)).await.status(), StatusCode::OK);
    let r = req(Method::POST, "/api/access/logout", PHONE).header(header::COOKIE, &c).body(Body::empty()).expect("req");
    let res = send(&app, r).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()[header::SET_COOKIE].to_str().expect("ascii").contains("Max-Age=0"));
    assert_eq!(get(&app, "/api/health", PHONE, Some(&c)).await.status(), StatusCode::UNAUTHORIZED);
    // Cross-site form posts are refused.
    let r = req(Method::POST, "/api/access/login", PHONE)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::from(format!("pin={pin}")))
        .expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn wrong_pins_are_rate_limited_per_device() {
    let f = Fixture::new("rate");
    let phone = f.phone(true, true);
    let app = f.app(&phone, Transport::Https);
    let pin = current_pin(&app).await;
    let wrong = if pin == "000000" { "111111" } else { "000000" };
    for _ in 0..gm_server::phone::access::PER_IP_FAILURES - 1 {
        assert_eq!(login(&app, wrong, PHONE).await.status(), StatusCode::UNAUTHORIZED);
    }
    // The 5th wrong try locks this device…
    assert_eq!(login(&app, wrong, PHONE).await.status(), StatusCode::TOO_MANY_REQUESTS);
    // …even for the right PIN, with a Retry-After hint.
    let res = login(&app, &pin, PHONE).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(res.headers().get(header::RETRY_AFTER).is_some());
    // Another device is not affected.
    assert_eq!(login(&app, &pin, OTHER_PHONE).await.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn plain_http_from_the_lan_goes_to_https_in_phone_mode() {
    let f = Fixture::new("redirect");
    let phone = f.phone(true, true);
    let app = f.app(&phone, Transport::Http);
    let res = get(&app, "/api/health?x=1", PHONE, None).await;
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(res.headers()[header::LOCATION], "https://192.168.1.20:8443/api/health?x=1");
    // Login over plain HTTP is not offered either.
    assert_eq!(get(&app, "/login", PHONE, None).await.status(), StatusCode::TEMPORARY_REDIRECT);
    // The CA download works over plain HTTP (that's how the phone gets it before trusting HTTPS).
    assert_eq!(get(&app, "/phone/ca.crt", PHONE, None).await.status(), StatusCode::NOT_FOUND);
    phone.set_tls(TlsInfo { ca_der: vec![0x30, 0x03, 1, 2, 3], names: vec!["localhost".into()] });
    let res = get(&app, "/phone/ca.crt", PHONE, None).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/x-x509-ca-cert");
    // The desktop keeps plain HTTP.
    assert_eq!(get(&app, "/api/health", LOOPBACK, None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn device_sync_keeps_its_own_pairing_code() {
    let f = Fixture::new("sync");
    let phone = f.phone(true, true);
    for transport in [Transport::Http, Transport::Https] {
        let app = f.app(&phone, transport);
        // Not stopped by the PIN gate (no `login` hint) — the pairing code decides.
        let res = get(&app, "/api/sync/snapshot", PHONE, None).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert!(body_json(res).await.get("login").is_none());
    }
}

#[tokio::test]
async fn pin_can_be_turned_off() {
    let f = Fixture::new("off");
    let phone = f.phone(false, false);
    let app = f.app(&phone, Transport::Http);
    assert_eq!(get(&app, "/api/health", PHONE, None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn status_and_qr_are_loopback_only() {
    let f = Fixture::new("status");
    let phone = f.phone(true, true);
    let app = f.app(&phone, Transport::Https);
    let st = body_json(get(&app, "/api/phone/status", LOOPBACK, None).await).await;
    assert_eq!(st["lan"]["running"], true);
    assert_eq!(st["pin"].as_str().map(str::len), Some(6));
    // QR only for advertised addresses.
    let res = get(&app, "/api/phone/qr?url=https%3A%2F%2Fevil.example%2F", LOOPBACK, None).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    if let Some(url) = st["app_urls"].as_array().and_then(|a| a.first()).and_then(Value::as_str) {
        let res = get(&app, &format!("/api/phone/qr?url={}", urlencode(url)), LOOPBACK, None).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/svg+xml");
    }
    // A web page on another origin can't read the PIN through this machine's browser.
    let r = req(Method::GET, "/api/phone/status", LOOPBACK).header(header::ORIGIN, "http://evil.example").body(Body::empty()).expect("req");
    assert_eq!(send(&app, r).await.status(), StatusCode::FORBIDDEN);
    // The PIN is saved so it survives a restart.
    assert!(f.dir.join("phone/access.json").is_file());
}

#[tokio::test]
async fn default_setup_writes_nothing() {
    let f = Fixture::new("default");
    let phone = Arc::new(PhoneState::new(PhoneOptions { dir: Some(f.dir.join("phone")), ..PhoneOptions::default() }));
    let app = f.app(&phone, Transport::Http);
    let st = body_json(get(&app, "/api/phone/status", LOOPBACK, None).await).await;
    assert_eq!(st["lan"]["running"], false);
    assert_eq!(st["network_visible"], false);
    assert_eq!(st["app_urls"], json!([]));
    assert!(!f.dir.join("phone").exists(), "nothing is written while phone access is off");
    // Turning it on only saves the switch (it takes effect after a restart).
    let r = req(Method::PUT, "/api/phone/lan", LOOPBACK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "enabled": true }).to_string()))
        .expect("req");
    let st = body_json(send(&app, r).await).await;
    assert_eq!(st["lan"]["enabled"], true);
    assert_eq!(st["lan"]["restart_required"], true);
    assert!(f.dir.join("phone/phone.json").is_file());
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b == b'.' || b == b'-' { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect()
}
