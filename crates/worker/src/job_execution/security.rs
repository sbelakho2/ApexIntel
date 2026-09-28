use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use uuid::Uuid;

use apex_core::config::ConfigErrors;
use apex_core::measurement::Measurement;
use apex_crawl::dns::{
    extract_dmarc_record, extract_spf_record, DkimStatus, DnsLookupOutcome, StructuredDnsResolver,
};

use crate::intelligence_ingress::{IngressCounters, IntelligenceIngress, NewWarning};
use crate::*;

/// Parse one numeric security threshold from the environment.
///
/// An absent variable keeps its default; a present-but-malformed value is a
/// configuration error (never a silent fallback) so the job reports the
/// misconfiguration instead of running with a threshold nobody chose.
fn security_threshold<T>(
    name: &str,
    default: T,
    parse: impl Fn(&str) -> Option<T>,
    expected: &str,
) -> std::result::Result<T, ConfigErrors> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(raw) => parse(raw.trim()).ok_or_else(|| ConfigErrors::single(name, raw, expected)),
    }
}

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

    // Load domains from env var first, fall back to DB companies. Each domain
    // carries its owning company id when known so the breach alert is resolved
    // against that company's subscribers instead of broadcasting to everyone.
    let domains_raw = std::env::var("MONITORED_DOMAINS").unwrap_or_default();
    let mut domains: Vec<(String, Option<Uuid>)> = domains_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|domain| (domain, None))
        .collect();

    if domains.is_empty() {
        let companies = match store
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
        {
            Ok(companies) => companies,
            Err(error) => {
                run.fail(&format!(
                    "breach_scan: failed to load monitored domains: {error}"
                ));
                return run;
            }
        };
        domains = companies
            .iter()
            .filter_map(|c| {
                c.domain
                    .clone()
                    .filter(|d| !d.is_empty())
                    .map(|domain| (domain, Some(c.id)))
            })
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
    let mut counters = IngressCounters::default();

    for (domain, company_id) in &domains {
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
            let mut warning = NewWarning::new("breach", &title, severity)
                .description(&description)
                .source_urls(breach_urls)
                .confidence(0.9);
            match company_id {
                // Entity-scoped breach: notify the company's subscribers.
                Some(company_id) => warning = warning.entity_ids(vec![*company_id]),
                // Domain came from MONITORED_DOMAINS with no owning company:
                // there is no entity to resolve, so this is an explicit
                // system-wide security alert.
                None => warning = warning.system_broadcast(),
            }
            match ingress.submit_warning(warning).await {
                Ok(result) => counters.record(&result),
                Err(error) => {
                    tracing::warn!(%error, domain = %domain, "breach_scan: failed to ingest warning");
                }
            }
        } else {
            tracing::info!(domain = %domain, "breach_scan: clean");
        }
    }

    let summary = format!(
        "scanned {} domain(s): {} breach events found; {}",
        domains.len(),
        total_hits,
        counters.summary(),
    );
    match counters.success_blocker() {
        Some(reason) => run.degrade(counters.warnings_persisted, &format!("{summary}; {reason}")),
        None => run.succeed(counters.warnings_persisted, &summary),
    }
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
    // Each entity carries its id when known so a sanctions hit resolves the
    // entity's subscribers; env-configured names have no id and fall back to an
    // explicit system-wide alert.
    let entities_raw = std::env::var("MONITORED_ENTITIES").unwrap_or_default();
    let mut entity_names: Vec<(String, Option<Uuid>)> = entities_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|name| (name, None))
        .collect();

    if entity_names.is_empty() {
        let companies = match store
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
        {
            Ok(companies) => companies,
            Err(error) => {
                run.fail(&format!(
                    "sanctions_screen: failed to load screened entities: {error}"
                ));
                return run;
            }
        };
        entity_names.extend(companies.iter().map(|c| (c.name.clone(), Some(c.id))));

        let persons = match store
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
        {
            Ok(persons) => persons,
            Err(error) => {
                run.fail(&format!(
                    "sanctions_screen: failed to load screened persons: {error}"
                ));
                return run;
            }
        };
        entity_names.extend(persons.iter().map(|p| (p.name.clone(), Some(p.id))));
    }

    if entity_names.is_empty() {
        run.skip("sanctions_screen: no entities found (neither in MONITORED_ENTITIES nor in DB)");
        return run;
    }

    let threshold: f64 = match security_threshold(
        "SANCTIONS_THRESHOLD",
        0.92_f64,
        |raw| {
            raw.parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        },
        "a finite number in 0..=1",
    ) {
        Ok(threshold) => threshold,
        Err(errors) => {
            run.fail(&format!(
                "sanctions_screen: invalid configuration: {errors}"
            ));
            return run;
        }
    };

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
    let mut counters = IngressCounters::default();

    for (name, entity_id) in &entity_names {
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
                let mut warning = NewWarning::new("sanctions", &title, severity)
                    .description(&description)
                    .source_urls(vec![list_url.to_string()])
                    .confidence(m.similarity);
                match entity_id {
                    Some(entity_id) => warning = warning.entity_ids(vec![*entity_id]),
                    // Env-configured name with no entity row: a compliance
                    // alert this severe is an explicit system-wide alert.
                    None => warning = warning.system_broadcast(),
                }
                match ingress.submit_warning(warning).await {
                    Ok(result) => counters.record(&result),
                    Err(error) => {
                        tracing::warn!(%error, entity = %name, "sanctions_screen: failed to ingest warning");
                    }
                }
            }
        }
    }

    let summary = format!(
        "screened {} entities against {} sanctions entries: {} matches; {}",
        entity_names.len(),
        screener.entry_count(),
        total_hits,
        counters.summary(),
    );
    match counters.success_blocker() {
        Some(reason) => run.degrade(counters.warnings_persisted, &format!("{summary}; {reason}")),
        None => run.succeed(counters.warnings_persisted, &summary),
    }
    run
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(super) async fn run_sla_enforcement(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let pool = store.pool.clone();
    let rows = sqlx::query(
        "SELECT id::text AS id, title, severity, warning_type, COALESCE(entity_ids, ARRAY[]::uuid[]) AS entity_ids, is_system_broadcast, created_at, acknowledged \
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
                    let entity_ids: Option<Vec<uuid::Uuid>> = r.try_get("entity_ids").ok();
                    let is_system_broadcast: Option<bool> = r.try_get("is_system_broadcast").ok();
                    let created_at: Option<chrono::DateTime<Utc>> = r.try_get("created_at").ok();
                    let acknowledged: Option<bool> = r.try_get("acknowledged").ok();
                    Some(SlaWarningRecord {
                        id: id?,
                        title: title?,
                        severity: severity?,
                        warning_type: warning_type?,
                        // false-success-classification: best-effort — optional display metadata; a decode failure renders the row with no linked entities
                        entity_ids: entity_ids
                            .unwrap_or_default()
                            .into_iter()
                            .map(|entity_id| entity_id.to_string())
                            .collect(),
                        // false-success-classification: best-effort — optional broadcast flag; a decode failure renders the row as non-broadcast
                        is_system_broadcast: is_system_broadcast.unwrap_or(false),
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

    let enforcer = match SlaEnforcer::from_env() {
        Ok(enforcer) => enforcer,
        Err(errors) => {
            run.fail(&format!("sla_enforcement: invalid configuration: {errors}"));
            return run;
        }
    };
    let reminder_ahead_seconds = match security_threshold(
        "SLA_REMINDER_AHEAD_SECONDS",
        900_i64,
        |raw| raw.parse::<i64>().ok().filter(|value| *value >= 0),
        "a non-negative integer",
    ) {
        Ok(seconds) => seconds,
        Err(errors) => {
            run.fail(&format!("sla_enforcement: invalid configuration: {errors}"));
            return run;
        }
    };

    let mut pending_alerts = Vec::new();

    for record in enforcer.approaching_sla(&records, reminder_ahead_seconds) {
        let delivery_key = format!("sla-reminder:{}", record.id);
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let detail = serde_json::json!({
            "warning_id": record.id,
            "severity": record.severity,
            "kind": "approaching",
            // Use the enforcer's resolved windows, not the code defaults, so
            // the recorded metadata matches the window actually enforced.
            "seconds_remaining": record.sla_seconds_remaining(enforcer.windows())
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

    let mut channel_deliveries = 0usize;
    if !pending_alerts.is_empty() {
        // Same durable pipeline as every other alert: persist the domain
        // alert/event and its alert outbox row, then persist one
        // `notification_delivery_state` row per configured channel BEFORE any
        // attempt. The retry-processor job owns the channel sends, backoff and
        // dead-lettering — this job never publishes or sends directly.
        let channels = match apex_worker::notification_delivery::ConfiguredChannelRouter::from_env()
        {
            Ok(router) => router.channels(),
            Err(error) => {
                run.fail(&format!(
                    "sla_enforcement: failed to build the channel router: {error}"
                ));
                return run;
            }
        };
        match apex_worker::notification_delivery::enqueue_sla_alerts(
            store.as_ref(),
            pending_alerts,
            &channels,
        )
        .await
        {
            Ok(summary) => {
                channel_deliveries = summary.channel_deliveries;
                tracing::warn!(
                    violations = violation_count,
                    alerts_enqueued = summary.alerts_enqueued,
                    already_enqueued = summary.alerts_already_enqueued,
                    channel_deliveries = summary.channel_deliveries,
                    "sla_enforcement: SLA alerts queued into the durable notification pipeline"
                );
            }
            Err(error) => {
                run.fail(&format!(
                    "sla_enforcement: failed to enqueue SLA alerts into the outbox: {error}"
                ));
                return run;
            }
        }
    }

    run.succeed(
        violation_count,
        &format!(
            "checked {} unacknowledged warnings: {} SLA breaches queued ({} channel deliveries)",
            records.len(),
            violation_count,
            channel_deliveries
        ),
    );
    run
}

// ─────────────────────────────────────────────────────────────────────────────
// DNS email-authentication checks (structured, hickory-backed)
//
// These previously shelled out to `dig`, which collapsed NXDOMAIN, empty
// answers, resolver timeouts and process failures into an empty string. The
// structured resolver keeps definitive absence separate from an indeterminate
// lookup so warnings are never asserted from a failed DNS query.
// ─────────────────────────────────────────────────────────────────────────────

/// Result of one SPF/DMARC check: the record when found plus the raw
/// resolution outcome so callers can distinguish "absent" from "unknown".
struct EmailAuthCheck {
    record: Option<String>,
    policy: Option<String>,
    outcome: DnsLookupOutcome,
}

impl EmailAuthCheck {
    /// True when the record is definitively absent (never true for an
    /// indeterminate lookup).
    fn definitively_absent(&self) -> bool {
        self.record.is_none() && !self.outcome.is_indeterminate()
    }

    fn is_unknown(&self) -> bool {
        self.record.is_none() && self.outcome.is_indeterminate()
    }
}

/// Check for SPF (TXT record containing v=spf1).
async fn check_spf(resolver: &StructuredDnsResolver, domain: &str) -> EmailAuthCheck {
    let outcome = resolver.txt(domain).await;
    let record = outcome.records().and_then(extract_spf_record);
    EmailAuthCheck {
        record,
        policy: None,
        outcome,
    }
}

/// Check DKIM across the common selectors, tri-state.
async fn check_dkim(resolver: &StructuredDnsResolver, domain: &str) -> DkimStatus {
    resolver.dkim_status(domain).await
}

/// Check DMARC (TXT at _dmarc.<domain>), returning the record, policy and
/// structured resolution outcome.
async fn check_dmarc(resolver: &StructuredDnsResolver, domain: &str) -> EmailAuthCheck {
    let lookup_domain = format!("_dmarc.{domain}");
    let outcome = resolver.txt(&lookup_domain).await;
    let parsed = outcome
        .records()
        .and_then(extract_dmarc_record)
        .map(|(record, policy)| (Some(record), policy))
        .unwrap_or((None, None));
    EmailAuthCheck {
        record: parsed.0,
        policy: parsed.1,
        outcome,
    }
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
    spf_unknown: bool,
    dmarc_unknown: bool,
    dkim_status: DkimStatus,
    score: f64,
}

/// Findings that are safe to assert as *missing* from the collected facts.
///
/// DKIM is only reported missing for `NotObservedOnKnownSelectors`; an
/// indeterminate lookup is surfaced separately (and never as a missing
/// record).
fn dns_missing_findings(
    has_spf: bool,
    has_dmarc: bool,
    dkim_status: &DkimStatus,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if !has_spf {
        missing.push("SPF");
    }
    if dkim_status.is_confirmed_absent() {
        missing.push("DKIM");
    }
    if !has_dmarc {
        missing.push("DMARC");
    }
    missing
}

/// Caveats for checks whose resolution was indeterminate.
fn dns_unknown_notes(
    spf_unknown: bool,
    dmarc_unknown: bool,
    dkim_status: &DkimStatus,
) -> Vec<String> {
    let mut notes = Vec::new();
    if spf_unknown {
        notes.push("SPF lookup was indeterminate (resolver timeout/failure)".to_string());
    }
    if dmarc_unknown {
        notes.push("DMARC lookup was indeterminate (resolver timeout/failure)".to_string());
    }
    if let DkimStatus::Unknown { reason } = dkim_status {
        notes.push(format!("DKIM state unknown: {reason}"));
    }
    notes
}

pub(super) async fn run_dns_posture_scan(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let companies = match store
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
    {
        Ok(companies) => companies,
        Err(error) => {
            // Authoritative input: an empty list would silently turn "the
            // company load failed" into a clean zero-domain scan.
            run.fail(&format!(
                "dns_posture_scan: failed to load companies (refusing empty scan): {error}"
            ));
            return run;
        }
    };

    let resolver = StructuredDnsResolver::new();
    if !resolver.is_available() {
        run.fail(
            "dns_posture_scan: structured DNS resolver unavailable (system resolver config); \
             refusing to report DNS findings without a resolver",
        );
        return run;
    }

    let mut checked = 0u64;
    let mut dns_issues: Vec<DnsIssueResult> = Vec::new();
    let mut counters = IngressCounters::default();
    let mut warning_ingest_failures: u64 = 0;
    // Distinct false-success counters: an indeterminate lookup is not a
    // missing record, and a failed observation write is not a clean scan.
    let mut indeterminate_lookups: u64 = 0;
    let mut dns_persist_failures: u64 = 0;
    let mut observation_write_failures: u64 = 0;

    for company in &companies {
        let Some(domain) = &company.domain else {
            continue;
        };
        if domain.is_empty() {
            continue;
        }

        // Perform structured DNS lookups (SPF, DKIM tri-state, DMARC) via
        // hickory-resolver — no `dig` shell-outs.
        let spf = check_spf(&resolver, domain).await;
        let dkim_status = check_dkim(&resolver, domain).await;
        let dmarc = check_dmarc(&resolver, domain).await;

        let spf_unknown = spf.is_unknown();
        let dmarc_unknown = dmarc.is_unknown();
        if spf_unknown || dmarc_unknown || dkim_status.is_unknown() {
            indeterminate_lookups += 1;
        }

        let has_spf = spf.record.is_some();
        // `has_dkim` is only true for a confirmed key record; the tri-state is
        // persisted separately so "unknown" is never rendered as "missing".
        let has_dkim = dkim_status.is_confirmed();
        let has_dmarc = dmarc.record.is_some();
        let posture_score = compute_dns_score(has_spf, has_dkim, has_dmarc);

        // Persist to dedicated dns_posture_entries table (authoritative for
        // the security page): a failed write must be counted, not swallowed.
        let dkim_unknown_reason = match &dkim_status {
            DkimStatus::Unknown { reason } => Some(reason.as_str()),
            _ => None,
        };
        if let Err(e) = store
            .insert_dns_posture_entry(
                Some(company.id),
                domain,
                has_spf,
                has_dkim,
                has_dmarc,
                dmarc.policy.as_deref(),
                spf.record.as_deref(),
                posture_score,
                dkim_status.as_str(),
                dkim_unknown_reason,
            )
            .await
        {
            dns_persist_failures += 1;
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
            "spf_status": spf.outcome.as_label(),
            "has_dkim": has_dkim,
            "dkim_status": dkim_status.as_str(),
            "dkim_unknown_reason": dkim_unknown_reason,
            "has_dmarc": has_dmarc,
            "dmarc_status": dmarc.outcome.as_label(),
            "dmarc_policy": dmarc.policy,
            "posture_score": posture_score,
            "dns_lookups_indeterminate": spf_unknown || dmarc_unknown || dkim_status.is_unknown(),
        });
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let provenance = serde_json::json!({
            "source": "worker_dns_posture_scan",
            "content_hash": format!("dns_{}_{}", domain, now.format("%Y%m%d")),
        });

        // Delete stale entry + insert fresh. Both writes are authoritative
        // for the security API, so failures are counted and degrade the run.
        if let Err(error) = sqlx::query(
            r#"DELETE FROM observations
               WHERE observation_type = 'dns_posture' AND entity_id = $1"#,
        )
        .bind(company.id)
        .execute(pool)
        .await
        {
            observation_write_failures += 1;
            tracing::warn!(
                domain = %domain,
                error = %error,
                "dns_posture_scan: failed to clear stale dns_posture observation"
            );
        }

        if let Err(error) = sqlx::query(
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
        .await
        {
            observation_write_failures += 1;
            tracing::warn!(
                domain = %domain,
                error = %error,
                "dns_posture_scan: failed to write dns_posture observation"
            );
        }

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
                spf_unknown,
                dmarc_unknown,
                dkim_status: dkim_status.clone(),
                score: posture_score,
            });
        }

        tracing::info!(
            domain = %domain,
            company = %company.name,
            has_spf = has_spf,
            dkim_status = dkim_status.as_str(),
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
        let missing = dns_missing_findings(result.has_spf, result.has_dmarc, &result.dkim_status);
        let unknown_notes = dns_unknown_notes(
            result.spf_unknown,
            result.dmarc_unknown,
            &result.dkim_status,
        );
        if missing.is_empty() {
            // Nothing definitively missing: an unknown lookup must not be
            // escalated into a finding.
            tracing::info!(
                domain = %result.domain,
                notes = ?unknown_notes,
                "dns_posture_scan: no definitive DNS findings"
            );
            continue;
        }

        let title = format!(
            "DNS posture: {} — missing {}",
            result.company_name,
            missing.join(", ")
        );
        let mut description = format!(
            "{} is missing {} email authentication record{} (posture score: {:.0}%). \
             This may increase email spoofing risk for this domain.",
            result.domain,
            missing.join(", "),
            if missing.len() > 1 { "s" } else { "" },
            result.score,
        );
        if !unknown_notes.is_empty() {
            description.push_str(&format!(
                " Note: {}. Findings are limited to definitively absent records.",
                unknown_notes.join("; ")
            ));
        }
        // High severity requires complete evidence; an incomplete DNS picture
        // is capped at low severity.
        let severity = if result.score < 20.0 && unknown_notes.is_empty() {
            "high"
        } else {
            "low"
        };
        let confidence = if unknown_notes.is_empty() { 0.85 } else { 0.6 };

        let mut warning = NewWarning::new("security", &title, severity)
            .description(&description)
            .confidence(confidence)
            // Entity-scoped warning: address the company's real subscribers
            // instead of every connected user.
            .entity_ids(vec![result.company_id]);
        if let Some(region) = result.region.as_deref() {
            warning = warning.region(region);
        }
        match ingress.submit_warning(warning).await {
            Ok(result) => counters.record(&result),
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
        "dns_posture_scan: checked {} domains ({} with issues, {} indeterminate lookups, \
         {} dns_posture persist failures, {} observation write failures, \
         {} warning ingest failures); {}",
        checked,
        dns_issues.len(),
        indeterminate_lookups,
        dns_persist_failures,
        observation_write_failures,
        warning_ingest_failures,
        counters.summary(),
    );
    if warning_ingest_failures > 0 || dns_persist_failures > 0 || observation_write_failures > 0 {
        run.items_processed = checked;
        run.fail(&format!(
            "{summary} — DNS posture persistence or warning ingestion degraded"
        ));
    } else if indeterminate_lookups > 0 {
        run.degrade(
            counters.warnings_persisted,
            &format!("{summary}; indeterminate DNS lookups (unknown, not missing)"),
        );
    } else if let Some(reason) = counters.success_blocker() {
        run.degrade(counters.warnings_persisted, &format!("{summary}; {reason}"));
    } else {
        run.succeed(counters.warnings_persisted, &summary);
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

    // CISA blocks the production host's IP and the proxy provider blocks HTTP
    // CONNECT to .gov, so the KEV fetch goes through the same paid SOCKS5 proxy
    // the rest of the crawler uses — via the shared reqwest stack, not an
    // external `curl` binary whose absence silently disabled the job.
    let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .user_agent(user_agent);
    if let Some(proxy_url) = crate::build_paid_proxy_url_from_env() {
        let host = proxy_url
            .replacen("http://", "", 1)
            .replacen("https://", "", 1);
        let socks_url = format!("socks5h://{host}");
        match reqwest::Proxy::all(&socks_url) {
            Ok(proxy) => {
                tracing::info!("kev_catalog_fetch: fetching via SOCKS5 proxy");
                builder = builder.proxy(proxy);
            }
            Err(error) => {
                run.fail(&format!(
                    "kev_catalog_fetch: invalid proxy configuration {proxy_url:?}: {error}"
                ));
                return run;
            }
        }
    } else {
        tracing::info!("kev_catalog_fetch: fetching directly (no proxy configured)");
    }
    let client = match builder.build() {
        Ok(client) => client,
        Err(error) => {
            run.fail(&format!(
                "kev_catalog_fetch: failed to build the HTTP client: {error}"
            ));
            return run;
        }
    };
    let body = match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(bytes) => Ok(bytes.to_vec()),
            Err(error) => Err(format!("failed to read response body: {error}")),
        },
        Ok(resp) => Err(format!("HTTP {} from CISA", resp.status())),
        Err(error) => Err(format!("request failed: {error}")),
    };

    match body {
        Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(catalog) => {
                // Authoritative parser contract: a missing/non-array
                // `vulnerabilities` field is a schema change, not an empty
                // catalog — refusing to report a clean zero-CVE fetch.
                let Some(vulnerabilities) = catalog["vulnerabilities"].as_array().cloned() else {
                    run.fail(
                        "kev_catalog_fetch: schema change — 'vulnerabilities' is missing or not \
                         an array in the CISA KEV catalog",
                    );
                    return run;
                };
                let count = vulnerabilities.len();
                // B328: the downloaded catalog was previously parsed, counted,
                // and discarded — no KEV observation was ever produced, yet
                // cross-domain mining consumes `kev_match` observations. Store
                // each entry with a deterministic ID so re-fetches dedup.
                let mut stored = 0u64;
                let mut store_failures = 0u64;
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
                            // Authoritative persistence: the KEV observations
                            // feed cross-domain mining, so a failed insert must
                            // degrade the job instead of being warning-only.
                            store_failures += 1;
                            tracing::warn!(cve = %cve_id, error = %e, "kev_catalog_fetch: insert failed");
                        }
                    }
                }
                tracing::info!(
                    cve_count = count,
                    stored,
                    store_failures,
                    "kev_catalog_fetch: catalog downloaded successfully"
                );
                if store_failures > 0 {
                    run.items_processed = stored;
                    run.fail(&format!(
                        "kev_catalog_fetch: downloaded {count} CVEs, stored {stored}, \
                         {store_failures} observation insert failures"
                    ));
                } else {
                    run.succeed(
                        stored,
                        &format!("kev_catalog_fetch: downloaded {count} CVEs from CISA KEV, stored {stored} new"),
                    );
                }
            }
            Err(e) => run.fail(&format!("kev_catalog_fetch: failed to parse JSON: {e}")),
        },
        Err(e) => run.fail(&format!("kev_catalog_fetch: {e}")),
    }
    run
}

// ─────────────────────────────────────────────────────────────────────────────
// Lookalike-domain evidence (B329 extension)
//
// Registration alone is not a threat: anyone can register a typo of any
// domain, and most are parked. A lookalike only becomes a finding when
// independent evidence shows it is actually serving something (HTTP/TLS
// reachability, certificate issuance, MX, redirects, brand impersonation).
// Confidence and severity are derived from that evidence, and warnings are
// only emitted once enough of it exists.
// ─────────────────────────────────────────────────────────────────────────────

/// Reputation signal derived from fetched content and registration metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LookalikeReputation {
    Unknown,
    Neutral,
    Suspicious,
}

/// Independent evidence collected for one registered lookalike candidate.
#[derive(Debug, Clone, Serialize)]
pub(super) struct LookalikeEvidence {
    pub registered: bool,
    pub rdap_age_days: Option<i64>,
    pub rdap_registrar: Option<String>,
    pub http_reachable: bool,
    pub https_reachable: bool,
    pub tls_valid: bool,
    pub redirect_target: Option<String>,
    pub mx_present: bool,
    pub ct_certificate_present: bool,
    pub brand_content_match: bool,
    pub reputation: LookalikeReputation,
}

impl LookalikeEvidence {
    /// Evidence for a candidate that is only known to be registered.
    #[cfg(test)]
    pub(super) fn registration_only() -> Self {
        Self {
            registered: true,
            rdap_age_days: None,
            rdap_registrar: None,
            http_reachable: false,
            https_reachable: false,
            tls_valid: false,
            redirect_target: None,
            mx_present: false,
            ct_certificate_present: false,
            brand_content_match: false,
            reputation: LookalikeReputation::Unknown,
        }
    }

    /// Count of independent active-infrastructure signals.
    pub(super) fn active_infrastructure_count(&self) -> u8 {
        let mut count = 0;
        if self.https_reachable {
            count += 1;
        }
        if self.http_reachable {
            count += 1;
        }
        if self.tls_valid {
            count += 1;
        }
        if self.mx_present {
            count += 1;
        }
        if self.ct_certificate_present {
            count += 1;
        }
        if self.redirect_target.is_some() {
            count += 1;
        }
        if self.brand_content_match {
            count += 1;
        }
        count
    }

    pub(super) fn has_active_infrastructure(&self) -> bool {
        self.active_infrastructure_count() > 0
    }

    /// Confidence in `[0.05, 0.95]` derived from evidence strength.
    ///
    /// Registration alone is 0.30 — deliberately below any warning threshold.
    pub(super) fn confidence(&self) -> f64 {
        if !self.registered {
            return 0.05;
        }
        let mut confidence: f64 = 0.30;
        if self.https_reachable {
            confidence += 0.15;
        }
        if self.http_reachable {
            confidence += 0.05;
        }
        if self.tls_valid {
            confidence += 0.10;
        }
        if self.mx_present {
            confidence += 0.05;
        }
        if self.ct_certificate_present {
            confidence += 0.10;
        }
        if self.redirect_target.is_some() {
            confidence += 0.10;
        }
        if self.brand_content_match {
            confidence += 0.15;
        }
        if self.reputation == LookalikeReputation::Suspicious {
            confidence += 0.10;
        }
        if let Some(age_days) = self.rdap_age_days {
            if age_days < 180 {
                confidence += 0.05;
            }
        }
        confidence.clamp(0.05, 0.95)
    }

    /// Severity derived from evidence. High requires brand impersonation on a
    /// TLS-valid host plus at least one more active signal.
    pub(super) fn severity(&self) -> &'static str {
        let active = self.active_infrastructure_count();
        if self.brand_content_match && self.tls_valid && active >= 3 {
            "high"
        } else if active >= 2 {
            "medium"
        } else {
            "low"
        }
    }

    /// Only evidence-backed candidates above the warning threshold generate a
    /// warning. A registered-but-parked domain never does.
    pub(super) fn should_warn(&self) -> bool {
        self.confidence() >= 0.60
            && self.active_infrastructure_count() >= 2
            && self.severity() != "low"
    }
}

/// Parse an RDAP date, which may be RFC3339 or a bare `YYYY-MM-DD`.
fn parse_rdap_date(raw: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|date| date.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|date_time| date_time.and_utc())
        })
}

/// Best-effort brand-impersonation signal from an untrusted page body.
///
/// The page is treated as suspicious when it mentions the original domain's
/// base label and contains credential-collection markers. This is heuristic
/// telemetry that only ever *adds* evidence; its absence proves nothing.
fn brand_signal_in_body(original_domain: &str, body: &str) -> bool {
    let base = original_domain
        .split('.')
        .next()
        .unwrap_or(original_domain)
        .to_lowercase();
    if base.len() < 3 {
        return false;
    }
    let sample: String = body
        .chars()
        .take(64 * 1024)
        .collect::<String>()
        .to_lowercase();
    let markers = [
        "login", "sign in", "password", "username", "account", "verify",
    ];
    sample.contains(&base) && markers.iter().any(|marker| sample.contains(marker))
}

/// Collect independent evidence for one registered lookalike candidate.
#[allow(clippy::too_many_arguments)]
async fn gather_lookalike_evidence(
    original_domain: &str,
    variant: &str,
    resolver: &StructuredDnsResolver,
    http: &reqwest::Client,
    rdap: &apex_crawl::rdap::RdapClient,
    ct: &apex_crawl::ct::CtMonitor,
) -> LookalikeEvidence {
    // RDAP registration age/registrar (best-effort evidence).
    let (rdap_age_days, rdap_registrar) = match rdap.lookup(variant).await {
        Ok(Some(record)) => {
            let age_days = record
                .registration_date
                .as_deref()
                .and_then(parse_rdap_date)
                .map(|registered| (Utc::now() - registered).num_days());
            (age_days, record.registrar)
        }
        _ => (None, None),
    };

    // MX presence (structured, definite absence vs unknown not needed here:
    // only a positive record adds evidence).
    let mx_present = matches!(resolver.mx(variant).await, DnsLookupOutcome::Records(_));

    // HTTP/TLS reachability with redirects disabled so the redirect target is
    // observable.
    let mut https_reachable = false;
    let mut http_reachable = false;
    let mut tls_valid = false;
    let mut redirect_target = None;
    let mut brand_content_match = false;

    if let Ok(resp) = http.get(format!("https://{variant}/")).send().await {
        // A completed HTTPS request means rustls validated the certificate.
        tls_valid = true;
        let status = resp.status();
        if status.is_redirection() {
            redirect_target = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            https_reachable = true;
        } else if status.is_success() {
            https_reachable = true;
            // Best-effort content heuristic; read failure only loses evidence.
            if let Ok(body) = resp.text().await {
                brand_content_match = brand_signal_in_body(original_domain, &body);
            }
        }
    }
    if !https_reachable {
        if let Ok(resp) = http.get(format!("http://{variant}/")).send().await {
            if resp.status().is_success() {
                http_reachable = true;
                if let Ok(body) = resp.text().await {
                    brand_content_match = brand_signal_in_body(original_domain, &body);
                }
            }
        }
    }

    // Certificate Transparency issuance (structured: parser failures are
    // counted by the CT adapter and simply do not add evidence here).
    let ct_certificate_present = match ct.search_certificates(variant, false).await {
        apex_crawl::parse_outcome::ParseOutcome::ParsedSuccessfully { items } => !items.is_empty(),
        _ => false,
    };

    let reputation = if brand_content_match {
        LookalikeReputation::Suspicious
    } else if https_reachable || http_reachable {
        LookalikeReputation::Neutral
    } else {
        LookalikeReputation::Unknown
    };

    LookalikeEvidence {
        registered: true,
        rdap_age_days,
        rdap_registrar,
        http_reachable,
        https_reachable,
        tls_valid,
        redirect_target,
        mx_present,
        ct_certificate_present,
        brand_content_match,
        reputation,
    }
}

pub(super) async fn run_lookalike_domain_scan(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let companies = match store
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
    {
        Ok(companies) => companies,
        Err(error) => {
            run.fail(&format!(
                "lookalike_domain_scan: failed to load companies (refusing empty scan): {error}"
            ));
            return run;
        }
    };
    let mut domains_scanned = 0u64;
    let mut total_variants = 0u64;
    let mut warned_variants = 0u64;
    let mut evidence_skipped = 0u64;
    let mut lookalike_persist_failures = 0u64;
    let mut observation_write_failures = 0u64;
    let mut warning_ingest_failures = 0u64;
    let mut registration_checks = 0u64;
    let mut registration_lookup_failures = 0u64;
    let mut counters = IngressCounters::default();

    // B329: verify DNS registration before persisting a lookalike. The
    // previous version asserted every generated typosquat variant was an
    // active 0.8-confidence threat — thousands of false positives presented
    // as real findings. Unregistered domains cannot host anything.
    let dns_checker = apex_crawl::dns::DnsChecker::new();
    let resolver = StructuredDnsResolver::new();
    let rdap = apex_crawl::rdap::RdapClient::new();
    let ct = apex_crawl::ct::CtMonitor::new(apex_crawl::ct::CtMonitorConfig::default());
    // Evidence checks (RDAP + HTTP + CT) are much heavier than a DNS
    // round-trip, so they are capped separately from registration checks.
    let max_checks_per_domain: usize = match security_threshold(
        "LOOKALIKE_MAX_CHECKS_PER_DOMAIN",
        15_usize,
        |raw| raw.parse::<usize>().ok().filter(|value| *value > 0),
        "a positive integer",
    ) {
        Ok(value) => value,
        Err(errors) => {
            run.fail(&format!(
                "lookalike_domain_scan: invalid configuration: {errors}"
            ));
            return run;
        }
    };
    let max_evidence_checks: usize = match security_threshold(
        "LOOKALIKE_MAX_EVIDENCE_CHECKS_PER_DOMAIN",
        5_usize,
        |raw| raw.parse::<usize>().ok(),
        "a non-negative integer",
    ) {
        Ok(value) => value,
        Err(errors) => {
            run.fail(&format!(
                "lookalike_domain_scan: invalid configuration: {errors}"
            ));
            return run;
        }
    }
    .min(max_checks_per_domain);

    // Redirects are disabled so the redirect target itself is observable
    // evidence instead of being silently followed.
    let evidence_http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("ApexIntel-LookalikeEvidence/1.0")
        .build()
        .unwrap_or_else(|error| {
            tracing::warn!(
                %error,
                "lookalike_domain_scan: evidence HTTP client build failed; using default client"
            );
            reqwest::Client::new()
        });

    for company in &companies {
        if let Some(domain) = &company.domain {
            let variants: Vec<String> = generate_typosquat_variants(domain);
            tracing::debug!(
                domain = %domain,
                variants = variants.len(),
                company = %company.name,
                "lookalike_domain_scan: variants generated"
            );
            let mut evidence_checks = 0usize;
            for variant in variants.iter().take(max_checks_per_domain) {
                registration_checks += 1;
                // A failed lookup is never "not registered": the variant is
                // simply left unverified and counted, never silently dropped
                // as if DNS had proven it absent.
                match dns_checker.check_lookalike_registration(variant).await {
                    Measurement::Measured(true) => {}
                    Measurement::Measured(false) => continue,
                    Measurement::Unavailable(reason) => {
                        registration_lookup_failures += 1;
                        tracing::debug!(
                            domain = %domain,
                            variant = %variant,
                            reason = %reason.display(),
                            "lookalike_domain_scan: registration lookup failed; registration not asserted"
                        );
                        continue;
                    }
                    Measurement::NotMeasured | Measurement::InsufficientEvidence => {
                        registration_lookup_failures += 1;
                        tracing::debug!(
                            domain = %domain,
                            variant = %variant,
                            "lookalike_domain_scan: registration lookup left no measurement; registration not asserted"
                        );
                        continue;
                    }
                }
                if evidence_checks >= max_evidence_checks {
                    evidence_skipped += 1;
                    continue;
                }
                evidence_checks += 1;

                let evidence = gather_lookalike_evidence(
                    domain,
                    variant,
                    &resolver,
                    &evidence_http,
                    &rdap,
                    &ct,
                )
                .await;
                let confidence = evidence.confidence();
                let active = evidence.has_active_infrastructure();

                // Persist to dedicated lookalike_domains table
                if let Err(e) = store
                    .insert_lookalike_domain(
                        Some(company.id),
                        domain,
                        variant,
                        "typosquat",
                        1,
                        active,
                    )
                    .await
                {
                    lookalike_persist_failures += 1;
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
                if let Err(error) = sqlx::query(
                    r#"DELETE FROM observations
                       WHERE observation_type = 'typosquat' AND entity_id = $1
                         AND value->>'domain' = $2"#,
                )
                .bind(company.id)
                .bind(variant)
                .execute(pool)
                .await
                {
                    observation_write_failures += 1;
                    tracing::warn!(
                        domain = %domain,
                        variant = %variant,
                        error = %error,
                        "lookalike_domain_scan: failed to clear stale typosquat observation"
                    );
                }

                let obs_id = Uuid::new_v4();
                let now = chrono::Utc::now();
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let value = serde_json::json!({
                    "original_domain": domain,
                    "domain": variant,
                    "distance": 1,
                    "threat_type": "typosquat",
                    "active": active,
                    "dns_verified": true,
                    "confidence": confidence,
                    "evidence": evidence,
                });
                #[allow(clippy::unwrap_used, clippy::expect_used)]
                let provenance = serde_json::json!({
                    "source": "worker_lookalike_scan",
                    "content_hash": format!("la_{}_{}", domain, variant),
                });

                if let Err(error) = sqlx::query(
                    r#"INSERT INTO observations
                       (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
                       VALUES ($1, 'typosquat', $2, 'company', $3, $4::jsonb, $5::jsonb, $6)"#,
                )
                .bind(obs_id)
                .bind(company.id)
                .bind(now)
                .bind(value)
                .bind(provenance)
                .bind(confidence)
                .execute(pool)
                .await
                {
                    observation_write_failures += 1;
                    tracing::warn!(
                        domain = %domain,
                        variant = %variant,
                        error = %error,
                        "lookalike_domain_scan: failed to write typosquat observation"
                    );
                }

                // Registration alone is never a warning; emit only when the
                // collected evidence crosses the warning threshold.
                if evidence.should_warn() {
                    let title = format!("Lookalike domain with active infrastructure: {variant}");
                    let description = format!(
                        "Lookalike domain {variant} imitates {domain} and shows active \
                         infrastructure (severity: {}, confidence: {:.2}, active signals: {}, \
                         https reachable: {}, mx: {}, CT certificate: {}, brand content: {}).",
                        evidence.severity(),
                        confidence,
                        evidence.active_infrastructure_count(),
                        evidence.https_reachable,
                        evidence.mx_present,
                        evidence.ct_certificate_present,
                        evidence.brand_content_match,
                    );
                    let mut warning = NewWarning::new("security", &title, evidence.severity())
                        .description(&description)
                        .confidence(confidence);
                    if let Some(target) = evidence.redirect_target.as_deref() {
                        warning = warning.source_urls(vec![target.to_string()]);
                    }
                    warning = warning.entity_ids(vec![company.id]);
                    match ingress.submit_warning(warning).await {
                        Ok(result) => {
                            warned_variants += 1;
                            counters.record(&result);
                        }
                        Err(error) => {
                            warning_ingest_failures += 1;
                            tracing::warn!(
                                %error,
                                domain = %domain,
                                variant = %variant,
                                "lookalike_domain_scan: failed to ingest warning"
                            );
                        }
                    }
                }

                total_variants += 1;
            }
            domains_scanned += 1;
        }
    }
    let summary = format!(
        "lookalike_domain_scan: scanned {} domains, {} registered lookalike variants \
         ({} with warning-level evidence, {} evidence checks skipped by cap, \
         {} lookalike persist failures, {} observation write failures, \
         {} warning ingest failures, {} registration lookups failed of {} attempted); {}",
        domains_scanned,
        total_variants,
        warned_variants,
        evidence_skipped,
        lookalike_persist_failures,
        observation_write_failures,
        warning_ingest_failures,
        registration_lookup_failures,
        registration_checks,
        counters.summary(),
    );
    if lookalike_persist_failures > 0
        || observation_write_failures > 0
        || warning_ingest_failures > 0
    {
        run.items_processed = total_variants;
        run.fail(&format!(
            "{summary} — lookalike persistence or warning ingestion degraded"
        ));
    } else if registration_checks > 0 && registration_lookup_failures == registration_checks {
        // A dependency-wide DNS outage must stay visible without hard-failing
        // the job: five consecutive failures would auto-disable the scan until
        // an operator reset the circuit breaker.
        run.degrade(
            total_variants,
            &format!(
                "{summary} — every lookalike registration lookup failed; nothing was verified"
            ),
        );
    } else if registration_lookup_failures > 0 {
        run.degrade(
            total_variants,
            &format!(
                "{summary} — {} registration lookups failed; those variants are unverified, not absent",
                registration_lookup_failures
            ),
        );
    } else if let Some(reason) = counters.success_blocker() {
        run.degrade(total_variants, &format!("{summary}; {reason}"));
    } else {
        run.succeed(total_variants, &summary);
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookalike_registration_alone_is_low_confidence_and_not_warned() {
        let evidence = LookalikeEvidence::registration_only();
        assert_eq!(evidence.active_infrastructure_count(), 0);
        assert!(!evidence.has_active_infrastructure());
        assert!(
            (evidence.confidence() - 0.30).abs() < 1e-9,
            "registration-only confidence must stay at the 0.30 floor, got {}",
            evidence.confidence()
        );
        assert_eq!(evidence.severity(), "low");
        assert!(
            !evidence.should_warn(),
            "registration alone must never emit a warning"
        );
    }

    #[test]
    fn lookalike_evidence_raises_confidence_and_high_requires_brand_and_tls() {
        let mut evidence = LookalikeEvidence::registration_only();
        evidence.https_reachable = true;
        evidence.tls_valid = true;
        evidence.ct_certificate_present = true;
        evidence.mx_present = true;
        evidence.brand_content_match = true;
        evidence.reputation = LookalikeReputation::Suspicious;
        assert!(evidence.confidence() > 0.8);
        assert_eq!(evidence.severity(), "high");
        assert!(evidence.should_warn());

        // Active infrastructure without brand impersonation must not be high.
        let mut active_without_brand = LookalikeEvidence::registration_only();
        active_without_brand.https_reachable = true;
        active_without_brand.tls_valid = true;
        active_without_brand.mx_present = true;
        assert_eq!(active_without_brand.severity(), "medium");
        assert_ne!(active_without_brand.severity(), "high");
    }

    #[test]
    fn dkim_unknown_is_not_reported_as_missing() {
        let unknown = DkimStatus::Unknown {
            reason: "resolver timeout".to_string(),
        };
        let not_observed = DkimStatus::NotObservedOnKnownSelectors;
        let present = DkimStatus::ConfirmedPresent {
            selectors: vec!["default".to_string()],
        };

        // Unknown DKIM only leaves the definite findings, never "DKIM".
        assert_eq!(
            dns_missing_findings(false, false, &unknown),
            vec!["SPF", "DMARC"]
        );
        assert_eq!(
            dns_missing_findings(true, true, &not_observed),
            vec!["DKIM"]
        );
        assert!(dns_missing_findings(true, true, &present).is_empty());
        assert_eq!(dns_unknown_notes(false, false, &unknown).len(), 1);
        assert!(dns_unknown_notes(false, false, &not_observed).is_empty());
    }

    #[test]
    fn brand_signal_requires_base_label_and_credential_markers() {
        assert!(brand_signal_in_body(
            "acme.com",
            "<html><form>Sign in to your Acme account with your password</form></html>"
        ));
        assert!(!brand_signal_in_body(
            "acme.com",
            "<h1>Totally unrelated page</h1>"
        ));
        assert!(!brand_signal_in_body(
            "acme.com",
            "<h1>Acme — about our history</h1>"
        ));
    }

    #[test]
    fn parse_rdap_date_accepts_rfc3339_and_bare_dates() {
        assert!(parse_rdap_date("2024-03-01T10:00:00Z").is_some());
        assert!(parse_rdap_date("2024-03-01").is_some());
        assert!(parse_rdap_date("not-a-date").is_none());
    }

    #[test]
    fn malformed_security_threshold_is_a_configuration_error() {
        use std::sync::{LazyLock, Mutex};
        static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let name = "SANCTIONS_THRESHOLD";
        std::env::set_var(name, "not-a-number");
        let result = security_threshold(
            name,
            0.92_f64,
            |raw| {
                raw.parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            },
            "a finite number in 0..=1",
        );
        std::env::remove_var(name);

        let errors = result.expect_err("not-a-number is not a threshold");
        assert_eq!(errors.errors[0].variable, name);

        // Unset variables still keep their defaults.
        let default = security_threshold(name, 0.92_f64, |_| None, "anything")
            .expect("absent variable keeps the default");
        assert!((default - 0.92).abs() < f64::EPSILON);
    }
}
