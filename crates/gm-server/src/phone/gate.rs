//! Access gate in front of the whole app (API, websockets and static files).
//!
//! * Requests from this computer (loopback peer) always pass.
//! * `/phone/ca.crt` and `/favicon.ico` are public; so are `/login`, the manifest and app icons.
//! * In home-network mode, plain HTTP from another device only serves the CA download and the
//!   device-sync endpoints; everything else is redirected to the HTTPS address.
//! * Device sync (`/api/sync/snapshot`, `/api/sync/merge`) carries its own short-lived pairing
//!   code (routes/backup.rs), so it is not asked for the PIN.
//! * Everything else needs the `gm_access` cookie from `/login`; without it the API answers
//!   `401 {"error", "login": "/login"}` and page loads are redirected to `/login`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use super::{cookie, is_loopback, tr, PhoneState, Transport, CA_PATH, COOKIE, LOGIN_PATH};
use crate::lang::lang_of;

/// Paths reachable without signing in: the login page and its form target, plus the web app
/// manifest and its icons — browsers fetch the manifest *without cookies*, so it must be public
/// for the app to be installable. None of these contain user data.
const PUBLIC_PATHS: &[&str] = &[LOGIN_PATH, "/api/access/login", "/manifest.webmanifest"];
const PUBLIC_PREFIXES: &[&str] = &["/img/icons/"];
const SYNC_PATHS: &[&str] = &["/api/sync/snapshot", "/api/sync/merge"];

#[derive(Clone)]
pub struct GateCtx {
    pub phone: Arc<PhoneState>,
    pub transport: Transport,
}

pub async fn gate(State(ctx): State<GateCtx>, mut req: Request, next: Next) -> Response {
    req.extensions_mut().insert(ctx.transport);
    // No peer address only happens in-process (tests); real listeners always provide one.
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    if peer.is_none_or(is_loopback) {
        return next.run(req).await;
    }
    let path = req.uri().path();
    if path == CA_PATH || path == "/favicon.ico" {
        return next.run(req).await;
    }
    let sync = SYNC_PATHS.contains(&path);
    if ctx.transport == Transport::Http && ctx.phone.opts.lan_running {
        if sync {
            return next.run(req).await;
        }
        return to_https(&ctx.phone, &req);
    }
    if !ctx.phone.opts.pin_required || sync || PUBLIC_PATHS.contains(&path) || PUBLIC_PREFIXES.iter().any(|p| path.starts_with(p) && !path.contains("..")) {
        return next.run(req).await;
    }
    if cookie(req.headers(), COOKIE).is_some_and(|t| ctx.phone.session_valid(t)) {
        return next.run(req).await;
    }
    deny(&req)
}

/// Host part of the `Host` header if it only has host-name characters.
pub fn request_host(req_headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = req_headers.get(header::HOST)?.to_str().ok()?;
    let host = if let Some(v6) = raw.strip_prefix('[') { format!("[{}]", v6.split(']').next()?) } else { raw.split(':').next()?.to_string() };
    let ok = !host.is_empty()
        && host.len() <= 253
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    ok.then_some(host)
}

fn to_https(phone: &PhoneState, req: &Request) -> Response {
    let lang = lang_of(req.uri(), req.headers());
    let host = request_host(req.headers());
    if let (true, Some(host)) = (req.method() == Method::GET || req.method() == Method::HEAD, host) {
        let pq = req.uri().path_and_query().map_or("/", |p| p.as_str());
        let loc = format!("https://{host}:{}{pq}", phone.opts.https_port);
        if let Ok(v) = HeaderValue::from_str(&loc) {
            return (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, v)]).into_response();
        }
    }
    let msg = tr(
        lang,
        [
            "Use the secure (https) address shown in Settings on your computer.",
            "Usa la dirección segura (https) que aparece en Ajustes en tu ordenador.",
            "Use o endereço seguro (https) mostrado nas Configurações do seu computador.",
            "Utilise l'adresse sécurisée (https) affichée dans les Réglages de ton ordinateur.",
            "Nutze die sichere Adresse (https), die in den Einstellungen auf deinem Computer steht.",
        ],
    );
    (StatusCode::FORBIDDEN, Json(json!({ "error": msg }))).into_response()
}

fn deny(req: &Request) -> Response {
    let path = req.uri().path();
    let lang = lang_of(req.uri(), req.headers());
    let is_page = (req.method() == Method::GET || req.method() == Method::HEAD)
        && !path.starts_with("/api/")
        && (path == "/"
            || path.ends_with(".html")
            || req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html")));
    if is_page {
        let next = if path == "/" || path.ends_with(".html") { String::new() } else { format!("?next={}", super::login::encode_component(path)) };
        let loc = format!("{LOGIN_PATH}{next}");
        let mut res = (StatusCode::SEE_OTHER, [(header::LOCATION, loc)]).into_response();
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return res;
    }
    let msg = tr(
        lang,
        [
            "Enter the access PIN first. You'll find it on the computer that runs GrandMentor, in Settings.",
            "Primero escribe el PIN de acceso. Lo encontrarás en el ordenador donde se ejecuta GrandMentor, en Ajustes.",
            "Digite primeiro o PIN de acesso. Ele está no computador que executa o GrandMentor, em Configurações.",
            "Saisis d'abord le code PIN d'accès. Tu le trouveras sur l'ordinateur qui exécute GrandMentor, dans les Réglages.",
            "Gib zuerst die Zugangs-PIN ein. Du findest sie auf dem Computer, auf dem GrandMentor läuft, in den Einstellungen.",
        ],
    );
    let mut res = (StatusCode::UNAUTHORIZED, Json(json!({ "error": msg, "login": LOGIN_PATH }))).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}
