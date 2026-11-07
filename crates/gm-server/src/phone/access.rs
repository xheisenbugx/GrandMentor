//! Access PIN, signed-in devices and the failed-attempt limiter.
//!
//! * The PIN is 6 digits, stored in `access.json` (owner-only file) next to the certificates.
//! * Signing in gives the device a random 256-bit token (cookie); the server keeps at most
//!   [`MAX_SESSIONS`] tokens. "Sign out all devices" forgets them all.
//! * Wrong PINs are limited **per address** with exponential backoff: [`PER_IP_FREE`] tries, then
//!   every further wrong PIN locks that address for [`BACKOFF_BASE`] × 2ⁿ (30 s, 1 min, 2 min, …,
//!   at most [`BACKOFF_MAX`]). A right PIN clears the address; it is forgotten after
//!   [`FORGET_AFTER`] without a wrong PIN. One address can try roughly 35 PINs a day.
//! * A **global ceiling** of [`GLOBAL_FAILURES`] wrong PINs per [`GLOBAL_WINDOW`] caps guessing
//!   from many addresses at once (reaching it takes about 9 addresses guessing all day). A random
//!   6-digit PIN needs 500 000 guesses on average, so even at the ceiling that is ~1 700 days
//!   (~4½ years; ~9 years to try them all). Devices that are already signed in (valid cookie)
//!   never go through this check, so a lockout never affects them.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rand::RngExt as _;
use serde::{Deserialize, Serialize};

use super::tls::write_private;

pub const PIN_LEN: usize = 6;
pub const MAX_SESSIONS: usize = 32;
/// Wrong PINs an address may send before the backoff starts.
pub const PER_IP_FREE: u32 = 5;
/// First lock after the free tries; doubles with every further wrong PIN.
pub const BACKOFF_BASE: Duration = Duration::from_secs(30);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60 * 60);
/// An address with no wrong PIN for this long starts from scratch.
pub const FORGET_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
pub const GLOBAL_FAILURES: u32 = 300;
pub const GLOBAL_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);
/// Addresses remembered by the limiter.
const MAX_TRACKED: usize = 1024;
const TOKEN_BYTES: usize = 32;

/// Constant-time equality for byte strings (length is not secret).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Digits only (`123 456`, `123-456` → `123456`), bounded.
pub fn normalize_pin(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).take(32).collect()
}

fn new_pin() -> String {
    let mut rng = rand::rng();
    (0..PIN_LEN).map(|_| char::from(b'0' + rng.random_range(0..10u8))).collect()
}

fn new_token() -> String {
    let mut b = [0u8; TOKEN_BYTES];
    rand::rng().fill(&mut b[..]);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub token: String,
    pub created: u64,
    #[serde(default)]
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct AccessFile {
    pin: String,
    #[serde(default)]
    sessions: Vec<Session>,
}

/// PIN + sessions, optionally persisted to a file.
pub struct AccessStore {
    file: Option<PathBuf>,
    data: AccessFile,
    /// Data not written yet (a fresh PIN is only saved once someone may need it).
    dirty: bool,
}

impl AccessStore {
    /// Load `file` if it exists; otherwise start with a fresh PIN (saved on [`Self::persist`]).
    pub fn load(file: Option<PathBuf>) -> Self {
        let loaded = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str::<AccessFile>(&s).ok())
            .filter(|d| d.pin.len() == PIN_LEN && d.pin.bytes().all(|b| b.is_ascii_digit()));
        match loaded {
            Some(mut data) => {
                data.sessions.retain(|s| s.token.len() == TOKEN_BYTES * 2);
                data.sessions.truncate(MAX_SESSIONS);
                AccessStore { file, data, dirty: false }
            }
            None => AccessStore { file, data: AccessFile { pin: new_pin(), sessions: Vec::new() }, dirty: true },
        }
    }

    /// Write pending changes (no-op without a file or when nothing changed).
    pub fn persist(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = &self.file else {
            self.dirty = false;
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(&self.data).map_err(std::io::Error::other).and_then(|b| write_private(path, &b)) {
            Ok(()) => self.dirty = false,
            Err(e) => tracing::warn!("could not save {}: {e}", path.display()),
        }
    }

    pub fn pin(&self) -> &str {
        &self.data.pin
    }

    pub fn pin_matches(&self, attempt: &str) -> bool {
        ct_eq(normalize_pin(attempt).as_bytes(), self.data.pin.as_bytes())
    }

    pub fn regenerate_pin(&mut self) -> String {
        self.data.pin = new_pin();
        self.dirty = true;
        self.persist();
        self.data.pin.clone()
    }

    /// New signed-in device; returns its token. The oldest session is dropped past the cap.
    pub fn add_session(&mut self, label: &str) -> String {
        let token = new_token();
        let label: String = label.chars().filter(|c| !c.is_control()).take(120).collect();
        self.data.sessions.push(Session { token: token.clone(), created: now_unix(), label });
        if self.data.sessions.len() > MAX_SESSIONS {
            let extra = self.data.sessions.len() - MAX_SESSIONS;
            self.data.sessions.drain(..extra);
        }
        self.dirty = true;
        self.persist();
        token
    }

    /// Whether `token` belongs to a signed-in device (checks every session in constant time each).
    pub fn session_valid(&self, token: &str) -> bool {
        if token.len() != TOKEN_BYTES * 2 {
            return false;
        }
        self.data.sessions.iter().fold(false, |found, s| found | ct_eq(s.token.as_bytes(), token.as_bytes()))
    }

    pub fn remove_session(&mut self, token: &str) {
        let before = self.data.sessions.len();
        self.data.sessions.retain(|s| !ct_eq(s.token.as_bytes(), token.as_bytes()));
        if self.data.sessions.len() != before {
            self.dirty = true;
            self.persist();
        }
    }

    pub fn clear_sessions(&mut self) -> usize {
        let n = self.data.sessions.len();
        self.data.sessions.clear();
        self.dirty = true;
        self.persist();
        n
    }

    pub fn sessions(&self) -> &[Session] {
        &self.data.sessions
    }
}

#[derive(Clone, Copy, Debug)]
struct Window {
    start: Instant,
    count: u32,
}

impl Window {
    fn active(&self, now: Instant, len: Duration) -> bool {
        now.saturating_duration_since(self.start) < len
    }
}

/// Wrong PINs of one address.
#[derive(Clone, Copy, Debug)]
struct Strikes {
    count: u32,
    last: Instant,
    until: Option<Instant>,
}

/// How long an address is locked after its `count`-th wrong PIN.
pub fn backoff(count: u32) -> Option<Duration> {
    let extra = count.checked_sub(PER_IP_FREE)?;
    Some(BACKOFF_BASE.saturating_mul(1u32 << extra.min(16)).min(BACKOFF_MAX))
}

/// Failed-PIN limiter: per-address backoff plus a global ceiling.
#[derive(Default)]
pub struct Limiter {
    per_ip: HashMap<IpAddr, Strikes>,
    global: Option<Window>,
}

impl Limiter {
    /// `Some(seconds)` to wait when `ip` may not try right now.
    pub fn blocked(&self, ip: IpAddr, now: Instant) -> Option<u64> {
        let secs = |d: Duration| d.as_secs().max(1);
        if let Some(g) = self.global.filter(|g| g.active(now, GLOBAL_WINDOW) && g.count >= GLOBAL_FAILURES) {
            return Some(secs(GLOBAL_WINDOW.saturating_sub(now.saturating_duration_since(g.start))));
        }
        let s = self.per_ip.get(&ip)?;
        s.until.filter(|u| *u > now).map(|u| secs(u.saturating_duration_since(now)))
    }

    pub fn record_failure(&mut self, ip: IpAddr, now: Instant) {
        let g = self.global.get_or_insert(Window { start: now, count: 0 });
        if !g.active(now, GLOBAL_WINDOW) {
            *g = Window { start: now, count: 0 };
        }
        g.count += 1;
        let stale = |s: &Strikes| now.saturating_duration_since(s.last) >= FORGET_AFTER;
        if self.per_ip.len() >= MAX_TRACKED && !self.per_ip.contains_key(&ip) {
            self.per_ip.retain(|_, s| !stale(s));
            if self.per_ip.len() >= MAX_TRACKED {
                if let Some(oldest) = self.per_ip.iter().min_by_key(|(_, s)| s.last).map(|(k, _)| *k) {
                    self.per_ip.remove(&oldest);
                }
            }
        }
        let s = self.per_ip.entry(ip).or_insert(Strikes { count: 0, last: now, until: None });
        if stale(s) {
            *s = Strikes { count: 0, last: now, until: None };
        }
        s.count = s.count.saturating_add(1);
        s.last = now;
        s.until = backoff(s.count).map(|d| now + d);
    }

    pub fn record_success(&mut self, ip: IpAddr) {
        self.per_ip.remove(&ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_and_sessions() {
        let mut s = AccessStore::load(None);
        assert_eq!(s.pin().len(), PIN_LEN);
        let pin = s.pin().to_string();
        let spaced = format!("{} {}", &pin[..3], &pin[3..]);
        assert!(s.pin_matches(&spaced));
        assert!(!s.pin_matches("12345"));
        let t = s.add_session("Phone");
        assert!(s.session_valid(&t));
        assert!(!s.session_valid(&"0".repeat(64)));
        assert!(!s.session_valid("short"));
        assert_eq!(s.clear_sessions(), 1);
        assert!(!s.session_valid(&t));
        for i in 0..(MAX_SESSIONS + 5) {
            s.add_session(&format!("d{i}"));
        }
        assert_eq!(s.sessions().len(), MAX_SESSIONS);
    }

    #[test]
    fn persists_to_file() {
        let dir = std::env::temp_dir().join(format!("gm-access-{}-{}", std::process::id(), now_unix()));
        let file = dir.join("access.json");
        let mut s = AccessStore::load(Some(file.clone()));
        s.persist();
        let t = s.add_session("x");
        let pin = s.pin().to_string();
        let s2 = AccessStore::load(Some(file));
        assert_eq!(s2.pin(), pin);
        assert!(s2.session_valid(&t));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn limiter_backs_off_per_ip() {
        let mut l = Limiter::default();
        let a: IpAddr = "192.168.1.5".parse().expect("ip");
        let b: IpAddr = "192.168.1.6".parse().expect("ip");
        let t0 = Instant::now();
        for _ in 0..PER_IP_FREE - 1 {
            l.record_failure(a, t0);
            assert!(l.blocked(a, t0).is_none());
        }
        l.record_failure(a, t0);
        assert_eq!(l.blocked(a, t0), Some(30));
        // Other addresses are not affected.
        assert!(l.blocked(b, t0).is_none());
        // The lock passes, the next wrong PIN doubles it.
        let t1 = t0 + Duration::from_secs(31);
        assert!(l.blocked(a, t1).is_none());
        l.record_failure(a, t1);
        assert_eq!(l.blocked(a, t1), Some(60));
        assert_eq!(backoff(PER_IP_FREE + 2), Some(Duration::from_secs(120)));
        assert_eq!(backoff(PER_IP_FREE + 40), Some(BACKOFF_MAX));
        assert_eq!(backoff(PER_IP_FREE - 1), None);
        // A day later the address starts from scratch.
        let t2 = t1 + FORGET_AFTER;
        l.record_failure(a, t2);
        assert!(l.blocked(a, t2).is_none());
        // A right PIN clears it.
        l.record_success(a);
        assert!(l.blocked(a, t2).is_none());
    }

    #[test]
    fn limiter_global_ceiling() {
        let t0 = Instant::now();
        let b: IpAddr = "192.168.1.6".parse().expect("ip");
        let mut g = Limiter::default();
        for i in 0..GLOBAL_FAILURES {
            let ip: IpAddr = format!("10.0.{}.{}", i / 200, i % 200 + 1).parse().expect("ip");
            assert!(g.blocked(b, t0).is_none());
            g.record_failure(ip, t0);
        }
        assert!(g.blocked(b, t0).is_some());
        assert!(g.blocked(b, t0 + GLOBAL_WINDOW).is_none());
    }

    #[test]
    fn ct_eq_works() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
