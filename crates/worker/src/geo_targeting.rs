//! Geographic go-to-market targeting for Starz battery-pack sales.
//!
//! Starz sells its battery energy-storage packs (BESS) *mostly* into Morocco,
//! Tunisia and Egypt, with a *smaller* focus on the European Union, and nowhere
//! else. This module turns that commercial reality into a deterministic,
//! testable ranking signal so that demand-side opportunities in the target
//! markets rise to the top of the insight stream while out-of-footprint buyers
//! are de-prioritised.
//!
//! ## Scope — demand side only
//!
//! Geographic weighting applies **only** to demand-side entities: customers,
//! prospects, project developers, integrators and other potential pack buyers.
//!
//! Two entity classes are explicitly *exempt* and are always monitored globally
//! at full weight, regardless of where they are located:
//!
//! * **Competitors** (`is_competitor = true`) — rival pack / BMS makers
//!   (Pylontech, Dyness, Pytes, Fox ESS, …) must be tracked wherever they are.
//! * **Upstream suppliers** — cell manufacturers (CATL, BYD, LGES, …),
//!   distributors, semiconductor and PCB makers, test-equipment vendors and
//!   industry bodies. Starz buys from / depends on these globally.
//!
//! The single entry point used by the ranking pipeline is
//! [`ranking_geo_multiplier`]. Everything else is a building block for it and is
//! exercised directly by the unit tests.

use std::sync::LazyLock;

/// Default primary sales markets (ISO 3166-1 alpha-2 country codes): the three
/// North-African countries where Starz concentrates pack sales.
const DEFAULT_PRIMARY_MARKETS: &[&str] = &["TN", "MA", "EG"];

/// Default secondary sales markets: the 27 European Union member states. Starz
/// keeps a smaller EU focus, so buyers here are boosted mildly above neutral.
const DEFAULT_SECONDARY_MARKETS: &[&str] = &[
    "AT", "BE", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE", "GR", "HU", "IE", "IT", "LV",
    "LT", "LU", "MT", "NL", "PL", "PT", "RO", "SK", "SI", "ES", "SE",
];

/// `company_type` substrings (lower-cased) that mark an entity as upstream
/// supply / infrastructure rather than a pack buyer. Such entities are monitored
/// globally and never receive a geographic demand weight.
const GLOBAL_SUPPLY_TYPE_MARKERS: &[&str] = &[
    "cell",          // "Cell Manufacturer" / "Cell Supplier"
    "distributor",   // component distributors
    "semiconductor", // chip makers
    "pcb",           // bare-board makers
    "test",          // test & measurement vendors
    "trade association",
    "industry body",
    "component supplier",
    "raw material",
];

/// Parse a comma / semicolon / whitespace separated list of country codes from
/// an environment variable, upper-casing and de-duplicating, falling back to the
/// provided defaults when the variable is unset or yields no usable entries.
fn market_codes_from_env(key: &str, defaults: &[&str]) -> Vec<String> {
    if let Ok(raw) = std::env::var(key) {
        let parsed: Vec<String> = raw
            .split([',', ';', ' ', '\t', '\n'])
            .map(|token| token.trim().to_ascii_uppercase())
            .filter(|token| !token.is_empty())
            .collect();
        if parsed.is_empty() {
            tracing::warn!(
                env_var = key,
                raw_value = %raw,
                "geo targeting: env var contained no usable country codes, using defaults"
            );
        } else {
            let mut deduped: Vec<String> = Vec::with_capacity(parsed.len());
            for code in parsed {
                if !deduped.contains(&code) {
                    deduped.push(code);
                }
            }
            return deduped;
        }
    }
    defaults.iter().map(|code| code.to_string()).collect()
}

/// Primary sales markets. Override with `BESS_PRIMARY_MARKETS`
/// (e.g. `BESS_PRIMARY_MARKETS="TN,MA,EG"`).
static PRIMARY_MARKETS: LazyLock<Vec<String>> =
    LazyLock::new(|| market_codes_from_env("BESS_PRIMARY_MARKETS", DEFAULT_PRIMARY_MARKETS));

/// Secondary sales markets. Override with `BESS_SECONDARY_MARKETS`.
static SECONDARY_MARKETS: LazyLock<Vec<String>> =
    LazyLock::new(|| market_codes_from_env("BESS_SECONDARY_MARKETS", DEFAULT_SECONDARY_MARKETS));

/// Where a demand-side entity sits relative to Starz's sales footprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketTier {
    /// Primary market — Morocco, Tunisia, Egypt.
    Primary,
    /// Secondary market — European Union.
    Secondary,
    /// Confirmed outside every target market.
    OffTarget,
    /// Insufficient location data to classify; treated neutrally.
    Unknown,
}

/// Classify a demand-side entity into a [`MarketTier`] from its location.
///
/// `country_code` (ISO alpha-2) is the precise, authoritative signal. `region`
/// is only consulted as a coarse fallback when the country code is absent, and
/// only when it is unambiguous — ambiguous macro-regions such as "MENA" or
/// "North Africa" (which mix target and non-target countries) yield
/// [`MarketTier::Unknown`] so that no entity is wrongly penalised.
pub fn classify_market(country_code: Option<&str>, region: Option<&str>) -> MarketTier {
    if let Some(code) = country_code {
        let code = code.trim().to_ascii_uppercase();
        if !code.is_empty() {
            if PRIMARY_MARKETS.iter().any(|c| c == &code) {
                return MarketTier::Primary;
            }
            if SECONDARY_MARKETS.iter().any(|c| c == &code) {
                return MarketTier::Secondary;
            }
            return MarketTier::OffTarget;
        }
    }

    if let Some(region) = region {
        let region = region.trim().to_ascii_lowercase();
        if region.is_empty() {
            return MarketTier::Unknown;
        }
        // Unambiguously European Union footprint.
        if region == "eu" || region == "european union" || region.contains("europe") {
            return MarketTier::Secondary;
        }
        // Unambiguously outside the footprint.
        const OFF_TARGET_REGIONS: &[&str] = &[
            "north america",
            "south america",
            "latin america",
            "asia",
            "asia-pacific",
            "asia pacific",
            "apac",
            "oceania",
            "caribbean",
            "central america",
        ];
        if OFF_TARGET_REGIONS.iter().any(|r| region.contains(r)) {
            return MarketTier::OffTarget;
        }
        // "mena", "north africa", "middle east", "africa", "global" mix target
        // and non-target countries → cannot decide safely.
    }

    MarketTier::Unknown
}

/// Ranking multiplier for a demand-side entity in the given market tier.
pub fn demand_geo_weight(tier: MarketTier) -> f64 {
    match tier {
        MarketTier::Primary => *crate::config::GEO_PRIMARY_DEMAND_WEIGHT,
        MarketTier::Secondary => *crate::config::GEO_SECONDARY_DEMAND_WEIGHT,
        MarketTier::OffTarget => *crate::config::GEO_OFFTARGET_DEMAND_WEIGHT,
        MarketTier::Unknown => 1.0,
    }
}

/// Whether an entity is monitored globally (competitors and upstream suppliers)
/// and must therefore be exempt from any geographic demand weighting.
pub fn is_global_monitoring_entity(is_competitor: bool, company_type: Option<&str>) -> bool {
    if is_competitor {
        return true;
    }
    if let Some(company_type) = company_type {
        let company_type = company_type.to_ascii_lowercase();
        return GLOBAL_SUPPLY_TYPE_MARKERS
            .iter()
            .any(|marker| company_type.contains(marker));
    }
    false
}

/// Single entry point for the ranking pipeline.
///
/// Returns the multiplier to fold into an insight candidate's ranking score:
///
/// * `1.0` for competitors and upstream suppliers (global monitoring — never
///   re-weighted by geography);
/// * otherwise the demand weight for the entity's [`MarketTier`].
pub fn ranking_geo_multiplier(
    is_competitor: bool,
    company_type: Option<&str>,
    country_code: Option<&str>,
    region: Option<&str>,
) -> f64 {
    if is_global_monitoring_entity(is_competitor, company_type) {
        return 1.0;
    }
    demand_geo_weight(classify_market(country_code, region))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_markets_are_morocco_tunisia_egypt() {
        assert_eq!(classify_market(Some("MA"), None), MarketTier::Primary);
        assert_eq!(classify_market(Some("TN"), None), MarketTier::Primary);
        assert_eq!(classify_market(Some("EG"), None), MarketTier::Primary);
        // Case / whitespace insensitive.
        assert_eq!(classify_market(Some(" eg "), None), MarketTier::Primary);
    }

    #[test]
    fn eu_members_are_secondary() {
        for code in ["FR", "DE", "ES", "IT", "PL", "NL"] {
            assert_eq!(
                classify_market(Some(code), None),
                MarketTier::Secondary,
                "{code} should be a secondary (EU) market"
            );
        }
    }

    #[test]
    fn non_target_countries_are_off_target() {
        // United States, Israel, China, UK, Norway, South Africa, UAE, Nigeria,
        // Algeria, Libya — none are sales targets.
        for code in ["US", "IL", "CN", "GB", "NO", "ZA", "AE", "NG", "DZ", "LY"] {
            assert_eq!(
                classify_market(Some(code), None),
                MarketTier::OffTarget,
                "{code} should be off-target"
            );
        }
    }

    #[test]
    fn region_fallback_is_conservative() {
        // Ambiguous macro-regions must not penalise — they stay Unknown.
        assert_eq!(classify_market(None, Some("MENA")), MarketTier::Unknown);
        assert_eq!(
            classify_market(None, Some("North Africa")),
            MarketTier::Unknown
        );
        assert_eq!(
            classify_market(None, Some("Middle East")),
            MarketTier::Unknown
        );
        assert_eq!(classify_market(None, Some("Global")), MarketTier::Unknown);
        assert_eq!(classify_market(None, None), MarketTier::Unknown);
        // Unambiguous regions still classify.
        assert_eq!(classify_market(None, Some("Europe")), MarketTier::Secondary);
        assert_eq!(classify_market(None, Some("EU")), MarketTier::Secondary);
        assert_eq!(
            classify_market(None, Some("North America")),
            MarketTier::OffTarget
        );
        assert_eq!(
            classify_market(None, Some("Asia-Pacific")),
            MarketTier::OffTarget
        );
    }

    #[test]
    fn country_code_overrides_region() {
        // Precise country code wins over a broad region label.
        assert_eq!(
            classify_market(Some("EG"), Some("North America")),
            MarketTier::Primary
        );
        assert_eq!(
            classify_market(Some("US"), Some("Europe")),
            MarketTier::OffTarget
        );
    }

    #[test]
    fn competitors_are_always_global() {
        assert!(is_global_monitoring_entity(true, None));
        assert!(is_global_monitoring_entity(true, Some("OEM")));
        // A competitor in a primary market is still monitored globally (weight 1.0).
        assert_eq!(
            ranking_geo_multiplier(true, Some("Pack Maker"), Some("MA"), None),
            1.0
        );
        // …and a competitor outside every market is not penalised either.
        assert_eq!(ranking_geo_multiplier(true, None, Some("CN"), None), 1.0);
    }

    #[test]
    fn cell_suppliers_are_always_global() {
        assert!(is_global_monitoring_entity(
            false,
            Some("Cell Manufacturer")
        ));
        assert!(is_global_monitoring_entity(
            false,
            Some("Component Distributor")
        ));
        assert!(is_global_monitoring_entity(false, Some("Semiconductor")));
        // A Chinese cell supplier is monitored at full weight, never down-weighted.
        assert_eq!(
            ranking_geo_multiplier(false, Some("Cell Manufacturer"), Some("CN"), None),
            1.0
        );
        // A pack buyer (no supply marker) is not exempt.
        assert!(!is_global_monitoring_entity(
            false,
            Some("Project Developer")
        ));
        assert!(!is_global_monitoring_entity(false, Some("OEM")));
        assert!(!is_global_monitoring_entity(false, None));
    }

    #[test]
    fn demand_weight_ordering_primary_secondary_offtarget() {
        let primary = ranking_geo_multiplier(false, Some("OEM"), Some("MA"), None);
        let secondary = ranking_geo_multiplier(false, Some("OEM"), Some("FR"), None);
        let off_target = ranking_geo_multiplier(false, Some("OEM"), Some("US"), None);
        let unknown = ranking_geo_multiplier(false, Some("OEM"), None, None);

        assert!(primary > secondary, "primary should outrank secondary");
        assert!(secondary > unknown, "secondary should outrank neutral");
        assert!(unknown > off_target, "off-target should rank below neutral");
        assert!(
            (unknown - 1.0).abs() < f64::EPSILON,
            "unknown is neutral 1.0"
        );
    }

    #[test]
    fn weight_helpers_match_config_defaults() {
        assert_eq!(
            demand_geo_weight(MarketTier::Primary),
            *crate::config::GEO_PRIMARY_DEMAND_WEIGHT
        );
        assert_eq!(
            demand_geo_weight(MarketTier::Secondary),
            *crate::config::GEO_SECONDARY_DEMAND_WEIGHT
        );
        assert_eq!(
            demand_geo_weight(MarketTier::OffTarget),
            *crate::config::GEO_OFFTARGET_DEMAND_WEIGHT
        );
        assert_eq!(demand_geo_weight(MarketTier::Unknown), 1.0);
    }
}
