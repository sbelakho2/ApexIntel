//! POI feature computation from artifacts.

use crate::model::*;

/// Keyword categories used for priority vector computation.
const KEYWORD_CATEGORIES: &[(&str, &[&str])] = &[
    ("cost", &["cost", "price", "budget", "savings", "tco", "should-cost", "تكلفة", "coût", "prix"]),
    ("quality", &["quality", "ppm", "defect", "yield", "zero defects", "جودة", "qualité"]),
    ("speed", &["speed", "lead time", "fast", "agile", "npi", "time-to-market", "سرعة", "rapidité"]),
    ("resilience", &["resilience", "risk", "disruption", "continuity", "dual source", "مرونة", "résilience"]),
    ("compliance", &["compliance", "audit", "regulation", "standard", "certification", "امتثال", "conformité"]),
    ("security", &["security", "cyber", "dmarc", "breach", "zero trust", "أمن", "sécurité"]),
];

/// Compute priority vector from artifact text analysis.
pub fn compute_priority_vector(artifacts: &[PoiArtifact]) -> PriorityVector {
    if artifacts.is_empty() {
        return PriorityVector::zero();
    }

    let total_text: String = artifacts
        .iter()
        .map(|a| format!("{} {}", a.title, a.content_summary))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    let mut scores = [0.0_f64; 6];
    let mut total = 0.0;

    for (i, (_, keywords)) in KEYWORD_CATEGORIES.iter().enumerate() {
        for kw in *keywords {
            scores[i] += total_text.matches(kw).count() as f64;
        }
        total += scores[i];
    }

    if total > 0.0 {
        for s in &mut scores {
            *s /= total;
        }
    }

    PriorityVector {
        cost: scores[0],
        quality: scores[1],
        speed: scores[2],
        resilience: scores[3],
        compliance: scores[4],
        security: scores[5],
        confidence: (artifacts.len() as f64 / 20.0).min(1.0),
    }
}

/// Infer decision style from priority vector.
pub fn infer_decision_style(pv: &PriorityVector) -> DecisionStyle {
    let dominant = pv.dominant();
    let max_val = match dominant {
        "cost" => pv.cost,
        "quality" => pv.quality,
        "speed" => pv.speed,
        "resilience" => pv.resilience,
        "compliance" => pv.compliance,
        "security" => pv.compliance,
        _ => 0.0,
    };

    // If no dimension dominates strongly, they're balanced
    if max_val < 0.25 {
        return DecisionStyle::BalancedAnalytical;
    }

    match dominant {
        "cost" => DecisionStyle::CostFirst,
        "quality" => DecisionStyle::QualityFirst,
        "speed" => DecisionStyle::SpeedFirst,
        "resilience" => DecisionStyle::RiskFirst,
        "compliance" => DecisionStyle::ComplianceFirst,
        _ => DecisionStyle::BalancedAnalytical,
    }
}

/// Compute influence score: 0.3*centrality + 0.4*seniority + 0.3*recurrence
pub fn compute_influence_score(
    graph_centrality: f64,
    role_seniority: f64,
    public_recurrence: f64,
) -> f64 {
    let raw = 0.3 * graph_centrality + 0.4 * role_seniority + 0.3 * public_recurrence;
    raw.clamp(0.0, 100.0)
}

/// Map role title to seniority score (0-100).
pub fn role_seniority_score(title: &str) -> f64 {
    let lower = title.to_lowercase();

    // Check director before C-level to avoid substring match
    if lower.contains("director") {
        return 70.0;
    }
    if lower.contains("ceo") || lower.contains("cto") || lower.contains("cfo")
        || lower.contains("coo") || lower.contains("cpo") || lower.contains("chief")
    {
        return 95.0;
    }
    if lower.contains("vp") || lower.contains("vice president") {
        return 85.0;
    }
    if lower.contains("senior manager") {
        return 60.0;
    }
    if lower.contains("manager") {
        return 50.0;
    }
    if lower.contains("lead") {
        return 40.0;
    }
    if lower.contains("senior") {
        return 30.0;
    }
    20.0
}

/// Compute pain index from artifacts — higher if recent disruption/complaint mentions.
pub fn compute_pain_index(artifacts: &[PoiArtifact], now_utc: i64) -> f64 {
    let pain_keywords = [
        "problem", "issue", "delay", "shortage", "failure", "complaint",
        "disruption", "late", "defect", "recall", "crisis", "مشكلة", "problème",
    ];

    let mut pain_score = 0.0;
    for artifact in artifacts {
        let text = format!("{} {}", artifact.title, artifact.content_summary).to_lowercase();
        let age_days = ((now_utc - artifact.ts_utc) as f64 / 86400.0).max(1.0);
        let recency_weight = 1.0 / (1.0 + age_days / 90.0);

        for kw in &pain_keywords {
            if text.contains(kw) {
                pain_score += recency_weight;
            }
        }
    }

    // Normalize to 0-1 range
    (pain_score / 5.0).min(1.0)
}

/// Infer change appetite from role history.
pub fn infer_change_appetite(role_history: &[RoleHistoryEntry]) -> ChangeAppetite {
    if role_history.len() >= 4 {
        ChangeAppetite::EarlyAdopter // Frequent movers are open to new things
    } else if role_history.len() >= 2 {
        ChangeAppetite::Pragmatist
    } else if role_history.len() == 1 {
        ChangeAppetite::Conservative
    } else {
        ChangeAppetite::Laggard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_artifact(title: &str, summary: &str, ts: i64) -> PoiArtifact {
        PoiArtifact {
            artifact_type: "article".to_string(),
            title: title.to_string(),
            content_summary: summary.to_string(),
            source_url: None,
            ts_utc: ts,
        }
    }

    #[test]
    fn test_compute_priority_vector_cost_dominant() {
        let artifacts = vec![
            make_artifact("Cost reduction strategies", "Budget savings and TCO analysis", 1700000000),
            make_artifact("Price negotiation", "Cost optimization approach with price benchmarking", 1700000000),
        ];
        let pv = compute_priority_vector(&artifacts);
        assert_eq!(pv.dominant(), "cost");
        assert!(pv.cost > 0.3);
    }

    #[test]
    fn test_compute_priority_vector_quality_dominant() {
        let artifacts = vec![
            make_artifact("Quality management", "PPM defect analysis and yield improvement", 1700000000),
            make_artifact("Zero defects", "Quality control excellence", 1700000000),
        ];
        let pv = compute_priority_vector(&artifacts);
        assert_eq!(pv.dominant(), "quality");
    }

    #[test]
    fn test_compute_priority_vector_empty() {
        let pv = compute_priority_vector(&[]);
        assert!((pv.confidence - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_infer_decision_style() {
        let pv = PriorityVector {
            cost: 0.5,
            quality: 0.2,
            speed: 0.1,
            resilience: 0.1,
            compliance: 0.05,
            security: 0.05,
            confidence: 0.8,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::CostFirst);
    }

    #[test]
    fn test_infer_decision_style_balanced() {
        let pv = PriorityVector {
            cost: 0.18,
            quality: 0.17,
            speed: 0.16,
            resilience: 0.17,
            compliance: 0.16,
            security: 0.16,
            confidence: 0.5,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::BalancedAnalytical);
    }

    #[test]
    fn test_compute_influence_score() {
        let score = compute_influence_score(80.0, 90.0, 70.0);
        // 0.3*80 + 0.4*90 + 0.3*70 = 24 + 36 + 21 = 81.0
        assert!((score - 81.0).abs() < 0.01);
    }

    #[test]
    fn test_role_seniority_score() {
        assert!((role_seniority_score("CEO") - 95.0).abs() < 0.01);
        assert!((role_seniority_score("VP Procurement") - 85.0).abs() < 0.01);
        assert!((role_seniority_score("Director of Engineering") - 70.0).abs() < 0.01);
        assert!((role_seniority_score("Manager") - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_pain_index_high() {
        let now = 1700100000_i64;
        let artifacts = vec![
            make_artifact("Supply chain crisis", "Major shortage and delays causing failure", now - 86400),
            make_artifact("Quality problem report", "Defect recall and complaint escalation", now - 43200),
        ];
        let pain = compute_pain_index(&artifacts, now);
        assert!(pain > 0.5, "Recent pain artifacts should yield high pain index, got {}", pain);
    }

    #[test]
    fn test_compute_pain_index_low() {
        let now = 1700100000_i64;
        let artifacts = vec![
            make_artifact("Annual report", "Growth and expansion plans", now - 86400),
        ];
        let pain = compute_pain_index(&artifacts, now);
        assert!(pain < 0.2);
    }

    #[test]
    fn test_infer_change_appetite() {
        let history_4 = vec![
            RoleHistoryEntry { org: "A".to_string(), title: "Eng".to_string(), role_family: RoleFamily::Engineering, start_ts: 0, end_ts: Some(100) },
            RoleHistoryEntry { org: "B".to_string(), title: "Eng".to_string(), role_family: RoleFamily::Engineering, start_ts: 100, end_ts: Some(200) },
            RoleHistoryEntry { org: "C".to_string(), title: "Eng".to_string(), role_family: RoleFamily::Engineering, start_ts: 200, end_ts: Some(300) },
            RoleHistoryEntry { org: "D".to_string(), title: "Eng".to_string(), role_family: RoleFamily::Engineering, start_ts: 300, end_ts: None },
        ];
        assert_eq!(infer_change_appetite(&history_4), ChangeAppetite::EarlyAdopter);
        assert_eq!(infer_change_appetite(&history_4[..2]), ChangeAppetite::Pragmatist);
        assert_eq!(infer_change_appetite(&history_4[..1]), ChangeAppetite::Conservative);
        assert_eq!(infer_change_appetite(&[]), ChangeAppetite::Laggard);
    }
}
