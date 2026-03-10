use serde::{Deserialize, Serialize};

/// Shared severity-based SLA windows across alerting subsystems.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeveritySlaConfig {
    pub critical_seconds: i64,
    pub high_seconds: i64,
    pub medium_seconds: i64,
    pub low_seconds: i64,
}

impl Default for SeveritySlaConfig {
    fn default() -> Self {
        Self {
            critical_seconds: 900,
            high_seconds: 3_600,
            medium_seconds: 14_400,
            low_seconds: 86_400,
        }
    }
}

impl SeveritySlaConfig {
    pub fn deadline_seconds(&self, priority: &str) -> i64 {
        match priority.to_ascii_lowercase().as_str() {
            "p0" | "critical" => self.critical_seconds,
            "p1" | "high" => self.high_seconds,
            "p2" | "medium" => self.medium_seconds,
            _ => self.low_seconds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SeveritySlaConfig;

    #[test]
    fn deadline_seconds_maps_priority_aliases() {
        let config = SeveritySlaConfig::default();
        assert_eq!(config.deadline_seconds("P0"), 900);
        assert_eq!(config.deadline_seconds("high"), 3_600);
        assert_eq!(config.deadline_seconds("medium"), 14_400);
        assert_eq!(config.deadline_seconds("anything-else"), 86_400);
    }
}
