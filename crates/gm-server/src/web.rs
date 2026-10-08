//! Static frontend: files from `web/`, SPA fallback to `index.html`, cache headers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower::ServiceExt as _;
use tower_http::services::{ServeDir, ServeFile};

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

#[derive(Clone)]
struct WebFiles {
    dir: ServeDir,
    index: PathBuf,
}

/// Router serving `web_dir` (used as the app's fallback for every non-`/api` path).
pub fn router(web_dir: &Path) -> Router {
    let files = WebFiles {
        dir: ServeDir::new(web_dir)
            .precompressed_br()
            .precompressed_gzip()
            .append_index_html_on_directories(true),
        index: web_dir.join("index.html"),
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

async fn serve(State(files): State<Arc<WebFiles>>, req: Request) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }
    let path = req.uri().path().to_string();
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
    let cc = if res.status().is_success() || res.status() == StatusCode::NOT_MODIFIED {
        cache_control_for(&path)
    } else {
        NO_CACHE
    };
    let headers = res.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cc));
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

    #[test]
    fn cache_rules() {
        assert_eq!(cache_control_for("/vendor/chess.js"), IMMUTABLE);
        assert_eq!(cache_control_for("/img/pieces/cburnett/wK.svg"), IMMUTABLE);
        assert_eq!(cache_control_for("/index.html"), NO_CACHE);
        assert_eq!(cache_control_for("/js/app.js"), NO_CACHE);
        assert!(looks_like_file("/js/x.js"));
        assert!(!looks_like_file("/play/max"));
    }
}
