//! Weekly strategy memo generator.
//!
//! Produces structured weekly memos for GM/Board level, with:
//! - Executive summary
//! - Ranked insights by priority
//! - Regional breakdown (TN, MA, IL, CN, EA, EU, US)
//! - Security posture summary
//! - Recommended actions

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::renderer::{InsightCard, format_card_text, group_by_category, group_by_region};

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
}

/// Security posture summary section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySummary {
    pub total_security_insights: usize,
    pub critical_security: usize,
    pub top_threats: Vec<String>,
    pub posture_assessment: String,
}

// ────────────────────────────────────────────
// Region labels
// ────────────────────────────────────────────

/// Map region code to human-readable label.
pub fn region_label(code: &str) -> &str {
    match code {
        "TN" => "Tunisia",
        "MA" => "Morocco",
        "IL" => "Israel",
        "CN" => "China",
        "EA" => "East Asia",
        "EU" => "Europe",
        "US" => "United States",
        "global" => "Global / Multi-region",
        _ => code,
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
pub fn count_by_severity(cards: &[InsightCard]) -> (usize, usize, usize) {
    let mut critical = 0usize;
    let mut warning = 0usize;
    let mut info = 0usize;
    for card in cards {
        match card.severity.as_str() {
            "critical" => critical += 1,
            "warning" => warning += 1,
            _ => info += 1,
        }
    }
    (critical, warning, info)
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
            priority += 1;
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
pub fn build_regional_sections(cards: &[InsightCard], max_per_region: usize) -> Vec<RegionalSection> {
    let by_region = group_by_region(cards);
    let order = region_order();

    let mut sections = Vec::new();
    for region_code in &order {
        if let Some(region_cards) = by_region.get(*region_code) {
            let top: Vec<RegionalInsightSummary> = region_cards
                .iter()
                .take(max_per_region)
                .map(|c| RegionalInsightSummary {
                    recipe_code: c.recipe_code.clone(),
                    entity_name: c.entity_name.clone(),
                    title: c.title.clone(),
                    severity: c.severity.clone(),
                    impact_label: c.impact_label.clone(),
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
    extra.sort_by_key(|(k, _)| (*k).clone());

    for (region_code, region_cards) in extra {
        let top: Vec<RegionalInsightSummary> = region_cards
            .iter()
            .take(max_per_region)
            .map(|c| RegionalInsightSummary {
                recipe_code: c.recipe_code.clone(),
                entity_name: c.entity_name.clone(),
                title: c.title.clone(),
                severity: c.severity.clone(),
                impact_label: c.impact_label.clone(),
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

// ────────────────────────────────────────────
// Security summary
// ────────────────────────────────────────────

/// Build security posture summary from insight cards.
pub fn build_security_summary(cards: &[InsightCard]) -> SecuritySummary {
    let security_cards: Vec<&InsightCard> = cards
        .iter()
        .filter(|c| c.category == "security")
        .collect();

    let total = security_cards.len();
    let critical = security_cards
        .iter()
        .filter(|c| c.severity == "critical")
        .count();

    let top_threats: Vec<String> = security_cards
        .iter()
        .take(5)
        .map(|c| c.title.clone())
        .collect();

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
pub fn render_memo_text(
    week: u32,
    year: i32,
    exec_summary: &str,
    cards: &[InsightCard],
    actions: &[ActionItem],
    sections: &[RegionalSection],
    security: &SecuritySummary,
) -> String {
    let mut lines = Vec::new();

    lines.push(format!("# Weekly Strategy Memo — W{:02}/{}", week, year));
    lines.push(String::new());

    // Executive summary
    lines.push("## Executive Summary".to_string());
    lines.push(exec_summary.to_string());
    lines.push(String::new());

    // Top actions
    lines.push("## Priority Actions".to_string());
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

    // Regional breakdown
    lines.push("## Regional Breakdown".to_string());
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
        }
        lines.push(String::new());
    }

    // Security summary
    lines.push("## Security Posture".to_string());
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
    }
    lines.push(String::new());

    // Detailed insights
    if !cards.is_empty() {
        lines.push("## Detailed Insights".to_string());
        for card in cards.iter().take(20) {
            lines.push(format_card_text(card));
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
    let week = now.iso_week().week();
    let year = now.year();

    let (critical, warning, info) = count_by_severity(cards);
    let top_actions = extract_top_actions(cards, 10);
    let regional_sections = build_regional_sections(cards, 5);
    let security = build_security_summary(cards);
    let cat_breakdown = category_breakdown(cards);

    let exec_summary = generate_executive_summary(
        cards.len(),
        critical,
        warning,
        &top_actions,
        &security,
    );

    let full_text = render_memo_text(
        week,
        year,
        &exec_summary,
        cards,
        &top_actions,
        &regional_sections,
        &security,
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
        category_breakdown: cat_breakdown,
        full_text,
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::{InsightCandidate, EvidenceSlot, render_batch};

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
            make_candidate("A001", "Foxconn", "critical", "demand", Some("TN"), 0.9, 0.9),
            make_candidate("B001", "Jabil", "warning", "competitor", Some("MA"), 0.7, 0.8),
            make_candidate("C001", "Starz", "critical", "security", Some("TN"), 0.85, 0.95),
            make_candidate("D001", "Flex", "info", "supply_chain", Some("CN"), 0.4, 0.6),
            make_candidate("A010", "Celestica", "warning", "demand", Some("EU"), 0.6, 0.7),
        ];
        render_batch(&candidates)
    }

    #[test]
    fn test_region_label() {
        assert_eq!(region_label("TN"), "Tunisia");
        assert_eq!(region_label("MA"), "Morocco");
        assert_eq!(region_label("IL"), "Israel");
        assert_eq!(region_label("CN"), "China");
        assert_eq!(region_label("XX"), "XX");
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
        let candidates = vec![
            make_candidate("A001", "Foxconn", "warning", "demand", Some("TN"), 0.7, 0.8),
        ];
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
            posture_assessment: "ELEVATED — Critical security threats require immediate attention".to_string(),
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
        assert!(!memo.full_text.is_empty());

        // Full text should contain key sections
        assert!(memo.full_text.contains("# Weekly Strategy Memo"));
        assert!(memo.full_text.contains("## Executive Summary"));
        assert!(memo.full_text.contains("## Priority Actions"));
        assert!(memo.full_text.contains("## Regional Breakdown"));
        assert!(memo.full_text.contains("## Security Posture"));
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
}
