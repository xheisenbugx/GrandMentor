//! Phone & home use (docs/PHONE.md): HTTPS on the home network with a local CA, an access PIN
//! for every device that is not this computer, and the "Use on your phone" settings API.
//!
//! * [`tls`] — local CA + server certificate (rcgen), rustls config.
//! * [`access`] — PIN, signed-in devices, failed-attempt limiter.
//! * [`gate`] — middleware in front of the whole app: loopback passes, everyone else needs the
//!   access cookie (or is sent to `/login`); plain HTTP from the LAN is redirected to HTTPS.
//! * [`routes`] — `/api/phone/*` (loopback only), `/api/access/*`, `/login`, `/phone/ca.crt`.

pub mod access;
pub mod gate;
pub mod login;
pub mod net;
pub mod routes;
pub mod tls;

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use gm_content::Lang;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use access::{AccessStore, Limiter};

/// Default HTTPS port for the home network.
pub const DEFAULT_LAN_PORT: u16 = 8443;
/// Cookie holding a signed-in device's token.
pub const COOKIE: &str = "gm_access";
/// One year.
pub const COOKIE_MAX_AGE: u64 = 365 * 24 * 60 * 60;
/// Public path of the CA certificate download.
pub const CA_PATH: &str = "/phone/ca.crt";
pub const LOGIN_PATH: &str = "/login";
const CONFIG_FILE: &str = "phone.json";
const ACCESS_FILE: &str = "access.json";

/// Which listener a request came through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Http,
    Https,
}

static NETWORK_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Whether this server can be reached from other devices (any listener).
pub fn network_visible() -> bool {
    NETWORK_VISIBLE.load(Ordering::Relaxed)
}

/// Persisted settings (`phone.json`).
#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhoneConfig {
    /// Serve HTTPS on the home network (takes effect after a restart).
    #[serde(default)]
    pub lan: bool,
}

impl PhoneConfig {
    pub fn load(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(CONFIG_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(CONFIG_FILE), bytes)
    }
}

/// `GM_LAN`-style switch: `1/true/on/yes` → `Some(true)`, `0/false/off/no` → `Some(false)`.
pub fn parse_switch(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

/// How the server was started (filled in by `main`).
#[derive(Clone, Debug)]
pub struct PhoneOptions {
    /// Where certificates, the PIN and `phone.json` live. `None` keeps everything in memory.
    pub dir: Option<PathBuf>,
    pub http_port: u16,
    pub https_port: u16,
    /// The HTTPS listener runs in this process.
    pub lan_running: bool,
    /// `GM_LAN` forced the mode on/off.
    pub lan_env: Option<bool>,
    /// The plain HTTP listener accepts connections from other devices.
    pub http_network_visible: bool,
    /// `GM_ACCESS_PIN=off` turns the PIN off (not recommended).
    pub pin_required: bool,
}

impl Default for PhoneOptions {
    fn default() -> Self {
        PhoneOptions {
            dir: None,
            http_port: 8080,
            https_port: DEFAULT_LAN_PORT,
            lan_running: false,
            lan_env: None,
            http_network_visible: false,
            pin_required: true,
        }
    }
}

/// Certificate details shown in Settings.
#[derive(Clone, Debug, Default)]
pub struct TlsInfo {
    pub ca_der: Vec<u8>,
    pub names: Vec<String>,
}

/// Shared phone/access state.
pub struct PhoneState {
    pub opts: PhoneOptions,
    pub hostname: Option<String>,
    access: Mutex<AccessStore>,
    limiter: Mutex<Limiter>,
    tls: RwLock<Option<TlsInfo>>,
}

impl PhoneState {
    pub fn new(opts: PhoneOptions) -> Self {
        let mut store = AccessStore::load(opts.dir.as_ref().map(|d| d.join(ACCESS_FILE)));
        // Other devices can connect: the PIN must survive restarts from now on.
        if opts.lan_running || opts.http_network_visible {
            store.persist();
        }
        NETWORK_VISIBLE.store(opts.lan_running || opts.http_network_visible, Ordering::Relaxed);
        PhoneState {
            hostname: net::hostname(),
            opts,
            access: Mutex::new(store),
            limiter: Mutex::new(Limiter::default()),
            tls: RwLock::new(None),
        }
    }

    /// In-memory state with defaults (tests, `gm_server::app`).
    pub fn in_memory() -> Arc<Self> {
        Arc::new(PhoneState {
            hostname: None,
            opts: PhoneOptions::default(),
            access: Mutex::new(AccessStore::load(None)),
            limiter: Mutex::new(Limiter::default()),
            tls: RwLock::new(None),
        })
    }

    pub fn set_tls(&self, info: TlsInfo) {
        *self.tls.write() = Some(info);
    }

    pub fn tls(&self) -> Option<TlsInfo> {
        self.tls.read().clone()
    }

    /// Names the server certificate should cover right now.
    pub fn server_names(&self) -> tls::ServerNames {
        tls::ServerNames { hostname: self.hostname.clone(), ips: net::lan_ipv4s().into_iter().map(IpAddr::V4).collect() }
    }

    pub fn session_valid(&self, token: &str) -> bool {
        self.access.lock().session_valid(token)
    }

    /// The current PIN. Saved once other devices can connect (it was already saved at startup
    /// then); with phone access off nothing is written, so the default setup leaves no files.
    pub fn pin(&self) -> String {
        let mut a = self.access.lock();
        if self.opts.lan_running || self.opts.http_network_visible {
            a.persist();
        }
        a.pin().to_string()
    }

    pub fn regenerate_pin(&self) -> String {
        self.access.lock().regenerate_pin()
    }

    pub fn device_count(&self) -> usize {
        self.access.lock().sessions().len()
    }

    pub fn sign_out_all(&self) -> usize {
        self.access.lock().clear_sessions()
    }

    pub fn sign_out(&self, token: &str) {
        self.access.lock().remove_session(token);
    }

    /// Check a PIN for `ip`. `Err(Some(secs))` when locked out, `Err(None)` when wrong.
    pub fn try_login(&self, ip: IpAddr, pin: &str, label: &str) -> Result<String, Option<u64>> {
        let now = Instant::now();
        if let Some(wait) = self.limiter.lock().blocked(ip, now) {
            return Err(Some(wait));
        }
        let mut a = self.access.lock();
        if a.pin_matches(pin) {
            self.limiter.lock().record_success(ip);
            Ok(a.add_session(label))
        } else {
            drop(a);
            let mut l = self.limiter.lock();
            l.record_failure(ip, now);
            Err(l.blocked(ip, now))
        }
    }

    pub fn config(&self) -> PhoneConfig {
        self.opts.dir.as_deref().map(PhoneConfig::load).unwrap_or_default()
    }

    /// Addresses other devices can use (empty when none).
    pub fn lan_addresses(&self) -> Vec<Ipv4Addr> {
        net::lan_ipv4s()
    }
}

/// Whether a peer address is this machine.
pub fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback() || ip.to_canonical().is_loopback()
}

/// Pick the message for `lang` from `[en, es, pt, fr, de]`.
pub fn tr(lang: Lang, s: [&str; 5]) -> String {
    match lang {
        Lang::En => s[0],
        Lang::Es => s[1],
        Lang::Pt => s[2],
        Lang::Fr => s[3],
        Lang::De => s[4],
    }
    .to_string()
}

/// Value of cookie `name` from the request headers.
pub fn cookie<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .take(64)
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches() {
        assert_eq!(parse_switch(" ON "), Some(true));
        assert_eq!(parse_switch("0"), Some(false));
        assert_eq!(parse_switch("maybe"), None);
    }

    #[test]
    fn cookie_parsing() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(axum::http::header::COOKIE, "a=1; gm_access=abc ; b=2".parse().expect("hv"));
        assert_eq!(cookie(&h, COOKIE), Some("abc"));
        assert_eq!(cookie(&h, "zz"), None);
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("127.0.0.1".parse().expect("ip")));
        assert!(is_loopback("::1".parse().expect("ip")));
        assert!(is_loopback("::ffff:127.0.0.1".parse().expect("ip")));
        assert!(!is_loopback("192.168.1.2".parse().expect("ip")));
    }
}
