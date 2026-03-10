use rand::seq::SliceRandom;
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue};

/// Desktop and mobile user agents reflecting current browser versions (2025/2026).
/// Covers Chrome 131/130/129, Firefox 133/132, Safari 18/17, Edge 131/130,
/// Brave, Opera, and mobile variants (Android Chrome, iPhone Safari).
const USER_AGENTS: &[&str] = &[
    // ── Chrome on Windows (131 / 130 / 129) ─────────────────────────────────
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36",
    // ── Chrome on Mac (131 / 130) ────────────────────────────────────────────
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_1) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    // ── Chrome on Linux ──────────────────────────────────────────────────────
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    // ── Firefox (133 / 132 / 131) ────────────────────────────────────────────
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:133.0) Gecko/20100101 Firefox/133.0",
    "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0",
    "Mozilla/5.0 (X11; Ubuntu; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
    // ── Safari on macOS (18.x / 17.x) ───────────────────────────────────────
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_1) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15",
    // ── Edge (131 / 130) ─────────────────────────────────────────────────────
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
    // ── Brave (131 / 130) ────────────────────────────────────────────────────
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Brave/131",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Brave/130",
    // ── Opera ────────────────────────────────────────────────────────────────
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 OPR/115.0.0.0",
    // ── Mobile: Android Chrome ───────────────────────────────────────────────
    "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.6778.81 Mobile Safari/537.36",
    "Mozilla/5.0 (Linux; Android 14; SM-S928B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.6723.107 Mobile Safari/537.36",
    "Mozilla/5.0 (Linux; Android 13; SM-G991B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.6778.81 Mobile Safari/537.36",
    // ── Mobile: iPhone Safari (iOS 18 / 17) ─────────────────────────────────
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Mobile/15E148 Safari/604.1",
    // ── Mobile: iPad Safari ──────────────────────────────────────────────────
    "Mozilla/5.0 (iPad; CPU OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
];

const ACCEPT_LANGUAGES: &[(&str, &[&str])] = &[
    (
        "default",
        &[
            "en-US,en;q=0.9",
            "en-US,en;q=0.9,es;q=0.8",
            "en-GB,en;q=0.9,en-US;q=0.8",
            "en-US,en;q=0.9,fr;q=0.8",
            "en-US,en;q=0.8",
        ],
    ),
    (
        "FR",
        &[
            "fr-FR,fr;q=0.9,en;q=0.8",
            "fr,fr-FR;q=0.9,en-US;q=0.8,en;q=0.7",
        ],
    ),
    (
        "DE",
        &[
            "de-DE,de;q=0.9,en;q=0.8",
            "de,de-DE;q=0.9,en-US;q=0.8,en;q=0.7",
        ],
    ),
    (
        "AR",
        &[
            "ar,ar-SA;q=0.9,en;q=0.8,fr;q=0.7",
            "ar-TN,ar;q=0.9,fr;q=0.8,en;q=0.7",
            "ar-MA,ar;q=0.9,fr;q=0.8,en;q=0.7",
        ],
    ),
    (
        "TN",
        &[
            "ar-TN,ar;q=0.9,fr;q=0.8,en;q=0.7",
            "fr-TN,fr;q=0.9,ar;q=0.8,en;q=0.7",
        ],
    ),
    (
        "MA",
        &[
            "ar-MA,ar;q=0.9,fr;q=0.8,en;q=0.7",
            "fr-MA,fr;q=0.9,ar;q=0.8,en;q=0.7",
        ],
    ),
    (
        "IL",
        &["he-IL,he;q=0.9,en;q=0.8", "he,en-US;q=0.9,en;q=0.8"],
    ),
    (
        "CN",
        &[
            "zh-CN,zh;q=0.9,en;q=0.8",
            "zh-CN,zh;q=0.8,zh-TW;q=0.7,en;q=0.6",
        ],
    ),
    (
        "ES",
        &["es-ES,es;q=0.9,en;q=0.8", "es-MX,es;q=0.9,en;q=0.8"],
    ),
    ("JP", &["ja-JP,ja;q=0.9,en;q=0.8"]),
    ("KR", &["ko-KR,ko;q=0.9,en;q=0.8"]),
    ("RU", &["ru-RU,ru;q=0.9,en;q=0.8"]),
    ("TR", &["tr-TR,tr;q=0.9,en;q=0.8"]),
    (
        "IN",
        &["en-IN,en;q=0.9,hi;q=0.8", "hi-IN,hi;q=0.9,en;q=0.8"],
    ),
    ("BR", &["pt-BR,pt;q=0.9,en;q=0.8"]),
];

const REFERERS: &[&str] = &[
    "https://www.google.com/",
    "https://www.google.fr/",
    "https://www.google.de/",
    "https://www.google.co.ma/",
    "https://www.google.tn/",
    "https://www.google.co.il/",
    "https://www.google.co.in/",
    "https://www.google.co.uk/",
    "https://www.google.com.tr/",
    "https://www.google.com.br/",
    "https://www.bing.com/",
    "https://duckduckgo.com/",
    "https://search.yahoo.com/",
    "https://yandex.com/",
    "https://www.linkedin.com/",
    "https://twitter.com/",
    "", // direct (no referer)
];

/// Generate realistic random HTTP headers for a crawl request.
pub fn random_headers(region: Option<&str>) -> HeaderMap {
    let mut rng = rand::thread_rng();
    random_headers_with_rng(region, &mut rng)
}

/// Generate headers using a caller-provided RNG (deterministic in tests).
pub fn random_headers_with_rng<R: Rng + ?Sized>(region: Option<&str>, rng: &mut R) -> HeaderMap {
    let mut headers = HeaderMap::new();

    let ua = USER_AGENTS.choose(rng).unwrap();
    headers.insert("User-Agent", HeaderValue::from_str(ua).unwrap());

    let is_mobile = ua.contains("Mobile")
        || ua.contains("iPhone")
        || ua.contains("iPad")
        || ua.contains("Android");

    // Accept — mobile browsers omit avif/webp support on older Android
    let accept = if is_mobile {
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
    } else {
        "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
    };
    headers.insert("Accept", HeaderValue::from_static(accept));

    let lang_key = region.unwrap_or("default");
    let langs = ACCEPT_LANGUAGES
        .iter()
        .find(|(k, _)| *k == lang_key)
        .map(|(_, v)| *v)
        .unwrap_or(ACCEPT_LANGUAGES[0].1);
    let lang = langs.choose(rng).unwrap();
    headers.insert("Accept-Language", HeaderValue::from_str(lang).unwrap());

    headers.insert(
        "Accept-Encoding",
        HeaderValue::from_static("gzip, deflate, br"),
    );
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));

    // Mobile browsers rarely send Upgrade-Insecure-Requests
    if !is_mobile {
        headers.insert("Upgrade-Insecure-Requests", HeaderValue::from_static("1"));
    }

    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));
    headers.insert("Cache-Control", HeaderValue::from_static("max-age=0"));

    // DNT ~25% of time (less common now with Global Privacy Control)
    if rng.gen_range(0..100) < 25 {
        headers.insert("DNT", HeaderValue::from_static("1"));
    }

    // Sec-CH-UA for Chrome-based UAs (includes Edge, Brave, Opera, Chrome mobile)
    if ua.contains("Chrome") || ua.contains("Edg/") {
        let version = ua
            .split("Chrome/")
            .nth(1)
            .and_then(|s| s.split('.').next())
            .unwrap_or("131");

        // Sec-CH-UA brand list — varies by browser
        let ch_ua = if ua.contains("Edg/") {
            let edge_ver = ua
                .split("Edg/")
                .nth(1)
                .and_then(|s| s.split('.').next())
                .unwrap_or(version);
            format!(
                "\"Microsoft Edge\";v=\"{edge_ver}\", \"Chromium\";v=\"{version}\", \"Not=A?Brand\";v=\"99\""
            )
        } else if ua.contains("Brave") {
            format!(
                "\"Brave\";v=\"{version}\", \"Chromium\";v=\"{version}\", \"Not-A.Brand\";v=\"8\""
            )
        } else if ua.contains("OPR/") {
            format!(
                "\"Opera\";v=\"{version}\", \"Chromium\";v=\"{version}\", \"Not_A Brand\";v=\"99\""
            )
        } else {
            // Standard Chrome brand hint — rotate the "Not A Brand" string spelling
            let not_brand = ["Not-A.Brand", "Not_A Brand", "Not(A:Brand", "Not;A=Brand"]
                .choose(rng)
                .unwrap();
            format!(
                "\"Google Chrome\";v=\"{version}\", \"Chromium\";v=\"{version}\", \"{not_brand}\";v=\"99\""
            )
        };

        headers.insert("Sec-CH-UA", HeaderValue::from_str(&ch_ua).unwrap());
        headers.insert(
            "Sec-CH-UA-Mobile",
            HeaderValue::from_static(if is_mobile { "?1" } else { "?0" }),
        );
        let platform = if ua.contains("Windows") {
            "\"Windows\""
        } else if ua.contains("Macintosh")
            || ua.contains("iPhone")
            || ua.contains("iPad")
            || ua.contains("Mac OS X")
        {
            "\"macOS\""
        } else if ua.contains("Android") {
            "\"Android\""
        } else {
            "\"Linux\""
        };
        headers.insert(
            "Sec-CH-UA-Platform",
            HeaderValue::from_str(platform).unwrap(),
        );
    }

    // Referer — mobile browsers less likely to send social referers
    let referer_pool: Vec<&str> = if is_mobile {
        REFERERS
            .iter()
            .filter(|r| r.is_empty() || r.contains("google") || r.contains("bing"))
            .copied()
            .collect()
    } else {
        REFERERS.to_vec()
    };
    let referer = referer_pool.choose(rng).unwrap_or(&&"");
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
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn test_random_headers_has_user_agent() {
        let mut rng = StdRng::seed_from_u64(1);
        let headers = random_headers_with_rng(None, &mut rng);
        assert!(headers.contains_key("User-Agent"));
        let ua = headers.get("User-Agent").unwrap().to_str().unwrap();
        assert!(ua.contains("Mozilla"));
    }

    #[test]
    fn test_random_headers_has_accept() {
        let mut rng = StdRng::seed_from_u64(2);
        let headers = random_headers_with_rng(None, &mut rng);
        assert!(headers.contains_key("Accept"));
        assert!(headers.contains_key("Accept-Language"));
        assert!(headers.contains_key("Accept-Encoding"));
    }

    #[test]
    fn test_random_headers_region_ar() {
        let mut rng = StdRng::seed_from_u64(3);
        let headers = random_headers_with_rng(Some("AR"), &mut rng);
        let lang = headers.get("Accept-Language").unwrap().to_str().unwrap();
        assert!(lang.contains("ar"));
    }

    #[test]
    fn test_random_headers_region_tn() {
        let mut rng = StdRng::seed_from_u64(4);
        let headers = random_headers_with_rng(Some("TN"), &mut rng);
        let lang = headers.get("Accept-Language").unwrap().to_str().unwrap();
        // Should contain Arabic or French variant for Tunisia
        assert!(lang.contains("ar") || lang.contains("fr"));
    }

    #[test]
    fn test_random_headers_region_cn() {
        let mut rng = StdRng::seed_from_u64(5);
        let headers = random_headers_with_rng(Some("CN"), &mut rng);
        let lang = headers.get("Accept-Language").unwrap().to_str().unwrap();
        assert!(lang.contains("zh"));
    }

    #[test]
    fn test_random_headers_has_security_headers() {
        let mut rng = StdRng::seed_from_u64(6);
        let headers = random_headers_with_rng(None, &mut rng);
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
        let mut rng = StdRng::seed_from_u64(7);
        for _ in 0..20 {
            let headers = random_headers_with_rng(None, &mut rng);
            let ua = headers
                .get("User-Agent")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            user_agents.insert(ua);
        }
        // With 12 UAs and 20 samples, we should see at least 2 different ones
        assert!(user_agents.len() >= 2);
    }
}
