use leptos::*;

#[component]
pub fn FilterBar(title: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="filter-bar">
            <div class="filter-bar-title">{title}</div>
            <div class="filter-chip-row">{children()}</div>
        </section>
    }
}

#[component]
pub fn FilterChip(#[prop(into)] label: String, #[prop(into)] active: MaybeSignal<bool>, on_click: Callback<()>) -> impl IntoView {
    view! {
        <button
            type="button"
            class=move || {
                if active.get() {
                    "filter-chip filter-chip-active"
                } else {
                    "filter-chip"
                }
            }
            on:click=move |_| on_click.call(())
        >
            {label}
        </button>
    }
}

#[component]
pub fn Pagination(page: ReadSignal<u32>, total: u64, per_page: u32, set_page: WriteSignal<u32>) -> impl IntoView {
    let total_pages = move || ((total + per_page as u64).saturating_sub(1) / per_page as u64).max(1) as u32;

    view! {
        <div class="pagination-row">
            <button
                type="button"
                class="pagination-button"
                disabled=move || page.get() <= 1
                on:click=move |_| set_page.update(|value| *value = value.saturating_sub(1).max(1))
            >
                "Previous"
            </button>
            <span class="pagination-label">
                {move || format!("Page {} of {}", page.get(), total_pages())}
            </span>
            <button
                type="button"
                class="pagination-button"
                disabled=move || page.get() >= total_pages()
                on:click=move |_| set_page.update(|value| *value = (*value + 1).min(total_pages()))
            >
                "Next"
            </button>
        </div>
    }
}