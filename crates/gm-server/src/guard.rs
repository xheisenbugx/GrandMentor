//! Request guard in front of the whole app (outside the access gate): DNS-rebinding and
//! cross-site request protection for a server that usually runs on `localhost`.
//!
//! * **Host allowlist.** The `Host` header (or the HTTP/2 authority) must name this machine:
//!   `localhost` / `*.localhost`, any IP literal (a rebinding attack always uses a DNS name the
//!   attacker controls), this computer's host name (bare, `.local`, or as the OS reports it), the
//!   names on the HTTPS certificate, or an entry of `GM_ALLOWED_HOSTS` (comma separated, for
//!   custom DNS names). Anything else → `421 {error}`. Requests without a host (in-process
//!   tests) pass.
//! * **Origin check.** State-changing methods (`POST`, `PUT`, `PATCH`, `DELETE`) and websocket
//!   upgrades are refused with `403 {error}` when their `Origin` is a different site: allowed are
//!   the request's own origin (`Origin` host = `Host`), a localhost origin when the peer is this
//!   machine (dev servers on another port), and the device-sync endpoints when the request
//!   carries the `X-GM-Pair` header (a browser only sends that custom header cross-origin after
//!   the CORS preflight passed; the handler then checks the code). Without an `Origin`, a
//!   `Sec-Fetch-Site: cross-site` request is refused the same way. Clients that send neither
//!   (curl, other programs on this machine) are not affected.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::lang::lang_of;
use crate::phone::{is_loopback, tr, PhoneState};
use crate::routes::backup::{PAIR_HEADER, SYNC_PATHS};

/// Max entries read from `GM_ALLOWED_HOSTS`.
const MAX_EXTRA_HOSTS: usize = 64;

#[derive(Clone)]
pub struct GuardCtx {
    phone: Arc<PhoneState>,
    /// Lower-case host names accepted besides the built-in ones.
    extra_hosts: Arc<Vec<String>>,
}

impl GuardCtx {
    pub fn new(phone: Arc<PhoneState>) -> Self {
        let mut extra: Vec<String> = std::env::var("GM_ALLOWED_HOSTS")
            .ok()
            .map(|v| v.split(',').filter_map(normalize_host).take(MAX_EXTRA_HOSTS).collect())
            .unwrap_or_default();
        // The name the OS reports (`Alexs-MacBook-Pro.local`, `chessbox.lan`), besides the
        // sanitized DNS label `PhoneState::hostname` holds.
        if let Some(raw) = normalize_host(&gethostname::gethostname().to_string_lossy()) {
            extra.push(raw);
        }
        // The sanitized label and its mDNS form, even when phone mode is off (`PhoneState::hostname`
        // is then unset) and the OS name has no `.local` suffix (most Linux hosts).
        if let Some(label) = crate::phone::net::hostname() {
            extra.push(format!("{label}.local"));
            extra.push(label);
        }
        extra.dedup();
        GuardCtx { phone, extra_hosts: Arc::new(extra) }
    }

    fn host_allowed(&self, host: &str) -> bool {
        if host == "localhost" || host.ends_with(".localhost") || host.parse::<IpAddr>().is_ok() {
            return true;
        }
        if let Some(h) = &self.phone.hostname {
            if host == h || host.strip_suffix(".local") == Some(h.as_str()) {
                return true;
            }
        }
        if self.extra_hosts.iter().any(|h| h == host) {
            return true;
        }
        self.phone.tls().is_some_and(|t| t.names.iter().any(|n| n.eq_ignore_ascii_case(host)))
    }
}

/// `Host: Example.com:8080` → `example.com`; `[::1]:8080` → `::1`. `None` for junk.
pub fn normalize_host(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let host = if let Some(v6) = raw.strip_prefix('[') {
        v6.split_once(']')?.0
    } else {
        raw.rsplit_once(':').map_or(raw, |(h, port)| if port.bytes().all(|b| b.is_ascii_digit()) { h } else { raw })
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let ok = !host.is_empty()
        && host.len() <= 253
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'_'));
    ok.then_some(host)
}

/// The authority the client asked for: `Host`, or the URI authority (HTTP/2).
fn authority(req: &Request) -> Option<String> {
    match req.headers().get(header::HOST) {
        Some(h) => Some(h.to_str().unwrap_or("\u{0}").to_string()),
        None => req.uri().authority().map(|a| a.as_str().to_string()),
    }
}

fn is_state_changing(method: &Method) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::PATCH | Method::DELETE)
}

fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// `Origin` `scheme://host[:port]` is the same authority the request was sent to.
fn same_origin(origin: &str, authority: Option<&str>) -> bool {
    let Some(authority) = authority else { return false };
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://")).unwrap_or("");
    !rest.is_empty() && rest.eq_ignore_ascii_case(authority)
}

/// Whether a state-changing request (or websocket upgrade) may come from where it says it does.
pub fn origin_allowed(headers: &HeaderMap, path: &str, authority: Option<&str>, peer: Option<IpAddr>) -> bool {
    // Device sync carries its own pairing code; a browser can only attach that header
    // cross-origin once the CORS preflight (`backup::sync_cors_allowed`) agreed.
    if SYNC_PATHS.contains(&path) && headers.contains_key(PAIR_HEADER) {
        return true;
    }
    match headers.get(header::ORIGIN) {
        Some(origin) => {
            let Ok(o) = origin.to_str() else { return false };
            same_origin(o, authority) || (peer.is_none_or(is_loopback) && crate::is_local_origin(origin.as_bytes()))
        }
        None => !headers
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("cross-site")),
    }
}

fn error(status: StatusCode, msg: String) -> Response {
    let mut res = (status, Json(json!({ "error": msg }))).into_response();
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

pub async fn guard(State(ctx): State<GuardCtx>, req: Request, next: Next) -> Response {
    let authority = authority(&req);
    if let Some(raw) = &authority {
        if !normalize_host(raw).is_some_and(|h| ctx.host_allowed(&h)) {
            tracing::warn!(host = %raw.chars().take(80).collect::<String>(), "refused a request for an unknown host name");
            let lang = lang_of(req.uri(), req.headers());
            return error(
                StatusCode::MISDIRECTED_REQUEST,
                tr(
                    lang,
                    [
                        "This address is not GrandMentor's. Open it with localhost or the address shown in Settings.",
                        "Esta dirección no es la de GrandMentor. Ábrelo con localhost o con la dirección que aparece en Ajustes.",
                        "Este endereço não é o do GrandMentor. Abra-o com localhost ou com o endereço mostrado nas Configurações.",
                        "Cette adresse n'est pas celle de GrandMentor. Ouvre-le avec localhost ou l'adresse affichée dans les Réglages.",
                        "Diese Adresse gehört nicht zu GrandMentor. Öffne es über localhost oder die Adresse aus den Einstellungen.",
                    ],
                ),
            );
        }
    }
    if is_state_changing(req.method()) || is_websocket_upgrade(req.headers()) {
        let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
        if !origin_allowed(req.headers(), req.uri().path(), authority.as_deref(), peer) {
            tracing::warn!(
                method = %req.method(),
                path = %req.uri().path(),
                origin = ?req.headers().get(header::ORIGIN),
                "refused a cross-site request"
            );
            let lang = lang_of(req.uri(), req.headers());
            return error(
                StatusCode::FORBIDDEN,
                tr(
                    lang,
                    [
                        "Another website tried to change your GrandMentor data. That was blocked.",
                        "Otra página web intentó cambiar tus datos de GrandMentor. Se ha bloqueado.",
                        "Outro site tentou alterar seus dados do GrandMentor. Isso foi bloqueado.",
                        "Un autre site a essayé de modifier tes données GrandMentor. C'est bloqué.",
                        "Eine andere Website wollte deine GrandMentor-Daten ändern. Das wurde blockiert.",
                    ],
                ),
            );
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_normalized() {
        assert_eq!(normalize_host("Example.COM:8080").as_deref(), Some("example.com"));
        assert_eq!(normalize_host("[::1]:8080").as_deref(), Some("::1"));
        assert_eq!(normalize_host("127.0.0.1").as_deref(), Some("127.0.0.1"));
        assert_eq!(normalize_host("chessbox.local.").as_deref(), Some("chessbox.local"));
        assert_eq!(normalize_host("a b"), None);
        assert_eq!(normalize_host(""), None);
    }

    #[test]
    fn origin_rules() {
        let mut h = HeaderMap::new();
        let lan: IpAddr = "192.168.1.9".parse().expect("ip");
        // No Origin, no fetch metadata: curl & co.
        assert!(origin_allowed(&h, "/api/games", Some("localhost:8080"), None));
        h.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!origin_allowed(&h, "/api/games", Some("localhost:8080"), None));
        h.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(origin_allowed(&h, "/api/games", Some("localhost:8080"), None));

        h.insert(header::ORIGIN, HeaderValue::from_static("http://localhost:8080"));
        assert!(origin_allowed(&h, "/api/games", Some("localhost:8080"), None));
        // A dev server on another local port, from this machine only.
        h.insert(header::ORIGIN, HeaderValue::from_static("http://localhost:5173"));
        assert!(origin_allowed(&h, "/api/games", Some("localhost:8080"), None));
        assert!(!origin_allowed(&h, "/api/games", Some("192.168.1.20:8080"), Some(lan)));
        // Same origin over the network.
        h.insert(header::ORIGIN, HeaderValue::from_static("https://192.168.1.20:8443"));
        assert!(origin_allowed(&h, "/api/games", Some("192.168.1.20:8443"), Some(lan)));

        h.insert(header::ORIGIN, HeaderValue::from_static("https://evil.example"));
        assert!(!origin_allowed(&h, "/api/backup/import", Some("localhost:8080"), None));
        assert!(!origin_allowed(&h, "/api/sync/merge", Some("localhost:8080"), None));
        h.insert(PAIR_HEADER, HeaderValue::from_static("ABC234"));
        assert!(origin_allowed(&h, "/api/sync/merge", Some("localhost:8080"), None));
        assert!(!origin_allowed(&h, "/api/backup/import", Some("localhost:8080"), None));
        h.insert(header::ORIGIN, HeaderValue::from_static("null"));
        assert!(!origin_allowed(&h, "/api/games", Some("localhost:8080"), None));
    }
}
