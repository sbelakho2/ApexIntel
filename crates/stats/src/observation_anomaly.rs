//! Observation volume anomaly detector.
//!
//! Alerts when the observation ingest rate for any observation type drops
//! below 50% of its 30-day moving average — indicating a possible data
//! source failure, page structure change, or blocking event.

use chrono::{DateTime, Datelike, Utc};
use serde::Serialize;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// A daily count of observations by type.
#[derive(Debug, Clone)]
pub struct DailyObsCount {
    pub observation_type: String,
    pub date: DateTime<Utc>,
    pub count: u64,
}

/// An anomaly alert for a specific observation type.
#[derive(Debug, Clone, Serialize)]
pub struct VolumeAnomaly {
    pub observation_type: String,
    pub current_count: u64,
    pub moving_average: f64,
    pub seasonal_expected: f64,
    pub drop_pct: f64,
    pub severity: AnomalySeverity,
    pub message: String,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum AnomalySeverity {
    Warning,  // 50-75% drop
    Critical, // >75% drop
    Outage,   // 0 observations
}

// ─────────────────────────────────────────────────────────────────────────────
// Detection
// ─────────────────────────────────────────────────────────────────────────────

/// Detect volume anomalies from daily observation counts.
///
/// Expects `counts` sorted by date ascending, covering at least 30 days of history
/// plus the current day's count.
pub fn detect_volume_anomalies(
    counts: &[DailyObsCount],
    window_days: usize,
    threshold_pct: f64,
) -> Vec<VolumeAnomaly> {
    let window = if window_days == 0 { 30 } else { window_days };
    let threshold = if threshold_pct == 0.0 {
        0.5
    } else {
        threshold_pct
    };

    // Group by observation type
    let mut by_type: HashMap<&str, Vec<&DailyObsCount>> = HashMap::new();
    for count in counts {
        by_type
            .entry(&count.observation_type)
            .or_default()
            .push(count);
    }

    let mut anomalies = Vec::new();

    for (obs_type, type_counts) in &by_type {
        // `<= window` avoids the `window + 1` overflow for huge window values
        // and guarantees the index arithmetic below cannot underflow.
        if type_counts.len() <= window {
            continue; // Not enough history
        }

        // Most recent count is "current", preceding `window` entries form the average
        let Some(current) = type_counts.last() else {
            continue;
        };
        let end = type_counts.len() - 1;
        let history = &type_counts[end - window..end];

        let avg: f64 = history.iter().map(|c| c.count as f64).sum::<f64>() / history.len() as f64;
        let matching_weekday: Vec<&DailyObsCount> = history
            .iter()
            .copied()
            .filter(|count| count.date.weekday() == current.date.weekday())
            .collect();
        let seasonal_expected = if matching_weekday.len() >= 3 {
            matching_weekday.iter().map(|c| c.count as f64).sum::<f64>()
                / matching_weekday.len() as f64
        } else {
            avg
        };

        if seasonal_expected < 1.0 {
            continue; // Type has minimal volume, skip
        }

        let drop_pct = if seasonal_expected > 0.0 {
            (1.0 - current.count as f64 / seasonal_expected) * 100.0
        } else {
            0.0
        };

        if current.count as f64 <= seasonal_expected * threshold {
            let severity = if current.count == 0 {
                AnomalySeverity::Outage
            } else if drop_pct >= 75.0 {
                AnomalySeverity::Critical
            } else {
                AnomalySeverity::Warning
            };

            let message = match severity {
                AnomalySeverity::Outage => format!(
                    "OUTAGE: Zero '{}' observations — data source may be offline (seasonal baseline: {:.0}/day, rolling avg: {:.0}/day)",
                    obs_type, seasonal_expected, avg
                ),
                AnomalySeverity::Critical => format!(
                    "CRITICAL DROP: '{}' observations at {} vs seasonal expectation of {:.0} (rolling avg {:.0}, {:.1}% drop)",
                    obs_type, current.count, seasonal_expected, avg, drop_pct
                ),
                AnomalySeverity::Warning => format!(
                    "Volume drop: '{}' observations at {} vs seasonal expectation of {:.0} (rolling avg {:.0}, {:.1}% drop)",
                    obs_type, current.count, seasonal_expected, avg, drop_pct
                ),
            };

            anomalies.push(VolumeAnomaly {
                observation_type: obs_type.to_string(),
                current_count: current.count,
                moving_average: (avg * 10.0).round() / 10.0,
                seasonal_expected: (seasonal_expected * 10.0).round() / 10.0,
                drop_pct: (drop_pct * 10.0).round() / 10.0,
                severity,
                message,
                detected_at: Utc::now(),
            });
        }
    }

    anomalies.sort_by(|a, b| b.drop_pct.total_cmp(&a.drop_pct));
    anomalies
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Duration};

    fn make_counts(obs_type: &str, daily_counts: &[u64]) -> Vec<DailyObsCount> {
        let now = Utc::now();
        daily_counts
            .iter()
            .enumerate()
            .map(|(i, &count)| DailyObsCount {
                observation_type: obs_type.into(),
                date: now - Duration::days((daily_counts.len() - 1 - i) as i64),
                count,
            })
            .collect()
    }

    #[test]
    fn detect_outage() {
        let mut counts: Vec<u64> = vec![100; 31];
        counts.push(0); // current day = 0
        let data = make_counts("web_scrape", &counts);
        let anomalies = detect_volume_anomalies(&data, 30, 0.5);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].severity, AnomalySeverity::Outage);
    }

    #[test]
    fn detect_critical_drop() {
        let mut counts: Vec<u64> = vec![100; 31];
        counts.push(10); // 90% drop
        let data = make_counts("social_signal", &counts);
        let anomalies = detect_volume_anomalies(&data, 30, 0.5);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].severity, AnomalySeverity::Critical);
    }

    #[test]
    fn normal_volume_no_alert() {
        let mut counts: Vec<u64> = vec![100; 31];
        counts.push(95); // within normal range
        let data = make_counts("web_scrape", &counts);
        let anomalies = detect_volume_anomalies(&data, 30, 0.5);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn warning_threshold() {
        let mut counts: Vec<u64> = vec![100; 31];
        counts.push(40); // 60% drop — below 50% threshold
        let data = make_counts("news_crawl", &counts);
        let anomalies = detect_volume_anomalies(&data, 30, 0.5);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].severity, AnomalySeverity::Warning);
    }

    #[test]
    fn insufficient_history_skipped() {
        let data = make_counts("short", &[10, 20, 30]);
        let anomalies = detect_volume_anomalies(&data, 30, 0.5);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn weekday_baseline_prevents_false_alert_on_weekend_pattern() {
        let now = Utc::now();
        let mut data = Vec::new();
        for offset in 0..35 {
            let date = now - Duration::days((34 - offset) as i64);
            let count = if matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
                12
            } else {
                100
            };
            data.push(DailyObsCount {
                observation_type: "web_scrape".into(),
                date,
                count,
            });
        }
        let anomalies = detect_volume_anomalies(&data, 30, 0.5);
        assert!(
            anomalies.is_empty(),
            "seasonal weekday baseline should suppress routine weekend dips"
        );
    }
}
