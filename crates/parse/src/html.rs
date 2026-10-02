use anyhow::Result;
use encoding_rs::Encoding;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use url::Url;

use crate::normalizer;
use apex_core::validation::normalize_url;

/// Extracted page content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContent {
    pub title: String,
    pub description: String,
    pub body_text: String,
    pub links: Vec<ExtractedLink>,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub language: String,
    /// Per-field confidence: 0.0 = missing/default, 1.0 = strong signal (B109)
    pub field_confidence: FieldConfidence,
    /// Named entities extracted via per-language NER pipeline.
    #[serde(default)]
    pub entities: Vec<crate::ner::ExtractedEntity>,
}

/// Confidence scores for each extracted field (B109).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FieldConfidence {
    pub title: f64,
    pub description: f64,
    pub body_text: f64,
    pub language: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedLink {
    pub text: String,
    pub href: String,
}

/// Extract structured content from raw HTML.
///
/// Relative links are resolved against the first `<base href>` when the
/// document supplies one (an absolute base wins; a relative base is resolved
/// against `page_url`), falling back to `page_url`. Link extraction keeps only
/// absolute `http(s)` results, so a relative/protocol-relative or hostile
/// scheme value is never stored raw. There is deliberately no raw-href
/// fallback: whitespace/control-obfuscated `javascript:`, `data:`,
/// `vbscript:` or similar payloads never survive extraction.
#[instrument(skip(html_content))]
pub fn extract_page(html_content: &str, page_url: Option<&Url>) -> Result<PageContent> {
    let doc = Html::parse_document(html_content);

    let title = extract_title(&doc);
    let description = extract_meta_description(&doc);
    let body_text = extract_body_text(&doc);

    // B106: If content is only scripts/styles, return empty body
    let effective_body = if normalizer::is_only_scripts_or_styles(html_content) {
        String::new()
    } else {
        body_text
    };

    let links = extract_links(&doc, page_url);
    // B105: Deduplicate emails and phones
    let emails = normalizer::dedup_preserving_order(normalizer::extract_emails(&effective_body));
    let phones = normalizer::dedup_preserving_order(normalizer::extract_phones(&effective_body));
    let mut language = crate::multilingual::detect_language(&effective_body);
    if language.trim().is_empty() {
        language = "en".to_string();
    }

    // B109: Compute per-field confidence scores
    let field_confidence = FieldConfidence {
        title: if title.is_empty() { 0.0 } else { 1.0 },
        description: if description.is_empty() {
            0.0
        } else {
            // Lower confidence if description came from fallback (first <p>)
            let has_meta = has_meta_description(&doc);
            if has_meta {
                1.0
            } else {
                0.5
            }
        },
        body_text: if effective_body.is_empty() {
            0.0
        } else if effective_body.len() < 50 {
            0.3
        } else {
            1.0
        },
        language: if effective_body.len() < 30 { 0.3 } else { 0.9 },
    };

    // Extract entities using per-language NER pipeline (Phase 2.4)
    let mut entities = crate::ner::extract_entities(&effective_body, &language);
    crate::entity_canonical::resolve_entities_batch(&mut entities);

    Ok(PageContent {
        title,
        description,
        body_text: effective_body,
        links,
        emails,
        phones,
        language,
        field_confidence,
        entities,
    })
}

/// Extract structured content from raw HTML bytes.
/// Falls back to lossy decoding if UTF-8 decoding fails.
pub fn extract_page_bytes(html_bytes: &[u8], page_url: Option<&Url>) -> Result<PageContent> {
    let html = if let Ok(s) = std::str::from_utf8(html_bytes) {
        s.to_string()
    } else {
        let encoding = detect_charset(html_bytes).unwrap_or(encoding_rs::UTF_8);
        let (decoded, _, _) = encoding.decode(html_bytes);
        decoded.to_string()
    };
    extract_page(&html, page_url)
}

fn detect_charset(html_bytes: &[u8]) -> Option<&'static Encoding> {
    let probe = String::from_utf8_lossy(&html_bytes[..html_bytes.len().min(2048)]);
    let lower = probe.to_lowercase();
    let markers = ["charset=", "charset\"", "charset\'"];
    for marker in &markers {
        if let Some(idx) = lower.find(marker) {
            let start = idx + marker.len();
            let tail = &lower[start..];
            let value = tail
                .trim_start_matches(['"', '\'', '=', ' '].as_ref())
                .split(|c: char| c == '"' || c == '\'' || c.is_whitespace() || c == ';')
                .next()
                .unwrap_or("")
                .trim();
            if !value.is_empty() {
                if let Some(enc) = Encoding::for_label(value.as_bytes()) {
                    return Some(enc);
                }
            }
        }
    }
    None
}

fn extract_title(doc: &Html) -> String {
    let sel = Selector::parse("title")
        .unwrap_or_else(|error| panic!("invalid title selector: {error:?}"));
    doc.select(&sel)
        .next()
        .map(|el| normalizer::normalize_whitespace(&el.text().collect::<String>()))
        .unwrap_or_default()
}

fn extract_meta_description(doc: &Html) -> String {
    let selectors = [
        r#"meta[name="description"]"#,
        r#"meta[property="og:description"]"#,
        r#"meta[name="twitter:description"]"#,
    ];
    for sel in selectors {
        if let Ok(selector) = Selector::parse(sel) {
            if let Some(desc) = doc
                .select(&selector)
                .next()
                .and_then(|el| el.value().attr("content"))
            {
                let normalized = normalizer::normalize_whitespace(desc);
                if !normalized.is_empty() {
                    return normalized;
                }
            }
        }
    }
    // Fallback to first paragraph text if no meta description is present
    if let Ok(p_sel) = Selector::parse("p") {
        if let Some(p) = doc.select(&p_sel).next() {
            let normalized = normalizer::normalize_whitespace(&p.text().collect::<String>());
            if !normalized.is_empty() {
                return normalized;
            }
        }
    }
    String::new()
}

/// Check if any real meta description tag exists (not fallback to <p>).
fn has_meta_description(doc: &Html) -> bool {
    let selectors = [
        r#"meta[name="description"]"#,
        r#"meta[property="og:description"]"#,
        r#"meta[name="twitter:description"]"#,
    ];
    for sel in selectors {
        if let Ok(selector) = Selector::parse(sel) {
            if doc
                .select(&selector)
                .next()
                .and_then(|el| el.value().attr("content"))
                .map(|c| !c.trim().is_empty())
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

/// Upper bound on the raw body text collected by [`collect_text_skipping`].
///
/// The walk itself is iterative and therefore independent of document depth,
/// but a hostile page can still present an unbounded volume of text nodes.
/// When the next text node would push the buffer past this cap the walk stops,
/// so the returned text is truncated at a text-node boundary (never inside a
/// UTF-8 code point) rather than growing without limit.
const MAX_BODY_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Collect text under `root` in document order with an explicit stack,
/// skipping `script`/`style`/`noscript` subtrees entirely.
///
/// Text fragments are separated by a single space (later collapsed by
/// [`normalizer::normalize_whitespace`]) so inline tags do not glue words
/// together. The walk is iterative, so a deeply nested document cannot exhaust
/// the call stack; at most [`MAX_BODY_TEXT_BYTES`] bytes are collected.
fn collect_text_skipping(root: scraper::ElementRef<'_>) -> String {
    let mut text = String::new();
    // `root` derefs to a `NodeRef`; seeding it lets the loop handle its
    // children uniformly. Nodes are pushed as `NodeRef`s whose type is
    // inferred without naming the `ego-tree` crate (not a direct dependency).
    let mut stack = vec![*root];
    while let Some(node) = stack.pop() {
        match node.value() {
            scraper::Node::Text(t) => {
                let fragment = t.text.as_ref();
                if text.len().saturating_add(fragment.len()) > MAX_BODY_TEXT_BYTES {
                    break;
                }
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(fragment);
                continue;
            }
            scraper::Node::Element(el) if matches!(el.name(), "script" | "style" | "noscript") => {
                // Skip the whole subtree: script/style/noscript text is never
                // document content.
                continue;
            }
            _ => {}
        }
        // Push children in reverse so the LIFO stack pops them in document
        // order.
        let mut children = Vec::new();
        let mut child = node.first_child();
        while let Some(current) = child {
            children.push(current);
            child = current.next_sibling();
        }
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    text
}

/// Extract normalized visible body text, skipping `script`/`style`/`noscript`
/// subtrees. Falls back to the whole document (minus those subtrees) when no
/// `<body>` element exists or the body holds no usable text.
fn extract_body_text(doc: &Html) -> String {
    let body_sel =
        Selector::parse("body").unwrap_or_else(|error| panic!("invalid body selector: {error:?}"));

    match doc.select(&body_sel).next() {
        Some(body) => {
            let text = collect_text_skipping(body);
            let normalized = normalizer::remove_boilerplate(&text);
            if normalized.is_empty() {
                let fallback = collect_text_skipping(doc.root_element());
                normalizer::remove_boilerplate(&fallback)
            } else {
                normalized
            }
        }
        None => {
            let text = collect_text_skipping(doc.root_element());
            normalizer::normalize_whitespace(&text)
        }
    }
}

/// Resolve the base used for relative hrefs.
///
/// The first `<base href>` in document order wins, matching browser behaviour.
/// An absolute base value is used as-is; a relative one is resolved against
/// `page_url` with `Url::join`. When the tag is missing, empty, or entirely
/// unresolvable, `page_url` is used instead. The base is not filtered to
/// http(s) here: [`resolve_href`] still requires every stored link to resolve
/// to an absolute `http(s)` URL, so a hostile base can at worst make relative
/// links unresolvable (they are dropped) — it can never be stored as a link.
fn resolve_document_base(doc: &Html, page_url: Option<&Url>) -> Option<Url> {
    let base_sel = Selector::parse("base[href]")
        .unwrap_or_else(|error| panic!("invalid base selector: {error:?}"));
    if let Some(raw) = doc
        .select(&base_sel)
        .next()
        .and_then(|el| el.value().attr("href"))
    {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(absolute) = Url::parse(trimmed) {
                return Some(absolute);
            }
            if let Some(page) = page_url {
                if let Ok(joined) = page.join(trimmed) {
                    return Some(joined);
                }
            }
        }
    }
    page_url.cloned()
}

/// Resolve one raw `href` against `base` to an absolute `http(s)` URL, or
/// `None` when the value is hostile, opaque, or unresolvable.
///
/// `Url::parse`/`Url::join` strip leading/trailing C0-control/space characters
/// and remove ASCII tab/newline anywhere in the input, so ` javascript:`,
/// `\tjavascript:` and `java\nscript:` all collapse to the hostile
/// `javascript:` scheme before the scheme gate; they are rejected together
/// with `data:`, `vbscript:`, `file:` and friends. Relative and
/// protocol-relative values are joined against `base` when one is available
/// and dropped otherwise — the raw value is never returned.
fn resolve_href(raw: &str, base: Option<&Url>) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let resolved = match base {
        Some(base) => base.join(trimmed).ok()?,
        None => Url::parse(trimmed).ok()?,
    };
    if !matches!(resolved.scheme(), "http" | "https") {
        return None;
    }
    normalize_url(resolved.as_str())
}

fn extract_links(doc: &Html, page_url: Option<&Url>) -> Vec<ExtractedLink> {
    let sel = Selector::parse("a[href]")
        .unwrap_or_else(|error| panic!("invalid link selector: {error:?}"));
    let base = resolve_document_base(doc, page_url);
    doc.select(&sel)
        .filter_map(|el| {
            let text = normalizer::normalize_whitespace(&el.text().collect::<String>());
            let href = resolve_href(el.value().attr("href")?, base.as_ref())?;
            Some(ExtractedLink { text, href })
        })
        .collect()
}

/// Extract all text content from specific CSS selectors.
pub fn extract_by_selector(html_content: &str, css_selector: &str) -> Vec<String> {
    let doc = Html::parse_document(html_content);
    match Selector::parse(css_selector) {
        Ok(sel) => doc
            .select(&sel)
            .map(|el| normalizer::normalize_whitespace(&el.text().collect::<String>()))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Extract table data as rows of cells.
pub fn extract_table(html_content: &str, table_selector: &str) -> Vec<Vec<String>> {
    let doc = Html::parse_document(html_content);
    let table_sel = match Selector::parse(table_selector) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let tr_sel = Selector::parse("tr")
        .unwrap_or_else(|error| panic!("invalid table-row selector: {error:?}"));
    let td_sel = Selector::parse("td, th")
        .unwrap_or_else(|error| panic!("invalid table-cell selector: {error:?}"));

    let mut rows = Vec::new();
    if let Some(table) = doc.select(&table_sel).next() {
        for tr in table.select(&tr_sel) {
            let cells: Vec<String> = tr
                .select(&td_sel)
                .map(|td| normalizer::normalize_whitespace(&td.text().collect::<String>()))
                .collect();
            if !cells.is_empty() {
                rows.push(cells);
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_HTML: &str = r#"
    <!DOCTYPE html>
    <html>
    <head>
        <title>Starz Electronics - EMS Manufacturer</title>
        <meta name="description" content="Leading EMS provider in Tunisia">
    </head>
    <body>
        <nav>Home | About | Contact</nav>
        <main>
            <h1>Welcome to Starz Electronics</h1>
            <p>We provide SMT assembly and PCB manufacturing services for automotive clients.</p>
            <p>Contact us at info@starz-electronics.com or call +216 71 123 456.</p>
            <a href="https://starz-electronics.com/about">About Us</a>
            <a href="https://starz-electronics.com/capabilities">Capabilities</a>
            <a href="/top">Back to top</a>
        </main>
        <footer>© 2024 Starz Electronics. All rights reserved.</footer>
    </body>
    </html>
    "#;

    fn sample_page(html: &str) -> PageContent {
        extract_page(html, None).unwrap_or_else(|error| panic!("sample HTML should parse: {error}"))
    }

    fn page_url(raw: &str) -> Url {
        Url::parse(raw).unwrap_or_else(|error| panic!("test URL `{raw}` should parse: {error}"))
    }

    #[test]
    fn test_extract_page_title() {
        let page = sample_page(SAMPLE_HTML);
        assert_eq!(page.title, "Starz Electronics - EMS Manufacturer");
    }

    #[test]
    fn test_extract_page_description() {
        let page = sample_page(SAMPLE_HTML);
        assert_eq!(page.description, "Leading EMS provider in Tunisia");
    }

    #[test]
    fn test_extract_page_body() {
        let page = sample_page(SAMPLE_HTML);
        assert!(page.body_text.contains("SMT assembly"));
        assert!(page.body_text.contains("PCB manufacturing"));
    }

    #[test]
    fn test_extract_page_emails() {
        let page = sample_page(SAMPLE_HTML);
        assert!(page
            .emails
            .contains(&"info@starz-electronics.com".to_string()));
    }

    #[test]
    fn test_extract_page_phones() {
        let page = sample_page(SAMPLE_HTML);
        assert!(!page.phones.is_empty());
    }

    #[test]
    fn test_extract_page_links() {
        let page = sample_page(SAMPLE_HTML);
        // Should exclude #top link
        let hrefs: Vec<&str> = page.links.iter().map(|l| l.href.as_str()).collect();
        assert!(hrefs.contains(&"https://starz-electronics.com/about"));
        assert!(hrefs.contains(&"https://starz-electronics.com/capabilities"));
        assert!(!hrefs.iter().any(|h| h.starts_with('#')));
    }

    // Audit #84: link extraction must resolve relative and protocol-relative
    // hrefs against the page URL and keep only absolute http(s) results — no
    // raw-href fallback.
    #[test]
    fn test_extract_links_resolves_relative_against_page_url() {
        let html = r#"<html><body>
            <a href="/about">About</a>
            <a href="capabilities">Capabilities</a>
            <a href="//cdn.example.com/lib.js">CDN</a>
            <a href="../up">Up</a>
        </body></html>"#;
        let base = page_url("https://starz-electronics.com/company/index.html");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("relative-link HTML should parse: {error}"));
        let hrefs: Vec<&str> = page.links.iter().map(|l| l.href.as_str()).collect();
        assert!(hrefs.contains(&"https://starz-electronics.com/about"));
        assert!(hrefs.contains(&"https://starz-electronics.com/company/capabilities"));
        assert!(hrefs.contains(&"https://cdn.example.com/lib.js"));
        assert!(hrefs.contains(&"https://starz-electronics.com/up"));
    }

    #[test]
    fn test_extract_links_keeps_absolute_http_s_only() {
        let html = r#"<html><body>
            <a href="https://example.com/a">A</a>
            <a href="http://example.com/b">B</a>
            <a href="ftp://example.com/c">C</a>
            <a href="file:///etc/passwd">D</a>
        </body></html>"#;
        let page = sample_page(html);
        let hrefs: Vec<&str> = page.links.iter().map(|l| l.href.as_str()).collect();
        assert!(hrefs.contains(&"https://example.com/a"));
        assert!(hrefs.contains(&"http://example.com/b"));
        assert_eq!(hrefs.len(), 2);
    }

    #[test]
    fn test_extract_links_drops_javascript_mixed_case() {
        let html = r#"<html><body>
            <a href="javascript:alert(1)">lower</a>
            <a href="JavaScript:alert(1)">mixed</a>
            <a href="JAVASCRIPT:alert(1)">upper</a>
        </body></html>"#;
        let base = page_url("https://example.com/");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("hostile-link HTML should parse: {error}"));
        assert!(page.links.is_empty(), "javascript: hrefs must be dropped");
    }

    #[test]
    fn test_extract_links_drops_data_and_vbscript() {
        let html = r#"<html><body>
            <a href="data:text/html,<script>alert(1)</script>">data</a>
            <a href="vbscript:msgbox(1)">vb</a>
        </body></html>"#;
        let base = page_url("https://example.com/");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("hostile-link HTML should parse: {error}"));
        assert!(
            page.links.is_empty(),
            "data:/vbscript: hrefs must be dropped"
        );
    }

    #[test]
    fn test_extract_links_drops_whitespace_obfuscated_javascript() {
        let html = "<html><body>\
            <a href=\" javascript:alert(1)\">space</a>\
            <a href=\"\tjavascript:alert(1)\">tab</a>\
            <a href=\"java\nscript:alert(1)\">newline</a>\
            <a href=\" \tjavascript:alert(1)\">space-tab</a>\
            </body></html>";
        let base = page_url("https://example.com/");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("obfuscated-link HTML should parse: {error}"));
        assert!(
            page.links.is_empty(),
            "whitespace/control-obfuscated javascript hrefs must be dropped: {:?}",
            page.links
        );
    }

    #[test]
    fn test_extract_links_without_base_drops_relative_hrefs() {
        let html = r#"<html><body>
            <a href="/about">About</a>
            <a href="capabilities">Capabilities</a>
        </body></html>"#;
        let page = sample_page(html);
        assert!(page.links.is_empty());
    }

    // Audit #84: relative paths resolve to the page origin, `..` walks up the
    // document path, and protocol-relative hrefs pick up the page scheme.
    #[test]
    fn test_extract_links_relative_admin_and_parent_path() {
        let html = r#"<html><body>
            <a href="/admin">Admin</a>
            <a href="../up">Up</a>
            <a href="//evil.example/x">Protocol relative</a>
        </body></html>"#;
        let base = page_url("https://shop.example.com/a/b/index.html");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("relative-link HTML should parse: {error}"));
        let hrefs: Vec<&str> = page.links.iter().map(|l| l.href.as_str()).collect();
        assert_eq!(
            hrefs,
            vec![
                "https://shop.example.com/admin",
                "https://shop.example.com/a/up",
                "https://evil.example/x",
            ]
        );
    }

    #[test]
    fn test_extract_links_honours_absolute_base_tag() {
        let html = r#"<html><head><base href="https://cdn.example/assets/"></head>
            <body>
                <a href="app.js">Asset</a>
                <a href="https://other.example/x">Absolute</a>
            </body></html>"#;
        let base = page_url("https://starz.example.com/page/index.html");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("base-tag HTML should parse: {error}"));
        let hrefs: Vec<&str> = page.links.iter().map(|l| l.href.as_str()).collect();
        assert_eq!(
            hrefs,
            vec![
                "https://cdn.example/assets/app.js",
                "https://other.example/x"
            ]
        );
    }

    #[test]
    fn test_extract_links_resolves_relative_base_tag_against_page_url() {
        let html = r#"<html><head><base href="/base/"></head>
            <body><a href="x.html">X</a></body></html>"#;
        let base = page_url("https://starz.example.com/dir/page.html");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("base-tag HTML should parse: {error}"));
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].href, "https://starz.example.com/base/x.html");
    }

    #[test]
    fn test_extract_links_base_tag_without_page_url_is_unresolvable() {
        let html = r#"<html><head><base href="/base/"></head>
            <body><a href="x.html">X</a></body></html>"#;
        let page = sample_page(html);
        assert!(
            page.links.is_empty(),
            "relative <base> needs a page URL: {:?}",
            page.links
        );
    }

    #[test]
    fn test_extract_links_hostile_base_tag_cannot_smuggle_hrefs() {
        let html = r#"<html><head><base href="javascript:alert(1)"></head>
            <body>
                <a href="about">Relative</a>
                <a href="data:text/html,x">Data</a>
                <a href="https://safe.example/ok">Absolute</a>
            </body></html>"#;
        let base = page_url("https://starz.example.com/");
        let page = extract_page(html, Some(&base))
            .unwrap_or_else(|error| panic!("hostile-base HTML should parse: {error}"));
        assert_eq!(
            page.links
                .iter()
                .map(|l| l.href.as_str())
                .collect::<Vec<_>>(),
            vec!["https://safe.example/ok"]
        );
    }

    #[test]
    fn test_extract_links_never_stores_raw_hostile_href() {
        let hostile = [
            "JaVaScRiPt:alert(1)",
            " javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "/admin",
            "../up",
            "//evil.example/x",
        ];
        let base = page_url("https://shop.example.com/a/b/index.html");
        for raw in hostile {
            let html = format!("<html><body><a href=\"{raw}\">x</a></body></html>");
            let page = extract_page(&html, Some(&base))
                .unwrap_or_else(|error| panic!("hostile-href HTML should parse: {error}"));
            for link in &page.links {
                assert_ne!(link.href, raw, "raw href leaked for {raw:?}");
                assert!(
                    link.href.starts_with("http://") || link.href.starts_with("https://"),
                    "stored href {actual:?} for {raw:?} is not absolute http(s)",
                    actual = link.href
                );
            }
        }
    }

    #[test]
    fn test_extract_page_language() {
        let page = sample_page(SAMPLE_HTML);
        assert_eq!(page.language, "en");
    }

    #[test]
    fn test_extract_by_selector() {
        let results = extract_by_selector(SAMPLE_HTML, "h1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "Welcome to Starz Electronics");
    }

    #[test]
    fn test_extract_empty_html() {
        let page = sample_page("");
        assert!(page.title.is_empty());
        assert!(page.description.is_empty());
        assert!(page.body_text.is_empty());
    }

    #[test]
    fn test_extract_malformed_html() {
        let malformed = "<html><head><title>Test<title><body><p>hello";
        let page = sample_page(malformed);
        assert!(page.title.contains("Test"));
        assert!(page.body_text.contains("hello"));
    }

    #[test]
    fn test_extract_rtl_text() {
        let html = "<html><body><p>مرحبا   بك</p></body></html>";
        let page = sample_page(html);
        assert!(page.body_text.contains("مرحبا بك"));
    }

    #[test]
    fn test_extract_table() {
        let html = r#"
        <html><body>
        <table id="data">
            <tr><th>Company</th><th>Country</th></tr>
            <tr><td>Starz Electronics</td><td>TN</td></tr>
            <tr><td>Foxconn</td><td>TW</td></tr>
        </table>
        </body></html>
        "#;
        let rows = extract_table(html, "#data");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["Company", "Country"]);
        assert_eq!(rows[1], vec!["Starz Electronics", "TN"]);
        assert_eq!(rows[2], vec!["Foxconn", "TW"]);
    }

    #[test]
    fn test_extract_page_empty_html() {
        let page = sample_page("");
        assert_eq!(page.title, "");
        assert_eq!(page.description, "");
        assert!(page.body_text.is_empty());
    }

    #[test]
    fn test_extract_page_malformed_html() {
        let html = "<html><head><title>Broken<title></head><body><p>Test";
        let page = sample_page(html);
        assert!(page.title.contains("Broken"));
        assert!(page.body_text.contains("Test"));
    }

    // B104: Mixed-language content edge case
    #[test]
    fn test_extract_mixed_language_content() {
        let html = r#"<html><body>
            <p>Welcome to our factory. مرحبا بكم في مصنعنا.</p>
            <p>Nous offrons des services de fabrication électronique.</p>
            <p>电子制造服务</p>
        </body></html>"#;
        let page = sample_page(html);
        assert!(page.body_text.contains("Welcome"));
        assert!(page.body_text.contains("مرحبا"));
        assert!(page.body_text.contains("电子制造"));
        // Language detection should pick something valid
        assert!(!page.language.is_empty());
    }

    // B106: Only scripts/styles should yield empty body
    #[test]
    fn test_extract_page_scripts_only() {
        let html = r#"<html><head><title>Tracker</title></head>
        <body><script>var x = 1; document.write('hello');</script>
        <style>.cls { display: none; }</style></body></html>"#;
        let page = sample_page(html);
        // Body should be empty or near-empty since only scripts/styles
        assert!(page.body_text.len() < 10 || page.field_confidence.body_text < 0.5);
    }

    // B109: Confidence scores present
    #[test]
    fn test_field_confidence_scores() {
        let page = sample_page(SAMPLE_HTML);
        assert!(page.field_confidence.title > 0.0);
        assert!(page.field_confidence.description > 0.0);
        assert!(page.field_confidence.body_text > 0.0);
        assert!(page.field_confidence.language > 0.0);

        // Empty HTML should have zero confidence
        let empty = sample_page("");
        assert_eq!(empty.field_confidence.title, 0.0);
        assert_eq!(empty.field_confidence.body_text, 0.0);
    }

    // B105: Deduplicated emails
    #[test]
    fn test_extract_page_dedup_emails() {
        let html = r#"<html><body>
            <p>Contact a@b.com or a@b.com or x@y.com</p>
        </body></html>"#;
        let page = sample_page(html);
        assert_eq!(page.emails.len(), 2); // deduped
    }

    // Audit #90: recursive body walk — script/style/noscript subtrees are
    // skipped, text is collected in document order, and the walk is iterative
    // so hostile nesting cannot overflow the stack.
    #[test]
    fn test_extract_body_text_skips_script_style_noscript_subtrees() {
        let html = r#"<html><body>
            <p>visible one</p>
            <script>var hidden = "script text";</script>
            <style>.hidden { color: red; }</style>
            <noscript><p>noscript fallback text</p></noscript>
            <p>visible two</p>
        </body></html>"#;
        let page = sample_page(html);
        assert!(page.body_text.contains("visible one"));
        assert!(page.body_text.contains("visible two"));
        assert!(!page.body_text.contains("script text"));
        assert!(!page.body_text.contains("color: red"));
        assert!(!page.body_text.contains("noscript fallback"));
    }

    #[test]
    fn test_extract_body_text_preserves_document_order() {
        let html = r#"<html><body>
            <p>alpha <b>beta</b></p><div>gamma<span>delta</span></div>
        </body></html>"#;
        let page = sample_page(html);
        assert_eq!(page.body_text, "alpha beta gamma delta");
    }

    #[test]
    fn test_extract_body_text_deeply_nested_does_not_panic() {
        const DEPTH: usize = 20_000;
        let mut html = String::with_capacity(DEPTH * 11 + 64);
        html.push_str("<html><body>");
        html.push_str(&"<div>".repeat(DEPTH));
        html.push_str("deep marker");
        html.push_str(&"</div>".repeat(DEPTH));
        html.push_str("</body></html>");
        let page = sample_page(&html);
        assert!(
            page.body_text.contains("deep marker"),
            "deeply nested text was lost"
        );
    }

    #[test]
    fn test_extract_body_text_pathological_page_completes_with_correct_text() {
        let mut html = String::from("<html><body>");
        for i in 0..5_000 {
            html.push_str("<script>ignore</script><style>.a{}</style><p>item");
            html.push_str(&i.to_string());
            html.push_str("</p>");
        }
        html.push_str("<p>final marker</p></body></html>");
        let page = sample_page(&html);
        assert!(page.body_text.contains("item0 "));
        assert!(page.body_text.contains("item2500 "));
        assert!(page.body_text.contains("item4999"));
        assert!(page.body_text.ends_with("final marker"));
        assert!(!page.body_text.contains("ignore"));
        assert!(!page.body_text.contains(".a{}"));
    }

    #[test]
    fn test_extract_page_bytes_resolves_relative_links() {
        let html = b"<html><body><a href=\"/about\">About</a></body></html>";
        let base = page_url("https://starz.example.com/index.html");
        let page = extract_page_bytes(html, Some(&base)).expect("bytes HTML should parse");
        assert_eq!(page.links.len(), 1);
        assert_eq!(page.links[0].href, "https://starz.example.com/about");
    }
}
