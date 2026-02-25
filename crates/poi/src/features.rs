//! POI feature computation from artifacts.

use crate::model::*;

const MAX_KEYWORD_HITS_PER_TERM: usize = 5;

/// Keyword categories used for priority vector computation.
const KEYWORD_CATEGORIES: &[(&str, &[&str])] = &[
    ("cost", &["cost", "price", "budget", "savings", "tco", "should-cost", "تكلفة", "coût", "prix"]),
    ("quality", &["quality", "ppm", "defect", "yield", "zero defects", "جودة", "qualité"]),
    ("speed", &["speed", "lead time", "fast", "agile", "npi", "time-to-market", "سرعة", "rapidité"]),
    ("resilience", &["resilience", "risk", "disruption", "continuity", "dual source", "مرونة", "résilience"]),
    ("compliance", &["compliance", "audit", "regulation", "standard", "certification", "امتثال", "conformité"]),
    ("security", &["security", "cyber", "dmarc", "breach", "zero trust", "أمن", "sécurité"]),
];

/// Locale-specific keyword extensions for priority vector (B130).
/// Callers may provide additional keyword sets keyed by (category, locale).
pub type LocaleKeywords = Vec<(&'static str, &'static str, &'static [&'static str])>;

/// Get default locale extensions for common markets.
pub fn default_locale_keywords() -> LocaleKeywords {
    vec![
        ("cost", "ko", &["비용", "가격", "예산"]),
        ("quality", "ko", &["품질", "결함", "수율"]),
        ("cost", "ja", &["コスト", "価格", "予算"]),
        ("quality", "ja", &["品質", "不良", "歩留まり"]),
        ("cost", "zh", &["成本", "价格", "预算"]),
        ("quality", "zh", &["质量", "缺陷", "良率"]),
        ("cost", "he", &["עלות", "מחיר", "תקציב"]),
        ("quality", "he", &["איכות", "פגם"]),
    ]
}

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
            scores[i] += bounded_keyword_hits(&total_text, kw) as f64;
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

/// Compute priority vector with optional locale-specific keyword extensions (B130).
pub fn compute_priority_vector_with_locale(
    artifacts: &[PoiArtifact],
    locale_keywords: &LocaleKeywords,
) -> PriorityVector {
    if artifacts.is_empty() {
        return PriorityVector::zero();
    }

    let total_text: String = artifacts
        .iter()
        .map(|a| format!("{} {}", a.title, a.content_summary))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    let category_names = ["cost", "quality", "speed", "resilience", "compliance", "security"];
    let mut scores = [0.0_f64; 6];
    let mut total = 0.0;

    // Base keywords
    for (i, (_, keywords)) in KEYWORD_CATEGORIES.iter().enumerate() {
        for kw in *keywords {
            scores[i] += bounded_keyword_hits(&total_text, kw) as f64;
        }
    }

    // Locale extensions
    for (cat, _locale, keywords) in locale_keywords {
        if let Some(idx) = category_names.iter().position(|c| c == cat) {
            for kw in *keywords {
                scores[idx] += bounded_keyword_hits(&total_text, kw) as f64;
            }
        }
    }

    for s in &scores {
        total += s;
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

fn bounded_keyword_hits(text: &str, keyword: &str) -> usize {
    text.matches(keyword).count().min(MAX_KEYWORD_HITS_PER_TERM)
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
        "security" => pv.security,
        _ => 0.0,
    };

    // If no dimension dominates strongly, they're balanced
    if !max_val.is_finite() || max_val < 0.25 {
        return DecisionStyle::BalancedAnalytical;
    }

    match dominant {
        "cost" => DecisionStyle::CostFirst,
        "quality" => DecisionStyle::QualityFirst,
        "speed" => DecisionStyle::SpeedFirst,
        "resilience" | "security" => DecisionStyle::RiskFirst,
        "compliance" => DecisionStyle::ComplianceFirst,
        _ => DecisionStyle::BalancedAnalytical,
    }
}

/// Compute influence score: 0.3*centrality + 0.4*seniority + 0.3*recurrence.
/// Clamps to [0, 100] and guards against negative inputs (B112).
pub fn compute_influence_score(
    graph_centrality: f64,
    role_seniority: f64,
    public_recurrence: f64,
) -> f64 {
    let gc = graph_centrality.max(0.0);
    let rs = role_seniority.max(0.0);
    let pr = public_recurrence.max(0.0);
    let raw = 0.3 * gc + 0.4 * rs + 0.3 * pr;
    raw.clamp(0.0, 100.0)
}

/// Map role title to seniority score (0-100).
pub fn role_seniority_score(title: &str) -> f64 {
    let lower = title.to_lowercase();

    // C-level outranks director. Use token-level matching for 3-letter acronyms
    // to avoid false positives: "director" contains "cto" as a substring.
    let tokens: std::collections::HashSet<&str> = lower.split_whitespace().collect();
    if tokens.contains("ceo") || tokens.contains("cto") || tokens.contains("cfo")
        || tokens.contains("coo") || tokens.contains("cpo") || lower.contains("chief")
    {
        return 95.0;
    }
    // VP outranks Director; check first so "VP & Director" gets 85, not 70.
    if tokens.contains("vp") || lower.contains("vice president") {
        return 85.0;
    }
    if lower.contains("director") {
        return 70.0;
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
        if artifact.ts_utc > now_utc {
            continue; // skip future-dated artifacts — they get no recency weight
        }
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
/// Accounts for overlapping roles — concurrent roles count as one move (B127).
pub fn infer_change_appetite(role_history: &[RoleHistoryEntry]) -> ChangeAppetite {
    // Count distinct non-overlapping role transitions
    let distinct_moves = if role_history.len() < 2 {
        role_history.len()
    } else {
        let mut moves = 1usize;
        for i in 1..role_history.len() {
            let prev = &role_history[i - 1];
            let curr = &role_history[i];
            // If current starts after prev ends, it's a new distinct move
            let prev_end = prev.end_ts.unwrap_or(i64::MAX);
            if curr.start_ts >= prev_end || curr.org != prev.org {
                moves += 1;
            }
        }
        moves
    };

    if distinct_moves >= 4 {
        ChangeAppetite::EarlyAdopter
    } else if distinct_moves >= 2 {
        ChangeAppetite::Pragmatist
    } else if distinct_moves == 1 {
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
    fn test_infer_decision_style_all_zero_values() {
        let pv = PriorityVector::zero();
        assert_eq!(infer_decision_style(&pv), DecisionStyle::BalancedAnalytical);
    }

    #[test]
    fn test_infer_decision_style_nan_values_defaults_balanced() {
        let pv = PriorityVector {
            cost: f64::NAN,
            quality: f64::NAN,
            speed: f64::NAN,
            resilience: f64::NAN,
            compliance: f64::NAN,
            security: f64::NAN,
            confidence: 0.0,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::BalancedAnalytical);
    }

    #[test]
    fn test_infer_decision_style_tie_prefers_stable_primary_dimension() {
        let pv = PriorityVector {
            cost: 0.40,
            quality: 0.40,
            speed: 0.05,
            resilience: 0.05,
            compliance: 0.05,
            security: 0.05,
            confidence: 0.9,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::CostFirst);
    }

    #[test]
    fn test_infer_decision_style_tie_risk_dimensions() {
        let pv = PriorityVector {
            cost: 0.05,
            quality: 0.05,
            speed: 0.05,
            resilience: 0.40,
            compliance: 0.05,
            security: 0.40,
            confidence: 0.9,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::RiskFirst);
    }

    #[test]
    fn test_compute_influence_score() {
        let score = compute_influence_score(80.0, 90.0, 70.0);
        // 0.3*80 + 0.4*90 + 0.3*70 = 24 + 36 + 21 = 81.0
        assert!((score - 81.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_influence_score_negative_inputs() {
        let score = compute_influence_score(-10.0, -20.0, -30.0);
        assert_eq!(score, 0.0);
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
    fn test_compute_pain_index_empty_is_zero() {
        let now = 1700100000_i64;
        let pain = compute_pain_index(&[], now);
        assert_eq!(pain, 0.0);
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

    // B112: Guard against negative influence_score
    #[test]
    fn test_influence_score_negative_inputs() {
        let score = compute_influence_score(-10.0, -5.0, -20.0);
        assert!(score >= 0.0, "Negative inputs should be clamped: got {}", score);
    }

    // B119: Future-dated artifacts beyond threshold
    #[test]
    fn test_pain_index_future_artifacts_ignored() {
        let now = 1700100000_i64;
        let artifacts = vec![
            make_artifact("Future crisis", "Major problem and failure", now + 86400 * 365),
        ];
        let pain = compute_pain_index(&artifacts, now);
        assert!(pain < 0.01, "Future artifacts should not contribute to pain, got {}", pain);
    }

    // B120: Empty role family / unknown values
    #[test]
    fn test_role_seniority_unknown_title() {
        let score = role_seniority_score("");
        assert!(score >= 0.0);
        assert!(score <= 100.0);
    }

    // B124: Pain index uses bounded recency weights
    #[test]
    fn test_pain_index_bounded() {
        let now = 1700100000_i64;
        // Many pain artifacts should still be bounded to [0,1]
        let artifacts: Vec<PoiArtifact> = (0..100)
            .map(|i| make_artifact("crisis failure problem", "shortage delay defect", now - i * 3600))
            .collect();
        let pain = compute_pain_index(&artifacts, now);
        assert!(pain <= 1.0, "Pain index should be bounded to 1.0, got {}", pain);
        assert!(pain >= 0.0);
    }

    // B128: Network size validation (handled in model, tested here for completeness)
    #[test]
    fn test_influence_score_clamps_to_100() {
        let score = compute_influence_score(200.0, 200.0, 200.0);
        assert!((score - 100.0).abs() < 0.01);
    }

    // B128: Network size upper bound clamp
    #[test]
    fn test_network_size_clamp() {
        let mut inf = InfluenceProfile {
            influence_score: 50.0,
            graph_centrality: 50.0,
            public_recurrence: 50.0,
            role_seniority_score: 50.0,
            network_size: 100_000,
        };
        inf.clamp_network_size();
        assert_eq!(inf.network_size, MAX_NETWORK_SIZE);
    }

    // B130: Locale-specific keyword sets for priority vector
    #[test]
    fn test_priority_vector_with_locale_keywords() {
        let artifacts = vec![
            make_artifact("비용 절감 보고서", "예산 초과 및 가격 인상", 1700000000),
        ];
        let locale_kws = default_locale_keywords();
        let pv = compute_priority_vector_with_locale(&artifacts, &locale_kws);
        // Korean cost keywords should boost cost dimension
        assert!(pv.cost > 0.0, "Korean cost keywords should register, got cost={}", pv.cost);
    }

    #[test]
    fn test_compute_priority_vector_repeated_keywords_bounded() {
        let repeated_cost = "cost ".repeat(200);
        let artifacts = vec![
            make_artifact("Cost storm", &format!("{} quality", repeated_cost), 1700000000),
        ];

        let pv = compute_priority_vector(&artifacts);
        assert!(pv.cost < 0.9, "repeated keyword should be bounded, got {}", pv.cost);
        assert!(pv.cost > pv.quality, "cost should still dominate but not saturate");
    }

    #[test]
    fn test_compute_priority_vector_repeated_keywords_cap_is_stable() {
        let artifacts_low = vec![
            make_artifact("A", &"cost ".repeat(MAX_KEYWORD_HITS_PER_TERM), 1700000000),
        ];
        let artifacts_high = vec![
            make_artifact("A", &"cost ".repeat(MAX_KEYWORD_HITS_PER_TERM * 20), 1700000000),
        ];

        let pv_low = compute_priority_vector(&artifacts_low);
        let pv_high = compute_priority_vector(&artifacts_high);

        assert!((pv_low.cost - pv_high.cost).abs() < 1e-9);
    }
}
