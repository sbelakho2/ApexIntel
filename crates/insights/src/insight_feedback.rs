//! Insight Feedback Loop Module
//!
//! This module implements a feedback loop to track insight quality and enable
//! continuous improvement. This fixes Issue #7 (No Feedback Loop).
//!
//! Key capabilities:
//! 1. Insight action tracking - did users act on insights?
//! 2. False positive/negative tracking - learn from mistakes
//! 3. Recipe performance tuning - auto-adjust thresholds
//! 4. Insight fatigue detection - suppress repetitive insights

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Feedback types for insights
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InsightFeedback {
    /// User bookmarked this insight
    Bookmarked,
    /// User acted on this insight
    Actioned,
    /// User dismissed this insight (not useful)
    Dismissed,
    /// User marked as false positive
    FalsePositive,
    /// User marked as false negative (missed something)
    FalseNegative,
    /// User confirmed as true positive
    TruePositive,
    /// User marked as relevant
    Relevant,
    /// User marked as irrelevant
    Irrelevant,
    /// Insight was viewed but no action
    Viewed,
    /// No feedback received
    None,
}

/// Feedback record for an insight
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightFeedbackRecord {
    pub insight_id: String,
    pub entity_id: String,
    pub recipe_code: String,
    pub feedback: InsightFeedback,
    pub timestamp: i64,
    pub user_id: Option<String>,
    pub notes: Option<String>,
}

/// Recipe performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePerformance {
    pub recipe_code: String,
    pub total_firings: u32,
    pub true_positives: u32,
    pub false_positives: u32,
    pub false_negatives: u32,
    pub bookmark_rate: f64,
    pub action_rate: f64,
    pub dismiss_rate: f64,
    pub avg_relevance_score: f64,
    pub last_updated: i64,
}

impl RecipePerformance {
    /// Calculate precision
    pub fn precision(&self) -> f64 {
        let tp = self.true_positives as f64;
        let fp = self.false_positives as f64;
        if tp + fp == 0.0 {
            0.5 // Neutral prior
        } else {
            tp / (tp + fp)
        }
    }

    /// Calculate recall (estimated)
    pub fn recall(&self) -> f64 {
        let tp = self.true_positives as f64;
        let fn_ = self.false_negatives as f64;
        if tp + fn_ == 0.0 {
            0.5
        } else {
            tp / (tp + fn_)
        }
    }

    /// Calculate F1 score
    pub fn f1_score(&self) -> f64 {
        let p = self.precision();
        let r = self.recall();
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * (p * r) / (p + r)
        }
    }

    /// Get recommendation for recipe
    pub fn recommendation(&self) -> RecipeRecommendation {
        let f1 = self.f1_score();
        let precision = self.precision();
        let recall = self.recall();

        if f1 > 0.8 {
            RecipeRecommendation::KeepActive
        } else if f1 > 0.5 {
            if precision < recall {
                RecipeRecommendation::IncreaseThreshold
            } else {
                RecipeRecommendation::AddMoreSignals
            }
        } else if f1 > 0.2 {
            RecipeRecommendation::NeedsReview
        } else {
            RecipeRecommendation::Retire
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecipeRecommendation {
    KeepActive,
    IncreaseThreshold,
    DecreaseThreshold,
    AddMoreSignals,
    NeedsReview,
    Retire,
}

/// Insight fatigue detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FatigueAnalysis {
    pub entity_id: String,
    pub recipe_code: String,
    pub firing_count: u32,
    pub unique_content_count: u32,
    pub repetition_rate: f64,
    pub is_fatigued: bool,
    pub recommended_action: FatigueAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FatigueAction {
    Suppress,
    ReduceFrequency,
    RequireNewContent,
    NoAction,
}

/// Main feedback tracker
pub struct InsightFeedbackTracker {
    /// Feedback records keyed by insight_id
    feedback_records: HashMap<String, Vec<InsightFeedbackRecord>>,
    /// Recipe performance metrics
    recipe_performance: HashMap<String, RecipePerformance>,
    /// Entity-recipe firing history
    firing_history: HashMap<String, Vec<FiringRecord>>,
    /// Stale insight hashes
    stale_hashes: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // fields stored for tracking/auditing, insight_hash actively read
struct FiringRecord {
    pub timestamp: i64,
    pub insight_hash: String,
    pub entity_id: String,
    pub recipe_code: String,
}

impl Default for InsightFeedbackTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl InsightFeedbackTracker {
    /// Create a new feedback tracker
    pub fn new() -> Self {
        Self {
            feedback_records: HashMap::new(),
            recipe_performance: HashMap::new(),
            firing_history: HashMap::new(),
            stale_hashes: HashMap::new(),
        }
    }

    /// Record feedback for an insight
    pub fn record_feedback(&mut self, record: InsightFeedbackRecord) {
        // Add to feedback records
        let entry = self
            .feedback_records
            .entry(record.insight_id.clone())
            .or_default();
        entry.push(record.clone());

        // Update recipe performance
        self.update_recipe_performance(&record);
    }

    /// Update recipe performance based on feedback
    fn update_recipe_performance(&mut self, record: &InsightFeedbackRecord) {
        let perf = self
            .recipe_performance
            .entry(record.recipe_code.clone())
            .or_insert_with(|| RecipePerformance {
                recipe_code: record.recipe_code.clone(),
                total_firings: 0,
                true_positives: 0,
                false_positives: 0,
                false_negatives: 0,
                bookmark_rate: 0.0,
                action_rate: 0.0,
                dismiss_rate: 0.0,
                avg_relevance_score: 0.5,
                last_updated: Utc::now().timestamp(),
            });

        perf.total_firings += 1;
        perf.last_updated = Utc::now().timestamp();

        match record.feedback {
            InsightFeedback::FalsePositive => {
                perf.false_positives += 1;
            }
            InsightFeedback::TruePositive
            | InsightFeedback::Actioned
            | InsightFeedback::Relevant => {
                perf.true_positives += 1;
            }
            InsightFeedback::FalseNegative => {
                perf.false_negatives += 1;
            }
            InsightFeedback::Dismissed | InsightFeedback::Irrelevant => {
                // These don't directly affect TP/FP but indicate quality issues
            }
            InsightFeedback::Bookmarked => {
                // Bookmarks are positive signals
                let current = perf.bookmark_rate * (perf.total_firings - 1) as f64;
                perf.bookmark_rate = (current + 1.0) / perf.total_firings as f64;
            }
            _ => {}
        }
    }

    /// Record an insight firing
    pub fn record_firing(&mut self, entity_id: &str, recipe_code: &str, insight_hash: &str) {
        let key = format!("{}:{}", entity_id, recipe_code);

        let entry = self.firing_history.entry(key.clone()).or_default();

        entry.push(FiringRecord {
            timestamp: Utc::now().timestamp(),
            insight_hash: insight_hash.to_string(),
            entity_id: entity_id.to_string(),
            recipe_code: recipe_code.to_string(),
        });

        // Track hash for staleness detection
        let stale_entry = self.stale_hashes.entry(key.clone()).or_default();

        stale_entry.push(insight_hash.to_string());

        // Keep only last 20 hashes
        if stale_entry.len() > 20 {
            stale_entry.remove(0);
        }
    }

    /// Get recipe performance
    pub fn get_recipe_performance(&self, recipe_code: &str) -> Option<&RecipePerformance> {
        self.recipe_performance.get(recipe_code)
    }

    /// Get all recipe performances sorted by F1
    pub fn get_all_performances(&self) -> Vec<&RecipePerformance> {
        let mut perfs: Vec<&RecipePerformance> = self.recipe_performance.values().collect();
        perfs.sort_by(|a, b| {
            let f1_a = a.f1_score();
            let f1_b = b.f1_score();
            f1_b.partial_cmp(&f1_a).unwrap_or(std::cmp::Ordering::Equal)
        });
        perfs
    }

    /// Get recipe recommendations
    pub fn get_recipe_recommendations(&self) -> Vec<(String, RecipeRecommendation)> {
        self.recipe_performance
            .iter()
            .map(|(code, perf)| (code.clone(), perf.recommendation()))
            .collect()
    }

    /// Detect insight fatigue for entity-recipe combinations
    pub fn detect_fatigue(&self, entity_id: &str, recipe_code: &str) -> Option<FatigueAnalysis> {
        let key = format!("{}:{}", entity_id, recipe_code);

        let history = self.firing_history.get(&key)?;
        if history.is_empty() {
            return None;
        }

        let firing_count = history.len() as u32;

        // Use stale_hashes sliding window for efficient recent uniqueness,
        // falling back to full history if the window isn't populated yet.
        let unique_content_count = if let Some(recent_hashes) = self.stale_hashes.get(&key) {
            let unique: std::collections::HashSet<_> = recent_hashes.iter().collect();
            unique.len() as u32
        } else {
            let unique_hashes: std::collections::HashSet<_> =
                history.iter().map(|r| &r.insight_hash).collect();
            unique_hashes.len() as u32
        };

        // Calculate repetition rate (lower = more repetitive)
        let repetition_rate = if firing_count > 0 {
            unique_content_count as f64 / firing_count as f64
        } else {
            1.0
        };

        // Determine if fatigued
        let is_fatigued = firing_count >= 5 && repetition_rate < 0.5;

        // Determine recommended action
        let recommended_action = if is_fatigued {
            if firing_count >= 10 && repetition_rate < 0.2 {
                FatigueAction::Suppress
            } else if firing_count >= 7 {
                FatigueAction::ReduceFrequency
            } else {
                FatigueAction::RequireNewContent
            }
        } else {
            FatigueAction::NoAction
        };

        Some(FatigueAnalysis {
            entity_id: entity_id.to_string(),
            recipe_code: recipe_code.to_string(),
            firing_count,
            unique_content_count,
            repetition_rate,
            is_fatigued,
            recommended_action,
        })
    }

    /// Get all fatigued combinations
    pub fn get_fatigued_combinations(&self) -> Vec<FatigueAnalysis> {
        let mut results = Vec::new();

        for key in self.firing_history.keys() {
            let parts: Vec<&str> = key.split(':').collect();
            if parts.len() != 2 {
                continue;
            }

            if let Some(analysis) = self.detect_fatigue(parts[0], parts[1]) {
                if analysis.is_fatigued {
                    results.push(analysis);
                }
            }
        }

        // Sort ascending: lowest repetition_rate first (most fatigued = worst)
        results.sort_by(|a, b| {
            a.repetition_rate
                .partial_cmp(&b.repetition_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    /// Calculate staleness penalty for confidence
    pub fn staleness_penalty(&self, entity_id: &str, recipe_code: &str) -> f64 {
        if let Some(analysis) = self.detect_fatigue(entity_id, recipe_code) {
            // Apply penalty based on fatigue level
            match analysis.recommended_action {
                FatigueAction::Suppress => 0.1,
                FatigueAction::ReduceFrequency => 0.3,
                FatigueAction::RequireNewContent => 0.5,
                FatigueAction::NoAction => 1.0,
            }
        } else {
            1.0 // No penalty if not fatigued
        }
    }

    /// Get insight quality score
    pub fn get_insight_quality(&self, insight_id: &str) -> f64 {
        let records = self.feedback_records.get(insight_id);

        match records {
            Some(feedback_list) if !feedback_list.is_empty() => {
                let total = feedback_list.len() as f64;
                let positive = feedback_list
                    .iter()
                    .filter(|r| {
                        matches!(
                            r.feedback,
                            InsightFeedback::Bookmarked
                                | InsightFeedback::Actioned
                                | InsightFeedback::Relevant
                                | InsightFeedback::TruePositive
                        )
                    })
                    .count() as f64;

                positive / total
            }
            _ => 0.5, // Default neutral score
    }
}
}

/// Compute a title-diversity penalty factor using semantic similarity.
///
/// Returns a multiplier in `[0.0, 1.0]`:
/// - `1.0`  — the new title is semantically distinct from all recent titles
/// - `0.5`  — moderate overlap (Jaccard ~0.3–0.6)
/// - `0.0`  — near-identical to a recent title (Jaccard ≥ 0.8)
///
/// Uses [`semantic_diversity_score`] to compare the proposed title against a
/// sliding window of recent titles for the same entity.
pub fn title_diversity_penalty(new_title: &str, recent_titles: &[String]) -> f64 {
if recent_titles.is_empty() {
    return 1.0; // No history → no penalty
}
let max_similarity = recent_titles
    .iter()
    .map(|t| crate::title_diversity::semantic_diversity_score(new_title, std::slice::from_ref(t)))
    .fold(0.0_f64, f64::max);

// Map similarity to penalty: 0.0 similarity → 1.0 multiplier
// 1.0 similarity → 0.0 multiplier
let penalty = 1.0 - max_similarity;
// Clamp to [0.0, 1.0]
penalty.clamp(0.0, 1.0)
}

/// Threshold recommendation based on performance
pub struct ThresholdRecommendation {
    pub recipe_code: String,
    pub current_threshold: f64,
    pub recommended_threshold: f64,
    pub reason: String,
    pub confidence: f64,
}

impl InsightFeedbackTracker {
    /// Recommend threshold adjustments based on performance
    pub fn recommend_threshold_adjustments(&self) -> Vec<ThresholdRecommendation> {
        let mut recommendations = Vec::new();

        for (code, perf) in &self.recipe_performance {
            let precision = perf.precision();
            let recall = perf.recall();

            // If precision is low, increase threshold
            // If recall is low, decrease threshold
            let (recommended, reason) = if precision < 0.4 && recall > 0.6 {
                // High recall but low precision - increase threshold to reduce FP
                let new_threshold = 0.15; // Example adjustment
                (new_threshold, "Low precision indicates too many false positives. Recommend increasing signal threshold.".to_string())
            } else if recall < 0.4 && precision > 0.6 {
                // High precision but low recall - decrease threshold to catch more
                let new_threshold = 0.05;
                (new_threshold, "High precision but low recall indicates we may be missing valid insights. Recommend decreasing threshold.".to_string())
            } else if precision < 0.3 {
                (0.20, "Very low precision - recommend significant threshold increase or recipe review.".to_string())
            } else {
                continue; // No recommendation needed
            };

            recommendations.push(ThresholdRecommendation {
                recipe_code: code.clone(),
                current_threshold: 0.10, // Would need to load from recipe
                recommended_threshold: recommended,
                reason,
                confidence: perf.f1_score(),
            });
        }

        recommendations.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        recommendations
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CLOSED-LOOP FEEDBACK — Actionable Signals for Generators
// ═══════════════════════════════════════════════════════════════════════════════
//
// The types below transform feedback data into actionable signals that
// generators can consume. This closes the loop between detection and action.
// See docs/analysis/insight_system_analysis.md §6.1 for the design rationale.

use std::collections::HashSet;

/// A feedback signal that generators can consume to adjust behaviour.
///
/// This is the **closed-loop** component that connects detection to action.
/// Each signal carries metadata about what needs to change, how urgently,
/// and a human-readable explanation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackSignal {
    /// What needs to change
    pub signal_type: FeedbackSignalType,
    /// Which entity is affected
    pub entity: Option<String>,
    /// How strong the signal is (0.0–1.0)
    pub intensity: f64,
    /// Which category of insight needs change
    pub category: Option<String>,
    /// Human-readable reason
    pub reason: String,
    /// When this signal was generated
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

/// The type of a feedback signal, encoding the detected condition and its
/// severity parameters.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FeedbackSignalType {
    /// Entity has been over-covered — generators should switch to other entities
    EntityFatigue {
        times_covered: usize,
        period_days: usize,
    },
    /// This category of insight is getting formulaic (title similarity too high)
    CategoryRepetition {
        title_similarity: f64,
        recent_count: usize,
    },
    /// User feedback indicates declining quality for this entity / category
    QualityDecline {
        avg_rating: f64,
        trend: f64,
    },
    /// A new entity is emerging and should be added to the active set
    NewEntitySignal {
        observation_velocity: f64,
    },
    /// A previously ignored entity now has sufficient data to revive coverage
    StaleEntityRevival {
        data_accumulated: usize,
    },
    /// Comparison frames for this entity have not changed recently
    ComparisonStaleness {
        days_since_last_comparison: usize,
    },
    /// A dynamically discovered entity needs an insight generated urgently
    /// to establish analytical coverage (Discovery Integration — Phase 2).
    ///
    /// This signal is emitted by the [`FeedbackController::analyze`] method
    /// when it detects entities in the registry that were dynamically
    /// discovered but have not yet received any insight coverage.
    DiscoveryUrgency {
        /// How long (in seconds) the entity has been registered without coverage.
        uncoverage_duration_secs: u64,
        /// The confidence level from the discovery pipeline (0.0 – 1.0).
        discovery_confidence: f64,
    },
}

/// Simplified feedback entry consumed by [`FeedbackController::analyze`].
#[derive(Debug, Clone)]
pub struct FeedbackEntry {
    /// Entity the feedback relates to
    pub entity: String,
    /// Optional category context
    pub category: Option<String>,
    /// Numeric rating (0.0 = worst, 1.0 = best), if available
    pub rating: Option<f64>,
    /// Unix-timestamp of the feedback event
    pub timestamp: i64,
}

/// A record of a previously-generated insight, consumed by
/// [`FeedbackController::analyze`] to detect repetition and staleness.
#[derive(Debug, Clone)]
pub struct InsightRecord {
    /// Entity the insight was generated for
    pub entity: String,
    /// Category of the insight (e.g. "demand", "supply_chain")
    pub category: String,
    /// Generated title string
    pub title: String,
    /// Unix-timestamp of generation
    pub timestamp: i64,
}

/// Tracks coverage statistics for a single entity over two lookback windows.
#[derive(Debug, Clone)]
struct EntityCoverage {
    insight_count_7d: usize,
    insight_count_30d: usize,
    avg_rating_30d: f64,
    last_insight_at: Option<chrono::DateTime<chrono::Utc>>,
    title_history: Vec<String>,
}

/// Tracks quality metrics for a single insight category.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields used during analysis, consumed externally via signals
struct CategoryQualityMetrics {
    recent_ratings: Vec<f64>,
    title_diversity_score: f64,
    signal_count: usize,
}

/// The central hub that collects feedback and produces actionable signals.
///
/// # Usage
/// 1. Call [`analyze`](Self::analyze) with recent feedback entries and insights.
/// 2. Read the returned [`Vec<FeedbackSignal>`] for routing into generators.
/// 3. Call [`get_priority_entities`](Self::get_priority_entities) or
///    [`get_suppressed_categories`](Self::get_suppressed_categories) for
///    orchestration decisions.
///
/// # Thread safety
/// `FeedbackController` is `Send + Sync` because all fields are
/// `Send + Sync` (`Vec`, `HashMap`, `String`, `f64`, etc.).
pub struct FeedbackController {
    /// Signals generated by the most recent [`analyze`](Self::analyze) call
    signals: Vec<FeedbackSignal>,
    /// Entity-coverage map built during [`analyze`](Self::analyze)
    entity_coverage: HashMap<String, EntityCoverage>,
    /// Category-quality map built during [`analyze`](Self::analyze)
    category_quality: HashMap<String, CategoryQualityMetrics>,
}

impl Default for FeedbackController {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedbackController {
    /// Create a new, empty controller.
    pub fn new() -> Self {
        Self {
            signals: Vec::new(),
            entity_coverage: HashMap::new(),
            category_quality: HashMap::new(),
        }
    }

    /// Main entry point: analyse feedback entries and recent insights, then
    /// produce actionable signals for generators.
    ///
    /// # Signal rules
    ///
    /// 1. **EntityFatigue** — entities with >5 insights in the last 7 days.
    /// 2. **CategoryRepetition** — categories where average pairwise title
    ///    similarity exceeds 0.6.
    /// 3. **QualityDecline** — entities whose rolling average rating < 0.4.
    /// 4. **StaleEntityRevival** — entities without an insight for ≥7 days
    ///    but with accumulated data in the 30-day window.
    pub fn analyze(
        &mut self,
        feedback_entries: &[FeedbackEntry],
        recent_insights: &[InsightRecord],
    ) -> Vec<FeedbackSignal> {
        self.signals.clear();
        self.entity_coverage.clear();
        self.category_quality.clear();

        let now = chrono::Utc::now();
        let seven_days_ago = now - chrono::Duration::days(7);
        let thirty_days_ago = now - chrono::Duration::days(30);

        // ── 1. Build entity + category state from recent insights ──
        let mut category_titles: HashMap<String, Vec<String>> = HashMap::new();

        for insight in recent_insights {
            let ts = chrono::DateTime::from_timestamp(insight.timestamp, 0)
                .unwrap_or(now);

            // Entity coverage
            let cov = self
                .entity_coverage
                .entry(insight.entity.clone())
                .or_insert_with(|| EntityCoverage {
                    insight_count_7d: 0,
                    insight_count_30d: 0,
                    avg_rating_30d: 0.5,
                    last_insight_at: None,
                    title_history: Vec::new(),
                });

            if ts > seven_days_ago {
                cov.insight_count_7d += 1;
            }
            if ts > thirty_days_ago {
                cov.insight_count_30d += 1;
            }
            if cov.last_insight_at.is_none_or(|last| ts > last) {
                cov.last_insight_at = Some(ts);
            }
            cov.title_history.push(insight.title.clone());

            // Category titles (for similarity analysis)
            category_titles
                .entry(insight.category.clone())
                .or_default()
                .push(insight.title.clone());

            // Category quality (initial)
            self.category_quality
                .entry(insight.category.clone())
                .or_insert_with(|| CategoryQualityMetrics {
                    recent_ratings: Vec::new(),
                    title_diversity_score: 1.0,
                    signal_count: 0,
                })
                .signal_count += 1;
        }

        // ── 2. Incorporate feedback ratings ──
        for entry in feedback_entries {
            if let Some(rating) = entry.rating {
                let rating = rating.clamp(0.0, 1.0);
                if let Some(cov) = self.entity_coverage.get_mut(&entry.entity) {
                    // Exponential moving average (α = 0.1)
                    cov.avg_rating_30d = cov.avg_rating_30d * 0.9 + rating * 0.1;
                }
                if let Some(ref cat) = entry.category {
                    if let Some(q) = self.category_quality.get_mut(cat) {
                        q.recent_ratings.push(rating);
                    }
                }
            }
        }

        // ── 3. Emit signals ──

        // 3a. EntityFatigue: >5 insights in 7 days
        for (entity, cov) in &self.entity_coverage {
            if cov.insight_count_7d > 5 {
                let intensity =
                    ((cov.insight_count_7d as f64 - 5.0) / 15.0).clamp(0.0, 1.0);
                self.signals.push(FeedbackSignal {
                    signal_type: FeedbackSignalType::EntityFatigue {
                        times_covered: cov.insight_count_7d,
                        period_days: 7,
                    },
                    entity: Some(entity.clone()),
                    intensity,
                    category: None,
                    reason: format!(
                        "Entity '{entity}' has {} insights in 7 days (threshold: 5). \
                         Recommend rotating to other entities.",
                        cov.insight_count_7d,
                    ),
                    generated_at: now,
                });
            }
        }

        // 3b. CategoryRepetition: avg pairwise title similarity > 0.6
        for (cat, titles) in &category_titles {
            if titles.len() < 3 {
                continue;
            }
            let avg_sim = Self::avg_pairwise_similarity(titles);
            if avg_sim > 0.6 {
                let intensity = ((avg_sim - 0.6) / 0.4).clamp(0.0, 1.0);
                self.signals.push(FeedbackSignal {
                    signal_type: FeedbackSignalType::CategoryRepetition {
                        title_similarity: avg_sim,
                        recent_count: titles.len(),
                    },
                    entity: None,
                    intensity,
                    category: Some(cat.clone()),
                    reason: format!(
                        "Category '{cat}' has avg title similarity {avg_sim:.2} \
                         (threshold: 0.6). Titles are becoming formulaic.",
                    ),
                    generated_at: now,
                });
            }
        }

        // 3c. QualityDecline: avg rating trending downward
        for (entity, cov) in &self.entity_coverage {
            if cov.avg_rating_30d < 0.4 {
                let intensity = (0.4 - cov.avg_rating_30d) / 0.4;
                self.signals.push(FeedbackSignal {
                    signal_type: FeedbackSignalType::QualityDecline {
                        avg_rating: cov.avg_rating_30d,
                        trend: cov.avg_rating_30d - 0.5, // simple offset from neutral
                    },
                    entity: Some(entity.clone()),
                    intensity,
                    category: None,
                    reason: format!(
                        "Entity '{entity}' has average rating {:.2}. \
                         Quality may be declining.",
                        cov.avg_rating_30d,
                    ),
                    generated_at: now,
                });
            }
        }

        // 3d. StaleEntityRevival: ≥7 days since last insight + data exists
        for (entity, cov) in &self.entity_coverage {
            if let Some(last) = cov.last_insight_at {
                let days_since = (now - last).num_days();
                if days_since >= 7 && cov.insight_count_30d > 0 {
                    let intensity = (days_since as f64 / 30.0).clamp(0.0, 1.0);
                    self.signals.push(FeedbackSignal {
                        signal_type: FeedbackSignalType::StaleEntityRevival {
                            data_accumulated: cov.insight_count_30d,
                        },
                        entity: Some(entity.clone()),
                        intensity,
                        category: None,
                        reason: format!(
                            "Entity '{entity}' has not had an insight in {days_since} days. \
                             Data accumulated: {} insights in 30d.",
                            cov.insight_count_30d,
                        ),
                        generated_at: now,
                    });
                }
            }
        }

        // 3e. NewEntitySignal — detect entities that appear in feedback but
        //     have very few insights (emerging entities).
        {
            let known_entities: HashSet<&str> =
                self.entity_coverage.keys().map(|s| s.as_str()).collect();
            let mut seen: HashSet<&str> = HashSet::new();
            for entry in feedback_entries {
                let e = entry.entity.as_str();
                if seen.insert(e) && !known_entities.contains(e) {
                    // Entity appears in feedback but not in insights → emerging
                    self.signals.push(FeedbackSignal {
                        signal_type: FeedbackSignalType::NewEntitySignal {
                            observation_velocity: 1.0,
                        },
                        entity: Some(entry.entity.clone()),
                        intensity: 0.6,
                        category: entry.category.clone(),
                        reason: format!(
                            "Entity '{}' appears in feedback but has no recent insights. \
                             May be an emerging entity worth tracking.",
                            entry.entity,
                        ),
                        generated_at: now,
                    });
                }
            }
        }

        self.signals.clone()
    }

    /// Compute average pairwise Jaccard similarity across a set of titles.
    fn avg_pairwise_similarity(titles: &[String]) -> f64 {
        if titles.len() < 2 {
            return 0.0;
        }
        let mut total = 0.0_f64;
        let mut count = 0_usize;
        for i in 0..titles.len() {
            for j in (i + 1)..titles.len() {
                // diversity = 1 - similarity → similarity = 1 - diversity
                let sim = 1.0
                    - crate::title_diversity::semantic_diversity_score(
                        &titles[i],
                        &[titles[j].clone()],
                    );
                total += sim;
                count += 1;
            }
        }
        if count > 0 {
            total / count as f64
        } else {
            0.0
        }
    }

    /// Emit [`DiscoveryUrgency`] signals for dynamically discovered entities
    /// that have not yet received any insight coverage.
    ///
    /// Should be called after [`analyze`](Self::analyze) when an entity registry
    /// is available (e.g., from the orchestrator).
    ///
    /// # Priority
    ///
    /// DiscoveryUrgency signals get the **highest** priority tier (Tier 0) in
    /// [`get_priority_entities`], above revived and emerging entities.
    pub fn emit_discovery_urgency_signals(
        &mut self,
        registry: &crate::entity_relevance::EntityRegistry,
        recent_insights: &[InsightRecord],
    ) {
        let now = chrono::Utc::now();

        for profile in registry.dynamically_discovered() {
            let entity_name = &profile.entity_name;

            // Skip if entity already has some insight coverage
            let has_coverage = recent_insights
                .iter()
                .any(|r| r.entity.eq_ignore_ascii_case(entity_name));
            if has_coverage {
                continue;
            }

            // Determine how long without coverage (using verification time as proxy)
            let uncoverage_duration = profile
                .last_verified
                .map(|v| (now - v).num_seconds() as u64)
                .unwrap_or(0);

            // Intensity: longer without coverage = more urgent
            let max_urgency_secs = 7 * 86_400u64; // 7 days -> max urgency
            let intensity =
                ((uncoverage_duration as f64) / (max_urgency_secs as f64)).clamp(0.1, 1.0);

            // Discovery confidence from verification count
            let discovery_confidence =
                (profile.verification_count as f64 * 0.3).clamp(0.3, 1.0);

            self.signals.push(FeedbackSignal {
                signal_type: FeedbackSignalType::DiscoveryUrgency {
                    uncoverage_duration_secs: uncoverage_duration,
                    discovery_confidence,
                },
                entity: Some(entity_name.clone()),
                intensity,
                category: None,
                reason: format!(
                    "Dynamically discovered entity '{}' has not yet received any insight \
                     coverage (uncovered for {}s). Prioritise for initial analysis.",
                    entity_name, uncoverage_duration,
                ),
                generated_at: now,
            });
        }
    }

    /// Return entities ranked by priority for insight generation.
    ///
    /// Priority order:
    /// 0. Entities with [`DiscoveryUrgency`] signals (newly discovered, highest priority).
    /// 1. Entities with [`StaleEntityRevival`] signals (need coverage).
    /// 2. Entities from [`NewEntitySignal`] (emerging).
    /// 3. Entities **not** in [`EntityFatigue`] (not over-covered).
    ///
    /// Within each tier results are sorted by a composite urgency score.
    pub fn get_priority_entities(&self, count: usize) -> Vec<String> {
        let mut fatigued: HashSet<&str> = HashSet::new();
        let mut discovery_urgent: Vec<(&str, f64)> = Vec::new();
        let mut revived: Vec<(&str, f64)> = Vec::new();
        let mut emerging: Vec<&str> = Vec::new();

        for sig in &self.signals {
            match &sig.signal_type {
                FeedbackSignalType::EntityFatigue { .. } => {
                    if let Some(ref e) = sig.entity {
                        fatigued.insert(e.as_str());
                    }
                }
                FeedbackSignalType::DiscoveryUrgency { .. } => {
                    if let Some(ref e) = sig.entity {
                        discovery_urgent.push((e.as_str(), sig.intensity));
                    }
                }
                FeedbackSignalType::StaleEntityRevival { .. } => {
                    if let Some(ref e) = sig.entity {
                        revived.push((e.as_str(), sig.intensity));
                    }
                }
                FeedbackSignalType::NewEntitySignal { .. } => {
                    if let Some(ref e) = sig.entity {
                        emerging.push(e.as_str());
                    }
                }
                _ => {}
            }
        }

        let mut scored: Vec<(String, f64)> = Vec::new();

        // Tier 0: discovery-urgent entities (highest priority)
        for (entity, intensity) in &discovery_urgent {
            scored.push((entity.to_string(), 3.0 + intensity));
        }

        // Tier 1: revived entities (boosted urgency)
        for (entity, intensity) in &revived {
            scored.push((entity.to_string(), 2.0 + intensity));
        }

        // Tier 2: emerging entities
        for entity in &emerging {
            scored.push((entity.to_string(), 1.5));
        }

        // Tier 3: non-fatigued entities with coverage data
        for (entity, cov) in &self.entity_coverage {
            if !fatigued.contains(entity.as_str())
                && !scored.iter().any(|(e, _)| e == entity)
            {
                // Prefer entities with decent ratings and low 7d count
                let score = cov.avg_rating_30d * 0.5
                    + (1.0 / (cov.insight_count_7d.max(1) as f64)) * 0.5;
                scored.push((entity.clone(), score));
            }
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(count);
        scored.into_iter().map(|(e, _)| e).collect()
    }

    /// Return the set of categories that should be deprioritised.
    ///
    /// A category is suppressed when:
    /// 1. It has a [`CategoryRepetition`] signal.
    /// 2. It has a [`QualityDecline`] signal with a trend below -0.3.
    pub fn get_suppressed_categories(&self) -> HashSet<String> {
        let mut suppressed = HashSet::new();

        for sig in &self.signals {
            match &sig.signal_type {
                FeedbackSignalType::CategoryRepetition { .. } => {
                    if let Some(ref cat) = sig.category {
                        suppressed.insert(cat.clone());
                    }
                }
                FeedbackSignalType::QualityDecline { trend, .. } if *trend < -0.3 => {
                    if let Some(ref cat) = sig.category {
                        suppressed.insert(cat.clone());
                    }
                }
                _ => {}
            }
        }

        suppressed
    }

    /// Returns `true` when an entity's fatigue threshold has been exceeded
    /// and generators should rotate to a different entity.
    pub fn should_rotate_entity(entity: &str, signals: &[FeedbackSignal]) -> bool {
        signals.iter().any(|s| {
            matches!(&s.signal_type, FeedbackSignalType::EntityFatigue { .. })
                && s.entity.as_deref() == Some(entity)
        })
    }

    /// Suggest entities that could serve as fresh comparison frames for the
    /// given entity, based on category affinity and the entity registry.
    pub fn suggest_new_comparison_frames(
        entity: &str,
        registry: &crate::entity_relevance::EntityRegistry,
    ) -> Vec<String> {
        let mut candidates: Vec<String> = Vec::new();

        // Prefer same-category entities
        if let Some(profile) = registry.get(entity) {
            for candidate in registry.entities_in_category(&profile.category) {
                let name = &candidate.entity_name;
                if !name.eq_ignore_ascii_case(entity) {
                    candidates.push(name.clone());
                    if candidates.len() >= 5 {
                        break;
                    }
                }
            }
        }

        // Fallback: any other entity
        if candidates.is_empty() {
            for name in registry.entity_names() {
                if !name.eq_ignore_ascii_case(entity) {
                    candidates.push(name.clone());
                    if candidates.len() >= 5 {
                        break;
                    }
                }
            }
        }

        candidates
    }

    /// Signals from the last [`analyze`](Self::analyze) call.
    pub fn signals(&self) -> &[FeedbackSignal] {
        &self.signals
    }

    /// Categories that have been observed in recent insights.
    pub fn known_categories(&self) -> Vec<String> {
        self.category_quality.keys().cloned().collect()
    }
}

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

    #[test]
    fn test_feedback_recording() {
        let mut tracker = InsightFeedbackTracker::new();

        let record = InsightFeedbackRecord {
            insight_id: "insight-1".to_string(),
            entity_id: "nvidia-123".to_string(),
            recipe_code: "A001".to_string(),
            feedback: InsightFeedback::FalsePositive,
            timestamp: Utc::now().timestamp(),
            user_id: Some("user-1".to_string()),
            notes: None,
        };

        tracker.record_feedback(record);

        let perf = tracker.get_recipe_performance("A001").unwrap();
        assert_eq!(perf.total_firings, 1);
        assert_eq!(perf.false_positives, 1);
    }

    #[test]
    fn test_firing_history() {
        let mut tracker = InsightFeedbackTracker::new();

        tracker.record_firing("nvidia", "A001", "hash1");
        tracker.record_firing("nvidia", "A001", "hash2");
        tracker.record_firing("nvidia", "A001", "hash1"); // Repeat

        let fatigue = tracker.detect_fatigue("nvidia", "A001").unwrap();
        assert_eq!(fatigue.firing_count, 3);
        assert_eq!(fatigue.unique_content_count, 2);
    }

    #[test]
    fn test_fatigue_detection() {
        let mut tracker = InsightFeedbackTracker::new();

        // Fire same insight 10 times (high fatigue)
        for _i in 0..10 {
            tracker.record_firing("nvidia", "A001", "same_hash");
        }

        let fatigue = tracker.detect_fatigue("nvidia", "A001").unwrap();

        assert!(fatigue.is_fatigued);
        assert!(matches!(
            fatigue.recommended_action,
            FatigueAction::Suppress | FatigueAction::ReduceFrequency
        ));
    }

    #[test]
    fn test_recipe_performance_precision() {
        let perf = RecipePerformance {
            recipe_code: "TEST".to_string(),
            total_firings: 10,
            true_positives: 7,
            false_positives: 3,
            false_negatives: 2,
            bookmark_rate: 0.5,
            action_rate: 0.4,
            dismiss_rate: 0.3,
            avg_relevance_score: 0.7,
            last_updated: Utc::now().timestamp(),
        };

        assert!((perf.precision() - 0.7).abs() < 0.01);
    }

    #[test]
    fn test_recipe_recommendation() {
        // High performing recipe
        let good = RecipePerformance {
            recipe_code: "GOOD".to_string(),
            total_firings: 100,
            true_positives: 90,
            false_positives: 5,
            false_negatives: 5,
            bookmark_rate: 0.8,
            action_rate: 0.7,
            dismiss_rate: 0.1,
            avg_relevance_score: 0.9,
            last_updated: Utc::now().timestamp(),
        };

        assert!(matches!(
            good.recommendation(),
            RecipeRecommendation::KeepActive
        ));

        // Poor performing recipe
        let bad = RecipePerformance {
            recipe_code: "BAD".to_string(),
            total_firings: 100,
            true_positives: 5,
            false_positives: 90,
            false_negatives: 5,
            bookmark_rate: 0.1,
            action_rate: 0.05,
            dismiss_rate: 0.9,
            avg_relevance_score: 0.2,
            last_updated: Utc::now().timestamp(),
        };

        assert!(matches!(bad.recommendation(), RecipeRecommendation::Retire));
    }

    #[test]
    fn test_insight_quality_score() {
        let mut tracker = InsightFeedbackTracker::new();

        // Record mixed feedback
        tracker.record_feedback(InsightFeedbackRecord {
            insight_id: "test-1".to_string(),
            entity_id: "e1".to_string(),
            recipe_code: "R1".to_string(),
            feedback: InsightFeedback::Relevant,
            timestamp: Utc::now().timestamp(),
            user_id: None,
            notes: None,
        });

        tracker.record_feedback(InsightFeedbackRecord {
            insight_id: "test-1".to_string(),
            entity_id: "e1".to_string(),
            recipe_code: "R1".to_string(),
            feedback: InsightFeedback::Irrelevant,
            timestamp: Utc::now().timestamp(),
            user_id: None,
            notes: None,
        });

        let quality = tracker.get_insight_quality("test-1");
        assert!((quality - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_threshold_recommendations() {
        let mut tracker = InsightFeedbackTracker::new();

        // Add low precision performance
        tracker.record_feedback(InsightFeedbackRecord {
            insight_id: "1".to_string(),
            entity_id: "e1".to_string(),
            recipe_code: "LOW_PREC".to_string(),
            feedback: InsightFeedback::FalsePositive,
            timestamp: Utc::now().timestamp(),
            user_id: None,
            notes: None,
        });

        // Add 9 more FP
        for i in 2..=10 {
            tracker.record_feedback(InsightFeedbackRecord {
                insight_id: format!("{}", i),
                entity_id: "e1".to_string(),
                recipe_code: "LOW_PREC".to_string(),
                feedback: InsightFeedback::FalsePositive,
                timestamp: Utc::now().timestamp(),
                user_id: None,
                notes: None,
            });
        }

        let recs = tracker.recommend_threshold_adjustments();
        assert!(!recs.is_empty());

        let low_prec_rec = recs.iter().find(|r| r.recipe_code == "LOW_PREC").unwrap();
        assert!(low_prec_rec.recommended_threshold > low_prec_rec.current_threshold);
    }

    // ── FeedbackController tests ──

    fn make_insight(entity: &str, category: &str, title: &str, days_ago: i64) -> InsightRecord {
        let ts = Utc::now().timestamp() - days_ago * 86400;
        InsightRecord {
            entity: entity.to_string(),
            category: category.to_string(),
            title: title.to_string(),
            timestamp: ts,
        }
    }

    #[test]
    fn test_feedback_controller_detects_entity_fatigue() {
        let mut controller = FeedbackController::new();

        // 7 insights in the last 7 days for "nvidia" — exceeds threshold of 5
        let insights: Vec<InsightRecord> = (0..7)
            .map(|i| make_insight("nvidia", "demand", &format!("NVIDIA surge {}", i), i))
            .collect();

        let signals = controller.analyze(&[], &insights);

        let fatigue_signals: Vec<&FeedbackSignal> = signals
            .iter()
            .filter(|s| matches!(s.signal_type, FeedbackSignalType::EntityFatigue { .. }))
            .collect();

        assert_eq!(fatigue_signals.len(), 1, "Should emit exactly one EntityFatigue signal");
        let sig = fatigue_signals[0];
        assert_eq!(sig.entity.as_deref(), Some("nvidia"));
        if let FeedbackSignalType::EntityFatigue { times_covered, period_days } = &sig.signal_type {
            assert_eq!(*times_covered, 7);
            assert_eq!(*period_days, 7);
        } else {
            panic!("Expected EntityFatigue");
        }
    }

    #[test]
    fn test_feedback_controller_emits_stale_entity_revival() {
        let mut controller = FeedbackController::new();

        // Entity with insights 20 days ago (≥7 days stale) but data in 30d window
        let insights = vec![
            make_insight("intel", "demand", "Intel update", 20),
            make_insight("intel", "supply_chain", "Intel supply", 22),
        ];

        let signals = controller.analyze(&[], &insights);

        let stale_signals: Vec<&FeedbackSignal> = signals
            .iter()
            .filter(|s| matches!(s.signal_type, FeedbackSignalType::StaleEntityRevival { .. }))
            .collect();

        assert!(
            !stale_signals.is_empty(),
            "Should emit StaleEntityRevival for stale entity with data"
        );
        assert_eq!(stale_signals[0].entity.as_deref(), Some("intel"));
    }

    #[test]
    fn test_feedback_controller_detects_category_repetition() {
        let mut controller = FeedbackController::new();

        // Three titles with high lexical similarity → avg pairwise similarity > 0.6
        let insights = vec![
            make_insight("nvidia", "demand", "NVIDIA demand surge amid AI growth", 1),
            make_insight("nvidia", "demand", "NVIDIA demand growth amid AI", 2),
            make_insight("nvidia", "demand", "NVIDIA demand boom amid AI era", 3),
        ];

        let signals = controller.analyze(&[], &insights);

        let repetition_signals: Vec<&FeedbackSignal> = signals
            .iter()
            .filter(|s| {
                matches!(s.signal_type, FeedbackSignalType::CategoryRepetition { .. })
            })
            .collect();

        assert!(
            !repetition_signals.is_empty(),
            "Should emit CategoryRepetition for highly similar titles"
        );
    }

    #[test]
    fn test_get_priority_entities_excludes_fatigued() {
        let mut controller = FeedbackController::new();

        // One fatigued entity (nvidia, 7 insights) and one fresh entity (amd, 1 insight)
        let insights = vec![
            make_insight("nvidia", "demand", "NVIDIA 1", 1),
            make_insight("nvidia", "demand", "NVIDIA 2", 1),
            make_insight("nvidia", "demand", "NVIDIA 3", 1),
            make_insight("nvidia", "demand", "NVIDIA 4", 1),
            make_insight("nvidia", "demand", "NVIDIA 5", 1),
            make_insight("nvidia", "demand", "NVIDIA 6", 1),
            make_insight("nvidia", "demand", "NVIDIA 7", 1),
            make_insight("amd", "demand", "AMD 1", 1),
        ];

        controller.analyze(&[], &insights);
        let priorities = controller.get_priority_entities(5);

        // nvidia is fatigued; should NOT be in priority list (unless revived/emerging)
        // amd is non-fatigued and should appear
        assert!(
            !priorities.contains(&"nvidia".to_string()),
            "Fatigued entity 'nvidia' should be excluded from priorities, got: {:?}",
            priorities
        );
        assert!(
            priorities.contains(&"amd".to_string()),
            "Non-fatigued entity 'amd' should appear in priorities, got: {:?}",
            priorities
        );
    }

    #[test]
    fn test_get_suppressed_categories_returns_declining() {
        let mut controller = FeedbackController::new();

        // Analyze with repetitive titles to trigger CategoryRepetition signal
        let repetitive = vec![
            make_insight("nvidia", "supply_chain", "NVIDIA supply chain risk update", 1),
            make_insight("nvidia", "supply_chain", "NVIDIA supply chain risk analysis", 1),
            make_insight("nvidia", "supply_chain", "NVIDIA supply chain risk assessment", 1),
        ];

        let _signals = controller.analyze(&[], &repetitive);
        let suppressed = controller.get_suppressed_categories();

        assert!(
            suppressed.contains("supply_chain"),
            "Category with repetitive titles should be suppressed, got: {:?}",
            suppressed
        );
    }

    #[test]
    fn test_should_rotate_entity_returns_true_for_fatigued() {
        let signals = vec![FeedbackSignal {
            signal_type: FeedbackSignalType::EntityFatigue {
                times_covered: 8,
                period_days: 7,
            },
            entity: Some("nvidia".to_string()),
            intensity: 0.6,
            category: None,
            reason: "test".to_string(),
            generated_at: Utc::now(),
        }];

        assert!(FeedbackController::should_rotate_entity("nvidia", &signals));
        assert!(!FeedbackController::should_rotate_entity("amd", &signals));
    }

    #[test]
    fn test_suggest_new_comparison_frames_returns_candidates() {
        use crate::entity_relevance::{EntityCategory, EntityProfile, EntityRegistry};

        let mut registry = EntityRegistry::empty();
        registry.register(
            EntityProfile::new("NVIDIA").with_category(EntityCategory::Semiconductor),
        );
        registry.register(
            EntityProfile::new("AMD").with_category(EntityCategory::Semiconductor),
        );
        registry.register(
            EntityProfile::new("Intel").with_category(EntityCategory::Semiconductor),
        );

        let candidates = FeedbackController::suggest_new_comparison_frames("nvidia", &registry);

        assert!(!candidates.is_empty(), "Should suggest comparison candidates");
        assert!(
            !candidates.iter().any(|c| c.eq_ignore_ascii_case("nvidia")),
            "Should not suggest the entity itself"
        );
    }

    #[test]
    fn test_feedback_controller_default() {
        let controller = FeedbackController::default();
        assert!(controller.signals().is_empty());
        assert!(controller.known_categories().is_empty());
    }

    #[test]
    fn test_priority_entities_empty_when_no_analysis() {
        let controller = FeedbackController::new();
        let priorities = controller.get_priority_entities(5);
        assert!(priorities.is_empty());
    }

    #[test]
    fn test_suppressed_categories_empty_when_no_signals() {
        let controller = FeedbackController::new();
        let suppressed = controller.get_suppressed_categories();
        assert!(suppressed.is_empty());
    }

    #[test]
    fn test_analyze_with_empty_inputs() {
        let mut controller = FeedbackController::new();
        let signals = controller.analyze(&[], &[]);
        assert!(signals.is_empty(), "No signals expected from empty inputs");
    }
}
