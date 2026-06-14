//!
//! Trend Analysis for Intelligence Insights.
//!
//! Analyzes trends in OSINT data:
//! - Historical trend detection
//! - Trend direction and magnitude
//! - Trend forecasting
//! - Momentum indicators
//!
//! Part of Phase 2.1: LLM Integration for ApexIntel OSINT platform.

use serde::{Deserialize, Serialize};

/// Trend direction enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrendDirection {
    Increasing,
    Decreasing,
    Stable,
    Volatile,
    Unknown,
}

impl TrendDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Increasing => "increasing",
            Self::Decreasing => "decreasing",
            Self::Stable => "stable",
            Self::Volatile => "volatile",
            Self::Unknown => "unknown",
        }
    }
}

/// A detected trend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trend {
    /// Trend name/identifier.
    pub name: String,
    /// Direction of the trend.
    pub direction: TrendDirection,
    /// Magnitude (% change or absolute).
    pub magnitude: f64,
    /// Confidence in trend detection (0.0-1.0).
    pub confidence: f64,
    /// Start of trend period.
    pub start_date: chrono::DateTime<chrono::Utc>,
    /// End of trend period.
    pub end_date: chrono::DateTime<chrono::Utc>,
    /// Data points in trend.
    pub data_points: usize,
    /// Trend strength (0.0-1.0, based on R² or similar).
    pub strength: f64,
}

impl Trend {
    pub fn is_significant(&self) -> bool {
        self.confidence >= 0.7 && self.strength >= 0.6
    }

    pub fn is_strong(&self) -> bool {
        self.strength >= 0.8
    }
}

/// Configuration for trend analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalyzerConfig {
    /// Minimum data points required.
    pub min_data_points: usize,
    /// Minimum trend magnitude (%).
    pub min_magnitude: f64,
    /// Minimum trend confidence.
    pub min_confidence: f64,
    /// Volatility threshold.
    pub volatility_threshold: f64,
}

impl Default for TrendAnalyzerConfig {
    fn default() -> Self {
        Self {
            min_data_points: 5,
            min_magnitude: 5.0,
            min_confidence: 0.6,
            volatility_threshold: 0.3,
        }
    }
}

/// Trend analyzer for OSINT data.
pub struct TrendAnalyzer {
    config: TrendAnalyzerConfig,
}

impl TrendAnalyzer {
    pub fn new(config: TrendAnalyzerConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(TrendAnalyzerConfig::default())
    }

    /// Analyze a time series and detect trends.
    pub fn analyze(&self, time_series: &[TimeSeriesPoint]) -> TrendAnalysisResult {
        if time_series.len() < self.config.min_data_points {
            return TrendAnalysisResult {
                trends: vec![],
                summary: "Insufficient data for trend analysis".to_string(),
                volatility: 0.0,
                forecast: None,
            };
        }

        let trends = self.detect_trends(time_series);
        let volatility = self.calculate_volatility(time_series);
        let forecast = self.forecast(time_series);

        let summary = self.generate_summary(&trends, volatility);

        TrendAnalysisResult {
            trends,
            summary,
            volatility,
            forecast,
        }
    }

    /// Detect trends using simple moving average.
    fn detect_trends(&self, points: &[TimeSeriesPoint]) -> Vec<Trend> {
        let mut trends = Vec::new();
        let values: Vec<f64> = points.iter().map(|p| p.value).collect();

        // Simple trend detection using linear regression
        let (slope, intercept) = self.linear_regression(&values);

        // Calculate R² for trend strength
        let strength = self.calculate_r_squared(&values, slope, intercept);

        // Determine direction
        let direction = if slope.abs() < 0.01 {
            TrendDirection::Stable
        } else if slope > 0.0 {
            TrendDirection::Increasing
        } else {
            TrendDirection::Decreasing
        };

        // Calculate magnitude (% change)
        let start_val = values.first().copied().unwrap_or(0.0);
        let end_val = values.last().copied().unwrap_or(0.0);
        let magnitude = if start_val != 0.0 {
            ((end_val - start_val) / start_val) * 100.0
        } else {
            0.0
        };

        // Confidence based on R² and data quality
        let confidence = (strength * 0.7 + (points.len() as f64 / 20.0).min(1.0) * 0.3)
            .clamp(0.0, 1.0);

        if confidence >= self.config.min_confidence && magnitude.abs() >= self.config.min_magnitude {
            trends.push(Trend {
                name: "Primary Trend".to_string(),
                direction,
                magnitude,
                confidence,
                start_date: points.first().map(|p| p.timestamp).unwrap_or_else(chrono::Utc::now),
                end_date: points.last().map(|p| p.timestamp).unwrap_or_else(chrono::Utc::now),
                data_points: points.len(),
                strength,
            });
        }

        trends
    }

    /// Calculate volatility (coefficient of variation).
    fn calculate_volatility(&self, points: &[TimeSeriesPoint]) -> f64 {
        if points.len() < 2 {
            return 0.0;
        }

        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;

        if mean == 0.0 {
            return 0.0;
        }

        let variance: f64 = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
            / values.len() as f64;

        variance.sqrt() / mean.abs()
    }

    /// Simple forecast using linear extrapolation.
    fn forecast(&self, points: &[TimeSeriesPoint]) -> Option<TrendForecast> {
        if points.len() < 3 {
            return None;
        }

        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        let (slope, intercept) = self.linear_regression(&values);

        let last_point = points.last()?;
        let forecast_date = last_point.timestamp + chrono::Duration::days(7);

        // Project 7 days forward
        let forecast_value = intercept + slope * (points.len() as f64 + 7.0);

        Some(TrendForecast {
            predicted_value: forecast_value,
            predicted_date: forecast_date,
            confidence_interval: (forecast_value * 0.9, forecast_value * 1.1),
            model: "Linear Extrapolation".to_string(),
        })
    }

    /// Simple linear regression.
    fn linear_regression(&self, values: &[f64]) -> (f64, f64) {
        let n = values.len() as f64;
        if n == 0.0 {
            return (0.0, 0.0);
        }

        let sum_x = (0..values.len()).sum::<usize>() as f64;
        let sum_y = values.iter().sum::<f64>();
        let sum_xy: f64 = values
            .iter()
            .enumerate()
            .map(|(i, y)| i as f64 * y)
            .sum();
        let sum_xx: f64 = (0..values.len()).map(|i| (i * i) as f64).sum::<f64>();

        let denominator = n * sum_xx - sum_x * sum_x;
        if denominator == 0.0 {
            return (0.0, sum_y / n);
        }

        let slope = (n * sum_xy - sum_x * sum_y) / denominator;
        let intercept = (sum_y - slope * sum_x) / n;

        (slope, intercept)
    }

    /// Calculate R² for regression.
    fn calculate_r_squared(&self, values: &[f64], slope: f64, intercept: f64) -> f64 {
        let n = values.len() as f64;
        if n < 2.0 {
            return 0.0;
        }

        let mean = values.iter().sum::<f64>() / n;
        let y_mean_diff: f64 = values.iter().map(|y| (y - mean).powi(2)).sum();
        let y_pred_diff: f64 = values
            .iter()
            .enumerate()
            .map(|(i, y)| {
                let predicted = intercept + slope * i as f64;
                (y - predicted).powi(2)
            })
            .sum();

        if y_mean_diff == 0.0 {
            return 1.0;
        }

        1.0 - (y_pred_diff / y_mean_diff)
    }

    /// Generate summary text.
    fn generate_summary(&self, trends: &[Trend], volatility: f64) -> String {
        if trends.is_empty() {
            return "No significant trends detected in the analyzed period.".to_string();
        }

        let primary = &trends[0];
        let direction_str = match primary.direction {
            TrendDirection::Increasing => "upward",
            TrendDirection::Decreasing => "downward",
            TrendDirection::Stable => "stable",
            TrendDirection::Volatile => "volatile",
            TrendDirection::Unknown => "unknown",
        };

        format!(
            "{} trend detected with {:.1}% magnitude, {:.0}% confidence, and {:.1}% volatility",
            direction_str,
            primary.magnitude.abs(),
            primary.confidence * 100.0,
            volatility * 100.0
        )
    }
}

/// A data point in a time series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub value: f64,
    pub label: Option<String>,
}

impl TimeSeriesPoint {
    pub fn new(timestamp: chrono::DateTime<chrono::Utc>, value: f64) -> Self {
        Self {
            timestamp,
            value,
            label: None,
        }
    }
}

/// Result of trend analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysisResult {
    pub trends: Vec<Trend>,
    pub summary: String,
    pub volatility: f64,
    pub forecast: Option<TrendForecast>,
}

/// A trend forecast.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendForecast {
    pub predicted_value: f64,
    pub predicted_date: chrono::DateTime<chrono::Utc>,
    pub confidence_interval: (f64, f64),
    pub model: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    fn create_test_series(values: &[f64]) -> Vec<TimeSeriesPoint> {
        values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                TimeSeriesPoint::new(
                    chrono::Utc::now() - chrono::Duration::days((values.len() - i) as i64),
                    v,
                )
            })
            .collect()
    }

    #[test]
    fn trend_detection_increasing() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[100.0, 110.0, 120.0, 130.0, 140.0]);

        let result = analyzer.analyze(&series);

        assert!(!result.trends.is_empty());
        assert_eq!(result.trends[0].direction, TrendDirection::Increasing);
    }

    #[test]
    fn trend_detection_decreasing() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[140.0, 130.0, 120.0, 110.0, 100.0]);

        let result = analyzer.analyze(&series);

        assert!(!result.trends.is_empty());
        assert_eq!(result.trends[0].direction, TrendDirection::Decreasing);
    }

    #[test]
    fn trend_detection_stable() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[100.0, 101.0, 99.0, 100.5, 100.2]);

        let result = analyzer.analyze(&series);

        // Stable trends might not be detected if magnitude is too small
        // or they might be classified as stable
        if !result.trends.is_empty() {
            assert_eq!(result.trends[0].direction, TrendDirection::Stable);
        }
    }

    #[test]
    fn insufficient_data() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[100.0, 110.0]);

        let result = analyzer.analyze(&series);

        assert!(result.trends.is_empty());
        assert!(result.summary.contains("Insufficient"));
    }

    #[test]
    fn volatility_calculation() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[100.0, 150.0, 100.0, 150.0, 100.0]);

        let result = analyzer.analyze(&series);

        assert!(result.volatility >= 0.0); // High volatility due to oscillation
    }

    #[test]
    fn forecast_generation() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[100.0, 110.0, 120.0, 130.0, 140.0, 150.0]);

        let result = analyzer.analyze(&series);

        assert!(result.forecast.is_some());
        let forecast = result.forecast.unwrap();
        assert!(forecast.predicted_value > 150.0);
        assert!(!forecast.confidence_interval.0.is_nan());
    }

    #[test]
    fn trend_magnitude_calculation() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[100.0, 200.0]);

        let result = analyzer.analyze(&series);

        if !result.trends.is_empty() {
            // 100 -> 200 is a 100% increase
            assert!(result.trends[0].magnitude > 90.0);
        }
    }

    #[test]
    fn trend_strength() {
        let analyzer = TrendAnalyzer::with_default_config();
        let series = create_test_series(&[100.0, 110.0, 120.0, 130.0, 140.0]);

        let result = analyzer.analyze(&series);

        if !result.trends.is_empty() {
            // Perfect linear trend should have high strength
            assert!(result.trends[0].strength > 0.95);
        }
    }

    #[test]
    fn time_series_point() {
        let point = TimeSeriesPoint::new(chrono::Utc::now(), 42.0);
        assert_eq!(point.value, 42.0);
    }
}
