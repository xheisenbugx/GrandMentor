//! This computer's LAN addresses and host name.

use std::net::{IpAddr, Ipv4Addr};

/// Max addresses reported.
const MAX_ADDRS: usize = 8;

/// Ranking: home Wi-Fi style ranges first, then other private ranges, then VPN (CGNAT) ranges.
fn rank(ip: Ipv4Addr) -> u8 {
    let o = ip.octets();
    match o {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        [172, b, ..] if (16..32).contains(&b) => 2,
        [100, b, ..] if (64..128).contains(&b) => 3,
        _ => 4,
    }
}

/// Whether an address can be reached by other devices on a home network.
pub fn usable(ip: Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() || ip.is_documentation())
}

/// Order + de-duplicate a list of candidate addresses.
pub fn sort_addrs(mut ips: Vec<Ipv4Addr>) -> Vec<Ipv4Addr> {
    ips.retain(|ip| usable(*ip));
    ips.sort_by_key(|ip| (rank(*ip), u32::from(*ip)));
    ips.dedup();
    ips.truncate(MAX_ADDRS);
    ips
}

/// Non-loopback IPv4 addresses of this machine, best first.
pub fn lan_ipv4s() -> Vec<Ipv4Addr> {
    let ifaces = if_addrs::get_if_addrs().unwrap_or_default();
    sort_addrs(
        ifaces
            .into_iter()
            .filter_map(|i| match i.ip() {
                IpAddr::V4(v4) => Some(v4),
                IpAddr::V6(_) => None,
            })
            .collect(),
    )
}

/// Turn a raw host name into a DNS label: `Osvaldo's MacBook.local` → `osvaldos-macbook`.
pub fn sanitize_hostname(raw: &str) -> Option<String> {
    let first = raw.trim().trim_end_matches('.').trim_end_matches(".local").split('.').next().unwrap_or("");
    let mut out = String::new();
    for c in first.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if (c == '-' || c == ' ' || c == '_') && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 63 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    (!out.is_empty() && out != "localhost").then_some(out)
}

/// This computer's host name as a DNS label, if it has a usable one.
pub fn hostname() -> Option<String> {
    sanitize_hostname(&gethostname::gethostname().to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostnames() {
        assert_eq!(sanitize_hostname("Osvaldos-MacBook-Pro.local").as_deref(), Some("osvaldos-macbook-pro"));
        assert_eq!(sanitize_hostname("chess box_1").as_deref(), Some("chess-box-1"));
        assert_eq!(sanitize_hostname("localhost"), None);
        assert_eq!(sanitize_hostname("---"), None);
    }

    #[test]
    fn address_order() {
        let ips: Vec<Ipv4Addr> = ["100.70.1.2", "127.0.0.1", "10.0.0.5", "192.168.1.20", "169.254.3.3", "192.168.1.20"]
            .iter()
            .map(|s| s.parse().expect("ip"))
            .collect();
        let got: Vec<String> = sort_addrs(ips).iter().map(ToString::to_string).collect();
        assert_eq!(got, ["192.168.1.20", "10.0.0.5", "100.70.1.2"]);
    }
}
