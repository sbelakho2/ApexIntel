use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::intelligence_ingress::{IntelligenceIngress, NewWarning};
use crate::*;

pub(super) async fn run_breach_scan(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let hibp_key = std::env::var("HIBP_API_KEY").ok();
    let intelx_key = std::env::var("INTELX_API_KEY").ok();
    let pastebin_key = std::env::var("PASTEBIN_API_DEV_KEY").ok();

    // Load domains from env var first, fall back to DB companies
    let domains_raw = std::env::var("MONITORED_DOMAINS").unwrap_or_default();
    let mut domains: Vec<String> = domains_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if domains.is_empty() {
        let companies = store
            .list_companies(
                &apex_store::postgres::CompanyListFilters {
                    regions: vec![],
                    search: None,
                    is_competitor: None,
                },
                Some(apex_store::postgres::CompanyOrderBy::Name),
                false,
                500,
                0,
            )
            .await
            .unwrap_or_default();
        domains = companies
            .iter()
            .filter_map(|c| c.domain.clone())
            .filter(|d| !d.is_empty())
            .collect();
    }

    if domains.is_empty() {
        run.skip(
            "breach_scan: no domains found (neither in MONITORED_DOMAINS nor in companies table)",
        );
        return run;
    }

    let monitor = match BreachMonitor::new(hibp_key, intelx_key, pastebin_key) {
        Ok(m) => m,
        Err(e) => {
            run.fail(&format!("breach_scan: failed to build monitor: {e}"));
            return run;
        }
    };
    let mut total_hits: u64 = 0;

    for domain in &domains {
        let events = monitor.full_domain_exposure_check(domain).await;
        let count = events.len() as u64;
        if count > 0 {
            tracing::warn!(
                domain = %domain,
                breach_count = count,
                "breach_scan: domain has known breaches"
            );
            total_hits += count;
            let breach_urls: Vec<String> =
                events.iter().filter_map(|e| e.source_url.clone()).collect();
            let title = format!("Domain breach exposure: {domain}");
            let description = format!(
                "{count} breach event(s) detected for domain '{domain}'. Immediate review recommended."
            );
            let severity = if count > 5 { "critical" } else { "high" };
            if let Err(error) = ingress
                .submit_warning(
                    NewWarning::new("breach", &title, severity)
                        .description(&description)
                        .source_urls(breach_urls)
                        .confidence(0.9),
                )
                .await
            {
                tracing::warn!(%error, domain = %domain, "breach_scan: failed to ingest warning");
            }
        } else {
            tracing::info!(domain = %domain, "breach_scan: clean");
        }
    }

    run.succeed(
        total_hits,
        &format!(
            "scanned {} domain(s): {} breach events found",
            domains.len(),
            total_hits,
        ),
    );
    run
}

pub(super) async fn run_sanctions_screen(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    // Load entity names from env var first, fall back to DB companies + persons
    let entities_raw = std::env::var("MONITORED_ENTITIES").unwrap_or_default();
    let mut entity_names: Vec<String> = entities_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if entity_names.is_empty() {
        let companies = store
            .list_companies(
                &apex_store::postgres::CompanyListFilters {
                    regions: vec![],
                    search: None,
                    is_competitor: None,
                },
                Some(apex_store::postgres::CompanyOrderBy::Name),
                false,
                500,
                0,
            )
            .await
            .unwrap_or_default();
        entity_names.extend(companies.iter().map(|c| c.name.clone()));

        let persons = store
            .list_persons(
                &apex_store::postgres::PersonListFilters {
                    regions: vec![],
                    roles: vec![],
                    search: None,
                    min_priority: None,
                    max_priority: None,
                },
                Some(apex_store::postgres::PersonOrderBy::Name),
                false,
                500,
                0,
            )
            .await
            .unwrap_or_default();
        entity_names.extend(persons.iter().map(|p| p.name.clone()));
    }

    if entity_names.is_empty() {
        run.skip("sanctions_screen: no entities found (neither in MONITORED_ENTITIES nor in DB)");
        return run;
    }

    let threshold: f64 = std::env::var("SANCTIONS_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.92);

    let screener = match SanctionsScreener::load_from_web().await {
        Ok(s) => s.with_threshold(threshold),
        Err(e) => {
            run.fail(&format!(
                "sanctions_screen: failed to load sanctions lists: {e}"
            ));
            return run;
        }
    };
    tracing::info!(
        entries = screener.entry_count(),
        "sanctions_screen: lists loaded"
    );

    let mut total_hits: u64 = 0;

    for name in &entity_names {
        let matches = screener.screen_entity(name, &[]);
        if matches.is_empty() {
            tracing::debug!(entity = %name, "sanctions_screen: no match");
        } else {
            total_hits += matches.len() as u64;
            for m in &matches {
                tracing::warn!(
                    entity = %name,
                    matched = %m.matched_name,
                    similarity = m.similarity,
                    list = ?m.list,
                    is_exact = m.is_exact,
                    "sanctions_screen: MATCH FOUND"
                );
                let title = format!("Sanctions match: {name} → {}", m.matched_name);
                let description = format!(
                    "Entity '{}' matched sanctions entry '{}' (similarity {:.2}, list: {:?}, exact: {}).",
                    name, m.matched_name, m.similarity, m.list, m.is_exact
                );
                let severity = if m.is_exact { "critical" } else { "high" };
                let list_url = match &m.list {
                    SanctionsList::OfacSdn | SanctionsList::OfacNs =>
                        "https://home.treasury.gov/policy-issues/financial-sanctions/sdn-list",
                    SanctionsList::EuConsolidated =>
                        "https://eeas.europa.eu/topics/sanctions-policy/8442/consolidated-list_en",
                    SanctionsList::UnSecurity =>
                        "https://www.un.org/securitycouncil/content/un-sc-consolidated-list",
                    SanctionsList::BisEntityList =>
                        "https://www.bis.doc.gov/index.php/policy-guidance/lists-of-parties-of-concern/entity-list",
                    SanctionsList::BisDeniedPersons =>
                        "https://www.bis.doc.gov/index.php/policy-guidance/lists-of-parties-of-concern/denied-persons-list",
                };
                if let Err(error) = ingress
                    .submit_warning(
                        NewWarning::new("sanctions", &title, severity)
                            .description(&description)
                            .source_urls(vec![list_url.to_string()])
                            .confidence(m.similarity),
                    )
                    .await
                {
                    tracing::warn!(%error, entity = %name, "sanctions_screen: failed to ingest warning");
                }
            }
        }
    }

    run.succeed(
        total_hits,
        &format!(
            "screened {} entities against {} sanctions entries: {} matches",
            entity_names.len(),
            screener.entry_count(),
            total_hits
        ),
    );
    run
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) async fn run_sla_enforcement(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let pool = store.pool.clone();
    let rows = sqlx::query(
        "SELECT id::text AS id, title, severity, warning_type, COALESCE(entity_ids[1]::text, NULL) AS entity_id, created_at, acknowledged \
         FROM warnings WHERE acknowledged = false ORDER BY created_at ASC LIMIT 500",
    )
    .fetch_all(&pool)
    .await;

    let records: Vec<SlaWarningRecord> = match rows {
        Ok(rows) => {
            use sqlx::Row as _;
            rows.into_iter()
                .filter_map(|r| {
                    let id: Option<String> = r.try_get("id").ok();
                    let title: Option<String> = r.try_get("title").ok();
                    let severity: Option<String> = r.try_get("severity").ok();
                    let warning_type: Option<String> = r.try_get("warning_type").ok();
                    let entity_id: Option<String> = r.try_get("entity_id").ok();
                    let created_at: Option<chrono::DateTime<Utc>> = r.try_get("created_at").ok();
                    let acknowledged: Option<bool> = r.try_get("acknowledged").ok();
                    Some(SlaWarningRecord {
                        id: id?,
                        title: title?,
                        severity: severity?,
                        warning_type: warning_type?,
                        entity_id,
                        created_at: created_at?,
                        acknowledged: acknowledged?,
                    })
                })
                .collect()
        }
        Err(e) => {
            run.fail(&format!("sla_enforcement: query failed: {e}"));
            return run;
        }
    };

    let enforcer = SlaEnforcer::from_env();
    let reminder_ahead_seconds = std::env::var("SLA_REMINDER_AHEAD_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(900);

    let mut pending_alerts = Vec::new();

    for record in enforcer.approaching_sla(&records, reminder_ahead_seconds) {
        let delivery_key = format!("sla-reminder:{}", record.id);
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let detail = serde_json::json!({
            "warning_id": record.id,
            "severity": record.severity,
            "kind": "approaching",
            "seconds_remaining": record.sla_seconds_remaining(&apex_core::sla::SeveritySlaConfig::default())
        });
        match store
            .try_record_sla_reminder(&record.id, "approaching", &delivery_key, &detail)
            .await
        {
            Ok(true) => {
                if let Some(alert) =
                    enforcer.build_approaching_alert(record, reminder_ahead_seconds)
                {
                    pending_alerts.push(alert);
                }
            }
            Ok(false) => {}
            Err(err) => {
                run.fail(&format!("sla_enforcement: reminder tracking failed: {err}"));
                return run;
            }
        }
    }

    for alert in enforcer.check_sla_violations(&records) {
        let warning_id = alert.source_id.trim_start_matches("sla-breach:");
        let delivery_key = format!("notify:{}", alert.source_id);
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let detail = serde_json::json!({
            "warning_id": warning_id,
            "severity": alert.severity.as_str(),
            "kind": "breach"
        });
        match store
            .try_record_sla_reminder(warning_id, "breach", &delivery_key, &detail)
            .await
        {
            Ok(true) => pending_alerts.push(alert),
            Ok(false) => {}
            Err(err) => {
                run.fail(&format!("sla_enforcement: breach tracking failed: {err}"));
                return run;
            }
        }
    }

    let violation_count = pending_alerts
        .iter()
        .filter(|alert| alert.category == "sla_breach")
        .count() as u64;

    if !pending_alerts.is_empty() {
        let dispatcher = NotificationDispatcher::from_env();
        let dispatched = dispatcher.dispatch_batch(pending_alerts).await;
        for notification in &dispatched {
            let status = if notification.dispatch_success.unwrap_or(false) {
                "delivered"
            } else {
                "failed"
            };
            #[allow(clippy::unwrap_used, clippy::expect_used)]
            let payload = serde_json::json!({
                "alert_id": notification.alert.source_id,
                "subject": notification.subject,
                "body": notification.formatted_body,
                "category": notification.alert.category,
            });
            let next_retry_at = if status == "failed" {
                Some(Utc::now() + chrono::Duration::minutes(5))
            } else {
                None
            };
            let _ = store
                .record_notification_delivery_attempt(
                    &notification.id,
                    &notification.channel,
                    notification
                        .destination
                        .as_deref()
                        .unwrap_or(&notification.channel),
                    &payload,
                    status,
                    notification.error_message.as_deref(),
                    next_retry_at,
                )
                .await;
        }
        tracing::warn!(
            violations = violation_count,
            dispatched = dispatched.len(),
            "sla_enforcement: escalated SLA breaches"
        );
    }

    run.succeed(
        violation_count,
        &format!(
            "checked {} unacknowledged warnings: {} SLA breaches escalated",
            records.len(),
            violation_count
        ),
    );
    run
}

/// Run dig for a specific record type and return the combined stdout, or empty on failure.
async fn dig_lookup(domain: &str, rtype: &str, timeout_secs: u64) -> String {
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        tokio::process::Command::new("dig")
            .args(["+short", "+time=3", "+tries=1", rtype, domain])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// Check for SPF (TXT record containing v=spf1).
async fn check_spf(domain: &str) -> (bool, Option<String>) {
    let txt = dig_lookup(domain, "TXT", 6).await;
    for line in txt.lines() {
        let cleaned = line.trim().trim_matches('"');
        if cleaned.to_lowercase().contains("v=spf1") {
            return (true, Some(cleaned.to_string()));
        }
    }
    (false, None)
}

/// Check common DKIM selectors.
async fn check_dkim(domain: &str) -> bool {
    let selectors = [
        "default",
        "google",
        "selector1",
        "selector2",
        "k1",
        "mail",
        "dkim",
    ];
    for sel in &selectors {
        let dkim_domain = format!("{}._domainkey.{}", sel, domain);
        let result = dig_lookup(&dkim_domain, "TXT", 6).await;
        for line in result.lines() {
            let cleaned = line.trim().trim_matches('"');
            let lower = cleaned.to_lowercase();
            if lower.contains("v=dkim1") || lower.contains("p=") {
                return true;
            }
        }
    }
    false
}

/// Check DMARC (TXT at _dmarc.<domain>), returning (present, policy).
async fn check_dmarc(domain: &str) -> (bool, Option<String>) {
    let txt = dig_lookup(&format!("_dmarc.{}", domain), "TXT", 6).await;
    for line in txt.lines() {
        let cleaned = line.trim().trim_matches('"');
        let lower = cleaned.to_lowercase();
        if lower.contains("v=dmarc1") {
            let policy = lower
                .split(';')
                .find_map(|part| part.trim().strip_prefix("p="))
                .map(|p| p.trim().to_string());
            return (true, policy);
        }
    }
    (false, None)
}

/// Collect a DNS issue result for warning generation.
#[allow(dead_code)]
struct DnsIssueResult {
    company_name: String,
    domain: String,
    region: Option<String>,
    company_id: Uuid,
    has_spf: bool,
    has_dkim: bool,
    has_dmarc: bool,
    score: f64,
}

pub(super) async fn run_dns_posture_scan(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let companies = store
        .list_companies(
            &apex_store::postgres::CompanyListFilters {
                regions: vec![],
                search: None,
                is_competitor: None,
            },
            Some(apex_store::postgres::CompanyOrderBy::Name),
            false,
            500,
            0,
        )
        .await
        .unwrap_or_default();

    let mut checked = 0u64;
    let mut dns_issues: Vec<DnsIssueResult> = Vec::new();
    let mut warnings_generated: u64 = 0;
    let mut warning_ingest_failures: u64 = 0;

    for company in &companies {
        let Some(domain) = &company.domain else {
            continue;
        };
        if domain.is_empty() {
            continue;
        }

        // Perform DNS lookups (SPF, DKIM, DMARC) using dig subprocess
        let (has_spf, spf_record) = check_spf(domain).await;
        let has_dkim = check_dkim(domain).await;
        let (has_dmarc, dmarc_policy) = check_dmarc(domain).await;

        let posture_score = compute_dns_score(has_spf, has_dkim, has_dmarc);

        // Persist to dedicated dns_posture_entries table
        if let Err(e) = store
            .insert_dns_posture_entry(
                Some(company.id),
                domain,
                has_spf,
                has_dkim,
                has_dmarc,
                dmarc_policy.as_deref(),
                spf_record.as_deref(),
                posture_score,
            )
            .await
        {
            tracing::warn!(
                domain = %domain,
                error = %e,
                "dns_posture_scan: failed to persist to dns_posture_entries"
            );
        }

        // Persist to observations table (for API/page consumption)
        let pool = &store.pool;
        let obs_id = Uuid::new_v4();
        let now = chrono::Utc::now();
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let value = serde_json::json!({
            "domain": domain,
            "company_name": company.name,
            "has_spf": has_spf,
            "has_dkim": has_dkim,
            "has_dmarc": has_dmarc,
            "dmarc_policy": dmarc_policy,
            "posture_score": posture_score,
        });
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let provenance = serde_json::json!({
            "source": "worker_dns_posture_scan",
            "content_hash": format!("dns_{}_{}", domain, now.format("%Y%m%d")),
        });

        // Delete stale entry + insert fresh
        let _ = sqlx::query(
            r#"DELETE FROM observations
               WHERE observation_type = 'dns_posture' AND entity_id = $1"#,
        )
        .bind(company.id)
        .execute(pool)
        .await;

        let _ = sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
               VALUES ($1, 'dns_posture', $2, 'company', $3, $4::jsonb, $5::jsonb, 0.95)"#,
        )
        .bind(obs_id)
        .bind(company.id)
        .bind(now)
        .bind(value)
        .bind(provenance)
        .execute(pool)
        .await;

        checked += 1;
        if posture_score < 50.0 {
            dns_issues.push(DnsIssueResult {
                company_name: company.name.clone(),
                domain: domain.clone(),
                region: company.region.clone(),
                company_id: company.id,
                has_spf,
                has_dkim,
                has_dmarc,
                score: posture_score,
            });
        }

        tracing::info!(
            domain = %domain,
            company = %company.name,
            has_spf = has_spf,
            has_dkim = has_dkim,
            has_dmarc = has_dmarc,
            score = posture_score,
            "dns_posture_scan: domain checked"
        );

        // Small delay to avoid overwhelming DNS servers
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Generate security warnings for the worst DNS offenders (top 10 by worst score)
    dns_issues.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for result in dns_issues.iter().take(10) {
        let mut missing = Vec::new();
        if !result.has_spf {
            missing.push("SPF");
        }
        if !result.has_dkim {
            missing.push("DKIM");
        }
        if !result.has_dmarc {
            missing.push("DMARC");
        }

        let title = format!(
            "DNS posture: {} — missing {}",
            result.company_name,
            missing.join(", ")
        );
        let description = format!(
            "{} is missing {} email authentication record{} (posture score: {:.0}%). \
             This may increase email spoofing risk for this domain.",
            result.domain,
            missing.join(", "),
            if missing.len() > 1 { "s" } else { "" },
            result.score,
        );
        let severity = if result.score < 20.0 { "high" } else { "low" };

        let mut warning = NewWarning::new("security", &title, severity)
            .description(&description)
            .confidence(0.85);
        if let Some(region) = result.region.as_deref() {
            warning = warning.region(region);
        }
        match ingress.submit_warning(warning).await {
            Ok(_) => warnings_generated += 1,
            Err(error) => {
                warning_ingest_failures += 1;
                tracing::warn!(
                    %error,
                    domain = %result.domain,
                    "dns_posture_scan: failed to ingest warning"
                );
            }
        }
    }

    let summary = format!(
        "dns_posture_scan: checked {} domains ({} with issues, {} warnings generated, {} warning ingest failures)",
        checked,
        dns_issues.len(),
        warnings_generated,
        warning_ingest_failures,
    );
    if warning_ingest_failures > 0 {
        run.items_processed = checked;
        run.fail(&format!("{summary} — warning ingestion degraded"));
    } else {
        run.succeed(warnings_generated, &summary);
    }
    run
}

/// Compute a DNS posture score 0–100 based on SPF (30) + DKIM (30) + DMARC (40).
fn compute_dns_score(has_spf: bool, has_dkim: bool, has_dmarc: bool) -> f64 {
    let mut score = 0.0;
    if has_spf {
        score += 30.0;
    }
    if has_dkim {
        score += 30.0;
    }
    if has_dmarc {
        score += 40.0;
    }
    score
}

pub(super) async fn run_kev_catalog_fetch(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let url = "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json";

    // CISA blocks Hetzner IPs and the proxy provider blocks HTTP CONNECT to .gov,
    // so we shell out to curl with --socks5 which reliably tunnels through the proxy.
    let body = if let Some(proxy_url) = crate::build_paid_proxy_url_from_env() {
        let socks_url = proxy_url.replacen("http://", "", 1);
        tracing::info!("kev_catalog_fetch: fetching via SOCKS5 proxy");
        let output = tokio::process::Command::new("curl")
            .args([
                "-s",
                "--socks5",
                &socks_url,
                "-A",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
                "--connect-timeout",
                "30",
                "-m",
                "90",
                url,
            ])
            .output()
            .await;
        match output {
            Ok(out) if out.status.success() => Ok(out.stdout),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                Err(format!("curl exit {}: {}", out.status, stderr.trim()))
            }
            Err(e) => Err(format!("failed to spawn curl: {e}")),
        }
    } else {
        tracing::info!("kev_catalog_fetch: fetching directly (no proxy configured)");
        match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
            .build()
        {
            Ok(client) => match client.get(url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                    Ok(b) => Ok(b.to_vec()),
                    Err(e) => Err(format!("failed to read response body: {e}")),
                },
                Ok(resp) => Err(format!("HTTP {} from CISA", resp.status())),
                Err(e) => Err(format!("request failed: {e}")),
            },
            Err(e) => Err(format!("failed to build HTTP client: {e}")),
        }
    };

    match body {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(catalog) => {
                let vulnerabilities = catalog["vulnerabilities"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                let count = vulnerabilities.len();
                // B328: the downloaded catalog was previously parsed, counted,
                // and discarded — no KEV observation was ever produced, yet
                // cross-domain mining consumes `kev_match` observations. Store
                // each entry with a deterministic ID so re-fetches dedup.
                let mut stored = 0u64;
                for vuln in &vulnerabilities {
                    let cve_id = vuln["cveID"].as_str().unwrap_or_default().to_string();
                    if cve_id.is_empty() {
                        continue;
                    }
                    let mut obs = apex_core::entities::Observation::new(
                        apex_core::entities::ObservationType::VulnNotice,
                        chrono::Utc::now(),
                        serde_json::json!({
                            "cve_id": cve_id,
                            "vendor": vuln["vendorProject"].as_str().unwrap_or(""),
                            "product": vuln["product"].as_str().unwrap_or(""),
                            "vulnerability_name": vuln["vulnerabilityName"].as_str().unwrap_or(""),
                            "date_added": vuln["dateAdded"].as_str().unwrap_or(""),
                            "known_ransomware_use":
                                vuln["knownRansomwareCampaignUse"].as_str().unwrap_or("unknown"),
                            "required_action": vuln["requiredAction"].as_str().unwrap_or(""),
                            "due_date": vuln["dueDate"].as_str().unwrap_or(""),
                        }),
                        serde_json::json!({
                            "source": "cisa_kev",
                            "source_id": cve_id,
                        }),
                    );
                    obs.stabilize_id("kev");
                    match store.insert_observation(&obs).await {
                        Ok(_) => stored += 1,
                        Err(e) => {
                            tracing::warn!(cve = %cve_id, error = %e, "kev_catalog_fetch: insert failed");
                        }
                    }
                }
                tracing::info!(
                    cve_count = count,
                    stored,
                    "kev_catalog_fetch: catalog downloaded successfully"
                );
                run.succeed(
                    stored,
                    &format!("kev_catalog_fetch: downloaded {count} CVEs from CISA KEV, stored {stored} new"),
                );
            }
            Err(e) => run.fail(&format!("kev_catalog_fetch: failed to parse JSON: {e}")),
        },
        Err(e) => run.fail(&format!("kev_catalog_fetch: {e}")),
    }
    run
}

pub(super) async fn run_lookalike_domain_scan(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let companies = store
        .list_companies(
            &apex_store::postgres::CompanyListFilters {
                regions: vec![],
                search: None,
                is_competitor: None,
            },
            Some(apex_store::postgres::CompanyOrderBy::Name),
            false,
            500,
            0,
        )
        .await
        .unwrap_or_default();
    let mut domains_scanned = 0u64;
    let mut total_variants = 0u64;
    // B329: verify DNS registration before persisting a lookalike. The
    // previous version asserted every generated typosquat variant was an
    // active 0.8-confidence threat — thousands of false positives presented
    // as real findings. Unregistered domains cannot host anything.
    let dns_checker = apex_crawl::dns::DnsChecker::new();
    // Cap DNS checks per domain: typosquat generators can emit dozens of
    // variants and each check is a resolver round-trip.
    let max_checks_per_domain: usize = std::env::var("LOOKALIKE_MAX_CHECKS_PER_DOMAIN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    for company in &companies {
        if let Some(domain) = &company.domain {
            let variants: Vec<String> = generate_typosquat_variants(domain);
            tracing::debug!(
                domain = %domain,
                variants = variants.len(),
                company = %company.name,
                "lookalike_domain_scan: variants generated"
            );
            for variant in variants.iter().take(max_checks_per_domain) {
                let is_registered = dns_checker
                    .check_lookalike_registration(variant)
                    .await
                    .unwrap_or(false);
                if !is_registered {
                    continue;
                }

                // Persist to dedicated lookalike_domains table
                if let Err(e) = store
                    .insert_lookalike_domain(
                        Some(company.id),
                        domain,
                        variant,
                        "typosquat",
                        1,
                        true,
                    )
                    .await
                {
                    tracing::warn!(
                        domain = %domain,
                        variant = %variant,
                        error = %e,
                        "lookalike_domain_scan: failed to persist variant"
                    );
                }

                // Persist to observations table for API consumption
                // (delete stale entry first, same pattern as Python scanner)
                let pool = &store.pool;
                let _ = sqlx::query(
                    r#"DELETE FROM observations
                       WHERE observation_type = 'typosquat' AND entity_id = $1
                         AND value->>'domain' = $2"#,
                )
                .bind(company.id)
                .bind(variant)
                .execute(pool)
                .await;

                let obs_id = Uuid::new_v4();
                let now = chrono::Utc::now();
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let value = serde_json::json!({
                    "original_domain": domain,
                    "domain": variant,
                    "distance": 1,
                    "threat_type": "typosquat",
                    "active": true,
                    "dns_verified": true,
                });
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let provenance = serde_json::json!({
                    "source": "worker_lookalike_scan",
                    "content_hash": format!("la_{}_{}", domain, variant),
                });

                let _ = sqlx::query(
                    r#"INSERT INTO observations
                       (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
                       VALUES ($1, 'typosquat', $2, 'company', $3, $4::jsonb, $5::jsonb, 0.80)"#,
                )
                .bind(obs_id)
                .bind(company.id)
                .bind(now)
                .bind(value)
                .bind(provenance)
                .execute(pool)
                .await;

                total_variants += 1;
            }
            domains_scanned += 1;
        }
    }
    run.succeed(
        total_variants,
        &format!(
            "lookalike_domain_scan: scanned {} domains, {} registered lookalike variants confirmed via DNS",
            domains_scanned, total_variants
        ),
    );
    run
}
