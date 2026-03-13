use leptos::*;
use leptos::ev::SubmitEvent;

use crate::{api, components::cards::{PageHeader, SurfaceCard}};

#[component]
pub fn LoginPage() -> impl IntoView {
    let (username, set_username) = create_signal(String::new());
    let (password, set_password) = create_signal(String::new());
    let (error, set_error) = create_signal(String::new());
    let (submitting, set_submitting) = create_signal(false);

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        set_error.set(String::new());
        set_submitting.set(true);

        let username_value = username.get_untracked();
        let password_value = password.get_untracked();

        spawn_local(async move {
            match api::submit_login(&username_value, &password_value).await {
                Ok(_destination) => {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().set_href(&_destination);
                    }
                }
                Err(message) => set_error.set(message),
            }
            set_submitting.set(false);
        });
    };

    view! {
        <div class="page">
            <PageHeader eyebrow="Authentication" title="Login" subtitle="The WASM login route submits directly to the existing `/login` handler and surfaces backend credential errors inline." />
            <SurfaceCard title="Sign In" subtitle="Use the same credentials as the Askama login form.">
                <form class="login-form" on:submit=submit>
                    <label class="form-field">
                        <span class="filter-bar-title">"Username"</span>
                        <input class="search-input" type="text" prop:value=username on:input=move |ev| set_username.set(event_target_value(&ev)) />
                    </label>
                    <label class="form-field">
                        <span class="filter-bar-title">"Password"</span>
                        <input class="search-input" type="password" prop:value=password on:input=move |ev| set_password.set(event_target_value(&ev)) />
                    </label>
                    <Show when=move || !error.get().is_empty()>
                        <p class="error-copy">{error}</p>
                    </Show>
                    <button type="submit" class="pagination-button" disabled=submitting>
                        {move || if submitting.get() { "Signing In..." } else { "Sign In" }}
                    </button>
                </form>
            </SurfaceCard>
        </div>
    }
}