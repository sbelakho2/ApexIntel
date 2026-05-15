use anyhow::Result;
use encoding_rs::Encoding;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use tracing::instrument;

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
#[instrument(skip(html_content))]
pub fn extract_page(html_content: &str) -> Result<PageContent> {
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

    let links = extract_links(&doc);
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

    Ok(PageContent {
        title,
        description,
        body_text: effective_body,
        links,
        emails,
        phones,
        language,
        field_confidence,
    })
}

/// Extract structured content from raw HTML bytes.
/// Falls back to lossy decoding if UTF-8 decoding fails.
pub fn extract_page_bytes(html_bytes: &[u8]) -> Result<PageContent> {
    let html = if let Ok(s) = std::str::from_utf8(html_bytes) {
        s.to_string()
    } else {
        let encoding = detect_charset(html_bytes).unwrap_or(encoding_rs::UTF_8);
        let (decoded, _, _) = encoding.decode(html_bytes);
        decoded.to_string()
    };
    extract_page(&html)
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

fn extract_body_text(doc: &Html) -> String {
    // Skip <script>, <style>, and <noscript> elements to avoid contaminating body text
    // with JavaScript code, CSS rules, or fallback content.
    let body_sel =
        Selector::parse("body").unwrap_or_else(|error| panic!("invalid body selector: {error:?}"));
    let skip_sel = Selector::parse("script, style, noscript")
        .unwrap_or_else(|error| panic!("invalid script/style selector: {error:?}"));

    match doc.select(&body_sel).next() {
        Some(body) => {
            // Collect IDs of elements to skip (use ego_tree::NodeId via type inference)
            let skip_ids: std::collections::HashSet<_> =
                body.select(&skip_sel).map(|el| el.id()).collect();

            // Collect text from nodes not dominated by skip elements
            let mut parts = Vec::new();
            for node_ref in body.descendants() {
                if let scraper::node::Node::Text(ref t) = node_ref.value() {
                    // Check if any ancestor is a skipped element
                    let dominated = node_ref.ancestors().any(|a| skip_ids.contains(&a.id()));
                    if !dominated {
                        parts.push(t.text.as_ref());
                    }
                }
            }
            let text = parts.join(" ");
            let normalized =
                normalizer::normalize_whitespace(&normalizer::remove_boilerplate(&text));
            if normalized.is_empty() {
                let fallback = doc.root_element().text().collect::<Vec<_>>().join(" ");
                normalizer::normalize_whitespace(&normalizer::remove_boilerplate(&fallback))
            } else {
                normalized
            }
        }
        None => {
            let text: String = doc.root_element().text().collect::<Vec<_>>().join(" ");
            normalizer::normalize_whitespace(&text)
        }
    }
}

fn extract_links(doc: &Html) -> Vec<ExtractedLink> {
    let sel = Selector::parse("a[href]")
        .unwrap_or_else(|error| panic!("invalid link selector: {error:?}"));
    doc.select(&sel)
        .filter_map(|el| {
            let href = el.value().attr("href")?.to_string();
            let text = normalizer::normalize_whitespace(&el.text().collect::<String>());
            if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
                return None;
            }
            let normalized = normalize_url(&href).unwrap_or(href);
            Some(ExtractedLink {
                text,
                href: normalized,
            })
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
        extract_page(html).unwrap_or_else(|error| panic!("sample HTML should parse: {error}"))
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
}
