use leptos::*;

#[component]
pub fn PageHeader(
    title: &'static str,
    subtitle: &'static str,
    eyebrow: &'static str,
) -> impl IntoView {
    view! {
        <header class="page-header">
            <div>
                <div class="eyebrow">{eyebrow}</div>
                <h1 class="page-title">{title}</h1>
                <p class="page-subtitle">{subtitle}</p>
            </div>
        </header>
    }
}

#[component]
pub fn SurfaceCard(
    title: &'static str,
    #[prop(optional)] subtitle: Option<&'static str>,
    children: Children,
) -> impl IntoView {
    view! {
        <section class="apex-module">
            <div class="apex-module-header">
                <h2 class="apex-module-title">{title}</h2>
                {subtitle.map(|text| view! { <p class="apex-module-subtitle">{text}</p> })}
            </div>
            <div class="apex-module-body">{children()}</div>
        </section>
    }
}

#[component]
pub fn StatCard(
    label: &'static str,
    value: String,
    delta: String,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="apex-stat">
            <div class="apex-stat-rail"></div>
            <div class="apex-stat-body">
                <div class="stat-card-top">
                    <div>
                        <p class="apex-stat-label">{label}</p>
                        <p class="apex-stat-value">{value}</p>
                    </div>
                    <div class="apex-stat-icon">{children()}</div>
                </div>
                <div class="apex-stat-delta">{delta}</div>
            </div>
        </div>
    }
}
