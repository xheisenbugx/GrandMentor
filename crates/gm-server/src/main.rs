//! `grandmentor` — axum server: REST + WebSocket + static frontend.
//!
//! Env: GM_PORT (8080), GM_HOST (127.0.0.1), GM_DATA_DIR (./data), GM_DB (./grandmentor.db),
//! GM_WEB_DIR (./web), ANTHROPIC_API_KEY (optional), GM_MENTOR_MODEL (claude-opus-5-5),
//! GM_ENGINES (default: available cores - 1, clamped to 2..=8), GM_TT_MB (default 32 per engine),
//! GM_SYNC_ALLOW_ORIGINS (optional comma list restricting device-sync CORS, see routes/backup.rs).
//!
//! Phone & home use (docs/PHONE.md): GM_LAN (1/0, overrides the Settings toggle) serves HTTPS on
//! the home network on GM_LAN_PORT (8443) with a local CA; GM_PHONE_DIR (default
//! `<folder of GM_DB>/grandmentor-phone`) holds the certificates, the access PIN and phone.json;
//! GM_ACCESS_PIN=off disables the PIN for other devices (not recommended).

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use gm_server::phone::{self, PhoneOptions, PhoneState, Transport};
use gm_server::{app_with_phone, AppState};

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

    // ---- Phone & home use ------------------------------------------------------------
    let host_explicit = std::env::var("GM_HOST").is_ok_and(|v| !v.trim().is_empty());
    let phone_dir = std::env::var("GM_PHONE_DIR")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            db_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("grandmentor-phone")
        });
    let lan_env = std::env::var("GM_LAN").ok().and_then(|v| phone::parse_switch(&v));
    let lan_wanted = lan_env.unwrap_or_else(|| phone::PhoneConfig::load(&phone_dir).lan);
    let https_port: u16 = env_num("GM_LAN_PORT", phone::DEFAULT_LAN_PORT);
    let pin_required = std::env::var("GM_ACCESS_PIN").ok().and_then(|v| phone::parse_switch(&v)) != Some(false);
    if !pin_required {
        tracing::warn!("GM_ACCESS_PIN=off: other devices on your network can use GrandMentor without a PIN");
    }

    let mut tls = None;
    if lan_wanted {
        let dir = phone_dir.clone();
        let names = phone::tls::ServerNames {
            hostname: phone::net::hostname(),
            ips: phone::net::lan_ipv4s().into_iter().map(IpAddr::V4).collect(),
        };
        match tokio::task::spawn_blocking(move || phone::tls::ensure(&dir, &names)).await {
            Ok(Ok(bundle)) => match phone::tls::server_config(&bundle) {
                Ok(cfg) => tls = Some((bundle, cfg)),
                Err(e) => tracing::error!("phone access (HTTPS) is off: {e:#}"),
            },
            Ok(Err(e)) => tracing::error!("phone access (HTTPS) is off: could not prepare certificates: {e:#}"),
            Err(e) => tracing::error!("phone access (HTTPS) is off: {e}"),
        }
    }
    // Phone mode: the plain HTTP port also listens on the network (only for the certificate
    // download and device sync; everything else is redirected to HTTPS) unless GM_HOST says otherwise.
    let http_host = if tls.is_some() && !host_explicit { IpAddr::from([0, 0, 0, 0]) } else { host };

    let addr = SocketAddr::new(http_host, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let https_listener = match &tls {
        Some(_) => {
            let https_addr = SocketAddr::new(IpAddr::from([0, 0, 0, 0]), https_port);
            let l = std::net::TcpListener::bind(https_addr)
                .with_context(|| format!("binding {https_addr} for phone access (set GM_LAN_PORT)"))?;
            l.set_nonblocking(true)?;
            Some(l)
        }
        None => None,
    };

    let phone = Arc::new(PhoneState::new(PhoneOptions {
        dir: Some(phone_dir.clone()),
        http_port: port,
        https_port,
        lan_running: tls.is_some(),
        lan_env,
        http_network_visible: !http_host.is_loopback(),
        pin_required,
    }));

    let shown = if http_host.is_unspecified() || http_host.is_loopback() {
        "localhost".to_string()
    } else {
        http_host.to_string()
    };
    tracing::info!("GrandMentor is running → open http://{shown}:{port} (Ctrl-C to stop)");

    let https_handle = axum_server::Handle::new();
    let mut https_task = None;
    if let (Some((bundle, cfg)), Some(l)) = (tls, https_listener) {
        phone.set_tls(phone::TlsInfo { ca_der: bundle.ca_der.clone(), names: bundle.names.clone() });
        let rustls_cfg = axum_server::tls_rustls::RustlsConfig::from_config(cfg);
        let svc = app_with_phone(state.clone(), &web_dir, Arc::clone(&phone), Transport::Https)
            .into_make_service_with_connect_info::<SocketAddr>();
        let server = axum_server::from_tcp_rustls(l, rustls_cfg.clone()).handle(https_handle.clone()).serve(svc);
        https_task = Some(tokio::spawn(server));
        tokio::spawn(refresh_certs(Arc::clone(&phone), phone_dir.clone(), rustls_cfg, state.shutdown.subscribe()));
        for ip in phone::net::lan_ipv4s() {
            tracing::info!("phone access → https://{ip}:{https_port} (PIN and steps: Settings → Use on your phone)");
        }
    } else if !http_host.is_loopback() {
        tracing::info!("other devices can connect over plain HTTP; they need the access PIN from Settings");
    }

    let shutdown = Arc::clone(&state.shutdown);
    let mut shutdown_rx = shutdown.subscribe();
    let server = axum::serve(
        listener,
        app_with_phone(state, &web_dir, phone, Transport::Http).into_make_service_with_connect_info::<SocketAddr>(),
    ).with_graceful_shutdown(async move {
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
    https_handle.graceful_shutdown(Some(Duration::from_secs(5)));
    if let Some(t) = https_task {
        let _ = tokio::time::timeout(Duration::from_secs(6), t).await;
    }
    match tokio::time::timeout(Duration::from_secs(10), server).await {
        Ok(res) => res.context("server task failed")?.context("server error")?,
        Err(_) => tracing::warn!("graceful shutdown timed out; exiting"),
    }
    tracing::info!("bye");
    Ok(())
}

/// Re-issue the HTTPS certificate when this computer gets a new LAN address.
async fn refresh_certs(
    phone: Arc<PhoneState>,
    dir: PathBuf,
    cfg: axum_server::tls_rustls::RustlsConfig,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(phone::tls::REFRESH_EVERY) => {}
            _ = shutdown.wait_for(|v| *v) => return,
        }
        let p = Arc::clone(&phone);
        let d = dir.clone();
        let res = tokio::task::spawn_blocking(move || phone::tls::ensure(&d, &p.server_names())).await;
        match res {
            Ok(Ok(bundle)) if bundle.issued => match phone::tls::server_config(&bundle) {
                Ok(new_cfg) => {
                    cfg.reload_from_config(new_cfg);
                    phone.set_tls(phone::TlsInfo { ca_der: bundle.ca_der, names: bundle.names });
                }
                Err(e) => tracing::warn!("could not reload the HTTPS certificate: {e:#}"),
            },
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!("could not refresh the HTTPS certificate: {e:#}"),
            Err(_) => return,
        }
    }
}
