//! Fetch user-agent policy for HTTP source acquisition.
//!
//! Incident 2026-10-06: the browser User-Agent was hardcoded for exactly one
//! source; every other fetch used `ApexIntelBot/1.0`, which CDNs mass-403 even
//! for public RSS feeds. Seventy-eight sources sat in the failure ladder for
//! weeks while the same URLs answered a browser UA with HTTP 200. The policy
//! below is the single source of truth for the UA decision, unit-tested and
//! asserted by the source-pipeline dogfood.

/// Browser-class User-Agent used for source fetches.
pub const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Honest bot User-Agent for deployments that require it.
pub const BOT_USER_AGENT: &str = "ApexIntelBot/1.0 (+https://apex-intel.io/bot)";

/// Whether HTTP source fetches should present the browser User-Agent.
///
/// Default is `true`: robots.txt enforcement and per-domain pacing are
/// unchanged, so crawl politeness does not depend on the UA string, while
/// compatibility does. `APEX_CRAWL_USE_BOT_UA=1` restores the bot identity and
/// accepts the blocks that come with it.
pub fn prefer_browser_user_agent() -> bool {
    !std::env::var("APEX_CRAWL_USE_BOT_UA")
        .ok()
        .map(|value| apex_core::env::parse_truthy_flag(&value))
        .unwrap_or(false)
}

/// The User-Agent for a fetch under the current policy.
pub fn user_agent_for_fetch(prefers_browser: bool) -> &'static str {
    if prefers_browser {
        BROWSER_USER_AGENT
    } else {
        BOT_USER_AGENT
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn browser_ua_is_the_default_policy() {
        // The env override is checked in its own test; this asserts the
        // default contract that production depends on.
        if std::env::var("APEX_CRAWL_USE_BOT_UA").is_err() {
            assert!(prefer_browser_user_agent());
            assert_eq!(user_agent_for_fetch(true), BROWSER_USER_AGENT);
        }
    }

    #[test]
    fn bot_ua_override_is_honored() {
        let previous = std::env::var("APEX_CRAWL_USE_BOT_UA").ok();
        std::env::set_var("APEX_CRAWL_USE_BOT_UA", "1");
        assert!(!prefer_browser_user_agent());
        assert_eq!(user_agent_for_fetch(false), BOT_USER_AGENT);
        std::env::set_var("APEX_CRAWL_USE_BOT_UA", "0");
        assert!(prefer_browser_user_agent());
        match previous {
            Some(value) => std::env::set_var("APEX_CRAWL_USE_BOT_UA", value),
            None => std::env::remove_var("APEX_CRAWL_USE_BOT_UA"),
        }
    }
}
