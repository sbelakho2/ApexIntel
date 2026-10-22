//! Rust-native UI contract tests.
//!
//! These replace the former Node/Playwright UI checks with compile-time
//! template assertions: navigation groups and links, route wiring, the mobile
//! tab bar, the search and user-menu hooks, the warnings bulk-dismiss action,
//! the no-sticky-header rule, and the committed stylesheets. No browser, no
//! Node, no database.
#![allow(clippy::unwrap_used, clippy::expect_used)]

const BASE: &str = include_str!("../templates/base.html");
const WARNINGS: &str = include_str!("../templates/pages/warnings.html");
const ROUTES: &str = include_str!("../src/web/routes.rs");
const TAILWIND_CSS: &[u8] = include_bytes!("../static/css/tailwind.css");
const SENSEI_CSS: &[u8] = include_bytes!("../static/css/sensei-rams.css");
const APEX_CSS: &[u8] = include_bytes!("../static/css/apex-rams.css");

fn primary_nav_block() -> &'static str {
    let start = BASE
        .find("{% macro primary_nav() %}")
        .expect("primary_nav macro");
    let end = BASE[start..]
        .find("{% endmacro %}")
        .map(|offset| start + offset)
        .expect("primary_nav endmacro");
    &BASE[start..end]
}

#[test]
fn navigation_exposes_the_six_workflow_groups() {
    for group in [
        "Command Center",
        "Entities",
        "Signals",
        "Investigations",
        "Sales Intelligence",
        "Automations",
    ] {
        assert!(
            BASE.contains(&format!("nav_group(\"{group}\")")),
            "nav group missing: {group}"
        );
    }
}

#[test]
fn every_navigation_link_resolves_to_a_registered_get_route() {
    let nav = primary_nav_block();
    let mut checked = 0usize;
    for pattern in ["nav_item(\"", "nav_item_badge(\""] {
        let mut rest = nav;
        while let Some(start) = rest.find(pattern) {
            rest = &rest[start + pattern.len()..];
            let path = rest.split('"').next().unwrap_or("");
            let path = path.split('#').next().unwrap_or(path);
            if path.is_empty() {
                continue;
            }
            checked += 1;
            assert!(
                ROUTES.contains(&format!(".get(\"{path}\""))
                    || ROUTES.contains(&format!(".admin_get(\"{path}\"")),
                "navigation link {path} has no GET route in web/routes.rs"
            );
        }
    }
    assert!(
        checked >= 15,
        "expected the full nav, found {checked} links"
    );
}

#[test]
fn mobile_quick_navigation_keeps_its_five_tabs() {
    assert!(BASE.contains("aria-label=\"Mobile quick navigation\""));
    assert!(BASE.contains("apex-mobile-tabbar"));
    assert!(BASE.contains("apex-tab-indicator"));
    for label in ["Home", "Signals", "Entities", "Triage", "More"] {
        assert!(
            BASE.contains(&format!(">{label}<")) || BASE.contains(&format!("\"{label}\"")),
            "mobile tab label missing: {label}"
        );
    }
}

#[test]
fn shell_hooks_used_by_scripts_and_sessions_exist() {
    assert!(BASE.contains("action=\"/search\""), "search form missing");
    assert!(BASE.contains("name=\"q\""), "search input missing");
    assert!(BASE.contains("apex-user-menu"), "user menu missing");
    assert!(
        BASE.contains("href=\"/settings\""),
        "user menu settings link missing"
    );
    assert!(
        BASE.contains("aria-label=\"Sections\""),
        "primary nav landmark missing"
    );
}

#[test]
fn the_command_bar_never_sticks_over_content() {
    // Regression for the reported global bug: the top bar covered page
    // content on every screen while scrolling.
    assert!(
        !BASE.contains("sticky top-0"),
        "header must not be position:sticky"
    );
    assert!(
        BASE.contains("<header class=\"bg-rams-chassis"),
        "header should render as normal flow"
    );
}

#[test]
fn templates_carry_no_inline_style_blocks() {
    assert!(!BASE.contains("<style>"), "base.html must not inline CSS");
    assert!(
        BASE.contains("sensei-rams.css") && BASE.contains("apex-rams.css"),
        "Sensei system stylesheets must be linked"
    );
}

#[test]
fn warnings_page_exposes_single_and_bulk_dismissal() {
    assert!(
        WARNINGS.contains("hx-post=\"/warnings/acknowledge-all\""),
        "bulk acknowledge action missing"
    );
    assert!(
        WARNINGS.contains("Acknowledge all"),
        "bulk acknowledge button missing"
    );
    assert!(
        WARNINGS.contains("/warnings/{{ w.id }}/acknowledge"),
        "per-warning dismiss action missing"
    );
    assert!(
        ROUTES.contains("\"/warnings/acknowledge-all\""),
        "bulk acknowledge route missing"
    );
}

#[test]
fn committed_stylesheets_are_present_and_substantial() {
    assert!(TAILWIND_CSS.len() > 50_000, "tailwind.css looks truncated");
    assert!(
        SENSEI_CSS.len() > 100_000,
        "sensei-rams.css looks truncated"
    );
    assert!(APEX_CSS.len() > 5_000, "apex-rams.css looks truncated");
}
