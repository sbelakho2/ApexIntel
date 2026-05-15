use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceEntropyReport {
    pub top_source_share: f64,
    pub shannon_entropy: f64,
    pub effective_source_count: f64,
    pub anomalous: bool,
}

pub fn source_entropy_report(
    source_counts: &HashMap<String, usize>,
    top_k: usize,
) -> SourceEntropyReport {
    let mut counts = source_counts.values().copied().collect::<Vec<_>>();
    counts.sort_unstable_by(|left, right| right.cmp(left));
    counts.truncate(top_k);
    let total = counts.iter().sum::<usize>() as f64;

    if total <= f64::EPSILON {
        return SourceEntropyReport {
            top_source_share: 0.0,
            shannon_entropy: 0.0,
            effective_source_count: 0.0,
            anomalous: false,
        };
    }

    let probabilities = counts
        .iter()
        .map(|count| *count as f64 / total)
        .collect::<Vec<_>>();
    let top_source_share = probabilities.first().copied().unwrap_or(0.0);
    let shannon_entropy = probabilities
        .iter()
        .filter(|probability| **probability > 0.0)
        .map(|probability| -probability * probability.ln())
        .sum::<f64>();
    let effective_source_count = shannon_entropy.exp();
    let anomalous = top_source_share >= 0.35 || effective_source_count < 5.0;

    SourceEntropyReport {
        top_source_share,
        shannon_entropy,
        effective_source_count,
        anomalous,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concentrated_source_mix_is_anomalous() {
        let counts = HashMap::from([
            ("source-a".to_string(), 40usize),
            ("source-b".to_string(), 5usize),
            ("source-c".to_string(), 3usize),
            ("source-d".to_string(), 2usize),
        ]);
        let report = source_entropy_report(&counts, 50);
        assert!(report.anomalous);
        assert!(report.top_source_share > 0.35);
    }

    #[test]
    fn diversified_source_mix_is_not_anomalous() {
        let counts = HashMap::from([
            ("source-a".to_string(), 8usize),
            ("source-b".to_string(), 7usize),
            ("source-c".to_string(), 6usize),
            ("source-d".to_string(), 5usize),
            ("source-e".to_string(), 5usize),
            ("source-f".to_string(), 4usize),
            ("source-g".to_string(), 4usize),
            ("source-h".to_string(), 3usize),
            ("source-i".to_string(), 3usize),
            ("source-j".to_string(), 3usize),
        ]);
        let report = source_entropy_report(&counts, 50);
        assert!(!report.anomalous);
        assert!(report.effective_source_count >= 5.0);
    }

    #[test]
    fn source_entropy_anomaly() {
        let counts = HashMap::from([("source-a".to_string(), 12usize)]);
        let report = source_entropy_report(&counts, 50);
        assert_eq!(report.shannon_entropy, 0.0);
        assert!(report.anomalous);
    }
}
