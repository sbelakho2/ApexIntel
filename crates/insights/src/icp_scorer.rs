//! Ideal Customer Profile (ICP) fit scoring.
//!
//! Scores target accounts against a configurable ICP definition across four
//! dimensions — firmographics, technographics, intent, and strategic relevance —
//! producing a normalized 0..1 fit score with a transparent per-dimension
//! breakdown. This is the account-prioritization engine that answers
//! "which companies to target in sales".
//!
//! The breakdown is persisted to `companies.icp_breakdown` so sales reps can see
//! *why* an account scored high/low, not just the number.
//!
//! # Design
//!
//! - **Pure function** over an [`IcpInput`] snapshot — no DB coupling here;
//!   the worker/handler loads the company row and calls [`IcpScorer::score`].
//! - **Configurable** ICP via [`IcpDefinition`]; defaults encode Starz's
//!   electronics-distribution vertical (MENA focus, OEM/EMS buyers).
//! - **Explainable**: every contribution is a named, weighted component.

use serde::{Deserialize, Serialize};

/// The account attributes needed to score ICP fit. Built from a `CompanyRow`
/// (+ optional enrichment) by the caller.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IcpInput {
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub industry_tags: Vec<String>,
    pub tech_stack: Vec<String>,
    pub region: Option<String>,
    pub country_code: Option<String>,
    /// 0..1 — hiring/expansion/news intent signal already computed elsewhere.
    pub intent_signal_score: f64,
    /// 0..1 — existing strategic_relevance column.
    pub strategic_relevance: f64,
    pub funding_stage: Option<String>,
    pub headcount_growth_pct: Option<f64>,
}

/// A dimension's contribution to the final ICP score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreComponent {
    pub dimension: String,
    pub raw: f64,
    pub weighted: f64,
    pub weight: f64,
    pub note: String,
}

/// The full ICP scoring result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpScore {
    pub icp_fit_score: f64,
    pub intent_signal_score: f64,
    pub components: Vec<ScoreComponent>,
}

/// Configurable ICP definition. Defaults target Starz's vertical.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcpDefinition {
    /// Industries that map to the ideal customer (e.g. OEM, EMS, automotive).
    pub target_industries: Vec<String>,
    /// Regions/countries in the served market.
    pub target_regions: Vec<String>,
    /// Target employee-count band [min, max].
    pub target_employee_range: (i32, i32),
    /// Target annual revenue band (USD) [min, max].
    pub target_revenue_range: (i64, i64),
    /// Technologies whose presence indicates a strong-fit buyer.
    pub target_technologies: Vec<String>,
    /// Dimension weights (sum normalized internally).
    pub weight_firmographics: f64,
    pub weight_technographics: f64,
    pub weight_intent: f64,
    pub weight_strategic: f64,
}

impl Default for IcpDefinition {
    fn default() -> Self {
        Self {
            // Starz Electronics vertical: B2B electronics / semiconductor / OEM / EMS.
            target_industries: vec![
                "electronics".into(),
                "semiconductor".into(),
                "oem".into(),
                "ems".into(),
                "automotive".into(),
                "aerospace".into(),
                "defense".into(),
                "telecommunications".into(),
                "industrial".into(),
                "medical_devices".into(),
                "iot".into(),
            ],
            // MENA served market + expansion regions.
            target_regions: vec![
                "TN".into(), "MA".into(), "EG".into(), "AE".into(), "SA".into(),
                "IL".into(), "EU".into(), "US".into(), "ME".into(),
                "Tunisia".into(), "Morocco".into(), "MENA".into(), "Europe".into(),
            ],
            target_employee_range: (50, 50_000),
            target_revenue_range: (5_000_000, 5_000_000_000),
            target_technologies: vec![
                "sap".into(), "oracle".into(), "salesforce".into(),
            ],
            weight_firmographics: 0.35,
            weight_technographics: 0.15,
            weight_intent: 0.30,
            weight_strategic: 0.20,
        }
    }
}

/// Pure ICP scorer.
pub struct IcpScorer;

impl IcpScorer {
    /// Score an account against the ICP definition. Returns a 0..1 fit score
    /// plus the per-dimension breakdown.
    pub fn score(input: &IcpInput, def: &IcpDefinition) -> IcpScore {
        let firm = Self::score_firmographics(input, def);
        let tech = Self::score_technographics(input, def);
        let intent = Self::score_intent(input);
        let strategic = Self::score_strategic(input);

        // Normalize weights so partial-data accounts still produce a fair score.
        let total_weight = def.weight_firmographics
            + def.weight_technographics
            + def.weight_intent
            + def.weight_strategic;
        let total_weight = if total_weight <= 0.0 { 1.0 } else { total_weight };

        let mut components = Vec::with_capacity(4);
        let mut acc = 0.0_f64;

        for (dim, raw, w, note) in [
            ("firmographics", firm, def.weight_firmographics, Self::firmographic_note(input, def)),
            ("technographics", tech, def.weight_technographics, Self::technographic_note(input)),
            ("intent", intent, def.weight_intent, Self::intent_note(input)),
            ("strategic", strategic, def.weight_strategic, Self::strategic_note(input)),
        ] {
            let weighted = raw * (w / total_weight);
            acc += weighted;
            components.push(ScoreComponent {
                dimension: dim.to_string(),
                raw,
                weighted,
                weight: w / total_weight,
                note,
            });
        }

        IcpScore {
            icp_fit_score: acc.clamp(0.0, 1.0),
            intent_signal_score: input.intent_signal_score.clamp(0.0, 1.0),
            components,
        }
    }

    fn score_firmographics(input: &IcpInput, def: &IcpDefinition) -> f64 {
        let mut score = 0.0_f64;
        let mut parts = 0.0_f64;

        // Industry match (strongest firmographic signal).
        if !input.industry_tags.is_empty() {
            parts += 1.0;
            let matches = input
                .industry_tags
                .iter()
                .filter(|tag| {
                    let t = tag.to_lowercase();
                    def.target_industries.iter().any(|i| t.contains(&i.to_lowercase()))
                })
                .count();
            score += (matches as f64 / input.industry_tags.len() as f64).min(1.0);
        }

        // Region match.
        let region_hits = [&input.region, &input.country_code]
            .iter()
            .filter_map(|r| r.as_ref().map(|s| s.to_string()))
            .any(|r| {
                let rl = r.to_lowercase();
                def.target_regions.iter().any(|t| rl.contains(&t.to_lowercase()))
            });
        parts += 1.0;
        score += if region_hits { 1.0 } else { 0.0 };

        // Employee band.
        if let Some(emp) = input.employee_estimate {
            parts += 1.0;
            score += band_score(emp as f64, def.target_employee_range.0 as f64, def.target_employee_range.1 as f64);
        }

        // Revenue band.
        if let Some(rev) = input.revenue_estimate_usd {
            parts += 1.0;
            score += band_score(rev as f64, def.target_revenue_range.0 as f64, def.target_revenue_range.1 as f64);
        }

        if parts > 0.0 {
            (score / parts).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn score_technographics(input: &IcpInput, def: &IcpDefinition) -> f64 {
        if def.target_technologies.is_empty() || input.tech_stack.is_empty() {
            return 0.0;
        }
        let hits = input
            .tech_stack
            .iter()
            .filter(|t| {
                let tl = t.to_lowercase();
                def.target_technologies.iter().any(|tt| tl.contains(&tt.to_lowercase()))
            })
            .count();
        (hits as f64 / def.target_technologies.len().min(1) as f64).clamp(0.0, 1.0)
    }

    fn score_intent(input: &IcpInput) -> f64 {
        // Blend the external intent signal with headcount growth (a real
        // expansion/budget proxy). Both are 0..1.
        let mut score = input.intent_signal_score.clamp(0.0, 1.0) * 0.7;
        if let Some(growth) = input.headcount_growth_pct {
            // >20% growth saturates the remaining 0.3.
            score += (growth / 20.0).clamp(0.0, 1.0) * 0.3;
        }
        score.clamp(0.0, 1.0)
    }

    fn score_strategic(input: &IcpInput) -> f64 {
        input.strategic_relevance.clamp(0.0, 1.0)
    }

    fn firmographic_note(input: &IcpInput, def: &IcpDefinition) -> String {
        let industry_match = input
            .industry_tags
            .iter()
            .filter(|tag| {
                let t = tag.to_lowercase();
                def.target_industries.iter().any(|i| t.contains(&i.to_lowercase()))
            })
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "industries=[{}] employees={:?} revenue_usd={:?}",
            if industry_match.is_empty() {
                input.industry_tags.join(", ")
            } else {
                industry_match
            },
            input.employee_estimate,
            input.revenue_estimate_usd
        )
    }

    fn technographic_note(input: &IcpInput) -> String {
        format!("tech_stack=[{}]", input.tech_stack.join(", "))
    }

    fn intent_note(input: &IcpInput) -> String {
        format!(
            "intent={:.2} headcount_growth={:?}",
            input.intent_signal_score, input.headcount_growth_pct
        )
    }

    fn strategic_note(input: &IcpInput) -> String {
        format!("strategic_relevance={:.2}", input.strategic_relevance)
    }
}

/// Score how well a value fits a [min, max] band: 1.0 inside, decaying outside.
fn band_score(value: f64, min: f64, max: f64) -> f64 {
    if value >= min && value <= max {
        return 1.0;
    }
    if value < min {
        // Below floor: linear decay, full credit at 50% of floor.
        let half = (min / 2.0).max(1.0);
        return ((value - half) / (min - half)).max(0.0).min(1.0);
    }
    // Above ceiling: slow decay (large accounts still somewhat relevant).
    let over = value - max;
    (1.0 - (over / (max.max(1.0))).min(1.0) * 0.3).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ideal_account() -> IcpInput {
        IcpInput {
            employee_estimate: Some(1500),
            revenue_estimate_usd: Some(200_000_000),
            industry_tags: vec!["Electronics".into(), "OEM".into()],
            tech_stack: vec!["SAP".into()],
            region: Some("TN".into()),
            country_code: Some("TN".into()),
            intent_signal_score: 0.8,
            strategic_relevance: 0.7,
            funding_stage: None,
            headcount_growth_pct: Some(15.0),
        }
    }

    #[test]
    fn ideal_account_scores_high() {
        let score = IcpScorer::score(&ideal_account(), &IcpDefinition::default());
        assert!(score.icp_fit_score > 0.8, "expected >0.8, got {}", score.icp_fit_score);
        assert_eq!(score.components.len(), 4);
    }

    #[test]
    fn non_target_industry_scores_lower() {
        let ideal = IcpScorer::score(&ideal_account(), &IcpDefinition::default());
        let mut acct = ideal_account();
        acct.industry_tags = vec!["fashion".into(), "retail".into()];
        let score = IcpScorer::score(&acct, &IcpDefinition::default());
        // A non-target industry must score strictly lower than the ideal account,
        // even when intent/strategic signals are strong.
        assert!(
            score.icp_fit_score < ideal.icp_fit_score,
            "expected non-target ({}) < ideal ({})",
            score.icp_fit_score,
            ideal.icp_fit_score
        );
        // And the firmographics component specifically should drop vs. ideal.
        let ideal_firm = ideal
            .components
            .iter()
            .find(|c| c.dimension == "firmographics")
            .unwrap()
            .raw;
        let firm = score
            .components
            .iter()
            .find(|c| c.dimension == "firmographics")
            .unwrap();
        assert!(
            firm.raw < ideal_firm,
            "firmographic raw ({}) should drop below ideal ({})",
            firm.raw,
            ideal_firm
        );
    }

    #[test]
    fn empty_account_scores_zero() {
        let score = IcpScorer::score(&IcpInput::default(), &IcpDefinition::default());
        assert_eq!(score.icp_fit_score, 0.0);
    }

    #[test]
    fn band_score_inside_is_one() {
        assert!((band_score(500.0, 100.0, 1000.0) - 1.0).abs() < 1e-9);
    }
}
