//! GrandMentor HTTP server: REST + WebSocket + static frontend (CONTRACT §4).

pub mod api;
pub mod cache;
pub mod error;
pub mod puzzles;
pub mod state;
pub mod web;
pub mod ws;

use std::path::Path;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::{header, HeaderValue, Method};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;

pub use error::ApiError;
pub use state::AppState;

/// Request body limit for every endpoint.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// CORS: allow any port on localhost / 127.0.0.1 / [::1] (dev servers, file previews).
pub fn is_local_origin(origin: &[u8]) -> bool {
    let Ok(origin) = std::str::from_utf8(origin) else {
        return false;
    };
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let Some(rest) = rest else { return false };
    let host = if let Some(v6) = rest.strip_prefix('[') {
        match v6.split_once(']') {
            Some((h, tail)) if tail.is_empty() || tail.starts_with(':') => return h == "::1",
            _ => return false,
        }
    } else {
        rest.split(':').next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1")
}

fn panic_response(_err: Box<dyn std::any::Any + Send + 'static>) -> Response {
    tracing::error!("request handler panicked");
    ApiError::internal("internal error").into_response()
}

/// Build the full application router.
pub fn app(state: AppState, web_dir: &Path) -> Router {
    let api = api::router()
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &axum::http::Request<axum::body::Body>| {
                    // Log the full path (nesting strips the `/api` prefix from `req.uri()`).
                    let uri = req
                        .extensions()
                        .get::<axum::extract::OriginalUri>()
                        .map(|u| u.0.path().to_string())
                        .unwrap_or_else(|| req.uri().path().to_string());
                    tracing::info_span!("request", method = %req.method(), uri = %uri)
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        );

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            is_local_origin(origin.as_bytes())
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([header::CONTENT_TYPE, header::ACCEPT])
        .max_age(Duration::from_secs(3600));

    Router::new()
        .nest("/api", api)
        .with_state(state)
        .fallback_service(web::router(web_dir))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(CatchPanicLayer::custom(panic_response))
}

#[cfg(test)]
mod tests {
    use super::is_local_origin;

    #[test]
    fn local_origins() {
        assert!(is_local_origin(b"http://localhost:8080"));
        assert!(is_local_origin(b"http://localhost"));
        assert!(is_local_origin(b"http://127.0.0.1:5173"));
        assert!(is_local_origin(b"http://[::1]:3000"));
        assert!(!is_local_origin(b"http://localhost.evil.com"));
        assert!(!is_local_origin(b"https://example.com"));
        assert!(!is_local_origin(b"null"));
    }
}
