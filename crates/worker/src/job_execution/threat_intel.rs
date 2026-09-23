//! Threat Intelligence Refresh handler.
//!
//! Scheduled every 6 hours: updates supply chain risk scores, threat actor
//! profiles, and competitive intelligence across all tracked entities.
//! Writes structured threat intelligence records to the database.
use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

use crate::*;

#[cfg(feature = "llm")]
use apex_threat_intel::models::{IndustrySector, PaginationParams};
#[cfg(feature = "llm")]
use apex_threat_intel::threat_actor_database::{ActorStatus, ThreatActor};

/// Execute ThreatIntelRefresh job: refresh threat intel across all tracked entities.
pub(super) async fn run_threat_intel_refresh(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let total_start = Instant::now();

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

    if companies.is_empty() {
        run.skip("threat_intel_refresh: no companies found in database");
        return run;
    }

    let mut total_scores_updated: u64 = 0;
    let mut supply_chain_risks: u64 = 0;
    let mut threat_actor_matches: u64 = 0;
    let mut competitive_flags: u64 = 0;

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

    let activity_logger =
        apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());

    for company in &companies {
        // Assess supply chain risk heuristically from observations
        let heuristic_risk = assess_supply_chain_heuristic(store, company).await;
        if heuristic_risk > 0 {
            supply_chain_risks += heuristic_risk as u64;
            total_scores_updated += 1;
            // Log threat detection to activity feed (fire-and-forget)
            activity_logger
                .log_threat_detected(
                    "supply_chain_heuristic",
                    &company.name,
                    if heuristic_risk >= 5 { "high" } else { "medium" },
                    Some(&company.id.to_string()),
                    Some("company"),
                )
                .await;
        }

        // Threat actor profile matching against the curated, verified
        // threat-actor database (sector + geography overlap).
        #[cfg(feature = "llm")]
        {
            let matches =
                assess_threat_actor_matches(store, company, &known_threat_actors).await as u64;
            if matches > 0 {
                threat_actor_matches += matches;
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
    run.succeed(
        total_scores_updated,
        &format!(
            "threat_intel_refresh: {} companies assessed, {} supply chain risks, \
             {} threat actor matches, {} competitive flags in {:.1}s",
            companies.len(),
            supply_chain_risks,
            threat_actor_matches,
            competitive_flags,
            elapsed.as_secs_f64(),
        ),
    );
    run
}

async fn assess_supply_chain_heuristic(
    store: &Arc<PgStore>,
    company: &apex_store::postgres::CompanyRow,
) -> usize {
    let since = chrono::Utc::now() - chrono::Duration::days(30);
    let rows = sqlx::query(
        r#"SELECT COALESCE(value->>'description', value->>'text_content', '') AS text
           FROM observations
           WHERE entity_id = $1 AND entity_type = 'company' AND created_at >= $2
           LIMIT 50"#,
    )
    .bind(company.id)
    .bind(since)
    .fetch_all(&store.pool)
    .await
    .unwrap_or_default();

    let disruption_count = rows
        .iter()
        .filter(|row| {
            use sqlx::Row;
            let text: String = row.try_get("text").unwrap_or_default();
            let lower = text.to_lowercase();
            lower.contains("disruption")
                || lower.contains("shortage")
                || lower.contains("delay")
                || lower.contains("supply chain")
        })
        .count();

    if disruption_count > 0 {
        let now = chrono::Utc::now();
        // B326: stable ID per (company, signal count) — this self-feeding
        // heuristic no longer inserts a fresh row every 6h for the same
        // underlying observations.
        let obs_id = apex_core::entities::Observation::deterministic_id(
            "supply_heuristic",
            &format!("{}|{}", company.id, disruption_count),
        );
        #[allow(clippy::disallowed_methods)]
        let value = serde_json::json!({
            "company_name": company.name,
            "risk_type": "supply_chain_heuristic",
            "disruption_signals": disruption_count,
            "assessment_method": "heuristic",
        });
        #[allow(clippy::disallowed_methods)]
        let provenance = serde_json::json!({
            "source": "worker_threat_intel_refresh_heuristic",
        });

        let _ = sqlx::query(
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
        .await;

        if disruption_count >= 3 {
            let title = format!("Supply Chain Risk: {} — {} disruption signals", company.name, disruption_count);
            let description = format!(
                "{} shows {} supply chain disruption signals in the last 30 days. Review recommended.",
                company.name, disruption_count
            );
            let _ = store
                .insert_warning(
                    "supply_chain",
                    &title,
                    Some(&description),
                    if disruption_count >= 5 { "high" } else { "medium" },
                    company.region.as_deref(),
                    None,
                    None,
                    None,
                    Some(0.65),
                )
                .await;
        }
    }
    disruption_count
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
/// actor's `attributed_country` and `target_regions`.
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
/// attributed country and target regions.
#[cfg(feature = "llm")]
fn actor_geo_tokens(actor: &ThreatActor) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    if let Some(c) = &actor.attributed_country {
        let t = c.trim().to_uppercase();
        if !t.is_empty() && !tokens.contains(&t) {
            tokens.push(t);
        }
    }
    for r in &actor.target_regions {
        let t = r.trim().to_uppercase();
        if !t.is_empty() && !tokens.contains(&t) {
            tokens.push(t);
        }
    }
    tokens
}

/// Determine whether a threat actor is relevant to a company and, if so, return
/// the human-readable match reasons (sector and/or geography overlap). Defunct
/// actors are never matched. A bare "GLOBAL" target region does not, on its own,
/// match every company — a concrete token must overlap.
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

    let actor_geo = actor_geo_tokens(actor);
    for token in &actor_geo {
        if token.as_str() != "GLOBAL" && company_geo.contains(token) {
            reasons.push(format!("geo:{}", token));
        }
    }

    if reasons.is_empty() {
        None
    } else {
        reasons.dedup();
        Some(reasons)
    }
}

/// Assess a company against every known threat actor, recording a
/// `threat_actor_match` observation (and a high-severity warning for active,
/// high-sophistication actors that directly target the company's sector).
/// Returns the number of matches recorded.
#[cfg(feature = "llm")]
async fn assess_threat_actor_matches(
    store: &Arc<PgStore>,
    company: &apex_store::postgres::CompanyRow,
    known_actors: &[ThreatActor],
) -> usize {
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

        #[allow(clippy::disallowed_methods)]
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
        #[allow(clippy::disallowed_methods)]
        let provenance = serde_json::json!({
            "source": "worker_threat_intel_refresh",
        });

        let _ = sqlx::query(
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
        .await;

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
            let _ = store
                .insert_warning(
                    "threat_actor",
                    &title,
                    Some(&description),
                    "high",
                    company.region.as_deref(),
                    None,
                    None,
                    None,
                    Some(0.6),
                )
                .await;
        }

        matches += 1;
    }

    matches
}
