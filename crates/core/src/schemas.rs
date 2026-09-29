use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::{ApexError, Result};

// ────────────────────────────────────────────
// Recipe — the core unit of insight generation
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecipeStatus {
    Seed,      // hand-written, unvalidated
    Candidate, // discovered by pattern miner
    Staged,    // passed statistical gates, awaiting human review
    Promoted,  // live in production
    Retired,   // deactivated
}

impl RecipeStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Seed => "seed",
            Self::Candidate => "candidate",
            Self::Staged => "staged",
            Self::Promoted => "promoted",
            Self::Retired => "retired",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalSpec {
    pub observation_type: String,
    pub field: String,
    pub operator: String, // "increase", "decrease", "above", "below", "equals", "contains"
    pub threshold: Option<f64>,
    pub window_days: Option<i32>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformSpec {
    pub transform_type: String, // "zscore", "pct_change", "rolling_mean", "count", "diff"
    pub field: String,
    pub window_days: i32,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticalTest {
    pub test_type: String, // "fisher_exact", "cross_correlation", "mutual_information", "hazard_uplift"
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Applicability {
    pub geos: Vec<String>,
    pub industries: Vec<String>,
    pub notes: String,
}

impl Applicability {
    /// True when the recipe carries no geographic or industry restriction.
    pub fn is_unrestricted(&self) -> bool {
        self.geos.is_empty() && self.industries.is_empty()
    }

    /// Whether this recipe applies to an entity with the given context.
    ///
    /// Conservative: a restricted recipe with absent context does **not**
    /// apply. Matching is case-insensitive on trimmed values; a `global` entry
    /// matches any non-empty value.
    pub fn allows(&self, context: &EntityContext) -> bool {
        let geo_ok = self.geos.is_empty()
            || context
                .region
                .as_deref()
                .is_some_and(|region| list_matches(&self.geos, region));
        let industry_ok = self.industries.is_empty()
            || context
                .industry
                .as_deref()
                .is_some_and(|industry| list_matches(&self.industries, industry));
        geo_ok && industry_ok
    }
}

fn list_matches(allowed: &[String], value: &str) -> bool {
    let value = value.trim();
    allowed.iter().any(|entry| {
        let entry = entry.trim();
        entry.eq_ignore_ascii_case(value)
            || (entry.eq_ignore_ascii_case("global") && !value.is_empty())
    })
}

/// Entity context for runtime applicability checks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityContext {
    pub region: Option<String>,
    pub country: Option<String>,
    pub industry: Option<String>,
    pub entity_type: Option<String>,
}

/// How many of a recipe's signals must be satisfied for evaluation.
///
/// The engine previously hard-coded a 50% partial match for every recipe,
/// which let a four-signal recipe fire on two generic observations. The policy
/// is now part of the recipe: security/compliance recipes default to
/// [`MatchPolicy::All`], everything else keeps the historical fraction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchPolicy {
    /// Every signal must be satisfied (exact match only).
    All,
    /// At least this many signals must be satisfied.
    AtLeast(u32),
    /// At least this fraction (0.0..=1.0) of signals must be satisfied.
    Fraction(f64),
}

impl Default for MatchPolicy {
    fn default() -> Self {
        Self::Fraction(0.5)
    }
}

impl MatchPolicy {
    /// Minimum number of matched signals required for `total` signals.
    pub fn required_matches(&self, total: usize) -> usize {
        match self {
            Self::All => total,
            Self::AtLeast(count) => (*count as usize).min(total),
            Self::Fraction(fraction) => {
                let required = (fraction.clamp(0.0, 1.0) * total as f64).ceil() as usize;
                required.clamp(1, total)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub id: Uuid,
    pub code: String, // e.g. "A001", "B023"
    pub name: String,
    pub description: String,
    pub status: RecipeStatus,

    // Signal combination
    pub signals: Vec<SignalSpec>,

    // Transforms to apply before testing
    pub transforms: Vec<TransformSpec>,

    // Statistical test to validate pattern
    pub statistical_test: Option<StatisticalTest>,

    // Thresholds
    pub min_uplift: f64,
    pub max_p_value: f64,
    pub min_time_slices: i32,
    pub min_entities: i32,

    /// How many signals must match for the recipe to evaluate.
    pub match_policy: MatchPolicy,

    /// Runtime firing gate: when set, a candidate's confidence must reach this
    /// value to fire. Populated from `recipes.activation_threshold`
    /// (migration 089) so database calibration actually changes execution.
    /// `None` = no gate beyond the impact floor.
    pub activation_threshold: Option<f64>,

    // Narrative template
    pub insight_template: String,
    pub action_template: String,

    // Applicability
    pub applicability: Applicability,

    // Severity
    pub severity: String, // "info", "warning", "critical"
    pub category: String, // "demand", "supply_chain", "competitor", "security", "poi"

    // Lifecycle
    pub created_at: DateTime<Utc>,
    pub promoted_at: Option<DateTime<Utc>>,
    pub retired_at: Option<DateTime<Utc>>,
    pub last_fired: Option<DateTime<Utc>>,
    pub fire_count: u64,
    pub false_positive_count: u64,
}

impl Recipe {
    pub fn new(code: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            code: code.into(),
            name: name.into(),
            description: String::new(),
            status: RecipeStatus::Seed,
            signals: Vec::new(),
            transforms: Vec::new(),
            statistical_test: None,
            min_uplift: 1.5,
            max_p_value: 0.01,
            min_time_slices: 3,
            min_entities: 5,
            match_policy: MatchPolicy::default(),
            activation_threshold: None,
            insight_template: String::new(),
            action_template: String::new(),
            applicability: Applicability {
                geos: Vec::new(),
                industries: Vec::new(),
                notes: String::new(),
            },
            severity: "info".to_string(),
            category: "demand".to_string(),
            created_at: Utc::now(),
            promoted_at: None,
            retired_at: None,
            last_fired: None,
            fire_count: 0,
            false_positive_count: 0,
        }
    }

    /// Validate recipe has required fields.
    pub fn validate(&self) -> Result<()> {
        if self.code.is_empty() {
            return Err(ApexError::validation("recipe code cannot be empty"));
        }
        if self.name.is_empty() {
            return Err(ApexError::validation("recipe name cannot be empty"));
        }
        if self.signals.is_empty() {
            return Err(ApexError::validation(
                "recipe must have at least one signal",
            ));
        }
        Ok(())
    }

    pub fn precision(&self) -> f64 {
        let total = self.fire_count;
        if total == 0 {
            return 1.0;
        }
        let true_positives = total.saturating_sub(self.false_positive_count);
        true_positives as f64 / total as f64
    }
}

// ────────────────────────────────────────────
// Pattern Candidate — discovered by the miner
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternCandidate {
    pub id: Uuid,
    pub recipe_id: Option<Uuid>,
    pub signal_combination: Vec<String>,
    pub uplift: f64,
    pub p_value: f64,
    pub q_value: f64, // FDR-corrected
    pub time_slices_passed: i32,
    pub entities_passed: i32,
    pub negative_control_passed: bool,
    pub counterfactual_passed: bool,
    pub false_alarm_budget_ok: bool,
    pub discovered_at: DateTime<Utc>,
    pub promoted: bool,
}

impl PatternCandidate {
    pub fn passes_all_gates(&self) -> bool {
        self.uplift >= 1.5
            && self.p_value < 0.01
            && self.q_value < 0.05
            && self.time_slices_passed >= 3
            && self.entities_passed >= 5
            && self.negative_control_passed
            && self.counterfactual_passed
            && self.false_alarm_budget_ok
    }
}

// ────────────────────────────────────────────
// Warning / Insight output
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    pub id: Uuid,
    pub recipe_id: Uuid,
    pub recipe_code: String,
    pub severity: String,
    pub category: String,
    pub title: String,
    pub narrative: String,
    pub actions: Vec<String>,
    pub evidence: Vec<serde_json::Value>,
    pub affected_entities: Vec<Uuid>,
    pub confidence: f64,
    pub ts_utc: DateTime<Utc>,
    pub acknowledged: bool,
    pub false_positive: bool,
}

impl Warning {
    pub fn new(recipe: &Recipe, narrative: String, actions: Vec<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            recipe_id: recipe.id,
            recipe_code: recipe.code.clone(),
            severity: recipe.severity.clone(),
            category: recipe.category.clone(),
            title: recipe.name.clone(),
            narrative,
            actions,
            evidence: Vec::new(),
            affected_entities: Vec::new(),
            confidence: 1.0,
            ts_utc: Utc::now(),
            acknowledged: false,
            false_positive: false,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn applicability_conservative_matching() {
        let restricted = Applicability {
            geos: vec!["Tunisia".to_string(), "Global".to_string()],
            industries: vec!["EMS".to_string()],
            notes: String::new(),
        };
        assert!(!restricted.is_unrestricted());

        // Absent context never satisfies a restriction.
        assert!(!restricted.allows(&EntityContext::default()));

        let matching = EntityContext {
            region: Some("tunisia".to_string()),
            industry: Some("ems".to_string()),
            ..Default::default()
        };
        assert!(restricted.allows(&matching));

        let wrong_industry = EntityContext {
            region: Some("Tunisia".to_string()),
            industry: Some("automotive".to_string()),
            ..Default::default()
        };
        assert!(!restricted.allows(&wrong_industry));

        let global_geo = Applicability {
            geos: vec!["Global".to_string()],
            industries: vec![],
            notes: String::new(),
        };
        assert!(global_geo.allows(&EntityContext {
            region: Some("anywhere".to_string()),
            ..Default::default()
        }));
        assert!(!global_geo.allows(&EntityContext::default()));
    }

    use super::*;

    #[test]
    fn test_recipe_new() {
        let r = Recipe::new("A001", "Sourcing Cycle Detection");
        assert_eq!(r.code, "A001");
        assert_eq!(r.status, RecipeStatus::Seed);
        assert_eq!(r.min_uplift, 1.5);
        assert_eq!(r.fire_count, 0);
    }

    #[test]
    fn test_recipe_validate_empty_code() {
        let r = Recipe::new("", "Test");
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_recipe_validate_empty_name() {
        let r = Recipe::new("A001", "");
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_recipe_validate_no_signals() {
        let r = Recipe::new("A001", "Test Recipe");
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_recipe_validate_ok() {
        let mut r = Recipe::new("A001", "Test Recipe");
        r.signals.push(SignalSpec {
            observation_type: "JobPost".into(),
            field: "role_family".into(),
            operator: "equals".into(),
            threshold: None,
            window_days: Some(30),
            value: Some("procurement".into()),
        });
        assert!(r.validate().is_ok());
    }

    #[test]
    fn test_recipe_precision() {
        let mut r = Recipe::new("A001", "Test");
        assert_eq!(r.precision(), 1.0); // no fires

        r.fire_count = 100;
        r.false_positive_count = 10;
        assert!((r.precision() - 0.9).abs() < 1e-10);

        r.false_positive_count = 100;
        assert_eq!(r.precision(), 0.0);
    }

    #[test]
    fn test_pattern_candidate_gates() {
        let good = PatternCandidate {
            id: Uuid::new_v4(),
            recipe_id: None,
            signal_combination: vec!["JobPost".into(), "WebChange".into()],
            uplift: 2.0,
            p_value: 0.005,
            q_value: 0.03,
            time_slices_passed: 4,
            entities_passed: 10,
            negative_control_passed: true,
            counterfactual_passed: true,
            false_alarm_budget_ok: true,
            discovered_at: Utc::now(),
            promoted: false,
        };
        assert!(good.passes_all_gates());

        // Fail: low uplift
        let mut bad = good.clone();
        bad.uplift = 1.0;
        assert!(!bad.passes_all_gates());

        // Fail: high p-value
        let mut bad2 = good.clone();
        bad2.p_value = 0.05;
        assert!(!bad2.passes_all_gates());

        // Fail: negative control failed
        let mut bad3 = good.clone();
        bad3.negative_control_passed = false;
        assert!(!bad3.passes_all_gates());

        // Fail: not enough entities
        let mut bad4 = good.clone();
        bad4.entities_passed = 3;
        assert!(!bad4.passes_all_gates());
    }

    #[test]
    fn test_warning_new() {
        let mut r = Recipe::new("B005", "Port Shock Detection");
        r.severity = "critical".to_string();
        r.category = "supply_chain".to_string();

        let w = Warning::new(
            &r,
            "Port congestion at Tanger Med".into(),
            vec!["Alert ops team".into()],
        );
        assert_eq!(w.recipe_code, "B005");
        assert_eq!(w.severity, "critical");
        assert!(!w.acknowledged);
        assert!(!w.false_positive);
    }

    #[test]
    fn test_recipe_serialize_roundtrip() {
        let mut r = Recipe::new("C010", "Cert Lapse Monitor");
        r.signals.push(SignalSpec {
            observation_type: "CertificationUpdate".into(),
            field: "status".into(),
            operator: "equals".into(),
            threshold: None,
            window_days: None,
            value: Some("expired".into()),
        });
        let json = serde_json::to_string(&r).unwrap();
        let r2: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(r.id, r2.id);
        assert_eq!(r.code, r2.code);
        assert_eq!(r.signals.len(), r2.signals.len());
    }

    #[test]
    fn test_recipe_status_values() {
        assert_eq!(RecipeStatus::Seed.as_str(), "seed");
        assert_eq!(RecipeStatus::Candidate.as_str(), "candidate");
        assert_eq!(RecipeStatus::Staged.as_str(), "staged");
        assert_eq!(RecipeStatus::Promoted.as_str(), "promoted");
        assert_eq!(RecipeStatus::Retired.as_str(), "retired");
    }

    #[test]
    fn test_applicability() {
        let app = Applicability {
            geos: vec!["TN".into(), "MA".into(), "IL".into(), "CN".into()],
            industries: vec!["automotive".into(), "aerospace".into()],
            notes: "Defense sector included for IL".into(),
        };
        assert_eq!(app.geos.len(), 4);
        assert!(app.notes.contains("Defense"));
    }
}
