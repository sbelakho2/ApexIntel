use leptos::*;
use leptos_meta::*;
use leptos_router::*;
use wasm_bindgen::prelude::Closure;
use wasm_bindgen::JsCast;

use crate::routes::{
    activity::ActivityPage,
    admin::AdminPage,
    adversarial::AdversarialPage,
    analyst::AnalystPage,
    battlecards::BattlecardsPage,
    calibration::CalibrationPage,
    causality::CausalityPage,
    companies::CompaniesPage,
    company_detail::CompanyDetailPage,
    competitive_landscape::CompetitiveLandscapePage,
    competitors::CompetitorsPage,
    executive::ExecutivePage,
    graph::GraphPage,
    insights::InsightsPage,
    login::LoginPage,
    memos::MemosPage,
    operational::OperationalPage,
    overview::OverviewPage,
    person_detail::PersonDetailPage,
    persons::PersonsPage,
    psych_profiles::PsychProfilesPage,
    recipes::RecipesPage,
    search::SearchPage,
    security::SecurityPage,
    settings::SettingsPage,
    strategic_radar::StrategicRadarPage,
    supply_risk::SupplyRiskPage,
    threat_intel::ThreatIntelPage,
    timeline::TimelinePage,
    trends::TrendsPage,
    warnings::WarningsPage,
};

/// Organized navigation groups matching server-rendered rack sidebar.
const NAV_GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Core", &[
        ("Dashboard", "/wasm/"),
        ("Warnings", "/wasm/warnings"),
        ("Insights", "/wasm/insights"),
    ]),
    ("Intelligence", &[
        ("Companies", "/wasm/companies"),
        ("Persons", "/wasm/persons"),
        ("Executive", "/wasm/executive"),
        ("Analyst", "/wasm/analyst"),
        ("Search", "/wasm/search"),
    ]),
    ("Operations", &[
        ("Operations", "/wasm/operational"),
        ("Activity", "/wasm/activity"),
        ("Calibration", "/wasm/calibration"),
        ("Graph", "/wasm/graph"),
    ]),
    ("Analysis", &[
        ("Trends", "/wasm/trends"),
        ("Adversarial", "/wasm/adversarial"),
        ("Causality", "/wasm/causality"),
        ("Timeline", "/wasm/entities/demo/timeline"),
    ]),
    ("Competitive", &[
        ("Competitors", "/wasm/competitors"),
        ("Landscape", "/wasm/competitive-landscape"),
        ("Strategic Radar", "/wasm/strategic-radar"),
        ("Battlecards", "/wasm/battlecards"),
        ("Supply Chain", "/wasm/supply-risk"),
    ]),
    ("Content", &[
        ("Memos", "/wasm/memos"),
        ("Recipes", "/wasm/recipes"),
        ("Security", "/wasm/security"),
        ("Threat Intel", "/wasm/threat-intel"),
        ("Psych Profiles", "/wasm/psych-profiles"),
    ]),
    ("System", &[
        ("Settings", "/wasm/settings"),
        ("Admin", "/wasm/admin"),
    ]),
];

fn normalize_route_path(path: &str) -> String {
    let normalized = path.strip_prefix("/wasm").unwrap_or(path);
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized.to_string()
    }
}

fn route_label(path: &str) -> &'static str {
    let normalized = normalize_route_path(path);
    if normalized == "/" {
        return "Dashboard";
    }

    for (_, items) in NAV_GROUPS {
        for (label, href) in *items {
            let href_norm = normalize_route_path(href);
            if normalized == href_norm || normalized.starts_with(&format!("{href_norm}/")) {
                return label;
            }
        }
    }
    "ApexIntel"
}

fn nav_link_class(current_path: &str, href: &str) -> String {
    let normalized = normalize_route_path(current_path);
    let href_norm = normalize_route_path(href);
    let active = if href_norm == "/" {
        normalized == "/"
    } else {
        normalized == href_norm || normalized.starts_with(&format!("{href_norm}/"))
    };

    if active {
        "nav-link nav-link-active".to_string()
    } else {
        "nav-link".to_string()
    }
}

fn nav_aria_current(current_path: &str, href: &str) -> Option<&'static str> {
    let normalized = normalize_route_path(current_path);
    let href_norm = normalize_route_path(href);
    let active = if href_norm == "/" {
        normalized == "/"
    } else {
        normalized == href_norm || normalized.starts_with(&format!("{href_norm}/"))
    };
    if active { Some("page") } else { None }
}

/// Initializes dark mode from localStorage or system preference.
fn init_dark_mode() {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let html = document.document_element().unwrap();

        // Check localStorage first
        let stored = window
            .local_storage()
            .ok()
            .flatten()
            .and_then(|s| s.get_item("apex-theme").ok())
            .flatten();

        match stored.as_deref() {
            Some("dark") => {
                let _ = html.class_list().add_1("dark");
            }
            Some("light") => {
                let _ = html.class_list().remove_1("dark");
            }
            _ => {
                // Check system preference
                let prefers_dark = window
                    .match_media("(prefers-color-scheme: dark)")
                    .ok()
                    .flatten()
                    .map(|m| m.matches())
                    .unwrap_or(false);
                if prefers_dark {
                    let _ = html.class_list().add_1("dark");
                }
            }
        }
    }
}

/// Toggles dark mode and persists preference to localStorage.
fn toggle_dark_mode() {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let html = document.document_element().unwrap();

        if html.class_list().contains("dark") {
            let _ = html.class_list().remove_1("dark");
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item("apex-theme", "light");
            }
        } else {
            let _ = html.class_list().add_1("dark");
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item("apex-theme", "dark");
            }
        }
    }
}

/// Returns whether the current theme is dark.
fn is_dark() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let html = document.document_element().unwrap();
        return html.class_list().contains("dark");
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    // Initialize dark mode on mount
    let _ = create_effect(move |_| {
        init_dark_mode();
    });

    view! {
        <Stylesheet id="apex-frontend-style" href="/wasm/style.css" />
        <Title text="ApexIntel — Supply Chain Intelligence" />
        <Router trailing_slash=TrailingSlash::Exact>
            <AppShell />
        </Router>
    }
}

#[component]
fn AppShell() -> impl IntoView {
    let nav_open = create_rw_signal(false);
    let location = use_location();
    let dark = create_rw_signal(is_dark());

    // Listen for theme changes from other tabs/windows
    #[cfg(target_arch = "wasm32")]
    {
        let dark_clone = dark;
        create_effect(move |_| {
            let window = web_sys::window().unwrap();
            let callback = Closure::wrap(Box::new(move |_: web_sys::StorageEvent| {
                dark_clone.set(is_dark());
            }) as Box<dyn FnMut(web_sys::StorageEvent)>);
            let closure_fn: &js_sys::Function = callback.as_ref().unchecked_ref();
            let _ = window.add_event_listener_with_callback("storage", closure_fn);
            callback.forget();
        });
    }

    let theme_icon = move || {
        if dark.get() { "☀" } else { "☾" }
    };

    let theme_label = move || {
        if dark.get() { "Switch to light mode" } else { "Switch to dark mode" }
    };

    let on_theme_toggle = move |_| {
        toggle_dark_mode();
        dark.set(is_dark());
    };

    view! {
        <a href="#main-content" class="skip-link">Skip to main content</a>
        <div class="app-shell">
            {/* ── Header Strip ─────────────────────────────────────── */ }
            <header class="app-header">
                <div class="header-breadcrumb">
                    <span>ApexIntel</span>
                    <span>/</span>
                    <span class="header-breadcrumb-current">
                        {move || route_label(&location.pathname.get())}
                    </span>
                </div>
                <div class="header-actions">
                    <button
                        type="button"
                        class="theme-toggle"
                        aria-label=theme_label
                        title=theme_label
                        on:click=on_theme_toggle
                    >
                        {theme_icon}
                    </button>
                </div>
            </header>

            {/* ── Mobile Topbar ────────────────────────────────────── */ }
            <header class="mobile-topbar">
                <div>
                    <div class="mobile-nav-title">ApexIntel</div>
                    <div class="mobile-route-label" aria-live="polite">
                        {move || route_label(&location.pathname.get())}
                    </div>
                </div>
                <div style="display: flex; gap: 0.5rem; align-items: center;">
                    <button
                        type="button"
                        class="theme-toggle"
                        aria-label=theme_label
                        title=theme_label
                        on:click=on_theme_toggle
                    >
                        {theme_icon}
                    </button>
                    <button
                        type="button"
                        class="nav-toggle"
                        aria-label="Toggle navigation"
                        aria-expanded=move || nav_open.get().to_string()
                        on:click=move |_| nav_open.update(|open| *open = !*open)
                    >
                        {move || if nav_open.get() { "✕" } else { "☰" }}
                    </button>
                </div>
            </header>

            {/* ── Mobile Nav Backdrop ──────────────────────────────── */ }
            <button
                type="button"
                class=move || {
                    if nav_open.get() {
                        "nav-backdrop nav-backdrop-open"
                    } else {
                        "nav-backdrop"
                    }
                }
                aria-label="Close navigation"
                on:click=move |_| nav_open.set(false)
            ></button>

            {/* ── Sidebar Navigation (Rack-style) ──────────────────── */ }
            <aside class=move || {
                if nav_open.get() {
                    "site-nav site-nav-open"
                } else {
                    "site-nav"
                }
            }>
                <div class="site-nav-header">
                    <div class="nav-title">ApexIntel</div>
                    <p class="site-nav-copy">Supply chain intelligence platform</p>
                </div>
                <nav class="nav-links" aria-label="Main navigation">
                    {NAV_GROUPS
                        .iter()
                        .flat_map(|(group_name, items)| {
                            let mut views: Vec<View> = Vec::new();

                            // Group header
                            views.push(view! {
                                <div class="nav-group-label">{group_name.to_string()}</div>
                            }.into_view());

                            // Nav items
                            for (label, href) in *items {
                                let href = *href;
                                views.push(view! {
                                    <A
                                        href=href
                                        class=move || nav_link_class(&location.pathname.get(), href)
                                        attr:aria-current=move || nav_aria_current(&location.pathname.get(), href)
                                        on:click=move |_| nav_open.set(false)
                                    >
                                        {label.to_string()}
                                    </A>
                                }.into_view());
                            }

                            views
                        })
                        .collect_view()}
                </nav>
            </aside>

            {/* ── Main Content ─────────────────────────────────────── */ }
            <main id="main-content" class="app-main" tabindex="-1">
                <Routes>
                    <Route path="/wasm/" view=OverviewPage />
                    <Route path="/wasm/activity" view=ActivityPage />
                    <Route path="/wasm/login" view=LoginPage />
                    <Route path="/wasm/warnings" view=WarningsPage />
                    <Route path="/wasm/insights" view=InsightsPage />
                    <Route path="/wasm/companies" view=CompaniesPage />
                    <Route path="/wasm/companies/:company_id" view=CompanyDetailPage />
                    <Route path="/wasm/persons" view=PersonsPage />
                    <Route path="/wasm/persons/:person_id" view=PersonDetailPage />
                    <Route path="/wasm/executive" view=ExecutivePage />
                    <Route path="/wasm/analyst" view=AnalystPage />
                    <Route path="/wasm/operational" view=OperationalPage />
                    <Route path="/wasm/search" view=SearchPage />
                    <Route path="/wasm/memos" view=MemosPage />
                    <Route path="/wasm/calibration" view=CalibrationPage />
                    <Route path="/wasm/graph" view=GraphPage />
                    <Route path="/wasm/competitors" view=CompetitorsPage />
                    <Route path="/wasm/security" view=SecurityPage />
                    <Route path="/wasm/recipes" view=RecipesPage />
                    <Route path="/wasm/settings" view=SettingsPage />
                    <Route path="/wasm/admin" view=AdminPage />
                    <Route path="/wasm/causality" view=CausalityPage />
                    <Route path="/wasm/entities/:entity_id/timeline" view=TimelinePage />
                    <Route path="/wasm/competitive-landscape" view=CompetitiveLandscapePage />
                    <Route path="/wasm/trends" view=TrendsPage />
                    <Route path="/wasm/strategic-radar" view=StrategicRadarPage />
                    <Route path="/wasm/adversarial" view=AdversarialPage />
                    <Route path="/wasm/battlecards" view=BattlecardsPage />
                    <Route path="/wasm/supply-risk" view=SupplyRiskPage />
                    <Route path="/wasm/threat-intel" view=ThreatIntelPage />
                    <Route path="/wasm/psych-profiles" view=PsychProfilesPage />
                </Routes>
            </main>
        </div>
    }
}
