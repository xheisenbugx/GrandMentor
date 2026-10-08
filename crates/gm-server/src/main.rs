//! `grandmentor` — axum server: REST + WebSocket + static frontend.
//!
//! Env: GM_PORT (8080), GM_HOST (127.0.0.1), GM_DATA_DIR (./data), GM_DB (./grandmentor.db),
//! GM_WEB_DIR (./web), ANTHROPIC_API_KEY (optional), GM_MENTOR_MODEL (claude-opus-5-5),
//! GM_ENGINES (default: available cores - 1, clamped to 2..=8), GM_TT_MB (default 32 per engine).

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use gm_server::{app, AppState};

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_num<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,tower_http=info")),
        )
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("gm-worker")
        .build()?;
    let result = rt.block_on(run());
    // Don't hang on exit if a long review is still running on a blocking thread.
    rt.shutdown_timeout(Duration::from_secs(2));
    result
}

async fn run() -> anyhow::Result<()> {
    let port: u16 = env_num("GM_PORT", 8080);
    let host: IpAddr = env_num("GM_HOST", IpAddr::from([127, 0, 0, 1]));
    let data_dir = PathBuf::from(env_or("GM_DATA_DIR", "./data"));
    let db_path = PathBuf::from(env_or("GM_DB", "./grandmentor.db"));
    let web_dir = PathBuf::from(env_or("GM_WEB_DIR", "./web"));
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let engines: usize = env_num("GM_ENGINES", cores.saturating_sub(1).clamp(2, 8)).clamp(1, 64);
    let tt_mb: usize = env_num("GM_TT_MB", 32usize).clamp(1, 2048);

    if !web_dir.join("index.html").is_file() {
        tracing::warn!(
            "{} has no index.html — the UI will not load (set GM_WEB_DIR)",
            web_dir.display()
        );
    }

    let content = {
        let dir = data_dir.clone();
        tokio::task::spawn_blocking(move || gm_content::Content::load(&dir))
            .await
            .context("content loader task failed")?
            .with_context(|| format!("loading content from {}", data_dir.display()))?
    };
    let store = {
        let p = db_path.clone();
        tokio::task::spawn_blocking(move || gm_store::Store::open(&p))
            .await
            .context("store task failed")?
            .with_context(|| format!("opening database {}", db_path.display()))?
    };
    let state = AppState::new(
        Arc::new(content),
        gm_engine::EnginePool::new(engines, tt_mb),
        store,
        Arc::new(gm_mentor::Mentor::from_env()),
    );
    tracing::info!(
        engines,
        tt_mb,
        llm = state.mentor.llm_enabled(),
        "engine pool ready"
    );

    let addr = SocketAddr::new(host, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let shown = if host.is_unspecified() || host.is_loopback() {
        "localhost".to_string()
    } else {
        host.to_string()
    };
    tracing::info!("GrandMentor is running → open http://{shown}:{port} (Ctrl-C to stop)");

    let shutdown = Arc::clone(&state.shutdown);
    let mut shutdown_rx = shutdown.subscribe();
    let server = axum::serve(listener, app(state, &web_dir)).with_graceful_shutdown(async move {
        let _ = shutdown_rx.wait_for(|v| *v).await;
    });
    let mut server = tokio::spawn(async move { server.await });

    tokio::select! {
        res = &mut server => {
            return res.context("server task failed")?.context("server error");
        }
        _ = shutdown_signal() => {}
    }
    tracing::info!("shutting down: closing connections…");
    shutdown.send_replace(true);
    match tokio::time::timeout(Duration::from_secs(10), server).await {
        Ok(res) => res.context("server task failed")?.context("server error")?,
        Err(_) => tracing::warn!("graceful shutdown timed out; exiting"),
    }
    tracing::info!("bye");
    Ok(())
}
