//! Geospatial proximity calculator for sites and logistics nodes.
//!
//! Features:
//! - Haversine great-circle distance
//! - Road-factor adjusted estimates
//! - Nearest port/airport lookup
//! - Logistics corridor classification (Mediterranean, Atlantic, Red Sea, Pacific)
//!
//! Used by: supply chain analysis, site comparison, logistics scoring.

use serde::Serialize;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProximityResult {
    pub straight_line_km: f64,
    pub estimated_road_km: f64,
    pub nearest_port: Option<NamedPoint>,
    pub nearest_airport: Option<NamedPoint>,
    pub logistics_corridor: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamedPoint {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub distance_km: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Haversine
// ─────────────────────────────────────────────────────────────────────────────

const EARTH_RADIUS_KM: f64 = 6371.0;
const ROAD_FACTOR: f64 = 1.35; // empirical straight → road multiplier

/// Haversine great-circle distance in kilometers.
pub fn haversine_km(a: GeoPoint, b: GeoPoint) -> f64 {
    let d_lat = (b.lat - a.lat).to_radians();
    let d_lon = (b.lon - a.lon).to_radians();
    let lat1 = a.lat.to_radians();
    let lat2 = b.lat.to_radians();

    let h = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().asin()
}

/// Estimated driving distance (Haversine × road factor).
pub fn estimated_road_km(a: GeoPoint, b: GeoPoint) -> f64 {
    haversine_km(a, b) * ROAD_FACTOR
}

// ─────────────────────────────────────────────────────────────────────────────
// Major ports (EMS/electronics relevant)
// ─────────────────────────────────────────────────────────────────────────────

const MAJOR_PORTS: &[(&str, f64, f64)] = &[
    // Mediterranean
    ("Tunis (Radès)", 36.81, 10.28),
    ("Casablanca", 33.59, -7.62),
    ("Tangier Med", 35.87, -5.50),
    ("Haifa", 32.82, 35.00),
    ("Barcelona", 41.35, 2.17),
    ("Marseille", 43.30, 5.37),
    ("Genoa", 44.41, 8.93),
    ("Piraeus", 37.94, 23.63),
    // Atlantic / Northern Europe
    ("Rotterdam", 51.95, 4.14),
    ("Hamburg", 53.54, 9.97),
    ("Antwerp", 51.26, 4.39),
    ("Southampton", 50.89, -1.40),
    // Red Sea / Gulf
    ("Jeddah", 21.49, 39.19),
    ("Dubai (Jebel Ali)", 25.00, 55.06),
    // Pacific / Asia
    ("Shanghai", 31.23, 121.47),
    ("Shenzhen (Yantian)", 22.57, 114.27),
    ("Kaohsiung", 22.61, 120.29),
    ("Busan", 35.10, 129.04),
    ("Tokyo (Yokohama)", 35.44, 139.66),
    // Americas
    ("Los Angeles", 33.74, -118.26),
    ("New York (Newark)", 40.68, -74.15),
    ("Savannah", 32.08, -81.09),
];

const MAJOR_AIRPORTS: &[(&str, f64, f64)] = &[
    ("Tunis-Carthage", 36.85, 10.23),
    ("Mohammed V (Casablanca)", 33.37, -7.59),
    ("Ben Gurion (Tel Aviv)", 32.01, 34.87),
    ("Charles de Gaulle (Paris)", 49.01, 2.55),
    ("Frankfurt", 50.03, 8.57),
    ("Heathrow (London)", 51.47, -0.45),
    ("Schiphol (Amsterdam)", 52.31, 4.76),
    ("Dubai International", 25.25, 55.36),
    ("Shanghai Pudong", 31.14, 121.81),
    ("Hong Kong", 22.31, 113.91),
    ("Incheon (Seoul)", 37.46, 126.44),
    ("Narita (Tokyo)", 35.76, 140.39),
    ("JFK (New York)", 40.64, -73.78),
    ("LAX (Los Angeles)", 33.94, -118.41),
];

// ─────────────────────────────────────────────────────────────────────────────
// Nearest-facility lookup
// ─────────────────────────────────────────────────────────────────────────────

/// Find the nearest port to a given point.
pub fn nearest_port(point: GeoPoint) -> NamedPoint {
    nearest_facility(point, MAJOR_PORTS)
}

/// Find the nearest airport to a given point.
pub fn nearest_airport(point: GeoPoint) -> NamedPoint {
    nearest_facility(point, MAJOR_AIRPORTS)
}

fn nearest_facility(point: GeoPoint, facilities: &[(&str, f64, f64)]) -> NamedPoint {
    assert!(!facilities.is_empty(), "facilities list must not be empty");
    facilities
        .iter()
        .map(|(name, lat, lon)| {
            let dist = haversine_km(
                point,
                GeoPoint {
                    lat: *lat,
                    lon: *lon,
                },
            );
            NamedPoint {
                name: name.to_string(),
                lat: *lat,
                lon: *lon,
                distance_km: dist,
            }
        })
        .min_by(|a, b| {
            a.distance_km
                .partial_cmp(&b.distance_km)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap()
}

// ─────────────────────────────────────────────────────────────────────────────
// Logistics corridor classification
// ─────────────────────────────────────────────────────────────────────────────

/// Classify the logistics corridor between two points.
pub fn classify_corridor(a: GeoPoint, b: GeoPoint) -> &'static str {
    let mid_lat = (a.lat + b.lat) / 2.0;
    let mid_lon = (a.lon + b.lon) / 2.0;

    // Mediterranean corridor
    if mid_lat > 30.0 && mid_lat < 46.0 && mid_lon > -10.0 && mid_lon < 40.0 {
        return "Mediterranean";
    }
    // Atlantic corridor
    if mid_lon < -10.0 && mid_lat > 25.0 && mid_lat < 60.0 {
        return "Atlantic";
    }
    // Red Sea / Suez corridor
    if mid_lat > 10.0 && mid_lat < 35.0 && mid_lon > 30.0 && mid_lon < 60.0 {
        return "Red Sea / Suez";
    }
    // Pacific corridor
    if mid_lon > 100.0 || mid_lon < -100.0 {
        return "Pacific";
    }
    // Northern Europe
    if mid_lat > 46.0 && mid_lon > -10.0 && mid_lon < 30.0 {
        return "Northern Europe";
    }
    "Other"
}

// ─────────────────────────────────────────────────────────────────────────────
// Full proximity analysis
// ─────────────────────────────────────────────────────────────────────────────

/// Compute a full proximity analysis between two points.
pub fn compute_proximity(a: GeoPoint, b: GeoPoint) -> ProximityResult {
    let straight = haversine_km(a, b);
    let road = straight * ROAD_FACTOR;
    let mid = GeoPoint {
        lat: (a.lat + b.lat) / 2.0,
        lon: (a.lon + b.lon) / 2.0,
    };

    ProximityResult {
        straight_line_km: (straight * 10.0).round() / 10.0,
        estimated_road_km: (road * 10.0).round() / 10.0,
        nearest_port: Some(nearest_port(mid)),
        nearest_airport: Some(nearest_airport(mid)),
        logistics_corridor: classify_corridor(a, b).into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_tunis_casablanca() {
        let tunis = GeoPoint {
            lat: 36.81,
            lon: 10.17,
        };
        let casa = GeoPoint {
            lat: 33.57,
            lon: -7.59,
        };
        let d = haversine_km(tunis, casa);
        assert!(d > 1500.0 && d < 1700.0, "Expected ~1600km, got {}", d);
    }

    #[test]
    fn haversine_same_point() {
        let p = GeoPoint {
            lat: 48.86,
            lon: 2.35,
        };
        assert!(haversine_km(p, p) < 0.01);
    }

    #[test]
    fn nearest_port_tunis() {
        let tunis = GeoPoint {
            lat: 36.81,
            lon: 10.17,
        };
        let port = nearest_port(tunis);
        assert!(port.name.contains("Tunis") || port.name.contains("Radès"));
        assert!(port.distance_km < 20.0);
    }

    #[test]
    fn nearest_airport_paris() {
        let paris = GeoPoint {
            lat: 48.86,
            lon: 2.35,
        };
        let ap = nearest_airport(paris);
        assert!(ap.name.contains("Charles de Gaulle"));
    }

    #[test]
    fn corridor_mediterranean() {
        let tunis = GeoPoint {
            lat: 36.81,
            lon: 10.17,
        };
        let barcelona = GeoPoint {
            lat: 41.39,
            lon: 2.17,
        };
        assert_eq!(classify_corridor(tunis, barcelona), "Mediterranean");
    }

    #[test]
    fn corridor_pacific() {
        let shanghai = GeoPoint {
            lat: 31.23,
            lon: 121.47,
        };
        let la = GeoPoint {
            lat: 33.74,
            lon: -118.26,
        };
        assert_eq!(classify_corridor(shanghai, la), "Pacific");
    }

    #[test]
    fn full_proximity() {
        let tunis = GeoPoint {
            lat: 36.81,
            lon: 10.17,
        };
        let casa = GeoPoint {
            lat: 33.57,
            lon: -7.59,
        };
        let result = compute_proximity(tunis, casa);
        assert!(result.straight_line_km > 1500.0);
        assert!(result.estimated_road_km > result.straight_line_km);
        assert!(result.nearest_port.is_some());
        assert_eq!(result.logistics_corridor, "Mediterranean");
    }
}
