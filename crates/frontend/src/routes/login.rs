use leptos::*;

use crate::components::cards::{PageHeader, SurfaceCard};

/// Login page component.
///
/// SECURITY: This component does NOT handle credentials in WASM to avoid
/// exposing passwords in the compiled WASM bundle. Instead, it redirects
/// to the server-rendered login endpoint which handles authentication
/// securely via the Askama template system.
///
/// Previously, this component accepted username/password directly and
/// called `api::submit_login()`, which embedded credential handling in
/// the WASM binary — making it reverse-engineerable from the bundle.
#[component]
pub fn LoginPage() -> impl IntoView {
    // Redirect to server-side login page on mount
    #[cfg(target_arch = "wasm32")]
    {
        let current_path = web_sys::window()
            .map(|window| window.location().pathname().unwrap_or_default())
            .unwrap_or_default();
        // Only redirect if we're actually on the /login page (not SSR)
        if current_path == "/login" || current_path.ends_with("/login") {
            // Use server-side login via normal GET navigation.
            let _ = web_sys::window().map(|window| window.location().set_href("/login"));
        }
    }

    view! {
        <div class="page">
            <PageHeader
                eyebrow="Authentication"
                title="Login"
                subtitle="Redirecting to secure server-side login..."
            />
            <SurfaceCard title="Sign In" subtitle="You are being redirected to the secure login page.">
                <div class="login-redirect">
                    <p class="muted-copy">
                        "For security, login is handled server-side. "
                        <a href="/login" class="inline-link">"Click here if not redirected."</a>
                    </p>
                </div>
            </SurfaceCard>
        </div>
    }
}
