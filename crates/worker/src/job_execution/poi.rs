use std::sync::Arc;

#[cfg(feature = "llm")]
use std::collections::{BTreeMap, HashMap, HashSet};

#[cfg(feature = "llm")]
use apex_core::entities::{Company, CompanyType};
#[cfg(feature = "llm")]
use apex_core::env::parse_truthy_flag;
#[cfg(feature = "llm")]
use apex_core::person_names::is_place_name;
#[cfg(feature = "llm")]
use apex_parse::{
    award::{classify_award_relevance, extract_award, is_award_content},
    directory::{extract_directory, is_directory_content},
    normalizer::{dedup_preserving_order, normalize_entity_name, normalize_whitespace},
    patent::{classify_patent_relevance, extract_patent},
    tender::{extract_tender, is_ems_relevant},
    trade_show::{extract_trade_show, is_ems_trade_show},
};
#[cfg(feature = "llm")]
use apex_store::postgres::ObservationRow;

use crate::*;

#[cfg(feature = "llm")]
#[derive(Debug, Default)]
struct OrgDiscoveryStats {
    observations_scanned: u64,
    candidate_names: u64,
    inserted: u64,
    existing_matches: u64,
    skipped_generic: u64,
    skipped_duplicate: u64,
}

#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
struct OrgDiscoveryCandidate {
    name: String,
    event_name: String,
    source_url: String,
    source_kind: &'static str,
    description: String,
    country: Option<String>,
}

#[cfg(feature = "llm")]
#[derive(Debug, Clone, sqlx::FromRow)]
struct CompanyPoiSeedRow {
    id: Uuid,
    name: String,
    domain: Option<String>,
    region: Option<String>,
    country_code: Option<String>,
    is_competitor: bool,
    poi_count: i64,
}

#[cfg(feature = "llm")]
fn normalize_company_candidate_name(raw: &str) -> Option<String> {
    let normalized = normalize_whitespace(raw)
        .trim_matches(|character: char| {
            matches!(
                character,
                '-' | '|' | ',' | ':' | ';' | '.' | '(' | ')' | '[' | ']'
            )
        })
        .trim()
        .to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(feature = "llm")]
fn is_discoverable_company_name(name: &str) -> bool {
    let normalized = match normalize_company_candidate_name(name) {
        Some(value) => value,
        None => return false,
    };
    if normalized.len() < 3 || normalized.len() > 96 {
        return false;
    }

    let lower = normalized.to_ascii_lowercase();
    let blocked_exact = [
        "exhibitor list",
        "speaker list",
        "conference program",
        "registration",
        "more exhibitors",
        "event partners",
        "download brochure",
    ];
    if blocked_exact.iter().any(|blocked| lower == *blocked) {
        return false;
    }
    let blocked_fragments = [
        " booth ",
        " hall ",
        " stand ",
        "click here",
        "learn more",
        "read more",
        "sponsor",
        "speaker",
        "conference",
        "summit",
        "expo 202",
        "2026 exhibitors",
        "2025 exhibitors",
    ];
    if blocked_fragments
        .iter()
        .any(|blocked| lower.contains(blocked))
    {
        return false;
    }

    // Reject single-word generic nouns that slip through as org candidates
    let generic_single_words = [
        "technology",
        "services",
        "solutions",
        "systems",
        "international",
        "group",
        "corporation",
        "company",
        "industries",
        "electronics",
        "automotive",
        "global",
        "digital",
        "advanced",
        "precision",
    ];
    let token_count = normalized.split_whitespace().count();
    if token_count == 1 && generic_single_words.iter().any(|w| lower == *w) {
        return false;
    }

    // Reject bare place names that slipped through as org candidates
    // (e.g. "Casablanca", "Ho Chi Minh", "Rabat Sale Kenitra"). The gazetteer
    // matches the whole name, so legitimate companies that merely contain a
    // place word ("Ho Chi Minh Electronics") still pass — only names that ARE
    // a place are blocked.
    if is_place_name(&normalized) {
        return false;
    }

    let alpha_count = normalized
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count();
    if alpha_count < 2 {
        return false;
    }
    if token_count > 8 {
        return false;
    }

    true
}

#[cfg(feature = "llm")]
fn company_seed_website(domain: Option<&str>) -> Option<String> {
    let domain = domain?.trim();
    if domain.is_empty() {
        None
    } else if domain.starts_with("http://") || domain.starts_with("https://") {
        Some(domain.to_string())
    } else {
        Some(format!("https://{domain}"))
    }
}

#[cfg(feature = "llm")]
async fn load_company_poi_seeds(
    store: &Arc<PgStore>,
    limit: i64,
    target_pois_per_company: i64,
) -> Result<Vec<CompanyPoiSeedRow>> {
    let limit = limit.clamp(10, 500);
    let target_pois_per_company = target_pois_per_company.clamp(1, 4);

    Ok(sqlx::query_as::<_, CompanyPoiSeedRow>(
        r#"SELECT
               c.id,
               c.name,
               c.domain,
               c.region,
               c.country_code,
               COALESCE((c.metadata->>'is_competitor')::boolean, false) AS is_competitor,
               COUNT(p.id)::bigint AS poi_count
           FROM companies c
           LEFT JOIN persons p ON p.primary_org_id = c.id
           WHERE COALESCE(c.name, '') <> ''
           GROUP BY
               c.id,
               c.name,
               c.domain,
               c.region,
               c.country_code,
               COALESCE((c.metadata->>'is_competitor')::boolean, false)
           HAVING COUNT(p.id) < $2
           ORDER BY
               COALESCE((c.metadata->>'is_competitor')::boolean, false) DESC,
               COUNT(p.id) ASC,
               c.name ASC
           LIMIT $1"#,
    )
    .bind(limit)
    .bind(target_pois_per_company)
    .fetch_all(&store.pool)
    .await?)
}

#[cfg(feature = "llm")]
fn org_discovery_candidates_from_observation(
    observation: &ObservationRow,
) -> Vec<OrgDiscoveryCandidate> {
    let title = observation
        .value
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let description = observation
        .value
        .get("description")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let body_excerpt = observation
        .value
        .get("body_excerpt")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let source_url = observation
        .value
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let source_id = observation
        .value
        .get("source_id")
        .and_then(|value| value.as_str())
        .unwrap_or("webchange");

    let combined_text = format!("{title}\n{description}\n{body_excerpt}");
    let extract = extract_trade_show(&combined_text, title, &source_url);
    let trade_show_like = is_ems_trade_show(&extract.event_name)
        || extract.exhibitors.len() >= 2
        || extract
            .speakers
            .iter()
            .filter(|speaker| speaker.company.is_some())
            .count()
            >= 2;

    let mut candidates = Vec::new();
    if trade_show_like {
        for exhibitor in extract.exhibitors {
            if !is_discoverable_company_name(&exhibitor.name) {
                continue;
            }
            candidates.push(OrgDiscoveryCandidate {
                name: exhibitor.name,
                event_name: extract.event_name.clone(),
                source_url: extract.url.clone(),
                source_kind: "trade_show_exhibitor",
                description: exhibitor.description,
                country: exhibitor.country,
            });
        }
        for speaker in extract.speakers {
            let Some(company) = speaker.company else {
                continue;
            };
            if !is_discoverable_company_name(&company) {
                continue;
            }
            let description = speaker
                .title
                .as_deref()
                .map(|title| format!("speaker organization: {title}"))
                .unwrap_or_else(|| "speaker organization".to_string());
            candidates.push(OrgDiscoveryCandidate {
                name: company,
                event_name: extract.event_name.clone(),
                source_url: extract.url.clone(),
                source_kind: "trade_show_speaker_company",
                description,
                country: None,
            });
        }
    }

    let tender = extract_tender(&combined_text, title, &source_url, source_id);
    if is_ems_relevant(&tender) {
        if let Some(buyer) = tender.buyer {
            if is_discoverable_company_name(&buyer) {
                candidates.push(OrgDiscoveryCandidate {
                    name: buyer,
                    event_name: tender.title.clone(),
                    source_url: tender.url.clone(),
                    source_kind: "tender_buyer",
                    description: tender.description.clone(),
                    country: None,
                });
            }
        }
    }

    // Extract patent applicant (organization)
    let patent = extract_patent(&combined_text, title, &source_url, "unknown");
    if classify_patent_relevance(&patent) > 0.3
        && !patent.applicant.is_empty() && is_discoverable_company_name(&patent.applicant)
    {
        candidates.push(OrgDiscoveryCandidate {
            name: patent.applicant.clone(),
            event_name: patent.title.clone(),
            source_url: patent.url.clone(),
            source_kind: "patent_applicant",
            description: format!(
                "Applicant of patent {}: {}",
                patent.patent_number, patent.title
            ),
            country: None,
        });
    }

    // Extract award recipients (organizations only — people are not companies)
    if is_award_content(&combined_text, title) {
        let award = extract_award(&combined_text, title, &source_url, "unknown");
        if classify_award_relevance(&award) > 0.3
            && award.recipient_type == apex_parse::award::RecipientType::Organization
            && is_discoverable_company_name(&award.recipient)
        {
            candidates.push(OrgDiscoveryCandidate {
                name: award.recipient,
                event_name: award.award_name.clone(),
                source_url: award.url.clone(),
                source_kind: "award_recipient",
                description: format!(
                    "Recipient of {} award: {}",
                    award.award_name, award.description
                ),
                country: None,
            });
        }
    }

    // Extract directory members (business/industry directories)
    if is_directory_content(title, &combined_text) {
        let directory = extract_directory(&combined_text, title, &source_url);
        for member in directory.members {
            if is_discoverable_company_name(&member.name) {
                candidates.push(OrgDiscoveryCandidate {
                    name: member.name,
                    event_name: directory.directory_name.clone(),
                    source_url: directory.url.clone(),
                    source_kind: "directory_member",
                    description: if !member.description.is_empty() {
                        member.description
                    } else {
                        format!("Member of {} directory", directory.directory_type)
                    },
                    country: member.location,
                });
            }
        }
    }

    let mut seen = HashSet::new();
    dedup_preserving_order(
        candidates
            .iter()
            .map(|candidate| normalize_entity_name(&candidate.name))
            .collect(),
    )
    .into_iter()
    .filter_map(|normalized_name| {
        candidates
            .iter()
            .find(|candidate| normalize_entity_name(&candidate.name) == normalized_name)
            .cloned()
    })
    .filter(|candidate| seen.insert(normalize_entity_name(&candidate.name)))
    .collect()
}

#[cfg(feature = "llm")]
fn infer_decision_style_heuristic(role: &str, family: &str) -> String {
    let role_lower = role.to_lowercase();
    let family_lower = family;

    if family_lower.contains("executive") || family_lower.contains("government") || family_lower.contains("military") {
        "Decisive".to_string()
    } else if family_lower.contains("procurement") || family_lower.contains("supply") || family_lower.contains("chain") {
        "Analytical".to_string()
    } else if family_lower.contains("engineering") || family_lower.contains("technical") {
        "Analytical".to_string()
    } else if family_lower.contains("quality") || family_lower.contains("compliance") || family_lower.contains("audit") {
        "Analytical".to_string()
    } else if family_lower.contains("finance") || family_lower.contains("legal") {
        "Analytical".to_string()
    } else if family_lower.contains("operations") || family_lower.contains("logistics") {
        "Decisive".to_string()
    } else if role_lower.contains("ceo") || role_lower.contains("president") || role_lower.contains("director") {
        "Decisive".to_string()
    } else if role_lower.contains("vp") || role_lower.contains("head") || role_lower.contains("chief") {
        "Decisive".to_string()
    } else if role_lower.contains("manager") || role_lower.contains("lead") {
        "Collaborative".to_string()
    } else {
        "Unknown".to_string()
    }
}

#[cfg(feature = "llm")]
fn infer_communication_style_heuristic(role: &str, family: &str) -> String {
    let family_lower = family;

    if family_lower.contains("executive") || family_lower.contains("government") || family_lower.contains("military") {
        "Direct".to_string()
    } else if family_lower.contains("procurement") || family_lower.contains("supply") {
        "Data-driven".to_string()
    } else if family_lower.contains("engineering") {
        "Consultative".to_string()
    } else if family_lower.contains("quality") || family_lower.contains("compliance") {
        "Data-driven".to_string()
    } else if family_lower.contains("operations") {
        "Direct".to_string()
    } else if family_lower.contains("finance") || family_lower.contains("legal") {
        "Data-driven".to_string()
    } else {
        "Unknown".to_string()
    }
}

#[cfg(feature = "llm")]
fn infer_risk_tolerance_heuristic(role: &str, family: &str) -> String {
    let role_lower = role.to_lowercase();
    let family_lower = family;

    if family_lower.contains("government") || family_lower.contains("military") || family_lower.contains("defense") {
        "Risk-averse".to_string()
    } else if family_lower.contains("compliance") || family_lower.contains("legal") || family_lower.contains("audit") {
        "Risk-averse".to_string()
    } else if family_lower.contains("executive") {
        if role_lower.contains("ceo") || role_lower.contains("founder") || role_lower.contains("president") {
            "Risk-tolerant".to_string()
        } else {
            "Moderate".to_string()
        }
    } else if family_lower.contains("operations") || family_lower.contains("logistics") {
        "Moderate".to_string()
    } else {
        "Moderate".to_string()
    }
}

#[cfg(feature = "llm")]
fn infer_change_appetite_heuristic(role: &str, family: &str) -> String {
    let role_lower = role.to_lowercase();
    let family_lower = family;

    if family_lower.contains("government") || family_lower.contains("military") || family_lower.contains("defense") {
        "Conservative".to_string()
    } else if family_lower.contains("compliance") || family_lower.contains("legal") || family_lower.contains("audit") {
        "Conservative".to_string()
    } else if role_lower.contains("founder") || role_lower.contains("innovation") || role_lower.contains("transformation") {
        "Aggressive".to_string()
    } else if family_lower.contains("executive") && (role_lower.contains("ceo") || role_lower.contains("strategy")) {
        "Aggressive".to_string()
    } else {
        "Moderate".to_string()
    }
}

#[cfg(feature = "llm")]
async fn run_org_first_company_discovery(
    store: &Arc<PgStore>,
    now: DateTime<Utc>,
) -> Result<OrgDiscoveryStats> {
    let lookback_days = std::env::var("POI_ORG_DISCOVERY_LOOKBACK_DAYS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(45)
        .clamp(1, 120);
    let observation_limit = std::env::var("POI_ORG_DISCOVERY_OBSERVATION_LIMIT")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1000)
        .clamp(50, 5000);
    let insert_limit = std::env::var("POI_ORG_DISCOVERY_INSERT_LIMIT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(120)
        .clamp(1, 500);

    let observations = store
        .get_observations_by_type(
            "WebChange",
            now - chrono::Duration::days(lookback_days),
            observation_limit,
        )
        .await?;

    let mut stats = OrgDiscoveryStats {
        observations_scanned: observations.len() as u64,
        ..OrgDiscoveryStats::default()
    };
    let mut seen_names: HashSet<String> = HashSet::new();

    for observation in observations {
        let candidates = org_discovery_candidates_from_observation(&observation);
        if candidates.is_empty() {
            continue;
        }

        for candidate in candidates {
            stats.candidate_names += 1;
            let Some(name) = normalize_company_candidate_name(&candidate.name) else {
                stats.skipped_generic += 1;
                continue;
            };
            if !is_discoverable_company_name(&name) {
                stats.skipped_generic += 1;
                continue;
            }

            let dedup_key = normalize_entity_name(&name);
            if !seen_names.insert(dedup_key) {
                stats.skipped_duplicate += 1;
                continue;
            }

            if store.get_company_by_name_ci(&name).await?.is_some() {
                stats.existing_matches += 1;
                continue;
            }

            let mut company = Company::new(
                name.clone(),
                CompanyType::Other("org_discovered".to_string()),
            );
            company.region = candidate.country.clone();
            company.metadata = serde_json::json!({
                "discovered_via": "org_first_observation",
                "source_kind": candidate.source_kind,
                "source_url": candidate.source_url,
                "event_name": candidate.event_name,
                "description": crate::truncate_text(&candidate.description, 200),
                "is_competitor": false,
                "discovery_track": "organization_first",
                "confidence": observation.confidence.unwrap_or(0.55),
            });
            company.created_at = now;
            company.updated_at = now;

            store.insert_company(&company).await?;
            stats.inserted += 1;
            tracing::info!(
                company = %company.name,
                source_kind = candidate.source_kind,
                event = %candidate.event_name,
                "poi_discovery: inserted organization-first company candidate"
            );

            if stats.inserted as usize >= insert_limit {
                return Ok(stats);
            }
        }
    }

    Ok(stats)
}

#[cfg(all(test, feature = "llm"))]
mod tests {
    use super::*;
    use apex_parse::normalizer::normalize_entity_name;
    use apex_store::postgres::ObservationRow;

    fn make_observation(value: serde_json::Value) -> ObservationRow {
        ObservationRow {
            id: Uuid::new_v4(),
            observation_type: "WebChange".to_string(),
            entity_id: None,
            entity_type: Some("company".to_string()),
            ts_utc: Utc::now(),
            value,
            provenance: serde_json::json!({}),
            confidence: Some(0.7),
            created_at: Some(Utc::now()),
        }
    }

    #[test]
    fn discoverable_company_name_rejects_trade_show_boilerplate() {
        assert!(!is_discoverable_company_name("Speaker List"));
        assert!(!is_discoverable_company_name("Download Brochure"));
        assert!(!is_discoverable_company_name("Expo 2026 Exhibitors"));
        assert!(is_discoverable_company_name("Sagemcom"));
    }

    #[test]
    fn discoverable_company_name_rejects_bare_place_names() {
        // Bare place names that slipped through as org candidates must be
        // rejected, but legitimate companies that merely contain a place word
        // still pass.
        assert!(!is_discoverable_company_name("Casablanca"));
        assert!(!is_discoverable_company_name("Ho Chi Minh"));
        assert!(!is_discoverable_company_name("Istanbul"));
        assert!(!is_discoverable_company_name("Rabat Sale Kenitra"));
        assert!(is_discoverable_company_name("Ho Chi Minh Electronics"));
        assert!(is_discoverable_company_name("Sagemcom"));
    }

    #[test]
    fn org_discovery_extracts_and_deduplicates_trade_show_candidates() {
        let observation = make_observation(serde_json::json!({
            "title": "IPC APEX Expo 2026 exhibitors",
            "description": "Exhibitor list:\nSagemcom - Booth A101 - Hall 5\nLacroix Electronics - Booth B202 - Hall 3\nSpeaker: Jane Doe, VP Strategy at Sagemcom.",
            "body_excerpt": "Venue: Messe Munchen, Munich, Germany. January 15-17, 2026.",
            "url": "https://example.com/ipc-apex"
        }));

        let candidates = org_discovery_candidates_from_observation(&observation);
        let names = candidates
            .iter()
            .map(|candidate| normalize_entity_name(&candidate.name))
            .collect::<Vec<_>>();

        assert!(names.contains(&normalize_entity_name("Sagemcom")));
        assert!(names.contains(&normalize_entity_name("Lacroix Electronics")));
        assert_eq!(
            names
                .iter()
                .filter(|name| *name == &normalize_entity_name("Sagemcom"))
                .count(),
            1
        );
    }
}

#[cfg(feature = "llm")]
#[derive(Debug, Default)]
struct DiscoveryBatchStats {
    raw_candidates: usize,
    prepared_candidates: usize,
    validated_candidates: usize,
    raw_signals: BuyerSignalCounts,
    prepared_signals: BuyerSignalCounts,
    validated_signals: BuyerSignalCounts,
    inserted_signals: BuyerSignalCounts,
    inserted_role_families: BTreeMap<String, usize>,
    inserted: u64,
    artifacts_ingested: u64,
    skipped_dup: u64,
    llm_failures: u64,
    errors: Vec<String>,
}

#[cfg(feature = "llm")]
#[derive(Debug, Default, Clone, Copy)]
struct BuyerSignalCounts {
    total: usize,
    buyer_titles: usize,
    org_leadership: usize,
    opencorporates_board: usize,
    procurement_team_page: usize,
    engineering_team_page: usize,
    other_methods: usize,
}

#[cfg(feature = "llm")]
impl BuyerSignalCounts {
    fn from_discoveries(discoveries: &[DiscoveredPoi]) -> Self {
        let mut counts = Self::default();
        for discovery in discoveries {
            counts.observe(discovery);
        }
        counts
    }

    fn observe(&mut self, discovery: &DiscoveredPoi) {
        self.total += 1;
        if looks_like_buyer_candidate_role(discovery.inferred_role.as_deref()) {
            self.buyer_titles += 1;
        }

        match discovery.discovery_method.as_str() {
            "org_leadership" | "org_leadership_fallback" => self.org_leadership += 1,
            "opencorporates_board" => self.opencorporates_board += 1,
            "procurement_team_page" => self.procurement_team_page += 1,
            "engineering_team_page" => self.engineering_team_page += 1,
            _ => self.other_methods += 1,
        }
    }

    fn accumulate(&mut self, other: Self) {
        self.total += other.total;
        self.buyer_titles += other.buyer_titles;
        self.org_leadership += other.org_leadership;
        self.opencorporates_board += other.opencorporates_board;
        self.procurement_team_page += other.procurement_team_page;
        self.engineering_team_page += other.engineering_team_page;
        self.other_methods += other.other_methods;
    }
}

#[cfg(feature = "llm")]
fn merge_role_family_counts(
    target: &mut BTreeMap<String, usize>,
    source: &BTreeMap<String, usize>,
) {
    for (role_family, count) in source {
        *target.entry(role_family.clone()).or_default() += count;
    }
}

#[cfg(feature = "llm")]
fn log_buyer_candidate_batch_report(batch_label: &str, stats: &DiscoveryBatchStats) {
    tracing::info!(
        batch = %batch_label,
        raw_total = stats.raw_signals.total,
        raw_buyer_titles = stats.raw_signals.buyer_titles,
        raw_org_leadership = stats.raw_signals.org_leadership,
        raw_opencorporates_board = stats.raw_signals.opencorporates_board,
        raw_procurement_team_page = stats.raw_signals.procurement_team_page,
        raw_engineering_team_page = stats.raw_signals.engineering_team_page,
        raw_other_methods = stats.raw_signals.other_methods,
        prepared_total = stats.prepared_signals.total,
        prepared_buyer_titles = stats.prepared_signals.buyer_titles,
        prepared_org_leadership = stats.prepared_signals.org_leadership,
        prepared_opencorporates_board = stats.prepared_signals.opencorporates_board,
        prepared_procurement_team_page = stats.prepared_signals.procurement_team_page,
        prepared_engineering_team_page = stats.prepared_signals.engineering_team_page,
        prepared_other_methods = stats.prepared_signals.other_methods,
        validated_total = stats.validated_signals.total,
        validated_buyer_titles = stats.validated_signals.buyer_titles,
        validated_org_leadership = stats.validated_signals.org_leadership,
        validated_opencorporates_board = stats.validated_signals.opencorporates_board,
        validated_procurement_team_page = stats.validated_signals.procurement_team_page,
        validated_engineering_team_page = stats.validated_signals.engineering_team_page,
        validated_other_methods = stats.validated_signals.other_methods,
        inserted_total = stats.inserted_signals.total,
        inserted_buyer_titles = stats.inserted_signals.buyer_titles,
        inserted_org_leadership = stats.inserted_signals.org_leadership,
        inserted_opencorporates_board = stats.inserted_signals.opencorporates_board,
        inserted_procurement_team_page = stats.inserted_signals.procurement_team_page,
        inserted_engineering_team_page = stats.inserted_signals.engineering_team_page,
        inserted_other_methods = stats.inserted_signals.other_methods,
        inserted_role_families = ?stats.inserted_role_families,
        "poi_discovery: buyer-candidate batch report"
    );
}

#[cfg(feature = "llm")]
fn normalized_person_name_key(name: &str) -> String {
    name.split_whitespace()
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(feature = "llm")]
fn build_discovery_source_artifact(
    person_id: Uuid,
    discovery: &DiscoveredPoi,
    org_name: Option<&str>,
    now: DateTime<Utc>,
) -> Option<PoiArtifact> {
    let source_url = discovery.source_url.trim();
    if source_url.is_empty() {
        return None;
    }

    let role = discovery
        .inferred_role
        .as_deref()
        .map(normalize_whitespace)
        .filter(|value| !value.is_empty());
    let organization = discovery
        .inferred_org
        .as_deref()
        .or(org_name)
        .map(normalize_whitespace)
        .filter(|value| !value.is_empty());

    let title = match (role.as_deref(), organization.as_deref()) {
        (Some(role), Some(organization)) => {
            format!("{} listed as {} at {}", discovery.name, role, organization)
        }
        (Some(role), None) => format!("{} listed as {}", discovery.name, role),
        (None, Some(organization)) => {
            format!("{} listed on {} profile page", discovery.name, organization)
        }
        (None, None) => format!("{} discovered from source page", discovery.name),
    };

    let mut summary = Vec::new();
    summary.push(format!(
        "Discovery source recorded {} via {}.",
        discovery.name,
        discovery.discovery_method.replace('_', " ")
    ));
    if let Some(role) = role.as_deref() {
        summary.push(format!("Role: {}.", role));
    }
    if let Some(organization) = organization.as_deref() {
        summary.push(format!("Organization: {}.", organization));
    }
    if let Some(email) = discovery.contact_email.as_deref() {
        summary.push(format!("Public email found: {}.", email));
    }
    if discovery.contact_linkedin.is_some() {
        summary.push("LinkedIn profile URL captured from source.".to_string());
    }

    let mut artifact = PoiArtifact::new(
        person_id,
        ArtifactType::Other("discovery_source".to_string()),
        source_url.to_string(),
        now,
    );
    artifact.title = Some(title);
    artifact.content_summary = Some(summary.join(" "));
    artifact.source_domain = extract_domain(source_url);
    artifact.topics = vec![
        "poi_discovery".to_string(),
        discovery.discovery_method.clone(),
    ];
    artifact.sentiment_score = Some(discovery.confidence as f64);
    artifact.key_phrases = role.into_iter().collect();
    artifact.provenance = serde_json::json!({
        "source": "poi_discovery",
        "discovery_method": discovery.discovery_method,
        "seed_person_id": discovery.seed_person_id,
    });
    artifact.metadata = serde_json::json!({
        "confidence": discovery.confidence,
        "contact_email": discovery.contact_email,
        "linkedin_url": discovery.contact_linkedin,
        "inferred_org": discovery.inferred_org,
    });
    Some(artifact)
}

#[cfg(feature = "llm")]
async fn process_discovery_batch(
    store: &Arc<PgStore>,
    raw_discoveries: Vec<DiscoveredPoi>,
    known_name_keys: &mut HashSet<String>,
    seed_lookup: &HashMap<String, &apex_store::postgres::ExpansionSeedRow>,
    company_seed_lookup: &HashMap<String, CompanyPoiSeedRow>,
    llm: &OpenAiCompatibleClient,
    person_scraper: Option<&PersonOsintScraper>,
    tor_client: Option<&TorClient>,
    max_onion_people: usize,
    onion_enriched_people: &mut usize,
    now: DateTime<Utc>,
    max_llm_candidates: usize,
    batch_label: &str,
) -> DiscoveryBatchStats {
    let mut stats = DiscoveryBatchStats {
        raw_candidates: raw_discoveries.len(),
        raw_signals: BuyerSignalCounts::from_discoveries(&raw_discoveries),
        ..DiscoveryBatchStats::default()
    };
    let min_confidence = 0.25;

    let mut discoveries: Vec<_> = raw_discoveries
        .into_iter()
        .filter(|discovery| {
            // Deterministic junk rejection applies to ALL discovery methods, including
            // structured sources (org_leadership, opencorporates_board). Structured
            // sources are higher *priority* (sorted first below) but they are not
            // immune to place names, role strings, or project names leaking through
            // — e.g. "Chief Procurement Officer" (a title, not a person) or "Thai
            // Nguyen" (a Vietnamese province) were traced to structured seed data.
            if !looks_like_person_name(&discovery.name) {
                tracing::debug!(
                    batch = %batch_label,
                    name = %discovery.name,
                    method = %discovery.discovery_method,
                    "poi_discovery: skipping non-person-like candidate"
                );
                return false;
            }

            if discovery.discovery_method == "gdelt_co_mention" && discovery.confidence < 0.55 {
                tracing::debug!(
                    batch = %batch_label,
                    name = %discovery.name,
                    confidence = discovery.confidence,
                    "poi_discovery: skipping low-confidence gdelt co-mention"
                );
                return false;
            }

            if discovery.confidence < min_confidence {
                tracing::debug!(
                    batch = %batch_label,
                    name = %discovery.name,
                    confidence = discovery.confidence,
                    method = %discovery.discovery_method,
                    "poi_discovery: skipping low-confidence discovery"
                );
                false
            } else {
                true
            }
        })
        .collect();

    discoveries.sort_by(|left, right| {
        discovery_method_priority(&right.discovery_method)
            .cmp(&discovery_method_priority(&left.discovery_method))
            .then_with(|| {
                right
                    .confidence
                    .partial_cmp(&left.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    if discoveries.len() > max_llm_candidates {
        discoveries.truncate(max_llm_candidates);
    }
    stats.prepared_candidates = discoveries.len();
    stats.prepared_signals = BuyerSignalCounts::from_discoveries(&discoveries);

    tracing::info!(
        batch = %batch_label,
        discovered_total = stats.raw_candidates,
        above_confidence = stats.prepared_candidates,
        min_confidence,
        llm_cap = max_llm_candidates,
        "poi_discovery: candidates prepared for LLM validation"
    );

    if discoveries.is_empty() {
        log_buyer_candidate_batch_report(batch_label, &stats);
        return stats;
    }

    tracing::info!(
        batch = %batch_label,
        count = discoveries.len(),
        "poi_discovery: running LLM validation on candidates"
    );

    let mut validated_discoveries = Vec::new();
    for discovery in discoveries {
        match validate_person_via_llm(llm, &discovery).await {
            Ok(Some(validated)) => {
                tracing::debug!(
                    batch = %batch_label,
                    name = %validated.name,
                    role = ?validated.inferred_role,
                    org = ?validated.inferred_org,
                    "poi_discovery: LLM validated as real person"
                );
                validated_discoveries.push(validated);
            }
            Ok(None) => {
                tracing::info!(
                    batch = %batch_label,
                    name = %discovery.name,
                    method = %discovery.discovery_method,
                    "poi_discovery: LLM rejected as not a real person"
                );
            }
            Err(error) => {
                // LLM unreachable (network/timeout/down). Retry with backoff a few
                // times within this run; if still down, skip the candidate — it will
                // be re-extracted and re-validated on the next scheduled discovery
                // run (daily). We do NOT guess-accept: person validation must be
                // LLM-confirmed to avoid polluting the persons table with junk.
                tracing::warn!(
                    batch = %batch_label,
                    name = %discovery.name,
                    error = %error,
                    "poi_discovery: LLM validation failed (LLM unreachable) — retrying with backoff"
                );
                let mut accepted = false;
                for attempt in 1..=3 {
                    tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt))).await;
                    match validate_person_via_llm(llm, &discovery).await {
                        Ok(Some(validated)) => {
                            tracing::info!(
                                batch = %batch_label,
                                name = %validated.name,
                                attempt,
                                "poi_discovery: LLM recovered, candidate validated"
                            );
                            validated_discoveries.push(validated);
                            accepted = true;
                            break;
                        }
                        Ok(None) => {
                            // LLM came back and cleanly rejected — honor it.
                            accepted = true;
                            break;
                        }
                        Err(e) => {
                            tracing::warn!(
                                batch = %batch_label,
                                name = %discovery.name,
                                attempt,
                                error = %e,
                                "poi_discovery: LLM retry failed"
                            );
                        }
                    }
                }
                if !accepted {
                    tracing::warn!(
                        batch = %batch_label,
                        name = %discovery.name,
                        "poi_discovery: LLM unreachable after 3 retries — deferring candidate to next scheduled run"
                    );
                    stats.llm_failures += 1;
                }
            }
        }
    }

    stats.validated_candidates = validated_discoveries.len();
    stats.validated_signals = BuyerSignalCounts::from_discoveries(&validated_discoveries);
    tracing::info!(
        batch = %batch_label,
        count = stats.validated_candidates,
        names = ?validated_discoveries.iter().map(|discovery| discovery.name.clone()).collect::<Vec<_>>(),
        "poi_discovery: LLM validation passed {} candidates",
        stats.validated_candidates
    );

    for discovery in &validated_discoveries {
        let normalized_key = normalized_person_name_key(&discovery.name);
        if normalized_key.is_empty() || known_name_keys.contains(&normalized_key) {
            tracing::info!(
                batch = %batch_label,
                name = %discovery.name,
                method = %discovery.discovery_method,
                "poi_discovery: skipping duplicate"
            );
            stats.skipped_dup += 1;
            continue;
        }

        let role_family = classify_role_family(discovery.inferred_role.as_deref());
        let parent_seed = seed_lookup.get(&discovery.seed_person_id);
        let company_seed = company_seed_lookup.get(&discovery.seed_person_id);
        let seed_is_competitor = parent_seed
            .map(|seed| seed.is_competitor)
            .or_else(|| company_seed.map(|seed| seed.is_competitor))
            .unwrap_or(false);

        let primary_org_id = if let Some(company_seed) = company_seed {
            Some(company_seed.id)
        } else {
            match resolve_discovered_company_id(store, discovery, parent_seed.copied(), now).await {
                Ok(company_id) => company_id,
                Err(error) => {
                    tracing::warn!(
                        batch = %batch_label,
                        name = %discovery.name,
                        error = %error,
                        "poi_discovery: failed to resolve company, falling back to seed org"
                    );
                    parent_seed.and_then(|seed| seed.primary_org_id)
                }
            }
        };

        let person = Person {
            id: Uuid::new_v4(),
            name: discovery.name.clone(),
            name_ar: None,
            name_fr: None,
            primary_org_id,
            current_role: Some(
                discovery
                    .inferred_role
                    .clone()
                    .unwrap_or_else(|| format!("Discovered ({})", discovery.discovery_method)),
            ),
            role_family,
            region: parent_seed
                .map(|seed| seed.region.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    company_seed
                        .and_then(|seed| seed.region.clone())
                        .filter(|value| !value.is_empty())
                }),
            country_code: parent_seed
                .map(|seed| seed.country_code.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    company_seed
                        .and_then(|seed| seed.country_code.clone())
                        .filter(|value| !value.is_empty())
                }),
            public_bio: None,
            public_email: discovery.contact_email.clone(),
            phone: None,
            personal_email: None,
            photo_hash: None,
            priority_vector: PriorityVector::default(),
            decision_mode: None,
            influence_score: 0.0,
            role_drift_score: 0.0,
            change_risk: 0.0,
            pain_index: 0.0,
            preferred_proof_type: None,
            trigger_topics: vec![],
            decision_style: None,
            risk_tolerance: None,
            change_appetite: None,
            communication_style: None,
            metadata: serde_json::json!({
                "engagement_status": "untracked",
                "discovery_method": discovery.discovery_method,
                "source_url": discovery.source_url,
                "seed_person_id": discovery.seed_person_id,
                "seed_company_id": company_seed.map(|seed| seed.id.to_string()),
                "linkedin_url": discovery.contact_linkedin,
                "inferred_org": discovery.inferred_org,
                "confidence": discovery.confidence,
                "seed_is_competitor": seed_is_competitor,
                "discovery_track": if seed_is_competitor { "competitor" } else { "partner_or_prospect" },
            }),
            created_at: now,
            updated_at: now,
        };

        match store.insert_person(&person).await {
            Ok(()) => {
                known_name_keys.insert(normalized_key);
                stats.inserted += 1;
                stats.inserted_signals.observe(discovery);
                *stats
                    .inserted_role_families
                    .entry(person.role_family.as_str().to_string())
                    .or_default() += 1;
                // Log POI discovery to activity feed (fire-and-forget)
                {
                    let activity_logger =
                        apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
                    activity_logger
                        .log_poi_discovered(
                            &person.name,
                            discovery.inferred_org.as_deref().unwrap_or("Unknown"),
                            discovery.inferred_role.as_deref().unwrap_or("Unknown"),
                            person.role_family.as_str(),
                            Some(&person.id.to_string()),
                        )
                        .await;
                }
                tracing::debug!(
                    batch = %batch_label,
                    name = %discovery.name,
                    method = %discovery.discovery_method,
                    "poi_discovery: inserted new person"
                );

                if let Some(seed) = parent_seed {
                    let role_family_label = person.role_family.as_str().to_string();
                    let _ = store
                        .insert_role_history(
                            person.id,
                            person.primary_org_id,
                            &seed.org_name,
                            person.current_role.as_deref().unwrap_or("Unknown"),
                            Some(&role_family_label),
                            Some(now),
                            None,
                            Some(&discovery.source_url),
                            discovery.confidence as f64,
                        )
                        .await;
                } else if let Some(company_seed) = company_seed {
                    let role_family_label = person.role_family.as_str().to_string();
                    let _ = store
                        .insert_role_history(
                            person.id,
                            person.primary_org_id,
                            &company_seed.name,
                            person.current_role.as_deref().unwrap_or("Unknown"),
                            Some(&role_family_label),
                            Some(now),
                            None,
                            Some(&discovery.source_url),
                            discovery.confidence as f64,
                        )
                        .await;
                }

                let org_hint = parent_seed
                    .map(|seed| seed.org_name.as_str())
                    .or_else(|| company_seed.map(|seed| seed.name.as_str()));
                let mut inserted_for_person = 0u64;
                if let Some(discovery_artifact) =
                    build_discovery_source_artifact(person.id, discovery, org_hint, now)
                {
                    if store.insert_poi_artifact(&discovery_artifact).await.is_ok() {
                        inserted_for_person += 1;
                    }
                }

                if let Some(scraper) = person_scraper {
                    let org_hint = org_hint.unwrap_or("");
                    let mut raw_artifacts = scraper.aggregate(&person.name, org_hint).await;

                    if let Some(tor) = tor_client {
                        let org_domain = parent_seed
                            .and_then(|seed| seed.org_domain.as_deref())
                            .or_else(|| company_seed.and_then(|seed| seed.domain.as_deref()));
                        if tor.is_available() && *onion_enriched_people < max_onion_people {
                            let dark_web = tor
                                .aggregate_dark_web_contacts(&person.name, org_domain)
                                .await;
                            let onion_raw = dark_web_to_raw_artifacts(&dark_web, org_domain);
                            if !onion_raw.is_empty() {
                                *onion_enriched_people += 1;
                            }
                            raw_artifacts.extend(onion_raw);
                        }
                    }

                    for raw in raw_artifacts.into_iter().take(120) {
                        if !is_high_quality_raw_artifact(&raw) {
                            continue;
                        }
                        if let Some(artifact) =
                            raw_to_poi_artifact(person.id, raw, &discovery.source_url, now)
                        {
                            if store.insert_poi_artifact(&artifact).await.is_ok() {
                                inserted_for_person += 1;
                            }
                        }
                    }
                }

                stats.artifacts_ingested += inserted_for_person;
                tracing::info!(
                    batch = %batch_label,
                    person = %person.name,
                    artifacts = inserted_for_person,
                    "poi_discovery: deep profile artifacts ingested"
                );
            }
            Err(error) => {
                stats.errors.push(format!("{}: {}", discovery.name, error));
                tracing::warn!(
                    batch = %batch_label,
                    name = %discovery.name,
                    error = %error,
                    "poi_discovery: failed to insert person"
                );
            }
        }
    }

    log_buyer_candidate_batch_report(batch_label, &stats);

    stats
}

pub(super) async fn run_poi_refresh(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    #[cfg(feature = "llm")]
    {
        let filters = PersonListFilters {
            regions: vec![],
            roles: vec![],
            search: None,
            min_priority: None,
            max_priority: None,
        };
        let persons = match store
            .list_persons(&filters, Some(PersonOrderBy::Priority), true, 200, 0)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                run.fail(&format!("poi_refresh: failed to load persons: {e}"));
                return run;
            }
        };

        if persons.is_empty() {
            run.skip("poi_refresh: no persons in database");
            return run;
        }

        let now_utc = Utc::now().timestamp();
        let mut refreshed: u64 = 0;
        let mut unchanged: u64 = 0;
        let mut enriched_pois: u64 = 0;
        let mut role_history_backfilled: u64 = 0;

        // Construct the LLM client once for the whole refresh so the dormant
        // LLM entity-extraction path (extract_entities_llm / PoiLlmEnricher) is
        // actually switched ON. Previously this refresh passed `None`, leaving
        // the real LLM extraction code dead. Degrades to None if unreachable.
        let refresh_llm_client: Option<InferenceLlmClient> =
            match std::env::var("LLM_BASE_URL") {
                Ok(base_url) if !base_url.trim().is_empty() => {
                    let api_key = std::env::var("LLM_API_KEY").ok();
                    let cfg = apex_llm::inference::InferenceConfig {
                        model: std::env::var("LLM_MODEL")
                            .unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into()),
                        max_tokens: 900,
                        temperature: 0.35,
                        json_mode: true,
                        suppress_thinking: false,
                        timeout: std::time::Duration::from_secs(90),
                        ..Default::default()
                    };
                    let client = InferenceLlmClient::new(base_url, api_key, cfg);
                    if client.health_check().await {
                        tracing::info!("poi_refresh: LLM client reachable — entity extraction enabled");
                        Some(client)
                    } else {
                        tracing::warn!("poi_refresh: LLM client unreachable — falling back to heuristic extraction");
                        None
                    }
                }
                _ => {
                    tracing::debug!("poi_refresh: LLM_BASE_URL not set — heuristic-only extraction");
                    None
                }
            };

        for row in &persons {
            let mut profile = PoiProfile {
                person_id: row.id.to_string(),
                name: row.name.clone(),
                name_variants: vec![],
                org: row.organization.clone(),
                org_id: None,
                current_role: row.role.clone(),
                role_family: RoleFamily::Other(row.role.clone()),
                region: row.region.clone(),
                country_code: String::new(),
                public_bio: String::new(),
                public_email: None,
                artifacts: vec![],
                priority_vector: PoiPriorityVector::zero(),
                psychological: PsychProfile::default_profile(),
                influence: InfluenceProfile {
                    influence_score: row.priority_score,
                    graph_centrality: 0.0,
                    public_recurrence: 0.0,
                    role_seniority_score: 0.0,
                    network_size: 0,
                },
                engagement: None,
                role_history: vec![],
                last_updated_utc: 0,
                profile_completeness: 0.0,
            };

            apex_poi::updater::update_profile_with_llm(&mut profile, vec![], None, refresh_llm_client.as_ref());
            refreshed += 1;
            tracing::debug!(
                person = %row.name,
                completeness = profile.profile_completeness,
                "poi_refresh: profile updated"
            );
            if (profile.influence.influence_score - row.priority_score).abs() > 1e-6 {
                if let Err(e) = store
                    .update_person_influence_score(row.id, profile.influence.influence_score)
                    .await
                {
                    tracing::warn!(
                        person = %row.name,
                        error = %e,
                        "poi_refresh: failed to write-back influence score"
                    );
                }
            }

            // Fix 23-25: Persist computed scores (pain_index, change_risk, role_drift_score)
            let pain_index = profile.psychological.pain_index;
            let change_risk =
                apex_poi::features::compute_change_risk(&profile.role_history, now_utc);
            let role_drift = apex_poi::features::compute_role_drift_score(&profile.role_history);
            if let Err(e) = store
                .update_person_computed_scores(row.id, pain_index, change_risk, role_drift)
                .await
            {
                tracing::warn!(
                    person = %row.name,
                    error = %e,
                    "poi_refresh: failed to write-back computed scores"
                );
            }

            // Role change detection is now handled internally by update_profile
            // which adds RoleHistoryEntry records when org/title changes are detected.
            // The profile's role_history captures all changes — no separate
            // RoleChangeType enum or event-based tracking is needed.
        }

        #[derive(sqlx::FromRow)]
        struct MissingRoleHistoryRow {
            id: Uuid,
            org_id: Option<Uuid>,
            org_name: String,
            current_role: String,
            role_family: String,
        }

        let role_history_backfill_limit = std::env::var("POI_ROLE_HISTORY_BACKFILL_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(100)
            .clamp(0, 500);

        if role_history_backfill_limit > 0 {
            let missing_role_history = sqlx::query_as::<_, MissingRoleHistoryRow>(
                r#"SELECT p.id,
                          p.primary_org_id AS org_id,
                          COALESCE(c.name, 'Independent') AS org_name,
                          COALESCE(p."current_role", p.role_family, 'Unknown') AS current_role,
                          COALESCE(p.role_family, 'Unknown') AS role_family
                   FROM persons p
                   LEFT JOIN companies c ON p.primary_org_id = c.id
                   WHERE NOT EXISTS (
                       SELECT 1 FROM role_history rh WHERE rh.person_id = p.id
                   )
                   ORDER BY COALESCE(p.updated_at, p.created_at) DESC
                   LIMIT $1"#,
            )
            .bind(role_history_backfill_limit)
            .fetch_all(&store.pool)
            .await
            .unwrap_or_default();

            for row in missing_role_history {
                if store
                    .insert_role_history(
                        row.id,
                        row.org_id,
                        &row.org_name,
                        &row.current_role,
                        Some(&row.role_family),
                        Some(Utc::now()),
                        None,
                        None,
                        0.6,
                    )
                    .await
                    .is_ok()
                {
                    role_history_backfilled += 1;
                }
            }
        }

        #[derive(sqlx::FromRow)]
        struct ThinPersonRow {
            id: Uuid,
            name: String,
            org: String,
            current_role: String,
        }
        // ── Heuristic psych profile enrichment (runs before LLM) ────────
        let psych_enrich_limit = std::env::var("POI_PSYCH_ENRICH_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(500)
            .clamp(0, 5000);
        match store
            .get_persons_needing_psych_enrichment(psych_enrich_limit, 0)
            .await
        {
            Ok(needy) => {
                let mut psych_enriched: u64 = 0;
                for person in &needy {
                    let role_text =
                        person
                            .current_role
                            .as_deref()
                            .unwrap_or("");
                    let family = person
                        .role_family
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase();

                    let decision_style = infer_decision_style_heuristic(role_text, &family);
                    let communication_style =
                        infer_communication_style_heuristic(role_text, &family);
                    let risk_tolerance = infer_risk_tolerance_heuristic(role_text, &family);
                    let change_appetite = infer_change_appetite_heuristic(role_text, &family);

                    if store
                        .update_person_psych_profile(
                            person.id,
                            None, // priority_vector computed separately
                            Some(&decision_style),
                            Some(&risk_tolerance),
                            Some(&change_appetite),
                            Some(&communication_style),
                            None, // pain_index computed by features.rs
                            None, // influence_score computed by features.rs
                        )
                        .await
                        .is_ok()
                    {
                        psych_enriched += 1;
                    }
                }
                if psych_enriched > 0 {
                    tracing::info!(
                        psych_enriched = psych_enriched,
                        total_checked = needy.len(),
                        "poi_refresh: heuristic psych profiles enriched"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "poi_refresh: failed to load psych-enrichment candidates");
            }
        }
        // ── End heuristic psych enrichment ──────────────────────────────

        // B332: bounded default. The previous i64::MAX default made the
        // nightly job a sequential LLM pass over every profiled person
        // (worst case days at 90s/call) inside a job whose declared timeout
        // is one hour.
        let enrichment_limit = std::env::var("POI_LLM_ENRICH_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(40)
            .clamp(0, 10_000);
        let batch_size = std::env::var("POI_LLM_ENRICH_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10)
            .clamp(1, 50);
        let thin_persons: Vec<ThinPersonRow> = sqlx::query_as::<_, ThinPersonRow>(
            r#"SELECT p.id,
                      p.name,
                      COALESCE(c.name, '') AS org,
                      COALESCE(p.current_role, p.role_family, 'Unknown') AS current_role
               FROM persons p
               LEFT JOIN companies c ON p.primary_org_id = c.id
               WHERE (p.public_bio IS NULL OR length(COALESCE(p.public_bio, '')) < 250)
                  OR p.decision_style IS NULL
               ORDER BY COALESCE(p.influence_score, 0) DESC
             LIMIT $1"#,
        )
        .bind(enrichment_limit)
        .fetch_all(&store.pool)
        .await
        .unwrap_or_default();

        if !thin_persons.is_empty() {
            let poi_llm_client = {
                let base_url = std::env::var("LLM_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:8080".into());
                let api_key = std::env::var("LLM_API_KEY").ok();
                let model =
                    std::env::var("LLM_MODEL").unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
                let cfg = apex_llm::inference::InferenceConfig {
                    model,
                    max_tokens: 900,
                    temperature: 0.35,
                    json_mode: true,
                    suppress_thinking: false,
                    timeout: std::time::Duration::from_secs(90),
                    ..Default::default()
                };
                InferenceLlmClient::new(base_url, api_key, cfg)
            };

            let total_llm_candidates = thin_persons.len();
            let mut batch_idx: usize = 0;
            for batch in thin_persons.chunks(batch_size) {
                if batch_idx > 0 {
                    // Small delay between batches to avoid overwhelming the LLM
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                for thin in batch {
                // Build a prompt that demands source-attributed output and rejects hallucination.
                let prompt = format!(
                    "You are given a person record derived from OSINT crawling: NAME={name}, ROLE={role}, ORG={org}.\n\
\n\
IMPORTANT RULES (anti-hallucination):\n\
- NEVER invent biographical details. If you do not have real evidence from the person name / role / org combination, state \"Unknown — no OSINT evidence available\" for that field.\n\
- For decision_style, communication_style, risk_tolerance, change_appetite, and preferred_proof_type, infer ONLY from the person's actual role and org context. Do NOT guess personality traits.\n\
- For bio: if you have no concrete public information about this specific person, return \"No verified public bio available.\" Do not fabricate.\n\
- For trigger_topics: list ONLY topics logically connected to this person's role at this org.\n\
\n\
Return ONLY valid JSON (no markdown, no code fences) with these exact keys:\n\
{{\"bio\":\"<factual bio or 'No verified public bio available.'>\",\
\"decision_style\":\"one of: Analytical/Decisive/Collaborative/Consensus-driven/Unknown\",\
\"communication_style\":\"one of: Direct/Consultative/Data-driven/Relationship-focused/Unknown\",\
\"risk_tolerance\":\"one of: Risk-averse/Moderate/Risk-tolerant/Unknown\",\
\"change_appetite\":\"one of: Conservative/Moderate/Aggressive/Unknown\",\
\"preferred_proof_type\":\"one of: ROI metrics/Case studies/Peer references/Technical specs/Unknown\",\
\"trigger_topics\":[\"topic1\",\"topic2\"],\
\"hallucination_risk\":\"low|medium|high\"}}",
                    name = thin.name,
                    role = thin.current_role,
                    org = thin.org,
                );
                use apex_llm::inference::{ChatMessage, InferenceConfig};
                let messages = vec![
                    ChatMessage::system(
                        "You are an OSINT analyst assistant. Your task is to produce structured intelligence profiles \
from crawled public data. YOU MUST NOT FABRICATE OR HALLUCINATE ANY INFORMATION. \
If you do not have concrete OSINT evidence for a field, explicitly state that the information is unavailable. \
For psychological traits (decision_style, communication_style, risk_tolerance, change_appetite), \
only infer from the person's documented professional role and organizational context — never invent traits. \
Set hallucination_risk to \"high\" if the profile contains any fabricated details, \"medium\" if based only on role inference, \
\"low\" if grounded in verifiable public data. Return ONLY valid JSON, no markdown, no extra text.",
                    ),
                    ChatMessage::user(&prompt),
                ];
                let enrich_config = InferenceConfig {
                    max_tokens: 900,
                    temperature: 0.35,
                    json_mode: true,
                    suppress_thinking: false,
                    timeout: std::time::Duration::from_secs(90),
                    ..Default::default()
                };
                match poi_llm_client
                    .complete_with_config(messages, &enrich_config)
                    .await
                {
                    Ok(resp) => {
                        #[derive(serde::Deserialize)]
                        struct PoiEnrichResp {
                            bio: Option<String>,
                            decision_style: Option<String>,
                            communication_style: Option<String>,
                            risk_tolerance: Option<String>,
                            change_appetite: Option<String>,
                            preferred_proof_type: Option<String>,
                            #[serde(default)]
                            trigger_topics: Vec<String>,
                        }
                        match resp.parse_json::<PoiEnrichResp>() {
                            Ok(data) => {
                                let bio = data.bio.as_deref().unwrap_or_default();
                                if !bio.is_empty() {
                                    match store
                                        .update_person_llm_enrichment(
                                            thin.id,
                                            bio,
                                            data.decision_style.as_deref(),
                                            data.communication_style.as_deref(),
                                            data.risk_tolerance.as_deref(),
                                            data.change_appetite.as_deref(),
                                            data.preferred_proof_type.as_deref(),
                                            &data.trigger_topics,
                                        )
                                        .await
                                    {
                                        Ok(()) => {
                                            tracing::info!(
                                                person = %thin.name,
                                                bio_len = bio.len(),
                                                "poi_refresh: LLM profile enrichment applied"
                                            );
                                            enriched_pois += 1;
                                        }
                                        Err(e) => tracing::warn!(
                                            person = %thin.name,
                                            error = %e,
                                            "poi_refresh: failed to write LLM enrichment"
                                        ),
                                    }
                                }
                            }
                            Err(e) => tracing::warn!(
                                person = %thin.name,
                                error = %e,
                                "poi_refresh: LLM enrichment JSON parse failed"
                            ),
                        }
                    }
                    Err(e) => tracing::warn!(
                        person = %thin.name,
                        error = %e,
                        "poi_refresh: LLM enrichment call failed"
                    ),
                }
                }
            }
        }

        run.succeed(
            refreshed,
            &format!(
                "poi_refresh: {} persons processed — {} updated, {} unchanged, {} role-history backfilled, {} LLM-enriched",
                persons.len(),
                refreshed,
                unchanged,
                role_history_backfilled,
                enriched_pois,
            ),
        );
    }
    #[cfg(not(feature = "llm"))]
    {
        // Without the LLM feature we can still do the role-history backfill
        // and basic influence-score refresh using the database directly.
        let filters = PersonListFilters {
            regions: vec![],
            roles: vec![],
            search: None,
            min_priority: None,
            max_priority: None,
        };
        let persons = match store
            .list_persons(&filters, Some(PersonOrderBy::Priority), true, 200, 0)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                run.fail(&format!("poi_refresh(no-llm): failed to load persons: {e}"));
                return run;
            }
        };

        if persons.is_empty() {
            run.skip("poi_refresh(no-llm): no persons in database");
            return run;
        }

        let role_history_backfill_limit = std::env::var("POI_ROLE_HISTORY_BACKFILL_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(100)
            .clamp(0, 500);

        let mut role_history_backfilled: u64 = 0;
        if role_history_backfill_limit > 0 {
            #[derive(sqlx::FromRow)]
            struct MissingRoleHistoryRow {
                id: Uuid,
                org_id: Option<Uuid>,
                org_name: String,
                current_role: String,
                role_family: String,
            }

            let missing = sqlx::query_as::<_, MissingRoleHistoryRow>(
                r#"SELECT p.id,
                          p.primary_org_id AS org_id,
                          COALESCE(c.name, 'Independent') AS org_name,
                          COALESCE(p."current_role", p.role_family, 'Unknown') AS current_role,
                          COALESCE(p.role_family, 'Unknown') AS role_family
                   FROM persons p
                   LEFT JOIN companies c ON p.primary_org_id = c.id
                   WHERE NOT EXISTS (
                       SELECT 1 FROM role_history rh WHERE rh.person_id = p.id
                   )
                   ORDER BY COALESCE(p.updated_at, p.created_at) DESC
                   LIMIT $1"#,
            )
            .bind(role_history_backfill_limit)
            .fetch_all(&store.pool)
            .await
            .unwrap_or_default();

            for row in missing {
                if store
                    .insert_role_history(
                        row.id,
                        row.org_id,
                        &row.org_name,
                        &row.current_role,
                        Some(&row.role_family),
                        Some(Utc::now()),
                        None,
                        None,
                        0.6,
                    )
                    .await
                    .is_ok()
                {
                    role_history_backfilled += 1;
                }
            }
        }

        run.succeed(
            role_history_backfilled,
            &format!(
                "poi_refresh(no-llm): {} persons in DB, {} role-history entries backfilled",
                persons.len(),
                role_history_backfilled,
            ),
        );
    }
    run
}

pub(super) async fn run_poi_discovery(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::PoiDiscovery);
    run.start();
    #[cfg(feature = "llm")]
    {
        let now = Utc::now();
        let org_discovery = match run_org_first_company_discovery(store, now).await {
            Ok(stats) => stats,
            Err(error) => {
                run.fail(&format!(
                    "poi_discovery: organization-first discovery failed: {error}"
                ));
                return run;
            }
        };
        tracing::info!(
            observations_scanned = org_discovery.observations_scanned,
            candidate_names = org_discovery.candidate_names,
            inserted = org_discovery.inserted,
            existing_matches = org_discovery.existing_matches,
            "poi_discovery: organization-first discovery pass completed"
        );

        let seed_limit = std::env::var("POI_DISCOVERY_SEED_LIMIT")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(120)
            .clamp(20, 400);

        let seed_rows = match store.list_expansion_seeds(seed_limit).await {
            Ok(r) => r,
            Err(e) => {
                run.fail(&format!("poi_discovery: failed to load seed persons: {e}"));
                return run;
            }
        };

        let (seeds, seed_lookup, competitor_seed_count, partner_seed_count) = if !seed_rows
            .is_empty()
        {
            let min_competitor_seeds = std::env::var("POI_DISCOVERY_MIN_COMPETITOR_SEEDS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(25)
                .clamp(0, seed_rows.len());
            let min_partner_seeds = std::env::var("POI_DISCOVERY_MIN_PARTNER_SEEDS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(25)
                .clamp(0, seed_rows.len());

            let mut selected_seed_rows: Vec<&apex_store::postgres::ExpansionSeedRow> = Vec::new();
            let mut selected_ids: HashSet<String> = HashSet::new();

            for row in seed_rows
                .iter()
                .filter(|r| r.is_competitor)
                .take(min_competitor_seeds)
            {
                selected_ids.insert(row.id.to_string());
                selected_seed_rows.push(row);
            }
            for row in seed_rows
                .iter()
                .filter(|r| !r.is_competitor)
                .take(min_partner_seeds)
            {
                if selected_ids.insert(row.id.to_string()) {
                    selected_seed_rows.push(row);
                }
            }
            for row in &seed_rows {
                if selected_seed_rows.len() >= seed_limit as usize {
                    break;
                }
                if selected_ids.insert(row.id.to_string()) {
                    selected_seed_rows.push(row);
                }
            }

            let seeds: Vec<SeedPoi> = selected_seed_rows
                .iter()
                .map(|r| SeedPoi {
                    id: r.id.to_string(),
                    name: r.name.clone(),
                    organization: r.org_name.clone(),
                    org_website: r
                        .org_domain
                        .as_deref()
                        .filter(|d| !d.is_empty())
                        .map(|d| format!("https://www.{d}")),
                    region: Some(r.region.clone()).filter(|s| !s.is_empty()),
                    role_family: r.role_family.clone(),
                })
                .collect();

            let seed_lookup: std::collections::HashMap<
                String,
                &apex_store::postgres::ExpansionSeedRow,
            > = selected_seed_rows
                .iter()
                .map(|r| (r.id.to_string(), *r))
                .collect();

            let competitor_count = selected_seed_rows
                .iter()
                .filter(|r| r.is_competitor)
                .count();
            let partner_count = selected_seed_rows
                .iter()
                .filter(|r| !r.is_competitor)
                .count();

            (seeds, seed_lookup, competitor_count, partner_count)
        } else {
            (Vec::new(), std::collections::HashMap::new(), 0, 0)
        };

        let company_seed_limit = std::env::var("POI_DISCOVERY_COMPANY_SEED_LIMIT")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(200)
            .clamp(20, 500);
        let target_pois_per_company = std::env::var("POI_DISCOVERY_TARGET_POIS_PER_COMPANY")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(2)
            .clamp(1, 4);
        let company_seed_rows = match load_company_poi_seeds(
            store,
            company_seed_limit,
            target_pois_per_company,
        )
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                run.fail(&format!(
                    "poi_discovery: failed to load company seeds: {error}"
                ));
                return run;
            }
        };
        let company_seed_lookup: HashMap<String, CompanyPoiSeedRow> = company_seed_rows
            .iter()
            .map(|row| (row.id.to_string(), row.clone()))
            .collect();
        let company_seeds: Vec<SeedPoi> = company_seed_rows
            .iter()
            .map(|row| SeedPoi {
                id: row.id.to_string(),
                name: row.name.clone(),
                organization: row.name.clone(),
                org_website: company_seed_website(row.domain.as_deref()),
                region: row.region.clone().filter(|value| !value.is_empty()),
                role_family: "General".to_string(),
            })
            .collect();

        if seeds.is_empty() && company_seeds.is_empty() {
            if org_discovery.inserted > 0 {
                run.succeed(
                    org_discovery.inserted,
                    &format!(
                        "poi_discovery: organization-first discovery inserted {} companies from {} observations; no person or company seeds available for POI expansion",
                        org_discovery.inserted,
                        org_discovery.observations_scanned,
                    ),
                );
            } else {
                run.skip("poi_discovery: no person or company seeds available for expansion");
            }
            return run;
        }

        tracing::info!(
            seeds_total = seeds.len(),
            competitor_seeds = competitor_seed_count,
            partner_seeds = partner_seed_count,
            with_website = seeds.iter().filter(|s| s.org_website.is_some()).count(),
            "poi_discovery: loaded expansion seeds"
        );
        tracing::info!(
            company_seeds_total = company_seeds.len(),
            zero_poi_companies = company_seed_rows
                .iter()
                .filter(|row| row.poi_count == 0)
                .count(),
            target_pois_per_company,
            "poi_discovery: loaded company coverage seeds"
        );

        let filters = PersonListFilters {
            regions: vec![],
            roles: vec![],
            search: None,
            min_priority: None,
            max_priority: None,
        };
        let all_persons = match store.list_persons(&filters, None, true, 1000, 0).await {
            Ok(p) => p,
            Err(e) => {
                run.fail(&format!("poi_discovery: failed to load all persons: {e}"));
                return run;
            }
        };
        let mut known_name_keys: HashSet<String> = all_persons
            .iter()
            .map(|person| normalized_person_name_key(&person.name))
            .filter(|name| !name.is_empty())
            .collect();

        let proxy_rotator = build_proxy_rotator_from_env().map(|r| Arc::new(Mutex::new(r)));
        if let Some(rotator) = proxy_rotator.as_ref() {
            if let Ok(guard) = rotator.lock() {
                tracing::info!(
                    proxy_health = %guard.health_summary(),
                    "poi_discovery: proxy rotation enabled"
                );
            }
        }

        let engine = match PoiExpansionEngine::new(proxy_rotator) {
            Ok(e) => e,
            Err(e) => {
                run.fail(&format!(
                    "poi_discovery: failed to create expansion engine: {e}"
                ));
                return run;
            }
        };

        let company_candidate_limit = std::env::var("POI_DISCOVERY_COMPANY_CANDIDATE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(300)
            .clamp(20, 800);
        let person_candidate_limit = std::env::var("POI_DISCOVERY_PERSON_CANDIDATE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(80)
            .clamp(10, 200);
        let max_llm_candidates = std::env::var("POI_DISCOVERY_MAX_LLM_CANDIDATES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(80)
            .clamp(10, 200);
        let company_seed_batch_size = std::env::var("POI_DISCOVERY_COMPANY_SEED_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(20)
            .clamp(1, 100);
        let person_seed_batch_size = std::env::var("POI_DISCOVERY_PERSON_SEED_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(10)
            .clamp(1, 50);
        let enable_person_network = std::env::var("POI_DISCOVERY_ENABLE_PERSON_NETWORK")
            .ok()
            .map(|value| parse_truthy_flag(&value))
            .unwrap_or(true);

        let mut llm_config = ModelConfig::llamacpp_lightweight();
        if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
            llm_config.base_url = base_url;
        }
        let llm = OpenAiCompatibleClient::new(llm_config);

        let mut inserted: u64 = 0;
        let mut artifacts_ingested: u64 = 0;
        let mut skipped_dup: u64 = 0;
        let mut raw_candidates_total: usize = 0;
        let mut prepared_candidates_total: usize = 0;
        let mut validated_candidates_total: usize = 0;
        let mut raw_buyer_signals = BuyerSignalCounts::default();
        let mut prepared_buyer_signals = BuyerSignalCounts::default();
        let mut validated_buyer_signals = BuyerSignalCounts::default();
        let mut inserted_buyer_signals = BuyerSignalCounts::default();
        let mut inserted_role_families_total: BTreeMap<String, usize> = BTreeMap::new();
        let mut errors: Vec<String> = vec![];
        let enrichment_proxy = build_paid_proxy_url_from_env();
        let person_scraper = PersonOsintScraper::new(enrichment_proxy.as_deref())
            .map_err(|e| {
                tracing::warn!(error = %e, "poi_discovery: failed to init person scraper, continuing without deep artifact enrichment");
                e
            })
            .ok();

        let onion_enrich_enabled = std::env::var("POI_ONION_ENRICH_ENABLED")
            .ok()
            .map(|v| parse_truthy_flag(&v))
            .unwrap_or(true);
        let max_onion_people = std::env::var("POI_ONION_ENRICH_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(12)
            .clamp(0, 200);
        let tor_client = if onion_enrich_enabled {
            Some(TorClient::new().await)
        } else {
            None
        };
        let mut onion_enriched_people = 0usize;
        let mut company_batches_processed = 0usize;
        let company_batch_total = if company_seeds.is_empty() {
            0
        } else {
            company_seeds.len().div_ceil(company_seed_batch_size)
        };

        for (batch_index, company_seed_batch) in
            company_seeds.chunks(company_seed_batch_size).enumerate()
        {
            company_batches_processed = batch_index + 1;
            tracing::info!(
                batch = company_batches_processed,
                total_batches = company_batch_total,
                seed_count = company_seed_batch.len(),
                "poi_discovery: expanding company coverage batch"
            );

            let batch_discoveries = engine
                .expand_from_company_seeds(
                    company_seed_batch,
                    &known_name_keys,
                    company_candidate_limit,
                )
                .await;
            let batch_label = format!("company_coverage_{}", company_batches_processed);
            let batch_stats = process_discovery_batch(
                store,
                batch_discoveries,
                &mut known_name_keys,
                &seed_lookup,
                &company_seed_lookup,
                &llm,
                person_scraper.as_ref(),
                tor_client.as_ref(),
                max_onion_people,
                &mut onion_enriched_people,
                now,
                max_llm_candidates,
                &batch_label,
            )
            .await;

            raw_candidates_total += batch_stats.raw_candidates;
            prepared_candidates_total += batch_stats.prepared_candidates;
            validated_candidates_total += batch_stats.validated_candidates;
            raw_buyer_signals.accumulate(batch_stats.raw_signals);
            prepared_buyer_signals.accumulate(batch_stats.prepared_signals);
            validated_buyer_signals.accumulate(batch_stats.validated_signals);
            inserted_buyer_signals.accumulate(batch_stats.inserted_signals);
            merge_role_family_counts(
                &mut inserted_role_families_total,
                &batch_stats.inserted_role_families,
            );
            inserted += batch_stats.inserted;
            artifacts_ingested += batch_stats.artifacts_ingested;
            skipped_dup += batch_stats.skipped_dup;
            errors.extend(batch_stats.errors);
        }

        let mut person_batches_processed = 0usize;
        if enable_person_network {
            let person_batch_total = if seeds.is_empty() {
                0
            } else {
                seeds.len().div_ceil(person_seed_batch_size)
            };

            for (batch_index, person_seed_batch) in seeds.chunks(person_seed_batch_size).enumerate()
            {
                person_batches_processed = batch_index + 1;
                tracing::info!(
                    batch = person_batches_processed,
                    total_batches = person_batch_total,
                    seed_count = person_seed_batch.len(),
                    "poi_discovery: expanding person-network batch"
                );

                let batch_discoveries = engine
                    .expand_from_seeds(person_seed_batch, &known_name_keys, person_candidate_limit)
                    .await;
                let batch_label = format!("person_network_{}", person_batches_processed);
                let batch_stats = process_discovery_batch(
                    store,
                    batch_discoveries,
                    &mut known_name_keys,
                    &seed_lookup,
                    &company_seed_lookup,
                    &llm,
                    person_scraper.as_ref(),
                    tor_client.as_ref(),
                    max_onion_people,
                    &mut onion_enriched_people,
                    now,
                    max_llm_candidates,
                    &batch_label,
                )
                .await;

                raw_candidates_total += batch_stats.raw_candidates;
                prepared_candidates_total += batch_stats.prepared_candidates;
                validated_candidates_total += batch_stats.validated_candidates;
                raw_buyer_signals.accumulate(batch_stats.raw_signals);
                prepared_buyer_signals.accumulate(batch_stats.prepared_signals);
                validated_buyer_signals.accumulate(batch_stats.validated_signals);
                inserted_buyer_signals.accumulate(batch_stats.inserted_signals);
                merge_role_family_counts(
                    &mut inserted_role_families_total,
                    &batch_stats.inserted_role_families,
                );
                inserted += batch_stats.inserted;
                artifacts_ingested += batch_stats.artifacts_ingested;
                skipped_dup += batch_stats.skipped_dup;
                errors.extend(batch_stats.errors);
            }
        } else if !seeds.is_empty() {
            tracing::info!(
                company_seeds = company_seeds.len(),
                person_seeds = seeds.len(),
                "poi_discovery: deferring person-network expansion until company coverage backlog is reduced"
            );
        }

        if raw_candidates_total == 0 && inserted == 0 && org_discovery.inserted == 0 {
            run.succeed(0, "poi_discovery: no new persons discovered");
            return run;
        }

        let person_network_summary = if seeds.is_empty() {
            "no person seeds available".to_string()
        } else if enable_person_network {
            format!(
                "person network processed {} seeds across {} batches",
                seeds.len(),
                person_batches_processed
            )
        } else {
            format!(
                "person network deferred across {} seeds while {} company seeds remain below target",
                seeds.len(),
                company_seeds.len()
            )
        };

        tracing::info!(
            raw_total = raw_buyer_signals.total,
            raw_buyer_titles = raw_buyer_signals.buyer_titles,
            raw_org_leadership = raw_buyer_signals.org_leadership,
            raw_opencorporates_board = raw_buyer_signals.opencorporates_board,
            raw_procurement_team_page = raw_buyer_signals.procurement_team_page,
            raw_engineering_team_page = raw_buyer_signals.engineering_team_page,
            raw_other_methods = raw_buyer_signals.other_methods,
            prepared_total = prepared_buyer_signals.total,
            prepared_buyer_titles = prepared_buyer_signals.buyer_titles,
            prepared_org_leadership = prepared_buyer_signals.org_leadership,
            prepared_opencorporates_board = prepared_buyer_signals.opencorporates_board,
            prepared_procurement_team_page = prepared_buyer_signals.procurement_team_page,
            prepared_engineering_team_page = prepared_buyer_signals.engineering_team_page,
            prepared_other_methods = prepared_buyer_signals.other_methods,
            validated_total = validated_buyer_signals.total,
            validated_buyer_titles = validated_buyer_signals.buyer_titles,
            validated_org_leadership = validated_buyer_signals.org_leadership,
            validated_opencorporates_board = validated_buyer_signals.opencorporates_board,
            validated_procurement_team_page = validated_buyer_signals.procurement_team_page,
            validated_engineering_team_page = validated_buyer_signals.engineering_team_page,
            validated_other_methods = validated_buyer_signals.other_methods,
            inserted_total = inserted_buyer_signals.total,
            inserted_buyer_titles = inserted_buyer_signals.buyer_titles,
            inserted_org_leadership = inserted_buyer_signals.org_leadership,
            inserted_opencorporates_board = inserted_buyer_signals.opencorporates_board,
            inserted_procurement_team_page = inserted_buyer_signals.procurement_team_page,
            inserted_engineering_team_page = inserted_buyer_signals.engineering_team_page,
            inserted_other_methods = inserted_buyer_signals.other_methods,
            inserted_role_families = ?inserted_role_families_total,
            "poi_discovery: buyer-candidate run report"
        );

        if errors.is_empty() {
            run.succeed(
                inserted + org_discovery.inserted,
                &format!(
                    "poi_discovery: org-first {} inserted from {} observations; {} company seeds in {} batches; {} raw candidates, {} sent to LLM, {} validated, {} persons inserted, {} artifacts, {} duplicates skipped; {}",
                    org_discovery.inserted,
                    org_discovery.observations_scanned,
                    company_seeds.len(),
                    company_batches_processed,
                    raw_candidates_total,
                    prepared_candidates_total,
                    validated_candidates_total,
                    inserted,
                    artifacts_ingested,
                    skipped_dup,
                    person_network_summary,
                ),
            );
        } else {
            run.succeed(
                inserted + org_discovery.inserted,
                &format!(
                    "poi_discovery: org-first {} inserted; {} company seeds in {} batches; {} raw candidates, {} sent to LLM, {} validated, {} persons inserted, {} artifacts, {} duplicates, {} errors; {}: {}",
                    org_discovery.inserted,
                    company_seeds.len(),
                    company_batches_processed,
                    raw_candidates_total,
                    prepared_candidates_total,
                    validated_candidates_total,
                    inserted,
                    artifacts_ingested,
                    skipped_dup,
                    errors.len(),
                    person_network_summary,
                    errors.join("; ")
                ),
            );
        }
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        run.skip("poi_discovery: requires the `llm` feature");
    }
    run
}

// ─────────────────────────────────────────────────────────────────────────────
// POI Role Reclassification
// ─────────────────────────────────────────────────────────────────────────────
// Re-derives `role_family` for every person from their `current_role` title using
// the canonical `apex_poi::role_classifier::classify_role()` function. This
// corrects the pervasive stale-seeded 'C-Suite' default (every executive in the
// seed scripts was tagged 'C-Suite' regardless of their actual function), so
// procurement managers, sourcing leads, quality engineers, and operations
// directors are accurately categorized — which is what makes the buying-center
// contact recommendations in insights point to the RIGHT person instead of
// always suggesting "contact the CEO".
// ─────────────────────────────────────────────────────────────────────────────

/// Reclassify all persons' role_family from their current_role title.
///
/// Runs daily at 05:45 UTC. Pure CPU work (no network, no LLM) — it loads all
/// persons, applies the rule-based classifier, and UPDATEs only rows where the
/// classifier's result differs from the stored value.
pub(super) async fn run_poi_role_reclassify(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    #[cfg(feature = "llm")]
    {
        use sqlx::Row;
        let start = std::time::Instant::now();

        // Load all persons with a non-null current_role.
        #[derive(sqlx::FromRow)]
        struct PersonRoleRow {
            id: uuid::Uuid,
            name: String,
            current_role: Option<String>,
            role_family: Option<String>,
        }

        let persons: Vec<PersonRoleRow> = match sqlx::query_as::<_, PersonRoleRow>(
            r#"SELECT id, name, "current_role", role_family
                 FROM persons
                WHERE "current_role" IS NOT NULL
                  AND TRIM("current_role") != ''
                ORDER BY name"#,
        )
        .fetch_all(&store.pool)
        .await
        {
            Ok(rows) => rows,
            Err(e) => {
                run.fail(&format!("poi_role_reclassify: failed to load persons: {e}"));
                return run;
            }
        };

        if persons.is_empty() {
            run.skip("poi_role_reclassify: no persons with a current_role found");
            return run;
        }

        let mut reclassified: u64 = 0;
        let mut unchanged: u64 = 0;
        let mut errors: u64 = 0;
        let mut family_counts: BTreeMap<String, u64> = BTreeMap::new();

        for person in &persons {
            let title = match &person.current_role {
                Some(t) if !t.trim().is_empty() => t.trim(),
                _ => {
                    unchanged += 1;
                    continue;
                }
            };

            // Run the canonical classifier (procurement-first priority model).
            let new_family = apex_poi::role_classifier::classify_role(title);
            let new_family_str = role_family_to_db_string(&new_family);

            // Only UPDATE when the classifier disagrees with the stored value.
            // This minimizes DB writes and preserves any hand-corrected values.
            let needs_update = match &person.role_family {
                None => true,
                Some(stored) => {
                    let stored_norm = stored.trim().to_lowercase();
                    stored_norm.is_empty() || stored_norm != new_family_str.to_lowercase()
                }
            };

            if !needs_update {
                unchanged += 1;
                *family_counts.entry(new_family_str.to_string()).or_default() += 1;
                continue;
            }

            match sqlx::query(
                r#"UPDATE persons
                      SET role_family = $2,
                          metadata = metadata || $3::jsonb,
                          updated_at = NOW()
                    WHERE id = $1"#,
            )
            .bind(person.id)
            .bind(&new_family_str)
            .bind(serde_json::json!({
                "role_family_source": "canonical_classifier",
                "role_family_reclassified_at": chrono::Utc::now().to_rfc3339(),
            }))
            .execute(&store.pool)
            .await
            {
                Ok(_) => {
                    reclassified += 1;
                    *family_counts.entry(new_family_str.to_string()).or_default() += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        person_id = %person.id,
                        person_name = %person.name,
                        error = %e,
                        "poi_role_reclassify: failed to update role_family"
                    );
                    errors += 1;
                }
            }
        }

        let elapsed = start.elapsed();
        let family_summary: Vec<String> = family_counts
            .iter()
            .map(|(family, count)| format!("{family}: {count}"))
            .collect();

        run.succeed(
            reclassified,
            &format!(
                "poi_role_reclassify: {} persons loaded, {} reclassified, {} unchanged, {} errors in {:.1}s. Distribution: {}",
                persons.len(),
                reclassified,
                unchanged,
                errors,
                elapsed.as_secs_f64(),
                family_summary.join(", "),
            ),
        );
    }

    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        run.skip("poi_role_reclassify: requires the `llm` feature (apex-poi classifier)");
    }

    run
}

/// Map a `RoleFamily` enum to its canonical database string representation.
#[cfg(feature = "llm")]
fn role_family_to_db_string(family: &apex_core::entities::RoleFamily) -> String {
    use apex_core::entities::RoleFamily;
    match family {
        RoleFamily::Executive => "C-Suite".to_string(),
        RoleFamily::Procurement => "Procurement".to_string(),
        RoleFamily::Quality => "Quality".to_string(),
        RoleFamily::SupplierQuality => "SupplierQuality".to_string(),
        RoleFamily::Engineering => "Engineering".to_string(),
        RoleFamily::Operations => "Operations".to_string(),
        RoleFamily::Security => "Security".to_string(),
        RoleFamily::Finance => "Finance".to_string(),
        RoleFamily::Legal => "Legal".to_string(),
        RoleFamily::Government => "Government".to_string(),
        RoleFamily::Logistics => "Logistics".to_string(),
        RoleFamily::PortLogistics => "PortLogistics".to_string(),
        RoleFamily::FreeZoneAuthority => "FreeZoneAuthority".to_string(),
        RoleFamily::CertificationBody => "CertificationBody".to_string(),
        RoleFamily::IndustryAssociation => "IndustryAssociation".to_string(),
        RoleFamily::Distributor => "Distributor".to_string(),
        RoleFamily::Military => "Military".to_string(),
        RoleFamily::Intelligence => "Intelligence".to_string(),
        RoleFamily::Other(label) => label.clone(),
    }
}
