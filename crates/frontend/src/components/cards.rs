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
        <section class="surface-card">
            <div class="surface-card-header">
                <h2 class="surface-card-title">{title}</h2>
                {subtitle.map(|text| view! { <p class="surface-card-subtitle">{text}</p> })}
            </div>
            <div class="surface-card-body">{children()}</div>
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
        <div class="surface-card stat-card">
            <div class="surface-card-body">
                <div class="stat-card-top">
                    <div>
                        <p class="stat-label">{label}</p>
                        <p class="stat-value">{value}</p>
                    </div>
                    {children()}
                </div>
                <div class="stat-delta">{delta}</div>
            </div>
        </div>
    }
}
