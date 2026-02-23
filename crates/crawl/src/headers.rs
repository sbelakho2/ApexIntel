use rand::seq::SliceRandom;
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue};

const USER_AGENTS: &[&str] = &[
    // Chrome on Windows
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
    // Chrome on Mac
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_3) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    // Chrome on Linux
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    // Firefox
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:123.0) Gecko/20100101 Firefox/123.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:123.0) Gecko/20100101 Firefox/123.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:123.0) Gecko/20100101 Firefox/123.0",
    // Safari
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.3 Safari/605.1.15",
    // Edge
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Edg/122.0.0.0",
    // Brave
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 Brave/122",
    // Opera
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36 OPR/108.0.0.0",
];

const ACCEPT_LANGUAGES: &[(&str, &[&str])] = &[
    ("default", &["en-US,en;q=0.9", "en-US,en;q=0.9,es;q=0.8", "en-GB,en;q=0.9,en-US;q=0.8"]),
    ("FR", &["fr-FR,fr;q=0.9,en;q=0.8", "fr,fr-FR;q=0.9,en-US;q=0.8,en;q=0.7"]),
    ("DE", &["de-DE,de;q=0.9,en;q=0.8", "de,de-DE;q=0.9,en-US;q=0.8,en;q=0.7"]),
    ("AR", &["ar,ar-SA;q=0.9,en;q=0.8,fr;q=0.7", "ar-TN,ar;q=0.9,fr;q=0.8,en;q=0.7", "ar-MA,ar;q=0.9,fr;q=0.8,en;q=0.7"]),
    ("TN", &["ar-TN,ar;q=0.9,fr;q=0.8,en;q=0.7", "fr-TN,fr;q=0.9,ar;q=0.8,en;q=0.7"]),
    ("MA", &["ar-MA,ar;q=0.9,fr;q=0.8,en;q=0.7", "fr-MA,fr;q=0.9,ar;q=0.8,en;q=0.7"]),
    ("IL", &["he-IL,he;q=0.9,en;q=0.8", "he,en-US;q=0.9,en;q=0.8"]),
    ("CN", &["zh-CN,zh;q=0.9,en;q=0.8"]),
    ("ES", &["es-ES,es;q=0.9,en;q=0.8"]),
];

const REFERERS: &[&str] = &[
    "https://www.google.com/",
    "https://www.google.fr/",
    "https://www.google.de/",
    "https://www.google.co.ma/",
    "https://www.google.tn/",
    "https://www.bing.com/",
    "https://duckduckgo.com/",
    "", // direct
];

/// Generate realistic random HTTP headers for a crawl request.
pub fn random_headers(region: Option<&str>) -> HeaderMap {
    let mut rng = rand::thread_rng();
    let mut headers = HeaderMap::new();

    let ua = USER_AGENTS.choose(&mut rng).unwrap();
    headers.insert("User-Agent", HeaderValue::from_str(ua).unwrap());

    headers.insert(
        "Accept",
        HeaderValue::from_static(
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
    );

    let lang_key = region.unwrap_or("default");
    let langs = ACCEPT_LANGUAGES
        .iter()
        .find(|(k, _)| *k == lang_key)
        .map(|(_, v)| *v)
        .unwrap_or(ACCEPT_LANGUAGES[0].1);
    let lang = langs.choose(&mut rng).unwrap();
    headers.insert("Accept-Language", HeaderValue::from_str(lang).unwrap());

    headers.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate, br"));
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));
    headers.insert("Upgrade-Insecure-Requests", HeaderValue::from_static("1"));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));
    headers.insert("Cache-Control", HeaderValue::from_static("max-age=0"));

    // DNT ~30% of time
    if rng.gen_range(0..100) < 30 {
        headers.insert("DNT", HeaderValue::from_static("1"));
    }

    // Sec-CH-UA for Chrome-based UAs
    if ua.contains("Chrome") {
        let version = ua
            .split("Chrome/")
            .nth(1)
            .and_then(|s| s.split('.').next())
            .unwrap_or("122");
        let ch_ua = format!(
            "\"Chromium\";v=\"{version}\", \"Google Chrome\";v=\"{version}\", \"Not-A.Brand\";v=\"99\""
        );
        headers.insert("Sec-CH-UA", HeaderValue::from_str(&ch_ua).unwrap());
        headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
        let platform = if ua.contains("Windows") {
            "\"Windows\""
        } else if ua.contains("Mac") {
            "\"macOS\""
        } else {
            "\"Linux\""
        };
        headers.insert("Sec-CH-UA-Platform", HeaderValue::from_str(platform).unwrap());
    }

    // Referer
    let referer = REFERERS.choose(&mut rng).unwrap();
    if !referer.is_empty() {
        headers.insert("Referer", HeaderValue::from_str(referer).unwrap());
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("cross-site"));
    } else {
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    }

    headers
}

/// Get the list of available user agents.
pub fn user_agent_count() -> usize {
    USER_AGENTS.len()
}

/// Get a specific user agent by index.
pub fn get_user_agent(index: usize) -> Option<&'static str> {
    USER_AGENTS.get(index).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_headers_has_user_agent() {
        let headers = random_headers(None);
        assert!(headers.contains_key("User-Agent"));
        let ua = headers.get("User-Agent").unwrap().to_str().unwrap();
        assert!(ua.contains("Mozilla"));
    }

    #[test]
    fn test_random_headers_has_accept() {
        let headers = random_headers(None);
        assert!(headers.contains_key("Accept"));
        assert!(headers.contains_key("Accept-Language"));
        assert!(headers.contains_key("Accept-Encoding"));
    }

    #[test]
    fn test_random_headers_region_ar() {
        let headers = random_headers(Some("AR"));
        let lang = headers.get("Accept-Language").unwrap().to_str().unwrap();
        assert!(lang.contains("ar"));
    }

    #[test]
    fn test_random_headers_region_tn() {
        let headers = random_headers(Some("TN"));
        let lang = headers.get("Accept-Language").unwrap().to_str().unwrap();
        // Should contain Arabic or French variant for Tunisia
        assert!(lang.contains("ar") || lang.contains("fr"));
    }

    #[test]
    fn test_random_headers_region_cn() {
        let headers = random_headers(Some("CN"));
        let lang = headers.get("Accept-Language").unwrap().to_str().unwrap();
        assert!(lang.contains("zh"));
    }

    #[test]
    fn test_random_headers_has_security_headers() {
        let headers = random_headers(None);
        assert!(headers.contains_key("Sec-Fetch-Dest"));
        assert!(headers.contains_key("Sec-Fetch-Mode"));
    }

    #[test]
    fn test_user_agent_count() {
        assert!(user_agent_count() >= 10);
    }

    #[test]
    fn test_get_user_agent() {
        let ua = get_user_agent(0).unwrap();
        assert!(ua.contains("Mozilla"));
        assert!(get_user_agent(999).is_none());
    }

    #[test]
    fn test_headers_vary_between_calls() {
        // Call multiple times and verify we get different results sometimes
        let mut user_agents = std::collections::HashSet::new();
        for _ in 0..20 {
            let headers = random_headers(None);
            let ua = headers.get("User-Agent").unwrap().to_str().unwrap().to_string();
            user_agents.insert(ua);
        }
        // With 12 UAs and 20 samples, we should see at least 2 different ones
        assert!(user_agents.len() >= 2);
    }
}
