//! Static frontend: files from `web/`, SPA fallback to `index.html`, cache headers.
//!
//! PWA support (see docs/CONTRACT.md "Installable app (PWA)"):
//! - `/sw.js` is served with `Cache-Control: no-cache` and `Service-Worker-Allowed: /`. The
//!   `__GM_BUILD__` token inside it is replaced by a build id derived from the web assets
//!   (path + size + mtime), so any change to the frontend makes the worker byte-different and the
//!   browser installs the update.
//! - `/precache-manifest.json` lists every app-shell file the worker should precache, plus the
//!   same build id. The walk is bounded (depth and file count) and runs on a blocking thread.
//! - `/manifest.webmanifest` gets `application/manifest+json`.
//!
//! Security:
//! - HTML responses carry a `Content-Security-Policy` ([`content_security_policy`]). Inline
//!   `<script>` blocks are allowed by their SHA-256 hash, computed from the served file itself
//!   (cached by size + mtime), so editing `index.html` never needs a hard-coded hash update.
//! - `web/dev/*` (component demos) is only served by debug builds or with `GM_DEV=1`; release
//!   builds answer `404` so other devices on the network never see it.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use parking_lot::Mutex;
use serde::Serialize;
use tower::ServiceExt as _;
use tower_http::services::{ServeDir, ServeFile};

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

const SW_PATH: &str = "/sw.js";
const PRECACHE_PATH: &str = "/precache-manifest.json";
const MANIFEST_PATH: &str = "/manifest.webmanifest";
const BUILD_TOKEN: &str = "__GM_BUILD__";
/// Upper bounds for the asset walk and the worker script, so a stray huge directory can't hurt.
const MAX_PRECACHE_FILES: usize = 1500;
const MAX_DEPTH: usize = 8;
const MAX_SW_BYTES: u64 = 512 * 1024;
/// Top-level folders that are not part of the app shell.
const SKIP_TOP_DIRS: &[&str] = &["dev"];
/// Demo pages, only served by debug builds or with `GM_DEV=1`.
const DEV_DIR: &str = "dev";
/// HTML files larger than this get a policy without inline-script hashes.
const MAX_HASHED_HTML: u64 = 2 * 1024 * 1024;
/// Cached policies (one per HTML file).
const MAX_CSP_CACHE: usize = 64;

/// HTML file -> (size, mtime, policy).
type CspCache = Mutex<HashMap<PathBuf, (u64, SystemTime, HeaderValue)>>;

#[derive(Clone)]
struct WebFiles {
    dir: ServeDir,
    index: PathBuf,
    root: PathBuf,
    dev: bool,
    csp: Arc<CspCache>,
}

/// The `Content-Security-Policy` for an HTML page whose inline scripts have these SHA-256
/// hashes (base64). `connect-src` allows `http:`/`https:` because device sync fetches another
/// GrandMentor on the home network by address (Settings → Connect to another device).
pub fn content_security_policy(script_hashes: &[String]) -> String {
    let mut script = String::from("'self'");
    for h in script_hashes {
        script.push_str(" 'sha256-");
        script.push_str(h);
        script.push('\'');
    }
    format!(
        "default-src 'self'; script-src {script}; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
         font-src 'self' https://fonts.gstatic.com; img-src 'self' data: blob:; \
         connect-src 'self' ws: wss: http: https:; worker-src 'self'; manifest-src 'self'; \
         frame-ancestors 'none'; base-uri 'self'; form-action 'self'; object-src 'none'"
    )
}

/// [`content_security_policy`] as a header value.
pub fn csp_header(script_hashes: &[String]) -> HeaderValue {
    HeaderValue::from_str(&content_security_policy(script_hashes))
        .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'; object-src 'none'"))
}

/// Base64 (standard alphabet, padded).
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// SHA-256 (base64) of every inline `<script>` body (no `src` attribute) in `html`.
pub fn inline_script_hashes(html: &str) -> Vec<String> {
    // ASCII lowercasing keeps byte offsets, so indices into `lower` are valid in `html`.
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("<script") {
        let start = pos + i;
        let Some(gt) = lower[start..].find('>') else { break };
        let tag = &lower[start + "<script".len()..start + gt];
        let body_start = start + gt + 1;
        let Some(len) = lower[body_start..].find("</script") else { break };
        pos = body_start + len;
        let is_tag = tag.is_empty() || tag.starts_with(|c: char| c.is_ascii_whitespace() || c == '/');
        let has_src = tag.split(|c: char| c.is_ascii_whitespace()).any(|a| a == "src" || a.starts_with("src="));
        if is_tag && !has_src {
            let digest = ring::digest::digest(&ring::digest::SHA256, &html.as_bytes()[body_start..pos]);
            out.push(base64(digest.as_ref()));
        }
    }
    out.sort();
    out.dedup();
    out
}

impl WebFiles {
    /// Policy for the HTML file at `file` (hashes of its inline scripts; none when unreadable).
    async fn csp_for(&self, file: Option<&Path>) -> HeaderValue {
        let Some(file) = file else { return csp_header(&[]) };
        let Ok(meta) = tokio::fs::metadata(file).await else { return csp_header(&[]) };
        let stamp = (meta.len(), meta.modified().unwrap_or(UNIX_EPOCH));
        if let Some((len, mtime, v)) = self.csp.lock().get(file) {
            if (*len, *mtime) == stamp {
                return v.clone();
            }
        }
        if !meta.is_file() || meta.len() > MAX_HASHED_HTML {
            return csp_header(&[]);
        }
        let Ok(html) = tokio::fs::read_to_string(file).await else { return csp_header(&[]) };
        let value = csp_header(&inline_script_hashes(&html));
        let mut cache = self.csp.lock();
        if cache.len() >= MAX_CSP_CACHE {
            cache.clear();
        }
        cache.insert(file.to_path_buf(), (stamp.0, stamp.1, value.clone()));
        value
    }

    /// The file under `root` that serves `path` (`/` → `index.html`), if `path` is clean.
    fn file_for(&self, path: &str) -> Option<PathBuf> {
        let rel = Path::new(path.trim_start_matches('/'));
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return None;
        }
        let full = self.root.join(rel);
        Some(if path.ends_with('/') || full.is_dir() { full.join("index.html") } else { full })
    }
}

/// `%xx`-decoded path (what `ServeDir` resolves on disk).
fn decode_path(path: &str) -> String {
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len())
            .then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()))
            .flatten();
        match hex {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(b[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `path` points into `web/dev/` (case-insensitive: macOS file systems are).
fn is_dev_path(decoded: &str) -> bool {
    decoded.split('/').find(|seg| !seg.is_empty() && *seg != ".").is_some_and(|seg| seg.eq_ignore_ascii_case(DEV_DIR))
}

/// `GM_DEV=1` (or a debug build) serves `web/dev/*`.
fn dev_pages_enabled() -> bool {
    cfg!(debug_assertions) || std::env::var("GM_DEV").ok().and_then(|v| crate::phone::parse_switch(&v)) == Some(true)
}

/// Router serving `web_dir` (used as the app's fallback for every non-`/api` path).
pub fn router(web_dir: &Path) -> Router {
    router_with(web_dir, dev_pages_enabled())
}

/// [`router`] with an explicit choice about `web/dev/*`.
pub fn router_with(web_dir: &Path, dev: bool) -> Router {
    let files = WebFiles {
        dir: ServeDir::new(web_dir)
            .precompressed_br()
            .precompressed_gzip()
            .append_index_html_on_directories(true),
        index: web_dir.join("index.html"),
        root: web_dir.to_path_buf(),
        dev,
        csp: Arc::default(),
    };
    Router::new().fallback(serve).with_state(Arc::new(files))
}

/// `/vendor/*` and `/img/*` are content-stable → immutable; everything else (index.html, js, css)
/// revalidates every time so dev edits show up immediately (ServeDir sends Last-Modified).
pub fn cache_control_for(path: &str) -> &'static str {
    if path.starts_with("/vendor/") || path.starts_with("/img/") {
        IMMUTABLE
    } else {
        NO_CACHE
    }
}

/// Paths whose last segment looks like a file (`/js/missing.js`) get a real 404 instead of
/// the SPA shell, so broken asset references are visible.
fn looks_like_file(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|seg| seg.contains('.'))
}

/// The app-shell file list and a build id that changes whenever any of those files changes.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AssetIndex {
    pub version: String,
    pub files: Vec<String>,
}

/// Walk `root` (bounded) and build the precache list. Skips dot-files, precompressed variants,
/// `dev/`, the service worker itself and anything that isn't a regular file (no symlinks).
pub fn scan_assets(root: &Path) -> AssetIndex {
    let mut found: Vec<(String, u64, u128)> = Vec::new();
    walk(root, "", 0, &mut found);
    found.sort();
    let mut hasher = DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    for entry in &found {
        entry.hash(&mut hasher);
    }
    // The worker script is not precached but its own edits must still bump the build id.
    if let Ok(meta) = std::fs::metadata(root.join("sw.js")) {
        (meta.len(), mtime_nanos(&meta)).hash(&mut hasher);
    }
    AssetIndex {
        version: format!("{:016x}", hasher.finish()),
        files: found.into_iter().map(|(p, _, _)| p).collect(),
    }
}

fn mtime_nanos(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<(String, u64, u128)>) {
    if depth > MAX_DEPTH || out.len() >= MAX_PRECACHE_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= MAX_PRECACHE_FILES {
            return;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || name.ends_with(".br") || name.ends_with(".gz") {
            continue;
        }
        // file_type() does not follow symlinks: links are skipped entirely.
        let Ok(ft) = entry.file_type() else { continue };
        let rel = format!("{prefix}/{name}");
        if ft.is_dir() {
            if depth == 0 && SKIP_TOP_DIRS.contains(&name) {
                continue;
            }
            walk(&entry.path(), &rel, depth + 1, out);
        } else if ft.is_file() {
            if rel == SW_PATH || rel == PRECACHE_PATH {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            out.push((rel, meta.len(), mtime_nanos(&meta)));
        }
    }
}

fn pwa_headers(res: &mut Response) {
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_CACHE));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
}

async fn serve_sw(files: &WebFiles) -> Response {
    let root = files.root.clone();
    let built = tokio::task::spawn_blocking(move || {
        let path = root.join("sw.js");
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > MAX_SW_BYTES {
            return None;
        }
        let src = std::fs::read_to_string(&path).ok()?;
        let index = scan_assets(&root);
        Some(src.replace(BUILD_TOKEN, &index.version))
    })
    .await
    .ok()
    .flatten();
    let mut res = match built {
        Some(body) => (
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/javascript; charset=utf-8"),
                ),
                (
                    HeaderName::from_static("service-worker-allowed"),
                    HeaderValue::from_static("/"),
                ),
            ],
            body,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    };
    pwa_headers(&mut res);
    res
}

async fn serve_precache(files: &WebFiles) -> Response {
    let root = files.root.clone();
    let mut res = match tokio::task::spawn_blocking(move || scan_assets(&root)).await {
        Ok(index) => axum::Json(index).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": "could not list app files" })),
        )
            .into_response(),
    };
    pwa_headers(&mut res);
    res
}

async fn serve(State(files): State<Arc<WebFiles>>, req: Request) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }
    let path = req.uri().path().to_string();
    if path == SW_PATH {
        return serve_sw(&files).await;
    }
    if path == PRECACHE_PATH {
        return serve_precache(&files).await;
    }
    let decoded = decode_path(&path);
    if !files.dev && is_dev_path(&decoded) {
        let mut res = StatusCode::NOT_FOUND.into_response();
        res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_CACHE));
        return res;
    }
    let (parts, _) = req.into_parts();

    let res = match files
        .dir
        .clone()
        .oneshot(Request::from_parts(parts.clone(), Body::empty()))
        .await
    {
        Ok(r) => r.map(Body::new),
        Err(never) => match never {},
    };
    let fallback = res.status() == StatusCode::NOT_FOUND && !looks_like_file(&path);
    let mut res = if fallback {
        // SPA fallback (hash routing makes this rare, but deep links still work).
        let mut index_req = Request::from_parts(parts, Body::empty());
        *index_req.uri_mut() = axum::http::Uri::from_static("/index.html");
        match ServeFile::new(&files.index).oneshot(index_req).await {
            Ok(r) => r.map(Body::new),
            Err(never) => match never {},
        }
    } else {
        res
    };
    let ok = res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED;
    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map_or(res.status() == StatusCode::NOT_MODIFIED && (decoded.ends_with(".html") || !looks_like_file(&decoded)), |t| {
            t.starts_with("text/html")
        });
    if is_html {
        let file = if fallback { Some(files.index.clone()) } else { files.file_for(&decoded) };
        let csp = files.csp_for(file.as_deref()).await;
        res.headers_mut().insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    let cc = if ok { cache_control_for(&path) } else { NO_CACHE };
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cc));
    if ok && path == MANIFEST_PATH {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/manifest+json"),
        );
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt as _;

    #[test]
    fn cache_rules() {
        assert_eq!(cache_control_for("/vendor/chess.js"), IMMUTABLE);
        assert_eq!(cache_control_for("/img/pieces/cburnett/wK.svg"), IMMUTABLE);
        assert_eq!(cache_control_for("/index.html"), NO_CACHE);
        assert_eq!(cache_control_for("/js/app.js"), NO_CACHE);
        assert_eq!(cache_control_for("/sw.js"), NO_CACHE);
        assert!(looks_like_file("/js/x.js"));
        assert!(!looks_like_file("/play/max"));
    }

    /// A unique scratch web root for one test.
    fn temp_web(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gm-web-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let mk = |p: &str, body: &str| {
            let full = dir.join(p);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(full, body).unwrap();
        };
        mk("index.html", "<!doctype html><title>t</title>");
        mk("sw.js", "const BUILD = '__GM_BUILD__';");
        mk("manifest.webmanifest", "{\"name\":\"GrandMentor\"}");
        mk("js/app.js", "export {};");
        mk("js/app.js.gz", "x");
        mk("dev/demo.html", "x");
        mk(".hidden", "x");
        dir
    }

    #[test]
    fn scan_lists_shell_files_and_skips_noise() {
        let dir = temp_web("scan");
        let a = scan_assets(&dir);
        assert_eq!(
            a.files,
            vec!["/index.html", "/js/app.js", "/manifest.webmanifest"]
        );
        assert_eq!(a.version.len(), 16);
        // Stable when nothing changes, different when a file changes size.
        assert_eq!(scan_assets(&dir).version, a.version);
        std::fs::write(dir.join("js/app.js"), "export const x = 1;").unwrap();
        assert_ne!(scan_assets(&dir).version, a.version);
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn get(app: Router, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let res = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, headers, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn serves_service_worker_manifest_and_precache() {
        let dir = temp_web("serve");
        let app = router(&dir);

        let (status, headers, body) = get(app.clone(), "/sw.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], NO_CACHE);
        assert_eq!(headers["service-worker-allowed"], "/");
        assert!(headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/javascript"));
        assert!(!body.contains(BUILD_TOKEN));
        let version = scan_assets(&dir).version;
        assert!(body.contains(&version));

        let (status, headers, _) = get(app.clone(), "/manifest.webmanifest").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "application/manifest+json");
        assert_eq!(headers[header::CACHE_CONTROL], NO_CACHE);

        let (status, headers, body) = get(app.clone(), "/precache-manifest.json").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CACHE_CONTROL], NO_CACHE);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["version"], version);
        assert!(v["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "/js/app.js"));

        let (status, _, _) = get(app, "/learn/some/deep/link").await;
        assert_eq!(status, StatusCode::OK);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inline_script_hashes_cover_inline_blocks_only() {
        let html = "<script>a()</script><SCRIPT type=\"module\" src=\"/x.js\"></SCRIPT><scripts></scripts><script type=\"module\">b()</script>";
        let h = inline_script_hashes(html);
        assert_eq!(h.len(), 2);
        // echo -n 'a()' | openssl dgst -sha256 -binary | base64
        assert!(h.contains(&"qVpDBgj7bpq5hMAcGp3AOc79J3Y1Z4HvySTwKrWDoy4=".to_string()), "{h:?}");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        let csp = content_security_policy(&h);
        assert!(csp.contains("script-src 'self' 'sha256-"));
        assert!(csp.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn dev_paths() {
        assert!(is_dev_path(&decode_path("/dev/board-demo.html")));
        assert!(is_dev_path(&decode_path("/%64ev/board-demo.html")));
        assert!(is_dev_path(&decode_path("//DEV/")));
        assert!(is_dev_path("/dev"));
        assert!(!is_dev_path("/js/dev/x.js"));
        assert!(!is_dev_path("/developer"));
    }

    #[tokio::test]
    async fn dev_pages_and_csp() {
        let dir = temp_web("dev");
        std::fs::write(dir.join("index.html"), "<!doctype html><script>boot()</script><script src=\"/js/app.js\"></script>").unwrap();
        let (status, _, _) = get(router_with(&dir, false), "/dev/demo.html").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = get(router_with(&dir, false), "/%64ev/demo.html").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = get(router_with(&dir, true), "/dev/demo.html").await;
        assert_eq!(status, StatusCode::OK);

        let app = router_with(&dir, false);
        let want = format!("'sha256-{}'", inline_script_hashes("<script>boot()</script>")[0]);
        for path in ["/", "/index.html", "/play/max"] {
            let (status, headers, _) = get(app.clone(), path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
            assert!(csp.contains(&want), "{path}: {csp}");
        }
        // Editing the page updates the hash.
        std::fs::write(dir.join("index.html"), "<!doctype html><script>boot2()</script>").unwrap();
        let (_, headers, _) = get(app.clone(), "/").await;
        let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(csp.contains(&format!("'sha256-{}'", inline_script_hashes("<script>boot2()</script>")[0])), "{csp}");
        // Not on other files.
        let (_, headers, _) = get(app, "/js/app.js").await;
        assert!(headers.get(header::CONTENT_SECURITY_POLICY).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_service_worker_is_404() {
        let dir = temp_web("missing");
        std::fs::remove_file(dir.join("sw.js")).unwrap();
        let (status, _, _) = get(router(&dir), "/sw.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
