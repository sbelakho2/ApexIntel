#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Audit #52 adversarial regression tests: hostile `href` values must never
//! survive HTML link extraction.
//!
//! These tests use only the public `apex_parse::html` API. The contract under
//! test is:
//!   * extracted links are absolute `http(s)` URLs, resolved against the page
//!     URL with `Url::join`;
//!   * non-http(s) schemes (`javascript:`, `data:`, `vbscript:`, `file:`,
//!     `blob:`, …) are dropped in any case/whitespace/control/entity
//!     obfuscation;
//!   * relative hrefs are dropped when no usable `http(s)` page URL exists —
//!     there is no raw-href fallback.

use apex_parse::html::{extract_page, extract_page_with_base};
use apex_parse::{
    award, cert, commodity, directory, job_post, patent, person, press, tender, trade_show,
};

const BASE: &str = "https://starz-electronics.com/company/index.html";

fn extracted_hrefs(html: &str, base: Option<&str>) -> Vec<String> {
    extract_page_with_base(html, base)
        .expect("hostile HTML must still parse")
        .links
        .into_iter()
        .map(|link| link.href)
        .collect()
}

/// Every extracted href must be an absolute http(s) URL and must not contain
/// raw control characters.
fn assert_all_http_s(hrefs: &[String]) {
    for href in hrefs {
        let parsed = url::Url::parse(href)
            .unwrap_or_else(|error| panic!("extracted href {href:?} is not a URL: {error}"));
        assert!(
            matches!(parsed.scheme(), "http" | "https"),
            "extracted href {href:?} has non-http(s) scheme {:?}",
            parsed.scheme()
        );
        assert!(
            !href.chars().any(|c| c.is_ascii_control()),
            "extracted href {href:?} contains control characters"
        );
    }
}

fn single_link_html(href: &str) -> String {
    format!("<html><body><a href=\"{href}\">payload</a></body></html>")
}

#[test]
fn absolute_javascript_hrefs_are_dropped_in_any_case() {
    for payload in [
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "JAVASCRIPT:alert(1)",
        "JaVaScRiPt:alert(document.cookie)",
        "javascript:/*",
    ] {
        let hrefs = extracted_hrefs(&single_link_html(payload), Some(BASE));
        assert!(
            hrefs.is_empty(),
            "javascript href {payload:?} survived as {hrefs:?}"
        );
    }
}

#[test]
fn whitespace_and_control_obfuscated_javascript_hrefs_are_dropped() {
    // The WHATWG URL parser strips ASCII tab/newline anywhere and leading
    // C0 controls/space, so all of these reconstruct the `javascript:` scheme
    // and must be rejected by normalize_url.
    for payload in [
        " javascript:alert(1)",
        "  javascript:alert(1)  ",
        "\tjavascript:alert(1)",
        "\njavascript:alert(1)",
        "\rjavascript:alert(1)",
        " \tjavascript:alert(1)",
        "java\tscript:alert(1)",
        "java\nscript:alert(1)",
        "java\rscript:alert(1)",
        "jav\ta\nscript:alert(1)",
        "\u{000B}javascript:alert(1)",
        "\u{000C}javascript:alert(1)",
    ] {
        let hrefs = extracted_hrefs(&single_link_html(payload), Some(BASE));
        assert!(
            hrefs.is_empty(),
            "obfuscated javascript href {payload:?} survived as {hrefs:?}"
        );
    }
}

#[test]
fn nul_in_href_cannot_become_a_javascript_link() {
    // html5ever replaces NUL inside an attribute value with U+FFFD, so by the
    // time extraction sees it the leading C0 control is gone and the value is
    // a relative URL. It must resolve to an inert same-origin path — never a
    // `javascript:` scheme.
    let hrefs = extracted_hrefs(&single_link_html("\u{0000}javascript:alert(1)"), Some(BASE));
    assert_all_http_s(&hrefs);
    for href in &hrefs {
        let parsed = url::Url::parse(href).expect("extracted href parses");
        assert_eq!(parsed.host_str(), Some("starz-electronics.com"));
        assert_ne!(parsed.scheme(), "javascript");
    }
}

#[test]
fn html_entity_obfuscated_javascript_hrefs_are_dropped() {
    for payload in [
        "&#106;avascript:alert(1)",
        "&#x6A;avascript:alert(1)",
        "javascript&#58;alert(1)",
        "javascript&colon;alert(1)",
        "jav&#x09;ascript:alert(1)",
        "jav&#x0A;ascript:alert(1)",
        "&#74;ava&#83;cript:alert(1)",
    ] {
        let hrefs = extracted_hrefs(&single_link_html(payload), Some(BASE));
        assert!(
            hrefs.is_empty(),
            "entity-obfuscated javascript href {payload:?} survived as {hrefs:?}"
        );
    }
}

#[test]
fn unicode_scheme_lookalikes_are_not_schemes() {
    // Browsers only recognize an ASCII `:` as the scheme separator, so these
    // stay relative and must resolve to inert same-origin paths.
    for payload in [
        "javascript\u{FF1A}alert(1)", // fullwidth colon
        "javascript\u{FE55}alert(1)", // small colon
        "ｊavascript:alert(1)",       // fullwidth j
    ] {
        let hrefs = extracted_hrefs(&single_link_html(payload), Some(BASE));
        assert_all_http_s(&hrefs);
        for href in &hrefs {
            let parsed = url::Url::parse(href).expect("extracted href parses");
            assert_eq!(
                parsed.host_str(),
                Some("starz-electronics.com"),
                "unicode lookalike {payload:?} escaped the page origin: {href:?}"
            );
        }
    }
}

#[test]
fn non_web_schemes_are_dropped() {
    for payload in [
        "data:text/html,<script>alert(1)</script>",
        "data:image/svg+xml;base64,PHN2Zz4=",
        "DATA:text/html,x",
        "vbscript:msgbox(1)",
        "VBScript:msgbox(1)",
        "file:///etc/passwd",
        "file://C:/Windows/system32/calc.exe",
        "ftp://example.com/x",
        "blob:https://example.com/9f1e",
        "about:blank",
        "chrome://settings",
        "feed:https://example.com/rss",
        "gopher://example.com/",
    ] {
        let hrefs = extracted_hrefs(&single_link_html(payload), Some(BASE));
        assert!(
            hrefs.is_empty(),
            "non-web scheme href {payload:?} survived as {hrefs:?}"
        );
    }
}

#[test]
fn relative_hrefs_resolve_against_page_url() {
    let html = r#"<html><body>
        <a href="/about">root-relative</a>
        <a href="capabilities">doc-relative</a>
        <a href="./team">dot-relative</a>
        <a href="../region/tunisia">parent-relative</a>
        <a href="?q=1">query-only</a>
    </body></html>"#;
    let hrefs = extracted_hrefs(html, Some(BASE));
    assert_all_http_s(&hrefs);
    for expected in [
        "https://starz-electronics.com/about",
        "https://starz-electronics.com/company/capabilities",
        "https://starz-electronics.com/company/team",
        "https://starz-electronics.com/region/tunisia",
        "https://starz-electronics.com/company/index.html?q=1",
    ] {
        assert!(
            hrefs.iter().any(|h| h == expected),
            "missing resolved href {expected:?} in {hrefs:?}"
        );
    }
}

#[test]
fn protocol_relative_hrefs_become_absolute_with_page_scheme() {
    // `//host/path` is resolved by the browser against the page scheme; the
    // extractor must emit the resulting absolute http(s) URL, never `//host`.
    let html = r#"<html><body><a href="//cdn.example.com/lib.js">cdn</a></body></html>"#;
    let hrefs = extracted_hrefs(html, Some(BASE));
    assert_eq!(hrefs, vec!["https://cdn.example.com/lib.js".to_string()]);

    let http_base = "http://starz-electronics.com/index.html";
    let hrefs = extracted_hrefs(html, Some(http_base));
    assert_eq!(hrefs, vec!["http://cdn.example.com/lib.js".to_string()]);
}

#[test]
fn percent_encoded_scheme_is_not_decoded_into_a_link_scheme() {
    // `%6Aavascript:` is a literal relative path segment, not a scheme. It may
    // resolve to a same-origin path, but it must never become `javascript:`.
    let payload = "%6Aavascript:alert(1)";
    let hrefs = extracted_hrefs(&single_link_html(payload), Some(BASE));
    assert_all_http_s(&hrefs);
    for href in &hrefs {
        let parsed = url::Url::parse(href).expect("href parses");
        assert_eq!(parsed.host_str(), Some("starz-electronics.com"));
        assert!(!href.to_ascii_lowercase().starts_with("javascript:"));
    }
}

#[test]
fn relative_hrefs_are_dropped_without_a_page_url() {
    let html = r#"<html><body>
        <a href="/about">root-relative</a>
        <a href="capabilities">doc-relative</a>
        <a href="//cdn.example.com/lib.js">cdn</a>
        <a href="javascript:alert(1)">js</a>
    </body></html>"#;
    let page = extract_page(html).expect("HTML parses");
    assert!(
        page.links.is_empty(),
        "no relative/raw href may survive without a base URL: {:?}",
        page.links
    );
}

#[test]
fn hostile_page_url_does_not_leak_into_relative_links() {
    for hostile_base in [
        "javascript:alert(1)",
        "data:text/html,x",
        "file:///etc/passwd",
        "not a url",
        "",
        "   ",
    ] {
        let html = r#"<html><body><a href="/about">About</a></body></html>"#;
        let hrefs = extracted_hrefs(html, Some(hostile_base));
        assert!(
            hrefs.is_empty(),
            "relative link survived hostile base {hostile_base:?}: {hrefs:?}"
        );
    }
}

#[test]
fn document_base_tag_does_not_override_page_url() {
    // `<base href="javascript:...">` must be ignored: resolution uses the
    // caller-supplied page URL, not attacker-controlled in-document markup.
    let html = r#"<html><head><base href="javascript:alert(1)"></head>
        <body><a href="about">About</a></body></html>"#;
    let hrefs = extracted_hrefs(html, Some(BASE));
    assert_eq!(
        hrefs,
        vec!["https://starz-electronics.com/company/about".to_string()]
    );

    // Without a page URL, a document <base> must not resurrect the link.
    let page = extract_page(html).expect("HTML parses");
    assert!(page.links.is_empty(), "{:?}", page.links);
}

#[test]
fn fragments_and_empty_hrefs_are_dropped() {
    let html = r##"<html><body>
        <a href="#">top</a>
        <a href="#section">section</a>
        <a href="">empty</a>
        <a href="   ">whitespace</a>
    </body></html>"##;
    let hrefs = extracted_hrefs(html, Some(BASE));
    assert!(hrefs.is_empty(), "{hrefs:?}");
}

#[test]
fn mixed_document_keeps_only_safe_absolute_links() {
    let html = r#"<html><body>
        <a href="https://example.com/a">safe</a>
        <a href="javascript:alert(1)">js</a>
        <a href="data:text/html,x">data</a>
        <a href="/relative">relative</a>
        <a href="//cdn.example.com/x">cdn</a>
        <a href="ftp://example.com/y">ftp</a>
    </body></html>"#;
    let hrefs = extracted_hrefs(html, Some(BASE));
    assert_eq!(
        hrefs,
        vec![
            "https://example.com/a".to_string(),
            "https://starz-electronics.com/relative".to_string(),
            "https://cdn.example.com/x".to_string(),
        ]
    );
}

/// The same contract `html.rs` enforces for hrefs must hold for the `url`
/// field of every structured extractor: either empty (unresolvable/hostile) or
/// an absolute `http(s)` URL within the sanitizer's length bound.
fn assert_url_field_is_safe(url: &str, context: &str) {
    if url.is_empty() {
        return;
    }
    let parsed = url::Url::parse(url)
        .unwrap_or_else(|error| panic!("{context}: url {url:?} is not a URL: {error}"));
    assert!(
        matches!(parsed.scheme(), "http" | "https"),
        "{context}: url {url:?} has non-http(s) scheme {:?}",
        parsed.scheme()
    );
    // Mirrors `apex_core::validation::MAX_URL_LEN`.
    assert!(
        url.len() <= 2048,
        "{context}: url length {} exceeds MAX_URL_LEN",
        url.len()
    );
    assert!(
        !url.chars().any(|c| c.is_ascii_control()),
        "{context}: url {url:?} contains control characters"
    );
}

#[test]
fn structured_extractors_never_echo_hostile_page_urls() {
    const BODY: &str = "ISO 9001 active. Buyer: Acme. Award: Prize. Winner: John Smith. \
         Copper $10/kg. Speaker: John Smith. Booth: A1. Reference: R-1.";
    let hostile_urls = [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        " data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox(1)",
        "file:///etc/passwd",
        "ftp://example.com/x",
        "not a url",
        "",
        "   ",
    ];

    for hostile in hostile_urls {
        for cert in cert::extract_certifications(BODY, hostile) {
            assert_url_field_is_safe(&cert.url, "cert");
        }
        for price in commodity::extract_commodity_prices(BODY, "src", hostile) {
            assert_url_field_is_safe(&price.url, "commodity");
        }
        let directory = directory::extract_directory(BODY, "title", hostile);
        assert_url_field_is_safe(&directory.url, "directory");
        for p in person::extract_person("Contact john.smith@example.com", hostile) {
            assert_url_field_is_safe(&p.url, "person");
        }
        let press = press::extract_press(BODY, "title", hostile);
        assert_url_field_is_safe(&press.url, "press");
        let tender = tender::extract_tender(BODY, "title", hostile, "portal");
        assert_url_field_is_safe(&tender.url, "tender");
        let show = trade_show::extract_trade_show(BODY, "title", hostile);
        assert_url_field_is_safe(&show.url, "trade_show");
        let patent = patent::extract_patent(BODY, "title", hostile, "US");
        assert_url_field_is_safe(&patent.url, "patent");
        let award = award::extract_award(BODY, "title", hostile, "US");
        assert_url_field_is_safe(&award.url, "award");
        let posting = job_post::extract_job_posting(BODY, "title", hostile);
        assert_url_field_is_safe(&posting.source_url, "job_post");
    }
}

#[test]
fn person_linkedin_urls_are_sanitized() {
    let pathological = format!("Profile: https://www.linkedin.com/in/{}", "a".repeat(4000));
    for p in person::extract_person(&pathological, "https://example.com") {
        if let Some(url) = &p.linkedin_url {
            assert_url_field_is_safe(url, "person-linkedin");
            assert!(
                !url.contains(&"a".repeat(4000)),
                "raw over-long LinkedIn URL leaked"
            );
        }
    }
}

#[test]
fn structured_extractors_sanitize_overlong_page_urls() {
    let body = "ISO 9001 active. Copper $10/kg. Contact john.smith@example.com";
    let overlong = format!("https://example.com/{}", "a".repeat(5000));
    let cert = cert::extract_certifications(body, &overlong);
    for c in cert {
        assert_url_field_is_safe(&c.url, "cert-overlong");
    }
    let directory = directory::extract_directory(body, "title", &overlong);
    assert_url_field_is_safe(&directory.url, "directory-overlong");
    let press = press::extract_press(body, "title", &overlong);
    assert_url_field_is_safe(&press.url, "press-overlong");
}

#[test]
fn every_extracted_href_is_absolute_http_or_https() {
    let hostile = [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "\tjavascript:alert(1)",
        "java\nscript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox(1)",
        "file:///etc/passwd",
        "blob:https://example.com/id",
        "about:blank",
        "//cdn.example.com/ok.js",
        "/root-relative",
        "relative",
        "?query",
        "#frag",
        "",
        "https://example.com/a",
        "http://example.com/b",
        "HTTPS://EXAMPLE.COM/C",
    ];
    let anchors: String = hostile
        .iter()
        .map(|h| format!("<a href=\"{h}\">x</a>"))
        .collect();
    let html = format!("<html><body>{anchors}</body></html>");
    let page = extract_page_with_base(&html, Some(BASE)).expect("mixed HTML parses");
    assert_all_http_s(
        &page
            .links
            .iter()
            .map(|l| l.href.clone())
            .collect::<Vec<_>>(),
    );
    // The raw fallback is gone: no result may equal a hostile raw value.
    for link in &page.links {
        assert!(
            !hostile.contains(&link.href.as_str()) || link.href.starts_with("http"),
            "raw hostile href leaked: {:?}",
            link.href
        );
    }
}
