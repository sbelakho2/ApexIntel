//! Battlecard distribution — export to Markdown, Slack Block Kit, or PDF HTML.

use crate::battlecards::BattlecardData;

/// Formats battlecards for various output channels.
pub struct BattlecardDistributor;

impl BattlecardDistributor {
    /// Render the full battlecard as a Markdown string.
    pub fn to_markdown(card: &BattlecardData) -> String {
        let mut md = String::new();

        // Header
        md.push_str("# Competitive Battlecard\n\n");

        // Positioning
        md.push_str("## Positioning\n\n");
        md.push_str(&format!("**Market Position:** {}\n\n", card.positioning.market_position));
        md.push_str(&format!("**Value Proposition:** {}\n\n", card.positioning.value_proposition));
        if !card.positioning.differentiators.is_empty() {
            md.push_str("**Differentiators:**\n");
            for d in &card.positioning.differentiators {
                md.push_str(&format!("- {}\n", d));
            }
            md.push('\n');
        }

        // Pricing
        md.push_str("## Pricing\n\n");
        md.push_str(&format!(
            "**Model:** {} | **Range:** ${:.0} – ${:.0}\n\n",
            card.pricing.pricing_model, card.pricing.price_range_low, card.pricing.price_range_high
        ));

        // Feature Matrix
        md.push_str("## Feature Comparison\n\n");
        md.push_str(&format!("{}\n\n", card.feature_matrix.summary));
        for category in &card.feature_matrix.categories {
            md.push_str(&format!("### {}\n\n", category.category_name));
            md.push_str("| Feature | Us | Competitor | Advantage |\n");
            md.push_str("|---------|----|------------|-----------|\n");
            for f in &category.features {
                md.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    f.feature_name, f.our_support, f.competitor_support, f.advantage
                ));
            }
            md.push('\n');
        }

        // Strengths
        md.push_str("## Our Strengths\n\n");
        for s in &card.strengths {
            md.push_str(&format!("**{}** — {} ({})\n", s.title, s.description, s.impact_area));
            if !s.evidence_url.is_empty() {
                md.push_str(&format!("  Evidence: {}\n", s.evidence_url));
            }
        }
        md.push('\n');

        // Weaknesses
        md.push_str("## Competitor Weaknesses\n\n");
        for w in &card.weaknesses {
            md.push_str(&format!(
                "**{}** — {} (Severity: {:.0}%)\n",
                w.title, w.description, w.severity * 100.0
            ));
        }
        md.push('\n');

        // Objections
        md.push_str("## Objection Handlers\n\n");
        for oh in &card.objection_handlers {
            md.push_str(&format!(
                "**Objection:** {}\n**Response:** {}\n**Effectiveness:** {:.0}%\n\n",
                oh.objection, oh.counter_arg, oh.effectiveness * 100.0
            ));
        }

        // Kill Shots
        md.push_str("## Kill Shots\n\n");
        for ks in &card.kill_shots {
            md.push_str(&format!(
                "**{}** — {} (Priority: {:.2})\n",
                ks.title, ks.description, ks.priority_score
            ));
        }
        md.push('\n');

        // Recent News
        if !card.recent_news.is_empty() {
            md.push_str("## Recent News\n\n");
            for news in &card.recent_news {
                md.push_str(&format!("- [{}]({})\n", news.title, news.url));
            }
            md.push('\n');
        }

        // Win/Loss
        md.push_str("## Win/Loss Analysis\n\n");
        md.push_str(&format!(
            "**Win Rate:** {:.1}% ({} won / {} total)\n",
            card.win_loss.win_rate * 100.0,
            card.win_loss.won,
            card.win_loss.total_deals
        ));
        if !card.win_loss.top_loss_reasons.is_empty() {
            md.push_str("**Top Loss Reasons:**\n");
            for lr in &card.win_loss.top_loss_reasons {
                md.push_str(&format!(
                    "- {}: {} ({:.0}%)\n",
                    lr.reason, lr.count, lr.percentage * 100.0
                ));
            }
        }

        md
    }

    /// Generate Slack Block Kit JSON blocks for the battlecard.
    pub fn slack_blocks(card: &BattlecardData) -> Vec<serde_json::Value> {
        let mut blocks: Vec<serde_json::Value> = Vec::new();

        // Header
        blocks.push(serde_json::json!({
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "⚔ Competitive Battlecard"
            }
        }));

        // Positioning
        blocks.push(serde_json::json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!("*Positioning*\n{}", card.positioning.market_position)
            }
        }));

        // Feature summary
        blocks.push(serde_json::json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!("*Feature Summary*\n{}", card.feature_matrix.summary)
            }
        }));

        // Kill Shots (top 3)
        for ks in card.kill_shots.iter().take(3) {
            blocks.push(serde_json::json!({
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": format!("*🔪 {}*\n{}", ks.title, ks.description)
                }
            }));
        }

        // Win/Loss
        blocks.push(serde_json::json!({
            "type": "section",
            "text": {
                "type": "mrkdwn",
                "text": format!(
                    "*Win/Loss*\nWin Rate: {:.1}% | {} Won | {} Lost | Total Deals: {}",
                    card.win_loss.win_rate * 100.0,
                    card.win_loss.won,
                    card.win_loss.lost,
                    card.win_loss.total_deals
                )
            }
        }));

        blocks
    }

    /// Render the battlecard as an HTML fragment suitable for PDF generation.
    pub fn pdf_html(card: &BattlecardData) -> String {
        let mut html = String::new();
        html.push_str("<div class=\"battlecard\">");

        // Positioning
        html.push_str("<section><h2>Positioning</h2>");
        html.push_str(&format!("<p><strong>Market Position:</strong> {}</p>", card.positioning.market_position));
        html.push_str(&format!("<p><strong>Value Proposition:</strong> {}</p>", card.positioning.value_proposition));
        if !card.positioning.differentiators.is_empty() {
            html.push_str("<ul>");
            for d in &card.positioning.differentiators {
                html.push_str(&format!("<li>{}</li>", d));
            }
            html.push_str("</ul>");
        }
        html.push_str("</section>");

        // Feature Matrix
        html.push_str("<section><h2>Feature Comparison</h2>");
        html.push_str(&format!("<p>{}</p>", card.feature_matrix.summary));
        html.push_str("</section>");

        // Strengths
        if !card.strengths.is_empty() {
            html.push_str("<section><h2>Our Strengths</h2><ul>");
            for s in &card.strengths {
                html.push_str(&format!("<li><strong>{}</strong>: {}</li>", s.title, s.description));
            }
            html.push_str("</ul></section>");
        }

        // Kill Shots
        if !card.kill_shots.is_empty() {
            html.push_str("<section><h2>Kill Shots</h2><ul>");
            for ks in &card.kill_shots {
                html.push_str(&format!(
                    "<li><strong>{}</strong>: {} (Priority: {:.2})</li>",
                    ks.title, ks.description, ks.priority_score
                ));
            }
            html.push_str("</ul></section>");
        }

        html.push_str("</div>");
        html
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battlecards::{
        BattlecardData, FeatureCategory, FeatureComparisonData, FeatureMatrixSection, NewsItem,
        ObjectionHandlerPair, PositioningSection, PricingSection, StrengthItem, WeaknessItem,
        WinLossSection,
    };
    use crate::battlecards::kill_shot::KillShot;
    use crate::battlecards::win_loss_analyzer::LossReason;

    fn sample_battlecard() -> BattlecardData {
        BattlecardData {
            positioning: PositioningSection {
                market_position: "Market leader in cloud analytics".to_string(),
                value_proposition: "Faster insights with less data".to_string(),
                differentiators: vec!["Real-time processing".to_string()],
                target_segments: vec!["Enterprise".to_string()],
                brand_perception: "Innovative".to_string(),
            },
            pricing: PricingSection {
                pricing_model: "Subscription".to_string(),
                price_range_low: 10_000.0,
                price_range_high: 100_000.0,
                average_contract_value: Some(50_000.0),
                discounting_behavior: "Occasional".to_string(),
                competitive_position: "Premium".to_string(),
            },
            feature_matrix: FeatureMatrixSection {
                categories: vec![FeatureCategory {
                    category_name: "Analytics".to_string(),
                    features: vec![FeatureComparisonData {
                        feature_name: "Real-time".to_string(),
                        our_support: "Supported".to_string(),
                        competitor_support: "Partial".to_string(),
                        advantage: "Us".to_string(),
                    }],
                }],
                summary: "1 feature compared: 1 advantage us.".to_string(),
            },
            strengths: vec![StrengthItem {
                title: "Speed".to_string(),
                description: "Fast processing".to_string(),
                impact_area: "Performance".to_string(),
                evidence_url: "https://example.com".to_string(),
            }],
            weaknesses: vec![WeaknessItem {
                title: "Coverage".to_string(),
                description: "Limited regions".to_string(),
                impact_area: "Geography".to_string(),
                severity: 0.5,
            }],
            objection_handlers: vec![ObjectionHandlerPair {
                objection: "They are cheaper".to_string(),
                counter_arg: "We provide more value".to_string(),
                evidence_url: "https://example.com".to_string(),
                effectiveness: 0.8,
            }],
            kill_shots: vec![KillShot {
                title: "No AI capability".to_string(),
                description: "Competitor lacks AI".to_string(),
                evidence_url: "".to_string(),
                confidence: 0.9,
                severity: 0.7,
                market_relevance: 0.8,
                priority_score: 0.504,
            }],
            recent_news: vec![NewsItem {
                title: "Competitor launches new product".to_string(),
                url: "https://news.example.com".to_string(),
                published_at: None,
                relevance_score: 0.8,
            }],
            win_loss: WinLossSection {
                win_rate: 0.6,
                total_deals: 10,
                won: 6,
                lost: 4,
                total_value_won: 600_000.0,
                total_value_lost: 400_000.0,
                top_loss_reasons: vec![LossReason {
                    reason: "Price".to_string(),
                    count: 2,
                    percentage: 0.5,
                }],
                trends: vec![],
            },
        }
    }

    #[test]
    fn test_markdown_export_contains_all_sections() {
        let card = sample_battlecard();
        let md = BattlecardDistributor::to_markdown(&card);

        assert!(md.contains("Competitive Battlecard"));
        assert!(md.contains("Positioning"));
        assert!(md.contains("Pricing"));
        assert!(md.contains("Feature Comparison"));
        assert!(md.contains("Our Strengths"));
        assert!(md.contains("Competitor Weaknesses"));
        assert!(md.contains("Objection Handlers"));
        assert!(md.contains("Kill Shots"));
        assert!(md.contains("Recent News"));
        assert!(md.contains("Win/Loss Analysis"));
    }

    #[test]
    fn test_slack_blocks_are_valid() {
        let card = sample_battlecard();
        let blocks = BattlecardDistributor::slack_blocks(&card);

        assert!(!blocks.is_empty());
        for block in &blocks {
            assert!(block.get("type").is_some(), "each block must have a type");
        }
        assert_eq!(blocks[0]["type"], "header");
    }

    #[test]
    fn test_pdf_html_contains_sections() {
        let card = sample_battlecard();
        let html = BattlecardDistributor::pdf_html(&card);

        assert!(html.contains("battlecard"));
        assert!(html.contains("Positioning"));
        assert!(html.contains("Feature Comparison"));
        assert!(html.contains("Our Strengths"));
        assert!(html.contains("Kill Shots"));
    }
}
