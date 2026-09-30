//! Trusted client address extraction.
//!
//! With `API_TRUST_PROXY=1` the deployment sits behind exactly one reverse
//! proxy. The proxy *appends* the peer address to `X-Forwarded-For`
//! (`$proxy_add_x_forwarded_for`), so the **rightmost** entry is the address
//! the proxy observed; the leftmost entry is whatever the client sent and is
//! trivially rotated to land in a fresh rate-limit or lockout bucket.
//!
//! Parsing as [`IpAddr`] also stops garbage strings from becoming bucket keys.

use std::net::IpAddr;
use std::sync::OnceLock;

use axum::http::HeaderMap;

fn trust_proxy() -> bool {
    static TRUST: OnceLock<bool> = OnceLock::new();
    *TRUST.get_or_init(|| {
        std::env::var("API_TRUST_PROXY")
            .map(|value| value == "1")
            .unwrap_or(false)
    })
}

/// Resolve the client address for rate limiting and lockout bucketing.
///
/// Under `API_TRUST_PROXY=1` the rightmost `X-Forwarded-For` entry wins when it
/// parses as an IP address; otherwise the TCP peer address is used.
pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>) -> Option<IpAddr> {
    client_ip_trusting(trust_proxy(), headers, peer)
}

/// Bucket identity for one client address.
///
/// IPv6 clients are bucketed by /64: a single subscriber is routinely assigned
/// a whole /64 (or larger), so per-address buckets let them rotate through the
/// prefix to bypass rate limits and lockouts.
pub fn rate_limit_identity(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            format!(
                "{:x}:{:x}:{:x}:{:x}::/64",
                segments[0], segments[1], segments[2], segments[3]
            )
        }
    }
}

/// Pure decision function (testable without mutating the process environment).
fn client_ip_trusting(
    trust_forwarded: bool,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Option<IpAddr> {
    if trust_forwarded {
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit(',').next())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
        {
            return Some(ip);
        }
    }
    peer
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn rightmost_forwarded_entry_wins_over_client_supplied_leftmost() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("1.2.3.4, 10.0.0.9"),
        );
        let peer = Some("10.0.0.9".parse::<IpAddr>().unwrap());
        assert_eq!(
            client_ip_trusting(true, &headers, peer),
            Some("10.0.0.9".parse::<IpAddr>().unwrap()),
            "the proxy-appended rightmost entry is the real client, not the spoofed leftmost"
        );

        // Without the trust switch the header is ignored entirely.
        assert_eq!(client_ip_trusting(false, &headers, peer), peer);

        // Garbage entries do not become bucket keys.
        let mut garbage = HeaderMap::new();
        garbage.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        assert_eq!(client_ip_trusting(true, &garbage, peer), peer);
    }
}
