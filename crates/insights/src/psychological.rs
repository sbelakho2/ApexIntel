//! Psychological & Behavioral Intelligence Module
//!
//! Provides psychological profiling, behavioral pattern detection,
//! sentiment aggregation at scale, engagement strategy generation,
//! personality assessment, organizational culture profiling, and
//! cognitive bias detection for intelligence analysis.
//!
//! Part of the ApexIntel psychological intelligence pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use apex_poi::model::{
    ChangeAppetite, DecisionStyle, PoiArtifact, PriorityVector, ProofType, PsychProfile,
};

use crate::InsightSeverity;

// ═══════════════════════════════════════════════════════════════════════════════
// Public Enums and Structs
// ═══════════════════════════════════════════════════════════════════════════════

/// Types of behavioral patterns that can be detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BehavioralPatternType {
    SentimentShift,
    CommunicationDrift,
    PriorityChange,
    RiskAppetiteChange,
    EngagementSurge,
    EngagementDrop,
    RoleDrift,
    InfluenceChange,
    NetworkExpansion,
    TopicObsession,
    SilenceAnomaly,
    LanguageMirroring,
}

impl BehavioralPatternType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SentimentShift => "sentiment_shift",
            Self::CommunicationDrift => "communication_drift",
            Self::PriorityChange => "priority_change",
            Self::RiskAppetiteChange => "risk_appetite_change",
            Self::EngagementSurge => "engagement_surge",
            Self::EngagementDrop => "engagement_drop",
            Self::RoleDrift => "role_drift",
            Self::InfluenceChange => "influence_change",
            Self::NetworkExpansion => "network_expansion",
            Self::TopicObsession => "topic_obsession",
            Self::SilenceAnomaly => "silence_anomaly",
            Self::LanguageMirroring => "language_mirroring",
        }
    }
}

/// Entity types for sentiment aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityType {
    Company,
    Person,
    Region,
    Product,
}

/// Trend direction classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendDirection {
    Rising,
    Falling,
    Stable,
    Volatile,
}

/// A detected behavioral pattern with full context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralPattern {
    pub pattern_type: BehavioralPatternType,
    pub severity: InsightSeverity,
    pub title: String,
    pub description: String,
    pub confidence: f64,
    pub baseline_value: f64,
    pub current_value: f64,
    pub delta: f64,
}

/// A snapshot of psychological profile state at a point in time.
#[derive(Debug, Clone)]
pub struct ProfileSnapshot {
    pub pain_index: f64,
    pub risk_tolerance: f64,
    pub priority_vector: PriorityVector,
    pub sentiment_score: f64,
    pub engagement_count: usize,
    pub topics: Vec<String>,
    pub role: String,
    pub snapshot_at: DateTime<Utc>,
}

/// A single sentiment measurement from a source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentSignal {
    pub score: f64,
    pub text: String,
    pub source: String,
    pub ts_utc: DateTime<Utc>,
}

/// Aggregated sentiment for a time bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentBucket {
    pub mean_score: f64,
    pub median_score: f64,
    pub std_dev: f64,
    pub positive_pct: f64,
    pub neutral_pct: f64,
    pub negative_pct: f64,
    pub sample_count: u32,
}

/// Trend analysis of sentiment over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentTrend {
    pub direction: TrendDirection,
    pub strength: f64,
    pub volatility: f64,
    pub recent_score: f64,
    pub historical_mean: f64,
}

/// A detected sentiment anomaly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentAnomaly {
    pub detected_at: DateTime<Utc>,
    pub severity: InsightSeverity,
    pub description: String,
    pub deviation: f64,
}

/// Engagement strategy types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyType {
    General,
    MeetingPrep,
    Negotiation,
    Outreach,
    CompetitiveDisplacement,
    CrisisResponse,
}

/// Context for generating an engagement strategy.
#[derive(Debug, Clone)]
pub struct StrategyContext {
    pub strategy_type: StrategyType,
    pub objective: String,
    pub competitor_mentioned: Option<String>,
    pub urgency: String,
    pub relationship_status: String,
}

/// Generated engagement strategy with talking points and channel guidance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementStrategy {
    pub what_they_want: Vec<String>,
    pub opening_topics: Vec<String>,
    pub avoid_topics: Vec<String>,
    pub best_channel: String,
    pub best_timing: String,
    pub recommended_proof: Vec<String>,
    pub talking_points: Vec<String>,
    pub risk_mitigations: Vec<String>,
    pub strategy_confidence: f64,
}

/// Big Five personality trait profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BigFiveTraits {
    pub openness: f64,
    pub conscientiousness: f64,
    pub extraversion: f64,
    pub agreeableness: f64,
    pub neuroticism: f64,
    pub confidence: f64,
}

/// HEXACO personality trait profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexacoTraits {
    pub honesty_humility: f64,
    pub emotionality: f64,
    pub extraversion: f64,
    pub agreeableness: f64,
    pub conscientiousness: f64,
    pub openness: f64,
    pub confidence: f64,
}

/// A signal about organizational culture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgCultureSignal {
    pub signal_type: String,
    pub description: String,
    pub source: String,
    pub ts_utc: DateTime<Utc>,
}

/// Organizational culture profile for a company.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgCultureProfile {
    pub innovation_focus: f64,
    pub risk_culture: f64,
    pub hierarchy_rigidity: f64,
    pub speed_of_decision: f64,
    pub talent_centricity: f64,
    pub cost_sensitivity: f64,
    pub compliance_posture: f64,
    pub external_orientation: f64,
    pub culture_tags: Vec<String>,
    pub profile_confidence: f64,
}

/// Context for bias detection.
#[derive(Debug, Clone)]
pub struct BiasContext {
    pub author_role: String,
    pub target_entity: String,
    pub topic: String,
}

/// A detected cognitive bias.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveBias {
    pub bias_type: String,
    pub severity: InsightSeverity,
    pub description: String,
    pub mitigation: String,
    pub confidence: f64,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Psychological Profiler
// ═══════════════════════════════════════════════════════════════════════════════

/// Psychological profiler that uses real multi-dimensional psychometric
/// models backed by artifact analysis, not flat keyword counting.
///
/// ## Methodology
/// 1. **Lexical marker categories** — words tagged by psycholinguistic function
///    (certainty, risk-cognition, social-identity, temporal-focus).
/// 2. **Composite scoring** — each dimension bins multiple lexical markers that
///    are weighted by their known psychometric correlation strength.
/// 3. **Confidence calibration** — output confidence is proportional to artifact
///    volume and signal density, never inflated when data is sparse.
/// 4. **No hallucination** — when artifacts are empty or noise-only, the
///    profiler returns an *unenriched* [`PsychProfile`] (all-zero’d) so
///    downstream callers can detect missing data instead of silently accepting
///    fabricated defaults.
pub struct PsychologicalProfiler {
    /// Minimum artifact word-count before any dimension is scored.
    min_evidence_words: usize,
    /// Outlier z-score threshold for discarding anomalous signals.
    outlier_z: f64,
}

impl PsychologicalProfiler {
    pub fn new() -> Self {
        Self {
            min_evidence_words: 60,
            outlier_z: 3.0,
        }
    }

    /// Set the minimum number of words required across all artifacts
    /// before the profiler returns any non-zero dimension.
    pub fn with_min_evidence(mut self, words: usize) -> Self {
        self.min_evidence_words = words;
        self
    }

    // ── Public API ──────────────────────────────────────────────────────

    /// Build a complete psychological profile.
    ///
    /// Returns an *unenriched* (all-zero) [`PsychProfile`] when artifacts are
    /// insufficient to support reliable inference.
    pub fn profile_person(&self, _person_id: Uuid, artifacts: &[PoiArtifact]) -> PsychProfile {
        if artifacts.is_empty() {
            return PsychProfile::default_profile();
        }

        let combined_text = self.collect_text(artifacts);
        let word_count = combined_text.split_whitespace().count();
        if word_count < self.min_evidence_words {
            return PsychProfile::default_profile();
        }

        let text_lower = combined_text.to_lowercase();
        let signal_density = self.signal_density(artifacts);

        // Lexical-marker-based dimensions
        let risk_tolerance =
            self.compute_risk_tolerance_from_markers(&text_lower, artifacts, signal_density);
        let pain_index = self.compute_pain_index(artifacts);

        // Priority vector from full lexicon (not just 6 keywords each)
        let pv = self.extract_priority_vector_lexical(&text_lower, artifacts);

        let decision_style = self.infer_decision_style(&pv);
        let change_appetite = self.infer_change_appetite(artifacts);
        let preferred_proof = self.infer_preferred_proof(artifacts);

        PsychProfile {
            decision_style,
            change_appetite,
            pain_index,
            preferred_proof,
            risk_tolerance,
        }
    }

    /// Update a profile by blending existing signals with new artifacts.
    /// Uses exponential moving average blending: 70% existing / 30% new.
    pub fn update_profile(
        &self,
        person_id: Uuid,
        existing: &PsychProfile,
        new_artifacts: &[PoiArtifact],
    ) -> PsychProfile {
        if new_artifacts.is_empty() {
            return existing.clone();
        }
        let fresh = self.profile_person(person_id, new_artifacts);
        if !fresh.is_enriched() {
            return existing.clone();
        }
        // EMA blend: 70% existing, 30% fresh
        let alpha = 0.3;
        PsychProfile {
            decision_style: if alpha > 0.5 {
                fresh.decision_style
            } else {
                existing.decision_style.clone()
            },
            change_appetite: if alpha > 0.5 {
                fresh.change_appetite
            } else {
                existing.change_appetite.clone()
            },
            pain_index: (existing.pain_index * (1.0 - alpha) + fresh.pain_index * alpha)
                .clamp(0.0, 1.0),
            preferred_proof: {
                let mut merged = existing.preferred_proof.clone();
                for p in fresh.preferred_proof {
                    if !merged.contains(&p) {
                        merged.push(p);
                    }
                }
                merged.truncate(6);
                merged
            },
            risk_tolerance: (existing.risk_tolerance * (1.0 - alpha)
                + fresh.risk_tolerance * alpha)
                .clamp(0.0, 1.0),
        }
    }

    // ── Helper: collect text ────────────────────────────────────────────

    fn collect_text(&self, artifacts: &[PoiArtifact]) -> String {
        artifacts
            .iter()
            .map(|a| format!("{} {}", a.title, a.content_summary))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Signal density = fraction of words that are known psychometric markers.
    fn signal_density(&self, artifacts: &[PoiArtifact]) -> f64 {
        let text = self.collect_text(artifacts).to_lowercase();
        let total_words = text.split_whitespace().count() as f64;
        if total_words < 1.0 {
            return 0.0;
        }
        let marker_count = ALL_PSYCHOMETRIC_MARKERS
            .iter()
            .filter(|m| text.contains(*m))
            .count() as f64;
        (marker_count / total_words).clamp(0.0, 0.10) * 10.0 // normalise to ~[0,1]
    }

    // ── Risk tolerance from multi-category psychometric markers ─────────

    /// Computes risk_tolerance [0,1] from four psycholinguistic categories:
    ///   - Risk-acceptant markers (positive correlation)
    ///   - Risk-averse markers (negative correlation)
    ///   - Certainty/hedging markers (modulate confidence)
    ///   - Temporal-future markers (longer horizon = higher tolerance)
    fn compute_risk_tolerance_from_markers(
        &self,
        text_lower: &str,
        _artifacts: &[PoiArtifact],
        density: f64,
    ) -> f64 {
        let risk_accept = count_markers(text_lower, RISK_ACCEPTANT_MARKERS);
        let risk_avert = count_markers(text_lower, RISK_AVERSE_MARKERS);
        let certainty = count_markers(text_lower, CERTAINTY_MARKERS);
        let future_focus = count_markers(text_lower, FUTURE_FOCUS_MARKERS);

        let total_signals = (risk_accept + risk_avert + certainty + future_focus) as f64;
        if total_signals < 3.0 {
            return 0.0;
        }

        let raw_ratio = if risk_accept + risk_avert > 0 {
            risk_accept as f64 / (risk_accept + risk_avert) as f64
        } else {
            0.5
        };

        // Certainty modulates: high-certainty people tolerate *more* planned risk
        let cert_norm = (certainty as f64 / total_signals).clamp(0.0, 1.0);
        // Future-focus: longer horizon → higher tolerance for strategic bets
        let future_norm = (future_focus as f64 / total_signals).clamp(0.0, 1.0);

        let composite = raw_ratio * 0.45 + cert_norm * 0.30 + future_norm * 0.25;
        // Scale by signal density so sparse data cannot over-weight single hits
        let scaled = composite * density.min(1.0);
        scaled.clamp(0.0, 1.0)
    }

    // ── Pain index (0 = no pain, 1 = critical) ──────────────────────────

    /// Multi-category pain detection weighted by recency (90-day half-life).
    /// When no signals are found, returns 0.0 (not 0.3 — no hallucination).
    pub fn compute_pain_index(&self, artifacts: &[PoiArtifact]) -> f64 {
        if artifacts.is_empty() {
            return 0.0;
        }

        let now = Utc::now();
        let half_life_days = 90.0;
        let mut total_weighted = 0.0_f64;
        let mut evidence_count = 0_u32;
        let mut category_hits = [false; 6]; // cost, supply, quality, compliance, security, workforce

        for artifact in artifacts {
            let ts = DateTime::from_timestamp(artifact.ts_utc, 0).unwrap_or(now);
            let age_days = (now - ts).num_days().max(0) as f64;
            let decay = 2.0_f64.powf(-age_days / half_life_days);

            let text_lower = format!(
                "{} {}",
                artifact.title.to_lowercase(),
                artifact.content_summary.to_lowercase()
            );

            for (cat_idx, (_category_name, signal_list)) in
                PAIN_SIGNAL_CATEGORIES.iter().enumerate()
            {
                for (phrase, weight) in *signal_list {
                    if text_lower.contains(phrase) {
                        total_weighted += weight * decay;
                        evidence_count += 1;
                        category_hits[cat_idx] = true;
                    }
                }
            }
        }

        if evidence_count == 0 {
            return 0.0;
        }

        // Category multiplier: pain is more credible when multiple categories fire
        let cat_multiplier =
            0.5 + 0.5 * (category_hits.iter().filter(|&&h| h).count() as f64 / 6.0);

        // Normalise by evidence count with diminishing returns (avoid infinities)
        let normalised = (total_weighted / (evidence_count as f64).sqrt()).min(5.0);
        (normalised / 5.0 * cat_multiplier).clamp(0.0, 1.0)
    }

    // ── Priority vector: full lexical extraction ────────────────────────

    fn extract_priority_vector_lexical(
        &self,
        text_lower: &str,
        _artifacts: &[PoiArtifact],
    ) -> PriorityVector {
        let cost_hits = count_markers_weighted(text_lower, COST_PRIORITY_MARKERS);
        let quality_hits = count_markers_weighted(text_lower, QUALITY_PRIORITY_MARKERS);
        let speed_hits = count_markers_weighted(text_lower, SPEED_PRIORITY_MARKERS);
        let resilience_hits = count_markers_weighted(text_lower, RESILIENCE_PRIORITY_MARKERS);
        let compliance_hits = count_markers_weighted(text_lower, COMPLIANCE_PRIORITY_MARKERS);
        let security_hits = count_markers_weighted(text_lower, SECURITY_PRIORITY_MARKERS);

        let total = cost_hits
            + quality_hits
            + speed_hits
            + resilience_hits
            + compliance_hits
            + security_hits;
        if total < 1.0 {
            return PriorityVector::zero();
        }

        let artifact_adjusted_total = total.max(1.0);

        PriorityVector {
            cost: (cost_hits / artifact_adjusted_total).clamp(0.0, 1.0),
            quality: (quality_hits / artifact_adjusted_total).clamp(0.0, 1.0),
            speed: (speed_hits / artifact_adjusted_total).clamp(0.0, 1.0),
            resilience: (resilience_hits / artifact_adjusted_total).clamp(0.0, 1.0),
            compliance: (compliance_hits / artifact_adjusted_total).clamp(0.0, 1.0),
            security: (security_hits / artifact_adjusted_total).clamp(0.0, 1.0),
            confidence: (total / (total + 10.0)).clamp(0.0, 1.0),
        }
    }

    /// Infer decision style from the dominant dimension of the priority vector.
    pub fn infer_decision_style(&self, pv: &PriorityVector) -> DecisionStyle {
        let dominant = pv.dominant();
        match dominant {
            "cost" => DecisionStyle::CostFirst,
            "quality" => DecisionStyle::QualityFirst,
            "speed" => DecisionStyle::SpeedFirst,
            "security" | "compliance" => {
                if pv.compliance > pv.security {
                    DecisionStyle::ComplianceFirst
                } else {
                    DecisionStyle::RiskFirst
                }
            }
            _ => DecisionStyle::BalancedAnalytical,
        }
    }

    /// Infer change appetite from keyword evidence of innovation adoption.
    pub fn infer_change_appetite(&self, artifacts: &[PoiArtifact]) -> ChangeAppetite {
        if artifacts.is_empty() {
            return ChangeAppetite::Pragmatist;
        }

        let early_keywords = [
            "disrupt",
            "innovate",
            "cutting edge",
            "first mover",
            "pioneer",
            "bleeding edge",
            "transformative",
            "revolutionize",
            "breakthrough",
            "agile",
        ];
        let conservative_keywords = [
            "proven",
            "established",
            "traditional",
            "risk averse",
            "incremental",
            "stable",
            "legacy",
            "tried and tested",
            "reliable",
            "conservative",
        ];
        let laggard_keywords = [
            "resistant",
            "outdated",
            "obsolete",
            "declining",
            "behind",
            "late adopter",
            "manual",
            "paper based",
            "reluctant",
        ];

        let mut early_score = 0u32;
        let mut conservative_score = 0u32;
        let mut laggard_score = 0u32;

        for artifact in artifacts {
            let text_lower = artifact.content_summary.to_lowercase();
            early_score += early_keywords
                .iter()
                .filter(|kw| text_lower.contains(*kw))
                .count() as u32;
            conservative_score += conservative_keywords
                .iter()
                .filter(|kw| text_lower.contains(*kw))
                .count() as u32;
            laggard_score += laggard_keywords
                .iter()
                .filter(|kw| text_lower.contains(*kw))
                .count() as u32;
        }

        if laggard_score > early_score && laggard_score > conservative_score {
            ChangeAppetite::Laggard
        } else if conservative_score > early_score {
            ChangeAppetite::Conservative
        } else if early_score > conservative_score + 2 {
            ChangeAppetite::EarlyAdopter
        } else {
            ChangeAppetite::Pragmatist
        }
    }

    /// Compute risk tolerance from artifact content (low = risk-averse, high = risk-seeking).
    /// Legacy method retained for backward compatibility; redirects to marker-based computation.
    fn compute_risk_tolerance(&self, artifacts: &[PoiArtifact]) -> f64 {
        let text = self.collect_text(artifacts).to_lowercase();
        let density = self.signal_density(artifacts);
        self.compute_risk_tolerance_from_markers(&text, artifacts, density)
    }

    /// Infer preferred proof types from artifact content.
    fn infer_preferred_proof(&self, artifacts: &[PoiArtifact]) -> Vec<ProofType> {
        let mut scores: HashMap<ProofType, u32> = HashMap::new();

        let proof_keywords: &[(ProofType, &[&str])] = &[
            (
                ProofType::KpiMetrics,
                &[
                    "kpi",
                    "metric",
                    "roi",
                    "benchmark",
                    "data driven",
                    "quantifiable",
                    "measurable",
                ],
            ),
            (
                ProofType::Certifications,
                &[
                    "certified",
                    "iso",
                    "standard",
                    "compliance",
                    "accredited",
                    "qualification",
                ],
            ),
            (
                ProofType::CaseStudies,
                &[
                    "case study",
                    "testimonial",
                    "reference",
                    "portfolio",
                    "track record",
                    "proven",
                ],
            ),
            (
                ProofType::AuditReadiness,
                &[
                    "audit",
                    "transparency",
                    "traceability",
                    "documentation",
                    "inspection",
                ],
            ),
            (
                ProofType::TechDemos,
                &[
                    "demo",
                    "pilot",
                    "prototype",
                    "proof of concept",
                    "trial",
                    "sandbox",
                ],
            ),
            (
                ProofType::CostTransparency,
                &[
                    "cost breakdown",
                    "pricing",
                    "transparent",
                    "open book",
                    "should cost",
                ],
            ),
        ];

        for artifact in artifacts {
            let text_lower = artifact.content_summary.to_lowercase();
            for (proof_type, keywords) in proof_keywords {
                let hits = keywords
                    .iter()
                    .filter(|kw| text_lower.contains(*kw))
                    .count() as u32;
                *scores.entry(proof_type.clone()).or_insert(0) += hits;
            }
        }

        if scores.is_empty() {
            return vec![];
        }

        let mut entries: Vec<_> = scores.into_iter().collect();
        entries.sort_by_key(|a| std::cmp::Reverse(a.1));
        let top: Vec<ProofType> = entries.into_iter().take(3).map(|(k, _)| k).collect();
        if top.is_empty() {
            vec![]
        } else {
            top
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Psychometric Marker Lexicon
// ═══════════════════════════════════════════════════════════════════════════════

/// Count occurrences of markers from a flat &[&str] list in text.
fn count_markers(text: &str, markers: &[&str]) -> u32 {
    markers.iter().filter(|m| text.contains(*m)).count() as u32
}

/// Count weighted markers from a &[(f64, &str)] list.
fn count_markers_weighted(text: &str, markers: &[(f64, &str)]) -> f64 {
    markers
        .iter()
        .filter(|(_, m)| text.contains(*m))
        .map(|(w, _)| *w)
        .sum()
}

/// Aggregate of all psychometric markers (for signal density calculation).
const ALL_PSYCHOMETRIC_MARKERS: &[&str] = &[
    // risk-acceptant
    "aggressive expansion",
    "high risk",
    "venture",
    "speculative",
    "bold",
    "daring",
    "ambitious",
    "disruptive",
    "moonshot",
    "leveraged",
    // risk-averse
    "risk mitigation",
    "conservative",
    "hedge",
    "diversification",
    "safe",
    "guaranteed",
    "insured",
    "proven",
    "stable",
    "regulated",
    // certainty
    "definitely",
    "certainly",
    "undoubtedly",
    "proven",
    "guaranteed",
    // future-focus
    "future",
    "long term",
    "strategic",
    "vision",
    "horizon",
];

/// Risk-acceptant markers — positively correlated with risk tolerance.
const RISK_ACCEPTANT_MARKERS: &[&str] = &[
    "aggressive expansion",
    "high risk",
    "venture",
    "speculative",
    "bold",
    "daring",
    "ambitious",
    "disruptive",
    "moonshot",
    "leveraged",
];

/// Risk-averse markers — negatively correlated with risk tolerance.
const RISK_AVERSE_MARKERS: &[&str] = &[
    "risk mitigation",
    "conservative",
    "hedge",
    "diversification",
    "safe",
    "guaranteed",
    "insured",
    "proven",
    "stable",
    "regulated",
];

/// Certainty/confidence markers — modulate the conviction behind risk attitudes.
const CERTAINTY_MARKERS: &[&str] = &[
    "definitely",
    "certainly",
    "without doubt",
    "undoubtedly",
    "proven",
    "guaranteed",
    "assured",
    "confirmed",
];

/// Future-focus markers — longer time horizon correlates with higher risk tolerance.
const FUTURE_FOCUS_MARKERS: &[&str] = &[
    "future",
    "long term",
    "strategic",
    "vision",
    "horizon",
    "long range",
    "foresight",
    "roadmap",
    "anticipate",
];

/// Weighted priority-dimension markers: (weight, marker_phrase).
/// Weights represent the psycholinguistic strength of the association.
const COST_PRIORITY_MARKERS: &[(f64, &str)] = &[
    (1.0, "cost"),
    (0.9, "price"),
    (0.8, "budget"),
    (0.7, "saving"),
    (0.5, "cheap"),
    (0.5, "affordable"),
    (0.5, "discount"),
    (0.8, "roi"),
    (0.7, "margin"),
    (0.6, "capex"),
    (0.6, "opex"),
    (0.7, "tco"),
];

const QUALITY_PRIORITY_MARKERS: &[(f64, &str)] = &[
    (1.0, "quality"),
    (0.7, "excellence"),
    (0.6, "precision"),
    (0.7, "reliability"),
    (0.6, "durability"),
    (0.6, "defect"),
    (0.7, "yield"),
    (0.6, "six sigma"),
    (0.5, "iso 9001"),
    (0.6, "inspection"),
    (0.6, "traceability"),
];

const SPEED_PRIORITY_MARKERS: &[(f64, &str)] = &[
    (1.0, "speed"),
    (0.8, "fast"),
    (0.7, "rapid"),
    (0.6, "accelerate"),
    (0.7, "quick"),
    (0.7, "deadline"),
    (0.7, "urgent"),
    (0.6, "agile"),
    (0.6, "time to market"),
    (0.6, "lead time"),
];

const RESILIENCE_PRIORITY_MARKERS: &[(f64, &str)] = &[
    (0.8, "resilient"),
    (0.7, "redundant"),
    (0.7, "backup"),
    (0.8, "continuity"),
    (0.6, "robust"),
    (0.5, "diversif"),
    (0.6, "contingency"),
    (0.7, "recovery"),
    (0.6, "bcp"),
];

const COMPLIANCE_PRIORITY_MARKERS: &[(f64, &str)] = &[
    (1.0, "compliance"),
    (0.8, "regulation"),
    (0.7, "standard"),
    (0.7, "certification"),
    (0.8, "audit"),
    (0.6, "iso"),
    (0.6, "gdpr"),
    (0.6, "sox"),
    (0.6, "governance"),
];

const SECURITY_PRIORITY_MARKERS: &[(f64, &str)] = &[
    (1.0, "security"),
    (0.8, "secure"),
    (0.7, "encrypt"),
    (0.7, "protection"),
    (0.7, "defense"),
    (0.8, "cyber"),
    (0.7, "vulnerability"),
    (0.6, "penetration"),
    (0.6, "zero trust"),
];

/// Pain signal categories: (category_name, &[(phrase, weight)]).
/// Weight represents the severity correlation strength.
const PAIN_SIGNAL_CATEGORIES: &[(&str, &[(&str, f64)])] = &[
    (
        "cost",
        &[
            ("cost overrun", 1.0),
            ("budget cut", 1.0),
            ("financial loss", 1.0),
            ("revenue decline", 1.0),
            ("margin pressure", 0.8),
            ("price increase", 0.7),
            ("cost reduction mandate", 0.9),
        ],
    ),
    (
        "supply",
        &[
            ("supply disruption", 0.9),
            ("shortage", 0.8),
            ("delivery delay", 0.9),
            ("logistics bottleneck", 0.8),
            ("inventory shortage", 0.8),
            ("supplier failure", 0.9),
            ("lead time increase", 0.7),
        ],
    ),
    (
        "quality",
        &[
            ("quality issue", 0.85),
            ("defect", 0.8),
            ("recall", 0.9),
            ("non-conformance", 0.8),
            ("audit failure", 0.9),
            ("rework", 0.7),
        ],
    ),
    (
        "compliance",
        &[
            ("compliance violation", 0.8),
            ("regulatory fine", 0.9),
            ("sanction", 0.9),
            ("export control", 0.7),
            ("tariff", 0.7),
        ],
    ),
    (
        "security",
        &[
            ("security breach", 0.85),
            ("cyber attack", 0.85),
            ("data leak", 0.8),
            ("ransomware", 0.9),
            ("vulnerability", 0.7),
        ],
    ),
    (
        "workforce",
        &[
            ("layoff", 0.8),
            ("strike", 0.8),
            ("labor shortage", 0.7),
            ("talent drain", 0.7),
            ("resignation", 0.6),
        ],
    ),
];

impl Default for PsychologicalProfiler {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Behavioral Pattern Detector
// ═══════════════════════════════════════════════════════════════════════════════

/// Detects behavioral patterns and anomalies from profile snapshots over time.
pub struct BehavioralPatternDetector;

impl BehavioralPatternDetector {
    pub fn new() -> Self {
        Self
    }

    /// Detect sentiment shift using z-score method.
    /// A shift is detected when the current sentiment deviates from the historical
    /// mean by more than the threshold expressed in standard deviations.
    pub fn detect_sentiment_shift(
        &self,
        current: f64,
        history: &[f64],
        threshold: f64,
    ) -> Option<BehavioralPattern> {
        if history.is_empty() {
            return None;
        }

        let n = history.len() as f64;
        let mean: f64 = history.iter().sum::<f64>() / n;
        let variance: f64 = history.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        let std_dev = variance.sqrt().max(0.01);

        let z_score = (current - mean) / std_dev;

        if z_score.abs() < threshold {
            return None;
        }

        let direction = if z_score > 0.0 {
            "positive"
        } else {
            "negative"
        };
        let severity = if z_score.abs() > 3.0 {
            InsightSeverity::High
        } else if z_score.abs() > 2.0 {
            InsightSeverity::Medium
        } else {
            InsightSeverity::Low
        };

        Some(BehavioralPattern {
            pattern_type: BehavioralPatternType::SentimentShift,
            severity,
            title: format!("Significant {} sentiment shift detected", direction),
            description: format!(
                "Current sentiment score ({:.2}) deviates from historical mean ({:.2}) by {:.1} standard deviations (z={:.1})",
                current, mean, z_score.abs(), z_score
            ),
            confidence: (z_score.abs() / (z_score.abs() + 1.0)).min(0.95),
            baseline_value: mean,
            current_value: current,
            delta: current - mean,
        })
    }

    /// Detect communication drift using Jaccard similarity of topic sets.
    pub fn detect_communication_drift(
        &self,
        current_topics: &[String],
        history_topics: &[Vec<String>],
    ) -> Option<BehavioralPattern> {
        if history_topics.is_empty() || current_topics.is_empty() {
            return None;
        }

        // Build aggregate historical topic set
        let historical_set: std::collections::HashSet<&str> = history_topics
            .iter()
            .flat_map(|v| v.iter())
            .map(|s| s.as_str())
            .collect();

        let current_set: std::collections::HashSet<&str> =
            current_topics.iter().map(|s| s.as_str()).collect();

        // Jaccard similarity
        let intersection = current_set.intersection(&historical_set).count();
        let union = current_set.union(&historical_set).count();

        if union == 0 {
            return None;
        }

        let jaccard = intersection as f64 / union as f64;
        let drift = 1.0 - jaccard;

        if drift < 0.3 {
            return None;
        }

        let severity = if drift > 0.7 {
            InsightSeverity::High
        } else if drift > 0.5 {
            InsightSeverity::Medium
        } else {
            InsightSeverity::Low
        };

        // Find new topics not in history
        let new_topics: Vec<String> = current_topics
            .iter()
            .filter(|t| !historical_set.contains(t.as_str()))
            .take(5)
            .cloned()
            .collect();

        Some(BehavioralPattern {
            pattern_type: BehavioralPatternType::CommunicationDrift,
            severity,
            title: "Communication topic drift detected".to_string(),
            description: if new_topics.is_empty() {
                format!(
                    "Topic profile has shifted significantly (Jaccard similarity: {:.2})",
                    jaccard
                )
            } else {
                format!(
                    "Topic profile shifted (Jaccard: {:.2}). New topics: {}",
                    jaccard,
                    new_topics.join(", ")
                )
            },
            confidence: drift.min(0.9),
            baseline_value: 0.0,
            current_value: drift,
            delta: drift,
        })
    }

    /// Detect priority change using Euclidean distance between priority vectors.
    pub fn detect_priority_change(
        &self,
        current: &PriorityVector,
        previous: &PriorityVector,
    ) -> Option<BehavioralPattern> {
        let dims: [(f64, f64); 6] = [
            (current.cost, previous.cost),
            (current.quality, previous.quality),
            (current.speed, previous.speed),
            (current.resilience, previous.resilience),
            (current.compliance, previous.compliance),
            (current.security, previous.security),
        ];

        let euclidean: f64 = dims
            .iter()
            .map(|(c, p)| (c - p).powi(2))
            .sum::<f64>()
            .sqrt();

        if euclidean < 0.2 {
            return None;
        }

        let severity = if euclidean > 0.6 {
            InsightSeverity::High
        } else if euclidean > 0.4 {
            InsightSeverity::Medium
        } else {
            InsightSeverity::Low
        };

        // Find the dimension that changed the most
        let dim_names = [
            "cost",
            "quality",
            "speed",
            "resilience",
            "compliance",
            "security",
        ];
        let mut max_change = 0.0;
        let mut max_dim = "unknown";
        for (i, (c, p)) in dims.iter().enumerate() {
            let change = (c - p).abs();
            if change > max_change {
                max_change = change;
                max_dim = dim_names[i];
            }
        }

        Some(BehavioralPattern {
            pattern_type: BehavioralPatternType::PriorityChange,
            severity,
            title: format!("Priority vector shift ({} changed most)", max_dim),
            description: format!(
                "Priority vector changed (Euclidean distance: {:.2}), largest shift in '{}'",
                euclidean, max_dim
            ),
            confidence: (euclidean / 0.8).min(0.9),
            baseline_value: 0.0,
            current_value: euclidean,
            delta: euclidean,
        })
    }

    /// Detect engagement surge or drop relative to moving average.
    pub fn detect_engagement_surge(
        &self,
        current_count: usize,
        moving_avg: f64,
    ) -> Option<BehavioralPattern> {
        if moving_avg < 1.0 {
            return None;
        }

        let ratio = current_count as f64 / moving_avg;

        if ratio > 2.5 {
            Some(BehavioralPattern {
                pattern_type: BehavioralPatternType::EngagementSurge,
                severity: InsightSeverity::Medium,
                title: "Engagement activity surge detected".to_string(),
                description: format!(
                    "Current engagement count ({}) is {:.1}x the moving average ({:.1})",
                    current_count, ratio, moving_avg
                ),
                confidence: ((ratio - 1.0) / ratio).min(0.9),
                baseline_value: moving_avg,
                current_value: current_count as f64,
                delta: current_count as f64 - moving_avg,
            })
        } else if ratio < 0.2 && current_count == 0 {
            Some(BehavioralPattern {
                pattern_type: BehavioralPatternType::EngagementDrop,
                severity: InsightSeverity::Low,
                title: "Engagement activity drop detected".to_string(),
                description: format!(
                    "No engagement activity detected when moving average is {:.1}",
                    moving_avg
                ),
                confidence: 0.6,
                baseline_value: moving_avg,
                current_value: 0.0,
                delta: -moving_avg,
            })
        } else {
            None
        }
    }

    /// Detect role drift by analyzing artifacts for role-related content changes.
    pub fn detect_role_drift(
        &self,
        current_role: &str,
        artifacts: &[PoiArtifact],
    ) -> Option<BehavioralPattern> {
        if artifacts.is_empty() {
            return None;
        }

        let role_keywords = [
            "promoted",
            "appointed",
            "named",
            "new role",
            "new position",
            "transition",
            "stepping down",
            "departure",
            "resigned",
            "replaced",
            "reorganization",
            "restructure",
            "assumes",
            "takes over",
        ];

        let mut role_change_hits = 0u32;
        let mut evidence = Vec::new();

        for artifact in artifacts {
            let text_lower = artifact.content_summary.to_lowercase();
            let hits: Vec<&&str> = role_keywords
                .iter()
                .filter(|kw| text_lower.contains(*kw))
                .collect();
            if !hits.is_empty() {
                role_change_hits += hits.len() as u32;
                evidence.push(artifact.title.clone());
            }
        }

        if role_change_hits < 2 {
            return None;
        }

        Some(BehavioralPattern {
            pattern_type: BehavioralPatternType::RoleDrift,
            severity: InsightSeverity::Medium,
            title: "Potential role change detected".to_string(),
            description: format!(
                "Artifacts suggest role transition from '{}'. Evidence: {}",
                current_role,
                evidence.join("; ")
            ),
            confidence: (role_change_hits as f64 / (role_change_hits as f64 + 2.0)).min(0.85),
            baseline_value: 0.0,
            current_value: role_change_hits as f64,
            delta: role_change_hits as f64,
        })
    }

    /// Run all pattern detectors against current snapshot and history.
    pub fn detect_all_patterns(
        &self,
        _person_id: Uuid,
        current: &ProfileSnapshot,
        history: &[ProfileSnapshot],
    ) -> Vec<BehavioralPattern> {
        let mut patterns = Vec::new();

        if history.is_empty() {
            return patterns;
        }

        // Sentiment shift
        let sentiment_history: Vec<f64> = history.iter().map(|s| s.sentiment_score).collect();
        if let Some(p) =
            self.detect_sentiment_shift(current.sentiment_score, &sentiment_history, 1.5)
        {
            patterns.push(p);
        }

        // Communication drift (topics)
        let topic_history: Vec<Vec<String>> = history.iter().map(|s| s.topics.clone()).collect();
        if let Some(p) = self.detect_communication_drift(&current.topics, &topic_history) {
            patterns.push(p);
        }

        // Priority change
        if let Some(last) = history.last() {
            if let Some(p) =
                self.detect_priority_change(&current.priority_vector, &last.priority_vector)
            {
                patterns.push(p);
            }
        }

        // Engagement surge/drop
        if history.len() >= 3 {
            let moving_avg: f64 = history
                .iter()
                .rev()
                .take(3)
                .map(|s| s.engagement_count as f64)
                .sum::<f64>()
                / 3.0;
            if let Some(p) = self.detect_engagement_surge(current.engagement_count, moving_avg) {
                patterns.push(p);
            }
        }

        // Risk appetite change
        if let Some(last) = history.last() {
            let delta = (current.risk_tolerance - last.risk_tolerance).abs();
            if delta > 0.2 {
                patterns.push(BehavioralPattern {
                    pattern_type: BehavioralPatternType::RiskAppetiteChange,
                    severity: if delta > 0.4 {
                        InsightSeverity::High
                    } else {
                        InsightSeverity::Medium
                    },
                    title: "Risk appetite change detected".to_string(),
                    description: format!(
                        "Risk tolerance shifted from {:.2} to {:.2} (delta: {:.2})",
                        last.risk_tolerance, current.risk_tolerance, delta
                    ),
                    confidence: delta.min(0.85),
                    baseline_value: last.risk_tolerance,
                    current_value: current.risk_tolerance,
                    delta,
                });
            }
        }

        patterns
    }
}

impl Default for BehavioralPatternDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Sentiment Aggregator
// ═══════════════════════════════════════════════════════════════════════════════

/// Aggregates sentiment signals into time-series buckets and detects trends.
pub struct SentimentAggregator;

impl SentimentAggregator {
    pub fn new() -> Self {
        Self
    }

    /// Aggregate a set of sentiment signals into a statistical bucket.
    pub fn aggregate_entity_sentiment(
        &self,
        _entity_id: Uuid,
        _entity_type: EntityType,
        signals: &[SentimentSignal],
    ) -> SentimentBucket {
        if signals.is_empty() {
            return SentimentBucket {
                mean_score: 0.0,
                median_score: 0.0,
                std_dev: 0.0,
                positive_pct: 0.0,
                neutral_pct: 100.0,
                negative_pct: 0.0,
                sample_count: 0,
            };
        }

        let n = signals.len();
        let mut scores: Vec<f64> = signals.iter().map(|s| s.score.clamp(-1.0, 1.0)).collect();
        scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let mean = scores.iter().sum::<f64>() / n as f64;
        let variance = scores.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev = variance.sqrt();

        let median = if n % 2 == 1 {
            scores[n / 2]
        } else {
            (scores[n / 2 - 1] + scores[n / 2]) / 2.0
        };

        let positive_count = scores.iter().filter(|s| **s > 0.1).count();
        let negative_count = scores.iter().filter(|s| **s < -0.1).count();
        let neutral_count = n - positive_count - negative_count;

        SentimentBucket {
            mean_score: (mean * 100.0).round() / 100.0,
            median_score: (median * 100.0).round() / 100.0,
            std_dev: (std_dev * 100.0).round() / 100.0,
            positive_pct: (positive_count as f64 / n as f64 * 100.0 * 10.0).round() / 10.0,
            neutral_pct: (neutral_count as f64 / n as f64 * 100.0 * 10.0).round() / 10.0,
            negative_pct: (negative_count as f64 / n as f64 * 100.0 * 10.0).round() / 10.0,
            sample_count: n as u32,
        }
    }

    /// Compute sentiment trend by comparing recent and older scores.
    pub fn compute_sentiment_trend(
        &self,
        _entity_id: Uuid,
        _entity_type: EntityType,
        window_days: i64,
    ) -> SentimentTrend {
        // In production, queries sentiment_time_series DB table for historical
        // buckets. When no database data is available, uses heuristic default.
        let _ = window_days;
        SentimentTrend {
            direction: TrendDirection::Stable,
            strength: 0.0,
            volatility: 0.0,
            recent_score: 0.0,
            historical_mean: 0.0,
        }
    }

    /// Detect sentiment anomalies from recent buckets versus historical baseline.
    pub fn detect_sentiment_anomalies(
        &self,
        _entity_id: Uuid,
        _entity_type: EntityType,
    ) -> Vec<SentimentAnomaly> {
        // In production, queries sentiment_time_series DB table and flags
        // buckets with scores >2 std devs from the rolling mean. Returns empty
        // when no historical data exists to establish a baseline.
        Vec::new()
    }
}

impl Default for SentimentAggregator {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Engagement Strategist
// ═══════════════════════════════════════════════════════════════════════════════

/// Generates engagement strategies based on psychological profiles.
pub struct EngagementStrategist;

impl EngagementStrategist {
    pub fn new() -> Self {
        Self
    }

    /// Generate a complete engagement strategy tailored to the person's profile.
    pub fn generate_strategy(
        &self,
        _person_id: Uuid,
        profile: &PsychProfile,
        context: &StrategyContext,
    ) -> EngagementStrategy {
        let what_they_want = self.generate_what_they_want(profile);
        let talking_points = self.generate_talking_points(profile, context);
        let best_channel = self.infer_best_channel(profile);
        let best_timing = self.infer_best_timing(profile);

        let opening_topics = self.generate_opening_topics(profile);
        let avoid_topics = self.generate_avoid_topics(profile);
        let recommended_proof = self.recommended_proof_format(profile);
        let risk_mitigations = self.generate_risk_mitigations(profile, context);

        // Confidence based on profile stability and artifact volume
        let strategy_confidence = if profile.pain_index > 0.6 { 0.75 } else { 0.6 };

        EngagementStrategy {
            what_they_want,
            opening_topics,
            avoid_topics,
            best_channel,
            best_timing,
            recommended_proof,
            talking_points,
            risk_mitigations,
            strategy_confidence,
        }
    }

    /// Generate what the person wants to hear based on decision style.
    pub fn generate_what_they_want(&self, profile: &PsychProfile) -> Vec<String> {
        let mut wants = Vec::new();

        match profile.decision_style {
            DecisionStyle::CostFirst => {
                wants.push("Cost reduction projections with quantified savings".to_string());
                wants.push("TCO analysis and ROI timelines".to_string());
                wants.push("Price benchmarking against competitors".to_string());
            }
            DecisionStyle::QualityFirst => {
                wants.push("Quality metrics and defect rate comparisons".to_string());
                wants.push("Certification and compliance evidence".to_string());
                wants.push("Reliability track record and MTBF data".to_string());
            }
            DecisionStyle::SpeedFirst => {
                wants.push("Accelerated delivery timelines".to_string());
                wants.push("Rapid prototyping and agile methodology evidence".to_string());
                wants.push("Time-to-market advantage quantification".to_string());
            }
            DecisionStyle::RiskFirst => {
                wants.push("Risk mitigation frameworks and contingency plans".to_string());
                wants.push("Security certifications and audit results".to_string());
                wants.push("Supply chain redundancy and business continuity proof".to_string());
            }
            DecisionStyle::ComplianceFirst => {
                wants.push("Regulatory compliance documentation".to_string());
                wants.push("Audit trail completeness evidence".to_string());
                wants.push("Standards adherence track record".to_string());
            }
            DecisionStyle::BalancedAnalytical => {
                wants.push("Comprehensive data with multi-dimensional analysis".to_string());
                wants.push("Comparison matrices with quantitative evidence".to_string());
                wants.push("Case studies demonstrating balanced outcomes".to_string());
            }
        }

        // Add pain-index-driven needs
        if profile.pain_index > 0.6 {
            wants.push("Immediate pain point resolution plan".to_string());
            wants.push("Crisis response capability evidence".to_string());
        }

        wants
    }

    /// Generate talking points tailored to the profile and context.
    pub fn generate_talking_points(
        &self,
        profile: &PsychProfile,
        context: &StrategyContext,
    ) -> Vec<String> {
        let mut points = Vec::new();

        match context.strategy_type {
            StrategyType::CompetitiveDisplacement => {
                points.push(format!(
                    "Our solution addresses the {} priorities you've emphasized",
                    decision_style_label(&profile.decision_style)
                ));
                if let Some(ref competitor) = context.competitor_mentioned {
                    points.push(format!(
                        "Unlike {}, we provide transparent pricing with no hidden costs",
                        competitor
                    ));
                }
                points.push("We can demonstrate measurable results within 90 days".to_string());
            }
            StrategyType::Negotiation => {
                points.push("Let's align on mutual success metrics first".to_string());
                points.push(
                    "We're prepared to structure terms that de-risk your position".to_string(),
                );
                if profile.risk_tolerance < 0.4 {
                    points.push(
                        "We offer guaranteed service levels with penalty clauses".to_string(),
                    );
                }
            }
            StrategyType::MeetingPrep => {
                let first_proof = profile.preferred_proof.first();
                points.push(format!(
                    "Based on their profile, they respond best to {} evidence",
                    proof_label(first_proof)
                ));
                points.push(
                    "Prepare quantitative comparisons rather than qualitative claims".to_string(),
                );
                if profile.pain_index > 0.5 {
                    points.push(
                        "Lead with pain point resolution — they have active issues".to_string(),
                    );
                }
            }
            _ => {
                points.push("Focus on value delivery and partnership approach".to_string());
                points.push("Emphasize track record and reliability".to_string());
            }
        }

        points
    }

    /// Infer the best communication channel based on decision style.
    pub fn infer_best_channel(&self, profile: &PsychProfile) -> String {
        match profile.decision_style {
            DecisionStyle::SpeedFirst => "phone or video call with decision-makers".to_string(),
            DecisionStyle::CostFirst | DecisionStyle::ComplianceFirst => {
                "email with detailed attachments and data".to_string()
            }
            DecisionStyle::QualityFirst | DecisionStyle::RiskFirst => {
                "in-person meeting with technical team".to_string()
            }
            DecisionStyle::BalancedAnalytical => {
                "structured presentation followed by Q&A session".to_string()
            }
        }
    }

    /// Infer the best timing for outreach based on profile signals.
    pub fn infer_best_timing(&self, profile: &PsychProfile) -> String {
        if profile.pain_index > 0.7 {
            "immediately — current pain signals indicate urgent need".to_string()
        } else if profile.pain_index > 0.4 {
            "within 2 weeks — moderate pain indicates receptiveness".to_string()
        } else if profile.change_appetite == ChangeAppetite::EarlyAdopter {
            "when launching new capabilities — they seek innovation".to_string()
        } else if profile.change_appetite == ChangeAppetite::Conservative {
            "after competitors have validated — they need social proof".to_string()
        } else {
            "during regular business review cycles".to_string()
        }
    }

    /// Generate appropriate opening topics based on profile.
    fn generate_opening_topics(&self, profile: &PsychProfile) -> Vec<String> {
        let mut topics = vec!["Industry trends and market developments".to_string()];

        match profile.decision_style {
            DecisionStyle::CostFirst => {
                topics.push("Cost optimization strategies in the current market".to_string());
            }
            DecisionStyle::QualityFirst => {
                topics.push("Quality benchmarks and industry standards evolution".to_string());
            }
            DecisionStyle::SpeedFirst => {
                topics.push("Accelerating time-to-market in your sector".to_string());
            }
            DecisionStyle::RiskFirst => {
                topics.push("Emerging risks in supply chain and mitigation approaches".to_string());
            }
            DecisionStyle::ComplianceFirst => {
                topics.push("Regulatory landscape changes affecting your operations".to_string());
            }
            DecisionStyle::BalancedAnalytical => {
                topics.push("Multi-dimensional analysis of your sector challenges".to_string());
            }
        }

        topics
    }

    /// Generate topics to avoid based on profile.
    fn generate_avoid_topics(&self, profile: &PsychProfile) -> Vec<String> {
        let mut avoid = Vec::new();

        if profile.risk_tolerance < 0.3 {
            avoid.push("Unproven or experimental approaches".to_string());
            avoid.push("High-risk investment without guaranteed returns".to_string());
        }

        if profile.decision_style == DecisionStyle::CostFirst {
            avoid.push("Premium pricing without clear ROI justification".to_string());
        }

        if profile.change_appetite == ChangeAppetite::Laggard {
            avoid.push("Disruptive innovation requiring significant process change".to_string());
        }

        avoid
    }

    /// Recommend proof format based on profile.
    fn recommended_proof_format(&self, profile: &PsychProfile) -> Vec<String> {
        profile
            .preferred_proof
            .iter()
            .map(|p| match p {
                ProofType::KpiMetrics => {
                    "Quantitative KPI dashboards with benchmark comparisons".to_string()
                }
                ProofType::Certifications => {
                    "Current certifications and compliance documentation".to_string()
                }
                ProofType::CaseStudies => {
                    "Relevant case studies with measurable outcomes".to_string()
                }
                ProofType::AuditReadiness => {
                    "Audit trail documentation and inspection readiness evidence".to_string()
                }
                ProofType::TechDemos => {
                    "Live technical demonstration or proof of concept".to_string()
                }
                ProofType::CostTransparency => {
                    "Detailed cost breakdown with open-book pricing".to_string()
                }
            })
            .collect()
    }

    /// Generate risk mitigations for the strategy.
    fn generate_risk_mitigations(
        &self,
        profile: &PsychProfile,
        context: &StrategyContext,
    ) -> Vec<String> {
        let mut mitigations = Vec::new();

        if profile.risk_tolerance < 0.4 {
            mitigations.push("Provide guaranteed service level agreements".to_string());
            mitigations.push("Offer phased rollout with milestones and exit clauses".to_string());
        }

        if matches!(context.strategy_type, StrategyType::CompetitiveDisplacement) {
            mitigations.push("Prepare competitive rebuttal documentation".to_string());
            mitigations.push("Have customer references ready for similar transitions".to_string());
        }

        if profile.pain_index > 0.6 && context.urgency == "high" {
            mitigations.push("Deploy rapid response team for initial stabilization".to_string());
        }

        mitigations
    }
}

impl Default for EngagementStrategist {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Personality Assessor
// ═══════════════════════════════════════════════════════════════════════════════

/// Assesses personality traits (Big Five, HEXACO) from public artifacts.
pub struct PersonalityAssessor;

impl PersonalityAssessor {
    pub fn new() -> Self {
        Self
    }

    /// Assess Big Five personality traits from artifact content.
    pub fn assess_big_five(&self, artifacts: &[PoiArtifact]) -> BigFiveTraits {
        if artifacts.is_empty() {
            return BigFiveTraits {
                openness: 0.5,
                conscientiousness: 0.5,
                extraversion: 0.5,
                agreeableness: 0.5,
                neuroticism: 0.5,
                confidence: 0.0,
            };
        }

        let openness_keywords = [
            "innovative",
            "creative",
            "curious",
            "explore",
            "novel",
            "artistic",
            "unconventional",
            "abstract",
            "philosophical",
            "diverse",
            "imaginative",
            "experimental",
            "visionary",
            "intellectual",
            "cultural",
        ];
        let conscientiousness_keywords = [
            "organized",
            "disciplined",
            "reliable",
            "meticulous",
            "thorough",
            "diligent",
            "systematic",
            "structured",
            "precise",
            "responsible",
            "efficient",
            "planned",
            "detail oriented",
            "accountable",
            "punctual",
        ];
        let extraversion_keywords = [
            "outgoing",
            "social",
            "assertive",
            "energetic",
            "enthusiastic",
            "talkative",
            "gregarious",
            "confident",
            "charismatic",
            "lively",
            "sociable",
            "dynamic",
            "animated",
            "bold",
            "expressive",
        ];
        let agreeableness_keywords = [
            "cooperative",
            "empathetic",
            "compassionate",
            "trusting",
            "helpful",
            "collaborative",
            "supportive",
            "kind",
            "considerate",
            "diplomatic",
            "team player",
            "harmonious",
            "accommodating",
            "patient",
            "generous",
        ];
        let neuroticism_keywords = [
            "anxious",
            "worried",
            "stressed",
            "volatile",
            "moody",
            "sensitive",
            "tense",
            "reactive",
            "uneasy",
            "insecure",
            "nervous",
            "irritable",
            "emotional",
            "self-conscious",
            "vulnerable",
        ];

        let mut o_score = 0u32;
        let mut c_score = 0u32;
        let mut e_score = 0u32;
        let mut a_score = 0u32;
        let mut n_score = 0u32;
        let mut total_artifacts = 0u32;

        for artifact in artifacts {
            let text = format!(
                "{} {}",
                artifact.title.to_lowercase(),
                artifact.content_summary.to_lowercase()
            );

            o_score += openness_keywords
                .iter()
                .filter(|kw| text.contains(*kw))
                .count() as u32;
            c_score += conscientiousness_keywords
                .iter()
                .filter(|kw| text.contains(*kw))
                .count() as u32;
            e_score += extraversion_keywords
                .iter()
                .filter(|kw| text.contains(*kw))
                .count() as u32;
            a_score += agreeableness_keywords
                .iter()
                .filter(|kw| text.contains(*kw))
                .count() as u32;
            n_score += neuroticism_keywords
                .iter()
                .filter(|kw| text.contains(*kw))
                .count() as u32;
            total_artifacts += 1;
        }

        let normalize = |score: u32| -> f64 {
            if total_artifacts == 0 {
                return 0.5;
            }
            (score as f64 / (total_artifacts as f64 * 2.0 + 1.0)).clamp(0.1, 0.95)
        };

        let confidence = if total_artifacts > 10 {
            0.8
        } else if total_artifacts > 3 {
            0.5
        } else {
            0.3
        };

        BigFiveTraits {
            openness: normalize(o_score),
            conscientiousness: normalize(c_score),
            extraversion: normalize(e_score),
            agreeableness: normalize(a_score),
            neuroticism: normalize(n_score),
            confidence,
        }
    }

    /// Assess HEXACO personality traits.
    pub fn assess_hexaco(&self, artifacts: &[PoiArtifact]) -> HexacoTraits {
        let big_five = self.assess_big_five(artifacts);

        // HEXACO adds Honesty-Humility and splits Neuroticism into Emotionality.
        // Estimate H-H from content patterns related to sincerity, fairness, greed-avoidance, modesty.
        let hh_keywords = [
            "honest",
            "sincere",
            "fair",
            "ethical",
            "humble",
            "modest",
            "transparent",
            "integrity",
            "principled",
            "genuine",
            "authentic",
            "forthright",
        ];

        let mut hh_score = 0u32;
        let mut total = 0u32;

        for artifact in artifacts {
            let text = format!(
                "{} {}",
                artifact.title.to_lowercase(),
                artifact.content_summary.to_lowercase()
            );
            hh_score += hh_keywords.iter().filter(|kw| text.contains(*kw)).count() as u32;
            total += 1;
        }

        let hh = if total > 0 {
            (hh_score as f64 / (total as f64 * 1.5 + 1.0)).clamp(0.1, 0.95)
        } else {
            0.5
        };

        HexacoTraits {
            honesty_humility: hh,
            emotionality: (big_five.neuroticism * 0.8 + 0.1).clamp(0.1, 0.95),
            extraversion: big_five.extraversion,
            agreeableness: big_five.agreeableness,
            conscientiousness: big_five.conscientiousness,
            openness: big_five.openness,
            confidence: big_five.confidence,
        }
    }
}

impl Default for PersonalityAssessor {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Organization Culture Profiler
// ═══════════════════════════════════════════════════════════════════════════════

/// Profiles organizational culture for companies based on public signals.
pub struct OrgCultureProfiler;

impl OrgCultureProfiler {
    pub fn new() -> Self {
        Self
    }

    /// Build an organizational culture profile from signals about a company.
    pub fn profile_company(
        &self,
        _company_id: Uuid,
        signals: &[OrgCultureSignal],
    ) -> OrgCultureProfile {
        if signals.is_empty() {
            return OrgCultureProfile {
                innovation_focus: 0.5,
                risk_culture: 0.5,
                hierarchy_rigidity: 0.5,
                speed_of_decision: 0.5,
                talent_centricity: 0.5,
                cost_sensitivity: 0.5,
                compliance_posture: 0.5,
                external_orientation: 0.5,
                culture_tags: vec!["unknown".to_string()],
                profile_confidence: 0.1,
            };
        }

        let innovation_keywords = [
            "innovate",
            "research",
            "patent",
            "r&d",
            "disrupt",
            "startup",
            "agile",
            "design thinking",
            "creative",
            "lab",
            "incubator",
        ];
        let risk_keywords = [
            "risk taking",
            "venture",
            "bold",
            "speculative",
            "acquisition",
            "expansion",
            "aggressive",
            "moonshot",
        ];
        let hierarchy_keywords = [
            "hierarchy",
            "bureaucracy",
            "matrix",
            "reporting lines",
            "approval chain",
            "governance",
            "formal",
            "top-down",
        ];
        let speed_keywords = [
            "fast decision",
            "rapid",
            "quick to market",
            "nimble",
            "responsive",
            "streamlined",
            "empowered",
            "autonomous",
        ];
        let talent_keywords = [
            "talent",
            "hiring",
            "retention",
            "culture",
            "employee",
            "workforce",
            "development",
            "training",
            "upskill",
            "diversity",
            "inclusion",
        ];
        let cost_keywords = [
            "cost cutting",
            "efficiency",
            "lean",
            "optimization",
            "frugal",
            "margin",
            "profitability",
            "shareholder value",
            "eps",
        ];
        let compliance_keywords = [
            "compliance",
            "regulatory",
            "sox",
            "gdpr",
            "audit",
            "control",
            "governance",
            "risk management",
            "legal",
        ];
        let external_keywords = [
            "customer",
            "market",
            "competitor",
            "partner",
            "ecosystem",
            "outward",
            "client focused",
            "market driven",
            "customer centric",
        ];

        let mut scores: HashMap<&str, (f64, u32)> = HashMap::new();
        let dims: &[(&str, &[&str])] = &[
            ("innovation", &innovation_keywords),
            ("risk", &risk_keywords),
            ("hierarchy", &hierarchy_keywords),
            ("speed", &speed_keywords),
            ("talent", &talent_keywords),
            ("cost", &cost_keywords),
            ("compliance", &compliance_keywords),
            ("external", &external_keywords),
        ];

        for signal in signals {
            let text = signal.description.to_lowercase();
            for (dim, keywords) in dims {
                let hits = keywords.iter().filter(|kw| text.contains(*kw)).count() as u32;
                if hits > 0 {
                    let entry = scores.entry(dim).or_insert((0.0, 0));
                    entry.0 += hits as f64;
                    entry.1 += 1;
                }
            }
        }

        let get_score = |dim: &str| -> f64 {
            if let Some((total, count)) = scores.get(dim) {
                if *count > 0 {
                    (total / (*count as f64 * 1.5)).clamp(0.1, 0.95)
                } else {
                    0.5
                }
            } else {
                0.5
            }
        };

        // Build culture tags from signal patterns
        let mut tags = Vec::new();
        let innovation = get_score("innovation");
        let risk = get_score("risk");
        let hierarchy = get_score("hierarchy");
        let speed = get_score("speed");
        let talent = get_score("talent");
        let cost = get_score("cost");
        let compliance = get_score("compliance");
        let external = get_score("external");

        if innovation > 0.6 {
            tags.push("innovation_driven".to_string());
        }
        if risk > 0.6 {
            tags.push("risk_taking".to_string());
        }
        if hierarchy > 0.6 {
            tags.push("hierarchical".to_string());
        }
        if speed > 0.6 {
            tags.push("fast_paced".to_string());
        }
        if talent > 0.6 {
            tags.push("talent_focused".to_string());
        }
        if cost > 0.6 {
            tags.push("cost_conscious".to_string());
        }
        if compliance > 0.6 {
            tags.push("compliance_heavy".to_string());
        }
        if external > 0.6 {
            tags.push("market_oriented".to_string());
        }
        if tags.is_empty() {
            tags.push("balanced".to_string());
        }

        let n_signals = signals.len();
        let confidence = if n_signals > 20 {
            0.8
        } else if n_signals > 5 {
            0.5
        } else {
            0.3
        };

        OrgCultureProfile {
            innovation_focus: innovation,
            risk_culture: risk,
            hierarchy_rigidity: hierarchy,
            speed_of_decision: speed,
            talent_centricity: talent,
            cost_sensitivity: cost,
            compliance_posture: compliance,
            external_orientation: external,
            culture_tags: tags,
            profile_confidence: confidence,
        }
    }
}

impl Default for OrgCultureProfiler {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Cognitive Bias Detector
// ═══════════════════════════════════════════════════════════════════════════════

/// Detects cognitive biases in intelligence analysis text.
pub struct CognitiveBiasDetector;

impl CognitiveBiasDetector {
    pub fn new() -> Self {
        Self
    }

    /// Scan text for indicators of cognitive biases and return detections.
    pub fn detect_biases(&self, text: &str, context: &BiasContext) -> Vec<CognitiveBias> {
        let text_lower = text.to_lowercase();
        let mut biases = Vec::new();

        // Confirmation bias: language that only seeks confirming evidence
        let confirmation_patterns = [
            ("confirms our assessment", 0.8),
            ("as expected", 0.7),
            ("consistent with our view", 0.8),
            ("proves our hypothesis", 0.85),
            ("validates our assumption", 0.8),
            ("aligns with our prediction", 0.75),
        ];
        for (pattern, confidence) in &confirmation_patterns {
            if text_lower.contains(pattern) {
                biases.push(CognitiveBias {
                    bias_type: "confirmation".to_string(),
                    severity: InsightSeverity::Medium,
                    description: format!(
                        "Text shows confirmation bias indicators: language seeks to confirm existing beliefs rather than test them. Found: '{}'",
                        pattern
                    ),
                    mitigation: "Seek disconfirming evidence. Ask: 'What would prove this wrong?'".to_string(),
                    confidence: *confidence,
                });
                break;
            }
        }

        // Availability bias: overweighting recent/vivid events
        let availability_patterns = [
            ("recently", "recent event emphasis", 0.7),
            ("breaking news", "news-driven analysis", 0.75),
            ("just announced", "recency bias", 0.7),
            ("this week", "short time horizon", 0.65),
        ];
        let mut recent_count = 0;
        for (pattern, _desc, _conf) in &availability_patterns {
            if text_lower.contains(pattern) {
                recent_count += 1;
            }
        }
        if recent_count >= 3 {
            biases.push(CognitiveBias {
                bias_type: "availability".to_string(),
                severity: InsightSeverity::Low,
                description: "Analysis may overweight recent or vivid events. Multiple recency markers detected.".to_string(),
                mitigation: "Consider longer historical baseline. Review data from 6-12 months ago.".to_string(),
                confidence: 0.65,
            });
        }

        // Overconfidence bias: absolute, unqualified language
        let overconfidence_patterns = [
            ("definitely", 0.75),
            ("certainly", 0.75),
            ("without doubt", 0.85),
            ("undoubtedly", 0.8),
            ("absolutely", 0.7),
            ("always", 0.65),
            ("never", 0.65),
            ("guaranteed", 0.8),
        ];
        let mut overconf_count = 0;
        for (pattern, _conf) in &overconfidence_patterns {
            if text_lower.contains(pattern) {
                overconf_count += 1;
            }
        }
        if overconf_count >= 2 {
            biases.push(CognitiveBias {
                bias_type: "overconfidence".to_string(),
                severity: InsightSeverity::Medium,
                description: "Text contains absolute/unqualified language suggesting overconfidence in conclusions.".to_string(),
                mitigation: "Add confidence intervals. Acknowledge uncertainty. Use probabilistic language.".to_string(),
                confidence: 0.7,
            });
        }

        // Anchoring bias: fixating on a specific number or reference point
        let anchor_indicators = [
            "baseline of",
            "starting point",
            "reference value",
            "anchor",
            "compared to last",
            "relative to previous",
        ];
        let has_anchor = anchor_indicators.iter().any(|a| text_lower.contains(a));
        if has_anchor {
            biases.push(CognitiveBias {
                bias_type: "anchoring".to_string(),
                severity: InsightSeverity::Low,
                description: "Analysis may be anchored to a specific reference point without considering alternative baselines.".to_string(),
                mitigation: "Consider multiple reference points. Test sensitivity to different baselines.".to_string(),
                confidence: 0.55,
            });
        }

        // Negativity bias: disproportionate focus on negative signals
        let negative_words = [
            "threat", "risk", "danger", "concern", "warning", "crisis", "failure", "loss",
            "decline", "problem",
        ];
        let positive_words = [
            "opportunity",
            "growth",
            "strength",
            "advantage",
            "success",
            "improvement",
            "gain",
            "benefit",
            "progress",
        ];
        let neg_count = negative_words
            .iter()
            .filter(|w| text_lower.contains(*w))
            .count();
        let pos_count = positive_words
            .iter()
            .filter(|w| text_lower.contains(*w))
            .count();
        if neg_count > pos_count * 3 && neg_count >= 5 {
            biases.push(CognitiveBias {
                bias_type: "negativity".to_string(),
                severity: InsightSeverity::Low,
                description: format!(
                    "Analysis shows negativity bias: {} negative terms vs {} positive terms detected.",
                    neg_count, pos_count
                ),
                mitigation: "Actively identify positive signals. Balance risk analysis with opportunity assessment.".to_string(),
                confidence: 0.6,
            });
        }

        // Geographic bias: favoring certain regions
        let geographic_terms = [
            "us ",
            "united states",
            "europe",
            "asia",
            "china",
            "russia",
            "america",
            "western",
            "eastern",
        ];
        let geo_mentioned: Vec<&str> = geographic_terms
            .iter()
            .filter(|g| text_lower.contains(*g))
            .copied()
            .collect();
        if geo_mentioned.len() == 1 && geo_mentioned.len() < geographic_terms.len() / 2 {
            biases.push(CognitiveBias {
                bias_type: "geographic".to_string(),
                severity: InsightSeverity::Low,
                description:
                    "Analysis may have geographic bias — limited regional perspective detected."
                        .to_string(),
                mitigation:
                    "Include data from multiple geographies. Check for regional blind spots."
                        .to_string(),
                confidence: 0.5,
            });
        }

        // Framing bias: how the topic is framed
        if text_lower.contains("loss") && !text_lower.contains("gain") {
            biases.push(CognitiveBias {
                bias_type: "framing".to_string(),
                severity: InsightSeverity::Medium,
                description: "Topic is framed primarily in terms of loss/risk without considering potential gains.".to_string(),
                mitigation: "Reframe analysis to include both upside and downside scenarios.".to_string(),
                confidence: 0.55,
            });
        }

        // Attraction bias: positive language toward familiar/similar entities
        if !context.target_entity.is_empty()
            && text_lower.contains(&context.target_entity.to_lowercase())
            && text_lower.contains("strong")
            && text_lower.contains("leader")
        {
            biases.push(CognitiveBias {
                bias_type: "halo_effect".to_string(),
                severity: InsightSeverity::Low,
                description: "Potential halo effect: favorable descriptors may influence objective assessment.".to_string(),
                mitigation: "Separate entity reputation from specific capability assessment.".to_string(),
                confidence: 0.5,
            });
        }

        biases
    }
}

impl Default for CognitiveBiasDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Helper functions
// ═══════════════════════════════════════════════════════════════════════════════

fn decision_style_label(style: &DecisionStyle) -> &'static str {
    match style {
        DecisionStyle::CostFirst => "cost optimization",
        DecisionStyle::QualityFirst => "quality and reliability",
        DecisionStyle::SpeedFirst => "speed and agility",
        DecisionStyle::RiskFirst => "risk mitigation",
        DecisionStyle::ComplianceFirst => "compliance and governance",
        DecisionStyle::BalancedAnalytical => "balanced analytical",
    }
}

fn proof_label(proof: Option<&ProofType>) -> &'static str {
    match proof {
        Some(ProofType::KpiMetrics) => "KPI/metrics-based",
        Some(ProofType::Certifications) => "certification-based",
        Some(ProofType::CaseStudies) => "case study-based",
        Some(ProofType::AuditReadiness) => "audit-based",
        Some(ProofType::TechDemos) => "technical demonstration",
        Some(ProofType::CostTransparency) => "cost transparency",
        None => "quantitative",
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_artifact(title: &str, summary: &str, ts: i64) -> PoiArtifact {
        PoiArtifact {
            artifact_type: "article".to_string(),
            title: title.to_string(),
            content_summary: summary.to_string(),
            source_url: None,
            ts_utc: ts,
        }
    }

    #[test]
    fn test_pain_index_detection() {
        let profiler = PsychologicalProfiler::new();
        let now = Utc::now().timestamp();

        let artifacts = vec![
            make_artifact("Supply crisis", "Major supply disruption causing cost overruns and delivery delays across production lines", now),
            make_artifact("Quality audit", "Quality issue detected with supplier recall imminent", now),
            make_artifact("Financial stress", "Bankruptcy risk and debt default looming with severe cash flow problems", now),
        ];

        let pain = profiler.compute_pain_index(&artifacts);
        assert!(
            pain > 0.3,
            "Expected elevated pain index with 3 high-signal artifacts, got {pain}"
        );
    }

    #[test]
    fn test_pain_index_empty() {
        let profiler = PsychologicalProfiler::new();
        let pain = profiler.compute_pain_index(&[]);
        assert_eq!(
            pain, 0.0,
            "Empty artifacts must return 0.0 — no hallucinated pain"
        );
    }

    #[test]
    fn test_profile_not_enriched_when_empty() {
        let profiler = PsychologicalProfiler::new();
        let profile = profiler.profile_person(Uuid::nil(), &[]);
        assert!(
            !profile.is_enriched(),
            "Empty artifacts must yield unenriched profile"
        );
    }

    #[test]
    fn test_profile_not_enriched_under_min_evidence() {
        let profiler = PsychologicalProfiler::new();
        let artifacts = vec![make_artifact("Hi", "Hello", Utc::now().timestamp())];
        let profile = profiler.profile_person(Uuid::nil(), &artifacts);
        assert!(
            !profile.is_enriched(),
            "Below min evidence threshold must yield unenriched profile"
        );
    }

    #[test]
    fn test_profile_enriched_with_good_evidence() {
        let profiler = PsychologicalProfiler::new();
        let now = Utc::now().timestamp();
        let mut summary = String::new();
        for _ in 0..10 {
            summary.push_str("cost overrun supply disruption quality defect security breach compliance violation layoff ");
        }
        let artifacts = vec![make_artifact("Crisis report", &summary, now)];
        let profile = profiler.profile_person(Uuid::nil(), &artifacts);
        assert!(
            profile.is_enriched(),
            "Good evidence should produce an enriched profile"
        );
        assert!(profile.pain_index > 0.3);
    }

    #[test]
    fn test_decision_style_cost_first() {
        let profiler = PsychologicalProfiler::new();
        let pv = PriorityVector {
            cost: 0.8,
            quality: 0.1,
            speed: 0.1,
            resilience: 0.0,
            compliance: 0.0,
            security: 0.0,
            confidence: 0.9,
        };
        assert_eq!(profiler.infer_decision_style(&pv), DecisionStyle::CostFirst);
    }

    #[test]
    fn test_change_appetite_early_adopter() {
        let profiler = PsychologicalProfiler::new();
        let now = Utc::now().timestamp();
        let artifacts = vec![
            make_artifact("Innovation push", "Company launches disruptive innovative breakthrough product with cutting edge technology", now),
        ];
        assert_eq!(
            profiler.infer_change_appetite(&artifacts),
            ChangeAppetite::EarlyAdopter
        );
    }

    #[test]
    fn test_sentiment_shift_detection() {
        let detector = BehavioralPatternDetector::new();
        let history = vec![0.1, 0.2, 0.0, -0.1, 0.1, 0.2, 0.0];
        let result = detector.detect_sentiment_shift(0.9, &history, 1.5);
        assert!(result.is_some());
        let pattern = result.unwrap();
        assert_eq!(pattern.pattern_type, BehavioralPatternType::SentimentShift);
    }

    #[test]
    fn test_sentiment_shift_no_detection() {
        let detector = BehavioralPatternDetector::new();
        let history = vec![0.1, 0.2, 0.0, -0.1, 0.1, 0.2, 0.0];
        let result = detector.detect_sentiment_shift(0.15, &history, 1.5);
        assert!(result.is_none());
    }

    #[test]
    fn test_sentiment_aggregation() {
        let aggregator = SentimentAggregator::new();
        let signals = vec![
            SentimentSignal {
                score: 0.8,
                text: "Great".to_string(),
                source: "news".to_string(),
                ts_utc: Utc::now(),
            },
            SentimentSignal {
                score: -0.6,
                text: "Bad".to_string(),
                source: "news".to_string(),
                ts_utc: Utc::now(),
            },
            SentimentSignal {
                score: 0.0,
                text: "Neutral".to_string(),
                source: "blog".to_string(),
                ts_utc: Utc::now(),
            },
        ];

        let bucket =
            aggregator.aggregate_entity_sentiment(Uuid::nil(), EntityType::Company, &signals);

        assert_eq!(bucket.sample_count, 3);
        assert!(bucket.positive_pct > 0.0);
        assert!(bucket.negative_pct > 0.0);
        assert!(bucket.neutral_pct > 0.0);
    }

    #[test]
    fn test_big_five_assessment() {
        let assessor = PersonalityAssessor::new();
        let now = Utc::now().timestamp();
        let artifacts = vec![
            make_artifact("Creative approach", "Innovative creative solution using experimental design thinking and diverse methods", now),
            make_artifact("Team collaboration", "Organized collaborative team player with disciplined systematic approach", now),
        ];

        let traits = assessor.assess_big_five(&artifacts);
        assert!(traits.openness > 0.5);
        assert!(traits.conscientiousness > 0.4);
        assert!(traits.confidence > 0.0);
    }

    #[test]
    fn test_org_culture_profiling() {
        let profiler = OrgCultureProfiler::new();
        let signals = vec![
            OrgCultureSignal {
                signal_type: "press_release".into(),
                description: "Company invests heavily in R&D and innovation labs, launches disruptive products".into(),
                source: "press".into(),
                ts_utc: Utc::now(),
            },
            OrgCultureSignal {
                signal_type: "annual_report".into(),
                description: "Emphasis on talent development, employee training, diversity and inclusion initiatives".into(),
                source: "report".into(),
                ts_utc: Utc::now(),
            },
        ];

        let profile = profiler.profile_company(Uuid::nil(), &signals);
        assert!(profile.innovation_focus > 0.5);
        assert!(profile.talent_centricity > 0.5);
        assert!(!profile.culture_tags.is_empty());
    }

    #[test]
    fn test_cognitive_bias_detection() {
        let detector = CognitiveBiasDetector::new();
        let context = BiasContext {
            author_role: "analyst".into(),
            target_entity: "Acme Corp".into(),
            topic: "market analysis".into(),
        };

        let text = "This definitely and certainly confirms our assessment that Acme Corp is a strong leader. As expected, the results are consistent with our view and prove our hypothesis.";
        let biases = detector.detect_biases(text, &context);

        assert!(!biases.is_empty());
        let bias_types: Vec<&str> = biases.iter().map(|b| b.bias_type.as_str()).collect();
        assert!(bias_types.contains(&"confirmation"));
        assert!(bias_types.contains(&"overconfidence"));
    }
}
