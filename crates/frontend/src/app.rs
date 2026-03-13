use leptos::*;
use leptos_meta::*;
use leptos_router::*;

use crate::routes::{
    admin::AdminPage, adversarial::AdversarialPage, causality::CausalityPage, calibration::CalibrationPage,
    companies::CompaniesPage, company_detail::CompanyDetailPage, competitors::CompetitorsPage,
    graph::GraphPage, insights::InsightsPage, login::LoginPage, memos::MemosPage, overview::OverviewPage,
    person_detail::PersonDetailPage, persons::PersonsPage, recipes::RecipesPage,
    search::SearchPage, security::SecurityPage, settings::SettingsPage, timeline::TimelinePage,
    warnings::WarningsPage,
};

const NAV_ITEMS: [(&str, &str); 16] = [
    ("Overview", "/"),
    ("Warnings", "/warnings"),
    ("Insights", "/insights"),
    ("Companies", "/companies"),
    ("Persons", "/persons"),
    ("Search", "/search"),
    ("Memos", "/memos"),
    ("Calibration", "/calibration"),
    ("Graph", "/graph"),
    ("Competitors", "/competitors"),
    ("Security", "/security"),
    ("Recipes", "/recipes"),
    ("Settings", "/settings"),
    ("Admin", "/admin"),
    ("Causality", "/causality"),
    ("Timeline", "/entities/demo/timeline"),
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
        return "Overview";
    }

    NAV_ITEMS
        .iter()
        .find(|(_, href)| {
            normalized == *href || normalized.starts_with(&format!("{href}/"))
        })
        .map(|(label, _)| *label)
        .unwrap_or("ApexIntel")
}

fn nav_link_class(current_path: &str, href: &str) -> String {
    let normalized = normalize_route_path(current_path);
    let active = if href == "/" {
        normalized == "/"
    } else {
        normalized == href || normalized.starts_with(&format!("{href}/"))
    };

    if active {
        "nav-link nav-link-active".to_string()
    } else {
        "nav-link".to_string()
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let nav_open = create_rw_signal(false);
    let location = use_location();

    view! {
        <Stylesheet id="apex-frontend-style" href="/wasm/style.css" />
        <Title text="ApexIntel WASM Frontend" />
        <Router base="/wasm">
            <div class="app-shell">
                <header class="mobile-topbar">
                    <div>
                        <div class="nav-title">ApexIntel WASM</div>
                        <div class="mobile-route-label">{move || route_label(&location.pathname.get())}</div>
                    </div>
                    <button
                        type="button"
                        class="nav-toggle"
                        aria-label="Toggle navigation"
                        aria-expanded=move || nav_open.get().to_string()
                        on:click=move |_| nav_open.update(|open| *open = !*open)
                    >
                        {move || if nav_open.get() { "Close" } else { "Menu" }}
                    </button>
                </header>

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

                <aside class=move || {
                    if nav_open.get() {
                        "site-nav site-nav-open"
                    } else {
                        "site-nav"
                    }
                }>
                    <div class="site-nav-header">
                        <div class="nav-title">ApexIntel WASM</div>
                        <p class="site-nav-copy">Interactive intelligence views with responsive routing and graph analysis.</p>
                    </div>
                    <nav class="nav-links">
                        {move || {
                            let current_path = location.pathname.get();
                            NAV_ITEMS
                                .iter()
                                .map(|(label, href)| {
                                    let path_snapshot = current_path.clone();
                                    view! {
                                        <A
                                            href=*href
                                            class=move || nav_link_class(&path_snapshot, href)
                                            on:click=move |_| nav_open.set(false)
                                        >
                                            {label.to_string()}
                                        </A>
                                    }
                                })
                                .collect_view()
                        }}
                        <A
                            href="/adversarial"
                            class=move || nav_link_class(&location.pathname.get(), "/adversarial")
                            on:click=move |_| nav_open.set(false)
                        >
                            "Adversarial"
                        </A>
                    </nav>
                </aside>

                <main class="app-main">
                    <Routes base="/wasm".to_string()>
                        <Route path="/" view=OverviewPage />
                        <Route path="login" view=LoginPage />
                        <Route path="warnings" view=WarningsPage />
                        <Route path="insights" view=InsightsPage />
                        <Route path="companies" view=CompaniesPage />
                        <Route path="companies/:company_id" view=CompanyDetailPage />
                        <Route path="persons" view=PersonsPage />
                        <Route path="persons/:person_id" view=PersonDetailPage />
                        <Route path="search" view=SearchPage />
                        <Route path="memos" view=MemosPage />
                        <Route path="calibration" view=CalibrationPage />
                        <Route path="graph" view=GraphPage />
                        <Route path="competitors" view=CompetitorsPage />
                        <Route path="security" view=SecurityPage />
                        <Route path="recipes" view=RecipesPage />
                        <Route path="settings" view=SettingsPage />
                        <Route path="admin" view=AdminPage />
                        <Route path="causality" view=CausalityPage />
                        <Route path="entities/:entity_id/timeline" view=TimelinePage />
                        <Route path="adversarial" view=AdversarialPage />
                    </Routes>
                </main>
            </div>
        </Router>
    }
}