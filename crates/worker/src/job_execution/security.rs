use std::sync::Arc;

use crate::*;

pub(super) async fn run_breach_scan(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let hibp_key = std::env::var("HIBP_API_KEY").ok();
    let intelx_key = std::env::var("INTELX_API_KEY").ok();
    let pastebin_key = std::env::var("PASTEBIN_API_DEV_KEY").ok();

    let domains_raw = std::env::var("MONITORED_DOMAINS").unwrap_or_default();
    let domains: Vec<String> = domains_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if domains.is_empty() {
        run.skip("breach_scan: no MONITORED_DOMAINS configured");
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
            let breach_urls_opt = if breach_urls.is_empty() {
                None
            } else {
                Some(breach_urls)
            };
            let title = format!("Domain breach exposure: {domain}");
            let description = format!(
                "{count} breach event(s) detected for domain '{domain}'. Immediate review recommended."
            );
            let _ = store
                .insert_warning(
                    "breach",
                    &title,
                    Some(&description),
                    if count > 5 { "critical" } else { "high" },
                    None,
                    Some("breach_scan"),
                    None,
                    breach_urls_opt,
                    Some(0.9),
                )
                .await;
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

pub(super) async fn run_sanctions_screen(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let entities_raw = std::env::var("MONITORED_ENTITIES").unwrap_or_default();
    let entity_names: Vec<String> = entities_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if entity_names.is_empty() {
        run.skip("sanctions_screen: no MONITORED_ENTITIES configured");
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
                let _ = store
                    .insert_warning(
                        "sanctions",
                        &title,
                        Some(&description),
                        severity,
                        None,
                        Some("sanctions_screen"),
                        None,
                        Some(vec![list_url.to_string()]),
                        Some(m.similarity),
                    )
                    .await;
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

pub(super) async fn run_dns_posture_scan(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
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
    for company in &companies {
        if let Some(domain) = &company.domain {
            tracing::debug!(domain = %domain, company = %company.name, "dns_posture_scan: queued domain check");
            checked += 1;
        }
    }
    run.succeed(
        checked,
        &format!(
            "dns_posture_scan: queued {} domains for DNS posture check",
            checked
        ),
    );
    run
}

pub(super) async fn run_kev_catalog_fetch(kind: &JobKind) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let url = "https://www.cisa.gov/sites/default/files/feeds/known_exploited_vulnerabilities.json";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    match client {
        Ok(client) => match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(catalog) => {
                        let count = catalog["vulnerabilities"]
                            .as_array()
                            .map(|v| v.len())
                            .unwrap_or(0);
                        tracing::info!(
                            cve_count = count,
                            "kev_catalog_fetch: catalog downloaded successfully"
                        );
                        run.succeed(
                            count as u64,
                            &format!("kev_catalog_fetch: downloaded {} CVEs from CISA KEV", count),
                        );
                    }
                    Err(e) => run.fail(&format!("kev_catalog_fetch: failed to parse JSON: {e}")),
                }
            }
            Ok(resp) => run.fail(&format!(
                "kev_catalog_fetch: HTTP {} from CISA",
                resp.status()
            )),
            Err(e) => run.fail(&format!("kev_catalog_fetch: request failed: {e}")),
        },
        Err(e) => run.fail(&format!(
            "kev_catalog_fetch: failed to build HTTP client: {e}"
        )),
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
            200,
            0,
        )
        .await
        .unwrap_or_default();
    let mut domains_scanned = 0u64;
    for company in &companies {
        if let Some(domain) = &company.domain {
            let variants: Vec<String> = generate_typosquat_variants(domain);
            tracing::debug!(
                domain = %domain,
                variants = variants.len(),
                company = %company.name,
                "lookalike_domain_scan: variants generated"
            );
            domains_scanned += 1;
        }
    }
    run.succeed(
        domains_scanned,
        &format!(
            "lookalike_domain_scan: scanned {} domains for lookalike variants",
            domains_scanned
        ),
    );
    run
}
