pub mod api;
pub mod api_config;
pub mod app;
pub mod components;
pub mod routes;

#[cfg(target_arch = "wasm32")]
use leptos::*;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn mount() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <app::App /> });
}
