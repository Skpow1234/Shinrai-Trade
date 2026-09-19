//! Ops IP allowlist helpers (exact IPs and IPv4 CIDR prefixes).

use std::net::{IpAddr, Ipv4Addr};

/// Parsed allowlist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowEntry {
    /// Any address.
    Any,
    /// Exact IP.
    Exact(IpAddr),
    /// IPv4 network (`addr`/`prefix`).
    V4Net {
        /// Network address (host bits zeroed).
        network: u32,
        /// Prefix length 0..=32.
        prefix: u8,
    },
}

/// Parses `SHINRAI_OG_OPS_ALLOWLIST` (`*` / `ip` / `a.b.c.d/n`, comma-separated).
#[must_use]
pub fn parse_allowlist(raw: Option<&str>) -> Vec<AllowEntry> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Vec::new();
    };
    raw.split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            if part == "*" {
                return Some(AllowEntry::Any);
            }
            if let Some((addr, pref)) = part.split_once('/') {
                let ip: Ipv4Addr = addr.trim().parse().ok()?;
                let prefix: u8 = pref.trim().parse().ok()?;
                if prefix > 32 {
                    return None;
                }
                let mask = if prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                let network = u32::from(ip) & mask;
                return Some(AllowEntry::V4Net { network, prefix });
            }
            let ip: IpAddr = part.parse().ok()?;
            Some(AllowEntry::Exact(ip))
        })
        .collect()
}

/// Returns true when allowlist is empty (open) or `ip` matches an entry.
#[must_use]
pub fn ip_allowed(ip: IpAddr, allowlist: &[AllowEntry]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    allowlist.iter().any(|e| match e {
        AllowEntry::Any => true,
        AllowEntry::Exact(a) => *a == ip,
        AllowEntry::V4Net { network, prefix } => match ip {
            IpAddr::V4(v4) => {
                let mask = if *prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                (u32::from(v4) & mask) == *network
            }
            IpAddr::V6(_) => false,
        },
    })
}

/// Client IP from `X-Forwarded-For` (leftmost), else `X-Real-IP`, else peer.
#[must_use]
pub fn client_ip(forwarded_for: Option<&str>, peer: Option<IpAddr>) -> Option<IpAddr> {
    if let Some(xff) = forwarded_for {
        if let Some(first) = xff.split(',').next() {
            if let Ok(ip) = first.trim().parse::<IpAddr>() {
                return Some(ip);
            }
        }
    }
    peer
}

/// Resolves client IP from ops request headers (`X-Forwarded-For` or `X-Real-IP`).
#[must_use]
pub fn client_ip_from_headers(headers: &axum::http::HeaderMap) -> Option<IpAddr> {
    let xff = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok());
    if let Some(ip) = client_ip(xff, None) {
        return Some(ip);
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_and_exact() {
        let list = parse_allowlist(Some("127.0.0.1,10.0.0.0/8"));
        assert!(ip_allowed(IpAddr::V4(Ipv4Addr::LOCALHOST), &list));
        assert!(ip_allowed(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)), &list));
        assert!(!ip_allowed(
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            &list
        ));
    }
}
