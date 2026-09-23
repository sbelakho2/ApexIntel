//! Win/Loss Analyzer — analyzes closed deal data to compute win rates,
//! loss reasons, and trends against specific competitors.

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A closed deal for win/loss analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedDeal {
    pub deal_name: String,
    pub value: f64,
    pub won: bool,
    pub loss_reason: Option<String>,
    pub competitor_name: String,
    pub closed_at: DateTime<Utc>,
}

/// Win/loss analysis results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinLossAnalysis {
    pub total_deals: u32,
    pub won: u32,
    pub lost: u32,
    pub win_rate: f64,
    pub total_value_won: f64,
    pub total_value_lost: f64,
    pub top_loss_reasons: Vec<LossReason>,
    pub trends: Vec<WinLossTrend>,
}

/// A single loss reason with frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LossReason {
    pub reason: String,
    pub count: u32,
    pub percentage: f64,
}

/// Win/loss trend for a given time period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WinLossTrend {
    pub period: String,
    pub win_rate: f64,
    pub deals_count: u32,
}

/// Analyzes win/loss data for competitive insights.
pub struct WinLossAnalyzer;

impl WinLossAnalyzer {
    /// Analyze closed deals against a specific competitor.
    pub fn analyze(
        _our_company_id: Uuid,
        _competitor_id: Uuid,
        deals: &[ClosedDeal],
    ) -> WinLossAnalysis {
        let total_deals = deals.len() as u32;
        let won = deals.iter().filter(|d| d.won).count() as u32;
        let lost = deals.iter().filter(|d| !d.won).count() as u32;
        let win_rate = if total_deals > 0 {
            won as f64 / total_deals as f64
        } else {
            0.0
        };

        let total_value_won: f64 = deals.iter().filter(|d| d.won).map(|d| d.value).sum();
        let total_value_lost: f64 = deals.iter().filter(|d| !d.won).map(|d| d.value).sum();

        // Aggregate loss reasons
        let loss_reasons = Self::aggregate_loss_reasons(deals);
        let trends = Self::calculate_trends(deals);

        WinLossAnalysis {
            total_deals,
            won,
            lost,
            win_rate,
            total_value_won,
            total_value_lost,
            top_loss_reasons: loss_reasons,
            trends,
        }
    }

    fn aggregate_loss_reasons(deals: &[ClosedDeal]) -> Vec<LossReason> {
        let mut reason_counts: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();
        let lost_deals: Vec<&ClosedDeal> = deals.iter().filter(|d| !d.won).collect();
        let total_lost = lost_deals.len() as f64;

        for deal in &lost_deals {
            if let Some(reason) = &deal.loss_reason {
                *reason_counts.entry(reason.clone()).or_insert(0) += 1;
            } else {
                *reason_counts.entry("Unknown".to_string()).or_insert(0) += 1;
            }
        }

        let mut reasons: Vec<LossReason> = reason_counts
            .into_iter()
            .map(|(reason, count)| LossReason {
                reason,
                count,
                percentage: if total_lost > 0.0 {
                    count as f64 / total_lost
                } else {
                    0.0
                },
            })
            .collect();

        reasons.sort_by_key(|a| std::cmp::Reverse(a.count));
        reasons.truncate(5);
        reasons
    }

    fn calculate_trends(deals: &[ClosedDeal]) -> Vec<WinLossTrend> {
        let mut period_map: std::collections::BTreeMap<String, Vec<&ClosedDeal>> =
            std::collections::BTreeMap::new();

        for deal in deals {
            let period = format!(
                "{}-Q{}",
                deal.closed_at.year(),
                ((deal.closed_at.month() - 1) / 3) + 1
            );
            period_map.entry(period).or_default().push(deal);
        }

        period_map
            .into_iter()
            .map(|(period, period_deals)| {
                let total = period_deals.len() as u32;
                let won = period_deals.iter().filter(|d| d.won).count() as u32;
                let win_rate = if total > 0 {
                    won as f64 / total as f64
                } else {
                    0.0
                };
                WinLossTrend {
                    period,
                    win_rate,
                    deals_count: total,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_deal(
        name: &str,
        value: f64,
        won: bool,
        loss_reason: Option<&str>,
        date: (i32, u32, u32),
    ) -> ClosedDeal {
        ClosedDeal {
            deal_name: name.to_string(),
            value,
            won,
            loss_reason: loss_reason.map(|s| s.to_string()),
            competitor_name: "RivalCorp".to_string(),
            closed_at: Utc
                .with_ymd_and_hms(date.0, date.1, date.2, 0, 0, 0)
                .unwrap(),
        }
    }

    #[test]
    fn test_analyze_basic_win_rate() {
        let deals = vec![
            make_deal("Deal1", 100_000.0, true, None, (2026, 1, 15)),
            make_deal("Deal2", 200_000.0, false, Some("Price"), (2026, 2, 15)),
            make_deal("Deal3", 150_000.0, true, None, (2026, 3, 15)),
        ];

        let analysis = WinLossAnalyzer::analyze(Uuid::nil(), Uuid::nil(), &deals);
        assert_eq!(analysis.total_deals, 3);
        assert_eq!(analysis.won, 2);
        assert_eq!(analysis.lost, 1);
        assert!((analysis.win_rate - 2.0 / 3.0).abs() < 0.001);
        assert!((analysis.total_value_won - 250_000.0).abs() < 0.001);
        assert!((analysis.total_value_lost - 200_000.0).abs() < 0.001);
    }

    #[test]
    fn test_aggregate_loss_reasons() {
        let deals = vec![
            make_deal("Deal1", 100_000.0, false, Some("Price"), (2026, 1, 15)),
            make_deal("Deal2", 100_000.0, false, Some("Price"), (2026, 2, 15)),
            make_deal("Deal3", 100_000.0, false, Some("Features"), (2026, 3, 15)),
            make_deal("Deal4", 100_000.0, true, None, (2026, 4, 15)),
        ];

        let analysis = WinLossAnalyzer::analyze(Uuid::nil(), Uuid::nil(), &deals);
        assert_eq!(analysis.top_loss_reasons.len(), 2);
        assert_eq!(analysis.top_loss_reasons[0].reason, "Price");
        assert_eq!(analysis.top_loss_reasons[0].count, 2);
    }

    #[test]
    fn test_trends_calculation() {
        let deals = vec![
            make_deal("Deal1", 100_000.0, true, None, (2026, 1, 15)),
            make_deal("Deal2", 100_000.0, false, Some("Price"), (2026, 2, 15)),
            make_deal("Deal3", 100_000.0, true, None, (2026, 5, 15)),
        ];

        let analysis = WinLossAnalyzer::analyze(Uuid::nil(), Uuid::nil(), &deals);
        assert_eq!(analysis.trends.len(), 2);
        assert_eq!(analysis.trends[0].period, "2026-Q1");
        assert_eq!(analysis.trends[0].deals_count, 2);
        assert_eq!(analysis.trends[1].period, "2026-Q2");
        assert_eq!(analysis.trends[1].deals_count, 1);
    }

    #[test]
    fn test_empty_deals() {
        let analysis = WinLossAnalyzer::analyze(Uuid::nil(), Uuid::nil(), &[]);
        assert_eq!(analysis.total_deals, 0);
        assert_eq!(analysis.win_rate, 0.0);
        assert!(analysis.top_loss_reasons.is_empty());
        assert!(analysis.trends.is_empty());
    }
}
