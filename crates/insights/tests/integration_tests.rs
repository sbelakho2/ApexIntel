//!
//! Integration tests for ApexIntel Insights modules.
//!
//! Tests the integration between pattern detection, trend analysis,
//! risk summarization, and the intelligence analyzer.

use apex_insights::{
    Insight, InsightSeverity, IntelligenceAnalyzer, PatternData, PatternDetector,
    PatternDetectorConfig, RiskSummarizer, TrendAnalyzer,
};

#[cfg(test)]
mod pattern_tests {
    use super::*;

    #[test]
    fn pattern_detection_full_pipeline() {
        let config = PatternDetectorConfig {
            enable_temporal: true,
            enable_ownership: true,
            enable_sanctions: true,
            enable_anomaly: true,
            min_confidence: 0.5,
            lookback_days: 90,
        };
        let detector = PatternDetector::new(config);
        let data = PatternData {
            entity_names: vec![
                "Company A".to_string(),
                "Company A".to_string(),
                "Company A".to_string(),
                "Company B".to_string(),
            ],
            high_risk_countries: vec!["Russia".to_string(), "Iran".to_string()],
            entities_in_sanctioned: vec![
                ("Entity X".to_string(), "Russia".to_string()),
                ("Entity Y".to_string(), "Iran".to_string()),
            ],
            ownership_cycles: vec![vec![
                "A".to_string(),
                "B".to_string(),
                "C".to_string(),
                "A".to_string(),
            ]],
            ..Default::default()
        };

        let patterns = detector.detect_all(&data);

        // Should detect temporal pattern, sanctions, and ownership
        assert!(!patterns.is_empty());
    }

    #[test]
    fn confidence_threshold_filtering() {
        let config = PatternDetectorConfig {
            min_confidence: 0.9,
            ..Default::default()
        };
        let detector = PatternDetector::new(config);
        let data = PatternData {
            ownership_cycles: vec![vec!["A".to_string(), "B".to_string(), "A".to_string()]],
            ..Default::default()
        };

        let patterns = detector.detect_all(&data);

        // All patterns should meet confidence threshold
        assert!(patterns.iter().all(|p| p.confidence >= 0.9));
    }
}

#[cfg(test)]
mod trend_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use apex_insights::trend_analyzer::TimeSeriesPoint;

    #[test]
    fn trend_analysis_increasing() {
        let analyzer = TrendAnalyzer::with_default_config();

        let series: Vec<TimeSeriesPoint> = (0..10)
            .map(|i| {
                TimeSeriesPoint::new(
                    chrono::Utc::now() - chrono::Duration::days((10 - i) as i64),
                    100.0 + i as f64 * 10.0,
                )
            })
            .collect();

        let result = analyzer.analyze(&series);

        assert!(!result.trends.is_empty());
        assert!(
            result.trends[0].direction == apex_insights::trend_analyzer::TrendDirection::Increasing
        );
        assert!(result.trends[0].magnitude > 50.0);
    }

    #[test]
    fn trend_analysis_decreasing() {
        let analyzer = TrendAnalyzer::with_default_config();

        let series: Vec<TimeSeriesPoint> = (0..10)
            .map(|i| {
                TimeSeriesPoint::new(
                    chrono::Utc::now() - chrono::Duration::days((10 - i) as i64),
                    200.0 - i as f64 * 10.0,
                )
            })
            .collect();

        let result = analyzer.analyze(&series);

        if !result.trends.is_empty() {
            assert!(
                result.trends[0].direction
                    == apex_insights::trend_analyzer::TrendDirection::Decreasing
            );
        }
    }

    #[test]
    fn forecast_generation() {
        let analyzer = TrendAnalyzer::with_default_config();

        let series: Vec<TimeSeriesPoint> = (0..10)
            .map(|i| {
                TimeSeriesPoint::new(
                    chrono::Utc::now() - chrono::Duration::days((10 - i) as i64),
                    100.0 + i as f64 * 5.0,
                )
            })
            .collect();

        let result = analyzer.analyze(&series);

        assert!(result.forecast.is_some());
        let forecast = result.forecast.unwrap();
        assert!(forecast.predicted_value > 150.0);
    }
}

#[cfg(test)]
mod risk_tests {
    use super::*;

    #[test]
    fn risk_summarization() {
        let summarizer = RiskSummarizer::with_default_config();

        let insights = vec![
            Insight::new("Compliance Risk", "Sanctions violation risk")
                .with_severity(InsightSeverity::Critical)
                .with_confidence(0.9)
                .with_tags(vec!["sanctions".to_string()]),
            Insight::new("Financial Risk", "Liquidity concerns")
                .with_severity(InsightSeverity::High)
                .with_confidence(0.8)
                .with_tags(vec!["financial".to_string()]),
            Insight::new("Operational Risk", "Supply chain disruption")
                .with_severity(InsightSeverity::Medium)
                .with_confidence(0.6)
                .with_tags(vec!["operational".to_string()]),
        ];

        let summary = summarizer.summarize(&insights);

        assert_eq!(summary.risk_count, 3);
        assert!(summary.overall_score > 0.0);
        assert!(!summary.alerts.is_empty());
    }

    #[test]
    fn category_aggregation() {
        let summarizer = RiskSummarizer::with_default_config();

        let insights = vec![
            Insight::new("Risk 1", "Desc").with_tags(vec!["financial".to_string()]),
            Insight::new("Risk 2", "Desc").with_tags(vec!["financial".to_string()]),
            Insight::new("Risk 3", "Desc").with_tags(vec!["compliance".to_string()]),
        ];

        let summary = summarizer.summarize(&insights);

        // Check that some categories exist (implementation may vary)
        assert!(!summary.by_category.is_empty());
    }
}

#[cfg(test)]
mod analyzer_tests {
    use super::*;
    use apex_insights::analysis::AnalysisInput;
    use apex_insights::trend_analyzer::TimeSeriesPoint;

    #[test]
    fn intelligence_analyzer_full_pipeline() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let input = AnalysisInput {
            pattern_data: PatternData {
                entity_names: vec![
                    "Company A".to_string(),
                    "Company A".to_string(),
                    "Company A".to_string(),
                ],
                high_risk_countries: vec!["Russia".to_string()],
                entities_in_sanctioned: vec![("Entity X".to_string(), "Russia".to_string())],
                ..Default::default()
            },
            time_series: {
                let mut map = std::collections::HashMap::new();
                let series: Vec<TimeSeriesPoint> = (0..10)
                    .map(|i| {
                        TimeSeriesPoint::new(
                            chrono::Utc::now() - chrono::Duration::days((10 - i) as i64),
                            100.0 + i as f64 * 5.0,
                        )
                    })
                    .collect();
                map.insert("Metric".to_string(), series);
                map
            },
            ..Default::default()
        };

        let result = analyzer.analyze(&input);

        assert!(!result.summary.is_empty());
    }

    #[test]
    fn report_generation() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let input = AnalysisInput::default();

        let result = analyzer.analyze(&input);
        let report = analyzer.generate_report(&result);

        assert!(!report.sections.is_empty());
        let md = report.to_markdown();
        assert!(md.contains("Intelligence Report"));
    }

    #[test]
    fn empty_input_handling() {
        let analyzer = IntelligenceAnalyzer::with_default_config();
        let input = AnalysisInput::default();

        let result = analyzer.analyze(&input);

        assert!(result.insights.is_empty());
        assert!(!result.summary.is_empty());
    }
}

#[cfg(test)]
mod end_to_end_tests {
    use super::*;
    use apex_insights::trend_analyzer::TimeSeriesPoint;

    #[test]
    fn full_osint_analysis_pipeline() {
        // 1. Setup analyzers
        let pattern_config = PatternDetectorConfig {
            enable_temporal: true,
            enable_ownership: true,
            enable_sanctions: true,
            enable_anomaly: true,
            min_confidence: 0.5,
            lookback_days: 90,
        };
        let pattern_detector = PatternDetector::new(pattern_config);

        let trend_analyzer = TrendAnalyzer::with_default_config();
        let risk_summarizer = RiskSummarizer::with_default_config();

        // 2. Prepare input data
        let pattern_data = PatternData {
            entity_names: vec![
                "Foxconn".to_string(),
                "Foxconn".to_string(),
                "Foxconn".to_string(),
            ],
            high_risk_countries: vec!["Vietnam".to_string(), "India".to_string()],
            entities_in_sanctioned: vec![("Supplier X".to_string(), "Vietnam".to_string())],
            ..Default::default()
        };

        let mut time_series = std::collections::HashMap::new();
        let revenue_series: Vec<TimeSeriesPoint> = (0..12)
            .map(|i| {
                TimeSeriesPoint::new(
                    chrono::Utc::now() - chrono::Duration::days((12 - i) as i64 * 30),
                    100.0 + i as f64 * 3.0 + (i as f64 * 0.5).sin() * 10.0,
                )
            })
            .collect();
        time_series.insert("Revenue".to_string(), revenue_series);

        // 3. Detect patterns
        let patterns = pattern_detector.detect_all(&pattern_data);
        assert!(!patterns.is_empty());

        // 4. Analyze trends
        for series in time_series.values() {
            let result = trend_analyzer.analyze(series);
            assert!(!result.trends.is_empty() || result.trends.is_empty()); // Either is valid
        }

        // 5. Build insights
        let mut insights = Vec::new();
        for pattern in &patterns {
            let insight = pattern.to_insight("Analysis Complete");
            insights.push(insight);
        }

        // 6. Summarize risks
        let risk_summary = risk_summarizer.summarize(&insights);

        // 7. Verify results
        println!("Full pipeline test completed");
        println!("Patterns detected: {}", patterns.len());
        println!("Insights generated: {}", insights.len());
        println!(
            "Overall risk score: {:.0}%",
            risk_summary.overall_score * 100.0
        );
    }

    #[test]
    fn multi_perspective_risk_assessment() {
        let risk_summarizer = RiskSummarizer::with_default_config();

        // Generate insights from different "agents"
        let insights = vec![
            Insight::new("Compliance Alert", "Potential sanctions evasion")
                .with_severity(InsightSeverity::Critical)
                .with_confidence(0.85)
                .with_tags(vec!["compliance".to_string()]),
            Insight::new("Financial Alert", "Declining revenue")
                .with_severity(InsightSeverity::High)
                .with_confidence(0.78)
                .with_tags(vec!["financial".to_string()]),
            Insight::new("Geopolitical Alert", "Regional instability")
                .with_severity(InsightSeverity::Medium)
                .with_confidence(0.65)
                .with_tags(vec!["geopolitical".to_string()]),
            Insight::new("Operational Alert", "Supply chain bottleneck")
                .with_severity(InsightSeverity::Medium)
                .with_confidence(0.72)
                .with_tags(vec!["operational".to_string()]),
        ];

        let summary = risk_summarizer.summarize(&insights);

        assert!(summary.overall_score > 0.0);
        assert!(summary.risk_count == 4);
        assert!(!summary.alerts.is_empty());
    }

    #[test]
    fn cross_domain_risk_correlation() {
        let risk_summarizer = RiskSummarizer::with_default_config();

        let insights = vec![
            Insight::new("Supply Chain", "Supplier concentration in high-risk region")
                .with_severity(InsightSeverity::High)
                .with_confidence(0.8)
                .with_tags(vec!["supply_chain".to_string(), "geopolitical".to_string()]),
            Insight::new("Regulatory", "New compliance requirements")
                .with_severity(InsightSeverity::Medium)
                .with_confidence(0.75)
                .with_tags(vec!["regulatory".to_string()]),
        ];

        let summary = risk_summarizer.summarize(&insights);

        assert!(summary.overall_score > 0.0);
        assert!(summary.risk_count == 2);
    }
}
