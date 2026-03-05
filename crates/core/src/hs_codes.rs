//! HS (Harmonized System) code mapping engine.
//!
//! Maps product families to HS code families and provides tariff rate
//! lookup per trade corridor (EU→TN, EU→MA, US→IL, CN→EU, etc.).
//!
//! Coverage: Electronics chapters 84–85 (8534 PCB, 8542 IC, 8536 connectors,
//! 8544 cables, 8541 semiconductors, 8504 transformers, etc.).

use serde::Serialize;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct HsMapping {
    pub product_family: String,
    pub hs_codes: Vec<HsCodeEntry>,
    pub primary_hs4: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HsCodeEntry {
    pub code: String,
    pub description: String,
    pub chapter: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct TariffRate {
    pub corridor: String,
    pub hs4: String,
    pub mfn_rate_pct: f64,
    pub preferential_rate_pct: Option<f64>,
    pub agreement: Option<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TariffLookupResult {
    pub product_family: String,
    pub hs4: String,
    pub corridors: Vec<TariffRate>,
}

// ─────────────────────────────────────────────────────────────────────────────
// HS code database (electronics)
// ─────────────────────────────────────────────────────────────────────────────

/// Map a product family name to its HS code family.
pub fn map_product_to_hs(product_family: &str) -> HsMapping {
    let pf = product_family.to_lowercase();
    let (codes, primary) = if pf.contains("pcb") || pf.contains("printed circuit") {
        (
            vec![
                hs("8534.00", "Printed circuits", 85),
                hs("8534.00.10", "Single-sided PCB", 85),
                hs("8534.00.20", "Double-sided PCB", 85),
                hs("8534.00.30", "Multilayer PCB", 85),
            ],
            "8534",
        )
    } else if pf.contains("semiconductor") || pf.contains("ic ") || pf.contains("integrated circuit") {
        (
            vec![
                hs("8542.31", "Processors and controllers", 85),
                hs("8542.32", "Memories", 85),
                hs("8542.33", "Amplifiers", 85),
                hs("8542.39", "Other ICs", 85),
            ],
            "8542",
        )
    } else if pf.contains("connector") || pf.contains("switch") || pf.contains("relay") {
        (
            vec![
                hs("8536.10", "Fuses", 85),
                hs("8536.20", "Automatic circuit breakers", 85),
                hs("8536.50", "Switches ≤1kV", 85),
                hs("8536.90", "Other connectors ≤1kV", 85),
            ],
            "8536",
        )
    } else if pf.contains("cable") || pf.contains("wire") || pf.contains("harness") {
        (
            vec![
                hs("8544.11", "Copper winding wire", 85),
                hs("8544.20", "Coaxial cable", 85),
                hs("8544.42", "Connectors ≤1kV", 85),
                hs("8544.49", "Other electric conductors", 85),
            ],
            "8544",
        )
    } else if pf.contains("resistor") || pf.contains("capacitor") || pf.contains("passive") {
        (
            vec![
                hs("8532.10", "Fixed capacitors ≤50V", 85),
                hs("8532.21", "Tantalum capacitors", 85),
                hs("8533.10", "Fixed carbon resistors", 85),
                hs("8533.21", "Fixed resistors ≤20W", 85),
            ],
            "8532",
        )
    } else if pf.contains("transformer") || pf.contains("power supply") || pf.contains("inverter") {
        (
            vec![
                hs("8504.10", "Ballasts for discharge lamps", 85),
                hs("8504.31", "Transformers ≤1kVA", 85),
                hs("8504.40", "Static converters", 85),
                hs("8504.50", "Inductors", 85),
            ],
            "8504",
        )
    } else if pf.contains("led") || pf.contains("diode") || pf.contains("transistor") {
        (
            vec![
                hs("8541.10", "Diodes", 85),
                hs("8541.21", "Transistors ≤1W", 85),
                hs("8541.40", "Photosensitive devices", 85),
                hs("8541.41", "LEDs", 85),
            ],
            "8541",
        )
    } else if pf.contains("sensor") || pf.contains("transducer") {
        (
            vec![
                hs("9025.80", "Temperature sensors", 90),
                hs("9026.80", "Flow/level sensors", 90),
                hs("9027.80", "Chemical sensors", 90),
                hs("9031.80", "Other measuring instruments", 90),
            ],
            "9025",
        )
    } else {
        // Generic electronics
        (
            vec![
                hs("8543.70", "Other electrical machines", 85),
                hs("8543.90", "Parts of electrical apparatus", 85),
            ],
            "8543",
        )
    };

    HsMapping {
        product_family: product_family.into(),
        hs_codes: codes,
        primary_hs4: primary.into(),
    }
}

fn hs(code: &str, desc: &str, chapter: u8) -> HsCodeEntry {
    HsCodeEntry {
        code: code.into(),
        description: desc.into(),
        chapter,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tariff rate lookup
// ─────────────────────────────────────────────────────────────────────────────

/// Look up tariff rates for a given HS4 code across major trade corridors.
pub fn lookup_tariff(hs4: &str) -> Vec<TariffRate> {
    // Pre-built tariff database for EMS-relevant corridors
    let mut rates = Vec::new();

    // EU → Tunisia (AA agreement)
    rates.push(TariffRate {
        corridor: "EU→TN".into(),
        hs4: hs4.into(),
        mfn_rate_pct: tariff_mfn_eu(hs4),
        preferential_rate_pct: Some(0.0),
        agreement: Some("EU-Tunisia Association Agreement".into()),
        notes: "Industrial products duty-free under AA since 2008".into(),
    });

    // EU → Morocco (AA agreement)
    rates.push(TariffRate {
        corridor: "EU→MA".into(),
        hs4: hs4.into(),
        mfn_rate_pct: tariff_mfn_eu(hs4),
        preferential_rate_pct: Some(0.0),
        agreement: Some("EU-Morocco Association Agreement".into()),
        notes: "Industrial products duty-free under AA".into(),
    });

    // US → Israel (FTA)
    rates.push(TariffRate {
        corridor: "US→IL".into(),
        hs4: hs4.into(),
        mfn_rate_pct: tariff_mfn_us(hs4),
        preferential_rate_pct: Some(0.0),
        agreement: Some("US-Israel FTA".into()),
        notes: "All industrial products duty-free".into(),
    });

    // CN → EU
    rates.push(TariffRate {
        corridor: "CN→EU".into(),
        hs4: hs4.into(),
        mfn_rate_pct: tariff_mfn_eu(hs4),
        preferential_rate_pct: None,
        agreement: None,
        notes: "No preferential agreement — MFN rates apply".into(),
    });

    // CN → US (Section 301 tariffs)
    rates.push(TariffRate {
        corridor: "CN→US".into(),
        hs4: hs4.into(),
        mfn_rate_pct: tariff_mfn_us(hs4),
        preferential_rate_pct: None,
        agreement: None,
        notes: "Section 301 tariffs may apply (25% on List 1-3)".into(),
    });

    // TN → EU (reverse)
    rates.push(TariffRate {
        corridor: "TN→EU".into(),
        hs4: hs4.into(),
        mfn_rate_pct: tariff_mfn_eu(hs4),
        preferential_rate_pct: Some(0.0),
        agreement: Some("EU-Tunisia AA (EUR.1 certificate)".into()),
        notes: "Duty-free with proof of origin".into(),
    });

    rates
}

fn tariff_mfn_eu(hs4: &str) -> f64 {
    match hs4 {
        "8534" => 3.7,  // Printed circuits
        "8542" => 0.0,  // ICs — ITA duty-free
        "8536" => 2.7,  // Connectors
        "8544" => 3.3,  // Cables
        "8532" => 0.0,  // Capacitors — ITA
        "8533" => 0.0,  // Resistors — ITA
        "8504" => 2.5,  // Transformers
        "8541" => 0.0,  // Semiconductors — ITA
        "8543" => 2.2,  // Other electrical
        _ => 3.0,       // Default EU electronics rate
    }
}

fn tariff_mfn_us(hs4: &str) -> f64 {
    match hs4 {
        "8534" => 0.0,  // PCB — ITA
        "8542" => 0.0,  // ICs — ITA
        "8536" => 2.7,  // Connectors
        "8544" => 3.5,  // Cables
        "8532" => 0.0,  // Capacitors — ITA
        "8533" => 0.0,  // Resistors — ITA
        "8504" => 2.4,  // Transformers
        "8541" => 0.0,  // Semiconductors — ITA
        "8543" => 1.5,  // Other
        _ => 2.5,
    }
}

/// Full tariff lookup for a product family.
pub fn lookup_product_tariff(product_family: &str) -> TariffLookupResult {
    let mapping = map_product_to_hs(product_family);
    let corridors = lookup_tariff(&mapping.primary_hs4);
    TariffLookupResult {
        product_family: product_family.into(),
        hs4: mapping.primary_hs4,
        corridors,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_pcb() {
        let m = map_product_to_hs("Multilayer PCB");
        assert_eq!(m.primary_hs4, "8534");
        assert!(!m.hs_codes.is_empty());
    }

    #[test]
    fn map_semiconductor() {
        let m = map_product_to_hs("IC / Integrated Circuits");
        assert_eq!(m.primary_hs4, "8542");
    }

    #[test]
    fn map_connector() {
        let m = map_product_to_hs("Automotive Connectors");
        assert_eq!(m.primary_hs4, "8536");
    }

    #[test]
    fn tariff_eu_tunisia_duty_free() {
        let rates = lookup_tariff("8534");
        let eu_tn = rates.iter().find(|r| r.corridor == "EU→TN").unwrap();
        assert_eq!(eu_tn.preferential_rate_pct, Some(0.0));
    }

    #[test]
    fn tariff_cn_us_no_preference() {
        let rates = lookup_tariff("8542");
        let cn_us = rates.iter().find(|r| r.corridor == "CN→US").unwrap();
        assert!(cn_us.preferential_rate_pct.is_none());
    }

    #[test]
    fn full_product_lookup() {
        let result = lookup_product_tariff("Cable Harness Assembly");
        assert_eq!(result.hs4, "8544");
        assert!(!result.corridors.is_empty());
    }
}
