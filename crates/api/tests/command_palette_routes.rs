//! Guard test: every navigation URL promoted by the command palette must be a
//! registered route in `app_router.rs`. The palette command list lives in
//! plain JavaScript, so without this check a route rename would silently leave
//! stale palette entries that 404 until someone exercises them in a browser.

const PALETTE_JS: &str = include_str!("../static/js/command-palette.js");
const APP_ROUTER_RS: &str = include_str!("../src/app_router.rs");

#[test]
fn command_palette_urls_resolve_to_registered_routes() {
    let urls = palette_urls(PALETTE_JS);
    assert!(
        urls.len() >= 20,
        "expected to parse the palette command list, found {} urls",
        urls.len()
    );

    for url in urls {
        let quoted = format!("\"{url}\"");
        assert!(
            APP_ROUTER_RS.contains(&quoted),
            "command palette URL {url} is not registered in app_router.rs"
        );
    }
}

#[test]
fn command_palette_url_parser_handles_expected_shapes() {
    let sample = r#"
    { id: "nav-companies", label: "Companies", url: "/companies" },
    { id: "action-theme", label: "Toggle theme", run: "toggle-theme" },
    "#;
    assert_eq!(palette_urls(sample), vec!["/companies".to_string()]);
}

fn palette_urls(source: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        let Some(marker) = line.find("url:") else {
            continue;
        };
        // Ignore lookalike keys such as `source_url:`.
        if marker > 0 {
            let previous = line.as_bytes()[marker - 1] as char;
            if previous.is_ascii_alphanumeric() || previous == '_' {
                continue;
            }
        }
        let rest = &line[marker + "url:".len()..];
        let Some(start) = rest.find('"') else {
            continue;
        };
        let rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else {
            continue;
        };
        let url = &rest[..end];
        if url.starts_with('/') {
            urls.push(url.to_string());
        }
    }
    urls
}
