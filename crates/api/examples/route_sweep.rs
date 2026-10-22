//! Rust-native route sweep: authenticates, walks every navigation link, and
//! fails on any non-2xx page. Replaces the former Node smoke crawler so the
//! repository carries no JavaScript toolchain.
//!
//! Usage (server already running):
//!   BASE_URL=http://127.0.0.1:9095 ADMIN_USER=admin ADMIN_PASS=... \
//!     cargo run -p apex-api --example route_sweep

#![allow(clippy::unwrap_used, clippy::expect_used)] // CI harness must fail loudly.
#![allow(clippy::disallowed_methods)] // Loopback-only sweep of the just-built binary; not the crawler path.

use reqwest::redirect::Policy;
use std::collections::BTreeSet;
use std::time::Duration;

const CORE_PATHS: &[&str] = &[
    "/",
    "/executive",
    "/triage",
    "/activity",
    "/companies",
    "/competitors",
    "/persons",
    "/warnings",
    "/insights",
    "/trends",
    "/security",
    "/workspaces",
    "/graph",
    "/memos",
    "/pipeline",
    "/battlecards",
    "/recipes",
    "/admin",
];

#[tokio::main]
async fn main() {
    let base = std::env::var("BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:9095".into());
    let base = base.trim_end_matches('/').to_string();
    let user = std::env::var("ADMIN_USER").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("ADMIN_PASS").unwrap_or_else(|_| "adminpassword".into());

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(Policy::limited(5))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build client");

    let login = client
        .post(format!("{base}/login"))
        .form(&[("username", user.as_str()), ("password", pass.as_str())])
        .send()
        .await
        .expect("login request");
    if !(login.status().is_success() || login.status().is_redirection()) {
        eprintln!("route_sweep: login failed: {}", login.status());
        std::process::exit(1);
    }

    let home = client
        .get(format!("{base}/"))
        .send()
        .await
        .expect("dashboard request");
    if home.status() != reqwest::StatusCode::OK {
        eprintln!("route_sweep: dashboard returned {}", home.status());
        std::process::exit(1);
    }
    let html = home.text().await.unwrap_or_default();

    let mut paths: BTreeSet<String> = CORE_PATHS.iter().map(|p| (*p).to_string()).collect();
    for chunk in html.split("href=\"").skip(1) {
        let candidate = chunk.split('"').next().unwrap_or("");
        if let Some(rest) = candidate.strip_prefix('/') {
            if candidate.starts_with("//")
                || candidate.contains('#')
                || candidate.contains('?')
                || candidate.starts_with("/api/")
                || candidate.starts_with("/static/")
                || candidate.starts_with("/logout")
            {
                continue;
            }
            let path = format!("/{}", rest.split('#').next().unwrap_or(rest));
            if path.len() <= 160 {
                paths.insert(path);
            }
        }
    }

    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for path in &paths {
        let response = client.get(format!("{base}{path}")).send().await;
        match response {
            Ok(resp) => {
                checked += 1;
                let status = resp.status();
                let body_len = resp.bytes().await.map(|b| b.len()).unwrap_or(0);
                if !(status.is_success() || status.is_redirection()) {
                    failures.push(format!("{path} -> {status}"));
                } else if body_len < 256 {
                    failures.push(format!("{path} -> {status} but only {body_len} bytes"));
                }
            }
            Err(error) => failures.push(format!("{path} -> request error: {error}")),
        }
    }

    println!(
        "route_sweep: {checked} routes checked, {} failures",
        failures.len()
    );
    for failure in &failures {
        eprintln!("  {failure}");
    }
    if failures.is_empty() {
        println!("PASS: every navigation route rendered");
    } else {
        std::process::exit(1);
    }
}
