use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use apex_core::entities::FeatureRow;

/// Bucket size options for time-series feature aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BucketSize {
    Daily = 1,
    Weekly = 7,
    Monthly = 30,
}

impl BucketSize {
    pub fn days(&self) -> i64 {
        match self {
            Self::Daily => 1,
            Self::Weekly => 7,
            Self::Monthly => 30,
        }
    }

    pub fn from_days(d: i32) -> Option<Self> {
        match d {
            1 => Some(Self::Daily),
            7 => Some(Self::Weekly),
            30 => Some(Self::Monthly),
            _ => None,
        }
    }
}

/// Computes a time bucket epoch from a timestamp and bucket size.
pub fn time_bucket(ts: DateTime<Utc>, bucket: BucketSize) -> i64 {
    let epoch = ts.timestamp();
    let bucket_secs = bucket.days() * 86400;
    // Use div_euclid so pre-Unix-epoch timestamps bucket correctly
    // (Rust integer division truncates toward zero, collapsing negatives into bucket 0).
    epoch.div_euclid(bucket_secs) * bucket_secs
}

/// Accumulator for building feature rows from raw observations.
#[derive(Debug, Clone)]
pub struct FeatureAccumulator {
    pub entity_id: String,
    pub entity_type: String,
    pub bucket_start: i64,
    pub bucket_size: BucketSize,
    signal_counts: HashMap<String, f64>,
    topics: HashMap<String, f64>,
}

impl FeatureAccumulator {
    pub fn new(
        entity_id: impl Into<String>,
        entity_type: impl Into<String>,
        bucket_start: i64,
        bucket_size: BucketSize,
    ) -> Self {
        Self {
            entity_id: entity_id.into(),
            entity_type: entity_type.into(),
            bucket_start,
            bucket_size,
            signal_counts: HashMap::new(),
            topics: HashMap::new(),
        }
    }

    /// Increment a signal counter.
    pub fn count_signal(&mut self, signal_type: &str) {
        *self
            .signal_counts
            .entry(signal_type.to_string())
            .or_insert(0.0) += 1.0;
    }

    /// Record a topic mention with a score.
    pub fn add_topic(&mut self, topic: &str, score: f64) {
        *self.topics.entry(topic.to_string()).or_insert(0.0) += score;
    }

    /// Build a FeatureRow from the accumulated data.
    pub fn build(self) -> FeatureRow {
        FeatureRow {
            entity_id: self.entity_id,
            entity_type: self.entity_type,
            time_bucket: self.bucket_start,
            bucket_size_days: self.bucket_size.days() as i32,
            signal_counts: self.signal_counts,
            diffs: HashMap::new(),
            pct_changes: HashMap::new(),
            regime_flags: HashMap::new(),
            volatility: HashMap::new(),
            topic_drift: self.topics,
            neighbor_agg_1hop: HashMap::new(),
            neighbor_agg_2hop: HashMap::new(),
            poi_pain_index: None,
            poi_role_drift: None,
            poi_influence_delta: None,
        }
    }
}

/// Compute diffs and percent changes between two consecutive feature rows.
/// Modifies `current` in place, using `previous` as the reference period.
pub fn compute_diffs(previous: &FeatureRow, current: &mut FeatureRow) {
    for (signal, &curr_val) in &current.signal_counts.clone() {
        let prev_val = previous.signal_counts.get(signal).copied().unwrap_or(0.0);
        let diff = curr_val - prev_val;
        current.diffs.insert(signal.clone(), diff);

        if prev_val.abs() > f64::EPSILON {
            current.pct_changes.insert(signal.clone(), diff / prev_val);
        }
    }

    // Also compute diffs for signals in previous but not in current
    for (signal, &prev_val) in &previous.signal_counts {
        if !current.signal_counts.contains_key(signal) {
            current.diffs.insert(signal.clone(), -prev_val);
            if prev_val.abs() > f64::EPSILON {
                current.pct_changes.insert(signal.clone(), -1.0);
            }
        }
    }
}

/// Compute rolling volatility over a window of feature rows (by standard deviation of counts).
pub fn compute_volatility(history: &[FeatureRow], current: &mut FeatureRow) {
    if history.is_empty() {
        return;
    }

    // Gather all signal types across history
    let mut all_signals: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in history {
        for key in row.signal_counts.keys() {
            all_signals.insert(key.clone());
        }
    }

    for signal in all_signals {
        let values: Vec<f64> = history
            .iter()
            .map(|r| r.signal_counts.get(&signal).copied().unwrap_or(0.0))
            .collect();

        let n = values.len() as f64;
        if n < 2.0 {
            continue;
        }

        let mean = values.iter().sum::<f64>() / n;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let stddev = variance.sqrt();

        current.volatility.insert(signal, stddev);
    }
}

/// Compute topic drift between two periods as cosine distance.
pub fn compute_topic_drift(previous: &FeatureRow, current: &mut FeatureRow) {
    // Gather all topics
    let mut all_topics: std::collections::HashSet<String> = std::collections::HashSet::new();
    for key in previous.topic_drift.keys() {
        all_topics.insert(key.clone());
    }
    for key in current.topic_drift.keys() {
        all_topics.insert(key.clone());
    }

    if all_topics.is_empty() {
        return;
    }

    let prev_vec: Vec<f64> = all_topics
        .iter()
        .map(|t| previous.topic_drift.get(t).copied().unwrap_or(0.0))
        .collect();
    let curr_vec: Vec<f64> = all_topics
        .iter()
        .map(|t| current.topic_drift.get(t).copied().unwrap_or(0.0))
        .collect();

    let dot: f64 = prev_vec
        .iter()
        .zip(curr_vec.iter())
        .map(|(a, b)| a * b)
        .sum();
    let norm_a: f64 = prev_vec.iter().map(|a| a * a).sum::<f64>().sqrt();
    let norm_b: f64 = curr_vec.iter().map(|b| b * b).sum::<f64>().sqrt();

    let cosine_sim = if norm_a > f64::EPSILON && norm_b > f64::EPSILON {
        dot / (norm_a * norm_b)
    } else {
        0.0
    };

    // Drift = 1 - cosine similarity (0 = identical, 1 = orthogonal)
    let drift = 1.0 - cosine_sim;

    // Store overall drift as a special key
    current.topic_drift.insert("__drift__".to_string(), drift);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn test_bucket_size_days() {
        assert_eq!(BucketSize::Daily.days(), 1);
        assert_eq!(BucketSize::Weekly.days(), 7);
        assert_eq!(BucketSize::Monthly.days(), 30);
    }

    #[test]
    fn test_bucket_size_from_days() {
        assert_eq!(BucketSize::from_days(1), Some(BucketSize::Daily));
        assert_eq!(BucketSize::from_days(7), Some(BucketSize::Weekly));
        assert_eq!(BucketSize::from_days(30), Some(BucketSize::Monthly));
        assert_eq!(BucketSize::from_days(5), None);
    }

    #[test]
    fn test_time_bucket_daily() {
        use chrono::TimeZone;
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 14, 30, 0).unwrap();
        let bucket = time_bucket(ts, BucketSize::Daily);
        let bucket_dt = DateTime::from_timestamp(bucket, 0).unwrap();
        assert_eq!(bucket_dt.date_naive(), ts.date_naive());
    }

    #[test]
    fn test_time_bucket_weekly() {
        use chrono::TimeZone;
        let ts1 = Utc.with_ymd_and_hms(2024, 1, 15, 14, 30, 0).unwrap();
        let ts2 = Utc.with_ymd_and_hms(2024, 1, 16, 10, 0, 0).unwrap();
        let bucket1 = time_bucket(ts1, BucketSize::Weekly);
        let bucket2 = time_bucket(ts2, BucketSize::Weekly);
        assert_eq!(bucket1, bucket2);
    }

    #[test]
    fn test_feature_accumulator_basic() {
        let entity_id = "test-entity-001".to_string();
        let mut acc =
            FeatureAccumulator::new(entity_id.clone(), "company", 1700000000, BucketSize::Weekly);

        acc.count_signal("JobPost");
        acc.count_signal("JobPost");
        acc.count_signal("TenderPosted");
        acc.add_topic("automotive", 0.8);
        acc.add_topic("automotive", 0.3);

        let row = acc.build();
        assert_eq!(row.entity_id, entity_id);
        assert_eq!(row.bucket_size_days, 7);
        assert_eq!(row.signal_counts["JobPost"], 2.0);
        assert_eq!(row.signal_counts["TenderPosted"], 1.0);
        assert!((row.topic_drift["automotive"] - 1.1).abs() < 0.001);
    }

    #[test]
    fn test_compute_diffs() {
        let mut prev = FeatureRow::new("e1".to_string(), "company".to_string(), 0, 7);
        prev.signal_counts.insert("JobPost".to_string(), 3.0);
        prev.signal_counts.insert("TenderPosted".to_string(), 2.0);
        prev.signal_counts.insert("WebChange".to_string(), 1.0);

        let mut curr = FeatureRow::new("e1".to_string(), "company".to_string(), 604800, 7);
        curr.signal_counts.insert("JobPost".to_string(), 5.0);
        curr.signal_counts.insert("TenderPosted".to_string(), 2.0);

        compute_diffs(&prev, &mut curr);

        assert_eq!(curr.diffs["JobPost"], 2.0);
        assert_eq!(curr.diffs["TenderPosted"], 0.0);
        assert_eq!(curr.diffs["WebChange"], -1.0);

        assert!((curr.pct_changes["JobPost"] - 2.0 / 3.0).abs() < 0.001);
        assert!((curr.pct_changes["TenderPosted"]).abs() < 0.001);
        assert!((curr.pct_changes["WebChange"] - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_compute_diffs_from_zero() {
        let prev = FeatureRow::new("e1".to_string(), "company".to_string(), 0, 7);
        let mut curr = FeatureRow::new("e1".to_string(), "company".to_string(), 604800, 7);
        curr.signal_counts.insert("JobPost".to_string(), 3.0);

        compute_diffs(&prev, &mut curr);

        assert_eq!(curr.diffs["JobPost"], 3.0);
        assert!(!curr.pct_changes.contains_key("JobPost"));
    }

    #[test]
    fn test_compute_volatility() {
        let mut r1 = FeatureRow::new("e1".to_string(), "company".to_string(), 0, 7);
        r1.signal_counts.insert("JobPost".to_string(), 2.0);

        let mut r2 = FeatureRow::new("e1".to_string(), "company".to_string(), 604800, 7);
        r2.signal_counts.insert("JobPost".to_string(), 4.0);

        let mut r3 = FeatureRow::new("e1".to_string(), "company".to_string(), 1209600, 7);
        r3.signal_counts.insert("JobPost".to_string(), 6.0);

        let mut current = FeatureRow::new("e1".to_string(), "company".to_string(), 1814400, 7);
        current.signal_counts.insert("JobPost".to_string(), 8.0);

        compute_volatility(&[r1, r2, r3], &mut current);

        assert!((current.volatility["JobPost"] - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_compute_volatility_empty_history() {
        let mut current = FeatureRow::new("e1".to_string(), "company".to_string(), 0, 7);
        current.signal_counts.insert("JobPost".to_string(), 5.0);

        compute_volatility(&[], &mut current);

        assert!(current.volatility.is_empty());
    }

    #[test]
    fn test_compute_topic_drift_identical() {
        let mut prev = FeatureRow::new("e1".to_string(), "company".to_string(), 0, 7);
        prev.topic_drift.insert("automotive".to_string(), 0.8);
        prev.topic_drift.insert("quality".to_string(), 0.5);

        let mut curr = FeatureRow::new("e1".to_string(), "company".to_string(), 604800, 7);
        curr.topic_drift.insert("automotive".to_string(), 0.8);
        curr.topic_drift.insert("quality".to_string(), 0.5);

        compute_topic_drift(&prev, &mut curr);

        let drift = curr.topic_drift["__drift__"];
        assert!(drift.abs() < 0.001);
    }

    #[test]
    fn test_compute_topic_drift_different() {
        let mut prev = FeatureRow::new("e1".to_string(), "company".to_string(), 0, 7);
        prev.topic_drift.insert("automotive".to_string(), 1.0);

        let mut curr = FeatureRow::new("e1".to_string(), "company".to_string(), 604800, 7);
        curr.topic_drift.insert("aerospace".to_string(), 1.0);

        compute_topic_drift(&prev, &mut curr);

        let drift = curr.topic_drift["__drift__"];
        assert!((drift - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_compute_topic_drift_partial_overlap() {
        let mut prev = FeatureRow::new("e1".to_string(), "company".to_string(), 0, 7);
        prev.topic_drift.insert("automotive".to_string(), 0.5);
        prev.topic_drift.insert("quality".to_string(), 0.5);

        let mut curr = FeatureRow::new("e1".to_string(), "company".to_string(), 604800, 7);
        curr.topic_drift.insert("automotive".to_string(), 0.5);
        curr.topic_drift.insert("aerospace".to_string(), 0.5);

        compute_topic_drift(&prev, &mut curr);

        let drift = curr.topic_drift["__drift__"];
        assert!(drift > 0.0);
        assert!(drift < 1.0);
    }

    #[test]
    fn test_feature_row_serialization() {
        let mut row = FeatureRow::new("e1".to_string(), "company".to_string(), 1700000000, 7);
        row.signal_counts.insert("JobPost".to_string(), 5.0);
        row.poi_pain_index = Some(0.7);

        let json = serde_json::to_string(&row).unwrap();
        let deser: FeatureRow = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.entity_id, "e1");
        assert_eq!(deser.signal_counts["JobPost"], 5.0);
        assert_eq!(deser.poi_pain_index, Some(0.7));
    }
}
