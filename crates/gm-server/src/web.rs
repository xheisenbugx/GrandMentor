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

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
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

#[derive(Clone)]
struct WebFiles {
    dir: ServeDir,
    index: PathBuf,
    root: PathBuf,
}

/// Router serving `web_dir` (used as the app's fallback for every non-`/api` path).
pub fn router(web_dir: &Path) -> Router {
    let files = WebFiles {
        dir: ServeDir::new(web_dir)
            .precompressed_br()
            .precompressed_gzip()
            .append_index_html_on_directories(true),
        index: web_dir.join("index.html"),
        root: web_dir.to_path_buf(),
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
    let mut res = if res.status() == StatusCode::NOT_FOUND && !looks_like_file(&path) {
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

    #[tokio::test]
    async fn missing_service_worker_is_404() {
        let dir = temp_web("missing");
        std::fs::remove_file(dir.join("sw.js")).unwrap();
        let (status, _, _) = get(router(&dir), "/sw.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
