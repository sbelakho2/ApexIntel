//!
//! Risk Summarization for Intelligence Insights.
//!
//! Summarizes risk assessments from OSINT data:
//! - Risk score calculation
//! - Risk category aggregation
//! - Risk alert generation
//! - Risk trend tracking
//!
//! Part of Phase 2.1: LLM Integration for ApexIntel OSINT platform.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{Insight, InsightSeverity};

/// Risk category types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskCategory {
    Compliance,
    Financial,
    Reputational,
    Operational,
    Strategic,
    Geopolitical,
    Cyber,
    Legal,
}

impl RiskCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Compliance => "compliance",
            Self::Financial => "financial",
            Self::Reputational => "reputational",
            Self::Operational => "operational",
            Self::Strategic => "strategic",
            Self::Geopolitical => "geopolitical",
            Self::Cyber => "cyber",
            Self::Legal => "legal",
        }
    }
}

/// A risk item with scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskItem {
    pub category: RiskCategory,
    pub description: String,
    pub severity: InsightSeverity,
    pub likelihood: f64,
    pub impact: f64,
    pub score: f64,
    pub mitigations: Vec<String>,
}

impl RiskItem {
    pub fn calculate_score(&mut self) {
        // Risk = Likelihood × Impact × SeverityFactor
        let severity_factor = match self.severity {
            InsightSeverity::Critical => 1.0,
            InsightSeverity::High => 0.8,
            InsightSeverity::Medium => 0.5,
            InsightSeverity::Low => 0.3,
            InsightSeverity::Info => 0.1,
        };

        self.score = (self.likelihood * self.impact * severity_factor).clamp(0.0, 1.0);
    }
}

/// Configuration for risk summarization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummarizerConfig {
    pub high_risk_threshold: f64,
    pub medium_risk_threshold: f64,
    pub include_mitigations: bool,
    pub aggregate_by_category: bool,
}

impl Default for RiskSummarizerConfig {
    fn default() -> Self {
        Self {
            high_risk_threshold: 0.7,
            medium_risk_threshold: 0.4,
            include_mitigations: true,
            aggregate_by_category: true,
        }
    }
}

/// Risk summarizer for OSINT data.
pub struct RiskSummarizer {
    config: RiskSummarizerConfig,
}

impl RiskSummarizer {
    pub fn new(config: RiskSummarizerConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(RiskSummarizerConfig::default())
    }

    /// Summarize risks from a collection of insights.
    pub fn summarize(&self, insights: &[Insight]) -> RiskSummary {
        let mut risks = Vec::new();

        // Convert insights to risk items
        for insight in insights {
            let risk = self.insight_to_risk(insight);
            risks.push(risk);
        }

        // Calculate aggregate scores
        let overall_score = self.calculate_overall_score(&risks);
        let by_category = self.aggregate_by_category(&risks);

        // Generate alerts
        let alerts = self.generate_alerts(&risks);

        RiskSummary {
            overall_score,
            risk_count: risks.len(),
            high_risks: risks
                .iter()
                .filter(|r| r.score >= self.config.high_risk_threshold)
                .count(),
            medium_risks: risks
                .iter()
                .filter(|r| r.score >= self.config.medium_risk_threshold)
                .count(),
            low_risks: risks
                .iter()
                .filter(|r| r.score < self.config.medium_risk_threshold)
                .count(),
            by_category,
            top_risks: self.get_top_risks(&risks, 5),
            alerts,
        }
    }

    /// Convert an insight to a risk item.
    fn insight_to_risk(&self, insight: &Insight) -> RiskItem {
        let likelihood = insight.confidence; // Use confidence as proxy for likelihood
        let impact = match insight.severity {
            InsightSeverity::Critical => 0.9,
            InsightSeverity::High => 0.7,
            InsightSeverity::Medium => 0.5,
            InsightSeverity::Low => 0.3,
            InsightSeverity::Info => 0.1,
        };

        let mut risk = RiskItem {
            category: self.categorize_insight(insight),
            description: format!("{}: {}", insight.title, insight.description),
            severity: insight.severity,
            likelihood,
            impact,
            score: 0.0,
            mitigations: vec![],
        };

        risk.calculate_score();
        risk
    }

    /// Categorize an insight by its content.
    fn categorize_insight(&self, insight: &Insight) -> RiskCategory {
        let tags_lower: Vec<String> = insight.tags.iter().map(|t| t.to_lowercase()).collect();
        let desc_lower = insight.description.to_lowercase();

        if tags_lower.iter().any(|t| t.contains("sanction")) || desc_lower.contains("sanction") {
            RiskCategory::Compliance
        } else if tags_lower.iter().any(|t| t.contains("financial")) || desc_lower.contains("financial") {
            RiskCategory::Financial
        } else if tags_lower.iter().any(|t| t.contains("reputat")) || desc_lower.contains("reputat") {
            RiskCategory::Reputational
        } else if tags_lower.iter().any(|t| t.contains("operational")) || desc_lower.contains("operation") {
            RiskCategory::Operational
        } else if tags_lower.iter().any(|t| t.contains("geopolitical")) || desc_lower.contains("geopolitical") {
            RiskCategory::Geopolitical
        } else if tags_lower.iter().any(|t| t.contains("cyber")) || desc_lower.contains("cyber") {
            RiskCategory::Cyber
        } else if tags_lower.iter().any(|t| t.contains("legal")) || desc_lower.contains("legal") {
            RiskCategory::Legal
        } else {
            RiskCategory::Strategic
        }
    }

    /// Calculate overall risk score.
    fn calculate_overall_score(&self, risks: &[RiskItem]) -> f64 {
        if risks.is_empty() {
            return 0.0;
        }

        // Weighted average, emphasizing high risks
        let total_score: f64 = risks.iter().map(|r| r.score).sum();
        let count = risks.len() as f64;

        // Also consider the maximum risk
        let max_score = risks.iter().map(|r| r.score).fold(0.0f64, f64::max);

        // Combine average and max
        (total_score / count * 0.6 + max_score * 0.4).clamp(0.0, 1.0)
    }

    /// Aggregate risks by category.
    fn aggregate_by_category(&self, risks: &[RiskItem]) -> HashMap<RiskCategory, CategoryRisk> {
        let mut by_category: HashMap<RiskCategory, Vec<&RiskItem>> = HashMap::new();

        for risk in risks {
            by_category
                .entry(risk.category)
                .or_default()
                .push(risk);
        }

        by_category
            .into_iter()
            .map(|(category, items)| {
                let score = if items.is_empty() {
                    0.0
                } else {
                    items.iter().map(|r| r.score).sum::<f64>() / items.len() as f64
                };

                let category_risk = CategoryRisk {
                    category,
                    score,
                    risk_count: items.len(),
                    top_risk: items
                        .iter()
                        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
                        .map(|r| r.description.clone()),
                };

                (category, category_risk)
            })
            .collect()
    }

    /// Get top N risks by score.
    fn get_top_risks(&self, risks: &[RiskItem], n: usize) -> Vec<String> {
        let mut sorted = risks.to_vec();
        sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        sorted
            .into_iter()
            .take(n)
            .map(|r| format!("[{}] {}", r.category.as_str(), r.description))
            .collect()
    }

    /// Generate alerts for high-priority risks.
    fn generate_alerts(&self, risks: &[RiskItem]) -> Vec<RiskAlert> {
        let mut alerts = Vec::new();

        for risk in risks {
            if risk.score >= self.config.high_risk_threshold {
                alerts.push(RiskAlert {
                    severity: risk.severity,
                    message: format!(
                        "HIGH RISK: {} (Score: {:.0}%)",
                        risk.description,
                        risk.score * 100.0
                    ),
                    action_required: Some(self.suggest_action(&risk.category)),
                    category: risk.category,
                });
            }
        }

        // Sort by severity
        alerts.sort_by_key(|b| std::cmp::Reverse(b.severity.priority()));
        alerts
    }

    /// Suggest action based on category.
    fn suggest_action(&self, category: &RiskCategory) -> String {
        match category {
            RiskCategory::Compliance => "Review compliance controls and sanctions screening".to_string(),
            RiskCategory::Financial => "Conduct detailed financial due diligence".to_string(),
            RiskCategory::Reputational => "Monitor media and assess PR risk".to_string(),
            RiskCategory::Operational => "Review operational procedures and controls".to_string(),
            RiskCategory::Strategic => "Reassess strategic positioning and partnerships".to_string(),
            RiskCategory::Geopolitical => "Update geopolitical risk monitoring".to_string(),
            RiskCategory::Cyber => "Review cybersecurity posture".to_string(),
            RiskCategory::Legal => "Engage legal counsel".to_string(),
        }
    }

    /// Generate risk summary text.
    pub fn generate_summary_text(&self, summary: &RiskSummary) -> String {
        let mut text = String::new();

        text.push_str("## Risk Summary\n\n");
        text.push_str(&format!("Overall Risk Score: {:.0}%\n", summary.overall_score * 100.0));
        text.push_str(&format!("Total Risks Identified: {}\n\n", summary.risk_count));

        text.push_str("### Risk Distribution\n");
        text.push_str(&format!(
            "- High Risk: {} ({:.0}%)\n",
            summary.high_risks,
            summary.high_risks as f64 / summary.risk_count.max(1) as f64 * 100.0
        ));
        text.push_str(&format!(
            "- Medium Risk: {} ({:.0}%)\n",
            summary.medium_risks,
            summary.medium_risks as f64 / summary.risk_count.max(1) as f64 * 100.0
        ));
        text.push_str(&format!(
            "- Low Risk: {} ({:.0}%)\n\n",
            summary.low_risks,
            summary.low_risks as f64 / summary.risk_count.max(1) as f64 * 100.0
        ));

        if !summary.top_risks.is_empty() {
            text.push_str("### Top Risks\n");
            for risk in &summary.top_risks {
                text.push_str(&format!("- {}\n", risk));
            }
            text.push('\n');
        }

        if !summary.alerts.is_empty() {
            text.push_str("### Alerts\n");
            for alert in &summary.alerts {
                text.push_str(&format!(
                    "[{}] {}\n",
                    alert.severity.as_str().to_uppercase(),
                    alert.message
                ));
                if let Some(ref action) = alert.action_required {
                    text.push_str(&format!("  Action: {}\n", action));
                }
            }
        }

        text
    }
}

/// Risk summary aggregation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummary {
    pub overall_score: f64,
    pub risk_count: usize,
    pub high_risks: usize,
    pub medium_risks: usize,
    pub low_risks: usize,
    pub by_category: HashMap<RiskCategory, CategoryRisk>,
    pub top_risks: Vec<String>,
    pub alerts: Vec<RiskAlert>,
}

/// Risk score by category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryRisk {
    pub category: RiskCategory,
    pub score: f64,
    pub risk_count: usize,
    pub top_risk: Option<String>,
}

/// A risk alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAlert {
    pub severity: InsightSeverity,
    pub message: String,
    pub action_required: Option<String>,
    pub category: RiskCategory,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;

    #[test]
    fn risk_item_score_calculation() {
        let mut risk = RiskItem {
            category: RiskCategory::Compliance,
            description: "Test risk".to_string(),
            severity: InsightSeverity::High,
            likelihood: 0.8,
            impact: 0.7,
            score: 0.0,
            mitigations: vec![],
        };

        risk.calculate_score();
        assert!(risk.score > 0.0);
        assert!(risk.score <= 1.0);
    }

    #[test]
    fn risk_summarizer_basic() {
        let summarizer = RiskSummarizer::with_default_config();

        let insights = vec![
            Insight::new("Risk 1", "Description 1")
                .with_severity(InsightSeverity::High)
                .with_confidence(0.8),
            Insight::new("Risk 2", "Description 2")
                .with_severity(InsightSeverity::Medium)
                .with_confidence(0.6),
        ];

        let summary = summarizer.summarize(&insights);

        assert_eq!(summary.risk_count, 2);
        assert!(summary.overall_score > 0.0);
    }

    #[test]
    fn risk_categorization() {
        let summarizer = RiskSummarizer::with_default_config();

        let compliance_insight = Insight::new("Sanctions Risk", "Description")
            .with_tags(vec!["sanctions".to_string()]);

        let risk = summarizer.insight_to_risk(&compliance_insight);
        assert_eq!(risk.category, RiskCategory::Compliance);
    }

    #[test]
    fn alerts_generated_for_high_risks() {
        let summarizer = RiskSummarizer::with_default_config();

        let insights = vec![
            Insight::new("High Risk", "Description")
                .with_severity(InsightSeverity::Critical)
                .with_confidence(0.9),
        ];

        let summary = summarizer.summarize(&insights);

        assert!(!summary.alerts.is_empty());
        assert_eq!(summary.alerts[0].severity, InsightSeverity::Critical);
    }

    #[test]
    fn category_aggregation() {
        let summarizer = RiskSummarizer::with_default_config();

        let insights = vec![
            Insight::new("Risk 1", "Financial risk")
                .with_tags(vec!["financial".to_string()]),
            Insight::new("Risk 2", "Another financial risk")
                .with_tags(vec!["financial".to_string()]),
        ];

        let summary = summarizer.summarize(&insights);

        assert!(summary.by_category.contains_key(&RiskCategory::Financial));
        let fin_risk = summary.by_category.get(&RiskCategory::Financial).unwrap();
        assert_eq!(fin_risk.risk_count, 2);
    }

    #[test]
    fn summary_text_generation() {
        let summarizer = RiskSummarizer::with_default_config();

        let insights = vec![
            Insight::new("Test Risk", "Description")
                .with_severity(InsightSeverity::Medium)
                .with_confidence(0.5),
        ];

        let summary = summarizer.summarize(&insights);
        let text = summarizer.generate_summary_text(&summary);

        assert!(text.contains("Risk Summary"));
        assert!(text.contains("Overall Risk Score"));
    }

    #[test]
    fn empty_insights() {
        let summarizer = RiskSummarizer::with_default_config();
        let insights: Vec<Insight> = vec![];

        let summary = summarizer.summarize(&insights);

        assert_eq!(summary.risk_count, 0);
        assert_eq!(summary.overall_score, 0.0);
        assert!(summary.alerts.is_empty());
    }

    #[test]
    fn risk_category_string() {
        assert_eq!(RiskCategory::Compliance.as_str(), "compliance");
        assert_eq!(RiskCategory::Financial.as_str(), "financial");
        assert_eq!(RiskCategory::Geopolitical.as_str(), "geopolitical");
    }
}
