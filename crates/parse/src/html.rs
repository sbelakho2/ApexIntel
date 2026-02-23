use anyhow::Result;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::normalizer;

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedLink {
    pub text: String,
    pub href: String,
}

/// Extract structured content from raw HTML.
pub fn extract_page(html_content: &str) -> Result<PageContent> {
    let doc = Html::parse_document(html_content);

    let title = extract_title(&doc);
    let description = extract_meta_description(&doc);
    let body_text = extract_body_text(&doc);
    let links = extract_links(&doc);
    let emails = normalizer::extract_emails(&body_text);
    let phones = normalizer::extract_phones(&body_text);
    let language = crate::multilingual::detect_language(&body_text);

    Ok(PageContent {
        title,
        description,
        body_text,
        links,
        emails,
        phones,
        language,
    })
}

fn extract_title(doc: &Html) -> String {
    let sel = Selector::parse("title").unwrap();
    doc.select(&sel)
        .next()
        .map(|el| normalizer::normalize_whitespace(&el.text().collect::<String>()))
        .unwrap_or_default()
}

fn extract_meta_description(doc: &Html) -> String {
    let sel = Selector::parse(r#"meta[name="description"]"#).unwrap();
    doc.select(&sel)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| normalizer::normalize_whitespace(s))
        .unwrap_or_default()
}

fn extract_body_text(doc: &Html) -> String {
    // Remove script/style/nav/footer, then extract text from what remains
    let body_sel = Selector::parse("body").unwrap();
    let content_sel = Selector::parse("main, article, section, div, p, h1, h2, h3, h4, h5, h6, li, td, th, span, blockquote").unwrap();

    let body = doc.select(&body_sel).next();
    match body {
        Some(el) => {
            // Try extracting from content elements first
            let content_parts: Vec<String> = el
                .select(&content_sel)
                .flat_map(|e| e.text())
                .map(|t| t.to_string())
                .collect();

            let text = if content_parts.is_empty() {
                el.text().collect::<Vec<_>>().join(" ")
            } else {
                content_parts.join(" ")
            };
            normalizer::normalize_whitespace(&normalizer::remove_boilerplate(&text))
        }
        None => {
            // Fallback: get all text
            let text: String = doc.root_element().text().collect::<Vec<_>>().join(" ");
            normalizer::normalize_whitespace(&text)
        }
    }
}

fn extract_links(doc: &Html) -> Vec<ExtractedLink> {
    let sel = Selector::parse("a[href]").unwrap();
    doc.select(&sel)
        .filter_map(|el| {
            let href = el.value().attr("href")?.to_string();
            let text = normalizer::normalize_whitespace(&el.text().collect::<String>());
            if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
                return None;
            }
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
    let tr_sel = Selector::parse("tr").unwrap();
    let td_sel = Selector::parse("td, th").unwrap();

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

    #[test]
    fn test_extract_page_title() {
        let page = extract_page(SAMPLE_HTML).unwrap();
        assert_eq!(page.title, "Starz Electronics - EMS Manufacturer");
    }

    #[test]
    fn test_extract_page_description() {
        let page = extract_page(SAMPLE_HTML).unwrap();
        assert_eq!(page.description, "Leading EMS provider in Tunisia");
    }

    #[test]
    fn test_extract_page_body() {
        let page = extract_page(SAMPLE_HTML).unwrap();
        assert!(page.body_text.contains("SMT assembly"));
        assert!(page.body_text.contains("PCB manufacturing"));
    }

    #[test]
    fn test_extract_page_emails() {
        let page = extract_page(SAMPLE_HTML).unwrap();
        assert!(page.emails.contains(&"info@starz-electronics.com".to_string()));
    }

    #[test]
    fn test_extract_page_phones() {
        let page = extract_page(SAMPLE_HTML).unwrap();
        assert!(!page.phones.is_empty());
    }

    #[test]
    fn test_extract_page_links() {
        let page = extract_page(SAMPLE_HTML).unwrap();
        // Should exclude #top link
        let hrefs: Vec<&str> = page.links.iter().map(|l| l.href.as_str()).collect();
        assert!(hrefs.contains(&"https://starz-electronics.com/about"));
        assert!(hrefs.contains(&"https://starz-electronics.com/capabilities"));
        assert!(!hrefs.iter().any(|h| h.starts_with('#')));
    }

    #[test]
    fn test_extract_page_language() {
        let page = extract_page(SAMPLE_HTML).unwrap();
        assert_eq!(page.language, "en");
    }

    #[test]
    fn test_extract_by_selector() {
        let results = extract_by_selector(SAMPLE_HTML, "h1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], "Welcome to Starz Electronics");
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
}
