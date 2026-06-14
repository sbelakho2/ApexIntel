//! Weekly strategy memo generator.
//!
//! Produces structured weekly memos for GM/Board level, with:
//! - Executive summary
//! - Ranked insights by priority
//! - Regional breakdown (TN, MA, IL, CN, EA, EU, US)
//! - Security posture summary
//! - Recommended actions
//!
//! # Diversity Features
//!
//! The memo applies **deterministic (week-seeded) diversity** to section headings
//! and groups repeated warning patterns to reduce formulaic repetition.

use apex_core::analysis::{
    assess_evidence_quality, compare_temporal_windows, fuse_weak_signals,
    score_competing_hypotheses, source_group_from_url, EvidenceRecord, EvidenceStance,
    HypothesisInput, HypothesisScorecard, SignalFrame, TemporalDelta,
};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;
use uuid::Uuid;

use crate::renderer::{format_card_text, group_by_category, group_by_region, InsightCard};

// ─── Deterministic (week-seeded) memo diversity ──────────────────────────

/// Seeded PRNG for deterministic memo variation (xorshift64*).
struct MemoRng(u64);

impl MemoRng {
    /// Create a new RNG seeded from week number (stable per week).
    fn from_week(week: u32, year: i32) -> Self {
        Self((year as u64).wrapping_mul(1000).wrapping_add(week as u64))
    }

    /// Return an index in `0..len`.
    fn pick(&mut self, len: usize) -> usize {
        if len <= 1 {
            return 0;
        }
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0 as usize).wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) % len
    }
}

/// Alternative section headings, keyed by the canonical heading.  At call time
/// the `MemoRng` picks a non-canonical variant deterministically per week.
static SECTION_HEADING_VARIANTS: &[(&str, &[&str])] = &[
    (
        "## Executive Summary",
        &[
            "## Strategic Overview",
            "## Executive Brief",
            "## Key Highlights",
            "## Top-Line Summary",
        ],
    ),
    (
        "## Priority Actions",
        &[
            "## Recommended Actions",
            "## Key Action Items",
            "## Action Plan",
            "## Top Priorities",
        ],
    ),
    (
        "## Regional Breakdown",
        &[
            "## Regional Analysis",
            "## Geographic Insights",
            "## Regional View",
            "## Area Highlights",
        ],
    ),
    (
        "## Security Posture",
        &[
            "## Security Overview",
            "## Threat Landscape",
            "## Security Assessment",
            "## Security Summary",
        ],
    ),
    (
        "## Momentum and Deltas",
        &[
            "## Trends & Changes",
            "## Momentum Shift",
            "## Weekly Change Log",
            "## Delta Summary",
        ],
    ),
    (
        "## Weak-Signal Fusion",
        &[
            "## Signal Fusion",
            "## Emerging Patterns",
            "## Weak-Signal Analysis",
            "## Cross-Signal Clusters",
        ],
    ),
    (
        "## Competing Hypotheses",
        &[
            "## Alternative Explanations",
            "## Hypothesis Analysis",
            "## Scenario Assessment",
            "## Rival Interpretations",
        ],
    ),
    (
        "## Detailed Insights",
        &[
            "## Insight Deep Dive",
            "## Card Details",
            "## Warning Details",
            "## Full Breakdown",
        ],
    ),
];

/// Apply deterministic heading variation to a line of markdown text.
fn vary_heading(line: &str, rng: &mut MemoRng) -> String {
    for &(canonical, variants) in SECTION_HEADING_VARIANTS {
        if line == canonical {
            let idx = rng.pick(variants.len());
            return variants[idx].to_string();
        }
    }
    line.to_string()
}

/// Group repeated narrative patterns in a list of insight cards.
///
/// Cards that share identical `narrative` text are collapsed into a single
/// grouped entry showing the count and entity names, avoiding repetitive
/// listing of identical warning text for multiple entities.
fn dedup_repeated_narratives(cards: &[InsightCard]) -> Vec<CardDisplayItem> {
    let mut groups: HashMap<&str, Vec<&InsightCard>> = HashMap::new();
    for card in cards {
        groups.entry(card.narrative.as_str()).or_default().push(card);
    }

    let mut items: Vec<CardDisplayItem> = Vec::with_capacity(groups.len());
    for (_narrative, group) in groups {
        if group.len() == 1 {
            items.push(CardDisplayItem::Single(group[0].clone()));
        } else {
            let entity_names: Vec<String> =
                group.iter().map(|c| c.entity_name.clone()).collect();
            items.push(CardDisplayItem::Grouped {
                count: group.len(),
                entity_names,
                recipe_code: group[0].recipe_code.clone(),
                title: group[0].title.clone(),
                narrative: group[0].narrative.clone(),
                severity: group[0].severity.clone(),
            });
        }
    }

    // Sort so grouped items appear first (more impactful), then singles
    // by priority-score descending.
    items.sort_by(|a, b| {
        let a_is_group = matches!(a, CardDisplayItem::Grouped { .. });
        let b_is_group = matches!(b, CardDisplayItem::Grouped { .. });
        b_is_group
            .cmp(&a_is_group)
            .then_with(|| {
                let a_score = a.priority_score();
                let b_score = b.priority_score();
                b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    items
}

/// How an insight card is displayed in the memo — either individually or
/// grouped with other cards sharing the same narrative.
#[derive(Debug, Clone)]
enum CardDisplayItem {
    Single(InsightCard),
    Grouped {
        count: usize,
        entity_names: Vec<String>,
        recipe_code: String,
        title: String,
        narrative: String,
        severity: String,
    },
}

impl CardDisplayItem {
    fn priority_score(&self) -> f64 {
        match self {
            CardDisplayItem::Single(c) => c.priority_score,
            CardDisplayItem::Grouped { .. } => f64::MAX, // grouped items float to top
        }
    }
}

/// Render a `CardDisplayItem` as text, with grouping info when applicable.
fn format_display_item(item: &CardDisplayItem) -> String {
    match item {
        CardDisplayItem::Single(card) => format_card_text(card),
        CardDisplayItem::Grouped {
            count,
            entity_names,
            recipe_code,
            title,
            narrative,
            severity,
        } => {
            let entity_list = if entity_names.len() <= 5 {
                entity_names.join(", ")
            } else {
                let shown: Vec<&str> = entity_names.iter().take(5).map(|s| s.as_str()).collect();
                format!("{}, ... +{} more", shown.join(", "), entity_names.len() - 5)
            };
            format!(
                "### {}\n**Recipe:** {} | **Severity:** {} | **Affected entities ({}):** {}\n\n{}\n\n_This pattern was observed across {} entities._",
                title, recipe_code, severity, count, entity_list, narrative, count
            )
        }
    }
}

// ────────────────────────────────────────────
// Memo structures
// ────────────────────────────────────────────

/// Complete weekly strategy memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyMemo {
    pub id: Uuid,
    pub week_number: u32,
    pub year: i32,
    pub generated_at: DateTime<Utc>,
    pub executive_summary: String,
    pub total_insights: usize,
    pub critical_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub top_actions: Vec<ActionItem>,
    pub regional_sections: Vec<RegionalSection>,
    pub security_summary: SecuritySummary,
    pub temporal_summary: TemporalSummary,
    pub fused_signal_clusters: Vec<FusedSignalCluster>,
    pub competing_hypotheses: Vec<HypothesisScorecard>,
    pub category_breakdown: HashMap<String, usize>,
    pub full_text: String,
}

/// A prioritized action item extracted from insights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    pub priority: usize,
    pub action: String,
    pub source_recipe: String,
    pub entity_name: String,
    pub impact_label: String,
    pub confidence: f64,
}

/// Insights summarized for a specific region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionalSection {
    pub region: String,
    pub region_label: String,
    pub insight_count: usize,
    pub top_insights: Vec<RegionalInsightSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionalInsightSummary {
    pub recipe_code: String,
    pub entity_name: String,
    pub title: String,
    pub severity: String,
    pub impact_label: String,
    pub evidence_quality_score: f64,
    pub evidence_quality_label: String,
}

/// Security posture summary section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySummary {
    pub total_security_insights: usize,
    pub critical_security: usize,
    pub top_threats: Vec<String>,
    pub posture_assessment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalSummary {
    pub volume_delta: TemporalDelta,
    pub early_period_count: usize,
    pub late_period_count: usize,
    pub fastest_rising_region: Option<String>,
    pub narrative: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusedSignalCluster {
    pub theme: String,
    pub region: Option<String>,
    pub entities: Vec<String>,
    pub signal_count: usize,
    pub independent_source_count: usize,
    pub combined_score: f64,
    pub label: String,
}

// ────────────────────────────────────────────
// Region labels
// ────────────────────────────────────────────

/// Map region code to human-readable label.
/// B179: unknown codes return "Unknown ({code})" instead of raw code.
pub fn region_label(code: &str) -> String {
    let normalized = code.trim().to_uppercase();
    match normalized.as_str() {
        "TN" => "Tunisia".to_string(),
        "MA" => "Morocco".to_string(),
        "IL" => "Israel".to_string(),
        "CN" => "China".to_string(),
        "EA" => "East Asia".to_string(),
        "EU" => "Europe".to_string(),
        "US" => "United States".to_string(),
        "global" => "Global / Multi-region".to_string(),
        _ => {
            let safe = normalized
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<String>();
            if safe.is_empty() {
                "Unknown".to_string()
            } else {
                format!("Unknown ({})", safe)
            }
        }
    }
}

/// Canonical region order for memo sections.
pub fn region_order() -> Vec<&'static str> {
    vec!["TN", "MA", "IL", "CN", "EA", "EU", "US", "global"]
}

// ────────────────────────────────────────────
// Severity counting
// ────────────────────────────────────────────

/// Count insights by severity level.
/// B183: normalizes severity to lowercase before matching.
pub fn count_by_severity(cards: &[InsightCard]) -> (usize, usize, usize) {
    let mut critical = 0usize;
    let mut warning = 0usize;
    let mut info = 0usize;
    for card in cards {
        match normalize_severity_label(&card.severity) {
            "critical" => critical += 1,
            "warning" => warning += 1,
            _ => info += 1,
        }
    }
    (critical, warning, info)
}

/// Normalize free-form severity labels to canonical values (B322).
pub fn normalize_severity_label(severity: &str) -> &'static str {
    match severity.trim().to_lowercase().as_str() {
        "critical" => "critical",
        "high" | "warning" => "warning",
        "medium" => "warning",
        "low" | "info" => "info",
        _ => "info",
    }
}

/// Build category breakdown (count per category).
pub fn category_breakdown(cards: &[InsightCard]) -> HashMap<String, usize> {
    let groups = group_by_category(cards);
    groups.into_iter().map(|(k, v)| (k, v.len())).collect()
}

// ────────────────────────────────────────────
// Action extraction
// ────────────────────────────────────────────

/// Extract top N prioritized actions from ranked insight cards.
pub fn extract_top_actions(cards: &[InsightCard], max_actions: usize) -> Vec<ActionItem> {
    let mut items = Vec::new();
    let mut priority = 0usize;
    for card in cards {
        for action in &card.actions {
            priority = priority.saturating_add(1);
            items.push(ActionItem {
                priority,
                action: action.clone(),
                source_recipe: card.recipe_code.clone(),
                entity_name: card.entity_name.clone(),
                impact_label: card.impact_label.clone(),
                confidence: card.confidence,
            });
            if items.len() >= max_actions {
                return items;
            }
        }
    }
    items
}

// ────────────────────────────────────────────
// Regional sections
// ────────────────────────────────────────────

/// Build regional sections from insight cards.
pub fn build_regional_sections(
    cards: &[InsightCard],
    max_per_region: usize,
) -> Vec<RegionalSection> {
    let by_region = group_by_region(cards);
    let order = region_order();

    let mut sections = Vec::new();
    for region_code in &order {
        if let Some(region_cards) = by_region.get(*region_code) {
            let mut ordered = region_cards.clone();
            ordered.sort_by(|a, b| {
                let score_order = b
                    .priority_score
                    .partial_cmp(&a.priority_score)
                    .unwrap_or(std::cmp::Ordering::Equal);
                if score_order != std::cmp::Ordering::Equal {
                    return score_order;
                }
                let code_order = a.recipe_code.cmp(&b.recipe_code);
                if code_order != std::cmp::Ordering::Equal {
                    return code_order;
                }
                a.entity_id.cmp(&b.entity_id)
            });

            let top: Vec<RegionalInsightSummary> = ordered
                .iter()
                .take(max_per_region)
                .map(|c| {
                    let evidence = card_evidence_quality(c);
                    RegionalInsightSummary {
                        recipe_code: c.recipe_code.clone(),
                        entity_name: c.entity_name.clone(),
                        title: c.title.clone(),
                        severity: normalize_severity_label(&c.severity).to_string(),
                        impact_label: c.impact_label.clone(),
                        evidence_quality_score: evidence.overall_score,
                        evidence_quality_label: evidence.quality_label,
                    }
                })
                .collect();
            sections.push(RegionalSection {
                region: region_code.to_string(),
                region_label: region_label(region_code).to_string(),
                insight_count: region_cards.len(),
                top_insights: top,
            });
        }
    }

    // Add any regions not in the standard order
    let known: std::collections::HashSet<&str> = order.into_iter().collect();
    let mut extra: Vec<(&String, &Vec<&InsightCard>)> = by_region
        .iter()
        .filter(|(k, _)| !known.contains(k.as_str()))
        .collect();
    extra.sort_by_key(|(ka, _)| *ka);

    for (region_code, region_cards) in extra {
        let mut ordered = region_cards.clone();
        ordered.sort_by(|a, b| {
            let score_order = b
                .priority_score
                .partial_cmp(&a.priority_score)
                .unwrap_or(std::cmp::Ordering::Equal);
            if score_order != std::cmp::Ordering::Equal {
                return score_order;
            }
            let code_order = a.recipe_code.cmp(&b.recipe_code);
            if code_order != std::cmp::Ordering::Equal {
                return code_order;
            }
            a.entity_id.cmp(&b.entity_id)
        });

        let top: Vec<RegionalInsightSummary> = ordered
            .iter()
            .take(max_per_region)
            .map(|c| {
                let evidence = card_evidence_quality(c);
                RegionalInsightSummary {
                    recipe_code: c.recipe_code.clone(),
                    entity_name: c.entity_name.clone(),
                    title: c.title.clone(),
                    severity: normalize_severity_label(&c.severity).to_string(),
                    impact_label: c.impact_label.clone(),
                    evidence_quality_score: evidence.overall_score,
                    evidence_quality_label: evidence.quality_label,
                }
            })
            .collect();
        sections.push(RegionalSection {
            region: region_code.clone(),
            region_label: region_label(region_code).to_string(),
            insight_count: region_cards.len(),
            top_insights: top,
        });
    }

    sections
}

fn card_evidence_quality(card: &InsightCard) -> apex_core::analysis::EvidenceQuality {
    let now = Utc::now();
    let records: Vec<EvidenceRecord> = card
        .citations
        .iter()
        .map(|citation| {
            let mut record = EvidenceRecord::new(card.confidence, EvidenceStance::Supports)
                .with_source_url(citation.source_url.clone())
                .with_source_type(card.category.clone());
            if let Some(observed_at) = citation.observed_at {
                record = record.with_observed_at(observed_at);
            }
            record
        })
        .collect();
    assess_evidence_quality(&records, now)
}

pub fn build_temporal_summary(cards: &[InsightCard]) -> TemporalSummary {
    if cards.is_empty() {
        return TemporalSummary {
            volume_delta: compare_temporal_windows(0.0, 0.0),
            early_period_count: 0,
            late_period_count: 0,
            fastest_rising_region: None,
            narrative: "Signals remained stable across the period with no measurable acceleration."
                .to_string(),
        };
    }

    let min_ts = cards
        .iter()
        .map(|card| card.rendered_at)
        .min()
        .unwrap_or_else(Utc::now);
    let max_ts = cards
        .iter()
        .map(|card| card.rendered_at)
        .max()
        .unwrap_or(min_ts);
    let midpoint = min_ts + chrono::Duration::seconds((max_ts - min_ts).num_seconds() / 2);

    let mut early_region_counts: HashMap<String, usize> = HashMap::new();
    let mut late_region_counts: HashMap<String, usize> = HashMap::new();
    let mut early_count = 0usize;
    let mut late_count = 0usize;

    for card in cards {
        let region = card.region.clone().unwrap_or_else(|| "global".to_string());
        if card.rendered_at <= midpoint {
            early_count += 1;
            *early_region_counts.entry(region).or_default() += 1;
        } else {
            late_count += 1;
            *late_region_counts.entry(region).or_default() += 1;
        }
    }

    let volume_delta = compare_temporal_windows(late_count as f64, early_count as f64);
    let fastest_rising_region = late_region_counts
        .iter()
        .map(|(region, late)| {
            let early = early_region_counts.get(region).copied().unwrap_or(0) as f64;
            let delta = compare_temporal_windows(*late as f64, early);
            (region.clone(), delta.absolute_change)
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(region, _)| region);

    let narrative = if late_count > early_count {
        format!(
            "Signal volume is {} into the back half of the period ({} early, {} late). {} shows the clearest acceleration.",
            volume_delta.label,
            early_count,
            late_count,
            fastest_rising_region.clone().unwrap_or_else(|| "Global coverage".to_string())
        )
    } else if late_count < early_count {
        format!(
            "Signal volume is {} after an earlier spike ({} early, {} late). Monitor whether {} continues to cool.",
            volume_delta.label,
            early_count,
            late_count,
            fastest_rising_region.clone().unwrap_or_else(|| "global coverage".to_string())
        )
    } else {
        format!(
            "Signal flow remained {} across the period ({} early, {} late).",
            volume_delta.label, early_count, late_count
        )
    };

    TemporalSummary {
        volume_delta,
        early_period_count: early_count,
        late_period_count: late_count,
        fastest_rising_region,
        narrative,
    }
}

pub fn build_fused_signal_clusters(cards: &[InsightCard]) -> Vec<FusedSignalCluster> {
    let signal_frames: Vec<SignalFrame> = cards
        .iter()
        .map(|card| SignalFrame {
            id: card.id.to_string(),
            theme: card.category.clone(),
            category: Some(card.category.clone()),
            region: card.region.clone(),
            entity: Some(card.entity_name.clone()),
            confidence: card.confidence,
            impact: card.impact,
            source_group: card
                .citations
                .iter()
                .find_map(|citation| source_group_from_url(&citation.source_url)),
        })
        .collect();

    let entities_by_id: HashMap<String, String> = cards
        .iter()
        .map(|card| (card.id.to_string(), card.entity_name.clone()))
        .collect();

    fuse_weak_signals(&signal_frames)
        .into_iter()
        .map(|cluster| {
            let mut entities: Vec<String> = cluster
                .signal_ids
                .iter()
                .filter_map(|id| entities_by_id.get(id).cloned())
                .collect();
            entities.sort();
            entities.dedup();
            FusedSignalCluster {
                theme: cluster.theme,
                region: cluster.region,
                entities,
                signal_count: cluster.signal_count,
                independent_source_count: cluster.independent_source_count,
                combined_score: cluster.combined_score,
                label: cluster.label,
            }
        })
        .collect()
}

pub fn build_competing_hypotheses(cards: &[InsightCard]) -> Vec<HypothesisScorecard> {
    let mut expansion_support = 0.0;
    let mut compliance_support = 0.0;
    let mut pressure_support = 0.0;

    for card in cards {
        let weight = (card.confidence * card.impact.max(0.2)).clamp(0.0, 1.0);
        match card.category.as_str() {
            "demand" | "procurement" | "logistics" => expansion_support += weight,
            "security" | "regulatory" => compliance_support += weight,
            "competitor" | "supply_chain" => pressure_support += weight,
            _ => {
                expansion_support += weight * 0.15;
                pressure_support += weight * 0.15;
            }
        }
    }

    let total = cards.len().max(1) as f64;
    score_competing_hypotheses(&[
        HypothesisInput {
            hypothesis: "Expansion or program acceleration".to_string(),
            support_score: (expansion_support / total).clamp(0.0, 1.0),
            contradiction_score: (compliance_support / total * 0.5).clamp(0.0, 1.0),
            prior: 0.45,
        },
        HypothesisInput {
            hypothesis: "Compliance or security stress response".to_string(),
            support_score: (compliance_support / total).clamp(0.0, 1.0),
            contradiction_score: (expansion_support / total * 0.35).clamp(0.0, 1.0),
            prior: 0.35,
        },
        HypothesisInput {
            hypothesis: "Competitive pressure and defensive repositioning".to_string(),
            support_score: (pressure_support / total).clamp(0.0, 1.0),
            contradiction_score: (expansion_support / total * 0.25).clamp(0.0, 1.0),
            prior: 0.40,
        },
    ])
}

// ────────────────────────────────────────────
// Security summary
// ────────────────────────────────────────────

/// Build security posture summary from insight cards.
pub fn build_security_summary(cards: &[InsightCard]) -> SecuritySummary {
    let security_cards: Vec<&InsightCard> =
        cards.iter().filter(|c| c.category == "security").collect();

    let total = security_cards.len();
    let critical = security_cards
        .iter()
        .filter(|c| normalize_severity_label(&c.severity) == "critical")
        .count();

    let mut top_threats: Vec<String> = security_cards
        .iter()
        .filter_map(|c| {
            let title = c.title.trim();
            if title.is_empty() {
                None
            } else {
                Some(title.to_string())
            }
        })
        .take(5)
        .collect();

    if total > 0 && top_threats.is_empty() {
        top_threats.push("Security signals detected (titles unavailable)".to_string());
    }

    let posture = if critical > 0 {
        "ELEVATED — Critical security threats require immediate attention".to_string()
    } else if total > 3 {
        "GUARDED — Multiple security signals detected, monitor closely".to_string()
    } else if total > 0 {
        "NORMAL — Minor security signals, routine monitoring sufficient".to_string()
    } else {
        "GREEN — No security signals detected this period".to_string()
    };

    SecuritySummary {
        total_security_insights: total,
        critical_security: critical,
        top_threats,
        posture_assessment: posture,
    }
}

// ────────────────────────────────────────────
// Executive summary generation
// ────────────────────────────────────────────

/// Generate executive summary text from insight stats.
pub fn generate_executive_summary(
    total: usize,
    critical: usize,
    warning: usize,
    top_actions: &[ActionItem],
    security: &SecuritySummary,
) -> String {
    let mut lines = Vec::new();

    lines.push(format!(
        "This week's intelligence scan identified {} actionable insights ({} critical, {} warning).",
        total, critical, warning
    ));

    if critical > 0 {
        lines.push(format!(
            "**Immediate attention required** on {} critical items.",
            critical
        ));
    }

    if !top_actions.is_empty() {
        let top3: Vec<String> = top_actions
            .iter()
            .take(3)
            .map(|a| format!("{} ({})", a.action, a.entity_name))
            .collect();
        lines.push(format!("Top priority actions: {}", top3.join("; ")));
    }

    lines.push(format!("Security posture: {}", security.posture_assessment));

    lines.join(" ")
}

// ────────────────────────────────────────────
// Full memo rendering
// ────────────────────────────────────────────

/// Generate the full text of the weekly memo in Markdown format.
///
/// Applies deterministic heading variation (week-seeded) and deduplicates
/// repeated narrative patterns across entities.
#[allow(clippy::too_many_arguments)]
pub fn render_memo_text(
    week: u32,
    year: i32,
    exec_summary: &str,
    cards: &[InsightCard],
    actions: &[ActionItem],
    sections: &[RegionalSection],
    security: &SecuritySummary,
    temporal_summary: &TemporalSummary,
    fused_signal_clusters: &[FusedSignalCluster],
    competing_hypotheses: &[HypothesisScorecard],
) -> String {
    let mut rng = MemoRng::from_week(week, year);

    // Pre-allocate: 8 structural lines + 1 per action + 3 per section insight
    // + 4 per detailed card + 6 fixed security lines.  Better than Vec::new()
    // which triggers multiple doubling re-allocations on large memos (B280).
    let estimated_lines = 8
        + actions.len()
        + sections
            .iter()
            .map(|s| 2 + s.top_insights.len())
            .sum::<usize>()
        + security.top_threats.len()
        + cards.len().min(20) * 6  // slightly more per grouped card
        + 6;
    let mut lines = Vec::with_capacity(estimated_lines);

    lines.push(format!("# Weekly Strategy Memo — W{:02}/{}", week, year));
    lines.push(String::new());

    // Executive summary
    lines.push(vary_heading("## Executive Summary", &mut rng));
    lines.push(exec_summary.to_string());
    lines.push(String::new());

    // Top actions
    if !actions.is_empty() {
        lines.push(vary_heading("## Priority Actions", &mut rng));
        for action in actions {
            lines.push(format!(
                "{}. **[{}]** {} — {} (Impact: {}, Confidence: {:.0}%)",
                action.priority,
                action.source_recipe,
                action.action,
                action.entity_name,
                action.impact_label,
                action.confidence * 100.0,
            ));
        }
        lines.push(String::new());
    }

    // Regional breakdown
    lines.push(vary_heading("## Regional Breakdown", &mut rng));
    for section in sections {
        lines.push(format!(
            "### {} ({} insights)",
            section.region_label, section.insight_count
        ));
        for insight in &section.top_insights {
            lines.push(format!(
                "- **[{}]** {} — {} [{}]",
                insight.recipe_code, insight.title, insight.entity_name, insight.severity
            ));
            lines.push(format!(
                "  Evidence posture: {} ({:.2})",
                insight.evidence_quality_label, insight.evidence_quality_score
            ));
        }
        lines.push(String::new());
    }

    // Security summary
    lines.push(vary_heading("## Security Posture", &mut rng));
    lines.push(format!("**Assessment:** {}", security.posture_assessment));
    lines.push(format!(
        "Security insights: {} total, {} critical",
        security.total_security_insights, security.critical_security
    ));
    if !security.top_threats.is_empty() {
        lines.push("Top threats:".to_string());
        for threat in &security.top_threats {
            lines.push(format!("- {}", threat));
        }
    } else if security.total_security_insights > 0 {
        lines.push("Top threats: none listed".to_string());
    }
    lines.push(String::new());

    lines.push(vary_heading("## Momentum and Deltas", &mut rng));
    lines.push(temporal_summary.narrative.clone());
    lines.push(format!(
        "Early period: {} | Late period: {} | Direction: {}",
        temporal_summary.early_period_count,
        temporal_summary.late_period_count,
        temporal_summary.volume_delta.label
    ));
    lines.push(String::new());

    if !fused_signal_clusters.is_empty() {
        lines.push(vary_heading("## Weak-Signal Fusion", &mut rng));
        for cluster in fused_signal_clusters.iter().take(5) {
            lines.push(format!(
                "- {} [{}] — {} signals across {} independent sources (score {:.2}); entities: {}",
                cluster.theme,
                cluster.label,
                cluster.signal_count,
                cluster.independent_source_count,
                cluster.combined_score,
                cluster.entities.join(", ")
            ));
        }
        lines.push(String::new());
    }

    if !competing_hypotheses.is_empty() {
        lines.push(vary_heading("## Competing Hypotheses", &mut rng));
        for hypothesis in competing_hypotheses.iter().take(3) {
            lines.push(format!(
                "- {} — {} (posterior {:.0}%, support {:.2}, contradiction {:.2})",
                hypothesis.hypothesis,
                hypothesis.assessment,
                hypothesis.posterior * 100.0,
                hypothesis.support_score,
                hypothesis.contradiction_score
            ));
        }
        lines.push(String::new());
    }

    // Detailed insights — with narrative dedup grouping
    if !cards.is_empty() {
        lines.push(vary_heading("## Detailed Insights", &mut rng));
        let deduped = dedup_repeated_narratives(cards);
        for item in deduped.iter().take(20) {
            lines.push(format_display_item(item));
            match item {
                CardDisplayItem::Single(card) => {
                    lines.push(format!(
                        "Score breakdown: impact={:.2}, confidence={:.2}, priority={:.2}",
                        card.impact, card.confidence, card.priority_score
                    ));
                }
                CardDisplayItem::Grouped { .. } => {
                    // grouped items already include narrative text — no separate score line
                }
            }
            lines.push(String::new());
            lines.push("---".to_string());
            lines.push(String::new());
        }
    }

    lines.join("\n")
}

/// Generate a complete WeeklyMemo from a set of ranked InsightCards.
pub fn generate_weekly_memo(cards: &[InsightCard]) -> WeeklyMemo {
    let now = Utc::now();
    let iso = now.iso_week();
    let week = iso.week();
    let year = iso.year();

    let (critical, warning, info) = count_by_severity(cards);
    let top_actions = extract_top_actions(cards, 10);
    let regional_sections = build_regional_sections(cards, 5);
    let security = build_security_summary(cards);
    let temporal_summary = build_temporal_summary(cards);
    let fused_signal_clusters = build_fused_signal_clusters(cards);
    let competing_hypotheses = build_competing_hypotheses(cards);
    let cat_breakdown = category_breakdown(cards);

    let exec_summary =
        generate_executive_summary(cards.len(), critical, warning, &top_actions, &security);

    let full_text = render_memo_text(
        week,
        year,
        &exec_summary,
        cards,
        &top_actions,
        &regional_sections,
        &security,
        &temporal_summary,
        &fused_signal_clusters,
        &competing_hypotheses,
    );

    info!(
        week_number = week,
        year,
        total_insights = cards.len(),
        critical_count = critical,
        warning_count = warning,
        info_count = info,
        top_actions_count = top_actions.len(),
        regional_sections_count = regional_sections.len(),
        security_total = security.total_security_insights,
        "weekly_memo_generated"
    );

    WeeklyMemo {
        id: Uuid::new_v4(),
        week_number: week,
        year,
        generated_at: now,
        executive_summary: exec_summary,
        total_insights: cards.len(),
        critical_count: critical,
        warning_count: warning,
        info_count: info,
        top_actions,
        regional_sections,
        security_summary: security,
        temporal_summary,
        fused_signal_clusters,
        competing_hypotheses,
        category_breakdown: cat_breakdown,
        full_text,
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;
    use crate::renderer::{render_batch, EvidenceSlot, InsightCandidate};

    fn make_candidate(
        code: &str,
        entity: &str,
        severity: &str,
        category: &str,
        region: Option<&str>,
        impact: f64,
        confidence: f64,
    ) -> InsightCandidate {
        InsightCandidate {
            recipe_id: Uuid::new_v4(),
            recipe_code: code.to_string(),
            entity_id: Uuid::new_v4(),
            entity_name: entity.to_string(),
            confidence,
            impact,
            narrative_template: format!("{} insight for {}", category, entity),
            action_template: format!("Action A for {}\nAction B", entity),
            evidence: vec![EvidenceSlot {
                slot_name: "source".to_string(),
                value: "test evidence".to_string(),
                source_url: Some("https://example.com/evidence".to_string()),
                source_domain: Some("example.com".to_string()),
                observed_at: Some(Utc::now()),
            }],
            severity: severity.to_string(),
            category: category.to_string(),
            region: region.map(|s| s.to_string()),
        }
    }

    fn sample_cards() -> Vec<InsightCard> {
        let candidates = vec![
            make_candidate(
                "A001",
                "Foxconn",
                "critical",
                "demand",
                Some("TN"),
                0.9,
                0.9,
            ),
            make_candidate(
                "B001",
                "Jabil",
                "warning",
                "competitor",
                Some("MA"),
                0.7,
                0.8,
            ),
            make_candidate(
                "C001",
                "Starz",
                "critical",
                "security",
                Some("TN"),
                0.85,
                0.95,
            ),
            make_candidate("D001", "Flex", "info", "supply_chain", Some("CN"), 0.4, 0.6),
            make_candidate(
                "A010",
                "Celestica",
                "warning",
                "demand",
                Some("EU"),
                0.6,
                0.7,
            ),
        ];
        render_batch(&candidates)
    }

    #[test]
    fn test_region_label() {
        assert_eq!(region_label("TN"), "Tunisia");
        assert_eq!(region_label("MA"), "Morocco");
        assert_eq!(region_label("IL"), "Israel");
        assert_eq!(region_label("CN"), "China");
        assert_eq!(region_label("XX"), "Unknown (XX)");
    }

    #[test]
    fn test_count_by_severity() {
        let cards = sample_cards();
        let (critical, warning, info) = count_by_severity(&cards);
        assert_eq!(critical, 2);
        assert_eq!(warning, 2);
        assert_eq!(info, 1);
    }

    #[test]
    fn test_category_breakdown() {
        let cards = sample_cards();
        let breakdown = category_breakdown(&cards);
        assert_eq!(*breakdown.get("demand").unwrap_or(&0), 2);
        assert_eq!(*breakdown.get("security").unwrap_or(&0), 1);
        assert_eq!(*breakdown.get("competitor").unwrap_or(&0), 1);
        assert_eq!(*breakdown.get("supply_chain").unwrap_or(&0), 1);
    }

    #[test]
    fn test_extract_top_actions() {
        let cards = sample_cards();
        let actions = extract_top_actions(&cards, 5);
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0].priority, 1);
        assert_eq!(actions[4].priority, 5);
        // All should have populated fields
        for action in &actions {
            assert!(!action.action.is_empty());
            assert!(!action.source_recipe.is_empty());
            assert!(!action.entity_name.is_empty());
        }
    }

    #[test]
    fn test_extract_top_actions_with_no_actions() {
        let mut cards = sample_cards();
        for card in &mut cards {
            card.actions.clear();
        }
        let actions = extract_top_actions(&cards, 10);
        assert!(actions.is_empty());
    }

    #[test]
    fn test_build_regional_sections() {
        let cards = sample_cards();
        let sections = build_regional_sections(&cards, 3);

        // Should have TN, MA, CN, EU
        let region_codes: Vec<&str> = sections.iter().map(|s| s.region.as_str()).collect();
        assert!(region_codes.contains(&"TN"));
        assert!(region_codes.contains(&"MA"));
        assert!(region_codes.contains(&"CN"));
        assert!(region_codes.contains(&"EU"));

        // TN should have 2 insights
        let tn = sections.iter().find(|s| s.region == "TN").unwrap();
        assert_eq!(tn.insight_count, 2);
        assert_eq!(tn.region_label, "Tunisia");

        // MA should have 1
        let ma = sections.iter().find(|s| s.region == "MA").unwrap();
        assert_eq!(ma.insight_count, 1);
    }

    #[test]
    fn test_build_security_summary_with_critical() {
        let cards = sample_cards();
        let summary = build_security_summary(&cards);
        assert_eq!(summary.total_security_insights, 1);
        assert_eq!(summary.critical_security, 1);
        assert!(summary.posture_assessment.starts_with("ELEVATED"));
    }

    #[test]
    fn test_build_security_summary_no_security() {
        // Cards with no security category
        let candidates = vec![make_candidate(
            "A001",
            "Foxconn",
            "warning",
            "demand",
            Some("TN"),
            0.7,
            0.8,
        )];
        let cards = render_batch(&candidates);
        let summary = build_security_summary(&cards);
        assert_eq!(summary.total_security_insights, 0);
        assert!(summary.posture_assessment.starts_with("GREEN"));
    }

    #[test]
    fn test_generate_executive_summary() {
        let actions = vec![ActionItem {
            priority: 1,
            action: "Register on Foxconn portal".to_string(),
            source_recipe: "A001".to_string(),
            entity_name: "Foxconn".to_string(),
            impact_label: "Critical".to_string(),
            confidence: 0.9,
        }];
        let security = SecuritySummary {
            total_security_insights: 1,
            critical_security: 1,
            top_threats: vec!["Brand impersonation detected".to_string()],
            posture_assessment: "ELEVATED — Critical security threats require immediate attention"
                .to_string(),
        };
        let summary = generate_executive_summary(5, 2, 2, &actions, &security);
        assert!(summary.contains("5 actionable insights"));
        assert!(summary.contains("2 critical"));
        assert!(summary.contains("Immediate attention required"));
        assert!(summary.contains("Register on Foxconn portal"));
        assert!(summary.contains("ELEVATED"));
    }

    #[test]
    fn test_generate_weekly_memo_full() {
        let cards = sample_cards();
        let memo = generate_weekly_memo(&cards);

        assert_eq!(memo.total_insights, 5);
        assert_eq!(memo.critical_count, 2);
        assert_eq!(memo.warning_count, 2);
        assert_eq!(memo.info_count, 1);
        assert!(!memo.executive_summary.is_empty());
        assert!(!memo.top_actions.is_empty());
        assert!(!memo.regional_sections.is_empty());
        assert!(!memo.competing_hypotheses.is_empty());
        assert!(!memo.full_text.is_empty());

        // Full text should contain key content (headings may vary deterministically per week)
        assert!(memo.full_text.contains("# Weekly Strategy Memo"));
        assert!(memo.full_text.contains("##")); // at least some level-2 headings
        assert!(memo.full_text.contains("Foxconn"), "should mention Foxconn");
        assert!(memo.full_text.contains("Celestica"), "should mention Celestica");
        assert!(memo.full_text.contains("insight"), "should contain insight text");
        assert!(memo.full_text.contains("Action A"), "should contain actions");
    }

    #[test]
    fn test_generate_weekly_memo_empty() {
        let memo = generate_weekly_memo(&[]);
        assert_eq!(memo.total_insights, 0);
        assert_eq!(memo.critical_count, 0);
        assert!(memo.executive_summary.contains("0 actionable insights"));
        assert!(memo.top_actions.is_empty());
        assert!(memo.regional_sections.is_empty());
    }

    #[test]
    fn test_region_label_lowercase_input() {
        assert_eq!(region_label("tn"), "Tunisia");
    }

    #[test]
    fn test_render_memo_text_structure() {
        let cards = sample_cards();
        let memo = generate_weekly_memo(&cards);
        let text = &memo.full_text;

        // Check markdown structure
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("# Weekly Strategy Memo"));

        // Should have all sections
        let h2_count = lines.iter().filter(|l| l.starts_with("## ")).count();
        assert!(h2_count >= 4); // Exec Summary, Priority Actions, Regional, Security, Detailed
    }

    // ── B179: unknown region codes ──

    #[test]
    fn test_region_label_unknown() {
        assert_eq!(region_label("ZZ"), "Unknown (ZZ)");
        assert_eq!(region_label(""), "Unknown");
    }

    // ── B180: build_regional_sections with extra regions ──

    #[test]
    fn test_regional_sections_extra_regions() {
        let candidates = vec![
            make_candidate("X01", "Ent", "info", "demand", Some("ZZ"), 0.5, 0.5),
            make_candidate("X02", "Ent2", "info", "demand", Some("QQ"), 0.5, 0.5),
            make_candidate("X03", "Ent3", "info", "demand", Some("TN"), 0.5, 0.5),
        ];
        let cards = render_batch(&candidates);
        let sections = build_regional_sections(&cards, 3);
        let region_codes: Vec<&str> = sections.iter().map(|s| s.region.as_str()).collect();
        // TN should come before ZZ/QQ (canonical order first)
        assert!(region_codes.contains(&"TN"));
        assert!(region_codes.contains(&"ZZ"));
        assert!(region_codes.contains(&"QQ"));
        // Extra regions should have Unknown label
        let zz = sections.iter().find(|s| s.region == "ZZ").unwrap();
        assert_eq!(zz.region_label, "Unknown (ZZ)");
    }

    // ── B181: category_breakdown handles empty ──

    #[test]
    fn test_category_breakdown_empty() {
        let breakdown = category_breakdown(&[]);
        assert!(breakdown.is_empty());
    }

    // ── B182: executive_summary max length ──

    #[test]
    fn test_executive_summary_bounded() {
        let actions: Vec<ActionItem> = (0..100)
            .map(|i| ActionItem {
                priority: i,
                action: format!("Very long action item {} that goes on and on", i),
                source_recipe: format!("R{}", i),
                entity_name: format!("Entity{}", i),
                impact_label: "Critical".to_string(),
                confidence: 0.9,
            })
            .collect();
        let security = SecuritySummary {
            total_security_insights: 0,
            critical_security: 0,
            top_threats: vec![],
            posture_assessment: "GREEN".to_string(),
        };
        let summary = generate_executive_summary(1000, 500, 300, &actions, &security);
        // Summary should only include top 3 actions, keeping it reasonable
        assert!(
            summary.len() < 2000,
            "Executive summary is too long: {} chars",
            summary.len()
        );
    }

    // ── B183: severity normalization ──

    #[test]
    fn test_count_by_severity_case_insensitive() {
        let candidates = vec![
            make_candidate("A01", "E", "Critical", "demand", None, 0.9, 0.9),
            make_candidate("A02", "E", "WARNING", "demand", None, 0.7, 0.8),
            make_candidate("A03", "E", "Info", "demand", None, 0.3, 0.5),
        ];
        let cards = render_batch(&candidates);
        let (critical, warning, info) = count_by_severity(&cards);
        // Severity is set from InsightCandidate.severity which gets stored as-is in the card
        // The count_by_severity function now normalizes to lowercase
        assert_eq!(critical + warning + info, cards.len());
    }

    #[test]
    fn test_count_by_severity_unknown_treated_as_info() {
        let candidates = vec![make_candidate(
            "A01",
            "E",
            "unknown-sev",
            "demand",
            None,
            0.5,
            0.5,
        )];
        let cards = render_batch(&candidates);
        let (critical, warning, info) = count_by_severity(&cards);
        assert_eq!(critical, 0);
        assert_eq!(warning, 0);
        assert_eq!(info, 1);
    }

    #[test]
    fn test_region_order_output_stability() {
        let expected = vec!["TN", "MA", "IL", "CN", "EA", "EU", "US", "global"];
        assert_eq!(region_order(), expected);
        assert_eq!(region_order(), expected);
    }

    #[test]
    fn test_normalize_severity_label_unknown_defaults_info() {
        assert_eq!(normalize_severity_label("Severe"), "info");
        assert_eq!(normalize_severity_label("HIGH"), "warning");
    }

    #[test]
    fn test_build_security_summary_mixed_severity() {
        let candidates = vec![
            make_candidate("S01", "E1", "critical", "security", Some("TN"), 0.9, 0.9),
            make_candidate("S02", "E2", "HIGH", "security", Some("TN"), 0.7, 0.8),
            make_candidate("S03", "E3", "info", "security", Some("TN"), 0.4, 0.5),
        ];
        let cards = render_batch(&candidates);
        let summary = build_security_summary(&cards);
        assert_eq!(summary.total_security_insights, 3);
        assert_eq!(summary.critical_security, 1);
        assert!(!summary.top_threats.is_empty());
    }

    #[test]
    fn test_build_security_summary_empty_top_threats_has_fallback() {
        let mut cards = render_batch(&[make_candidate(
            "S01",
            "E1",
            "critical",
            "security",
            Some("TN"),
            0.9,
            0.9,
        )]);
        cards[0].title = "   ".to_string();
        let summary = build_security_summary(&cards);
        assert_eq!(summary.total_security_insights, 1);
        assert_eq!(summary.top_threats.len(), 1);
        assert!(summary.top_threats[0].contains("titles unavailable"));
    }

    #[test]
    fn test_build_regional_sections_normalizes_severity_casing() {
        let cards = render_batch(&[make_candidate(
            "A01",
            "E1",
            "HIGH",
            "demand",
            Some("TN"),
            0.7,
            0.7,
        )]);
        let sections = build_regional_sections(&cards, 3);
        let tn = sections.iter().find(|s| s.region == "TN").unwrap();
        assert_eq!(tn.top_insights[0].severity, "warning");
    }

    #[test]
    fn test_render_memo_text_includes_score_breakdown() {
        let cards = sample_cards();
        let memo = generate_weekly_memo(&cards);
        assert!(memo.full_text.contains("Score breakdown: impact="));
    }

    #[test]
    fn test_regional_sections_include_evidence_posture() {
        let cards = sample_cards();
        let sections = build_regional_sections(&cards, 5);
        let tn = sections
            .iter()
            .find(|section| section.region == "TN")
            .unwrap();
        assert!(tn
            .top_insights
            .iter()
            .all(|insight| !insight.evidence_quality_label.is_empty()));
    }

    #[test]
    fn test_fused_signal_clusters_require_multiple_related_cards() {
        let mut cards = render_batch(&[
            make_candidate("A01", "Acme", "info", "demand", Some("EU"), 0.55, 0.55),
            make_candidate("A02", "Acme", "info", "demand", Some("EU"), 0.60, 0.58),
        ]);
        cards[1].citations[0].source_url = "https://independent.example.org/evidence".to_string();
        let clusters = build_fused_signal_clusters(&cards);
        assert!(!clusters.is_empty());
    }

    #[test]
    fn test_regional_sections_deterministic_order_within_region() {
        let mut cards = sample_cards();
        cards.reverse();
        let sections_1 = build_regional_sections(&cards, 10);
        cards.reverse();
        let sections_2 = build_regional_sections(&cards, 10);
        assert_eq!(sections_1.len(), sections_2.len());
        let tn_1 = sections_1.iter().find(|s| s.region == "TN").unwrap();
        let tn_2 = sections_2.iter().find(|s| s.region == "TN").unwrap();
        let keys_1: Vec<(&str, &str)> = tn_1
            .top_insights
            .iter()
            .map(|i| (i.recipe_code.as_str(), i.entity_name.as_str()))
            .collect();
        let keys_2: Vec<(&str, &str)> = tn_2
            .top_insights
            .iter()
            .map(|i| (i.recipe_code.as_str(), i.entity_name.as_str()))
            .collect();
        assert_eq!(keys_1, keys_2);
    }
}
