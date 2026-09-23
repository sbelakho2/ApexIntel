//! Component shortage correlation engine.
//!
//! Cross-correlates component shortage signals (from Nexar/Mouser/DigiKey
//! APIs) with affected BOM product families to auto-generate C004-type
//! supply chain warnings.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Component shortage signal ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortageSignal {
    /// Component part number
    pub part_number: String,
    /// Component family (e.g., "MLCC", "MCU", "MOSFET")
    pub component_family: String,
    /// Source of the shortage info
    pub source: ShortageSource,
    /// Severity: "critical", "high", "medium", "low"
    pub severity: String,
    /// Lead time increase in weeks vs. normal
    pub lead_time_increase_weeks: Option<i32>,
    /// Price increase percentage
    pub price_increase_pct: Option<f64>,
    /// Estimated end date for the shortage
    pub estimated_resolution: Option<DateTime<Utc>>,
    /// Affected manufacturers
    pub affected_manufacturers: Vec<String>,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShortageSource {
    Nexar,
    Mouser,
    DigiKey,
    OctopartApi,
    IndustryReport,
    Manual,
}

// ─── BOM mapping ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BomEntry {
    pub product_family: String,
    pub component_families: Vec<String>,
    pub critical_components: Vec<String>,
    pub alternative_sources: i32,
    /// Revenue impact if this product family is disrupted (estimated)
    pub revenue_exposure_pct: f64,
}

/// Standard EMS BOM-to-component mapping.
pub fn default_bom_mapping() -> Vec<BomEntry> {
    vec![
        BomEntry {
            product_family: "PCBA_automotive".into(),
            component_families: vec![
                "MLCC".into(),
                "MCU".into(),
                "MOSFET".into(),
                "resistor".into(),
                "inductor".into(),
                "automotive_IC".into(),
            ],
            critical_components: vec!["STM32".into(), "NXP_S32".into(), "TI_TMS570".into()],
            alternative_sources: 2,
            revenue_exposure_pct: 35.0,
        },
        BomEntry {
            product_family: "PCBA_industrial".into(),
            component_families: vec![
                "MLCC".into(),
                "MCU".into(),
                "power_IC".into(),
                "connector".into(),
                "relay".into(),
            ],
            critical_components: vec!["STM32".into(), "ESP32".into()],
            alternative_sources: 3,
            revenue_exposure_pct: 25.0,
        },
        BomEntry {
            product_family: "PCBA_telecom".into(),
            component_families: vec![
                "RF_IC".into(),
                "FPGA".into(),
                "high_speed_connector".into(),
                "oscillator".into(),
                "power_module".into(),
            ],
            critical_components: vec!["Xilinx_Zynq".into(), "Qualcomm_QCA".into()],
            alternative_sources: 1,
            revenue_exposure_pct: 20.0,
        },
        BomEntry {
            product_family: "PCBA_medical".into(),
            component_families: vec![
                "precision_ADC".into(),
                "medical_IC".into(),
                "MLCC".into(),
                "precision_resistor".into(),
            ],
            critical_components: vec!["TI_ADS1299".into(), "AD7768".into()],
            alternative_sources: 1,
            revenue_exposure_pct: 15.0,
        },
        BomEntry {
            product_family: "wire_harness".into(),
            component_families: vec![
                "connector".into(),
                "terminal".into(),
                "wire".into(),
                "heat_shrink".into(),
            ],
            critical_components: vec!["TE_connector".into(), "Molex_connector".into()],
            alternative_sources: 3,
            revenue_exposure_pct: 10.0,
        },
    ]
}

// ─── Correlation result ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ShortageCorrelation {
    pub part_number: String,
    pub component_family: String,
    pub affected_product_families: Vec<String>,
    pub total_revenue_exposure_pct: f64,
    pub severity: String,
    pub alternative_sources: i32,
    pub warning_title: String,
    pub warning_description: String,
    pub warning_type: String,
    pub recommended_actions: Vec<String>,
}

// ─── Correlation engine ─────────────────────────────────────────────────

pub struct ShortageCorrelator {
    bom_mapping: Vec<BomEntry>,
}

impl ShortageCorrelator {
    pub fn new(bom_mapping: Vec<BomEntry>) -> Self {
        Self { bom_mapping }
    }

    pub fn with_defaults() -> Self {
        Self::new(default_bom_mapping())
    }

    /// Correlate a shortage signal with BOM and produce warnings.
    pub fn correlate(&self, signal: &ShortageSignal) -> Option<ShortageCorrelation> {
        let affected: Vec<&BomEntry> = self
            .bom_mapping
            .iter()
            .filter(|bom| {
                bom.component_families.contains(&signal.component_family)
                    || bom
                        .critical_components
                        .iter()
                        .any(|c| signal.part_number.contains(c))
            })
            .collect();

        if affected.is_empty() {
            return None;
        }

        let product_families: Vec<String> =
            affected.iter().map(|b| b.product_family.clone()).collect();
        let total_exposure: f64 = affected.iter().map(|b| b.revenue_exposure_pct).sum();
        let min_alternatives = affected
            .iter()
            .map(|b| b.alternative_sources)
            .min()
            .unwrap_or(0);

        let severity = if total_exposure > 40.0 || min_alternatives == 0 {
            "critical"
        } else if total_exposure > 20.0 || min_alternatives <= 1 {
            "high"
        } else if total_exposure > 10.0 {
            "medium"
        } else {
            "low"
        };

        let title = format!(
            "Component shortage: {} affecting {} product families",
            signal.component_family,
            product_families.len()
        );

        let description = format!(
            "Shortage detected for {} (family: {}). Affected product families: {}. \
             Total revenue exposure: {:.0}%. Alternative sources: {}. {}{}",
            signal.part_number,
            signal.component_family,
            product_families.join(", "),
            total_exposure,
            min_alternatives,
            signal
                .lead_time_increase_weeks
                .map(|w| format!("Lead time increased by {} weeks. ", w))
                .unwrap_or_default(),
            signal
                .price_increase_pct
                .map(|p| format!("Price increased by {:.0}%.", p))
                .unwrap_or_default(),
        );

        let mut actions = vec!["Review BOM for affected product families".into()];
        if min_alternatives > 0 {
            actions.push("Qualify alternative component sources".into());
        }
        if total_exposure > 20.0 {
            actions.push("Notify affected customers of potential delays".into());
        }
        if signal.price_increase_pct.map(|p| p > 20.0).unwrap_or(false) {
            actions.push("Negotiate bulk pricing with current supplier".into());
        }
        actions.push("Monitor shortage resolution timeline".into());

        Some(ShortageCorrelation {
            part_number: signal.part_number.clone(),
            component_family: signal.component_family.clone(),
            affected_product_families: product_families,
            total_revenue_exposure_pct: total_exposure,
            severity: severity.into(),
            alternative_sources: min_alternatives,
            warning_title: title,
            warning_description: description,
            warning_type: "C004".into(),
            recommended_actions: actions,
        })
    }

    /// Batch-correlate multiple shortage signals.
    pub fn correlate_batch(&self, signals: &[ShortageSignal]) -> Vec<ShortageCorrelation> {
        signals.iter().filter_map(|s| self.correlate(s)).collect()
    }

    /// Aggregate exposure by product family across multiple shortages.
    pub fn aggregate_exposure(&self, correlations: &[ShortageCorrelation]) -> HashMap<String, f64> {
        let mut exposure: HashMap<String, f64> = HashMap::new();
        for corr in correlations {
            for pf in &corr.affected_product_families {
                *exposure.entry(pf.clone()).or_default() += corr.total_revenue_exposure_pct;
            }
        }
        exposure
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;

    fn make_signal(family: &str, part: &str, severity: &str) -> ShortageSignal {
        ShortageSignal {
            part_number: part.into(),
            component_family: family.into(),
            source: ShortageSource::Nexar,
            severity: severity.into(),
            lead_time_increase_weeks: Some(8),
            price_increase_pct: Some(25.0),
            estimated_resolution: None,
            affected_manufacturers: vec!["TDK".into(), "Murata".into()],
            detected_at: Utc::now(),
        }
    }

    #[test]
    fn test_mlcc_shortage_affects_multiple() {
        let correlator = ShortageCorrelator::with_defaults();
        let signal = make_signal("MLCC", "GRM155R71C104K", "high");
        let result = correlator.correlate(&signal);
        assert!(result.is_some());
        let corr = result.unwrap();
        assert!(corr.affected_product_families.len() >= 2);
        assert!(corr.total_revenue_exposure_pct > 0.0);
    }

    #[test]
    fn test_mcu_shortage_automotive() {
        let correlator = ShortageCorrelator::with_defaults();
        let signal = make_signal("MCU", "STM32F407", "critical");
        let result = correlator.correlate(&signal);
        assert!(result.is_some());
        let corr = result.unwrap();
        assert!(corr
            .affected_product_families
            .contains(&"PCBA_automotive".into()));
    }

    #[test]
    fn test_unrelated_component_no_match() {
        let correlator = ShortageCorrelator::with_defaults();
        let signal = make_signal("LED_display", "LCD-12345", "low");
        let result = correlator.correlate(&signal);
        assert!(result.is_none());
    }

    #[test]
    fn test_severity_assessment() {
        let correlator = ShortageCorrelator::with_defaults();
        // MLCC affects automotive (35%) + industrial (25%) + medical (15%) = 75%
        let signal = make_signal("MLCC", "generic_mlcc", "high");
        let result = correlator.correlate(&signal).unwrap();
        assert_eq!(result.severity, "critical"); // >40% exposure
    }

    #[test]
    fn test_recommended_actions() {
        let correlator = ShortageCorrelator::with_defaults();
        let signal = make_signal("MCU", "STM32", "high");
        let result = correlator.correlate(&signal).unwrap();
        assert!(!result.recommended_actions.is_empty());
        assert!(result.recommended_actions.iter().any(|a| a.contains("BOM")));
    }

    #[test]
    fn test_batch_correlate() {
        let correlator = ShortageCorrelator::with_defaults();
        let signals = vec![
            make_signal("MLCC", "cap-001", "high"),
            make_signal("connector", "TE-123", "medium"),
            make_signal("LED_custom", "custom-001", "low"),
        ];
        let results = correlator.correlate_batch(&signals);
        assert_eq!(results.len(), 2); // LED_custom has no match
    }

    #[test]
    fn test_aggregate_exposure() {
        let correlator = ShortageCorrelator::with_defaults();
        let signals = vec![
            make_signal("MLCC", "cap-001", "high"),
            make_signal("MCU", "STM32", "high"),
        ];
        let results = correlator.correlate_batch(&signals);
        let exposure = correlator.aggregate_exposure(&results);
        assert!(exposure.contains_key("PCBA_automotive"));
    }
}
