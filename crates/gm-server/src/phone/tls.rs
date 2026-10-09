//! Local certificate authority + server certificate for HTTPS on the home network.
//!
//! Files (in the phone directory, see [`super::PhoneConfig`]):
//! * `ca.key.pem` / `ca.crt.pem` — long-lived local CA (10 years). The CA is *name-constrained*:
//!   it can only vouch for private / local addresses (10/8, 172.16/12, 192.168/16, 100.64/10,
//!   169.254/16, 127/8, `*.local`, `localhost` and this computer's host name), so even if its key
//!   leaked it could not impersonate real websites on the phone that trusts it.
//! * `ca.json` — the host name the CA was created for (its DNS constraint).
//! * `server.key.pem` / `server.crt.pem` / `server.json` — leaf certificate (397 days) for the
//!   LAN addresses, host name, `<host>.local` and `localhost`. It is re-issued whenever an
//!   address is missing, it is about to expire, or the CA changed.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context as _};
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree,
    IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};

const CA_KEY: &str = "ca.key.pem";
const CA_CERT: &str = "ca.crt.pem";
const CA_META: &str = "ca.json";
const LEAF_KEY: &str = "server.key.pem";
const LEAF_CERT: &str = "server.crt.pem";
const LEAF_META: &str = "server.json";

const DAY: u64 = 24 * 60 * 60;
const CA_DAYS: u64 = 3650;
/// Browsers cap leaf lifetimes at 398 days.
const LEAF_DAYS: u64 = 397;
/// Re-issue the leaf when it has less than this left.
const LEAF_RENEW_BEFORE_DAYS: u64 = 30;
/// At most this many addresses go into the certificate.
const MAX_IPS: usize = 16;

/// Private ranges the CA may sign for (IPv4 CIDR).
const PERMITTED_V4: &[([u8; 4], u8)] = &[
    ([10, 0, 0, 0], 8),
    ([172, 16, 0, 0], 12),
    ([192, 168, 0, 0], 16),
    ([100, 64, 0, 0], 10),
    ([169, 254, 0, 0], 16),
    ([127, 0, 0, 0], 8),
];

/// Names the server certificate should cover.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerNames {
    /// Short host name (a valid DNS label, lower case), if known.
    pub hostname: Option<String>,
    /// LAN addresses (loopback is always added).
    pub ips: Vec<IpAddr>,
}

impl ServerNames {
    /// All names as strings (DNS names first, then addresses), sorted and de-duplicated.
    fn wanted(&self, ca_host: Option<&str>) -> Vec<String> {
        let mut out = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        if let Some(h) = &self.hostname {
            out.push(format!("{h}.local"));
            // A bare host name is only allowed by the CA's constraints if it is the one it was made for.
            if ca_host == Some(h.as_str()) {
                out.push(h.clone());
            }
        }
        for ip in self.ips.iter().take(MAX_IPS) {
            if permitted_ip(*ip) {
                out.push(ip.to_string());
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Whether the CA's constraints allow this address.
pub fn permitted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => PERMITTED_V4.iter().any(|(net, bits)| in_v4(v4, Ipv4Addr::from(*net), *bits)),
        IpAddr::V6(v6) => v6 == Ipv6Addr::LOCALHOST,
    }
}

fn in_v4(ip: Ipv4Addr, net: Ipv4Addr, bits: u8) -> bool {
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - u32::from(bits)) };
    (u32::from(ip) & mask) == (u32::from(net) & mask)
}

/// What the HTTPS listener needs, plus the CA for downloads.
#[derive(Clone, Debug)]
pub struct CertBundle {
    /// Leaf + CA, DER.
    pub chain: Vec<Vec<u8>>,
    /// Leaf private key, PKCS#8 DER.
    pub key: Vec<u8>,
    pub ca_der: Vec<u8>,
    /// Names in the leaf certificate.
    pub names: Vec<String>,
    /// Whether the leaf was (re)issued by this call.
    pub issued: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct CaMeta {
    hostname: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct LeafMeta {
    names: Vec<String>,
    not_after: u64,
    /// SHA-256 of the CA certificate that signed it.
    ca: String,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn to_time(unix: u64) -> anyhow::Result<time::OffsetDateTime> {
    let secs = i64::try_from(unix).context("time out of range")?;
    time::OffsetDateTime::from_unix_timestamp(secs).context("time out of range")
}

/// SHA-256 as upper-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, bytes);
    d.as_ref().iter().map(|b| format!("{b:02X}")).collect()
}

/// `AB:CD:...` fingerprint shown to users.
pub fn fingerprint(der: &[u8]) -> String {
    let hex = sha256_hex(der);
    hex.as_bytes().chunks(2).filter_map(|c| std::str::from_utf8(c).ok()).collect::<Vec<_>>().join(":")
}

/// Write a file readable only by the owner (keys, PIN, sessions).
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write as _;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn ca_params(hostname: Option<&str>, now: u64) -> anyhow::Result<CertificateParams> {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "GrandMentor local CA");
    dn.push(DnType::OrganizationName, "GrandMentor");
    if let Some(h) = hostname {
        dn.push(DnType::OrganizationalUnitName, h.to_string());
    }
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    p.not_before = to_time(now.saturating_sub(DAY))?;
    p.not_after = to_time(now + CA_DAYS * DAY)?;
    let mut permitted: Vec<GeneralSubtree> = vec![
        GeneralSubtree::DnsName("local".into()),
        GeneralSubtree::DnsName("localhost".into()),
    ];
    if let Some(h) = hostname {
        permitted.push(GeneralSubtree::DnsName(h.to_string()));
    }
    permitted.extend(PERMITTED_V4.iter().map(|(net, bits)| GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix(*net, *bits))));
    permitted.push(GeneralSubtree::IpAddress(CidrSubnet::from_v6_prefix(Ipv6Addr::LOCALHOST.octets(), 128)));
    p.name_constraints = Some(NameConstraints { permitted_subtrees: permitted, excluded_subtrees: Vec::new() });
    Ok(p)
}

fn leaf_params(names: &[String], hostname: Option<&str>, now: u64) -> anyhow::Result<CertificateParams> {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, format!("GrandMentor ({})", hostname.unwrap_or("this computer")));
    p.distinguished_name = dn;
    p.subject_alt_names = names
        .iter()
        .map(|n| match n.parse::<IpAddr>() {
            Ok(ip) => Ok(SanType::IpAddress(ip)),
            Err(_) => Ok(SanType::DnsName(n.clone().try_into().map_err(|e| anyhow!("bad DNS name {n}: {e}"))?)),
        })
        .collect::<anyhow::Result<_>>()?;
    p.is_ca = IsCa::ExplicitNoCa;
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.use_authority_key_identifier_extension = true;
    p.not_before = to_time(now.saturating_sub(DAY))?;
    p.not_after = to_time(now + LEAF_DAYS * DAY)?;
    let mut serial = [0u8; 16];
    rand::Rng::fill(&mut rand::thread_rng(), &mut serial[..]);
    serial[0] &= 0x7f;
    p.serial_number = Some(serial.to_vec().into());
    Ok(p)
}

/// Load (or create) the CA. Returns `(ca_cert_pem, ca_key, ca_hostname)`.
fn ensure_ca(dir: &Path, hostname: Option<&str>) -> anyhow::Result<(String, KeyPair, Option<String>)> {
    if let (Some(cert), Some(key)) = (read(&dir.join(CA_CERT)), read(&dir.join(CA_KEY))) {
        if let Ok(kp) = KeyPair::from_pem(&key) {
            let meta: CaMeta = read(&dir.join(CA_META)).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            if Issuer::from_ca_cert_pem(&cert, &kp).is_ok() {
                return Ok((cert, kp, meta.hostname));
            }
        }
        tracing::warn!("the local CA in {} is unreadable; creating a new one", dir.display());
    }
    let kp = KeyPair::generate().context("generating CA key")?;
    let cert = ca_params(hostname, now_unix())?.self_signed(&kp).context("signing CA")?;
    let pem = cert.pem();
    write_private(&dir.join(CA_KEY), kp.serialize_pem().as_bytes())?;
    std::fs::write(dir.join(CA_CERT), &pem)?;
    let meta = CaMeta { hostname: hostname.map(str::to_string) };
    std::fs::write(dir.join(CA_META), serde_json::to_vec_pretty(&meta)?)?;
    tracing::info!("created a local certificate authority in {}", dir.display());
    Ok((pem, kp, meta.hostname))
}

fn pem_to_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    Ok(CertificateDer::from_pem_slice(pem.as_bytes()).map_err(|e| anyhow!("bad certificate PEM: {e}"))?.to_vec())
}

/// Make sure the CA and a server certificate covering `names` exist in `dir` (blocking I/O).
pub fn ensure(dir: &Path, names: &ServerNames) -> anyhow::Result<CertBundle> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let (ca_pem, ca_key, ca_host) = ensure_ca(dir, names.hostname.as_deref())?;
    let ca_der = pem_to_der(&ca_pem)?;
    let ca_sum = sha256_hex(&ca_der);
    let wanted = names.wanted(ca_host.as_deref());
    let now = now_unix();

    // Reuse the current leaf if it is still good.
    let meta: Option<LeafMeta> = read(&dir.join(LEAF_META)).and_then(|s| serde_json::from_str(&s).ok());
    if let (Some(meta), Some(cert), Some(key)) = (meta, read(&dir.join(LEAF_CERT)), read(&dir.join(LEAF_KEY))) {
        let fresh = meta.ca == ca_sum
            && meta.not_after > now + LEAF_RENEW_BEFORE_DAYS * DAY
            && wanted.iter().all(|n| meta.names.contains(n));
        if fresh {
            if let (Ok(leaf), Ok(k)) = (pem_to_der(&cert), PrivateKeyDer::from_pem_slice(key.as_bytes())) {
                return Ok(CertBundle {
                    chain: vec![leaf, ca_der.clone()],
                    key: k.secret_der().to_vec(),
                    ca_der,
                    names: meta.names,
                    issued: false,
                });
            }
        }
    }

    let issuer = Issuer::from_ca_cert_pem(&ca_pem, &ca_key).context("reading CA")?;
    let leaf_key = KeyPair::generate().context("generating server key")?;
    let params = leaf_params(&wanted, names.hostname.as_deref(), now)?;
    let leaf = params.signed_by(&leaf_key, &issuer).context("signing server certificate")?;
    write_private(&dir.join(LEAF_KEY), leaf_key.serialize_pem().as_bytes())?;
    std::fs::write(dir.join(LEAF_CERT), leaf.pem())?;
    let meta = LeafMeta { names: wanted.clone(), not_after: now + LEAF_DAYS * DAY, ca: ca_sum };
    std::fs::write(dir.join(LEAF_META), serde_json::to_vec_pretty(&meta)?)?;
    tracing::info!(names = ?wanted, "issued a server certificate for the home network");
    Ok(CertBundle {
        chain: vec![leaf.der().to_vec(), ca_der.clone()],
        key: leaf_key.serialize_der(),
        ca_der,
        names: wanted,
        issued: true,
    })
}

/// The CA certificate (DER), if one was created.
pub fn load_ca_der(dir: &Path) -> Option<Vec<u8>> {
    read(&dir.join(CA_CERT)).and_then(|p| pem_to_der(&p).ok())
}

/// rustls server config for a bundle (ring provider, HTTP/1.1 only so websockets keep working).
pub fn server_config(bundle: &CertBundle) -> anyhow::Result<Arc<rustls::ServerConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let chain: Vec<CertificateDer<'static>> = bundle.chain.iter().map(|c| CertificateDer::from(c.clone())).collect();
    let key = PrivateKeyDer::try_from(bundle.key.clone()).map_err(|e| anyhow!("bad server key: {e}"))?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("TLS versions")?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .context("TLS certificate")?;
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(cfg))
}

/// How often the LAN addresses are re-checked (the leaf is re-issued when one is new).
pub const REFRESH_EVERY: Duration = Duration::from_secs(120);

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gm-phone-tls-{tag}-{}-{}", std::process::id(), now_unix()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn sans(der: &[u8]) -> Vec<String> {
        let (_, cert) = x509_parser::parse_x509_certificate(der).expect("parse");
        let mut out = Vec::new();
        if let Ok(Some(ext)) = cert.subject_alternative_name() {
            for n in &ext.value.general_names {
                match n {
                    x509_parser::extensions::GeneralName::DNSName(d) => out.push(d.to_string()),
                    x509_parser::extensions::GeneralName::IPAddress(b) if b.len() == 4 => {
                        out.push(Ipv4Addr::new(b[0], b[1], b[2], b[3]).to_string());
                    }
                    _ => {}
                }
            }
        }
        out.sort();
        out
    }

    fn names(ips: &[&str]) -> ServerNames {
        ServerNames { hostname: Some("chessbox".into()), ips: ips.iter().map(|s| s.parse().expect("ip")).collect() }
    }

    #[test]
    fn leaf_has_expected_sans_and_is_reused() {
        let dir = tmpdir("sans");
        let b = ensure(&dir, &names(&["192.168.1.20", "10.0.0.7", "8.8.8.8"])).expect("certs");
        assert!(b.issued);
        let got = sans(&b.chain[0]);
        for want in ["localhost", "127.0.0.1", "chessbox", "chessbox.local", "192.168.1.20", "10.0.0.7"] {
            assert!(got.contains(&want.to_string()), "missing {want} in {got:?}");
        }
        // Public addresses are outside the CA's constraints and are left out.
        assert!(!got.contains(&"8.8.8.8".to_string()));

        // Same names: reused. CA stays the same.
        let again = ensure(&dir, &names(&["192.168.1.20"])).expect("certs");
        assert!(!again.issued);
        assert_eq!(again.ca_der, b.ca_der);

        // A new address: re-issued with the same CA.
        let moved = ensure(&dir, &names(&["192.168.50.3"])).expect("certs");
        assert!(moved.issued);
        assert_eq!(moved.ca_der, b.ca_der);
        assert!(sans(&moved.chain[0]).contains(&"192.168.50.3".to_string()));

        let (_, ca) = x509_parser::parse_x509_certificate(&b.ca_der).expect("ca");
        assert!(ca.is_ca());
        assert!(ca.name_constraints().ok().flatten().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn keys_are_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tmpdir("perm");
        ensure(&dir, &names(&["192.168.1.20"])).expect("certs");
        for f in [CA_KEY, LEAF_KEY] {
            let mode = std::fs::metadata(dir.join(f)).expect("meta").permissions().mode();
            assert_eq!(mode & 0o077, 0, "{f} must not be readable by others");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Full TLS handshake against the CA as the only trusted root (webpki enforces the name
    /// constraints and SANs, like Chrome does).
    #[tokio::test]
    async fn handshake_verifies_with_ca() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let dir = tmpdir("hs");
        let b = ensure(&dir, &names(&["192.168.1.20"])).expect("certs");
        let server_cfg = server_config(&b).expect("server cfg");
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(b.ca_der.clone())).expect("root");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let client_cfg = Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("versions")
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        for (server_name, ok) in [("192.168.1.20", true), ("chessbox.local", true), ("chessbox", true), ("192.168.1.99", false), ("example.com", false)] {
            let (c, s) = tokio::io::duplex(64 * 1024);
            let acceptor = tokio_rustls::TlsAcceptor::from(server_cfg.clone());
            let server = tokio::spawn(async move {
                if let Ok(mut tls) = acceptor.accept(s).await {
                    let mut buf = [0u8; 4];
                    if tls.read_exact(&mut buf).await.is_ok() {
                        let _ = tls.write_all(b"pong").await;
                        let _ = tls.flush().await;
                    }
                }
            });
            let connector = tokio_rustls::TlsConnector::from(client_cfg.clone());
            let name = rustls::pki_types::ServerName::try_from(server_name.to_string()).expect("name");
            let res = connector.connect(name, c).await;
            assert_eq!(res.is_ok(), ok, "{server_name}: {:?}", res.as_ref().err());
            if let Ok(mut tls) = res {
                tls.write_all(b"ping").await.expect("write");
                let mut buf = [0u8; 4];
                tls.read_exact(&mut buf).await.expect("read");
                assert_eq!(&buf, b"pong");
            }
            drop(server);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn permitted_ranges() {
        assert!(permitted_ip("192.168.0.1".parse().expect("ip")));
        assert!(permitted_ip("172.31.255.1".parse().expect("ip")));
        assert!(!permitted_ip("172.32.0.1".parse().expect("ip")));
        assert!(permitted_ip("100.101.1.2".parse().expect("ip")));
        assert!(!permitted_ip("1.1.1.1".parse().expect("ip")));
    }
}
