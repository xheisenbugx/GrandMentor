//! Backup export / import (`/api/backup/*`) and device sync (`/api/sync/*`).
//!
//! Security model for sync (see docs/CONTRACT.md "Backup & sync"):
//! * `POST /api/sync/pair` only answers requests coming from this machine (loopback peer and,
//!   when an `Origin` is sent, a localhost origin). It creates a short pairing code that lives
//!   in memory ([`PairState`] in `AppState`) for [`PAIR_TTL`] — one code at a time, wiped on
//!   `DELETE /api/sync/pair` or after [`MAX_PAIR_FAILURES`] wrong guesses in total. Wrong codes
//!   are limited per address ([`MAX_PAIR_FAILURES_PER_IP`] per [`PAIR_TTL`] → `429`), so one
//!   device guessing cannot revoke the code for everybody else.
//! * `GET /api/sync/snapshot` and `POST /api/sync/merge` always require that code in the
//!   `X-GM-Pair` header — with or without CORS. `merge` checks it *before* reading the body.
//! * CORS for those two endpoints (only) is granted to non-local origins when the request
//!   carries a valid code (preflights: when they announce the `x-gm-pair` header). Setting
//!   `GM_SYNC_ALLOW_ORIGINS=http://a:8080,http://b:8080` restricts that to the listed origins.
//! * The server still binds to 127.0.0.1 by default; another device can only reach it after the
//!   user starts it with `GM_HOST=0.0.0.0` (or a LAN address) or turns on phone mode. The access
//!   PIN gate (`crate::phone::gate`) lets `snapshot` / `merge` through: they have their own code.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, DefaultBodyLimit, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use gm_content::Lang;
use gm_store::backup::{self, BackupError, BackupFile, ImportMode, ImportOptions, ImportSource};
use parking_lot::Mutex;
use rand::RngExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::blocking;
use crate::error::{ApiError, ApiQuery, ApiResult};
use crate::lang::ReqLang;
use crate::state::AppState;

/// Pairing codes expire after this long.
pub const PAIR_TTL: Duration = Duration::from_secs(10 * 60);
/// Wrong codes (from all addresses together) tolerated before the active code is revoked.
pub const MAX_PAIR_FAILURES: u32 = 200;
/// Wrong codes one address may send per [`PAIR_TTL`] before it has to wait (`429`).
pub const MAX_PAIR_FAILURES_PER_IP: u32 = 10;
/// Addresses remembered by the wrong-code limiter.
const MAX_TRACKED_IPS: usize = 1024;
/// Request header carrying the pairing code.
pub const PAIR_HEADER: &str = "x-gm-pair";
/// The two endpoints other devices call with a pairing code (no access PIN, CORS allowed).
pub const SYNC_PATHS: &[&str] = &["/api/sync/snapshot", "/api/sync/merge"];
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
        // Reads its body itself (after the pairing check), bounded by BACKUP_BODY_LIMIT.
        .route("/sync/merge", post(sync_merge))
}

// ---------------------------------------------------------------------------------------------
// Localized messages
// ---------------------------------------------------------------------------------------------

/// Pick the message for `lang` from `[en, es, pt, fr, de]`.
fn tr(lang: Lang, s: [&str; 5]) -> String {
    match lang {
        Lang::En => s[0],
        Lang::Es => s[1],
        Lang::Pt => s[2],
        Lang::Fr => s[3],
        Lang::De => s[4],
    }
    .to_string()
}

/// Maps store errors to `{error}`; backup-file problems become a localized 400.
fn backup_err(e: anyhow::Error, lang: Lang) -> ApiError {
    match e.downcast_ref::<BackupError>() {
        Some(be) => file_err(be, lang),
        None => ApiError::from(e),
    }
}

fn file_err(e: &BackupError, lang: Lang) -> ApiError {
    let mb = backup::MAX_BACKUP_BYTES >> 20;
    let msg = match (e, lang) {
        (BackupError::Malformed(_), _) => tr(
            lang,
            [
                "This file is not a valid backup.",
                "Este archivo no es una copia de seguridad válida.",
                "Este arquivo não é um backup válido.",
                "Ce fichier n'est pas une sauvegarde valide.",
                "Diese Datei ist keine gültige Sicherung.",
            ],
        ),
        (BackupError::NotABackup, _) => tr(
            lang,
            [
                "This file is not a GrandMentor backup.",
                "Este archivo no es una copia de seguridad de GrandMentor.",
                "Este arquivo não é um backup do GrandMentor.",
                "Ce fichier n'est pas une sauvegarde GrandMentor.",
                "Diese Datei ist keine GrandMentor-Sicherung.",
            ],
        ),
        (BackupError::UnsupportedVersion { found, supported }, Lang::En) => format!(
            "This backup uses format {found}, but this GrandMentor only understands up to {supported}. Update the app and try again."
        ),
        (BackupError::UnsupportedVersion { found, supported }, Lang::Es) => format!(
            "Esta copia usa el formato {found}, pero esta versión de GrandMentor solo entiende hasta el {supported}. Actualiza la aplicación e inténtalo de nuevo."
        ),
        (BackupError::UnsupportedVersion { found, supported }, Lang::Pt) => format!(
            "Este backup usa o formato {found}, mas esta versão do GrandMentor só entende até o {supported}. Atualize o aplicativo e tente de novo."
        ),
        (BackupError::UnsupportedVersion { found, supported }, Lang::Fr) => format!(
            "Cette sauvegarde utilise le format {found}, mais cette version de GrandMentor ne comprend que jusqu'au format {supported}. Mets l'application à jour et réessaie."
        ),
        (BackupError::UnsupportedVersion { found, supported }, Lang::De) => format!(
            "Diese Sicherung verwendet Format {found}, aber diese GrandMentor-Version versteht nur bis Format {supported}. Aktualisiere die App und versuch es erneut."
        ),
        (BackupError::TooLarge, Lang::En) => format!("The backup is too large (max {mb} MB)."),
        (BackupError::TooLarge, Lang::Es) => format!("La copia es demasiado grande (máximo {mb} MB)."),
        (BackupError::TooLarge, Lang::Pt) => format!("O backup é grande demais (máximo de {mb} MB)."),
        (BackupError::TooLarge, Lang::Fr) => format!("La sauvegarde est trop volumineuse ({mb} Mo maximum)."),
        (BackupError::TooLarge, Lang::De) => format!("Die Sicherung ist zu groß (höchstens {mb} MB)."),
        (BackupError::RowsFailed { failed }, Lang::En) => format!(
            "{failed} entries in this backup could not be restored, so nothing was changed. Your current data is untouched."
        ),
        (BackupError::RowsFailed { failed }, Lang::Es) => format!(
            "No se pudieron restaurar {failed} elementos de esta copia, así que no se ha cambiado nada. Tus datos actuales siguen intactos."
        ),
        (BackupError::RowsFailed { failed }, Lang::Pt) => format!(
            "Não foi possível restaurar {failed} itens deste backup, então nada foi alterado. Seus dados atuais continuam intactos."
        ),
        (BackupError::RowsFailed { failed }, Lang::Fr) => format!(
            "{failed} éléments de cette sauvegarde n'ont pas pu être restaurés, donc rien n'a été modifié. Tes données actuelles sont intactes."
        ),
        (BackupError::RowsFailed { failed }, Lang::De) => format!(
            "{failed} Einträge dieser Sicherung konnten nicht wiederhergestellt werden, deshalb wurde nichts geändert. Deine aktuellen Daten sind unverändert."
        ),
    };
    let status = match e {
        BackupError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        BackupError::RowsFailed { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::BAD_REQUEST,
    };
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

/// Backup bodies must be sent as `application/json`: a browser can only send that type to
/// another site after a CORS preflight, which closes the "simple request" CSRF path.
fn require_json(headers: &HeaderMap, lang: Lang) -> ApiResult<()> {
    let ok = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
    if ok {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        tr(
            lang,
            [
                "Send the backup file as JSON (Content-Type: application/json).",
                "Envía la copia de seguridad como JSON (Content-Type: application/json).",
                "Envie o backup como JSON (Content-Type: application/json).",
                "Envoie la sauvegarde en JSON (Content-Type: application/json).",
                "Sende die Sicherung als JSON (Content-Type: application/json).",
            ],
        ),
    ))
}

async fn parse_body(body: Bytes, lang: Lang) -> ApiResult<BackupFile> {
    if body.is_empty() {
        return Err(ApiError::bad_request(tr(
            lang,
            [
                "Choose a backup file first.",
                "Primero elige un archivo de copia de seguridad.",
                "Primeiro escolha um arquivo de backup.",
                "Choisis d'abord un fichier de sauvegarde.",
                "Wähle zuerst eine Sicherungsdatei aus.",
            ],
        )));
    }
    blocking(move || backup::parse_backup(&body)).await?.map_err(|e| file_err(&e, lang))
}

async fn preview(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<backup::BackupPreview>> {
    require_json(&headers, lang)?;
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
    /// `1` / `true`: a replace that could not restore every row still commits (otherwise it is
    /// rolled back and answered with `422`).
    allow_partial: Option<String>,
}

async fn import(
    State(st): State<AppState>,
    ReqLang(lang): ReqLang,
    ApiQuery(q): ApiQuery<ImportQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<backup::ImportReport>> {
    require_json(&headers, lang)?;
    let mode = ImportMode::parse(q.mode.as_deref().unwrap_or("merge"))
        .ok_or_else(|| ApiError::bad_request(tr(
                lang,
                [
                    "`mode` must be `merge` or `replace`.",
                    "`mode` debe ser `merge` o `replace`.",
                    "`mode` deve ser `merge` ou `replace`.",
                    "`mode` doit valoir `merge` ou `replace`.",
                    "`mode` muss `merge` oder `replace` sein.",
                ],
            )))?;
    if mode == ImportMode::Replace && q.confirm.as_deref() != Some("replace") {
        return Err(ApiError::bad_request(tr(
            lang,
            [
                "Replacing everything needs `confirm=replace`.",
                "Para reemplazarlo todo hace falta `confirm=replace`.",
                "Para substituir tudo é preciso `confirm=replace`.",
                "Pour tout remplacer, il faut `confirm=replace`.",
                "Zum Ersetzen von allem ist `confirm=replace` nötig.",
            ],
        )));
    }
    let source = if q.source.as_deref() == Some("sync") { ImportSource::Sync } else { ImportSource::File };
    let opts = ImportOptions { allow_partial: matches!(q.allow_partial.as_deref(), Some("1" | "true")) };
    let file = parse_body(body, lang).await?;
    let store = st.store.clone();
    let report = blocking(move || store.import_backup_with(&file, mode, source, opts))
        .await?
        .map_err(|e| backup_err(e, lang))?;
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

/// Wrong codes from one address within a [`PAIR_TTL`] window.
#[derive(Clone, Copy)]
struct Misses {
    start: Instant,
    count: u32,
}

/// The active pairing code and the wrong-code limiter (one per server, in `AppState`).
#[derive(Default)]
pub struct PairState {
    code: Mutex<Option<Pairing>>,
    misses: Mutex<HashMap<IpAddr, Misses>>,
}

/// Why a pairing code was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRejected {
    /// Wrong, expired or no active code.
    Wrong,
    /// This address sent too many wrong codes; try again in this many seconds.
    Blocked(u64),
}

impl PairState {
    /// Seconds `ip` has to wait before it may try a code again.
    fn blocked(&self, ip: IpAddr, now: Instant) -> Option<u64> {
        let misses = self.misses.lock();
        let m = misses.get(&ip)?;
        let elapsed = now.saturating_duration_since(m.start);
        (elapsed < PAIR_TTL && m.count >= MAX_PAIR_FAILURES_PER_IP).then(|| PAIR_TTL.saturating_sub(elapsed).as_secs().max(1))
    }

    fn record_miss(&self, ip: IpAddr, now: Instant) {
        let mut misses = self.misses.lock();
        if misses.len() >= MAX_TRACKED_IPS && !misses.contains_key(&ip) {
            misses.retain(|_, m| now.saturating_duration_since(m.start) < PAIR_TTL);
            if misses.len() >= MAX_TRACKED_IPS {
                if let Some(oldest) = misses.iter().min_by_key(|(_, m)| m.start).map(|(k, _)| *k) {
                    misses.remove(&oldest);
                }
            }
        }
        let m = misses.entry(ip).or_insert(Misses { start: now, count: 0 });
        if now.saturating_duration_since(m.start) >= PAIR_TTL {
            *m = Misses { start: now, count: 0 };
        }
        m.count += 1;
    }

    /// Checks `code` sent by `ip` against the active pairing code. Wrong guesses count towards
    /// that address's limit and towards revoking the code ([`MAX_PAIR_FAILURES`] in total).
    pub fn check(&self, ip: IpAddr, code: &str) -> Result<(), PairRejected> {
        let now = Instant::now();
        if let Some(wait) = self.blocked(ip, now) {
            return Err(PairRejected::Blocked(wait));
        }
        let code = normalize_code(code);
        let ok = {
            let mut guard = self.code.lock();
            let Some(p) = guard.as_mut() else { return self.miss(ip, now) };
            if p.expires <= now {
                *guard = None;
                return self.miss(ip, now);
            }
            // Constant-time comparison over equal-length strings.
            let ok = code.len() == p.code.len() && code.bytes().zip(p.code.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
            if !ok {
                p.failures += 1;
                if p.failures >= MAX_PAIR_FAILURES {
                    tracing::warn!("pairing code revoked after {MAX_PAIR_FAILURES} wrong attempts");
                    *guard = None;
                }
            }
            ok
        };
        if ok {
            self.misses.lock().remove(&ip);
            Ok(())
        } else {
            self.miss(ip, now)
        }
    }

    fn miss(&self, ip: IpAddr, now: Instant) -> Result<(), PairRejected> {
        self.record_miss(ip, now);
        Err(PairRejected::Wrong)
    }

    /// Read-only check (CORS predicate): does not count failures.
    fn code_is_active(&self, code: &str) -> bool {
        let code = normalize_code(code);
        let guard = self.code.lock();
        guard.as_ref().is_some_and(|p| p.expires > Instant::now() && p.code == code)
    }
}

/// Upper-case, alphanumerics only (users may type `abc-234` or `ABC 234`).
fn normalize_code(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).take(32).map(|c| c.to_ascii_uppercase()).collect()
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
        || crate::phone::network_visible()
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
            [
                "Pairing codes can only be created on the device that runs GrandMentor.",
                "Los códigos de enlace solo se pueden crear en el dispositivo donde se ejecuta GrandMentor.",
                "Os códigos de pareamento só podem ser criados no dispositivo que executa o GrandMentor.",
                "Les codes d'appairage ne peuvent être créés que sur l'appareil qui exécute GrandMentor.",
                "Kopplungscodes können nur auf dem Gerät erstellt werden, auf dem GrandMentor läuft.",
            ],
        ),
    )
}

async fn pair_status(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    if !is_local_request(peer.map(|Extension(c)| c.0), &headers) {
        return Err(forbid_remote(lang));
    }
    let guard = st.pairing.code.lock();
    Ok(Json(pair_json(guard.as_ref())))
}

async fn pair_create(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    if !is_local_request(peer.map(|Extension(c)| c.0), &headers) {
        return Err(forbid_remote(lang));
    }
    let code: String = {
        let mut rng = rand::rng();
        (0..CODE_LEN).map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char).collect()
    };
    // A fresh code is a fresh start for devices that mistyped the old one.
    st.pairing.misses.lock().clear();
    let mut guard = st.pairing.code.lock();
    *guard = Some(Pairing { code, expires: Instant::now() + PAIR_TTL, failures: 0 });
    Ok(Json(pair_json(guard.as_ref())))
}

async fn pair_revoke(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Json<Value>> {
    if !is_local_request(peer.map(|Extension(c)| c.0), &headers) {
        return Err(forbid_remote(lang));
    }
    *st.pairing.code.lock() = None;
    Ok(Json(pair_json(None)))
}

/// Address used for the wrong-code limiter (in-process callers count as this machine).
fn peer_ip(peer: Option<&Extension<ConnectInfo<SocketAddr>>>) -> IpAddr {
    peer.map_or(IpAddr::from([127, 0, 0, 1]), |Extension(c)| c.0.ip().to_canonical())
}

/// `401` unless the request carries the active pairing code; `429` for an address that sent
/// too many wrong codes.
fn require_pair(pairing: &PairState, ip: IpAddr, headers: &HeaderMap, lang: Lang) -> ApiResult<()> {
    let code = headers.get(PAIR_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    let result = if code.is_empty() { Err(PairRejected::Wrong) } else { pairing.check(ip, code) };
    match result {
        Ok(()) => Ok(()),
        Err(PairRejected::Blocked(wait)) => {
            let mins = wait.div_ceil(60);
            let msg = match lang {
                Lang::En => format!("Too many wrong pairing codes from this device. Try again in {mins} min."),
                Lang::Es => format!("Demasiados códigos de enlace incorrectos desde este dispositivo. Vuelve a intentarlo en {mins} min."),
                Lang::Pt => format!("Códigos de pareamento errados demais neste dispositivo. Tente de novo em {mins} min."),
                Lang::Fr => format!("Trop de codes d'appairage incorrects depuis cet appareil. Réessaie dans {mins} min."),
                Lang::De => format!("Zu viele falsche Kopplungscodes von diesem Gerät. Versuch es in {mins} Min. erneut."),
            };
            Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, msg))
        }
        Err(PairRejected::Wrong) => Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            tr(
                lang,
                [
                    "That pairing code is wrong or has expired. Create a new one on the other device.",
                    "Ese código de enlace no es correcto o ha caducado. Crea uno nuevo en el otro dispositivo.",
                    "Esse código de pareamento está errado ou expirou. Crie um novo no outro dispositivo.",
                    "Ce code d'appairage est incorrect ou a expiré. Crées-en un nouveau sur l'autre appareil.",
                    "Dieser Kopplungscode ist falsch oder abgelaufen. Erstelle auf dem anderen Gerät einen neuen.",
                ],
            ),
        )),
    }
}

async fn sync_snapshot(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
) -> ApiResult<Response> {
    require_pair(&st.pairing, peer_ip(peer.as_ref()), &headers, lang)?;
    let store = st.store.clone();
    let bytes = blocking(move || store.export_backup(false)).await?.map_err(|e| backup_err(e, lang))?;
    Ok(json_attachment(bytes, None))
}

/// Takes the raw body so the pairing code is checked before a single byte is buffered.
async fn sync_merge(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    ReqLang(lang): ReqLang,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Json<backup::ImportReport>> {
    require_pair(&st.pairing, peer_ip(peer.as_ref()), &headers, lang)?;
    require_json(&headers, lang)?;
    let too_large = || file_err(&BackupError::TooLarge, lang);
    let declared = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > BACKUP_BODY_LIMIT as u64) {
        return Err(too_large());
    }
    // Errors here are the limit or a client that went away; either way nothing was imported.
    let body = axum::body::to_bytes(body, BACKUP_BODY_LIMIT).await.map_err(|_| too_large())?;
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
pub fn sync_cors_allowed(pairing: &PairState, origin: &HeaderValue, parts: &Parts) -> bool {
    if !SYNC_PATHS.contains(&parts.uri.path()) {
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
        return pairing.code_is_active(code);
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
