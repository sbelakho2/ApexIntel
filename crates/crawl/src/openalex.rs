//! OpenAlex academic-publication fetcher.
//!
//! [OpenAlex](https://openalex.org) is a fully open catalogue of the global
//! research system — a free, no-API-key superset of the deprecated Microsoft
//! Academic Graph.  Supplying a `mailto` parameter (even a synthetic one)
//! routes the request into the "polite pool", which has higher rate limits.
//!
//! The fetcher searches `api.openalex.org/works`, reconstructs the inverted
//! abstract index that OpenAlex uses for storage, and projects each work onto
//! a strongly-typed [`OpenAlexWork`].  Network errors degrade gracefully to an
//! empty result with a `tracing::warn!`.

use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use apex_core::entities::{Observation, ObservationType};

/// OpenAlex API root.
const OPENALEX_API: &str = "https://api.openalex.org/works";
/// Synthetic contact for the polite pool — gets us onto the higher-rate-limit
/// tier without requiring a real key or sign-up.
const OPENALEX_MAILTO: &str = "research@apexintel.local";

/// A single academic work projected from the OpenAlex API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpenAlexWork {
    /// DOI (e.g. `"10.1000/xyz"`), if assigned.
    pub doi: Option<String>,
    /// Work title (`display_name`).
    pub title: String,
    /// Publication date string (`YYYY-MM-DD` or a prefix thereof).
    pub publication_date: Option<String>,
    /// Display names of the listed authors, in authorship order.
    pub authors: Vec<String>,
    /// Display names of the top concepts assigned to the work.
    pub concepts: Vec<String>,
    /// Total citation count.
    pub cited_by_count: u64,
    /// Reconstructed abstract text (OpenAlex stores an inverted index).
    pub abstract_text: Option<String>,
    /// Open-access URL, if one is available.
    pub oa_url: Option<String>,
    /// Display name of the primary host venue (`primary_location.source`).
    pub source: Option<String>,
}

impl OpenAlexWork {
    /// Wrap this work into a crawl [`Observation`] of type
    /// [`ObservationType::PersonMention`] — academic publications are
    /// person-mention signals.
    pub fn to_observation(&self, entity_id: Option<Uuid>) -> Observation {
        let mut obs = Observation::new(
            ObservationType::PersonMention,
            Utc::now(),
            serde_json::to_value(self).unwrap_or(json!(null)),
            json!({
                "source": "openalex",
                "url": format!("{OPENALEX_API}?search={}", self.title),
            }),
        );
        obs.entity_id = entity_id;
        obs
    }
}

/// OpenAlex client.  Owns a `reqwest::Client` tuned for the polite pool.
pub struct OpenAlexClient {
    client: Client,
}

impl OpenAlexClient {
    /// Build a client with a 15s timeout and a descriptive User-Agent.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("ApexIntel-OpenAlex/1.0")
            .build()
            .unwrap_or_else(|error| {
                warn!(error = %error, "openalex: failed to build HTTP client; using default");
                Client::new()
            });
        Self { client }
    }

    /// Search OpenAlex works and return at most `limit` results.
    ///
    /// `limit` is clamped to a maximum of 200 (the OpenAlex per-page cap).
    /// Any network or parse error is logged and yields an empty vec.
    pub async fn search_works(&self, query: &str, limit: usize) -> Result<Vec<OpenAlexWork>> {
        let per_page = limit.clamp(1, 200);

        // Filter to only recent publications (last 90 days) so the system
        // generates intelligence from current research, not historical papers.
        // The user explicitly flagged old articles as a problem.
        let since_date = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::days(90))
            .unwrap_or_else(chrono::Utc::now)
            .format("%Y-%m-%d");

        let url = format!(
            "{OPENALEX_API}?search={}&per-page={per_page}&mailto={OPENALEX_MAILTO}&filter=from_publication_date:{since_date}",
            urlencoding::encode(query)
        );

        let response = match self.client.get(&url).send().await {
            Ok(resp) => resp,
            Err(error) => {
                warn!(query = %query, error = %error, "openalex: network error");
                return Ok(Vec::new());
            }
        };

        let status = response.status();
        if status.as_u16() == 429 {
            warn!(query = %query, "openalex: rate limited (429); skipping");
            return Ok(Vec::new());
        }
        if !status.is_success() {
            warn!(query = %query, status = status.as_u16(), "openalex: non-success status");
            return Ok(Vec::new());
        }

        let body: serde_json::Value = match response.json().await {
            Ok(value) => value,
            Err(error) => {
                warn!(query = %query, error = %error, "openalex: failed to parse JSON");
                return Ok(Vec::new());
            }
        };

        let results = body
            .get("results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let works: Vec<OpenAlexWork> = results
            .iter()
            .map(|raw| Self::parse_work(raw))
            .take(limit)
            .collect();

        Ok(works)
    }

    /// Project a single raw OpenAlex work object onto [`OpenAlexWork`].
    fn parse_work(raw: &serde_json::Value) -> OpenAlexWork {
        // DOI: stored as "https://doi.org/10.x/yyy" — strip the prefix.
        let doi = raw
            .get("doi")
            .and_then(|v| v.as_str())
            .map(|s| s.trim_start_matches("https://doi.org/").to_string());

        let title = raw
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let publication_date = raw
            .get("publication_date")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let authors = raw
            .get("authorships")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        a.get("author")
                            .and_then(|author| author.get("display_name"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let concepts = raw
            .get("concepts")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.get("display_name").and_then(|v| v.as_str()).map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let cited_by_count = raw
            .get("cited_by_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let abstract_text = raw
            .get("abstract_inverted_index")
            .and_then(|v| v.as_object())
            .and_then(reconstruct_abstract);

        // primary_location -> best_oa_location.url + source.display_name.
        let primary_location = raw.get("primary_location");
        let source = primary_location
            .and_then(|loc| loc.get("source"))
            .and_then(|s| s.get("display_name"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let oa_url = raw
            .get("best_oa_location")
            .and_then(|loc| loc.get("pdf_url").or_else(|| loc.get("url")))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| {
                primary_location
                    .and_then(|loc| loc.get("pdf_url").or_else(|| loc.get("url")))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            });

        OpenAlexWork {
            doi,
            title,
            publication_date,
            authors,
            concepts,
            cited_by_count,
            abstract_text,
            oa_url,
            source,
        }
    }
}

impl Default for OpenAlexClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Reconstruct an abstract from an OpenAlex inverted index.
///
/// The inverted index maps each word to the list of positions it occupies in
/// the original text.  We invert it back into a positional array and join.
fn reconstruct_abstract(index: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    // First find the maximum position so we can size the slots array.
    let max_pos = index.values().filter_map(|positions| positions.as_array()).flat_map(|positions| {
        positions.iter().filter_map(|p| p.as_u64().map(|n| n as usize))
    }).max()?;

    let mut slots: Vec<Option<String>> = vec![None; max_pos + 1];
    for (word, positions) in index {
        let Some(positions) = positions.as_array() else { continue };
        for pos in positions {
            if let Some(idx) = pos.as_u64() {
                let idx = idx as usize;
                if idx < slots.len() {
                    slots[idx] = Some(word.clone());
                }
            }
        }
    }

    let reconstructed: Vec<&str> = slots.iter().filter_map(|s| s.as_deref()).collect();
    if reconstructed.is_empty() {
        None
    } else {
        Some(reconstructed.join(" "))
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed but structurally faithful OpenAlex `/works` response with
    /// one result, including authorships, concepts, an inverted-index
    /// abstract, and a primary_location source.
    const SAMPLE_OPENALEX: &str = r#"{
      "meta": {"count": 1},
      "results": [
        {
          "doi": "https://doi.org/10.1000/demo",
          "display_name": "A Demo Paper on Supply-Chain Resilience",
          "publication_date": "2024-03-15",
          "cited_by_count": 42,
          "authorships": [
            {"author": {"display_name": "Alice Author"}},
            {"author": {"display_name": "Bob Builder"}}
          ],
          "concepts": [
            {"display_name": "Supply chain"},
            {"display_name": "Resilience"}
          ],
          "abstract_inverted_index": {
            "Supply": [0],
            "chain": [1],
            "resilience": [2],
            "matters": [3]
          },
          "primary_location": {
            "source": {"display_name": "Journal of Demo Studies"},
            "pdf_url": "https://example.org/demo.pdf",
            "url": "https://example.org/demo"
          },
          "best_oa_location": {
            "pdf_url": "https://example.org/demo.pdf"
          }
        }
      ]
    }"#;

    #[test]
    fn parses_sample_openalex_work() {
        let value: serde_json::Value = serde_json::from_str(SAMPLE_OPENALEX).unwrap();
        let results = value.get("results").and_then(|v| v.as_array()).unwrap();
        let work = OpenAlexClient::parse_work(&results[0]);

        assert_eq!(work.doi.as_deref(), Some("10.1000/demo"));
        assert_eq!(work.title, "A Demo Paper on Supply-Chain Resilience");
        assert_eq!(work.publication_date.as_deref(), Some("2024-03-15"));
        assert_eq!(work.authors, vec!["Alice Author", "Bob Builder"]);
        assert_eq!(work.concepts, vec!["Supply chain", "Resilience"]);
        assert_eq!(work.cited_by_count, 42);
        assert_eq!(work.abstract_text.as_deref(), Some("Supply chain resilience matters"));
        assert_eq!(work.source.as_deref(), Some("Journal of Demo Studies"));
        assert_eq!(work.oa_url.as_deref(), Some("https://example.org/demo.pdf"));
    }

    #[test]
    fn reconstruct_abstract_handles_gaps_and_order() {
        let mut index = serde_json::Map::new();
        index.insert("world".to_string(), json!([1]));
        index.insert("Hello".to_string(), json!([0]));
        // Position 2 missing -> should be skipped, not rendered as "null".
        index.insert("!".to_string(), json!([3]));
        let text = reconstruct_abstract(&index).unwrap();
        assert_eq!(text, "Hello world !");
    }

    #[test]
    fn parse_work_is_defensive_about_missing_fields() {
        let raw: serde_json::Value = serde_json::json!({"display_name": "Title Only"});
        let work = OpenAlexClient::parse_work(&raw);
        assert_eq!(work.title, "Title Only");
        assert!(work.doi.is_none());
        assert!(work.authors.is_empty());
        assert_eq!(work.cited_by_count, 0);
        assert!(work.abstract_text.is_none());
    }

    #[test]
    fn to_observation_is_person_mention() {
        let work = OpenAlexWork {
            doi: Some("10.1000/demo".to_string()),
            title: "Demo".to_string(),
            publication_date: None,
            authors: vec!["Alice".to_string()],
            concepts: vec![],
            cited_by_count: 0,
            abstract_text: None,
            oa_url: None,
            source: None,
        };
        let entity_id = Uuid::new_v4();
        let obs = work.to_observation(Some(entity_id));
        assert_eq!(obs.observation_type, ObservationType::PersonMention);
        assert_eq!(obs.entity_id, Some(entity_id));
        assert_eq!(
            obs.provenance.get("source").and_then(|v| v.as_str()),
            Some("openalex")
        );
    }
}
