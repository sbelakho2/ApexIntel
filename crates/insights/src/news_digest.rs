//! Regional news digest generator.
//!
//! Weekly auto-digest of regional news per market (TN, MA, IL, CN, EU, US)
//! filtered by EMS/electronics relevance. Powered by the RSS aggregator
//! module + LLM summarization prompts.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use std::collections::HashMap;

// ─── Digest types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RegionalDigest {
    pub region: String,
    pub region_name: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub generated_at: DateTime<Utc>,
    pub sections: Vec<DigestSection>,
    pub top_stories: Vec<DigestStory>,
    pub statistics: DigestStats,
}

#[derive(Debug, Clone, Serialize)]
pub struct DigestSection {
    pub title: String,
    pub category: NewsCategory,
    pub stories: Vec<DigestStory>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DigestStory {
    pub title: String,
    pub source: String,
    pub source_url: String,
    pub published_at: DateTime<Utc>,
    pub summary: String,
    pub relevance_score: f64,
    pub entities_mentioned: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum NewsCategory {
    CompetitorActivity,
    IndustryTrends,
    Regulation,
    SupplyChain,
    TradePolicy,
    Technology,
    Personnel,
    Economic,
}

impl NewsCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::CompetitorActivity => "Competitor Activity",
            Self::IndustryTrends => "Industry Trends",
            Self::Regulation => "Regulatory Updates",
            Self::SupplyChain => "Supply Chain",
            Self::TradePolicy => "Trade Policy",
            Self::Technology => "Technology",
            Self::Personnel => "Personnel Moves",
            Self::Economic => "Economic Indicators",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DigestStats {
    pub total_articles_scanned: usize,
    pub relevant_articles: usize,
    pub entities_mentioned: usize,
    pub categories_covered: usize,
}

// ─── Region configuration ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RegionConfig {
    pub code: String,
    pub name: String,
    /// RSS feed URLs for this region
    pub feeds: Vec<String>,
    /// Keywords that boost relevance for this region
    pub relevance_keywords: Vec<String>,
}

/// Default region configurations for EMS/electronics intelligence.
pub fn default_regions() -> Vec<RegionConfig> {
    vec![
        RegionConfig {
            code: "TN".into(),
            name: "Tunisia".into(),
            feeds: vec![
                "https://www.tap.info.tn/en/rss/economy".into(),
                "https://www.webmanagercenter.com/feed/".into(),
            ],
            relevance_keywords: vec![
                "electronics".into(),
                "manufacturing".into(),
                "export".into(),
                "investment".into(),
                "industry".into(),
                "Starz".into(),
                "zone franche".into(),
                "automobile".into(),
                "câblage".into(),
            ],
        },
        RegionConfig {
            code: "MA".into(),
            name: "Morocco".into(),
            feeds: vec![
                "https://www.medias24.com/feed/".into(),
                "https://www.leseco.ma/feed/".into(),
            ],
            relevance_keywords: vec![
                "electronics".into(),
                "automotive".into(),
                "Tanger".into(),
                "free zone".into(),
                "manufacturing".into(),
                "cable".into(),
                "aéronautique".into(),
                "industrie".into(),
            ],
        },
        RegionConfig {
            code: "IL".into(),
            name: "Israel".into(),
            feeds: vec![
                "https://www.calcalistech.com/rss/540".into(),
                "https://www.globes.co.il/news/rss/rss.technology.xml".into(),
            ],
            relevance_keywords: vec![
                "electronics".into(),
                "semiconductor".into(),
                "defense".into(),
                "technology".into(),
                "startup".into(),
                "manufacturing".into(),
                "high-tech".into(),
            ],
        },
        RegionConfig {
            code: "CN".into(),
            name: "China".into(),
            feeds: vec!["https://www.scmp.com/rss/4/feed".into()],
            relevance_keywords: vec![
                "electronics".into(),
                "semiconductor".into(),
                "PCB".into(),
                "manufacturing".into(),
                "export".into(),
                "supply chain".into(),
                "Shenzhen".into(),
                "Foxconn".into(),
            ],
        },
        RegionConfig {
            code: "EU".into(),
            name: "European Union".into(),
            feeds: vec!["https://www.eenewseurope.com/rss.xml".into()],
            relevance_keywords: vec![
                "electronics".into(),
                "EMS".into(),
                "automotive".into(),
                "semiconductor".into(),
                "regulation".into(),
                "REACH".into(),
                "RoHS".into(),
                "tariff".into(),
            ],
        },
        RegionConfig {
            code: "US".into(),
            name: "United States".into(),
            feeds: vec!["https://www.eetimes.com/feed/".into()],
            relevance_keywords: vec![
                "electronics".into(),
                "semiconductor".into(),
                "CHIPS Act".into(),
                "manufacturing".into(),
                "defense".into(),
                "supply chain".into(),
                "reshoring".into(),
            ],
        },
    ]
}

// ─── Relevance scoring ──────────────────────────────────────────────────

/// Score an article's relevance for a region based on keyword matches.
pub fn score_relevance(title: &str, summary: &str, keywords: &[String]) -> f64 {
    let text = format!("{} {}", title, summary).to_lowercase();
    let mut matches = 0;
    for kw in keywords {
        if text.contains(&kw.to_lowercase()) {
            matches += 1;
        }
    }
    if keywords.is_empty() {
        return 0.0;
    }
    (matches as f64 / keywords.len() as f64).min(1.0)
}

/// Classify a news article into a category based on content.
pub fn classify_article(title: &str, summary: &str) -> NewsCategory {
    let text = format!("{} {}", title, summary).to_lowercase();

    if text.contains("competitor")
        || text.contains("rival")
        || text.contains("acquired")
        || text.contains("merger")
    {
        return NewsCategory::CompetitorActivity;
    }
    if text.contains("regulation")
        || text.contains("compliance")
        || text.contains("law")
        || text.contains("directive")
    {
        return NewsCategory::Regulation;
    }
    if text.contains("supply chain")
        || text.contains("shortage")
        || text.contains("logistics")
        || text.contains("shipping")
    {
        return NewsCategory::SupplyChain;
    }
    if text.contains("tariff")
        || text.contains("trade")
        || text.contains("import")
        || text.contains("export")
        || text.contains("sanctions")
    {
        return NewsCategory::TradePolicy;
    }
    if text.contains("hire")
        || text.contains("appointment")
        || text.contains("CEO")
        || text.contains("director")
        || text.contains("resign")
    {
        return NewsCategory::Personnel;
    }
    if text.contains("GDP")
        || text.contains("inflation")
        || text.contains("currency")
        || text.contains("investment")
    {
        return NewsCategory::Economic;
    }
    if text.contains("technology")
        || text.contains("innovation")
        || text.contains("patent")
        || text.contains("R&D")
    {
        return NewsCategory::Technology;
    }

    NewsCategory::IndustryTrends
}

/// Build a digest from scored and classified articles.
pub fn build_digest(
    region: &RegionConfig,
    articles: &[(String, String, String, DateTime<Utc>, f64)], // (title, summary, url, date, score)
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> RegionalDigest {
    let mut by_category: HashMap<String, Vec<DigestStory>> = HashMap::new();

    for (title, summary, url, date, score) in articles {
        if *score < 0.1 {
            continue;
        }
        let category = classify_article(title, summary);
        let story = DigestStory {
            title: title.clone(),
            source: extract_domain(url),
            source_url: url.clone(),
            published_at: *date,
            summary: summary.clone(),
            relevance_score: *score,
            entities_mentioned: Vec::new(),
            tags: Vec::new(),
        };
        by_category
            .entry(category.label().to_string())
            .or_default()
            .push(story);
    }

    let mut sections: Vec<DigestSection> = by_category
        .into_iter()
        .map(|(title, mut stories)| {
            stories.sort_by(|a, b| {
                b.relevance_score
                    .partial_cmp(&a.relevance_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let category = classify_article(&title, "");
            DigestSection {
                title,
                category,
                stories,
            }
        })
        .collect();
    sections.sort_by(|a, b| b.stories.len().cmp(&a.stories.len()));

    let total_stories: usize = sections.iter().map(|s| s.stories.len()).sum();
    let num_categories = sections.len();
    let top_stories: Vec<DigestStory> = sections
        .iter()
        .flat_map(|s| s.stories.iter().cloned())
        .take(5)
        .collect();

    RegionalDigest {
        region: region.code.clone(),
        region_name: region.name.clone(),
        period_start,
        period_end,
        generated_at: Utc::now(),
        sections,
        top_stories,
        statistics: DigestStats {
            total_articles_scanned: articles.len(),
            relevant_articles: total_stories,
            entities_mentioned: 0,
            categories_covered: num_categories,
        },
    }
}

fn extract_domain(url: &str) -> String {
    url.split("//")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .unwrap_or("unknown")
        .to_string()
}

/// LLM prompt template for digest summarization.
pub fn digest_summary_prompt(region: &str, stories_json: &str) -> String {
    format!(
        "Summarize the following news articles for the {} region into a \
         concise weekly intelligence digest for an EMS/electronics \
         manufacturing competitive intelligence team. Focus on: \
         competitor moves, regulatory changes, supply chain risks, \
         trade policy impacts, and personnel changes. \
         Format as bullet points grouped by category.\n\n{}",
        region, stories_json
    )
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::disallowed_methods,
        clippy::field_reassign_with_default,
        clippy::manual_range_contains,
        clippy::needless_borrows_for_generic_args,
        clippy::cloned_ref_to_slice_refs
    )]

    use super::*;

    #[test]
    fn test_score_relevance() {
        let keywords = vec!["electronics".into(), "manufacturing".into(), "PCB".into()];
        let score = score_relevance(
            "New electronics factory opens",
            "PCB manufacturing line",
            &keywords,
        );
        assert!(score > 0.5);
    }

    #[test]
    fn test_score_zero_for_irrelevant() {
        let keywords = vec!["electronics".into(), "PCB".into()];
        let score = score_relevance("Football match results", "Local team wins", &keywords);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_classify_competitor() {
        assert_eq!(
            classify_article("Rival company acquired new factory", ""),
            NewsCategory::CompetitorActivity
        );
    }

    #[test]
    fn test_classify_supply_chain() {
        assert_eq!(
            classify_article("Global chip shortage worsens", "supply chain disruption"),
            NewsCategory::SupplyChain
        );
    }

    #[test]
    fn test_classify_regulation() {
        assert_eq!(
            classify_article("New EU regulation on electronics", "compliance required"),
            NewsCategory::Regulation
        );
    }

    #[test]
    fn test_classify_trade() {
        assert_eq!(
            classify_article("New tariff on Chinese imports", "trade war"),
            NewsCategory::TradePolicy
        );
    }

    #[test]
    fn test_default_regions_complete() {
        let regions = default_regions();
        assert_eq!(regions.len(), 6);
        assert!(regions.iter().any(|r| r.code == "TN"));
        assert!(regions.iter().any(|r| r.code == "MA"));
        assert!(regions.iter().any(|r| r.code == "IL"));
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://www.reuters.com/article/foo"),
            "www.reuters.com"
        );
    }

    #[test]
    fn test_build_empty_digest() {
        let regions = default_regions();
        let digest = build_digest(
            &regions[0],
            &[],
            NaiveDate::from_ymd_opt(2026, 2, 22).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        );
        assert_eq!(digest.region, "TN");
        assert_eq!(digest.statistics.relevant_articles, 0);
    }
}
