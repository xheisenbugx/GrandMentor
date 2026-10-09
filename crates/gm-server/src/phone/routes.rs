//! Phone & access endpoints (docs/CONTRACT.md "Phone & home use").
//!
//! Loopback only (the computer running GrandMentor):
//! * `GET /api/phone/status`, `PUT /api/phone/lan`, `POST /api/phone/pin`,
//!   `DELETE /api/phone/devices`, `GET /api/phone/qr?url=`.
//!
//! Any device: `GET /api/access/status`, `POST /api/access/login`, `POST /api/access/logout`,
//! `GET /login` (sign-in page) and `GET /phone/ca.crt` (the local CA certificate).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use gm_content::Lang;
use serde::Deserialize;
use serde_json::{json, Value};

use super::login::{self, LoginError};
use super::{cookie, is_loopback, tr, PhoneConfig, PhoneState, Transport, CA_PATH, COOKIE, COOKIE_MAX_AGE, LOGIN_PATH};
use crate::error::{ApiError, ApiResult};
use crate::lang::{lang_of, ReqLang};

type Phone = Arc<PhoneState>;
const LOGIN_BODY_LIMIT: usize = 4 * 1024;
const MAX_QR_TEXT: usize = 512;

/// `/api/phone/*` and `/api/access/*` (paths relative to `/api`).
pub fn api_router() -> Router<Phone> {
    Router::new()
        .route("/phone/status", get(status))
        .route("/phone/lan", put(set_lan))
        .route("/phone/pin", post(new_pin))
        .route("/phone/devices", delete(sign_out_all))
        .route("/phone/qr", get(qr))
        .route("/access/status", get(access_status))
        .route("/access/login", post(login_post).layer(DefaultBodyLimit::max(LOGIN_BODY_LIMIT)))
        .route("/access/logout", post(logout))
}

/// `/login` and `/phone/ca.crt`.
pub fn page_router() -> Router<Phone> {
    Router::new().route(LOGIN_PATH, get(login_page)).route(CA_PATH, get(ca_cert))
}

fn peer_ip(peer: Option<&ConnectInfo<SocketAddr>>) -> Option<IpAddr> {
    peer.map(|c| c.0.ip())
}

/// Loopback peer (or none, in-process) and a local or absent `Origin`.
fn is_local(peer: Option<IpAddr>, headers: &HeaderMap) -> bool {
    if peer.is_some_and(|ip| !is_loopback(ip)) {
        return false;
    }
    headers.get(header::ORIGIN).is_none_or(|o| crate::is_local_origin(o.as_bytes()))
}

fn require_local(peer: Option<IpAddr>, headers: &HeaderMap, lang: Lang) -> ApiResult<()> {
    if is_local(peer, headers) {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::FORBIDDEN,
        tr(
            lang,
            [
                "Phone settings can only be changed on the computer that runs GrandMentor.",
                "Los ajustes del móvil solo se pueden cambiar en el ordenador donde se ejecuta GrandMentor.",
                "As configurações do celular só podem ser alteradas no computador que executa o GrandMentor.",
                "Les réglages du téléphone ne peuvent être modifiés que sur l'ordinateur qui exécute GrandMentor.",
                "Die Handy-Einstellungen lassen sich nur auf dem Computer ändern, auf dem GrandMentor läuft.",
            ],
        ),
    ))
}

/// Addresses other devices can open, and where they download the CA certificate.
struct Urls {
    app: Vec<String>,
    ca: Vec<String>,
    local_name: Option<String>,
}

fn urls(p: &PhoneState) -> Urls {
    let ips = p.lan_addresses();
    let o = &p.opts;
    let mut out = Urls { app: Vec::new(), ca: Vec::new(), local_name: None };
    if o.lan_running {
        out.app = ips.iter().map(|ip| format!("https://{ip}:{}/", o.https_port)).collect();
        out.ca = ips
            .iter()
            .map(|ip| {
                if o.http_network_visible {
                    format!("http://{ip}:{}{CA_PATH}", o.http_port)
                } else {
                    format!("https://{ip}:{}{CA_PATH}", o.https_port)
                }
            })
            .collect();
        out.local_name = p.hostname.as_ref().map(|h| format!("https://{h}.local:{}/", o.https_port));
    } else if o.http_network_visible {
        out.app = ips.iter().map(|ip| format!("http://{ip}:{}/", o.http_port)).collect();
    }
    out
}

fn status_json(p: &PhoneState) -> Value {
    let cfg = p.config();
    let o = &p.opts;
    let wanted = o.lan_env.unwrap_or(cfg.lan);
    let u = urls(p);
    let tls = p.tls();
    json!({
        "lan": {
            "enabled": wanted,
            "running": o.lan_running,
            "env": o.lan_env,
            "restart_required": wanted != o.lan_running,
            "can_save": o.dir.is_some() && o.lan_env.is_none(),
        },
        "http_port": o.http_port,
        "https_port": o.https_port,
        "network_visible": o.lan_running || o.http_network_visible,
        "hostname": p.hostname,
        "addresses": p.lan_addresses().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "app_urls": u.app,
        "ca_urls": u.ca,
        "local_name_url": u.local_name,
        "ca": tls.map(|t| json!({
            "fingerprint": super::tls::fingerprint(&t.ca_der),
            "download": CA_PATH,
            "names": t.names,
        })),
        "pin_required": o.pin_required,
        "pin": p.pin(),
        "devices": p.device_count(),
    })
}

async fn status(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    require_local(peer_ip(peer.as_ref()), &headers, lang)?;
    let p2 = p.clone();
    let v = crate::api::blocking(move || status_json(&p2)).await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
struct LanBody {
    enabled: bool,
}

async fn set_lan(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<Value>> {
    require_local(peer_ip(peer.as_ref()), &headers, lang)?;
    let req: LanBody = serde_json::from_slice(&body).map_err(|_| ApiError::bad_request("expected {\"enabled\": true|false}"))?;
    if p.opts.lan_env.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            tr(
                lang,
                [
                    "Phone access is set by GM_LAN when GrandMentor starts. Change it there.",
                    "El acceso desde el móvil lo fija GM_LAN al iniciar GrandMentor. Cámbialo ahí.",
                    "O acesso pelo celular é definido por GM_LAN ao iniciar o GrandMentor. Altere lá.",
                    "L'accès depuis le téléphone est fixé par GM_LAN au démarrage de GrandMentor. Modifie-le là.",
                    "Der Handy-Zugang wird beim Start von GrandMentor über GM_LAN festgelegt. Ändere ihn dort.",
                ],
            ),
        ));
    }
    let Some(dir) = p.opts.dir.clone() else {
        return Err(ApiError::new(StatusCode::CONFLICT, "phone settings are not saved in this mode"));
    };
    let p2 = p.clone();
    let v = crate::api::blocking(move || {
        PhoneConfig { lan: req.enabled }.save(&dir).map(|()| status_json(&p2))
    })
    .await?
    .map_err(|e| ApiError::internal(format!("could not save phone settings: {e}")))?;
    Ok(Json(v))
}

async fn new_pin(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    require_local(peer_ip(peer.as_ref()), &headers, lang)?;
    let p2 = p.clone();
    let pin = crate::api::blocking(move || p2.regenerate_pin()).await?;
    Ok(Json(json!({ "pin": pin })))
}

async fn sign_out_all(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    require_local(peer_ip(peer.as_ref()), &headers, lang)?;
    let p2 = p.clone();
    let n = crate::api::blocking(move || p2.sign_out_all()).await?;
    Ok(Json(json!({ "signed_out": n, "devices": 0 })))
}

#[derive(Deserialize)]
struct QrQuery {
    url: String,
}

/// QR code (SVG) for one of the addresses listed by `/api/phone/status`.
async fn qr(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
    Query(q): Query<QrQuery>,
) -> ApiResult<Response> {
    require_local(peer_ip(peer.as_ref()), &headers, lang)?;
    let p2 = p.clone();
    let allowed = crate::api::blocking(move || {
        let u = urls(&p2);
        u.app.into_iter().chain(u.ca).chain(u.local_name).collect::<Vec<_>>()
    })
    .await?;
    if q.url.len() > MAX_QR_TEXT || !allowed.contains(&q.url) {
        return Err(ApiError::bad_request("unknown address"));
    }
    let svg = qr_svg(&q.url).ok_or_else(|| ApiError::internal("could not draw the QR code"))?;
    Ok((
        [(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "no-store")],
        svg,
    )
        .into_response())
}

/// QR code for `text` as a standalone SVG (black on white, quiet zone included).
pub fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color<'_>>()
            .min_dimensions(240, 240)
            .quiet_zone(true)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
    )
}

async fn access_status(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Json<Value> {
    let local = peer_ip(peer.as_ref()).is_none_or(is_loopback);
    let signed_in = cookie(&headers, COOKIE).is_some_and(|t| p.session_valid(t));
    Json(json!({ "local": local, "signed_in": signed_in, "pin_required": p.opts.pin_required }))
}

fn session_cookie(token: &str, secure: bool, max_age: u64) -> Option<HeaderValue> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{COOKIE}={token}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}")).ok()
}

/// `Origin` (when sent) must be the same host the request was made to.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else { return true };
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else { return false };
    let Ok(origin) = origin.to_str() else { return false };
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://")).unwrap_or("");
    rest.eq_ignore_ascii_case(host)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LoginJson {
    pin: String,
    next: Option<String>,
}

async fn login_post(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    transport: Option<Extension<Transport>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    body: Bytes,
) -> Response {
    let lang = lang_of(&uri, &headers);
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let (pin, next) = if is_json {
        let j: LoginJson = serde_json::from_slice(&body).unwrap_or_default();
        (j.pin, j.next)
    } else {
        let s = String::from_utf8_lossy(&body);
        (login::form_value(&s, "pin").unwrap_or_default(), login::form_value(&s, "next"))
    };
    let next = login::safe_next(next.as_deref());
    let fail = |status: StatusCode, err: LoginError, retry: Option<u64>| -> Response {
        let mut res = if is_json {
            let msg = match err {
                LoginError::Wrong => tr(lang, ["Wrong PIN.", "PIN incorrecto.", "PIN incorreto.", "Code PIN incorrect.", "Falsche PIN."]),
                LoginError::Locked(_) => tr(
                    lang,
                    ["Too many tries. Wait and try again.", "Demasiados intentos. Espera y vuelve a intentarlo.", "Tentativas demais. Espere e tente de novo.", "Trop d'essais. Attends et réessaie.", "Zu viele Versuche. Warte und versuch es erneut."],
                ),
                LoginError::Origin => "cross-origin sign-in refused".to_string(),
            };
            (status, Json(json!({ "error": msg }))).into_response()
        } else {
            (status, Html(login::page(lang, &next, Some(err)))).into_response()
        };
        if let Some(secs) = retry {
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        res
    };
    if !same_origin(&headers) {
        return fail(StatusCode::FORBIDDEN, LoginError::Origin, None);
    }
    let ip = peer_ip(peer.as_ref()).unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let label = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let p2 = p.clone();
    let result = match crate::api::blocking(move || p2.try_login(ip, &pin, &label)).await {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    match result {
        Ok(token) => {
            let secure = transport.is_some_and(|t| t.0 == Transport::Https);
            let mut res = if is_json {
                Json(json!({ "ok": true, "next": next })).into_response()
            } else {
                let mut r = StatusCode::SEE_OTHER.into_response();
                if let Ok(v) = HeaderValue::from_str(&next) {
                    r.headers_mut().insert(header::LOCATION, v);
                }
                r
            };
            if let Some(c) = session_cookie(&token, secure, COOKIE_MAX_AGE) {
                res.headers_mut().insert(header::SET_COOKIE, c);
            }
            res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            tracing::info!(%ip, "a device signed in with the access PIN");
            res
        }
        Err(Some(wait)) => {
            tracing::warn!(%ip, "access PIN locked after too many wrong tries");
            fail(StatusCode::TOO_MANY_REQUESTS, LoginError::Locked(wait), Some(wait))
        }
        Err(None) => fail(StatusCode::UNAUTHORIZED, LoginError::Wrong, None),
    }
}

async fn logout(State(p): State<Phone>, transport: Option<Extension<Transport>>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "cross-origin sign-out refused" }))).into_response();
    }
    if let Some(tok) = cookie(&headers, COOKIE).map(str::to_string) {
        let p2 = p.clone();
        let _ = crate::api::blocking(move || p2.sign_out(&tok)).await;
    }
    let secure = transport.is_some_and(|t| t.0 == Transport::Https);
    let mut res = Json(json!({ "ok": true })).into_response();
    if let Some(c) = session_cookie("", secure, 0) {
        res.headers_mut().insert(header::SET_COOKIE, c);
    }
    res
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LoginQuery {
    next: Option<String>,
}

async fn login_page(
    State(p): State<Phone>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(q): Query<LoginQuery>,
) -> Response {
    let next = login::safe_next(q.next.as_deref());
    let local = peer_ip(peer.as_ref()).is_none_or(is_loopback);
    let signed_in = cookie(&headers, COOKIE).is_some_and(|t| p.session_valid(t));
    if local || signed_in || !p.opts.pin_required {
        let mut r = StatusCode::SEE_OTHER.into_response();
        if let Ok(v) = HeaderValue::from_str(&next) {
            r.headers_mut().insert(header::LOCATION, v);
        }
        return r;
    }
    let lang = lang_of(&uri, &headers);
    let mut res = Html(login::page(lang, &next, None)).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// The local CA certificate (DER), for installing on phones.
async fn ca_cert(State(p): State<Phone>) -> ApiResult<Response> {
    let Some(tls) = p.tls() else {
        return Err(ApiError::not_found("phone access (HTTPS) is not turned on"));
    };
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("application/x-x509-ca-cert")),
            (header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment; filename=\"grandmentor-ca.crt\"")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        tls.ca_der,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode the QR matrix we draw and check it carries exactly the URL.
    #[test]
    fn qr_encodes_the_url() {
        let url = "https://192.168.1.20:8443/";
        let code = qrcode::QrCode::with_error_correction_level(url.as_bytes(), qrcode::EcLevel::M).expect("qr");
        let w = code.width();
        let colors = code.to_colors();
        let scale = 6;
        let quiet = 4;
        let size = (w + 2 * quiet) * scale;
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(size, size, |x, y| {
            let (cx, cy) = (x / scale, y / scale);
            if cx < quiet || cy < quiet || cx >= w + quiet || cy >= w + quiet {
                return 255;
            }
            if colors[(cy - quiet) * w + (cx - quiet)] == qrcode::Color::Dark { 0 } else { 255 }
        });
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1);
        let (_, content) = grids[0].decode().expect("decode");
        assert_eq!(content, url);
        let svg = qr_svg(url).expect("svg");
        assert!(svg.starts_with("<?xml") && svg.contains("<svg"));
    }

    #[test]
    fn origin_check() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_static("192.168.1.20:8443"));
        assert!(same_origin(&h));
        h.insert(header::ORIGIN, HeaderValue::from_static("https://192.168.1.20:8443"));
        assert!(same_origin(&h));
        h.insert(header::ORIGIN, HeaderValue::from_static("https://evil.example"));
        assert!(!same_origin(&h));
    }
}
