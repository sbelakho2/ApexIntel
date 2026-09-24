//! LLM-backed POI (person of interest) discovery validation and persistence.

use anyhow::Result;
use apex_core::company_names::normalize_company_name;
use apex_core::entities::{Company, CompanyType};
use apex_crawl::poi_expansion::DiscoveredPoi;
use apex_llm::{LlmClient, OpenAiCompatibleClient};
use apex_poi::model::RoleFamily;
use apex_store::postgres::PgStore;
use chrono::Utc;
use uuid::Uuid;
// ────────────────────────────────────────────────────────────────────────────
// LLM-based POI validation
// ────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "llm")]
pub(crate) fn discovery_method_priority(method: &str) -> u8 {
    match method {
        "org_leadership" | "org_leadership_fallback" => 3,
        "opencorporates_officer" => 2,
        "gov_directory" => 2,
        "gdelt_co_mention" => 0,
        _ => 1,
    }
}

#[cfg(feature = "llm")]
pub(crate) fn looks_like_person_name(name: &str) -> bool {
    let cleaned = name.trim();
    if cleaned.is_empty() || cleaned.len() > 80 {
        return false;
    }
    if !apex_core::person_names::looks_like_person_name(cleaned) {
        return false;
    }
    if cleaned.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }

    let blocked_terms = [
        "holdings",
        "limited",
        "ltd",
        "inc",
        "corp",
        "group",
        "reports",
        "transcript",
        "revenue",
        "demand",
        "guidance",
        "quarter",
        "deep dive",
        "boost",
        "street",
        "expectations",
        "call",
        "earnings",
        "meet",
        "cloud",
    ];
    let lower = cleaned.to_lowercase();
    if blocked_terms.iter().any(|t| lower.contains(t)) {
        return false;
    }

    let parts: Vec<&str> = cleaned
        .split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphabetic() && c != '-' && c != '\''))
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() < 2 || parts.len() > 4 {
        return false;
    }

    parts.iter().all(|p| {
        let mut chars = p.chars();
        match chars.next() {
            Some(first) if first.is_uppercase() => {
                chars.all(|c| c.is_alphabetic() || c == '-' || c == '\'')
            }
            _ => false,
        }
    })
}

#[cfg(feature = "llm")]
pub(crate) fn company_name_matches_seed(inferred_org: &str, seed_org: &str) -> bool {
    let inferred = normalize_company_name(inferred_org);
    let seed = normalize_company_name(seed_org);
    !inferred.is_empty() && inferred == seed
}

#[cfg(feature = "llm")]
pub(crate) fn is_public_email_domain(domain: &str) -> bool {
    matches!(
        domain,
        "gmail.com"
            | "googlemail.com"
            | "outlook.com"
            | "hotmail.com"
            | "live.com"
            | "yahoo.com"
            | "icloud.com"
            | "aol.com"
            | "proton.me"
            | "protonmail.com"
    )
}

#[cfg(feature = "llm")]
pub(crate) fn normalize_company_domain(domain: &str) -> Option<String> {
    let normalized = domain
        .trim()
        .trim_start_matches("www.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized.is_empty() || !normalized.contains('.') {
        return None;
    }
    Some(normalized)
}

#[cfg(feature = "llm")]
pub(crate) fn extract_candidate_company_domain(disc: &DiscoveredPoi) -> Option<String> {
    if let Some(email) = disc.contact_email.as_deref() {
        if let Some((_, domain)) = email.rsplit_once('@') {
            let normalized = normalize_company_domain(domain)?;
            if !is_public_email_domain(&normalized) {
                return Some(normalized);
            }
        }
    }

    if matches!(
        disc.discovery_method.as_str(),
        "org_leadership" | "org_leadership_fallback"
    ) {
        if let Ok(url) = reqwest::Url::parse(&disc.source_url) {
            if let Some(host) = url.host_str() {
                return normalize_company_domain(host);
            }
        }
    }

    None
}

#[cfg(feature = "llm")]
pub(crate) async fn resolve_discovered_company_id(
    store: &PgStore,
    disc: &DiscoveredPoi,
    parent_seed: Option<&apex_store::postgres::ExpansionSeedRow>,
    now: chrono::DateTime<Utc>,
) -> Result<Option<Uuid>> {
    let seed_is_competitor = parent_seed.map(|seed| seed.is_competitor).unwrap_or(false);
    let inferred_org = disc
        .inferred_org
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let (Some(seed), Some(org_name)) = (parent_seed, inferred_org) {
        if company_name_matches_seed(org_name, &seed.org_name) {
            return Ok(seed.primary_org_id);
        }
    }

    let inferred_domain = extract_candidate_company_domain(disc);
    if let Some(domain) = inferred_domain.as_deref() {
        if let Some(existing) = store.get_company_by_domain(domain).await? {
            if seed_is_competitor {
                persist_discovered_company_context(store, &existing, disc, parent_seed, now)
                    .await?;
            }
            return Ok(Some(existing.id));
        }
    }

    if let Some(org_name) = inferred_org {
        if let Some(existing) = store.get_company_by_name_ci(org_name).await? {
            if seed_is_competitor {
                persist_discovered_company_context(store, &existing, disc, parent_seed, now)
                    .await?;
            }
            return Ok(Some(existing.id));
        }

        let mut company = Company::new(
            org_name.to_string(),
            CompanyType::Other("poi_discovered".to_string()),
        );
        company.domain = inferred_domain;
        company.metadata = serde_json::json!({
            "discovered_via": "poi_discovery",
            "discovery_method": disc.discovery_method,
            "source_url": disc.source_url,
            "seed_person_id": disc.seed_person_id,
            "confidence": disc.confidence,
            "seed_org_name": parent_seed.map(|seed| seed.org_name.as_str()),
            "seed_org_id": parent_seed.and_then(|seed| seed.primary_org_id).map(|id| id.to_string()),
            "seed_is_competitor": seed_is_competitor,
            "is_competitor": seed_is_competitor,
            "discovery_track": if seed_is_competitor { "competitor" } else { "partner_or_prospect" },
        });
        company.created_at = now;
        company.updated_at = now;
        store.insert_company(&company).await?;
        tracing::info!(
            company = %company.name,
            domain = ?company.domain,
            method = %disc.discovery_method,
            "poi_discovery: inserted new company"
        );
        return Ok(Some(company.id));
    }

    Ok(parent_seed.and_then(|seed| seed.primary_org_id))
}

#[cfg(feature = "llm")]
pub(crate) async fn persist_discovered_company_context(
    store: &PgStore,
    existing: &apex_store::postgres::CompanyRow,
    disc: &DiscoveredPoi,
    parent_seed: Option<&apex_store::postgres::ExpansionSeedRow>,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let seed_is_competitor = parent_seed.map(|seed| seed.is_competitor).unwrap_or(false);
    let existing_is_competitor = existing
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("is_competitor"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let Some(mut metadata) = existing.metadata.clone() else {
        let mut empty = serde_json::Map::new();
        empty.insert(
            "discovered_via".to_string(),
            serde_json::Value::String("poi_discovery".to_string()),
        );
        update_existing_company_context(
            store,
            existing,
            serde_json::Value::Object(empty),
            disc,
            parent_seed,
            seed_is_competitor,
            now,
        )
        .await?;
        return Ok(());
    };

    if !seed_is_competitor && existing_is_competitor {
        return Ok(());
    }

    update_existing_company_context(
        store,
        existing,
        metadata.take(),
        disc,
        parent_seed,
        seed_is_competitor,
        now,
    )
    .await
}

#[cfg(feature = "llm")]
pub(crate) async fn update_existing_company_context(
    store: &PgStore,
    existing: &apex_store::postgres::CompanyRow,
    metadata: serde_json::Value,
    disc: &DiscoveredPoi,
    parent_seed: Option<&apex_store::postgres::ExpansionSeedRow>,
    seed_is_competitor: bool,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let mut metadata_obj = metadata.as_object().cloned().unwrap_or_default();
    metadata_obj.insert(
        "discovered_via".to_string(),
        serde_json::Value::String("poi_discovery".to_string()),
    );
    metadata_obj.insert(
        "discovery_method".to_string(),
        serde_json::Value::String(disc.discovery_method.clone()),
    );
    metadata_obj.insert(
        "source_url".to_string(),
        serde_json::Value::String(disc.source_url.clone()),
    );
    metadata_obj.insert(
        "seed_person_id".to_string(),
        serde_json::Value::String(disc.seed_person_id.to_string()),
    );
    metadata_obj.insert(
        "confidence".to_string(),
        serde_json::Value::from(disc.confidence as f64),
    );
    if let Some(seed) = parent_seed {
        metadata_obj.insert(
            "seed_org_name".to_string(),
            serde_json::Value::String(seed.org_name.clone()),
        );
        if let Some(seed_org_id) = seed.primary_org_id {
            metadata_obj.insert(
                "seed_org_id".to_string(),
                serde_json::Value::String(seed_org_id.to_string()),
            );
        }
    }
    if seed_is_competitor {
        metadata_obj.insert(
            "seed_is_competitor".to_string(),
            serde_json::Value::Bool(true),
        );
        metadata_obj.insert("is_competitor".to_string(), serde_json::Value::Bool(true));
        metadata_obj.insert(
            "discovery_track".to_string(),
            serde_json::Value::String("competitor".to_string()),
        );
    } else {
        metadata_obj
            .entry("seed_is_competitor".to_string())
            .or_insert(serde_json::Value::Bool(false));
        metadata_obj
            .entry("discovery_track".to_string())
            .or_insert_with(|| serde_json::Value::String("partner_or_prospect".to_string()));
    }

    let mut company = Company::new(
        existing.name.clone(),
        CompanyType::from_str(existing.company_type.as_deref().unwrap_or("other")),
    );
    company.id = existing.id;
    company.legal_name = existing.legal_name.clone();
    company.domain = existing.domain.clone();
    company.country_code = existing.country_code.clone();
    company.region = existing.region.clone();
    company.industry_tags = existing.industry_tags.clone().unwrap_or_default();
    company.employee_estimate = existing.employee_estimate;
    company.revenue_estimate_usd = existing.revenue_estimate_usd;
    company.risk_score = existing.risk_score.unwrap_or(0.0);
    company.threat_score = existing.threat_score.unwrap_or(0.0);
    company.overlap_score = existing.overlap_score.unwrap_or(0.0);
    company.strategic_relevance = existing.strategic_relevance.unwrap_or(0.0);
    company.metadata = serde_json::Value::Object(metadata_obj);
    company.created_at = existing.created_at.unwrap_or(now);
    company.updated_at = now;
    store.insert_company(&company).await?;
    tracing::info!(company = %company.name, competitor = seed_is_competitor, "poi_discovery: refreshed company discovery context");
    Ok(())
}

/// Classify an inferred role title into a canonical `RoleFamily`.
#[cfg(feature = "llm")]
pub(crate) fn classify_role_family(role: Option<&str>) -> RoleFamily {
    let r = match role {
        Some(s) if !s.is_empty() => s.to_lowercase(),
        _ => return RoleFamily::Other("Unknown".to_string()),
    };

    if looks_like_buyer_candidate_role(Some(&r)) {
        return RoleFamily::Procurement;
    }

    // C-suite / executive
    if r.contains("ceo")
        || r.contains("chief executive")
        || r.contains("chairman")
        || r.contains("chairwoman")
        || r.contains("president")
    {
        return RoleFamily::Executive;
    }
    if r.contains("cfo")
        || r.contains("chief financial")
        || r.contains("treasurer")
        || r.contains("controller")
        || r.contains("comptroller")
    {
        return RoleFamily::Finance;
    }
    if r.contains("cto") || r.contains("chief technology") || r.contains("chief information") {
        return RoleFamily::Engineering;
    }
    if r.contains("coo") || r.contains("chief operating") || r.contains("chief supply") {
        return RoleFamily::Operations;
    }
    if r.contains("ciso") || r.contains("chief security") {
        return RoleFamily::Security;
    }
    if r.contains("chief")
        || r.contains("director")
        || r.contains("board")
        || r.contains("executive vice")
        || r.contains("senior vice")
    {
        return RoleFamily::Executive;
    }
    // VP-level roles — classify by functional area
    if r.contains("vp") || r.contains("vice president") {
        if r.contains("finance") || r.contains("financial") {
            return RoleFamily::Finance;
        }
        if r.contains("engineer") || r.contains("technology") || r.contains("r&d") {
            return RoleFamily::Engineering;
        }
        if looks_like_buyer_candidate_role(Some(&r)) {
            return RoleFamily::Procurement;
        }
        if r.contains("operation") || r.contains("supply chain") || r.contains("manufacturing") {
            return RoleFamily::Operations;
        }
        if r.contains("quality") {
            return RoleFamily::Quality;
        }
        if r.contains("security") {
            return RoleFamily::Security;
        }
        if r.contains("legal") || r.contains("counsel") {
            return RoleFamily::Legal;
        }
        if r.contains("logistics") {
            return RoleFamily::Logistics;
        }
        return RoleFamily::Executive;
    }
    // Government
    if r.contains("minister")
        || r.contains("secretary")
        || r.contains("governor")
        || r.contains("commissioner")
        || r.contains("ambassador")
    {
        return RoleFamily::Government;
    }
    // Military
    if r.contains("general")
        || r.contains("admiral")
        || r.contains("colonel")
        || r.contains("military")
        || r.contains("commander")
    {
        return RoleFamily::Military;
    }
    // Functional keywords
    if r.contains("procurement") || r.contains("sourcing") || r.contains("purchasing") {
        return RoleFamily::Procurement;
    }
    if r.contains("quality") {
        return RoleFamily::Quality;
    }
    if r.contains("engineer") || r.contains("architect") {
        return RoleFamily::Engineering;
    }
    if r.contains("operation") || r.contains("manufacturing") || r.contains("plant manager") {
        return RoleFamily::Operations;
    }
    if r.contains("finance") || r.contains("accounting") || r.contains("audit") {
        return RoleFamily::Finance;
    }
    if r.contains("legal") || r.contains("counsel") || r.contains("compliance") {
        return RoleFamily::Legal;
    }
    if r.contains("security") || r.contains("cyber") {
        return RoleFamily::Security;
    }
    if r.contains("logistics") || r.contains("warehouse") || r.contains("shipping") {
        return RoleFamily::Logistics;
    }
    RoleFamily::Other(role.unwrap_or("Unknown").to_string())
}

#[cfg(feature = "llm")]
pub(crate) fn looks_like_buyer_candidate_role(role: Option<&str>) -> bool {
    let Some(role) = role else {
        return false;
    };

    let lower = role.to_lowercase();
    lower.contains("buyer")
        || lower.contains("procurement")
        || lower.contains("purchas")
        || lower.contains("sourcing")
        || lower.contains("supply chain")
        || lower.contains("commodity")
        || lower.contains("category manager")
        || lower.contains("vendor management")
        || lower.contains("supplier diversity")
        || lower.contains("supply planning")
        || lower.contains("inventory")
        || lower.contains("approvisionnement")
        || lower.contains("achat")
        || lower.contains("achats")
}

/// Validates that a discovered POI candidate is a real person name (not a topic,
/// navigation element, or garbage text). Returns `Some(validated)` with potentially
/// enriched role/org if valid, or `None` if not a real person.
#[cfg(feature = "llm")]
pub(crate) async fn validate_person_via_llm(
    llm: &OpenAiCompatibleClient,
    candidate: &DiscoveredPoi,
) -> anyhow::Result<Option<DiscoveredPoi>> {
    const SYSTEM_PROMPT: &str = r#"You are a strict POI validator for strategic intelligence.
Your task has TWO gates:
1) Is this a real person?
2) Is this person a TARGET business/government decision-maker profile?

Target profiles:
- Government: deputy/director-general, department directors, program/policy directors/managers,
  procurement/acquisition officials, licensing/regulatory/compliance leaders in ministries/agencies.
- Company: C-suite, presidents, board members, general managers, heads, vice presidents,
  and procurement/purchasing/sourcing/category/commodity, operations, supply-chain, quality,
  engineering, legal, compliance, security, and finance decision-makers.

Non-target profiles (reject):
- Investor relations contacts, press/media contacts, recruiters, HR, marketing, sales, support,
  assistants, coordinators, and generic contact-page staff.
- Generic text, concepts, product labels, navigation strings.

Respond ONLY valid JSON:
{"is_person":true/false,"target_fit":true/false,"name":"...","role":"...|null","org":"...|null","seniority_band":"mid|senior_mid|unknown"}
If invalid/non-target: {"is_person":false,"target_fit":false}"#;

    let user_prompt = format!(
        "Candidate name: \"{}\"\nSource: {}\nDiscovery method: {}\nInferred role: {}\nInferred org: {}",
        candidate.name,
        candidate.source_url,
        candidate.discovery_method,
        candidate.inferred_role.as_deref().unwrap_or("unknown"),
        candidate.inferred_org.as_deref().unwrap_or("unknown"),
    );

    let response = llm.generate_json(SYSTEM_PROMPT, &user_prompt).await?;

    // Parse LLM response
    #[derive(serde::Deserialize)]
    struct LlmResponse {
        is_person: bool,
        #[serde(default)]
        target_fit: Option<bool>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        role: Option<String>,
        #[serde(default)]
        org: Option<String>,
    }

    let parsed: LlmResponse = match serde_json::from_str(&response) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                candidate = %candidate.name,
                response = %response,
                error = %e,
                "poi_validation: failed to parse LLM response, rejecting candidate"
            );
            return Ok(None);
        }
    };

    if !parsed.is_person || !parsed.target_fit.unwrap_or(false) {
        return Ok(None);
    }

    // Return validated/enriched candidate
    let mut validated = candidate.clone();
    let name_rewritten = if let Some(ref name) = parsed.name {
        !name.is_empty() && name.trim() != candidate.name.trim()
    } else {
        false
    };
    if let Some(name) = parsed.name {
        if !name.is_empty() {
            validated.name = name;
        }
    }
    // Post-LLM backstop: the model can rewrite the name (correcting spelling,
    // expanding initials, etc.), but it can also produce a non-person string
    // (a role title, a place name, a transliterated phrase). Re-run the
    // deterministic junk check on the final name before accepting it. This
    // guards against LLM hallucination/over-correction that reintroduces the
    // exact junk classes the pre-LLM filter is designed to reject.
    if !apex_core::person_names::looks_like_person_name(&validated.name) {
        tracing::info!(
            original_name = %candidate.name,
            final_name = %validated.name,
            name_rewritten,
            "poi_validation: post-LLM backstop rejected non-person-like name"
        );
        return Ok(None);
    }
    let sanitized_org = sanitize_validated_org(candidate, parsed.org);
    if let Some(role) = sanitize_validated_role(candidate, parsed.role, sanitized_org.as_deref()) {
        validated.inferred_role = Some(role);
    }
    if let Some(org) = sanitized_org {
        validated.inferred_org = Some(org);
    }

    Ok(Some(validated))
}

#[cfg(feature = "llm")]
pub(crate) fn sanitize_validated_role(
    candidate: &DiscoveredPoi,
    parsed_role: Option<String>,
    parsed_org: Option<&str>,
) -> Option<String> {
    let role = sanitize_llm_field(parsed_role)?;
    if !looks_like_decision_role_label(&role) {
        return None;
    }

    let normalized_role = normalize_company_name(&role);
    if normalized_role == normalize_company_name("ApexIntel") {
        return None;
    }
    if let Some(existing_org) = candidate.inferred_org.as_deref() {
        if normalized_role == normalize_company_name(existing_org) {
            return None;
        }
    }
    if let Some(org) = parsed_org {
        if normalized_role == normalize_company_name(org) {
            return None;
        }
    }

    Some(role)
}

#[cfg(feature = "llm")]
pub(crate) fn sanitize_validated_org(
    candidate: &DiscoveredPoi,
    parsed_org: Option<String>,
) -> Option<String> {
    let org = sanitize_llm_field(parsed_org)?;
    let normalized_org = normalize_company_name(&org);
    if normalized_org.is_empty() || normalized_org == normalize_company_name("ApexIntel") {
        return None;
    }
    if normalize_company_name(&candidate.name) == normalized_org {
        return None;
    }
    if looks_like_decision_role_label(&org) {
        return None;
    }
    Some(org)
}

#[cfg(feature = "llm")]
pub(crate) fn sanitize_llm_field(value: Option<String>) -> Option<String> {
    let cleaned = value?.trim().trim_matches('"').to_string();
    if cleaned.is_empty() {
        return None;
    }
    let lower = cleaned.to_ascii_lowercase();
    if matches!(lower.as_str(), "unknown" | "null" | "none" | "n/a") {
        return None;
    }
    Some(cleaned)
}

#[cfg(feature = "llm")]
pub(crate) fn looks_like_decision_role_label(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.len() < 4 || lower.len() > 120 {
        return false;
    }

    let junk = [
        "investor relations",
        "media",
        "press",
        "communications",
        "marketing",
        "sales",
        "business development",
        "customer service",
        "support",
        "assistant",
        "coordinator",
        "specialist",
        "analyst",
        "recruiter",
        "human resources",
        "administrator",
        "receptionist",
    ];
    if junk.iter().any(|needle| lower.contains(needle)) {
        return false;
    }

    let role_keywords = [
        "chief",
        "ceo",
        "cfo",
        "cto",
        "coo",
        "president",
        "vice president",
        "vp",
        "head",
        "director",
        "manager",
        "officer",
        "chair",
        "board",
        "founder",
        "owner",
        "general manager",
        "procurement",
        "purchasing",
        "sourcing",
        "supply chain",
        "operations",
        "engineering",
        "quality",
        "compliance",
        "regulatory",
        "legal",
        "security",
        "finance",
        "strategy",
        "commercial",
        "program",
        "policy",
        "acquisition",
        "contracts",
    ];

    role_keywords.iter().any(|needle| lower.contains(needle))
}
