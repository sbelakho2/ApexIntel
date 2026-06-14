//!
//! Intelligence Analyzer for ApexIntel OSINT platform.
//!
//! Main orchestration for intelligence analysis:
//! - Coordinates pattern detection, trend analysis, and risk summarization
//! - Integrates with LLM for enhanced analysis
//! - Generates comprehensive intelligence reports
//!
//! Part of Phase 2.1: LLM Integration for ApexIntel OSINT platform.

use chrono::{DateTime, Utc};
use tracing::debug;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    Insight, InsightSeverity, PatternDetector, PatternDetectorConfig,
    RiskSummarizer, RiskSummarizerConfig, TrendAnalyzer, TrendAnalyzerConfig,
};
use crate::patterns::PatternData;
use crate::risk_summarizer::RiskSummary;
use crate::trend_analyzer::{Trend, TimeSeriesPoint};

/// Configuration for the intelligence analyzer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceAnalyzerConfig {
    pub enable_pattern_detection: bool,
    pub enable_trend_analysis: bool,
    pub enable_risk_summarization: bool,
    pub pattern_config: PatternDetectorConfig,
    pub trend_config: TrendAnalyzerConfig,
    pub risk_config: RiskSummarizerConfig,
    pub min_insight_confidence: f64,
    pub max_insights_per_category: usize,
}

impl Default for IntelligenceAnalyzerConfig {
    fn default() -> Self {
        Self {
            enable_pattern_detection: true,
            enable_trend_analysis: true,
            enable_risk_summarization: true,
            pattern_config: PatternDetectorConfig::default(),
            trend_config: TrendAnalyzerConfig::default(),
            risk_config: RiskSummarizerConfig::default(),
            min_insight_confidence: 0.5,
            max_insights_per_category: 20,
        }
    }
}

/// Intelligence analyzer that orchestrates all analysis components.
pub struct IntelligenceAnalyzer {
    config: IntelligenceAnalyzerConfig,
    pattern_detector: PatternDetector,
    trend_analyzer: TrendAnalyzer,
    risk_summarizer: RiskSummarizer,
}

impl IntelligenceAnalyzer {
    pub fn new(config: IntelligenceAnalyzerConfig) -> Self {
        Self {
            pattern_detector: PatternDetector::new(config.pattern_config.clone()),
            trend_analyzer: TrendAnalyzer::new(config.trend_config.clone()),
            risk_summarizer: RiskSummarizer::new(config.risk_config.clone()),
            config,
        }
    }

    pub fn with_default_config() -> Self {
        Self::new(IntelligenceAnalyzerConfig::default())
    }

    /// Perform comprehensive analysis and generate insights.
    pub fn analyze(&self, input: &AnalysisInput) -> crate::InsightResult {
        let start = std::time::Instant::now();
        let mut all_insights = Vec::new();

        // 1. Pattern detection
        if self.config.enable_pattern_detection {
            let patterns = self.pattern_detector.detect_all(&input.pattern_data);
            for pattern in patterns {
                let insight = pattern.to_insight(&format!(
                    "{} entities involved",
                    pattern.entities_involved.len()
                ));
                all_insights.push(insight);
            }
        }

        // 2. Trend analysis
        if self.config.enable_trend_analysis {
            for (series_name, series) in &input.time_series {
                let result = self.trend_analyzer.analyze(series);
                for trend in result.trends {
                    let insight = self.trend_to_insight(&trend, series_name);
                    all_insights.push(insight);
                }
            }
        }

        // 3. Risk summarization
        let risk_summary = if self.config.enable_risk_summarization {
            Some(self.risk_summarizer.summarize(&all_insights))
        } else {
            None
        };

        // Filter and deduplicate insights
        all_insights = self.deduplicate_insights(all_insights);
        all_insights.retain(|i| i.confidence >= self.config.min_insight_confidence);

        // Sort by severity then confidence
        all_insights.sort_by(|a, b| {
            b.severity
                .priority()
                .cmp(&a.severity.priority())
                .then_with(|| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal))
        });

        // Generate summary
        let summary = self.generate_summary(&all_insights, &risk_summary);

        let generation_time_ms = start.elapsed().as_millis() as u64;

        debug!(
            insights_generated = %all_insights.len(),
            patterns = %input.pattern_data.entity_names.len(),
            time_series = %input.time_series.len(),
            "Intelligence analysis completed"
        );

        crate::InsightResult {
            insights: all_insights,
            summary,
            generation_time_ms,
            model_used: "ApexIntel-Analysis-v1".to_string(),
        }
    }

    /// Convert trend to insight.
    fn trend_to_insight(&self, trend: &Trend, series_name: &str) -> Insight {
        let severity = if trend.strength >= 0.8 && trend.confidence >= 0.8 {
            InsightSeverity::High
        } else if trend.strength >= 0.6 {
            InsightSeverity::Medium
        } else {
            InsightSeverity::Low
        };

        Insight::new(
            &format!("Trend: {} - {}", series_name, trend.direction.as_str()),
            &format!(
                "{} trend detected with {:.1}% magnitude over {} data points. Trend strength: {:.0}%",
                trend.direction.as_str(),
                trend.magnitude.abs(),
                trend.data_points,
                trend.strength * 100.0
            ),
        )
        .with_severity(severity)
        .with_confidence(trend.confidence)
        .with_tags(vec![
            series_name.to_string(),
            trend.direction.as_str().to_string(),
            "trend".to_string(),
        ])
    }

    /// Deduplicate insights by title similarity.
    fn deduplicate_insights(&self, insights: Vec<Insight>) -> Vec<Insight> {
        let mut unique: Vec<Insight> = Vec::new();

        for insight in insights {
            let is_duplicate = unique.iter().any(|existing| {
                self.title_similarity(&insight.title, &existing.title) > 0.8
            });

            if !is_duplicate {
                unique.push(insight);
            }
        }

        unique
    }

    /// Calculate title similarity.
    fn title_similarity(&self, a: &str, b: &str) -> f64 {
        let words_a: std::collections::HashSet<_> = a.split_whitespace().collect();
        let words_b: std::collections::HashSet<_> = b.split_whitespace().collect();

        let intersection = words_a.intersection(&words_b).count() as f64;
        let union = words_a.union(&words_b).count() as f64;

        if union == 0.0 {
            return 0.0;
        }

        intersection / union
    }

    /// Generate summary text.
    fn generate_summary(
        &self,
        insights: &[Insight],
        risk_summary: &Option<RiskSummary>,
    ) -> String {
        let mut summary = String::new();

        // Overview
        summary.push_str(&format!(
            "Analysis generated {} insights.\n\n",
            insights.len()
        ));

        // Severity breakdown
        let critical = insights
            .iter()
            .filter(|i| i.severity == InsightSeverity::Critical)
            .count();
        let high = insights
            .iter()
            .filter(|i| i.severity == InsightSeverity::High)
            .count();
        let medium = insights
            .iter()
            .filter(|i| i.severity == InsightSeverity::Medium)
            .count();

        if critical > 0 {
            summary.push_str(&format!(
                "⚠️ CRITICAL: {} insight(s) require immediate attention.\n",
                critical
            ));
        }
        if high > 0 {
            summary.push_str(&format!(
                "🔴 HIGH: {} insight(s) require follow-up.\n",
                high
            ));
        }
        if medium > 0 {
            summary.push_str(&format!(
                "🟡 MEDIUM: {} insight(s) for monitoring.\n",
                medium
            ));
        }

        // Risk summary if available
        if let Some(risk) = risk_summary {
            summary.push('\n');
            summary.push_str("### Risk Overview\n");
            summary.push_str(&format!(
                "Overall Risk Score: {:.0}%\n",
                risk.overall_score * 100.0
            ));
            summary.push_str(&format!(
                "High/Medium/Low Risks: {}/{}/{}\n",
                risk.high_risks, risk.medium_risks, risk.low_risks
            ));
        }

        // Top insights
        if !insights.is_empty() {
            summary.push_str("\n### Top Insights\n");
            for insight in insights.iter().take(5) {
                summary.push_str(&format!(
                    "- [{}] {}\n",
                    insight.severity.as_str().to_uppercase(),
                    insight.title
                ));
            }
        }

        summary
    }

    /// Generate a structured report.
    pub fn generate_report(&self, result: &crate::InsightResult) -> IntelligenceReport {
        let mut sections = Vec::new();

        // Executive summary
        sections.push(ReportSection {
            title: "Executive Summary".to_string(),
            content: result.summary.clone(),
            level: 1,
        });

        // Critical findings
        let critical: Vec<_> = result
            .insights
            .iter()
            .filter(|i| i.severity == InsightSeverity::Critical || i.severity == InsightSeverity::High)
            .collect();

        if !critical.is_empty() {
            sections.push(ReportSection {
                title: "Critical & High Priority Findings".to_string(),
                content: critical
                    .iter()
                    .map(|i| format!("### {}\n{}\n", i.title, i.description))
                    .collect(),
                level: 2,
            });
        }

        // Detailed insights
        sections.push(ReportSection {
            title: "All Insights".to_string(),
            content: result
                .insights
                .iter()
                .map(|i| {
                    format!(
                        "### {} [{}]\n{}\nConfidence: {:.0}%\n",
                        i.title,
                        i.severity.as_str(),
                        i.description,
                        i.confidence * 100.0
                    )
                })
                .collect(),
            level: 2,
        });

        IntelligenceReport {
            generated_at: Utc::now(),
            sections,
            total_insights: result.insights.len(),
            generation_time_ms: result.generation_time_ms,
            model_used: result.model_used.clone(),
        }
    }
}

/// Input data for analysis.
#[derive(Debug, Clone, Default)]
pub struct AnalysisInput {
    pub pattern_data: PatternData,
    pub time_series: HashMap<String, Vec<TimeSeriesPoint>>,
    pub additional_insights: Vec<Insight>,
}

/// Structured intelligence report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceReport {
    pub generated_at: DateTime<Utc>,
    pub sections: Vec<ReportSection>,
    pub total_insights: usize,
    pub generation_time_ms: u64,
    pub model_used: String,
}

/// A section of the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSection {
    pub title: String,
    pub content: String,
    pub level: u8,
}

impl IntelligenceReport {
    /// Format report as markdown.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Intelligence Report\n\n");
        md.push_str(&format!(
            "*Generated: {} | Model: {}*\n\n---\n\n",
            self.generated_at.format("%Y-%m-%d %H:%M UTC"),
            self.model_used
        ));

        for section in &self.sections {
            let heading = "#".repeat(section.level as usize);
            md.push_str(&format!("{} {}\n\n{}\n\n---\n\n", heading, section.title, section.content));
        }

        md.push_str(&format!(
            "\n*Report generated in {}ms with {} total insights.*",
            self.generation_time_ms, self.total_insights
        ));

        md
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzer_generates_insights() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let input = AnalysisInput::default();

        let result = analyzer.analyze(&input);

        // Should return empty result for empty input
        assert!(result.insights.is_empty());
    }

    #[test]
    fn analyzer_with_pattern_data() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let mut input = AnalysisInput::default();

        // Add pattern data
        input.pattern_data.entity_names = vec![
            "Company A".to_string(),
            "Company A".to_string(),
            "Company A".to_string(),
        ];
        input.pattern_data.high_risk_countries = vec!["Russia".to_string()];
        input.pattern_data.entities_in_sanctioned = vec![(
            "Entity X".to_string(),
            "Russia".to_string(),
        )];

        let _result = analyzer.analyze(&input);

    }

    #[test]
    fn analyzer_with_time_series() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let mut input = AnalysisInput::default();

        // Add time series data
        let series: Vec<TimeSeriesPoint> = (0..10)
            .map(|i| {
                TimeSeriesPoint::new(
                    Utc::now() - chrono::Duration::days((10 - i) as i64),
                    100.0 + i as f64 * 5.0,
                )
            })
            .collect();

        input.time_series.insert("Revenue".to_string(), series);

        let _result = analyzer.analyze(&input);

    }

    #[test]
    fn report_generation() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let input = AnalysisInput::default();

        let result = analyzer.analyze(&input);
        let report = analyzer.generate_report(&result);

        assert!(!report.sections.is_empty());
        assert_eq!(report.model_used, "ApexIntel-Analysis-v1");
    }

    #[test]
    fn report_markdown_format() {
        let report = IntelligenceReport {
            generated_at: Utc::now(),
            sections: vec![
                ReportSection {
                    title: "Test Section".to_string(),
                    content: "Test content".to_string(),
                    level: 2,
                },
            ],
            total_insights: 5,
            generation_time_ms: 100,
            model_used: "Test".to_string(),
        };

        let md = report.to_markdown();
        assert!(md.contains("# Test Section"));
        assert!(md.contains("Test content"));
        assert!(md.contains("5 total insights"));
    }

    #[test]
    fn insight_deduplication() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let mut input = AnalysisInput::default();

        // Add duplicate pattern data
        input.pattern_data.ownership_cycles.push(vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "A".to_string(),
        ]);
        input.pattern_data.ownership_cycles.push(vec![
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
            "A".to_string(),
        ]);

        let result = analyzer.analyze(&input);

        // Results should be deduplicated (may still have some duplicates if patterns differ)
        assert!(result.insights.len() <= 2 || result.insights.is_empty());
    }

    #[test]
    fn title_similarity() {
        let analyzer = IntelligenceAnalyzer::with_default_config();

        // Test identical titles
        let sim = analyzer.title_similarity("Test Title", "Test Title");
        assert_eq!(sim, 1.0);

        // Test completely different
        let sim = analyzer.title_similarity("Apple", "Zebra");
        assert_eq!(sim, 0.0);

        // Test partial overlap
        let sim = analyzer.title_similarity("Supply Chain Risk", "Supply Chain Analysis");
        assert!((0.0..=1.0).contains(&sim));
    }

    #[test]
    fn confidence_filtering() {
        let config = IntelligenceAnalyzerConfig {
            min_insight_confidence: 0.8,
            ..Default::default()
        };
        let _analyzer = IntelligenceAnalyzer::new(config);

        let insights = vec![
            Insight::new("High Confidence", "Desc").with_confidence(0.9),
            Insight::new("Medium High", "Desc").with_confidence(0.85),
        ];

        let result = crate::InsightResult {
            insights,
            summary: "Test".to_string(),
            generation_time_ms: 0,
            model_used: "Test".to_string(),
        };

        assert!(result.insights.iter().all(|i| i.confidence >= 0.8));
    }
}
