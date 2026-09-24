//! Psychological Profiling Compute Pipeline — Canonical Engine
//!
//! Takes raw observation data and POI profiles and produces real psychological
//! assessments with evidence-backed scores. This is the **canonical compute engine**
//! that drives both batch persistence via `psych_store` and on-the-fly computation
//! used by `psychological.rs` in the insights pipeline.
//!
//! # Architecture
//!
//! There are two profiling entry points in the codebase:
//! - **`psych_compute.rs`** (this file): The canonical engine. Uses structured
//!   `PsychObservation` records with source URLs and timestamps. Produces
//!   `PsychProfileResult` with full evidence trails. Used by the worker pipeline
//!   and persists via `psych_store.rs`.
//! - **`psychological.rs`**: A convenience wrapper that computes profiles on-the-fly
//!   from `PoiArtifact` data during insight generation. Delegates to this module's
//!   keyword lexica and scoring functions for consistent results.
//!
//! Both modules share the same keyword lexica (authoritative, collaborative,
//! analytical, data-driven, urgency, change-open markers) to ensure consistent
//! profile dimensions regardless of which entry point is used.
//!
//! # Pipeline stages:
//! 1. Communication pattern analysis → decision_style
//! 2. Career history analysis → change_appetite
//! 3. Public signal analysis → pain_index
//! 4. Behavioral evidence → risk_tolerance
//! 5. Delta detection → behavioral patterns
//! 6. Strategy generation → engagement profile

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use std::collections::HashMap;
use tracing::{info, warn};
use uuid::Uuid;

/// A single observation record used as input to the psych pipeline.
#[derive(Debug, Clone)]
pub struct PsychObservation {
    pub text: String,
    pub source_url: Option<String>,
    pub source_domain: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub sentiment_score: Option<f64>,
}

/// A raw profile snapshot before enrichment.
#[derive(Debug, Clone)]
pub struct RawProfileSnapshot {
    pub person_id: String,
    pub person_name: String,
    pub current_title: String,
    pub role_family: String,
    pub organization: String,
    pub career_length_years: Option<f64>,
    pub job_change_count: Option<u32>,
    pub public_statements_count: u32,
    pub observations: Vec<PsychObservation>,
}

/// The computed psychological profile result.
#[derive(Debug, Clone)]
pub struct PsychProfileResult {
    pub profile_id: Uuid,
    pub person_id: String,
    pub decision_style: String,
    pub change_appetite: String,
    pub pain_index: f64,
    pub risk_tolerance: f64,
    pub preferred_proof: Vec<String>,
    pub enrichment_quality: f64,
    pub evidence_sources: Vec<String>,
    pub behavioral_patterns: Vec<BehavioralPatternResult>,
    pub engagement: Option<EngagementResult>,
}

#[derive(Debug, Clone)]
pub struct BehavioralPatternResult {
    pub event_type: String,
    pub title: String,
    pub description: String,
    pub confidence: f64,
    pub evidence_urls: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EngagementResult {
    pub talking_points: Vec<String>,
    pub opening_topics: Vec<String>,
    pub avoid_topics: Vec<String>,
    pub best_channel: String,
    pub best_timing: Option<String>,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Sentiment & language lexica
// ═══════════════════════════════════════════════════════════════════════════════

/// Keywords indicating an authoritative communication style.
const AUTHORITATIVE_KEYWORDS: &[&str] = &[
    "must",
    "require",
    "mandate",
    "directive",
    "order",
    "decree",
    "instruct",
    "command",
    "shall",
    "comply",
    "enforce",
    "demand",
];

/// Keywords indicating a collaborative communication style.
const COLLABORATIVE_KEYWORDS: &[&str] = &[
    "together",
    "collaborate",
    "partner",
    "team",
    "joint",
    "shared",
    "co-create",
    "alignment",
    "consensus",
    "stakeholder",
    "workshop",
    "roundtable",
    "input",
    "feedback",
    "coordinate",
];

/// Keywords indicating an analytical communication style.
const ANALYTICAL_KEYWORDS: &[&str] = &[
    "data",
    "analysis",
    "metric",
    "trend",
    "forecast",
    "model",
    "statistical",
    "evidence",
    "empirical",
    "quantitative",
    "regression",
    "benchmark",
    "kpi",
    "dashboard",
    "insight",
    "analytics",
];

/// Keywords indicating a data-driven communication style.
const DATA_DRIVEN_KEYWORDS: &[&str] = &[
    "evidence-based",
    "data-driven",
    "measure",
    "track",
    "monitor",
    "report",
    "analytics",
    "dashboard",
    "roi",
    "conversion",
    "funnel",
    "cohort",
    "a/b test",
    "experiment",
];

/// Keywords indicating positive sentiment.
const POSITIVE_KEYWORDS: &[&str] = &[
    "growth",
    "expansion",
    "opportunity",
    "success",
    "innovation",
    "partnership",
    "launch",
    "achievement",
    "breakthrough",
    "record",
    "award",
    "recognition",
    "milestone",
    "momentum",
    "optimistic",
];

/// Keywords indicating negative sentiment.
const NEGATIVE_KEYWORDS: &[&str] = &[
    "crisis",
    "failure",
    "loss",
    "decline",
    "risk",
    "threat",
    "disruption",
    "shortage",
    "delay",
    "cancellation",
    "lawsuit",
    "penalty",
    "violation",
    "warning",
    "downgrade",
    "layoff",
    "bankruptcy",
    "recall",
    "scandal",
    "investigation",
];

/// Keywords indicating urgency / high pain index.
const URGENCY_KEYWORDS: &[&str] = &[
    "urgent",
    "immediate",
    "critical",
    "deadline",
    "asap",
    "emergency",
    "crisis",
    "priority",
    "time-sensitive",
    "rush",
    "expedite",
    "accelerate",
    "fast-track",
];

/// Keywords indicating openness to change.
const CHANGE_OPEN_KEYWORDS: &[&str] = &[
    "transformation",
    "modernization",
    "digitization",
    "optimization",
    "restructuring",
    "pivot",
    "agile",
    "innovation",
    "disruption",
    "new approach",
    "rethink",
    "reimagine",
];

/// Topics to avoid in engagement (sensitive areas).
const SENSITIVE_TOPICS: &[&str] = &[
    "layoff",
    "downsizing",
    "lawsuit",
    "litigation",
    "scandal",
    "bankruptcy",
    "acquisition rumor",
    "merger rumor",
];

// ═══════════════════════════════════════════════════════════════════════════════
// Compute Engine
// ═══════════════════════════════════════════════════════════════════════════════

/// The psychological profiling compute engine.
pub struct PsychComputeEngine {
    pool: PgPool,
}

impl PsychComputeEngine {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Run the full psych profiling pipeline for a person.
    /// Returns the computed profile result and persists everything to the DB.
    pub async fn compute_and_persist(
        &self,
        snapshot: &RawProfileSnapshot,
    ) -> Result<PsychProfileResult, String> {
        let corpus: Vec<&str> = snapshot
            .observations
            .iter()
            .map(|o| o.text.as_str())
            .collect();

        // Stage 1: Decision style
        let decision_style = compute_decision_style(&corpus);

        // Stage 2: Change appetite
        let change_appetite = compute_change_appetite(
            &corpus,
            snapshot.career_length_years,
            snapshot.job_change_count,
        );

        // Stage 3: Pain index
        let pain_index = compute_pain_index(&corpus);

        // Stage 4: Risk tolerance
        let risk_tolerance = compute_risk_tolerance(&corpus, &snapshot.role_family);

        // Stage 5: Preferred proof
        let preferred_proof = compute_preferred_proof(&decision_style, &snapshot.role_family);

        // Stage 6: Evidence sources
        let evidence_sources: Vec<String> = snapshot
            .observations
            .iter()
            .filter_map(|o| o.source_domain.clone())
            .collect();

        // Stage 7: Enrichment quality
        let enrichment_quality =
            compute_enrichment_quality(snapshot.observations.len(), &evidence_sources);

        // Stage 8: Behavioral patterns
        let behavioral_patterns = detect_patterns(snapshot);

        // Stage 9: Engagement strategy
        let engagement = Some(generate_engagement(
            &decision_style,
            &change_appetite,
            &snapshot.person_name,
            &snapshot.role_family,
        ));

        // Persist via psych_store
        let db_decision = super::psych_store::decision_style_to_db(&decision_style);
        let db_appetite = super::psych_store::change_appetite_to_db(&change_appetite);
        let preferred_proof_strings: Vec<String> =
            preferred_proof.iter().map(|s| s.to_string()).collect();

        let metadata = json!({
            "person_name": snapshot.person_name,
            "organization": snapshot.organization,
            "current_title": snapshot.current_title,
            "corpus_size": corpus.len(),
            "observation_count": snapshot.observations.len(),
            "public_statements": snapshot.public_statements_count,
        });

        let profile_id = super::psych_store::upsert_psychological_profile(
            &self.pool,
            super::psych_store::PsychProfileUpsertRequest {
                person_id: &snapshot.person_id,
                decision_style: db_decision,
                change_appetite: db_appetite,
                pain_index,
                risk_tolerance,
                preferred_proof: &preferred_proof_strings,
                enrichment_quality,
                evidence_sources: &evidence_sources,
                metadata: &metadata,
            },
        )
        .await
        .map_err(|e| format!("Failed to persist psych profile: {}", e))?;

        // Persist behavioral patterns
        for pattern in &behavioral_patterns {
            if let Err(e) = super::psych_store::record_behavioral_pattern(
                &self.pool,
                &snapshot.person_id,
                &pattern.event_type,
                &pattern.title,
                &pattern.description,
                pattern.confidence,
                &pattern.evidence_urls,
            )
            .await
            {
                warn!("Failed to persist behavioral pattern: {}", e);
            }
        }

        // Persist engagement profile
        if let Some(ref eng) = engagement {
            let proof_pack = json!({
                "decision_style": decision_style,
                "change_appetite": change_appetite,
                "role_family": snapshot.role_family,
            });
            if let Err(e) = super::psych_store::upsert_engagement_profile(
                &self.pool,
                super::psych_store::EngagementProfileUpsertRequest {
                    person_id: &snapshot.person_id,
                    talking_points: &eng.talking_points,
                    opening_topics: &eng.opening_topics,
                    avoid_topics: &eng.avoid_topics,
                    best_channel: &eng.best_channel,
                    best_timing: eng.best_timing.as_deref(),
                    proof_pack: &proof_pack,
                },
            )
            .await
            {
                warn!("Failed to persist engagement profile: {}", e);
            }
        }

        info!(
            person_id = %snapshot.person_id,
            decision_style = %decision_style,
            pain_index = %pain_index,
            enrichment = %enrichment_quality,
            "psych_compute: profile computed and persisted"
        );

        Ok(PsychProfileResult {
            profile_id,
            person_id: snapshot.person_id.clone(),
            decision_style,
            change_appetite,
            pain_index,
            risk_tolerance,
            preferred_proof: preferred_proof_strings,
            enrichment_quality,
            evidence_sources,
            behavioral_patterns,
            engagement,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Analysis functions
// ═══════════════════════════════════════════════════════════════════════════════

/// Compute decision style from communication patterns in text corpus.
fn compute_decision_style(corpus: &[&str]) -> String {
    let combined = corpus.join(" ").to_lowercase();
    if combined.is_empty() {
        return "Unknown".to_string();
    }

    let authoritative = count_keywords(&combined, AUTHORITATIVE_KEYWORDS);
    let collaborative = count_keywords(&combined, COLLABORATIVE_KEYWORDS);
    let analytical = count_keywords(&combined, ANALYTICAL_KEYWORDS);
    let data_driven = count_keywords(&combined, DATA_DRIVEN_KEYWORDS);

    // Normalize by corpus size for fair comparison
    let word_count = combined.split_whitespace().count().max(1) as f64;
    let a = authoritative as f64 / word_count;
    let c = collaborative as f64 / word_count;
    let an = analytical as f64 / word_count;
    let d = data_driven as f64 / word_count;

    let max_score = a.max(c).max(an).max(d);
    if max_score == 0.0 {
        return "Collaborative".to_string(); // default
    }

    if (a - max_score).abs() < 0.0001 {
        "Authoritative".to_string()
    } else if (an - max_score).abs() < 0.0001 && an > d {
        "Analytical".to_string()
    } else if (d - max_score).abs() < 0.0001 {
        "DataDriven".to_string()
    } else {
        "Collaborative".to_string()
    }
}

/// Compute change appetite from career signals and language.
fn compute_change_appetite(
    corpus: &[&str],
    career_years: Option<f64>,
    job_changes: Option<u32>,
) -> String {
    let combined = corpus.join(" ").to_lowercase();
    let change_keywords = count_keywords(&combined, CHANGE_OPEN_KEYWORDS);

    // Career mobility signals
    let career_signal = match (career_years, job_changes) {
        (Some(years), Some(changes)) if years > 0.0 => {
            let change_rate = changes as f64 / years;
            if change_rate > 0.5 {
                0.8 // high mobility
            } else if change_rate > 0.25 {
                0.5
            } else {
                0.2
            }
        }
        _ => 0.3, // neutral when unknown
    };

    let keyword_signal = if corpus.is_empty() {
        0.3
    } else {
        let word_count = combined.split_whitespace().count().max(1) as f64;
        (change_keywords as f64 / word_count * 10.0).min(1.0)
    };

    let score = career_signal * 0.5 + keyword_signal * 0.5;

    if score > 0.65 {
        "High".to_string()
    } else if score > 0.35 {
        "Moderate".to_string()
    } else if score > 0.15 {
        "Low".to_string()
    } else {
        "Resistant".to_string()
    }
}

/// Compute pain index from urgency signals and negative sentiment.
fn compute_pain_index(corpus: &[&str]) -> f64 {
    let combined = corpus.join(" ").to_lowercase();
    if combined.is_empty() {
        return 0.3;
    }

    let urgency = count_keywords(&combined, URGENCY_KEYWORDS);
    let negative = count_keywords(&combined, NEGATIVE_KEYWORDS);
    let positive = count_keywords(&combined, POSITIVE_KEYWORDS);
    let word_count = combined.split_whitespace().count().max(1) as f64;

    let urgency_score = (urgency as f64 / word_count * 8.0).min(1.0);
    let sentiment_ratio = if positive + negative > 0 {
        negative as f64 / (positive + negative) as f64
    } else {
        0.5
    };

    (urgency_score * 0.4 + sentiment_ratio * 0.6).clamp(0.0, 1.0)
}

/// Compute risk tolerance from behavioral evidence.
fn compute_risk_tolerance(corpus: &[&str], role_family: &str) -> f64 {
    let combined = corpus.join(" ").to_lowercase();
    if combined.is_empty() {
        return role_default_risk(role_family);
    }

    let risk_keywords = [
        "risk",
        "uncertainty",
        "volatile",
        "speculative",
        "experimental",
        "pilot",
        "venture",
        "startup",
        "disruptive",
        "aggressive",
    ];
    let caution_keywords = [
        "safe",
        "stable",
        "proven",
        "reliable",
        "conservative",
        "mitigate",
        "hedge",
        "compliance",
        "regulation",
        "audit",
    ];

    let risk_count = count_keywords(&combined, &risk_keywords);
    let caution_count = count_keywords(&combined, &caution_keywords);

    let risk_signal = if risk_count + caution_count > 0 {
        risk_count as f64 / (risk_count + caution_count) as f64
    } else {
        0.5
    };

    // Blend with role default
    let role_default = role_default_risk(role_family);
    (risk_signal * 0.6 + role_default * 0.4).clamp(0.0, 1.0)
}

fn role_default_risk(role_family: &str) -> f64 {
    match role_family.to_lowercase().as_str() {
        "executive" => 0.7,
        "procurement" => 0.4,
        "supplierquality" => 0.3,
        "engineering" => 0.5,
        "security" => 0.35,
        "finance" => 0.3,
        "operations" => 0.45,
        _ => 0.5,
    }
}

/// Determine preferred proof types from decision style and role.
fn compute_preferred_proof(decision_style: &str, role_family: &str) -> Vec<&'static str> {
    let mut proofs = Vec::new();

    // Decision-style-based proofs
    match decision_style {
        "Analytical" | "DataDriven" => {
            proofs.push("ROI Analysis");
            proofs.push("Case Study");
            proofs.push("Benchmark Data");
        }
        "Authoritative" => {
            proofs.push("Expert Endorsement");
            proofs.push("Industry Recognition");
        }
        "Collaborative" => {
            proofs.push("Peer Reference");
            proofs.push("Partnership Case Study");
        }
        _ => {
            proofs.push("Case Study");
            proofs.push("ROI Analysis");
        }
    }

    // Role-specific proofs
    match role_family.to_lowercase().as_str() {
        "procurement" | "supplierquality" => {
            proofs.push("Supplier Qualification Data");
            proofs.push("Quality Certifications");
        }
        "engineering" => {
            proofs.push("Technical Whitepaper");
            proofs.push("Performance Benchmark");
        }
        "executive" => {
            proofs.push("Strategic Impact Analysis");
            proofs.push("Market Position Data");
        }
        "finance" => {
            proofs.push("Cost-Benefit Analysis");
            proofs.push("Financial Projections");
        }
        _ => {}
    }

    proofs
}

/// Compute enrichment quality based on evidence diversity.
fn compute_enrichment_quality(observation_count: usize, sources: &[String]) -> f64 {
    if observation_count == 0 {
        return 0.0;
    }

    // Count unique source domains
    let mut unique_domains: HashMap<&str, usize> = HashMap::new();
    for source in sources {
        *unique_domains.entry(source.as_str()).or_default() += 1;
    }

    let diversity = unique_domains.len() as f64 / sources.len().max(1) as f64;
    let volume_score = (observation_count as f64 / 20.0).min(1.0);

    (diversity * 0.6 + volume_score * 0.4).clamp(0.0, 1.0)
}

/// Detect behavioral patterns from observations.
fn detect_patterns(snapshot: &RawProfileSnapshot) -> Vec<BehavioralPatternResult> {
    let mut patterns = Vec::new();
    let combined: String = snapshot
        .observations
        .iter()
        .map(|o| o.text.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let lower = combined.to_lowercase();

    // Check for sentiment shifts
    let negative_count = count_keywords(&lower, NEGATIVE_KEYWORDS);
    let positive_count = count_keywords(&lower, POSITIVE_KEYWORDS);
    let total_sentiment = negative_count + positive_count;

    if total_sentiment > 3 && negative_count > positive_count * 2 {
        patterns.push(BehavioralPatternResult {
            event_type: "sentiment_shift".to_string(),
            title: "Negative Sentiment Detected".to_string(),
            description: format!(
                "{} negative vs {} positive signals in recent communications for {}.",
                negative_count, positive_count, snapshot.person_name
            ),
            confidence: (negative_count as f64 / total_sentiment as f64).min(0.95),
            evidence_urls: snapshot
                .observations
                .iter()
                .filter(|o| {
                    let t = o.text.to_lowercase();
                    NEGATIVE_KEYWORDS.iter().any(|kw| t.contains(kw))
                })
                .filter_map(|o| o.source_url.clone())
                .take(5)
                .collect(),
        });
    }

    // Check for urgency signals (high pain index precursor)
    let urgency_count = count_keywords(&lower, URGENCY_KEYWORDS);
    if urgency_count >= 3 {
        patterns.push(BehavioralPatternResult {
            event_type: "priority_change".to_string(),
            title: "Urgency Signal Detected".to_string(),
            description: format!(
                "{} urgency-related keywords detected in recent communications for {}.",
                urgency_count, snapshot.person_name
            ),
            confidence: (urgency_count as f64 / 10.0).min(0.9),
            evidence_urls: snapshot
                .observations
                .iter()
                .filter_map(|o| o.source_url.clone())
                .take(3)
                .collect(),
        });
    }

    // Check for significant public engagement
    if snapshot.public_statements_count >= 5 {
        patterns.push(BehavioralPatternResult {
            event_type: "engagement_surge".to_string(),
            title: "Elevated Public Engagement".to_string(),
            description: format!(
                "{} has made {} public statements recently, indicating elevated external engagement.",
                snapshot.person_name, snapshot.public_statements_count
            ),
            confidence: (snapshot.public_statements_count as f64 / 10.0).min(0.85),
            evidence_urls: snapshot
                .observations
                .iter()
                .take(3)
                .filter_map(|o| o.source_url.clone())
                .collect(),
        });
    }

    patterns
}

/// Generate engagement strategy.
fn generate_engagement(
    decision_style: &str,
    change_appetite: &str,
    person_name: &str,
    role_family: &str,
) -> EngagementResult {
    let mut talking_points = Vec::new();
    let mut opening_topics = Vec::new();
    let avoid_topics: Vec<String> = SENSITIVE_TOPICS.iter().map(|s| s.to_string()).collect();

    // Talking points by decision style
    match decision_style {
        "Analytical" | "DataDriven" => {
            talking_points.push(format!(
                "Present quantified value proposition with specific metrics for {}",
                person_name
            ));
            talking_points.push("Include comparative benchmarks and case study data".to_string());
        }
        "Authoritative" => {
            talking_points.push(format!(
                "Present top-down strategic alignment for {}",
                person_name
            ));
            talking_points.push("Emphasize industry leadership and market position".to_string());
        }
        "Collaborative" => {
            talking_points.push(format!(
                "Frame discussion as joint exploration of mutual opportunities with {}",
                person_name
            ));
            talking_points
                .push("Highlight partnership models and co-innovation potential".to_string());
        }
        _ => {
            talking_points.push(format!(
                "Present clear value proposition tailored to {}'s role",
                person_name
            ));
            talking_points.push("Include relevant industry context and evidence".to_string());
        }
    }

    // Role-specific talking points
    match role_family.to_lowercase().as_str() {
        "procurement" => {
            talking_points.push(
                "Focus on supply chain resilience, cost optimization, and supplier diversity"
                    .to_string(),
            );
            opening_topics.push("Supply chain trends and challenges".to_string());
            opening_topics.push("Sourcing strategy alignment".to_string());
        }
        "supplierquality" => {
            talking_points.push(
                "Focus on quality metrics, compliance, and certification requirements".to_string(),
            );
            opening_topics.push("Quality standards and certifications".to_string());
            opening_topics.push("Continuous improvement initiatives".to_string());
        }
        "executive" => {
            talking_points.push(
                "Focus on strategic impact, market positioning, and competitive advantage"
                    .to_string(),
            );
            opening_topics.push("Industry trends and market dynamics".to_string());
            opening_topics.push("Strategic growth opportunities".to_string());
        }
        _ => {
            opening_topics.push(format!(
                "Industry developments relevant to {}'s role in {}",
                person_name, role_family
            ));
            opening_topics.push("Mutual business opportunities".to_string());
        }
    }

    // Channel by change appetite
    let best_channel = match change_appetite {
        "High" => "video_call",
        "Moderate" => "email",
        "Low" | "Resistant" => "introduction",
        _ => "email",
    };

    let best_timing = match change_appetite {
        "High" => Some("morning".to_string()),
        "Moderate" => Some("midweek".to_string()),
        _ => None,
    };

    EngagementResult {
        talking_points,
        opening_topics,
        avoid_topics,
        best_channel: best_channel.to_string(),
        best_timing,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════════

fn count_keywords(text: &str, keywords: &[&str]) -> usize {
    keywords.iter().filter(|kw| text.contains(**kw)).count()
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_decision_style_analytical() {
        let corpus = vec![
            "The data shows a clear trend toward increased procurement volume.",
            "Our analysis indicates that benchmarks should be revised.",
            "Statistical evidence supports the proposed forecast model.",
            "We need to measure the KPIs before making any decision.",
        ];
        let style = compute_decision_style(&corpus);
        assert!(
            style == "Analytical" || style == "DataDriven",
            "Expected Analytical or DataDriven, got {}",
            style
        );
    }

    #[test]
    fn test_compute_decision_style_authoritative() {
        let corpus = vec![
            "We must comply with the new directive immediately.",
            "I demand that all teams enforce the mandate.",
            "This is a direct order from the board.",
        ];
        let style = compute_decision_style(&corpus);
        assert_eq!(style, "Authoritative");
    }

    #[test]
    fn test_compute_decision_style_collaborative() {
        let corpus = vec![
            "Let's work together to find alignment.",
            "I'd like your input and feedback on this.",
            "We should coordinate with all stakeholders.",
        ];
        let style = compute_decision_style(&corpus);
        assert_eq!(style, "Collaborative");
    }

    #[test]
    fn test_compute_decision_style_empty_corpus() {
        let style = compute_decision_style(&[]);
        assert_eq!(style, "Unknown");
    }

    #[test]
    fn test_compute_pain_index_high() {
        let corpus = vec![
            "This is an urgent crisis that needs immediate attention.",
            "The deadline is critical and we must act asap.",
            "We are facing an emergency situation with the supplier.",
            "Another threat has emerged in the supply chain.",
        ];
        let pain = compute_pain_index(&corpus);
        assert!(pain > 0.5, "Expected high pain index, got {}", pain);
    }

    #[test]
    fn test_compute_pain_index_low() {
        let corpus = vec![
            "We are seeing growth and expansion opportunities.",
            "The partnership has been a breakthrough success.",
            "Our innovation has led to record momentum.",
        ];
        let pain = compute_pain_index(&corpus);
        assert!(pain < 0.5, "Expected low pain index, got {}", pain);
    }

    #[test]
    fn test_compute_pain_index_empty() {
        let pain = compute_pain_index(&[]);
        assert!((pain - 0.3).abs() < 0.01);
    }

    #[test]
    fn test_compute_change_appetite_high() {
        let corpus = vec![
            "We are undergoing a digital transformation.",
            "The company needs to rethink its approach to modernization.",
            "Innovation is key to our agile pivot strategy.",
        ];
        let appetite = compute_change_appetite(
            &corpus,
            Some(8.0), // 8 years
            Some(5),   // 5 job changes = high mobility
        );
        assert_eq!(appetite, "High");
    }

    #[test]
    fn test_compute_change_appetite_resistant() {
        let corpus = vec!["We maintain stable operations."];
        let appetite = compute_change_appetite(
            &corpus,
            Some(20.0), // 20 years
            Some(1),    // 1 change = very stable
        );
        assert!(
            appetite == "Resistant" || appetite == "Low",
            "Expected Resistant or Low, got {}",
            appetite
        );
    }

    #[test]
    fn test_compute_risk_tolerance() {
        let aggressive = compute_risk_tolerance(
            &["We must take aggressive risks to disrupt the market."],
            "executive",
        );
        let conservative = compute_risk_tolerance(
            &["We must comply with all regulations and audits for safety."],
            "procurement",
        );
        assert!(aggressive > conservative);
    }

    #[test]
    fn test_compute_preferred_proof() {
        let proofs = compute_preferred_proof("Analytical", "procurement");
        assert!(proofs.contains(&"ROI Analysis"));
        assert!(proofs.contains(&"Supplier Qualification Data"));
    }

    #[test]
    fn test_compute_enrichment_quality() {
        let sources = vec![
            "linkedin.com".to_string(),
            "linkedin.com".to_string(),
            "twitter.com".to_string(),
            "news.example.com".to_string(),
        ];
        let quality = compute_enrichment_quality(4, &sources);
        // 3 unique domains out of 4 sources = 0.75 diversity
        // volume score = 4/20 = 0.2
        // total = 0.75*0.6 + 0.2*0.4 = 0.53
        assert!((quality - 0.53).abs() < 0.1);
    }

    #[test]
    fn test_detect_patterns() {
        let snapshot = RawProfileSnapshot {
            person_id: "test-1".to_string(),
            person_name: "Test Person".to_string(),
            current_title: "VP Procurement".to_string(),
            role_family: "Procurement".to_string(),
            organization: "Test Corp".to_string(),
            career_length_years: None,
            job_change_count: None,
            public_statements_count: 7,
            observations: vec![
                PsychObservation {
                    text: "This is an urgent crisis that demands immediate action.".to_string(),
                    source_url: Some("https://example.com/1".to_string()),
                    source_domain: Some("example.com".to_string()),
                    observed_at: Utc::now(),
                    sentiment_score: None,
                },
                PsychObservation {
                    text: "The supplier has failed and we face a major threat.".to_string(),
                    source_url: Some("https://example.com/2".to_string()),
                    source_domain: Some("example.com".to_string()),
                    observed_at: Utc::now(),
                    sentiment_score: None,
                },
                PsychObservation {
                    text: "Another crisis in the logistics pipeline.".to_string(),
                    source_url: Some("https://news.com/3".to_string()),
                    source_domain: Some("news.com".to_string()),
                    observed_at: Utc::now(),
                    sentiment_score: None,
                },
                PsychObservation {
                    text: "The delay is causing significant disruption.".to_string(),
                    source_url: Some("https://news.com/4".to_string()),
                    source_domain: Some("news.com".to_string()),
                    observed_at: Utc::now(),
                    sentiment_score: None,
                },
            ],
        };
        let patterns = detect_patterns(&snapshot);
        // Should detect negative sentiment and urgency
        assert!(!patterns.is_empty(), "Expected patterns to be detected");
        assert!(
            patterns.iter().any(|p| p.event_type == "sentiment_shift"),
            "Expected sentiment_shift pattern"
        );
    }

    #[test]
    fn test_generate_engagement() {
        let eng = generate_engagement("Analytical", "High", "Jane Smith", "procurement");
        assert!(!eng.talking_points.is_empty());
        assert!(eng
            .talking_points
            .iter()
            .any(|tp| tp.contains("Jane Smith")));
        assert_eq!(eng.best_channel, "video_call");
        assert!(eng.best_timing.is_some());
    }

    #[test]
    fn test_count_keywords() {
        // The compute pipeline lowercases text before keyword matching, so mirror
        // that here. Matches: "data", "kpi" (inside "kpis"), and "forecast".
        let text =
            "We need to analyze the data and measure the KPIs for our forecast.".to_lowercase();
        assert_eq!(count_keywords(&text, ANALYTICAL_KEYWORDS), 3);
    }

    /// Cross-module contract: every value the compute functions can emit must flow
    /// through `psych_store`'s converters into a literal that satisfies the DB CHECK
    /// constraints. This guards against the PascalCase/snake_case mismatch that
    /// would otherwise break DB writes (e.g. "DataDriven" must become "data_driven").
    #[test]
    fn test_compute_outputs_are_db_valid() {
        let decision_corpora: Vec<Vec<&str>> = vec![
            // authoritative
            vec!["We must comply with the directive and enforce the mandate."],
            // collaborative
            vec!["Let's collaborate and align with stakeholders for feedback."],
            // analytical / data-driven
            vec![
                "The data and analytics show a clear trend in the KPI benchmark.",
                "We measure ROI with a dashboard and run A/B test experiments.",
            ],
            // empty corpus -> "Unknown"
            vec![],
        ];
        for corpus in &decision_corpora {
            let style = compute_decision_style(corpus);
            let db = crate::psych_store::decision_style_to_db(&style);
            assert!(
                crate::psych_store::is_valid_decision_style_db(db),
                "compute_decision_style produced '{style}' -> '{db}' (not a valid DB literal)"
            );
        }

        let change_cases: Vec<(Vec<&str>, Option<f64>, Option<u32>)> = vec![
            (
                vec!["digital transformation modernization agile pivot"],
                Some(8.0),
                Some(5),
            ),
            (vec!["stable operations maintained"], Some(20.0), Some(1)),
            (vec![], None, None),
        ];
        for (corpus, years, changes) in &change_cases {
            let appetite = compute_change_appetite(corpus, *years, *changes);
            let db = crate::psych_store::change_appetite_to_db(&appetite);
            assert!(
                crate::psych_store::is_valid_change_appetite_db(db),
                "compute_change_appetite produced '{appetite}' -> '{db}' (not a valid DB literal)"
            );
        }
    }
}
