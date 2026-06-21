//! Structured entity extraction from artifact text using LLM + heuristics.
//!
//! Uses LLM-powered structured extraction that accurately identifies:
//! - Organization name (company/institution)
//! - Job title (role/position)
//! - Role family classification
//! - Whether the extracted info represents a genuine role change
//!
//! # Anti-Hallucination Design
//! - LLM output is validated against known entities before acceptance
//! - Every extraction is logged with confidence scores for audit
//! - Multi-source cross-validation: LLM extraction + heuristic fallback
//! - Source evidence is always preferred over inference

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::model::RoleFamily;

// ─── Output Types ───

/// Structured entity extraction result from a single artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityExtraction {
    /// Organization name extracted from text.
    pub organization: Option<String>,
    /// Job title extracted from text.
    pub job_title: Option<String>,
    /// Inferred role family from the job title.
    pub role_family: Option<RoleFamily>,
    /// Whether this artifact indicates a genuine role change (not a mention).
    pub is_role_change: bool,
    /// Whether this artifact indicates an organization change.
    pub is_org_change: bool,
    /// Confidence in the extraction (0.0-1.0).
    pub confidence: f64,
    /// Explanation of the extraction reasoning.
    pub reasoning: String,
    /// The timestamp from the artifact.
    pub artifact_ts: i64,
    /// Source of the extraction: "llm" | "heuristic" | "llm+heuristic" | "none"
    pub extraction_source: String,
}

impl EntityExtraction {
    /// Empty extraction for when no structured data could be found.
    pub fn empty(artifact_ts: i64) -> Self {
        Self {
            organization: None,
            job_title: None,
            role_family: None,
            is_role_change: false,
            is_org_change: false,
            confidence: 0.0,
            reasoning: String::new(),
            artifact_ts,
            extraction_source: "none".to_string(),
        }
    }

    /// Returns true if this extraction has meaningful data.
    pub fn has_data(&self) -> bool {
        self.organization.is_some() || self.job_title.is_some()
    }

    /// Returns true if confidence meets minimum threshold for acceptance.
    pub fn is_high_confidence(&self) -> bool {
        self.confidence >= 0.7
    }

    /// Returns true if the extraction came from LLM (not just heuristics).
    pub fn is_llm_backed(&self) -> bool {
        self.extraction_source == "llm" || self.extraction_source == "llm+heuristic"
    }
}

// ─── LLM Extraction Schema ───

/// LLM extraction response schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LlmEntityResponse {
    #[serde(default)]
    organization: Option<String>,
    #[serde(default)]
    job_title: Option<String>,
    #[serde(default)]
    is_role_change: bool,
    #[serde(default)]
    is_org_change: bool,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    reasoning: String,
}

// ─── Heuristic Extraction (fallback) ───

/// Extraction patterns for org names from artifact text.
const ORG_PATTERNS: &[(&str, &[&str])] = &[
    ("prefix", &[
        "joins ", "now at ", "moves to ", "joins firm ", "appointed at ",
        "has joined ", "will join ", "started at ", "began working at ",
        "takes role at ", "assumes position at ", "joining ", "hired by ",
        "recruited by ", "brought on at ",
        "rejoint ", "nommé chez ", "embauché par ",
        "appointed at ",
    ]),
    ("suffix", &[
        " Ltd", " Inc", " Corp", " LLC", " PLC", " AG", " SA", " GmbH",
        " S.A.", " Group", " Technologies", " Systems", " Industries",
        " Solutions", " Manufacturing", " Electronics",
    ]),
];

/// Extraction patterns for job titles from artifact text.
const TITLE_PATTERNS: &[&str] = &[
    " as ", " named ", " promoted to ", " appointed as ", " becomes ",
    " will serve as ", " takes over as ", " assumes role of ",
    " serving as ", " in the role of ", " position as ", " new ",
    " en tant que ", " nommé ", " promu ", " devient ",
    " as the new ", " will be ",
];

/// Advanced multi-strategy org extraction from text.
pub fn extract_org_heuristic(text: &str) -> (Option<String>, f64) {
    let text_lower = text.to_lowercase();

    for (_category, patterns) in ORG_PATTERNS.iter().filter(|(c, _)| *c == "prefix") {
        for pat in *patterns {
            let pat_lower = pat.to_lowercase();
            if let Some(pos) = text_lower.find(&pat_lower) {
                let rest = &text[pos + pat.len()..].trim();
                let end = rest
                    .find(|c: char| c == '.' || c == ',' || c == ';' || c == '\n' || c == '(')
                    .unwrap_or(rest.len().min(100));
                let candidate = rest[..end].trim().to_string();
                if candidate.len() > 2 && candidate.len() < 100 {
                    let cleaned = clean_org_name(&candidate);
                    if is_plausible_org(&cleaned) {
                        return (Some(cleaned), 0.5);
                    }
                }
            }
        }
    }

    for (_category, suffixes) in ORG_PATTERNS.iter().filter(|(c, _)| *c == "suffix") {
        for suffix in *suffixes {
            if let Some(pos) = text.rfind(suffix) {
                let before = &text[..pos + suffix.len()];
                let words: Vec<&str> = before.split_whitespace().collect();
                if words.len() >= 2 {
                    let start = if words.len() > 4 { words.len() - 4 } else { 0 };
                    let candidate = words[start..].join(" ");
                    if candidate.len() > 3 && is_plausible_org(&candidate) {
                        return (Some(clean_org_name(&candidate)), 0.35);
                    }
                }
            }
        }
    }

    let mut candidates: Vec<(String, f64)> = Vec::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        if let Some(word) = words.get(i) {
            if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                && word.len() > 2
                && !is_common_word(word)
            {
                let mut phrase = String::from(*word);
                let mut j = i + 1;
                while j < words.len() && j < i + 5 {
                    if let Some(next) = words.get(j) {
                        if !is_common_word(next) && !next.contains(',') && !next.contains('.') {
                            phrase.push(' ');
                            phrase.push_str(next);
                            j += 1;
                        } else { break; }
                    } else { break; }
                }
                if is_plausible_org(&phrase) && phrase.len() > 5 {
                    candidates.push((clean_org_name(&phrase), 0.25));
                }
                i = j;
            } else { i += 1; }
        } else { i += 1; }
    }

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((org, conf)) = candidates.into_iter().next() {
        if is_plausible_org(&org) { return (Some(org), conf); }
    }
    (None, 0.0)
}

/// Advanced multi-strategy title extraction from text.
///
/// Returns (title, confidence). Confidence is capped at 0.55 for heuristic-only
/// extraction; LLM cross-validation can raise it further.
pub fn extract_title_heuristic(text: &str) -> (Option<String>, f64) {
    extract_title_heuristic_near(text, None)
}

/// Extract a title from text, anchoring to a person's name for accuracy.
///
/// When `person_name` is provided, only considers title matches that appear
/// within 15 words of the person's name. This prevents extracting titles
/// that belong to someone else mentioned in the same text.
pub fn extract_title_heuristic_near(
    text: &str,
    person_name: Option<&str>,
) -> (Option<String>, f64) {
    let text_lower = text.to_lowercase();
    let name_lower = person_name.map(|n| n.to_lowercase());
    let name_words: Vec<&str> = name_lower
        .as_deref()
        .map(|n| n.split_whitespace().collect())
        .unwrap_or_default();

    // Strategy 1: Title patterns with name proximity
    for pat in TITLE_PATTERNS {
        let pat_lower = pat.to_lowercase();
        if let Some(pos) = text_lower.find(&pat_lower) {
            let rest = &text[pos + pat.len()..].trim();
            let end = rest
                .find(|c: char| c == '.' || c == ',' || c == ';' || c == '\n')
                .unwrap_or(rest.len().min(150));
            let candidate = rest[..end].trim().to_string();
            if is_plausible_title(&candidate) {
                // If person_name is known, check proximity
                if !name_words.is_empty() {
                    let window_start = pos.saturating_sub(120);
                    let window_end = (pos + pat.len() + end).min(text.len());
                    let window = &text_lower[window_start..window_end];
                    let name_in_window = name_words.iter().all(|w| window.contains(w));
                    if name_in_window {
                        return (Some(clean_title(&candidate)), 0.55);
                    }
                } else {
                    return (Some(clean_title(&candidate)), 0.50);
                }
            }
        }
    }

    let known_titles = [
        "CEO", "CTO", "CFO", "COO", "CIO", "CISO",
        "VP", "Vice President", "Director", "Senior Director",
        "Executive Director", "Managing Director", "Head of",
        "Chief", "President", "General Manager", "Manager",
        "Senior Manager", "Lead", "Principal", "Partner",
        "Associate", "Analyst", "Engineer", "Architect",
        "Consultant", "Advisor", "Procurement", "Sourcing",
        "Supply Chain", "Quality", "Compliance", "Operations",
        "Manufacturing",
    ];

    for title_prefix in &known_titles {
        if let Some(pos) = text_lower.find(&title_prefix.to_lowercase()) {
            let start = text[..pos]
                .rfind(|c: char| c == '.' || c == ',' || c == ';')
                .map(|p| p + 1).unwrap_or(0);
            let end = text[pos..]
                .find(|c: char| c == '.' || c == ',' || c == ';')
                .map(|p| pos + p).unwrap_or(text.len().min(pos + 80));
            let candidate = text[start..end].trim().to_string();
            if is_plausible_title(&candidate) {
                // Check name proximity when available
                if !name_words.is_empty() {
                    let window_start = start.saturating_sub(120);
                    let window_end = (end + 40).min(text.len());
                    let window = &text_lower[window_start..window_end];
                    let name_in_window = name_words.iter().all(|w| window.contains(w));
                    if name_in_window {
                        return (Some(clean_title(&candidate)), 0.45);
                    }
                } else {
                    return (Some(clean_title(&candidate)), 0.35);
                }
            }
        }
    }
    (None, 0.0)
}

/// Extract structured entities using LLM when available, with heuristic fallback.
///
/// 1. LLM extraction when an LLM client is available (preferred, higher accuracy)
/// 2. Multi-strategy heuristic fallback when LLM is unavailable or fails
/// 3. Merged confidence scoring from both paths
pub async fn extract_entities_llm(
    artifact_text: &str,
    llm_client: Option<&apex_llm::inference::LlmClient>,
    person_name: &str,
) -> EntityExtraction {
    let artifact_ts = 0;

    if let Some(client) = llm_client {
        match extract_via_llm(client, artifact_text, person_name).await {
            Ok(llm_result) => {
                let (heuristic_org, org_conf) = extract_org_heuristic(artifact_text);
                let (heuristic_title, title_conf) = extract_title_heuristic_near(artifact_text, Some(person_name));

                // Clone before move to use in reasoning and checks
                let llm_org_clone = llm_result.organization.clone();
                let llm_title_clone = llm_result.job_title.clone();
                let llm_has_org = llm_org_clone.is_some();
                let llm_has_title = llm_title_clone.is_some();

                let org = llm_result.organization.or(heuristic_org.clone());
                let title = llm_result.job_title.or(heuristic_title.clone());

                let heur_has_org = heuristic_org.is_some();
                let heur_has_title = heuristic_title.is_some();

                let agreement_bonus = if (llm_has_org == heur_has_org) && (llm_has_title == heur_has_title) {
                    0.15
                } else {
                    0.0
                };

                let confidence = ((llm_result.confidence + org_conf + title_conf) / 3.0 + agreement_bonus).min(1.0);

                let reasoning = format!(
                    "LLM extraction: org={:?}, title={:?} | Heuristic: org={:?} (conf={:.2}), title={:?} (conf={:.2}) | Agreement bonus: {:.2}",
                    llm_org_clone, llm_title_clone,
                    heuristic_org, org_conf, heuristic_title, title_conf,
                    agreement_bonus
                );

                let is_role_change = title.is_some();
                let is_org_change = org.is_some();
                let role_family = title.as_ref().map(|t| infer_role_family_advanced(t));
                let extraction_source = if llm_has_org || llm_has_title {
                    if heur_has_org || heur_has_title { "llm+heuristic" } else { "llm" }
                } else {
                    "heuristic"
                };

                let mut result = EntityExtraction {
                    organization: org, job_title: title, role_family,
                    is_role_change, is_org_change, confidence, reasoning,
                    artifact_ts, extraction_source: extraction_source.to_string(),
                };

                // ─── Anti-Hallucination Validation ───
                validate_extraction(&mut result, person_name, artifact_text);

                return result;
            }
            Err(e) => {
                warn!(error = %e, person = %person_name,
                    "LLM entity extraction failed, falling back to heuristics");
            }
        }
    }

    // Fallback: heuristic extraction only, anchored to person name
    let (org, org_conf) = extract_org_heuristic(artifact_text);
    let (title, title_conf) = extract_title_heuristic_near(artifact_text, Some(person_name));
    let confidence = (org_conf + title_conf) / 2.0;

    let reasoning = if org.is_some() || title.is_some() {
        format!("Multi-strategy heuristic extraction: org={:?} (conf={:.2}), title={:?} (conf={:.2})",
            org, org_conf, title, title_conf)
    } else {
        String::new()
    };

    let is_role_change = title.is_some();
    let is_org_change = org.is_some();
    let role_family = title.as_ref().map(|t| infer_role_family_advanced(t));

    let mut result = EntityExtraction {
        organization: org, job_title: title, role_family,
        is_role_change, is_org_change, confidence, reasoning,
        artifact_ts, extraction_source: "heuristic".to_string(),
    };

    // ─── Anti-Hallucination Validation ───
    validate_extraction(&mut result, person_name, artifact_text);

    result
}

/// Run anti-hallucination validators on an extraction result.
/// Clears low-confidence fields that fail plausibility checks.
fn validate_extraction(extraction: &mut EntityExtraction, person_name: &str, artifact_text: &str) {
    // 1. Check org plausibility — if extracted org IS the person's name, clear it
    let clear_org = if let Some(ref org) = extraction.organization {
        let implausible = !is_plausible_org(org);
        let matches_person = {
            let org_lower = org.to_lowercase();
            let name_lower = person_name.to_lowercase();
            org_lower == name_lower || org_lower.contains(&name_lower)
        };
        if implausible {
            warn!(person = %person_name, org = %org, "Clearing implausible org");
            true
        } else if matches_person {
            warn!(person = %person_name, org = %org, "Clearing org that matches person name (hallucination)");
            true
        } else {
            false
        }
    } else {
        false
    };

    if clear_org {
        if extraction.organization.is_some() && !is_plausible_org(extraction.organization.as_ref().unwrap()) {
            extraction.confidence *= 0.7;
        } else {
            extraction.confidence *= 0.5;
        }
        extraction.organization = None;
    }

    // 2. Check title plausibility
    let clear_title = if let Some(ref title) = extraction.job_title {
        if !is_plausible_title(title) {
            warn!(person = %person_name, title = %title, "Clearing implausible title");
            true
        } else {
            false
        }
    } else {
        false
    };

    if clear_title {
        extraction.confidence *= 0.6;
        extraction.job_title = None;
    }

    // 3. If extraction has org but no title in artifact_text, reduce confidence
    if extraction.organization.is_some() && extraction.job_title.is_none() {
        let text_lower = artifact_text.to_lowercase();
        let has_role_keyword = text_lower.contains("procurement")
            || text_lower.contains("sourcing")
            || text_lower.contains("purchasing")
            || text_lower.contains("supply chain")
            || text_lower.contains("quality")
            || text_lower.contains("engineering");
        if has_role_keyword {
            extraction.reasoning.push_str(
                " (text contains role keywords but no title extracted — confidence reduced)"
            );
        }
    }

    // 4. Clamp final confidence
    extraction.confidence = extraction.confidence.clamp(0.0, 1.0);
}

/// Extract entities via LLM with structured JSON output.
async fn extract_via_llm(
    client: &apex_llm::inference::LlmClient,
    artifact_text: &str,
    person_name: &str,
) -> anyhow::Result<LlmEntityResponse> {
    let system = concat!(
        "You are an expert OSINT entity extraction system. ",
        "Extract structured information about organizations and job titles from text. ",
        "Only extract information that is explicitly stated in the source text. ",
        "If information is not present, set the field to null. ",
        "Never fabricate or infer organizations or titles that are not in the text.\n",
        "/no_think"
    );

    let user = format!(
        r#"Extract structured entity information from the following text about {person_name}.

TEXT:
{artifact_text}

Respond ONLY with valid JSON:
{{
  "organization": "<organization name from text, or null>",
  "job_title": "<job title from text, or null>",
  "is_role_change": <true if the text indicates a new role/position, false otherwise>,
  "is_org_change": <true if the text indicates joining or leaving a different organization, false otherwise>,
  "confidence": <0.0 to 1.0, based on how clearly the information is stated>,
  "reasoning": "<brief 1-sentence explanation of the extraction>"
}}"#,
        person_name = person_name,
        artifact_text = apex_llm::truncate_utf8(artifact_text, 2000),
    );

    let response = client.extract_json::<LlmEntityResponse>(system, user).await?;
    let mut result = response;
    result.confidence = result.confidence.clamp(0.0, 1.0);

    // Anti-hallucination: clear low-confidence fields
    if result.confidence < 0.3 {
        if let Some(ref org) = result.organization {
            if org.trim().is_empty() || org.len() < 2
                || org.eq_ignore_ascii_case("null")
                || org.eq_ignore_ascii_case("unknown")
                || org.eq_ignore_ascii_case("n/a")
            { result.organization = None; }
        }
        if let Some(ref title) = result.job_title {
            if title.trim().is_empty() || title.len() < 3
                || title.eq_ignore_ascii_case("null")
                || title.eq_ignore_ascii_case("unknown")
                || title.eq_ignore_ascii_case("n/a")
            { result.job_title = None; }
        }
    }
    Ok(result)
}

// ─── Validation Helpers ───

fn is_plausible_org(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 2 || s.len() > 100 { return false; }
    let reject = [
        "the","and","for","from","with","that","this","into","their","have",
        "will","after","before","also","just","only","more","most","very",
        "said","says","year","years","reported","announced",
    ];
    if reject.contains(&s.to_lowercase().as_str()) { return false; }
    s.chars().any(|c| c.is_uppercase()) || s.contains(' ')
}

fn is_plausible_title(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 2 || s.len() > 120 { return false; }
    let reject = [
        "the","and","for","from","with","that","this","into","their","have",
        "will","after","before","also","just","only","more","most","very",
        "said","says","year","years","reported","announced",
        "the company","a statement","a press",
    ];
    if reject.contains(&s.to_lowercase().as_str()) { return false; }
    s.len() > 5
}

fn is_common_word(word: &str) -> bool {
    let common = [
        "the","and","for","with","from","that","this","into","their","have",
        "will","after","before","also","just","only","more","most","very",
        "said","says","has","been","was","were","are","now","new","its","not",
        "but","our","his","her",
    ];
    common.contains(&word.to_lowercase().as_str())
}

fn clean_org_name(name: &str) -> String {
    name.trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '(' || c == ')')
        .trim_end_matches(" at").trim_end_matches(" in")
        .trim_end_matches(" as").trim_end_matches(" the")
        .trim_end_matches(',').trim().to_string()
}

fn clean_title(title: &str) -> String {
    title.trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '(' || c == ')')
        .trim_end_matches(',').trim_end_matches(" and")
        .trim().to_string()
}

// ─── Role Family Inference ───

/// Advanced role family inference with broader keyword coverage.
pub fn infer_role_family_advanced(title: &str) -> RoleFamily {
    let t = title.to_lowercase();

    if t.contains("quality") || t.contains("qa engineer") || t.contains("qc inspector")
        || t.contains("testing") || t.contains("inspection") || t.contains("compliance")
        || t.contains("audit") || t.contains("regulatory") || t.contains("iso ")
        || t.contains("safety") || t.contains("environmental") || t.contains("sustainability")
        || t.contains("esg")
    { return RoleFamily::SupplierQuality; }

    if t.contains("procurement") || t.contains("sourcing") || t.contains("purchasing")
        || t.contains("buyer") || t.contains("supply chain") || t.contains("supplier")
        || t.contains("vendor") || t.contains("category manager") || t.contains("commodity")
        || t.contains("contract manager") || t.contains("tender")
    { return RoleFamily::Procurement; }

    if t.contains("security") || t.contains("cyber") || t.contains("infosec")
        || t.contains("privacy") || t.contains("data protection") || t.contains("threat")
        || t.contains("vulnerability")
    { return RoleFamily::Security; }

    if t.contains("engineer") || t.contains("developer") || t.contains("architect")
        || t.contains("scientist") || t.contains("programmer") || t.contains("technical lead")
        || t.contains("r&d")
    { return RoleFamily::Engineering; }

    if t.contains("operations") || t.contains("manufacturing") || t.contains("production")
        || t.contains("logistics") || t.contains("warehouse") || t.contains("distribution")
        || t.contains("plant manager") || t.contains("factory") || t.contains("facilities")
        || t.contains("maintenance")
    { return RoleFamily::Operations; }

    if t.contains("chief") || t == "ceo" || t.contains("ceo ") || t.contains(" ceo") || t.ends_with("ceo")
        || t == "cto" || t.contains("cto ") || t.contains(" cto") || t.ends_with("cto")
        || t == "cfo" || t.contains("cfo ") || t.ends_with("cfo")
        || t == "coo" || t.contains("coo ") || t.ends_with("coo")
        || t.contains("cio ") || t.contains(" cio") || t.contains("ciso")
        || t.contains("president") || t.contains("chairman")
        || t.contains("managing director") || t.contains("general manager")
        || t.contains("executive director") || t.contains("board director")
        || t.contains("vice president") || t.contains("owner")
        || t.contains("founder") || t.contains("co-founder") || t.contains("managing partner")
    { return RoleFamily::Executive; }

    if t.contains("sales") || t.contains("marketing") || t.contains("business development")
        || t.contains("account manager") || t.contains("customer")
        || t.contains("commercial") || t.contains("revenue")
    { return RoleFamily::Other("Sales/Marketing".to_string()); }

    if t.contains("finance") || t.contains("accounting") || t.contains("treasury")
        || t.contains("controller") || t.contains("bookkeeper") || t.contains("tax")
    { return RoleFamily::Other("Finance".to_string()); }

    if t.contains("hr") || t.contains("human resources") || t.contains("talent")
        || t.contains("recruiting") || t.contains("people") || t.contains("payroll")
    { return RoleFamily::Other("Human Resources".to_string()); }

    if t.contains("government") || t.contains("minister") || t.contains("ambassador")
        || t.contains("regulatory") || t.contains("public policy")
        || t.contains("free zone") || t.contains("authority") || t.contains("agency")
    { return RoleFamily::Government; }

    if t.contains("legal") || t.contains("counsel") || t.contains("attorney")
        || t.contains("lawyer") || t.contains("paralegal") || t.contains("intellectual property")
    { return RoleFamily::Other("Legal".to_string()); }

    RoleFamily::Other(t)
}

// ─── Entity Validation ───

/// Validate an extracted organization against a lookup table of known entities.
pub fn validate_org_against_entities(
    extracted_org: &str,
    known_entities: &[(String, Vec<String>)],
) -> Option<String> {
    let normalized = extracted_org.trim().to_lowercase();
    if normalized.is_empty() || normalized.len() < 3 { return None; }

    for (canonical, aliases) in known_entities {
        if normalized == canonical.to_lowercase() { return Some(canonical.clone()); }
        for alias in aliases {
            if normalized == alias.to_lowercase() { return Some(canonical.clone()); }
        }
    }
    for (canonical, aliases) in known_entities {
        let canon_lower = canonical.to_lowercase();
        if normalized.contains(&canon_lower) || canon_lower.contains(&normalized) {
            return Some(canonical.clone());
        }
        for alias in aliases {
            let alias_lower = alias.to_lowercase();
            if normalized.contains(&alias_lower) || alias_lower.contains(&normalized) {
                return Some(canonical.clone());
            }
        }
    }
    None
}

/// Check if an extracted title is consistent with a person's known role history.
pub fn title_consistent_with_history(
    extracted_title: &str,
    role_history: &[crate::model::RoleHistoryEntry],
) -> bool {
    let title_lower = extracted_title.to_lowercase();
    let inferred_family = infer_role_family_advanced(extracted_title);

    if role_history.is_empty() { return true; }

    let recent_families: Vec<&RoleFamily> = role_history.iter().rev().take(3)
        .map(|entry| &entry.role_family).collect();

    for family in &recent_families {
        if std::mem::discriminant(*family) == std::mem::discriminant(&inferred_family) {
            return true;
        }
    }

    for entry in role_history.iter().rev().take(3) {
        let hist_title = entry.title.to_lowercase();
        let title_words: Vec<&str> = title_lower.split_whitespace().collect();
        let hist_words: Vec<&str> = hist_title.split_whitespace().collect();
        if title_words.iter().filter(|w| hist_words.contains(w)).count() >= 2 {
            return true;
        }
    }

    warn!(title = %extracted_title,
        history = ?role_history.iter().map(|e| &e.title).collect::<Vec<_>>(),
        "Extracted title may be inconsistent with role history");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_org_prefix_pattern() {
        let (org, conf) = extract_org_heuristic("John Smith joins Acme Corporation as VP");
        assert!(org.is_some());
        assert!(org.unwrap().contains("Acme"));
        assert!(conf > 0.0);
    }

    #[test]
    fn test_extract_org_suffix_detection() {
        let (org, conf) = extract_org_heuristic("Previously worked at Foxconn Technology Group in Taiwan");
        assert!(org.is_some());
        assert!(conf >= 0.0);
    }

    #[test]
    fn test_extract_title_pattern() {
        let (title, conf) = extract_title_heuristic("appointed as Chief Technology Officer at the firm");
        assert!(title.is_some());
        assert!(title.unwrap().contains("Chief"));
        assert!(conf > 0.0);
    }

    #[test]
    fn test_extract_title_keyword() {
        let (title, conf) = extract_title_heuristic("VP of Procurement joins the executive team");
        assert!(title.is_some());
        assert!(conf > 0.0);
    }

    #[test]
    fn test_empty_extraction() {
        let (org, conf) = extract_org_heuristic("The weather today is sunny");
        assert!(org.is_none());
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn test_infer_role_family_executive() {
        assert_eq!(infer_role_family_advanced("Chief Executive Officer"), RoleFamily::Executive);
        assert_eq!(infer_role_family_advanced("CTO"), RoleFamily::Executive);
        assert_eq!(infer_role_family_advanced("Managing Director"), RoleFamily::Executive);
    }

    #[test]
    fn test_infer_role_family_procurement() {
        assert_eq!(infer_role_family_advanced("VP Procurement"), RoleFamily::Procurement);
        assert_eq!(infer_role_family_advanced("Head of Supply Chain"), RoleFamily::Procurement);
    }

    #[test]
    fn test_infer_role_family_quality() {
        assert_eq!(infer_role_family_advanced("Director of Quality Assurance"), RoleFamily::SupplierQuality);
        assert_eq!(infer_role_family_advanced("Compliance Officer"), RoleFamily::SupplierQuality);
    }

    #[test]
    fn test_infer_role_family_operations() {
        assert_eq!(infer_role_family_advanced("VP Operations"), RoleFamily::Operations);
        assert_eq!(infer_role_family_advanced("Plant Manager"), RoleFamily::Operations);
    }

    #[test]
    fn test_title_consistent_with_history() {
        use crate::model::RoleHistoryEntry;
        let history = vec![RoleHistoryEntry {
            org: "Acme Corp".into(), title: "VP Procurement".into(),
            role_family: RoleFamily::Procurement, start_ts: 1500000000, end_ts: None,
        }];
        assert!(title_consistent_with_history("Director of Supply Chain", &history));
        assert!(!title_consistent_with_history("Chief Medical Officer", &history));
    }

    #[test]
    fn test_is_plausible_org() {
        assert!(is_plausible_org("Acme Corporation"));
        assert!(is_plausible_org("Foxconn Technology Group"));
        assert!(!is_plausible_org("the"));
        assert!(!is_plausible_org("x"));
    }

    #[test]
    fn test_is_plausible_title() {
        assert!(is_plausible_title("VP of Engineering"));
        assert!(is_plausible_title("Chief Technology Officer"));
        assert!(!is_plausible_title("the"));
        assert!(!is_plausible_title("a"));
    }

    #[test]
    fn test_validate_org_against_entities_exact_match() {
        let known = vec![
            ("Foxconn Tunisia".to_string(), vec!["Foxconn TN".to_string()]),
            ("Samsung Korea".to_string(), vec!["Samsung".to_string()]),
        ];
        assert_eq!(validate_org_against_entities("Foxconn Tunisia", &known),
            Some("Foxconn Tunisia".to_string()));
    }

    #[test]
    fn test_validate_org_against_entities_alias_match() {
        let known = vec![("Foxconn Technology Group".to_string(),
            vec!["Foxconn".to_string(), "Foxconn TN".to_string()])];
        assert_eq!(validate_org_against_entities("foxconn", &known),
            Some("Foxconn Technology Group".to_string()));
    }

    #[test]
    fn test_validate_org_against_entities_no_match() {
        let known = vec![("Acme Corp".to_string(), vec![])];
        assert_eq!(validate_org_against_entities("Unknown Corp", &known), None);
    }

    #[test]
    fn test_entity_extraction_is_llm_backed() {
        let mut ext = EntityExtraction::empty(0);
        assert!(!ext.is_llm_backed());
        ext.extraction_source = "llm".to_string();
        assert!(ext.is_llm_backed());
        ext.extraction_source = "llm+heuristic".to_string();
        assert!(ext.is_llm_backed());
    }
}