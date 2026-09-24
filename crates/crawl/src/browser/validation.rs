//! URL hardening for the browser renderer.
//!
//! Discovered URLs are attacker-influenced, so every URL handed to Chromium is
//! validated first:
//!
//! - only `http`/`https` schemes are accepted (`file:`, `data:`,
//!   `javascript:`, `chrome:`, `about:`, `ftp:` … are rejected),
//! - the host must not be loopback, private, link-local, CGNAT, unique-local,
//!   unspecified, broadcast or `localhost`,
//! - the host is re-resolved immediately before navigation and rejected if it
//!   now points at a private address (DNS-rebinding mitigation),
//! - URLs are canonicalised through [`url::Url`] so the browser receives a
//!   normalised string rather than raw attacker input.
//!
//! Shell-metacharacter rejection is kept as defence in depth even though the
//! URL is passed as a process argument (never through a shell).

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{anyhow, Result};
use url::Url;

/// Maximum accepted URL length.
pub const MAX_URL_LENGTH: usize = 8192;

/// Characters that could matter if a URL ever reaches a shell or a
/// command-line parser that expects no arguments.
const DANGEROUS_CHARS: [char; 12] = [
    '|', ';', '&', '$', '`', '\n', '\r', '>', '<', '\\', '\'', '"',
];

/// Validate and canonicalise a URL for the browser renderer.
///
/// Returns the parsed, normalised [`Url`] (never the raw input) so callers
/// navigate to exactly what was validated.
pub fn validate_browser_url(url: &str) -> Result<Url> {
    if url.len() > MAX_URL_LENGTH {
        return Err(anyhow!(
            "URL exceeds maximum allowed length ({MAX_URL_LENGTH}): {:.50}",
            url
        ));
    }
    if let Some(bad) = url.chars().find(|c| DANGEROUS_CHARS.contains(c)) {
        return Err(anyhow!(
            "URL contains dangerous character {bad:?} which may enable command injection: {:.50}",
            url
        ));
    }

    let parsed = Url::parse(url).map_err(|error| anyhow!("invalid URL {:.80}: {error}", url))?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(anyhow!(
                "URL must use the http:// or https:// scheme, got {other:?}: {:.50}",
                url
            ));
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("URL has no host: {:.80}", url))?;
    if is_private_host(host) {
        return Err(anyhow!(
            "refusing to browse private/loopback/metadata host {host}: {:.50}",
            url
        ));
    }

    Ok(parsed)
}

/// Lowercased host of an http(s) URL, parsed via [`url::Url`].
pub fn host_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    Some(match parsed.host()? {
        url::Host::Domain(domain) => domain.trim_end_matches('.').to_ascii_lowercase(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(ip) => ip.to_string(),
    })
}

fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        // 0.0.0.0/8 "this network"
        || ip.octets()[0] == 0
        // 100.64.0.0/10 carrier-grade NAT
        || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
}

fn is_private_v6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_multicast()
        // IPv4-mapped (`::ffff:a.b.c.d`) and IPv4-compatible (`::a.b.c.d`)
        || ip.to_ipv4().is_some_and(is_private_v4)
}

/// True for loopback/private/link-local/CGNAT/unique-local/metadata addresses
/// and `localhost` names, including IPv4-mapped IPv6 forms.
pub fn is_private_host(host: &str) -> bool {
    // Trailing-dot FQDNs (`localhost.`) and zone-scoped IPv6 literals
    // (`fe80::1%eth0`) are normalised before classification.
    let host = host.split('%').next().unwrap_or(host);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = host.trim_end_matches('.');
    if host.is_empty() {
        return true;
    }
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => is_private_v4(ip),
        Ok(IpAddr::V6(ip)) => is_private_v6(ip),
        Err(_) => false,
    }
}

/// Re-resolve the host immediately before navigating and reject any private
/// address. Resolution failures are left to the browser (the page simply
/// fails to load).
pub async fn assert_public_resolution(url: &Url) -> Result<()> {
    assert_public_resolution_with(url, |host, port| async move {
        tokio::net::lookup_host((host.as_str(), port))
            .await
            .map(|addresses| addresses.map(|address| address.ip()).collect::<Vec<_>>())
            .map_err(|error| error.to_string())
    })
    .await
}

/// DNS-rebinding check with an injectable resolver so the logic is testable
/// without real DNS.
pub async fn assert_public_resolution_with<F, Fut>(url: &Url, resolve: F) -> Result<()>
where
    F: FnOnce(String, u16) -> Fut,
    Fut: Future<Output = Result<Vec<IpAddr>, String>>,
{
    let Some(host) = url.host_str() else {
        return Ok(());
    };
    if is_private_host(host) {
        return Err(anyhow!("refusing to browse private host {host}"));
    }
    // Literal IPs were already classified above.
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }

    let port = url.port_or_known_default().unwrap_or(443);
    match resolve(host.to_string(), port).await {
        Ok(addresses) => {
            for ip in addresses {
                if is_private_host(&ip.to_string()) {
                    return Err(anyhow!(
                        "host {host} resolves to private address {ip} — refusing (DNS rebinding)"
                    ));
                }
            }
            Ok(())
        }
        // Resolution failure is handled by the browser itself.
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn scheme(parsed: &Url) -> String {
        parsed.scheme().to_string()
    }

    #[test]
    fn ssrf_rejection_table() {
        let rejected = [
            "http://127.0.0.1/",
            "http://127.0.0.1:8080/admin",
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://192.168.1.1/",
            "http://[::1]/",
            "http://[fc00::1]/",
            "http://169.254.169.254/latest/meta-data/",
            "http://localhost/",
            "http://localhost:3000/",
            "http://LOCALHOST/",
            "http://api.localhost/",
            "http://localhost./",
            "file:///etc/passwd",
            "FILE:///etc/passwd",
            "data:text/html,<script>alert(1)</script>",
            "javascript:alert(1)",
        ];
        for url in rejected {
            let result = validate_browser_url(url);
            assert!(result.is_err(), "expected rejection for {url}");
        }
    }

    #[test]
    fn additional_private_ranges_are_rejected() {
        let rejected = [
            "http://100.64.0.1/",
            "http://100.127.255.254/",
            "http://0.0.0.0/",
            "http://0.1.2.3/",
            "http://255.255.255.255/",
            "http://[::]/",
            "http://[fe80::1]/",
            "http://[fd12:3456::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:10.0.0.1]/",
        ];
        for url in rejected {
            let result = validate_browser_url(url);
            assert!(result.is_err(), "expected rejection for {url}");
        }
    }

    #[test]
    fn public_urls_are_accepted_and_normalised() {
        for url in [
            "https://example.com/",
            "http://example.com/news?q=rust",
            "https://8.8.8.8/",
            "http://172.32.0.1/",
            "https://[2606:4700:4700::1111]/",
        ] {
            let parsed = validate_browser_url(url)
                .unwrap_or_else(|error| panic!("expected acceptance for {url}: {error}"));
            assert!(!scheme(&parsed).is_empty());
        }
    }

    #[test]
    fn url_parsing_normalises_via_url_crate() {
        let parsed = validate_browser_url("HTTP://EXAMPLE.COM:80/path").expect("public URL");
        assert_eq!(parsed.scheme(), "http");
        assert_eq!(parsed.host_str(), Some("example.com"));
        assert_eq!(parsed.port_or_known_default(), Some(80));

        // Userinfo must not be able to smuggle a private host.
        let error = validate_browser_url("http://example.com@127.0.0.1/").unwrap_err();
        assert!(error.to_string().contains("private"), "{error}");

        // WHATWG normalisation expands numeric IPv4 literals.
        let error = validate_browser_url("http://2130706433/").unwrap_err();
        assert!(error.to_string().contains("private"), "{error}");

        // Default ports and case-insensitive schemes normalise.
        let parsed = validate_browser_url("HTTPS://Example.COM./").expect("public URL");
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(
            host_from_url("HTTPS://Example.COM./").as_deref(),
            Some("example.com")
        );
    }

    #[test]
    fn host_from_url_handles_ports_userinfo_and_ipv6() {
        assert_eq!(
            host_from_url("https://user:pass@Example.com:8443/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_from_url("http://[::1]:9000/").as_deref(), Some("::1"));
        assert_eq!(host_from_url("not a url").as_deref(), None);
    }

    #[test]
    fn dangerous_characters_are_rejected() {
        for url in [
            "https://evil.com'; rm -rf /",
            "https://evil.com/`id`",
            "https://evil.com/$(id)",
            "https://evil.com/a|b",
            "https://evil.com/a\nb",
        ] {
            assert!(
                validate_browser_url(url).is_err(),
                "expected rejection for {url:?}"
            );
        }
    }

    #[test]
    fn overlong_urls_are_rejected() {
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LENGTH + 1));
        assert!(validate_browser_url(&long).is_err());
    }

    #[test]
    fn is_private_host_classifies_addresses() {
        for host in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "::1",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "::ffff:192.168.0.1",
            "localhost",
            "foo.localhost",
        ] {
            assert!(is_private_host(host), "expected private: {host}");
        }
        for host in [
            "8.8.8.8",
            "1.1.1.1",
            "172.32.0.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "example.com",
            "not-an-ip.example",
        ] {
            assert!(!is_private_host(host), "expected public: {host}");
        }
    }

    #[tokio::test]
    async fn dns_rebinding_to_private_address_is_rejected() {
        let url = Url::parse("https://rebind.example/").expect("url");
        let error = assert_public_resolution_with(&url, |_host, _port| async {
            Ok(vec![IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))])
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("DNS rebinding"), "{error}");
    }

    #[tokio::test]
    async fn public_resolution_is_accepted() {
        let url = Url::parse("https://example.com/").expect("url");
        assert_public_resolution_with(&url, |_host, _port| async {
            Ok(vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))])
        })
        .await
        .expect("public resolution should pass");
    }

    #[tokio::test]
    async fn resolution_failures_defer_to_the_browser() {
        let url = Url::parse("https://does-not-resolve.example/").expect("url");
        assert_public_resolution_with(&url, |_host, _port| async { Err("nxdomain".into()) })
            .await
            .expect("resolution failure should not block navigation");
    }

    #[tokio::test]
    async fn literal_private_hosts_never_resolve() {
        let url = Url::parse("http://10.1.2.3/").expect("url");
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let result = assert_public_resolution_with(&url, |_host, _port| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Ok(Vec::new()) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
