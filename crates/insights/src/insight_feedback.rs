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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::Utc;

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
        let entry = self.feedback_records
            .entry(record.insight_id.clone())
            .or_insert_with(Vec::new);
        entry.push(record.clone());

        // Update recipe performance
        self.update_recipe_performance(&record);
    }

    /// Update recipe performance based on feedback
    fn update_recipe_performance(&mut self, record: &InsightFeedbackRecord) {
        let perf = self.recipe_performance
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
            InsightFeedback::TruePositive | InsightFeedback::Actioned | InsightFeedback::Relevant => {
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
    pub fn record_firing(
        &mut self,
        entity_id: &str,
        recipe_code: &str,
        insight_hash: &str,
    ) {
        let key = format!("{}:{}", entity_id, recipe_code);
        
        let entry = self.firing_history
            .entry(key.clone())
            .or_insert_with(Vec::new);
        
        entry.push(FiringRecord {
            timestamp: Utc::now().timestamp(),
            insight_hash: insight_hash.to_string(),
            entity_id: entity_id.to_string(),
            recipe_code: recipe_code.to_string(),
        });

        // Track hash for staleness detection
        let stale_entry = self.stale_hashes
            .entry(key.clone())
            .or_insert_with(Vec::new);
        
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
            let unique_hashes: std::collections::HashSet<_> = history
                .iter()
                .map(|r| &r.insight_hash)
                .collect();
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
        
        for (key, _history) in &self.firing_history {
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
            a.repetition_rate.partial_cmp(&b.repetition_rate).unwrap_or(std::cmp::Ordering::Equal)
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
                let positive = feedback_list.iter()
                    .filter(|r| matches!(
                        r.feedback,
                        InsightFeedback::Bookmarked 
                            | InsightFeedback::Actioned 
                            | InsightFeedback::Relevant 
                            | InsightFeedback::TruePositive
                    ))
                    .count() as f64;
                
                positive / total
            }
            _ => 0.5, // Default neutral score
        }
    }
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

        recommendations.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
        recommendations
    }
}

#[cfg(test)]
mod tests {
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
        
        assert!(matches!(good.recommendation(), RecipeRecommendation::KeepActive));
        
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
}

