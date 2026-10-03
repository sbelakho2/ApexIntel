//! E-mail recipient policy shared by the API settings form and the worker's
//! digest sender.
//!
//! Digest recipients are restricted to an explicit allowlist of approved
//! company domains (`APEX_DIGEST_ALLOWED_DOMAINS`, comma/semicolon separated).
//! The allowlist is enforced on the write path (the settings form refuses an
//! out-of-policy address) and again at send time (the worker refuses to mail a
//! recipient that was stored before the allowlist existed or was tightened).
//! An empty/absent allowlist keeps the documented pre-existing behavior: no
//! domain restriction.

/// Environment variable holding the approved digest-recipient domains.
pub const ALLOWED_DOMAINS_ENV: &str = "APEX_DIGEST_ALLOWED_DOMAINS";

/// Parse an allowlist value into normalized domains.
///
/// Accepts `example.com`, `@example.com` and mixed separators; matching is
/// case-insensitive. Duplicates are removed while preserving the first
/// occurrence order.
pub fn parse_allowed_domains(raw: &str) -> Vec<String> {
    let mut domains: Vec<String> = Vec::new();
    for part in raw.split([',', ';', '\n', ' ']) {
        let domain = part.trim().trim_start_matches('@').to_ascii_lowercase();
        if domain.is_empty() {
            continue;
        }
        if !domains.contains(&domain) {
            domains.push(domain);
        }
    }
    domains
}

/// The domain part of an e-mail address, lowercased.
pub fn recipient_domain(email: &str) -> Option<String> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    if local.is_empty() || domain.is_empty() {
        return None;
    }
    Some(domain.to_ascii_lowercase())
}

/// Whether `email` may receive a digest under `allowed_domains`.
///
/// An empty allowlist allows every syntactically valid recipient (the
/// historical behavior). A non-empty allowlist matches the exact domain or a
/// subdomain of an approved one (`analyst@news.example.com` matches
/// `example.com`) but never a lookalike suffix (`example.com.evil.test` does
/// not match `example.com`).
pub fn is_approved_recipient(email: &str, allowed_domains: &[String]) -> bool {
    if allowed_domains.is_empty() {
        return true;
    }
    let Some(domain) = recipient_domain(email) else {
        return false;
    };
    allowed_domains.iter().any(|allowed| {
        domain == *allowed
            || domain
                .strip_suffix(allowed)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// Read and parse the allowlist from the process environment.
pub fn allowed_domains_from_env() -> Vec<String> {
    std::env::var(ALLOWED_DOMAINS_ENV)
        .ok()
        .map(|raw| parse_allowed_domains(&raw))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_parsing_normalizes_and_deduplicates() {
        assert_eq!(
            parse_allowed_domains(" Example.com, @example.org;example.net "),
            vec!["example.com", "example.org", "example.net"]
        );
        assert_eq!(parse_allowed_domains("a.com,a.com"), vec!["a.com"]);
        assert!(parse_allowed_domains(" , ; ").is_empty());
    }

    #[test]
    fn empty_allowlist_allows_every_recipient() {
        assert!(is_approved_recipient("anyone@wherever.test", &[]));
    }

    #[test]
    fn allowlist_matches_domain_and_subdomains_only() {
        let allowed = parse_allowed_domains("example.com");
        assert!(is_approved_recipient("analyst@example.com", &allowed));
        assert!(is_approved_recipient("analyst@news.example.com", &allowed));
        assert!(!is_approved_recipient(
            "analyst@example.com.evil.test",
            &allowed
        ));
        assert!(!is_approved_recipient("analyst@notexample.com", &allowed));
        assert!(!is_approved_recipient("not-an-email", &allowed));
        assert!(!is_approved_recipient("@example.com", &allowed));
    }

    #[test]
    fn recipient_domain_lowercases_and_validates_shape() {
        assert_eq!(
            recipient_domain("Ops@Example.COM"),
            Some("example.com".to_string())
        );
        assert_eq!(recipient_domain("missing-at"), None);
        assert_eq!(recipient_domain("@example.com"), None);
    }
}
