//! Backup export / import (`/api/backup/*`) and device sync (`/api/sync/*`).
//!
//! Security model for sync (see docs/CONTRACT.md "Backup & sync"):
//! * `POST /api/sync/pair` only answers requests coming from this machine (loopback peer and,
//!   when an `Origin` is sent, a localhost origin). It creates a short pairing code that lives
//!   in memory for [`PAIR_TTL`] — one code at a time, wiped after [`MAX_PAIR_FAILURES`] wrong
//!   guesses or on `DELETE /api/sync/pair`.
//! * `GET /api/sync/snapshot` and `POST /api/sync/merge` always require that code in the
//!   `X-GM-Pair` header — with or without CORS.
//! * CORS for those two endpoints (only) is granted to non-local origins when the request
//!   carries a valid code (preflights: when they announce the `x-gm-pair` header). Setting
//!   `GM_SYNC_ALLOW_ORIGINS=http://a:8080,http://b:8080` restricts that to the listed origins.
//! * The server still binds to 127.0.0.1 by default; another device can only reach it after the
//!   user starts it with `GM_HOST=0.0.0.0` (or a LAN address).

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use gm_content::Lang;
use gm_store::backup::{self, BackupError, BackupFile, ImportMode, ImportSource};
use parking_lot::Mutex;
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::blocking;
use crate::error::{ApiError, ApiQuery, ApiResult};
use crate::lang::ReqLang;
use crate::state::AppState;

/// Pairing codes expire after this long.
pub const PAIR_TTL: Duration = Duration::from_secs(10 * 60);
/// Wrong codes tolerated before the active code is revoked.
pub const MAX_PAIR_FAILURES: u32 = 20;
/// Request header carrying the pairing code.
pub const PAIR_HEADER: &str = "x-gm-pair";
/// Body limit for the routes that receive a backup file.
const BACKUP_BODY_LIMIT: usize = backup::MAX_BACKUP_BYTES + 64 * 1024;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 6;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/backup/status", get(status))
        .route("/backup/export", get(export))
        .route("/backup/preview", post(preview).layer(DefaultBodyLimit::max(BACKUP_BODY_LIMIT)))
        .route("/backup/import", post(import).layer(DefaultBodyLimit::max(BACKUP_BODY_LIMIT)))
        .route("/sync/pair", get(pair_status).post(pair_create).delete(pair_revoke))
        .route("/sync/snapshot", get(sync_snapshot))
        .route("/sync/merge", post(sync_merge).layer(DefaultBodyLimit::max(BACKUP_BODY_LIMIT)))
}

// ---------------------------------------------------------------------------------------------
// Localized messages
// ---------------------------------------------------------------------------------------------

fn tr(lang: Lang, en: &str, es: &str) -> String {
    match lang {
        Lang::Es => es.to_string(),
        _ => en.to_string(),
    }
}

/// Maps store errors to `{error}`; backup-file problems become a localized 400.
fn backup_err(e: anyhow::Error, lang: Lang) -> ApiError {
    match e.downcast_ref::<BackupError>() {
        Some(be) => file_err(be, lang),
        None => ApiError::from(e),
    }
}

fn file_err(e: &BackupError, lang: Lang) -> ApiError {
    let msg = match (e, lang) {
        (BackupError::Malformed(_), Lang::Es) => "Este archivo no es una copia de seguridad válida.".to_string(),
        (BackupError::Malformed(_), _) => "This file is not a valid backup.".to_string(),
        (BackupError::NotABackup, Lang::Es) => "Este archivo no es una copia de seguridad de GrandMentor.".to_string(),
        (BackupError::NotABackup, _) => "This file is not a GrandMentor backup.".to_string(),
        (BackupError::UnsupportedVersion { found, supported }, Lang::Es) => format!(
            "Esta copia usa el formato {found}, pero esta versión de GrandMentor solo entiende hasta el {supported}. Actualiza la aplicación e inténtalo de nuevo."
        ),
        (BackupError::UnsupportedVersion { found, supported }, _) => format!(
            "This backup uses format {found}, but this GrandMentor only understands up to {supported}. Update the app and try again."
        ),
        (BackupError::TooLarge, Lang::Es) => format!("La copia es demasiado grande (máximo {} MB).", backup::MAX_BACKUP_BYTES >> 20),
        (BackupError::TooLarge, _) => format!("The backup is too large (max {} MB).", backup::MAX_BACKUP_BYTES >> 20),
    };
    let status = if matches!(e, BackupError::TooLarge) { StatusCode::PAYLOAD_TOO_LARGE } else { StatusCode::BAD_REQUEST };
    ApiError::new(status, msg)
}

// ---------------------------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------------------------

async fn status(State(st): State<AppState>) -> ApiResult<Json<backup::BackupStatus>> {
    let store = st.store.clone();
    let s = blocking(move || store.backup_status()).await?.map_err(ApiError::from)?;
    Ok(Json(s))
}

/// `YYYY-MM-DD` (UTC) for file names.
fn today_iso() -> String {
    // Civil-from-days (Howard Hinnant), days since 1970-01-01.
    let z = crate::puzzles::today_utc() as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn json_attachment(bytes: Vec<u8>, filename: Option<String>) -> Response {
    let mut res = (StatusCode::OK, bytes).into_response();
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(name) = filename {
        if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
    }
    res
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ExportQuery {
    /// `false` skips recording this export as the last backup (used when pushing a sync).
    mark: Option<bool>,
}

async fn export(State(st): State<AppState>, ReqLang(lang): ReqLang, ApiQuery(q): ApiQuery<ExportQuery>) -> ApiResult<Response> {
    let store = st.store.clone();
    let mark = q.mark.unwrap_or(true);
    let bytes = blocking(move || store.export_backup(mark)).await?.map_err(|e| backup_err(e, lang))?;
    Ok(json_attachment(bytes, Some(format!("grandmentor-backup-{}.json", today_iso()))))
}

async fn parse_body(body: Bytes, lang: Lang) -> ApiResult<BackupFile> {
    if body.is_empty() {
        return Err(ApiError::bad_request(tr(lang, "Choose a backup file first.", "Primero elige un archivo de copia de seguridad.")));
    }
    blocking(move || backup::parse_backup(&body)).await?.map_err(|e| file_err(&e, lang))
}

async fn preview(State(st): State<AppState>, ReqLang(lang): ReqLang, body: Bytes) -> ApiResult<Json<backup::BackupPreview>> {
    let file = parse_body(body, lang).await?;
    let store = st.store.clone();
    let p = blocking(move || store.preview_backup(&file)).await?.map_err(|e| backup_err(e, lang))?;
    Ok(Json(p))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ImportQuery {
    mode: Option<String>,
    /// Required for `mode=replace`: must be `replace`.
    confirm: Option<String>,
    /// `sync` records the import as a device sync instead of a restore.
    source: Option<String>,
}

async fn import(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<ImportQuery>,
    body: Bytes,
) -> ApiResult<Json<backup::ImportReport>> {
    let mode = ImportMode::parse(q.mode.as_deref().unwrap_or("merge"))
        .ok_or_else(|| ApiError::bad_request(tr(lang, "`mode` must be `merge` or `replace`.", "`mode` debe ser `merge` o `replace`.")))?;
    if mode == ImportMode::Replace && q.confirm.as_deref() != Some("replace") {
        return Err(ApiError::bad_request(tr(
            lang,
            "Replacing everything needs `confirm=replace`.",
            "Para reemplazarlo todo hace falta `confirm=replace`.",
        )));
    }
    let source = if q.source.as_deref() == Some("sync") { ImportSource::Sync } else { ImportSource::File };
    let file = parse_body(body, lang).await?;
    let store = st.store.clone();
    let report = blocking(move || store.import_backup(&file, mode, source)).await?.map_err(|e| backup_err(e, lang))?;
    Ok(Json(report))
}

// ---------------------------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------------------------

struct Pairing {
    code: String,
    expires: Instant,
    failures: u32,
}

fn pairing() -> &'static Mutex<Option<Pairing>> {
    static P: OnceLock<Mutex<Option<Pairing>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

/// Upper-case, alphanumerics only (users may type `abc-234` or `ABC 234`).
fn normalize_code(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).take(32).map(|c| c.to_ascii_uppercase()).collect()
}

/// Whether `code` matches the active pairing code. Wrong guesses count towards revocation.
pub fn check_pair_code(code: &str) -> bool {
    let code = normalize_code(code);
    let mut guard = pairing().lock();
    let Some(p) = guard.as_mut() else { return false };
    if p.expires <= Instant::now() {
        *guard = None;
        return false;
    }
    // Constant-time comparison over equal-length strings.
    let ok = code.len() == p.code.len() && code.bytes().zip(p.code.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
    if !ok {
        p.failures += 1;
        if p.failures >= MAX_PAIR_FAILURES {
            *guard = None;
        }
    }
    ok
}

/// Read-only check (CORS predicate): does not count failures.
fn code_is_active(code: &str) -> bool {
    let code = normalize_code(code);
    let guard = pairing().lock();
    guard.as_ref().is_some_and(|p| p.expires > Instant::now() && p.code == code)
}

/// Whether this request was made on this machine (loopback peer, local or absent Origin).
fn is_local_request(peer: Option<SocketAddr>, headers: &HeaderMap) -> bool {
    if let Some(addr) = peer {
        if !addr.ip().is_loopback() && !addr.ip().to_canonical().is_loopback() {
            return false;
        }
    }
    match headers.get(header::ORIGIN) {
        Some(o) => crate::is_local_origin(o.as_bytes()),
        None => true,
    }
}

/// Whether the server listens beyond this machine (`GM_HOST` set to a non-loopback address).
fn network_visible() -> bool {
    std::env::var("GM_HOST")
        .ok()
        .and_then(|h| h.trim().parse::<std::net::IpAddr>().ok())
        .is_some_and(|ip| !ip.is_loopback())
}

fn pair_json(p: Option<&Pairing>) -> Value {
    let now = Instant::now();
    match p.filter(|p| p.expires > now) {
        Some(p) => json!({
            "active": true,
            "code": format!("{}-{}", &p.code[..3], &p.code[3..]),
            "expires_in": p.expires.saturating_duration_since(now).as_secs(),
            "network_visible": network_visible(),
        }),
        None => json!({ "active": false, "network_visible": network_visible() }),
    }
}

fn forbid_remote(lang: Lang) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        tr(
            lang,
            "Pairing codes can only be created on the device that runs GrandMentor.",
            "Los códigos de enlace solo se pueden crear en el dispositivo donde se ejecuta GrandMentor.",
        ),
    )
}

async fn pair_status(
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    if !is_local_request(peer.map(|c| c.0), &headers) {
        return Err(forbid_remote(lang));
    }
    let guard = pairing().lock();
    Ok(Json(pair_json(guard.as_ref())))
}

async fn pair_create(
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    if !is_local_request(peer.map(|c| c.0), &headers) {
        return Err(forbid_remote(lang));
    }
    let code: String = {
        let mut rng = rand::thread_rng();
        (0..CODE_LEN).map(|_| CODE_ALPHABET[rng.gen_range(0..CODE_ALPHABET.len())] as char).collect()
    };
    let mut guard = pairing().lock();
    *guard = Some(Pairing { code, expires: Instant::now() + PAIR_TTL, failures: 0 });
    Ok(Json(pair_json(guard.as_ref())))
}

async fn pair_revoke(
    peer: Option<ConnectInfo<SocketAddr>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    if !is_local_request(peer.map(|c| c.0), &headers) {
        return Err(forbid_remote(lang));
    }
    let mut guard = pairing().lock();
    *guard = None;
    Ok(Json(pair_json(None)))
}

/// 401 unless the request carries the active pairing code.
fn require_pair(headers: &HeaderMap, lang: Lang) -> ApiResult<()> {
    let code = headers.get(PAIR_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !code.is_empty() && check_pair_code(code) {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::UNAUTHORIZED,
        tr(
            lang,
            "That pairing code is wrong or has expired. Create a new one on the other device.",
            "Ese código de enlace no es correcto o ha caducado. Crea uno nuevo en el otro dispositivo.",
        ),
    ))
}

async fn sync_snapshot(State(st): State<AppState>, ReqLang(lang): ReqLang, headers: HeaderMap) -> ApiResult<Response> {
    require_pair(&headers, lang)?;
    let store = st.store.clone();
    let bytes = blocking(move || store.export_backup(false)).await?.map_err(|e| backup_err(e, lang))?;
    Ok(json_attachment(bytes, None))
}

async fn sync_merge(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<backup::ImportReport>> {
    require_pair(&headers, lang)?;
    let file = parse_body(body, lang).await?;
    let store = st.store.clone();
    let mut report = blocking(move || store.import_backup(&file, ImportMode::Merge, ImportSource::Sync))
        .await?
        .map_err(|e| backup_err(e, lang))?;
    // Browser settings belong to the device that sent them.
    report.browser = json!({});
    Ok(Json(report))
}

// ---------------------------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------------------------

/// Origins listed in `GM_SYNC_ALLOW_ORIGINS` (comma separated), if set.
fn allowed_origins() -> Option<&'static [String]> {
    static LIST: OnceLock<Option<Vec<String>>> = OnceLock::new();
    LIST.get_or_init(|| {
        std::env::var("GM_SYNC_ALLOW_ORIGINS").ok().map(|v| {
            v.split(',')
                .map(|s| s.trim().trim_end_matches('/').to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .take(64)
                .collect()
        })
    })
    .as_deref()
}

/// CORS predicate for non-local origins: only `/api/sync/snapshot` and `/api/sync/merge`, only
/// for requests carrying a valid pairing code (or preflights announcing the header), and only
/// for `GM_SYNC_ALLOW_ORIGINS` when that is set.
pub fn sync_cors_allowed(origin: &HeaderValue, parts: &Parts) -> bool {
    let path = parts.uri.path();
    if path != "/api/sync/snapshot" && path != "/api/sync/merge" {
        return false;
    }
    let Ok(origin) = origin.to_str() else { return false };
    if let Some(list) = allowed_origins() {
        let o = origin.trim_end_matches('/').to_ascii_lowercase();
        if !list.contains(&o) {
            return false;
        }
    }
    if let Some(code) = parts.headers.get(PAIR_HEADER).and_then(|v| v.to_str().ok()) {
        return code_is_active(code);
    }
    // Preflight: the browser announces the header it will send; the real request is checked.
    parts
        .headers
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|h| h.trim().eq_ignore_ascii_case(PAIR_HEADER)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_date_shape() {
        let d = today_iso();
        assert_eq!(d.len(), 10);
        assert!(d.starts_with("20"));
        assert_eq!(&d[4..5], "-");
    }

    #[test]
    fn code_normalization() {
        assert_eq!(normalize_code(" abc-23x "), "ABC23X");
    }

    #[test]
    fn local_request_rules() {
        let mut h = HeaderMap::new();
        let lo: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let lan: SocketAddr = "192.168.1.9:5000".parse().unwrap();
        assert!(is_local_request(Some(lo), &h));
        assert!(!is_local_request(Some(lan), &h));
        h.insert(header::ORIGIN, HeaderValue::from_static("http://evil.example"));
        assert!(!is_local_request(Some(lo), &h));
    }
}
