//! POI data validator — anti-hallucination guards and fact-checking.
//!
//! Ensures that POI profile data is grounded in verifiable sources:
//! - Validates extracted org names against known entities table
//! - Validates job titles against available source evidence
//! - Detects LLM hallucinations vs genuine extractions
//! - Computes evidence-backed confidence scores
//!
//! # Design
//! Every field on a PoiProfile should have an audit trail back to
//! either a crawled artifact source_url or an explicit manual entry.
//! Fields without evidence chains are flagged and their confidence reduced.

use crate::model::*;
use serde::{Deserialize, Serialize};
// tracing macros are used via the `tracing` crate re-export

// ─── Validation Result Types ───

/// Result of validating a single field on a POI profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldValidation {
    /// The field name (e.g., "org", "current_role", "public_email")
    pub field: String,
    /// The current value of the field
    pub current_value: String,
    /// Whether the value is backed by evidence
    pub has_evidence: bool,
    /// Evidence sources (URLs or artifact titles)
    pub evidence_sources: Vec<String>,
    /// Confidence in the field value (0.0-1.0)
    pub confidence: f64,
    /// Whether the value appears hallucinated (LLM-generated without evidence)
    pub suspected_hallucination: bool,
    /// Issues found with this field
    pub issues: Vec<String>,
}

/// Result of a full profile validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileValidation {
    /// The person_id being validated
    pub person_id: String,
    /// Per-field validation results
    pub fields: Vec<FieldValidation>,
    /// Overall evidence score (0.0-1.0)
    pub evidence_score: f64,
    /// Whether the profile passes minimum quality thresholds
    pub passes_quality: bool,
    /// Number of fields with zero evidence
    pub unverified_fields: usize,
    /// Whether any fields appear hallucinated
    pub has_hallucinations: bool,
    /// Recommendations for improving profile quality
    pub recommendations: Vec<String>,
}

impl ProfileValidation {
    /// Minimum evidence score required to pass quality check.
    pub const MIN_EVIDENCE_SCORE: f64 = 0.4;

    /// Maximum allowed unverified fields before quality fails.
    pub const MAX_UNVERIFIED_FIELDS: usize = 3;

    pub fn passes(&self) -> bool {
        self.evidence_score >= Self::MIN_EVIDENCE_SCORE
            && self.unverified_fields <= Self::MAX_UNVERIFIED_FIELDS
            && !self.has_hallucinations
    }
}

// ─── Validation Functions ───

/// Validate a POI profile against its artifact evidence.
///
/// Checks each key field against available artifact sources:
/// - `org` — must appear in at least one artifact's source URL domain or content
/// - `current_role` — must be extractable from artifact content
/// - `public_email` — format validation
/// - `public_bio` — must not be LLM-generated filler text
/// - `name` — must appear in at least one artifact title/content
pub fn validate_profile(profile: &PoiProfile) -> ProfileValidation {
    let mut fields = Vec::new();
    let mut unverified_fields = 0;
    let mut total_confidence = 0.0;
    let field_count = 5.0; // We validate 5 key fields

    // 1. Validate org
    let org_validation = validate_org_field(profile);
    total_confidence += org_validation.confidence;
    if !org_validation.has_evidence {
        unverified_fields += 1;
    }
    fields.push(org_validation);

    // 2. Validate current_role
    let role_validation = validate_role_field(profile);
    total_confidence += role_validation.confidence;
    if !role_validation.has_evidence {
        unverified_fields += 1;
    }
    fields.push(role_validation);

    // 3. Validate public_email
    let email_validation = validate_email_field(profile);
    total_confidence += email_validation.confidence;
    if !email_validation.has_evidence {
        // Email without evidence is acceptable if format is valid
    }
    fields.push(email_validation);

    // 4. Validate public_bio
    let bio_validation = validate_bio_field(profile);
    total_confidence += bio_validation.confidence;
    if !bio_validation.has_evidence {
        unverified_fields += 1;
    }
    fields.push(bio_validation);

    // 5. Validate name
    let name_validation = validate_name_field(profile);
    total_confidence += name_validation.confidence;
    if !name_validation.has_evidence {
        unverified_fields += 1;
    }
    fields.push(name_validation);

    let evidence_score = (total_confidence / field_count).clamp(0.0, 1.0);
    let has_hallucinations = fields
        .iter()
        .any(|f| f.suspected_hallucination);

    let mut recommendations = Vec::new();
    if evidence_score < ProfileValidation::MIN_EVIDENCE_SCORE {
        recommendations.push(format!(
            "Evidence score {:.2} is below minimum {:.2}. Add more artifacts.",
            evidence_score,
            ProfileValidation::MIN_EVIDENCE_SCORE
        ));
    }
    if unverified_fields > 0 {
        recommendations.push(format!(
            "{} fields have no evidence backing. Recrawl affected sources.",
            unverified_fields
        ));
    }
    if has_hallucinations {
        recommendations.push(
            "Suspected hallucinated data detected. Review LLM-generated fields.".to_string(),
        );
    }
    if profile.artifacts.len() < 3 {
        recommendations.push(
            "Very few artifacts available. Expand crawling to more sources.".to_string(),
        );
    }
    if profile.psychological.pain_index == 0.0
        && profile.psychological.risk_tolerance == 0.0
        && profile.psychological.preferred_proof.is_empty()
    {
        recommendations.push(
            "Psychological profile is at defaults. Run LLM enrichment or compute from artifacts."
                .to_string(),
        );
    }

    ProfileValidation {
        person_id: profile.person_id.clone(),
        fields,
        evidence_score,
        passes_quality: evidence_score >= ProfileValidation::MIN_EVIDENCE_SCORE
            && unverified_fields <= ProfileValidation::MAX_UNVERIFIED_FIELDS
            && !has_hallucinations,
        unverified_fields,
        has_hallucinations,
        recommendations,
    }
}

/// Validate the org field against evidence in artifacts.
fn validate_org_field(profile: &PoiProfile) -> FieldValidation {
    let org_lower = profile.org.to_lowercase();
    let mut evidence_sources = Vec::new();
    let mut found_in_content = false;
    let mut found_in_url = false;

    for artifact in &profile.artifacts {
        // Check if org name appears in artifact content
        if artifact
            .content_summary
            .to_lowercase()
            .contains(&org_lower)
            || artifact.title.to_lowercase().contains(&org_lower)
        {
            found_in_content = true;
            evidence_sources.push(artifact.title.clone());
        }

        // Check if org domain appears in source URL
        if let Some(ref url) = artifact.source_url {
            if url.to_lowercase().contains(&org_lower.replace(' ', ""))
                || url.to_lowercase().contains(&org_lower.replace(' ', "-"))
            {
                found_in_url = true;
                if !evidence_sources.contains(url) {
                    evidence_sources.push(url.clone());
                }
            }
        }
    }

    let has_evidence = found_in_content || found_in_url;
    let confidence = if found_in_url {
        0.9
    } else if found_in_content {
        0.6
    } else if !profile.org.is_empty() && !profile.artifacts.is_empty() {
        // Org is set but no evidence — possible hallucination
        0.1
    } else if profile.artifacts.is_empty() {
        0.05 // No artifacts at all — confidence is rock-bottom
    } else {
        0.0
    };

    let mut issues = Vec::new();
    if !has_evidence && !profile.org.is_empty() {
        issues.push(format!(
            "Org '{}' has no supporting evidence in {} artifacts",
            profile.org,
            profile.artifacts.len()
        ));
    }

    FieldValidation {
        field: "org".to_string(),
        current_value: profile.org.clone(),
        has_evidence,
        evidence_sources,
        confidence,
        suspected_hallucination: profile.artifacts.len() >= 3
            && !has_evidence
            && !profile.org.is_empty(),
        issues,
    }
}

/// Validate the current_role field against evidence.
fn validate_role_field(profile: &PoiProfile) -> FieldValidation {
    let role_lower = profile.current_role.to_lowercase();
    let mut evidence_sources = Vec::new();

    // Check if role appears in any artifact
    for artifact in &profile.artifacts {
        if artifact
            .content_summary
            .to_lowercase()
            .contains(&role_lower)
            || artifact.title.to_lowercase().contains(&role_lower)
        {
            evidence_sources.push(artifact.title.clone());
        }
    }

    // Also check role history for consistency
    let history_consistent = if !profile.role_history.is_empty() {
        profile
            .role_history
            .iter()
            .any(|entry| entry.title.to_lowercase().contains(&role_lower))
    } else {
        false
    };

    let has_evidence = !evidence_sources.is_empty() || history_consistent;
    let confidence = if !evidence_sources.is_empty() {
        0.8
    } else if history_consistent {
        0.5
    } else if !profile.current_role.is_empty() && !profile.artifacts.is_empty() {
        0.1 // Possible hallucination
    } else {
        0.0
    };

    let mut issues = Vec::new();
    if !has_evidence && !profile.current_role.is_empty() {
        issues.push(format!(
            "Role '{}' has no supporting evidence",
            profile.current_role
        ));
    }

    FieldValidation {
        field: "current_role".to_string(),
        current_value: profile.current_role.clone(),
        has_evidence,
        evidence_sources,
        confidence,
        suspected_hallucination: profile.artifacts.len() >= 3
            && !has_evidence
            && !profile.current_role.is_empty(),
        issues,
    }
}

/// Validate the public_email format and domain correlation.
fn validate_email_field(profile: &PoiProfile) -> FieldValidation {
    let evidence_sources = Vec::<String>::new();
    let has_email = profile.public_email.is_some();

    // Email format validation
    let format_valid = profile.has_valid_public_email();

    // Check if email domain correlates with org
    let domain_correlated = if let Some(ref email) = profile.public_email {
        if let Some(domain) = email.split('@').nth(1) {
            let domain_lower = domain.to_lowercase();
            // Check if org name appears in email domain
            let org_words: Vec<&str> = profile.org.split_whitespace().collect();
            org_words.iter().any(|w| {
                domain_lower.contains(&w.to_lowercase())
                    && w.len() >= 3
            }) || profile
                .org
                .to_lowercase()
                .replace(' ', "")
                .contains(&domain_lower.replace('.', ""))
        } else {
            false
        }
    } else {
        false
    };

    let has_evidence = format_valid;
    let confidence = if format_valid && domain_correlated {
        0.95
    } else if format_valid {
        0.6
    } else if has_email {
        0.1 // Invalid format
    } else {
        1.0 // No email is acceptable
    };

    let mut issues = Vec::new();
    if has_email && !format_valid {
        issues.push("Email format is invalid".to_string());
    }
    if has_email && format_valid && !domain_correlated {
        issues.push("Email domain does not correlate with org".to_string());
    }

    FieldValidation {
        field: "public_email".to_string(),
        current_value: profile
            .public_email
            .clone()
            .unwrap_or_else(|| "none".to_string()),
        has_evidence,
        evidence_sources,
        confidence,
        suspected_hallucination: false, // Emails are rarely hallucinated
        issues,
    }
}

/// Validate the public_bio field is not LLM-generated filler.
fn validate_bio_field(profile: &PoiProfile) -> FieldValidation {
    let mut evidence_sources = Vec::new();

    // Check if bio content appears in artifacts
    for artifact in &profile.artifacts {
        // Check for significant overlap (at least 30 chars in common)
        let overlap = longest_common_substring(
            &profile.public_bio.to_lowercase(),
            &artifact.content_summary.to_lowercase(),
        );
        if overlap.len() >= 30 {
            evidence_sources.push(artifact.title.clone());
        }
    }

    // Detect LLM-filler patterns
    let llm_filler_patterns = [
        "is a highly experienced",
        "brings over",
        "has extensive",
        "renowned for",
        "a proven track record",
        "passionate about",
        "dedicated to",
        "committed to",
        "industry veteran",
        "thought leader",
        "results-driven",
        "strategic thinker",
        "innovative",
        "visionary",
        "has demonstrated",
        "a wealth of experience",
        "deep understanding",
    ];

    let has_filler = llm_filler_patterns
        .iter()
        .any(|p| profile.public_bio.to_lowercase().contains(p));
    let is_too_short = profile.public_bio.len() < 20;
    let is_too_generic = profile.public_bio.len() < 50 && has_filler;

    let has_evidence = !evidence_sources.is_empty() || is_too_short;
    let confidence = if !evidence_sources.is_empty() {
        0.7
    } else if is_too_generic || has_filler {
        0.1 // Looks like LLM filler
    } else if is_too_short {
        0.3
    } else {
        0.0
    };

    let mut issues = Vec::new();
    if has_filler && evidence_sources.is_empty() {
        issues.push("Bio contains LLM-filler language without evidence".to_string());
    }
    if is_too_generic {
        issues.push("Bio is too generic — likely auto-generated".to_string());
    }

    FieldValidation {
        field: "public_bio".to_string(),
        current_value: if profile.public_bio.len() > 100 {
            format!("{}...", &profile.public_bio[..100])
        } else {
            profile.public_bio.clone()
        },
        has_evidence,
        evidence_sources,
        confidence,
        suspected_hallucination: has_filler,
        issues,
    }
}

/// Validate the name field appears in artifacts.
fn validate_name_field(profile: &PoiProfile) -> FieldValidation {
    let name_lower = profile.name.to_lowercase();
    let mut evidence_sources = Vec::new();

    for artifact in &profile.artifacts {
        if artifact.title.to_lowercase().contains(&name_lower)
            || artifact
                .content_summary
                .to_lowercase()
                .contains(&name_lower)
        {
            evidence_sources.push(artifact.title.clone());
        }
    }

    // Also check name variants
    for variant in &profile.name_variants {
        for artifact in &profile.artifacts {
            if artifact.title.to_lowercase().contains(&variant.to_lowercase()) {
                let src = format!("variant_match: {}", artifact.title);
                if !evidence_sources.contains(&src) {
                    evidence_sources.push(src);
                }
            }
        }
    }

    let has_evidence = !evidence_sources.is_empty();
    let confidence = if has_evidence {
        0.9
    } else if !profile.artifacts.is_empty() {
        0.2 // Name not found in artifacts
    } else {
        0.0 // No artifacts at all
    };

    let mut issues = Vec::new();
    if !has_evidence && !profile.name.is_empty() && profile.artifacts.len() >= 1 {
        issues.push(format!(
            "Name '{}' not found in {} artifacts",
            profile.name,
            profile.artifacts.len()
        ));
    }

    FieldValidation {
        field: "name".to_string(),
        current_value: profile.name.clone(),
        has_evidence,
        evidence_sources,
        confidence,
        suspected_hallucination: false, // Names are usually from source data
        issues,
    }
}

// ─── Utility Functions ───

/// Compute the longest common substring between two strings.
fn longest_common_substring(a: &str, b: &str) -> String {
    let a_bytes: Vec<u8> = a.bytes().collect();
    let b_bytes: Vec<u8> = b.bytes().collect();
    let mut max_len = 0;
    let mut max_end = 0;

    // Simple O(n*m) DP — good enough for short strings
    let mut dp = vec![vec![0u32; b_bytes.len() + 1]; a_bytes.len() + 1];
    for i in 1..=a_bytes.len() {
        for j in 1..=b_bytes.len() {
            if a_bytes[i - 1] == b_bytes[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
                if dp[i][j] > max_len {
                    max_len = dp[i][j];
                    max_end = i;
                }
            }
        }
    }

    if max_len > 0 {
        String::from_utf8_lossy(&a_bytes[max_end - max_len as usize..max_end]).to_string()
    } else {
        String::new()
    }
}

// ─── Sanity Checks ───

/// Quick sanity check on a profile — returns a severity and message if
/// something looks obviously wrong (e.g., name is empty, org is a person name).
pub fn sanity_check(profile: &PoiProfile) -> Vec<String> {
    let mut issues = Vec::new();

    if profile.name.is_empty() {
        issues.push("CRITICAL: Name is empty".to_string());
    }
    if profile.name.len() < 2 {
        issues.push(format!("SUSPICIOUS: Name is too short: '{}'", profile.name));
    }
    if profile.org.is_empty() {
        issues.push("WARNING: Organization is empty".to_string());
    }
    if profile.current_role.is_empty() {
        issues.push("WARNING: Current role is empty".to_string());
    }
    if profile.person_id.is_empty() {
        issues.push("CRITICAL: person_id is empty".to_string());
    }

    // Check for common hallucination patterns
    let llm_tells = [
        "Based on the available",
        "It appears that",
        "The evidence suggests",
        "According to the",
        "As an AI",
        "I cannot determine",
        "Unknown",
        "Not available",
        "N/A",
        "TBD",
        "Lorem ipsum",
    ];

    for tell in &llm_tells {
        if profile.public_bio.contains(tell) {
            issues.push(format!(
                "HALLUCINATION: Bio contains LLM phrase '{}'",
                tell
            ));
        }
        if profile.current_role.contains(tell) {
            issues.push(format!(
                "HALLUCINATION: Role contains placeholder '{}'",
                tell
            ));
        }
    }

    // Check that org doesn't look like a person name
    let person_name_words: Vec<&str> = profile.name.split_whitespace().collect();
    let org_lower = profile.org.to_lowercase();
    let matching_name_words = person_name_words
        .iter()
        .filter(|w| w.len() >= 3 && org_lower.contains(&w.to_lowercase()))
        .count();
    if matching_name_words >= 2 {
        issues.push(format!(
            "SUSPICIOUS: Org '{}' appears to contain person name '{}'",
            profile.org, profile.name
        ));
    }

    // Check timestamp validity
    if profile.last_updated_utc < 1000000000 {
        issues.push(format!(
            "SUSPICIOUS: Last updated timestamp is ancient: {}",
            profile.last_updated_utc
        ));
    }

    issues
}

/// Validate a PsychProfile — ensures it's not the neutral default.
pub fn validate_psych_profile(psych: &PsychProfile) -> Option<String> {
    if !psych.is_enriched() {
        return Some(
            "Psychological profile is at default values — has not been enriched with real data"
                .to_string(),
        );
    }

    if psych.pain_index < 0.0 || psych.pain_index > 1.0 {
        return Some(format!("Pain index out of range: {}", psych.pain_index));
    }
    if psych.risk_tolerance < 0.0 || psych.risk_tolerance > 1.0 {
        return Some(format!(
            "Risk tolerance out of range: {}",
            psych.risk_tolerance
        ));
    }

    // Check that psych profile was computed from actual artifacts
    let quality = psych.enrichment_quality();
    if quality < 0.3 {
        return Some(format!(
            "Psychological profile quality too low ({:.2}) — likely default or generated with insufficient data",
            quality
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_profile() -> PoiProfile {
        PoiProfile {
            person_id: "test-001".into(),
            name: "Jane Smith".into(),
            name_variants: vec![],
            org: "Acme Corporation".into(),
            org_id: Some("org_acme".into()),
            current_role: "VP Procurement".into(),
            role_family: RoleFamily::Procurement,
            region: "US".into(),
            country_code: "US".into(),
            public_bio: "20 years experience in procurement".into(),
            public_email: Some("jane@acme.com".into()),
            artifacts: vec![
                PoiArtifact {
                    artifact_type: "press_release".into(),
                    title: "Jane Smith appointed VP Procurement at Acme Corp".into(),
                    content_summary: "Acme Corporation announced that Jane Smith has been appointed VP Procurement".into(),
                    source_url: Some("https://acme.com/press/2024/jane-smith".into()),
                    ts_utc: 1700000000,
                },
            ],
            priority_vector: PriorityVector {
                cost: 0.4,
                quality: 0.2,
                speed: 0.1,
                resilience: 0.1,
                compliance: 0.1,
                security: 0.1,
                confidence: 0.7,
            },
            psychological: PsychProfile {
                decision_style: DecisionStyle::CostFirst,
                change_appetite: ChangeAppetite::Pragmatist,
                pain_index: 0.6,
                preferred_proof: vec![ProofType::KpiMetrics, ProofType::CostTransparency],
                risk_tolerance: 0.4,
            },
            influence: InfluenceProfile {
                influence_score: 50.0,
                graph_centrality: 40.0,
                public_recurrence: 30.0,
                role_seniority_score: 55.0,
                network_size: 10,
            },
            engagement: None,
            role_history: vec![],
            last_updated_utc: 1700000000,
            profile_completeness: 0.8,
        }
    }

    #[test]
    fn test_validate_profile_good_data() {
        let profile = make_test_profile();
        let validation = validate_profile(&profile);
        assert!(
            validation.passes_quality,
            "Well-evidenced profile should pass quality: evidence_score={}",
            validation.evidence_score
        );
        assert!(
            !validation.has_hallucinations,
            "Well-evidenced profile should not have hallucinations"
        );
    }

    #[test]
    fn test_validate_profile_no_artifacts() {
        let mut profile = make_test_profile();
        profile.artifacts = vec![];
        let validation = validate_profile(&profile);
        assert!(
            validation.evidence_score < 0.5,
            "Profile with no artifacts should have low evidence score"
        );
    }

    #[test]
    fn test_sanity_check_empty_name() {
        let mut profile = make_test_profile();
        profile.name = String::new();
        let issues = sanity_check(&profile);
        assert!(issues.iter().any(|i| i.contains("CRITICAL")));
    }

    #[test]
    fn test_sanity_check_llm_tells() {
        let mut profile = make_test_profile();
        profile.public_bio = "Based on the available information, this person appears to be...".into();
        let issues = sanity_check(&profile);
        assert!(issues.iter().any(|i| i.contains("HALLUCINATION")));
    }

    #[test]
    fn test_validate_psych_profile_enriched() {
        let psych = PsychProfile {
            decision_style: DecisionStyle::CostFirst,
            change_appetite: ChangeAppetite::EarlyAdopter,
            pain_index: 0.7,
            preferred_proof: vec![ProofType::KpiMetrics],
            risk_tolerance: 0.5,
        };
        assert!(validate_psych_profile(&psych).is_none());
    }

    #[test]
    fn test_validate_psych_profile_default() {
        let psych = PsychProfile::default_profile();
        let issue = validate_psych_profile(&psych);
        assert!(issue.is_some());
        assert!(issue.unwrap().contains("default"));
    }

    #[test]
    fn test_org_name_in_person_name_detection() {
        let mut profile = make_test_profile();
        profile.org = "Jane Smith Consulting".into();
        let issues = sanity_check(&profile);
        assert!(issues.iter().any(|i| i.contains("SUSPICIOUS")));
    }

    #[test]
    fn test_longest_common_substring() {
        let lcs = longest_common_substring("hello world", "world hello");
        assert!(!lcs.is_empty());
        // "hello" should be the LCS (length 5) - matches both contain it
        assert!(lcs.len() >= 5);
    }
}
