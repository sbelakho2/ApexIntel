//! Threat Intelligence Refresh handler.
//!
//! Scheduled every 6 hours: updates supply chain risk scores, threat actor
//! profiles, and competitive intelligence across all tracked entities.
//! Writes structured threat intelligence records to the database.
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use chrono::{Datelike, TimeZone};
use uuid::Uuid;

use crate::intelligence_ingress::{
    IngressCounters, IntelligenceIngress, NewWarning, WarningSubmissionResult,
};
use crate::*;

#[cfg(feature = "llm")]
use apex_threat_intel::models::{IndustrySector, PaginationParams};
#[cfg(feature = "llm")]
use apex_threat_intel::threat_actor_database::{ActorStatus, ThreatActor};

/// Signal class for the weekly supply-chain warning dedup key.
const SUPPLY_CHAIN_SIGNAL_CLASS: &str = "supply_chain";

/// Marker that carries the deterministic weekly key inside a warning
/// description, so a later run can recover it from the `warnings` table.
const SUPPLY_CHAIN_KEY_MARKER: &str = "weekly-key=";

/// Disruption phrases that count as a supply-chain signal.
///
/// Bare words like "delay" or "supply chain" appear in ordinary company
/// profiles ("provides supply chain software") and are not disruption
/// evidence; only concrete disruption phrases raise the heuristic (#168).
const SUPPLY_CHAIN_DISRUPTION_PHRASES: &[&str] = &[
    "supply chain disruption",
    "supply chain shortage",
    "supply chain delay",
    "supply disruption",
    "supply shortage",
    "supply delay",
    "production halted",
    "production halt",
    "production shutdown",
    "factory shutdown",
    "factory closure",
    "plant shutdown",
    "plant closure",
    "port congestion",
    "port closure",
    "shipping delay",
    "shipment delay",
    "logistics disruption",
    "raw material shortage",
    "material shortage",
    "component shortage",
    "parts shortage",
    "inventory shortage",
    "supplier bankruptcy",
    "supplier insolvency",
    "force majeure",
    "export ban",
    "export restriction",
];

/// Per-run warning-ingestion accounting for the threat-intel stage.
///
/// Mirrors the hard rule the security jobs enforce through
/// [`IngressCounters::success_blocker`]: a run that attempted warning
/// submission and had any failed submit must not report success, because
/// `/admin/jobs` would otherwise show a clean ThreatIntelRefresh while
/// actionable alerts were never created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WarningIngestTally {
    /// `submit_warning` calls made.
    attempted: u64,
    /// Calls that returned a persisted warning result.
    succeeded: u64,
    /// Calls that failed before anything was persisted.
    failed: u64,
    /// Warnings skipped because the weekly dedup key was already recorded.
    weekly_deduplicated: u64,
    /// Per-outcome counters for the successful submissions.
    counters: IngressCounters,
}

impl WarningIngestTally {
    fn record_success(&mut self, result: &WarningSubmissionResult) {
        self.attempted += 1;
        self.succeeded += 1;
        self.counters.record(result);
    }

    fn record_failure(&mut self) {
        self.attempted += 1;
        self.failed += 1;
    }

    fn record_weekly_dedup(&mut self) {
        self.weekly_deduplicated += 1;
    }

    /// The reason this run must not report success, if any.
    ///
    /// A failed submit is a hard blocker; persisted warnings with zero
    /// completed triage submissions are the degraded case the DNS and
    /// sanctions jobs already treat as non-success.
    fn success_blocker(&self) -> Option<String> {
        if self.failed > 0 {
            return Some(format!(
                "{} warning submit attempt(s), {} failed — actionable alerts may be missing",
                self.attempted, self.failed
            ));
        }
        self.counters.success_blocker()
    }

    /// One-line counter summary for job notes.
    fn summary(&self) -> String {
        format!(
            "warning_ingest: attempted={} succeeded={} failed={} weekly_deduplicated={}; {}",
            self.attempted,
            self.succeeded,
            self.failed,
            self.weekly_deduplicated,
            self.counters.summary(),
        )
    }
}

/// Apply the warning-ingestion and observation-persistence outcomes to the run.
///
/// Pure and unit-testable: any failed warning submit fails the run (matching
/// the DNS/sanctions hard rule), threat-actor match persistence failures
/// degrade, and the ingress `success_blocker` degrades rather than hiding a
/// persisted-warning / zero-triage pipeline outage.
fn finish_threat_intel_run(
    run: &mut JobRun,
    tally: &WarningIngestTally,
    threat_match_failures: u64,
    items: u64,
    summary: &str,
) {
    if tally.failed > 0 {
        run.items_processed = items;
        run.fail(&format!(
            "{summary} — {} of {} warning submit attempt(s) failed \
             (warning_ingest: attempted={} succeeded={} failed={})",
            tally.failed, tally.attempted, tally.attempted, tally.succeeded, tally.failed
        ));
    } else if threat_match_failures > 0 {
        run.degrade(
            items,
            &format!(
                "{summary} — {threat_match_failures} threat actor match persistence failure(s)"
            ),
        );
    } else if let Some(reason) = tally.counters.success_blocker() {
        run.degrade(items, &format!("{summary}; {reason}"));
    } else {
        run.succeed(items, summary);
    }
}

/// Deterministic weekly dedup key: signal class + company + ISO week.
fn supply_chain_weekly_key(company_id: Uuid, week: chrono::IsoWeek) -> String {
    format!(
        "{SUPPLY_CHAIN_SIGNAL_CLASS}:{company_id}:{}-W{:02}",
        week.year(),
        week.week()
    )
}

/// UTC Monday 00:00 of the ISO week containing `now`.
fn iso_week_start(now: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    let date = now.date_naive();
    let monday = date - chrono::Duration::days(i64::from(date.weekday().num_days_from_monday()));
    chrono::Utc.from_utc_datetime(&monday.and_time(chrono::NaiveTime::MIN))
}

/// Recover the weekly dedup key embedded in a warning description.
fn extract_supply_chain_weekly_key(description: &str) -> Option<String> {
    let (_, rest) = description.split_once(SUPPLY_CHAIN_KEY_MARKER)?;
    let key: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
        .collect();
    if key.is_empty() {
        None
    } else {
        Some(key)
    }
}

/// Whether a weekly key was already recorded for the current ISO week.
///
/// A same-week re-run of the job must not emit a second supply-chain warning
/// for the same company and signal class (#168).
fn weekly_key_recorded(recorded: &HashSet<String>, key: &str) -> bool {
    recorded.contains(key)
}

/// Match a lowercased text against the concrete disruption phrases.
fn has_supply_chain_disruption_phrase(text_lower: &str) -> bool {
    SUPPLY_CHAIN_DISRUPTION_PHRASES
        .iter()
        .any(|phrase| text_lower.contains(phrase))
}

/// Normalize a URL or bare host into a comparable lowercase host without
/// scheme, userinfo, port, path, query or a leading `www.`.
fn normalized_host(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let after_scheme = trimmed.split_once("://").map_or(trimmed, |(_, rest)| rest);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_port.split(':').next().unwrap_or_default();
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// Whether `source` is the company's own website. The company's own site is
/// not independent corroboration of a disruption and must not feed the
/// heuristic (#168).
fn is_company_own_source(source: &str, company_domain: Option<&str>) -> bool {
    let Some(company_domain) = company_domain else {
        return false;
    };
    let (Some(source_host), Some(company_host)) =
        (normalized_host(source), normalized_host(company_domain))
    else {
        return false;
    };
    source_host == company_host || source_host.ends_with(&format!(".{company_host}"))
}

/// The weekly dedup keys recorded for a company.
///
/// A key is recovered from the descriptions of existing supply-chain warnings.
/// Additionally, any supply-chain warning for the company whose `ts_utc` falls
/// inside the current ISO week counts as the current key being recorded even
/// when the store merged the submission into an older warning row (the merge
/// updates `ts_utc` but keeps the old description).
async fn recorded_supply_chain_weekly_keys(
    store: &PgStore,
    company_id: Uuid,
    current_key: &str,
    week_start: chrono::DateTime<chrono::Utc>,
) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<(String, bool)> = sqlx::query_as(
        r#"SELECT COALESCE(description, '') AS description,
                  COALESCE(ts_utc, created_at) >= $2 AS submitted_this_week
           FROM warnings
           WHERE deleted_at IS NULL
             AND warning_type = 'supply_chain'
             AND entity_ids @> ARRAY[$1]::uuid[]
             AND (
                 COALESCE(ts_utc, created_at) >= $2
                 OR POSITION($3 IN COALESCE(description, '')) > 0
             )
           ORDER BY created_at DESC
           LIMIT 100"#,
    )
    .bind(company_id)
    .bind(week_start)
    .bind(SUPPLY_CHAIN_KEY_MARKER)
    .fetch_all(&store.pool)
    .await?;

    let mut recorded: HashSet<String> = rows
        .iter()
        .filter_map(|(description, _)| extract_supply_chain_weekly_key(description))
        .collect();
    // A submission merged into an older row during this ISO week still means
    // this company and signal class were already warned about this week.
    if rows
        .iter()
        .any(|(_, submitted_this_week)| *submitted_this_week)
    {
        recorded.insert(current_key.to_string());
    }
    Ok(recorded)
}

/// Execute ThreatIntelRefresh job: refresh threat intel across all tracked entities.
pub(super) async fn run_threat_intel_refresh(
    kind: &JobKind,
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let total_start = Instant::now();

    // Every company: list calls clamp to 500 rows, and a name-ordered cap
    // would never reach companies past the first page.
    let companies = match store.list_all_companies().await {
        Ok(companies) => companies,
        Err(error) => {
            run.fail(&format!(
                "threat_intel_refresh: failed to load companies (refusing empty assessment): {error}"
            ));
            return run;
        }
    };

    if companies.is_empty() {
        run.skip("threat_intel_refresh: no companies found in database");
        return run;
    }

    let mut total_scores_updated: u64 = 0;
    let mut supply_chain_risks: u64 = 0;
    let mut threat_actor_matches: u64 = 0;
    let mut threat_match_failures: u64 = 0;
    let mut competitive_flags: u64 = 0;
    let mut warning_tally = WarningIngestTally::default();

    // The curated, verified threat-actor database is static intelligence, so it
    // is built once and the resulting actor set is reused across every company
    // assessment (no per-company reconstruction).
    #[cfg(feature = "llm")]
    let known_threat_actors: Vec<ThreatActor> =
        apex_threat_intel::threat_actor_database::ThreatActorDatabase::with_known_actors()
            .list_actors(PaginationParams {
                offset: 0,
                limit: 10_000,
            })
            .items;

    let activity_logger = apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());

    for company in &companies {
        // Assess supply chain risk heuristically from observations. A failed
        // observation read is an input failure, not "no risk observed".
        let heuristic_risk = match assess_supply_chain_heuristic(
            store,
            ingress,
            company,
            &mut warning_tally,
        )
        .await
        {
            Ok(risk) => risk,
            Err(error) => {
                run.fail(&format!(
                    "threat_intel_refresh: supply-chain observation read failed for {}: {error}",
                    company.name
                ));
                return run;
            }
        };
        if heuristic_risk > 0 {
            supply_chain_risks += heuristic_risk as u64;
            total_scores_updated += 1;
            // Log threat detection to activity feed (fire-and-forget)
            activity_logger
                .log_threat_detected(
                    "supply_chain_heuristic",
                    &company.name,
                    if heuristic_risk >= 5 {
                        "high"
                    } else {
                        "medium"
                    },
                    Some(&company.id.to_string()),
                    Some("company"),
                )
                .await;
        }

        // Threat actor profile matching against the curated, verified
        // threat-actor database (sector overlap AND target-region overlap).
        #[cfg(feature = "llm")]
        {
            match assess_threat_actor_matches(
                store,
                ingress,
                company,
                &known_threat_actors,
                &mut warning_tally,
            )
            .await
            {
                Ok(matches) if matches > 0 => {
                    threat_actor_matches += matches as u64;
                    activity_logger
                        .log_threat_detected(
                            "threat_actor_match",
                            &company.name,
                            "medium",
                            Some(&company.id.to_string()),
                            Some("company"),
                        )
                        .await;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(
                        company = %company.name,
                        %error,
                        "threat_intel_refresh: threat actor match persistence failed"
                    );
                    threat_match_failures += 1;
                }
            }
        }

        // Competitive intelligence assessment for competitor-tagged companies
        let is_competitor = company
            .metadata
            .as_ref()
            .and_then(|m| m.get("is_competitor"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_competitor {
            competitive_flags += 1;
        }
    }

    let elapsed = total_start.elapsed();
    let summary = format!(
        "threat_intel_refresh: {} companies assessed, {} supply chain risks, \
         {} threat actor matches, {} competitive flags, {} match persistence failure(s), \
         {} in {:.1}s",
        companies.len(),
        supply_chain_risks,
        threat_actor_matches,
        competitive_flags,
        threat_match_failures,
        warning_tally.summary(),
        elapsed.as_secs_f64(),
    );
    finish_threat_intel_run(
        &mut run,
        &warning_tally,
        threat_match_failures,
        total_scores_updated,
        &summary,
    );
    run
}

async fn assess_supply_chain_heuristic(
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
    company: &apex_store::postgres::CompanyRow,
    tally: &mut WarningIngestTally,
) -> Result<usize, sqlx::Error> {
    let since = chrono::Utc::now() - chrono::Duration::days(30);
    let rows = sqlx::query(
        r#"SELECT COALESCE(value->>'description', value->>'text_content', '') AS text,
                  COALESCE(
                      NULLIF(value->>'url', ''),
                      NULLIF(value->>'source_url', ''),
                      NULLIF(provenance->>'source_url', ''),
                      NULLIF(provenance->>'url', ''),
                      ''
                  ) AS source_url
           FROM observations
           WHERE entity_id = $1 AND entity_type = 'company' AND created_at >= $2
           LIMIT 50"#,
    )
    .bind(company.id)
    .bind(since)
    .fetch_all(&store.pool)
    .await?;

    let mut disruption_count = 0usize;
    for row in &rows {
        use sqlx::Row;
        // An unreadable text column is a decode failure: the caller must not
        // read it as "no disruption evidence".
        let text: String = row.try_get("text")?;
        let source_url: String = row.try_get("source_url")?;
        // The company's own website is not an independent source: an article
        // on its own site must not raise its supply-chain risk (#168).
        if is_company_own_source(&source_url, company.domain.as_deref()) {
            continue;
        }
        // Concrete disruption phrases only; generic words like "delay" or
        // "supply chain" are not disruption evidence.
        if has_supply_chain_disruption_phrase(&text.to_lowercase()) {
            disruption_count += 1;
        }
    }

    if disruption_count > 0 {
        let now = chrono::Utc::now();
        // B326: stable ID per (company, signal count) — this self-feeding
        // heuristic no longer inserts a fresh row every 6h for the same
        // underlying observations.
        let obs_id = apex_core::entities::Observation::deterministic_id(
            "supply_heuristic",
            &format!("{}|{}", company.id, disruption_count),
        );
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let value = serde_json::json!({
            "company_name": company.name,
            "risk_type": "supply_chain_heuristic",
            "disruption_signals": disruption_count,
            "assessment_method": "heuristic",
        });
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let provenance = serde_json::json!({
            "source": "worker_threat_intel_refresh_heuristic",
        });

        // Authoritative persistence: this observation feeds recipe evaluation,
        // so an insert failure propagates instead of being swallowed.
        sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
               VALUES ($1, 'supply_chain_heuristic', $2, 'company', $3, $4::jsonb, $5::jsonb, $6)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(obs_id)
        .bind(company.id)
        .bind(now)
        .bind(value)
        .bind(provenance)
        .bind(0.55)
        .execute(&store.pool)
        .await?;

        if disruption_count >= 3 {
            // Weekly dedup: a repeated run in the same ISO week for the same
            // company and signal class must not create a second warning. The
            // deterministic key is embedded in the persisted warning's
            // description, and any warning submitted for the company during
            // the week counts even when the store merged it into an older row
            // (#168).
            let weekly_key = supply_chain_weekly_key(company.id, now.iso_week());
            let recorded = recorded_supply_chain_weekly_keys(
                store,
                company.id,
                &weekly_key,
                iso_week_start(now),
            )
            .await?;
            if weekly_key_recorded(&recorded, &weekly_key) {
                tally.record_weekly_dedup();
                return Ok(disruption_count);
            }

            let title = format!(
                "Supply Chain Risk: {} — {} disruption signals",
                company.name, disruption_count
            );
            let description = format!(
                "{} shows {} supply chain disruption signals in the last 30 days. \
                 Review recommended. {SUPPLY_CHAIN_KEY_MARKER}{weekly_key}",
                company.name, disruption_count
            );
            let severity = if disruption_count >= 5 {
                "high"
            } else {
                "medium"
            };
            let mut warning = NewWarning::new("supply_chain", &title, severity)
                .description(&description)
                .confidence(0.65)
                // Entity-scoped: resolve the affected company's subscribers.
                .entity_ids(vec![company.id]);
            if let Some(region) = company.region.as_deref() {
                warning = warning.region(region);
            }
            match ingress.submit_warning(warning).await {
                Ok(result) => tally.record_success(&result),
                Err(error) => {
                    tally.record_failure();
                    tracing::warn!(
                        %error,
                        company = %company.name,
                        "threat_intel_refresh: failed to ingest supply chain warning"
                    );
                }
            }
        }
    }
    Ok(disruption_count)
}

/// Collect the industry sectors a company operates in from its stored
/// `industry_tags` plus any `industry` / `sector` / `industry_tags` entries in
/// its metadata. Unknown tags map to [`IndustrySector::Other`] and simply fail
/// to overlap any actor's concrete target sectors.
#[cfg(feature = "llm")]
fn company_industry_sectors(company: &apex_store::postgres::CompanyRow) -> Vec<IndustrySector> {
    let mut raw: Vec<String> = Vec::new();
    if let Some(tags) = &company.industry_tags {
        raw.extend(tags.iter().cloned());
    }
    if let Some(meta) = &company.metadata {
        if let Some(v) = meta.get("industry").and_then(|v| v.as_str()) {
            raw.push(v.to_string());
        }
        if let Some(v) = meta.get("sector").and_then(|v| v.as_str()) {
            raw.push(v.to_string());
        }
        if let Some(arr) = meta.get("industry_tags").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    raw.push(s.to_string());
                }
            }
        }
    }

    let mut sectors: Vec<IndustrySector> = Vec::new();
    for tag in &raw {
        let parsed = IndustrySector::from_str(tag);
        if !sectors.contains(&parsed) {
            sectors.push(parsed);
        }
    }
    sectors
}

/// Convert a slice of static strings into owned strings (keeps the geographic
/// expansion tables compact and readable).
#[cfg(feature = "llm")]
fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// Expand an ISO-3166 alpha-2 country code into the broader region tokens it
/// belongs to, so a company's country can overlap an actor's regional targeting.
#[cfg(feature = "llm")]
fn country_expansions(code: &str) -> Vec<String> {
    match code.trim().to_uppercase().as_str() {
        "US" | "USA" => strs(&["NORTH_AMERICA", "NA", "AMERICAS"]),
        "CA" | "MX" => strs(&["NORTH_AMERICA", "NA", "AMERICAS"]),
        "GB" | "UK" => strs(&["EUROPE", "EU", "UK"]),
        "DE" | "FR" | "IT" | "ES" | "NL" | "PL" | "SE" | "NO" | "FI" | "DK" | "BE" | "AT"
        | "IE" | "PT" | "CH" | "GR" | "CZ" => strs(&["EUROPE", "EU"]),
        "RU" => strs(&["EURASIA"]),
        "CN" | "JP" | "KR" | "TW" | "KP" => strs(&["APAC", "EAST_ASIA", "ASIA"]),
        "IN" | "PK" | "BD" | "LK" => strs(&["APAC", "SOUTH_ASIA", "ASIA"]),
        "SG" | "MY" | "ID" | "TH" | "VN" | "PH" => strs(&["APAC", "SOUTHEAST_ASIA", "ASIA"]),
        "AU" | "NZ" => strs(&["APAC", "OCEANIA"]),
        "IL" | "IR" | "IQ" | "SA" | "AE" | "TR" | "QA" | "KW" | "JO" | "LB" | "SY" | "YE"
        | "PS" => strs(&["ME", "MIDDLE_EAST", "MENA"]),
        "EG" => strs(&["ME", "MIDDLE_EAST", "MENA", "AFRICA"]),
        "BR" | "AR" | "CL" | "CO" | "PE" | "VE" | "UY" => {
            strs(&["LATAM", "SOUTH_AMERICA", "AMERICAS"])
        }
        "ZA" | "NG" | "KE" | "GH" | "ET" | "MA" => strs(&["AFRICA"]),
        _ => Vec::new(),
    }
}

/// Expand a free-form region name (e.g. "North America", "APAC") into the
/// concrete country / sub-region tokens it covers.
#[cfg(feature = "llm")]
fn region_expansions(region: &str) -> Vec<String> {
    match region.trim().to_uppercase().as_str() {
        "NORTH AMERICA" | "NORTH_AMERICA" | "NA" | "AMERICAS" => {
            strs(&["US", "CA", "MX", "NORTH_AMERICA"])
        }
        "EUROPE" | "EU" | "EUROPEAN UNION" | "EEA" => strs(&["EU", "EUROPE", "UK", "GB", "DE"]),
        "ASIA PACIFIC" | "ASIA_PACIFIC" | "ASIA-PACIFIC" | "APAC" => {
            strs(&["APAC", "CN", "JP", "ASIA"])
        }
        "ASIA" => strs(&["ASIA", "APAC"]),
        "EAST ASIA" | "EAST_ASIA" => strs(&["CN", "JP", "KR", "KP", "EAST_ASIA"]),
        "SOUTH ASIA" | "SOUTH_ASIA" => strs(&["IN", "SOUTH_ASIA"]),
        "SOUTHEAST ASIA" | "SOUTHEAST_ASIA" | "SEA" => strs(&["SG", "SOUTHEAST_ASIA"]),
        "MIDDLE EAST" | "MIDDLE_EAST" | "ME" | "MENA" | "MIDDLE EAST AND NORTH AFRICA" => {
            strs(&["ME", "MIDDLE_EAST", "IR", "IL", "SA"])
        }
        "LATIN AMERICA" | "LATIN_AMERICA" | "LATAM" | "SOUTH AMERICA" | "SOUTH_AMERICA" => {
            strs(&["LATAM", "SOUTH_AMERICA", "BR"])
        }
        "AFRICA" | "SUB-SAHARAN AFRICA" => strs(&["AFRICA", "ZA"]),
        "EURASIA" | "RUSSIA" => strs(&["RU", "EURASIA"]),
        "OCEANIA" | "AUSTRALASIA" => strs(&["AU", "OCEANIA"]),
        _ => Vec::new(),
    }
}

/// Build the canonical set of uppercase geographic tokens for a company from
/// its country code and region, used for overlap comparison against a threat
/// actor's `target_regions` (never its attributed country).
#[cfg(feature = "llm")]
fn company_geo_tokens(country: Option<&str>, region: Option<&str>) -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    if let Some(c) = country {
        raw.push(c.to_string());
        raw.extend(country_expansions(c));
    }
    if let Some(r) = region {
        raw.push(r.to_string());
        raw.extend(region_expansions(r));
    }

    let mut tokens: Vec<String> = Vec::new();
    for s in &raw {
        let t = s.trim().to_uppercase();
        if !t.is_empty() && !tokens.contains(&t) {
            tokens.push(t);
        }
    }
    tokens
}

/// Build the set of uppercase geographic tokens for a threat actor from its
/// target regions only. The actor's `attributed_country` describes origin, not
/// an operational target, so it never contributes geographic matching (#167).
#[cfg(feature = "llm")]
fn actor_geo_tokens(actor: &ThreatActor) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for r in &actor.target_regions {
        let t = r.trim().to_uppercase();
        if !t.is_empty() && !tokens.contains(&t) {
            tokens.push(t);
        }
    }
    tokens
}

/// Determine whether a threat actor is relevant to a company and, if so, return
/// the human-readable match reasons.
///
/// A match requires BOTH a concrete sector overlap AND a concrete target-region
/// overlap: an actor known to target the company's sector but not any region it
/// operates in (or the reverse) is not relevant. `attributed_country` is not an
/// operational target and never contributes geography (#167). Defunct actors
/// are never matched. A bare "GLOBAL" target region does not, on its own, match
/// every company — a concrete token must overlap.
#[cfg(feature = "llm")]
fn actor_match_reasons(
    actor: &ThreatActor,
    company_sectors: &[IndustrySector],
    company_geo: &[String],
) -> Option<Vec<String>> {
    if actor.status == ActorStatus::Defunct {
        return None;
    }

    let mut reasons: Vec<String> = Vec::new();

    for sector in &actor.target_sectors {
        if company_sectors.contains(sector) {
            reasons.push(format!("sector:{}", sector.as_str()));
        }
    }
    if reasons.is_empty() {
        // Geographic overlap without a sector match is not actionable.
        return None;
    }
    let sector_reason_count = reasons.len();

    for token in actor_geo_tokens(actor) {
        if token.as_str() != "GLOBAL" && company_geo.contains(&token) {
            reasons.push(format!("geo:{}", token));
        }
    }
    if reasons.len() == sector_reason_count {
        // No concrete target-region overlap: sector alone is not a match.
        return None;
    }

    reasons.dedup();
    Some(reasons)
}

/// Assess a company against every known threat actor, recording a
/// `threat_actor_match` observation (and a high-severity warning for active,
/// high-sophistication actors that directly target the company's sector).
/// Returns the number of matches recorded, or an error when a match could not
/// be persisted (the caller degrades the run instead of reporting a clean
/// assessment that silently dropped rows).
#[cfg(feature = "llm")]
async fn assess_threat_actor_matches(
    store: &Arc<PgStore>,
    ingress: &Arc<IntelligenceIngress>,
    company: &apex_store::postgres::CompanyRow,
    known_actors: &[ThreatActor],
    tally: &mut WarningIngestTally,
) -> Result<usize, String> {
    let company_sectors = company_industry_sectors(company);
    let company_geo =
        company_geo_tokens(company.country_code.as_deref(), company.region.as_deref());

    let mut matches = 0usize;
    for actor in known_actors {
        let Some(reasons) = actor_match_reasons(actor, &company_sectors, &company_geo) else {
            continue;
        };

        let now = chrono::Utc::now();
        // B326: stable ID per (company, actor) — matches re-computed every
        // 6h previously re-inserted identical rows.
        let obs_id = apex_core::entities::Observation::deterministic_id(
            "threat_match",
            &format!("{}|{}", company.id, actor.alias),
        );
        let threat_actor_label = match &actor.name {
            Some(n) => format!("{} ({})", actor.alias, n),
            None => actor.alias.clone(),
        };
        let target_sectors: Vec<&str> = actor.target_sectors.iter().map(|s| s.as_str()).collect();
        let primary_sector = target_sectors.first().copied().unwrap_or("company's");
        let sector_match = reasons.iter().any(|r| r.starts_with("sector:"));

        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let value = serde_json::json!({
            "company_name": company.name.as_str(),
            "threat_actor": threat_actor_label.clone(),
            "threat_actor_alias": actor.alias.clone(),
            "attributed_country": actor.attributed_country.clone(),
            "motivation": actor.motivation.as_str(),
            "target_sectors": target_sectors,
            "match_reasons": reasons,
            "sophistication_level": actor.sophistication_level,
            "assessment_method": "database_match",
        });
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        let provenance = serde_json::json!({
            "source": "worker_threat_intel_refresh",
        });

        sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc, value, provenance, confidence)
               VALUES ($1, 'threat_actor_match', $2, 'company', $3, $4::jsonb, $5::jsonb, $6)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(obs_id)
        .bind(company.id)
        .bind(now)
        .bind(value)
        .bind(provenance)
        .bind(0.6)
        .execute(&store.pool)
        .await
        .map_err(|error| {
            format!(
                "failed to persist threat actor match {} for {}: {error}",
                actor.alias, company.name
            )
        })?;

        // Surface the most actionable matches (active, high-sophistication
        // actors directly targeting the company's sector) as warnings.
        if sector_match && actor.status == ActorStatus::Active && actor.sophistication_level >= 8 {
            let title = format!(
                "Threat Actor Match: {} may target {}",
                threat_actor_label, company.name
            );
            let description = format!(
                "{} is an active, high-sophistication threat actor attributed to {} that is known to target the {} sector, which overlaps this company's profile. Motivation: {}. MITRE ATT&CK techniques are documented in the threat-actor database.",
                threat_actor_label,
                actor.attributed_country.as_deref().unwrap_or("an unknown country"),
                primary_sector,
                actor.motivation.as_str(),
            );
            let mut warning = NewWarning::new("threat_actor", &title, "high")
                .description(&description)
                .confidence(0.6)
                // Entity-scoped: resolve the affected company's subscribers.
                .entity_ids(vec![company.id]);
            if let Some(region) = company.region.as_deref() {
                warning = warning.region(region);
            }
            match ingress.submit_warning(warning).await {
                Ok(result) => tally.record_success(&result),
                Err(error) => {
                    tally.record_failure();
                    tracing::warn!(
                        %error,
                        company = %company.name,
                        "threat_intel_refresh: failed to ingest threat actor warning"
                    );
                }
            }
        }

        matches += 1;
    }

    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_failed_warning_submit_makes_the_run_not_succeed() {
        let mut run = JobRun::new(JobKind::ThreatIntelRefresh);
        run.start();
        let tally = WarningIngestTally {
            attempted: 2,
            succeeded: 1,
            failed: 1,
            weekly_deduplicated: 0,
            counters: IngressCounters::default(),
        };

        finish_threat_intel_run(&mut run, &tally, 0, 7, "threat_intel_refresh: summary");

        assert!(
            matches!(run.status, JobStatus::Failed { .. }),
            "a run with one failed warning submit must not report success, got {:?}",
            run.status
        );
        assert!(
            run.notes.contains("attempted=2") && run.notes.contains("failed=1"),
            "failure reason must carry the submit counts, got {:?}",
            run.notes
        );
        assert_eq!(run.items_processed, 7);
    }

    #[test]
    fn no_warning_attempts_still_succeeds() {
        let mut run = JobRun::new(JobKind::ThreatIntelRefresh);
        run.start();
        let tally = WarningIngestTally::default();
        finish_threat_intel_run(&mut run, &tally, 0, 0, "threat_intel_refresh: summary");
        assert!(matches!(run.status, JobStatus::Succeeded { .. }));
    }

    #[test]
    fn disruption_detection_requires_concrete_phrases() {
        assert!(has_supply_chain_disruption_phrase(
            "a supply chain disruption halted shipments"
        ));
        assert!(has_supply_chain_disruption_phrase(
            "port congestion worsened"
        ));
        assert!(has_supply_chain_disruption_phrase(
            "the supplier declared force majeure"
        ));
        assert!(
            !has_supply_chain_disruption_phrase("acme provides supply chain software"),
            "generic 'supply chain' text must not count as disruption"
        );
        assert!(
            !has_supply_chain_disruption_phrase("a short delay was reported"),
            "a bare 'delay' must not count as disruption"
        );
        assert!(
            !has_supply_chain_disruption_phrase("the company reported a disruption"),
            "a bare 'disruption' must not count as disruption"
        );
    }

    #[test]
    fn company_own_website_is_not_an_independent_source() {
        assert!(is_company_own_source(
            "https://www.acme.example/news/disruption-update",
            Some("acme.example")
        ));
        assert!(is_company_own_source(
            "https://news.acme.example/story",
            Some("https://acme.example")
        ));
        assert!(!is_company_own_source(
            "https://wire.example/story",
            Some("acme.example")
        ));
        assert!(
            !is_company_own_source("https://acme.example.evil.test/story", Some("acme.example")),
            "a lookalike suffix must not be treated as the company's own site"
        );
        assert!(!is_company_own_source("", Some("acme.example")));
        assert!(!is_company_own_source("https://wire.example/story", None));
    }

    #[test]
    fn weekly_key_is_deterministic_and_scoped_to_company_and_week() {
        let company = Uuid::new_v4();
        let other_company = Uuid::new_v4();
        let week = chrono::Utc::now().iso_week();
        let key = supply_chain_weekly_key(company, week);

        assert_eq!(
            key,
            supply_chain_weekly_key(company, week),
            "the weekly key must be deterministic"
        );
        assert_ne!(key, supply_chain_weekly_key(other_company, week));
        let next_week = chrono::Utc::now() + chrono::Duration::days(7);
        assert_ne!(
            key,
            supply_chain_weekly_key(company, next_week.iso_week()),
            "a different ISO week must produce a different key"
        );
        assert!(key.starts_with(SUPPLY_CHAIN_SIGNAL_CLASS));

        let description = format!("signals observed. {SUPPLY_CHAIN_KEY_MARKER}{key}");
        assert_eq!(
            extract_supply_chain_weekly_key(&description).as_deref(),
            Some(key.as_str())
        );
        assert_eq!(extract_supply_chain_weekly_key("no key here"), None);
    }

    #[test]
    fn iso_week_start_is_monday_midnight_and_contains_now() {
        let now = chrono::Utc::now();
        let start = iso_week_start(now);
        assert_eq!(start.weekday(), chrono::Weekday::Mon);
        assert_eq!(start.time(), chrono::NaiveTime::MIN);
        assert!(start <= now);
        assert!(
            now < start + chrono::Duration::days(7),
            "the ISO week window must contain now"
        );
    }

    #[test]
    fn same_week_rerun_does_not_emit_a_second_warning() {
        let company = Uuid::new_v4();
        let week = chrono::Utc::now().iso_week();
        let key = supply_chain_weekly_key(company, week);

        // Run 1: nothing recorded yet, so the warning is emitted and the
        // persisted description carries the weekly key.
        let recorded_before: HashSet<String> = HashSet::new();
        assert!(!weekly_key_recorded(&recorded_before, &key));
        let stored_description = format!("Supply Chain Risk. {SUPPLY_CHAIN_KEY_MARKER}{key}");
        let recorded_after_run_1: HashSet<String> =
            [extract_supply_chain_weekly_key(&stored_description)
                .expect("run 1 persisted its weekly key")]
            .into_iter()
            .collect();

        // Run 2 in the same ISO week: the recorded key suppresses a duplicate.
        assert!(
            weekly_key_recorded(&recorded_after_run_1, &key),
            "a same-week re-run must find the recorded key and skip the warning"
        );

        // A new ISO week has a new key and may warn again.
        let next_week = chrono::Utc::now() + chrono::Duration::days(7);
        let next_key = supply_chain_weekly_key(company, next_week.iso_week());
        assert!(!weekly_key_recorded(&recorded_after_run_1, &next_key));
    }

    #[cfg(feature = "llm")]
    mod actor_matching {
        use super::*;
        use apex_threat_intel::threat_actor_database::ActorMotivation;

        fn test_actor(
            status: ActorStatus,
            sectors: Vec<IndustrySector>,
            regions: &[&str],
            attributed_country: Option<&str>,
        ) -> ThreatActor {
            let mut actor = ThreatActor::new("TestActor", ActorMotivation::Espionage, status);
            actor.target_sectors = sectors;
            actor.target_regions = regions.iter().map(|r| (*r).to_string()).collect();
            actor.attributed_country = attributed_country.map(|c| c.to_string());
            actor
        }

        fn us_company_geo() -> Vec<String> {
            company_geo_tokens(Some("US"), Some("North America"))
        }

        #[test]
        fn attributed_country_alone_never_matches() {
            let actor = test_actor(
                ActorStatus::Active,
                vec![IndustrySector::Semiconductor],
                &[],
                Some("US"),
            );
            assert!(
                actor_match_reasons(&actor, &[IndustrySector::Semiconductor], &us_company_geo())
                    .is_none(),
                "an actor's attributed country is origin, not a target region"
            );
        }

        #[test]
        fn sector_and_target_region_overlap_matches() {
            let actor = test_actor(
                ActorStatus::Active,
                vec![IndustrySector::Semiconductor],
                &["US"],
                None,
            );
            let reasons =
                actor_match_reasons(&actor, &[IndustrySector::Semiconductor], &us_company_geo())
                    .expect("sector + concrete target region overlap must match");
            assert!(reasons.iter().any(|r| r == "sector:semiconductor"));
            assert!(reasons.iter().any(|r| r == "geo:US"));
        }

        #[test]
        fn sector_match_without_region_overlap_does_not_match() {
            let actor = test_actor(
                ActorStatus::Active,
                vec![IndustrySector::Semiconductor],
                &["RU"],
                None,
            );
            assert!(actor_match_reasons(
                &actor,
                &[IndustrySector::Semiconductor],
                &us_company_geo()
            )
            .is_none());
        }

        #[test]
        fn region_match_without_sector_overlap_does_not_match() {
            let actor = test_actor(
                ActorStatus::Active,
                vec![IndustrySector::Healthcare],
                &["US"],
                None,
            );
            assert!(actor_match_reasons(
                &actor,
                &[IndustrySector::Semiconductor],
                &us_company_geo()
            )
            .is_none());
        }

        #[test]
        fn global_region_never_matches_on_its_own() {
            let actor = test_actor(
                ActorStatus::Active,
                vec![IndustrySector::Semiconductor],
                &["GLOBAL"],
                None,
            );
            assert!(actor_match_reasons(
                &actor,
                &[IndustrySector::Semiconductor],
                &us_company_geo()
            )
            .is_none());
        }

        #[test]
        fn defunct_actor_never_matches() {
            let actor = test_actor(
                ActorStatus::Defunct,
                vec![IndustrySector::Semiconductor],
                &["US"],
                None,
            );
            assert!(actor_match_reasons(
                &actor,
                &[IndustrySector::Semiconductor],
                &us_company_geo()
            )
            .is_none());
        }
    }
}
