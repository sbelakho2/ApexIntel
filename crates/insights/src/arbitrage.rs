//! Cross-region arbitrage detector.
//!
//! When FX rates + tariff changes + logistics costs create a temporary
//! cost advantage for one manufacturing region over another, generate
//! a time-boxed strategic insight.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Rate data ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionCostProfile {
    pub region: String,
    pub currency: String,
    /// FX rate to USD
    pub fx_rate_usd: f64,
    /// Previous FX rate (for delta calculation)
    pub prev_fx_rate_usd: f64,
    /// Labor cost per hour (USD)
    pub labor_cost_usd: f64,
    /// Average tariff rate for electronics exports to EU (%)
    pub tariff_rate_eu_pct: f64,
    /// Average tariff rate for electronics exports to US (%)
    pub tariff_rate_us_pct: f64,
    /// Logistics cost per TEU to EU (USD)
    pub logistics_cost_eu_usd: f64,
    /// Logistics cost per TEU to US (USD)
    pub logistics_cost_us_usd: f64,
    /// Energy cost per kWh (USD)
    pub energy_cost_kwh_usd: f64,
    /// Timestamp of rate data
    pub as_of: DateTime<Utc>,
}

/// Default regional cost profiles for EMS manufacturing regions.
pub fn default_profiles() -> Vec<RegionCostProfile> {
    let now = Utc::now();
    vec![
        RegionCostProfile {
            region: "TN".into(),
            currency: "TND".into(),
            fx_rate_usd: 3.10,
            prev_fx_rate_usd: 3.05,
            labor_cost_usd: 3.50,
            tariff_rate_eu_pct: 0.0, // EU FTA
            tariff_rate_us_pct: 5.0,
            logistics_cost_eu_usd: 1200.0,
            logistics_cost_us_usd: 3500.0,
            energy_cost_kwh_usd: 0.08,
            as_of: now,
        },
        RegionCostProfile {
            region: "MA".into(),
            currency: "MAD".into(),
            fx_rate_usd: 9.90,
            prev_fx_rate_usd: 9.85,
            labor_cost_usd: 4.00,
            tariff_rate_eu_pct: 0.0, // EU FTA
            tariff_rate_us_pct: 5.0,
            logistics_cost_eu_usd: 1000.0, // Tanger Med proximity
            logistics_cost_us_usd: 3200.0,
            energy_cost_kwh_usd: 0.10,
            as_of: now,
        },
        RegionCostProfile {
            region: "IL".into(),
            currency: "ILS".into(),
            fx_rate_usd: 3.60,
            prev_fx_rate_usd: 3.55,
            labor_cost_usd: 15.00,
            tariff_rate_eu_pct: 0.0, // EU FTA
            tariff_rate_us_pct: 0.0, // US FTA
            logistics_cost_eu_usd: 1800.0,
            logistics_cost_us_usd: 2800.0,
            energy_cost_kwh_usd: 0.14,
            as_of: now,
        },
        RegionCostProfile {
            region: "CN".into(),
            currency: "CNY".into(),
            fx_rate_usd: 7.25,
            prev_fx_rate_usd: 7.20,
            labor_cost_usd: 6.00,
            tariff_rate_eu_pct: 3.7,
            tariff_rate_us_pct: 25.0, // Trade war tariffs
            logistics_cost_eu_usd: 2500.0,
            logistics_cost_us_usd: 2000.0,
            energy_cost_kwh_usd: 0.08,
            as_of: now,
        },
        RegionCostProfile {
            region: "EU".into(),
            currency: "EUR".into(),
            fx_rate_usd: 0.92,
            prev_fx_rate_usd: 0.93,
            labor_cost_usd: 25.00,
            tariff_rate_eu_pct: 0.0,
            tariff_rate_us_pct: 3.0,
            logistics_cost_eu_usd: 500.0, // intra-EU
            logistics_cost_us_usd: 2200.0,
            energy_cost_kwh_usd: 0.25,
            as_of: now,
        },
    ]
}

// ─── Arbitrage detection ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ArbitrageOpportunity {
    pub advantaged_region: String,
    pub disadvantaged_region: String,
    pub target_market: String,
    pub cost_advantage_pct: f64,
    pub fx_contribution_pct: f64,
    pub tariff_contribution_pct: f64,
    pub logistics_contribution_pct: f64,
    pub labor_contribution_pct: f64,
    pub severity: String,
    pub title: String,
    pub description: String,
    /// Estimated duration of the advantage (based on volatility)
    pub estimated_window_days: i64,
    pub detected_at: DateTime<Utc>,
}

pub struct ArbitrageDetector {
    /// Minimum cost advantage % to flag as opportunity
    min_advantage_pct: f64,
}

impl ArbitrageDetector {
    pub fn new(min_advantage_pct: f64) -> Self {
        Self { min_advantage_pct }
    }

    pub fn with_defaults() -> Self {
        Self::new(5.0) // 5% minimum advantage
    }

    /// Compare two regions for a target market and detect arbitrage.
    pub fn compare(
        &self,
        region_a: &RegionCostProfile,
        region_b: &RegionCostProfile,
        target_market: &str,
    ) -> Option<ArbitrageOpportunity> {
        let cost_a = self.total_cost(region_a, target_market);
        let cost_b = self.total_cost(region_b, target_market);

        if cost_a == 0.0 || cost_b == 0.0 {
            return None;
        }

        let advantage_pct = ((cost_b - cost_a) / cost_b) * 100.0;

        if advantage_pct.abs() < self.min_advantage_pct {
            return None;
        }

        let (advantaged, disadvantaged) = if advantage_pct > 0.0 {
            (region_a, region_b)
        } else {
            (region_b, region_a)
        };

        let abs_advantage = advantage_pct.abs();

        // Decompose advantage into contributing factors
        let labor_diff = (disadvantaged.labor_cost_usd - advantaged.labor_cost_usd)
            / cost_b
            * 100.0;
        let tariff_a = self.tariff_for(region_a, target_market);
        let tariff_b = self.tariff_for(region_b, target_market);
        let tariff_diff = (tariff_b - tariff_a) / cost_b * 100.0;
        let logistics_a = self.logistics_for(region_a, target_market);
        let logistics_b = self.logistics_for(region_b, target_market);
        let logistics_diff = (logistics_b - logistics_a) / cost_b * 100.0;
        let fx_change_a =
            (region_a.fx_rate_usd - region_a.prev_fx_rate_usd) / region_a.prev_fx_rate_usd * 100.0;
        let fx_change_b =
            (region_b.fx_rate_usd - region_b.prev_fx_rate_usd) / region_b.prev_fx_rate_usd * 100.0;
        let fx_diff = fx_change_b - fx_change_a;

        let severity = if abs_advantage > 20.0 {
            "critical"
        } else if abs_advantage > 10.0 {
            "high"
        } else {
            "medium"
        };

        // Estimate window based on FX volatility
        let estimated_window = if fx_diff.abs() > 2.0 {
            30 // FX-driven, may be temporary
        } else if tariff_diff.abs() > 5.0 {
            180 // tariff-driven, more stable
        } else {
            90 // mixed factors
        };

        let title = format!(
            "{} has {:.1}% cost advantage over {} for {} market",
            advantaged.region, abs_advantage, disadvantaged.region, target_market
        );

        let description = format!(
            "Manufacturing in {} currently offers a {:.1}% total cost advantage \
             over {} for electronics exports to {}. Contributing factors: \
             labor ({:+.1}%), tariff ({:+.1}%), logistics ({:+.1}%), FX ({:+.1}%). \
             Estimated advantage window: {} days.",
            advantaged.region,
            abs_advantage,
            disadvantaged.region,
            target_market,
            labor_diff,
            tariff_diff,
            logistics_diff,
            fx_diff,
            estimated_window
        );

        Some(ArbitrageOpportunity {
            advantaged_region: advantaged.region.clone(),
            disadvantaged_region: disadvantaged.region.clone(),
            target_market: target_market.into(),
            cost_advantage_pct: abs_advantage,
            fx_contribution_pct: fx_diff,
            tariff_contribution_pct: tariff_diff,
            logistics_contribution_pct: logistics_diff,
            labor_contribution_pct: labor_diff,
            severity: severity.into(),
            title,
            description,
            estimated_window_days: estimated_window,
            detected_at: Utc::now(),
        })
    }

    /// Scan all region pairs for all target markets.
    pub fn scan_all(
        &self,
        profiles: &[RegionCostProfile],
    ) -> Vec<ArbitrageOpportunity> {
        let markets = vec!["EU", "US"];
        let mut opportunities = Vec::new();

        for i in 0..profiles.len() {
            for j in (i + 1)..profiles.len() {
                for market in &markets {
                    if let Some(opp) = self.compare(&profiles[i], &profiles[j], market) {
                        opportunities.push(opp);
                    }
                }
            }
        }

        opportunities.sort_by(|a, b| {
            b.cost_advantage_pct
                .partial_cmp(&a.cost_advantage_pct)
                .unwrap()
        });
        opportunities
    }

    fn total_cost(&self, profile: &RegionCostProfile, target_market: &str) -> f64 {
        let tariff = self.tariff_for(profile, target_market);
        let logistics = self.logistics_for(profile, target_market);
        // Simplified: labor + energy + tariff (as added cost) + logistics
        profile.labor_cost_usd * 160.0 // 160 hours/month
            + profile.energy_cost_kwh_usd * 5000.0 // 5000 kWh/month
            + tariff
            + logistics
    }

    fn tariff_for(&self, profile: &RegionCostProfile, target_market: &str) -> f64 {
        // tariff_rate_*_pct is already a percentage (e.g. 25.0 for 25%).
        // Apply it to a reference product value ($1000) to get a USD cost.
        let reference_value = 1000.0;
        match target_market {
            "EU" => profile.tariff_rate_eu_pct / 100.0 * reference_value,
            "US" => profile.tariff_rate_us_pct / 100.0 * reference_value,
            _ => 0.0,
        }
    }

    fn logistics_for(&self, profile: &RegionCostProfile, target_market: &str) -> f64 {
        match target_market {
            "EU" => profile.logistics_cost_eu_usd,
            "US" => profile.logistics_cost_us_usd,
            _ => 0.0,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tn_vs_cn_for_eu() {
        let detector = ArbitrageDetector::with_defaults();
        let profiles = default_profiles();
        let tn = profiles.iter().find(|p| p.region == "TN").unwrap();
        let cn = profiles.iter().find(|p| p.region == "CN").unwrap();
        let result = detector.compare(tn, cn, "EU");
        // TN should have advantage: 0% tariff vs 3.7%, lower logistics
        assert!(result.is_some());
        let opp = result.unwrap();
        assert_eq!(opp.advantaged_region, "TN");
    }

    #[test]
    fn test_scan_all_finds_opportunities() {
        let detector = ArbitrageDetector::with_defaults();
        let profiles = default_profiles();
        let opportunities = detector.scan_all(&profiles);
        assert!(!opportunities.is_empty());
    }

    #[test]
    fn test_similar_regions_no_arbitrage() {
        let detector = ArbitrageDetector::new(50.0); // very high threshold
        let profiles = default_profiles();
        let tn = profiles.iter().find(|p| p.region == "TN").unwrap();
        let ma = profiles.iter().find(|p| p.region == "MA").unwrap();
        // TN and MA are very similar — may not trigger at 50% threshold
        let result = detector.compare(tn, ma, "EU");
        // Should be None or very small advantage
        if let Some(opp) = result {
            assert!(opp.cost_advantage_pct < 50.0);
        }
    }

    #[test]
    fn test_default_profiles_complete() {
        let profiles = default_profiles();
        assert_eq!(profiles.len(), 5);
        assert!(profiles.iter().any(|p| p.region == "TN"));
        assert!(profiles.iter().any(|p| p.region == "CN"));
        assert!(profiles.iter().any(|p| p.region == "IL"));
    }

    #[test]
    fn test_severity_classification() {
        let detector = ArbitrageDetector::with_defaults();
        let profiles = default_profiles();
        let tn = profiles.iter().find(|p| p.region == "TN").unwrap();
        let eu = profiles.iter().find(|p| p.region == "EU").unwrap();
        // TN vs EU should be a significant advantage (labor cost diff)
        if let Some(opp) = detector.compare(tn, eu, "EU") {
            assert!(opp.severity == "high" || opp.severity == "critical");
        }
    }
}
