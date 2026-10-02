//! High-velocity social media ingestion job.
//!
//! This job wires the existing social scrapers (Reddit, Telegram, Twitter/Nitter,
//! Hacker News) into the observation pipeline — something that was completely
//! missing. Before this, SocialPost observations were never produced by any job,
//! and the 3,200 "SocialPost" rows in production came from adversarial analysis
//! source-quarantine records, not real social media.
//!
//! # Sources ingested (all free, no API keys required)
//! - **Reddit**: 40+ OSINT-relevant subreddits via the free JSON API
//! - **Telegram**: public channels via t.me/s/{channel} HTML scraping
//! - **Twitter/X**: Nitter fallback (no bearer token needed)
//! - **Hacker News**: Algolia search API for company mentions
//!
//! Each post is stored as a `SocialPost` observation with full provenance,
//! then linked to tracked companies via entity name matching.
//!
//! Runs every 2 hours — high velocity, not daily like most other jobs.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;

use crate::{JobKind, JobRun, PgStore};
use apex_crawl::acquisition::{AcquisitionOutcome, AcquisitionRunCounters, AcquisitionRunDecision};

use super::nightly::{match_entities_in_text, EntityMatcher};

/// Subreddits most relevant to EMS/semiconductor/supply-chain intelligence.
const MONITORED_SUBREDDITS: &[&str] = &[
    "worldnews",
    "geopolitics",
    "supplychain",
    "semiconductors",
    "electronics",
    "cybersecurity",
    "osint",
    "business",
    "economics",
    "technology",
    "defense",
    "energy",
];

/// Telegram channels for open-source intelligence (public, no auth needed).
const MONITORED_TELEGRAM_CHANNELS: &[&str] = &["IntelSlavaZ", "ryaborig", "livemap", "nexaborig"];

/// Search queries for Hacker News (company names are added dynamically).
const HN_SEARCH_TERMS: &[&str] = &[
    "semiconductor shortage",
    "supply chain disruption",
    "EMS manufacturing",
    "PCBA procurement",
];

/// Maximum posts to store per platform per run.
const MAX_POSTS_PER_PLATFORM: usize = 50;

/// Hard cap on a fetched platform response body. The guarded HTTP client is
/// shared with the crawler, and its body cap is only enforced when callers
/// read through `apex_crawl::http::read_capped`; `.text()` would buffer an
/// unbounded hostile body.
const MAX_FETCH_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Run the social media scan.
pub(super) async fn run_social_scan(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let start = Instant::now();

    let mut total_posts: u64 = 0;
    let mut total_linked: u64 = 0;

    // Load tracked company names and build the same word-boundary matcher the
    // crawl cycle uses. A failed matcher build must fail the job, never fall
    // back to unlinked (or substring-linked) posts.
    let company_names = match load_company_names(store).await {
        Ok(names) => names,
        Err(error) => {
            run.fail(&format!(
                "social_scan: failed to load company names: {error}"
            ));
            return run;
        }
    };
    if company_names.is_empty() {
        run.skip("social_scan: no companies to match against");
        return run;
    }
    let matcher = match EntityMatcher::from_index(&company_names) {
        Ok(matcher) => matcher,
        Err(error) => {
            run.fail(&format!(
                "social_scan: failed to build entity matcher: {error}"
            ));
            return run;
        }
    };

    let mut counters = AcquisitionRunCounters::default();

    // Each platform reports an explicit outcome; posts are stored only after
    // the fetch succeeded, and every persistence failure is recorded.
    let platforms: [(&str, AcquisitionOutcome<IngestedPost>); 4] = [
        ("reddit", ingest_reddit(&company_names).await),
        ("telegram", ingest_telegram(&company_names).await),
        ("hackernews", ingest_hackernews(&company_names).await),
        ("twitter", ingest_twitter_nitter(&company_names).await),
    ];

    for (source, outcome) in platforms {
        counters.record(&outcome);
        let posts = outcome.into_items();
        for post in &posts {
            // One observation per matched entity (the Observation shape
            // carries a single entity id), or one unlinked observation when
            // nothing matched — the same contract as the crawl cycle.
            for entity_id in post_entity_targets(&post.text, &matcher) {
                if let Err(e) = store_social_observation(store, post, source, entity_id).await {
                    counters.record_persistence_failure();
                    tracing::warn!(error = %e, source, "social_scan: failed to store post");
                } else {
                    total_posts += 1;
                    if entity_id.is_some() {
                        total_linked += 1;
                    }
                }
            }
        }
        tracing::info!(
            count = posts.len(),
            source,
            "social_scan: platform ingested"
        );
    }

    // Log activity
    let activity_logger = apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
    activity_logger
        .log_crawl_completed(
            "social_scan",
            (MONITORED_SUBREDDITS.len() + MONITORED_TELEGRAM_CHANNELS.len() + HN_SEARCH_TERMS.len())
                as u32,
            total_posts as u32,
            start.elapsed().as_secs_f64(),
        )
        .await;

    let elapsed = start.elapsed();
    let notes = format!(
        "social_scan: {} observations stored ({} entity-linked) from Reddit+Telegram+HN+Twitter in {:.1}s; {}",
        total_posts,
        total_linked,
        elapsed.as_secs_f64(),
        counters.summary(),
    );
    match counters.decision() {
        AcquisitionRunDecision::Succeeded { .. } => run.succeed(total_posts, &notes),
        AcquisitionRunDecision::Degraded { reason } => {
            run.degrade(total_posts, &format!("{notes}; {reason}"))
        }
        AcquisitionRunDecision::Failed { reason } => {
            run.fail(&format!("social_scan: {reason} ({notes})"))
        }
        AcquisitionRunDecision::Skipped => run.skip(&notes),
    }
    run
}

/// A lightweight post struct (avoids depending on the full SocialPost type
/// which requires the full social module to compile).
struct IngestedPost {
    platform: String,
    text: String,
    author: String,
    url: String,
    /// Publication time as stated by the source. `None` when the source does
    /// not state one — never replaced with "now" (the observation records the
    /// actual basis in its provenance).
    published_at: Option<chrono::DateTime<Utc>>,
    engagement: u64,
}

/// Per-platform outcome accumulator: any successful fetch makes the platform
/// a success; otherwise the first failure is reported.
#[derive(Default)]
struct PlatformAccumulator {
    posts: Vec<IngestedPost>,
    any_success: bool,
    first_failure: Option<AcquisitionOutcome<IngestedPost>>,
}

impl PlatformAccumulator {
    fn record_fetch(
        &mut self,
        result: Result<String, AcquisitionOutcome<IngestedPost>>,
    ) -> Option<String> {
        match result {
            Ok(body) => {
                self.any_success = true;
                Some(body)
            }
            Err(failure) => {
                if self.first_failure.is_none() {
                    self.first_failure = Some(failure);
                }
                None
            }
        }
    }

    fn push(&mut self, post: IngestedPost) {
        self.posts.push(post);
    }

    fn finish(mut self) -> AcquisitionOutcome<IngestedPost> {
        if !self.any_success {
            return self
                .first_failure
                .unwrap_or(AcquisitionOutcome::NotApplicable);
        }
        self.posts.truncate(MAX_POSTS_PER_PLATFORM);
        AcquisitionOutcome::success_now(self.posts)
    }
}

/// Build a social HTTP client; a construction failure is an unavailable
/// platform, never a silent different client configuration.
fn build_social_client(user_agent: &str, timeout_secs: u64) -> Result<reqwest::Client, String> {
    apex_crawl::http::external_client_with(apex_crawl::http::ExternalClientOptions {
        timeout: std::time::Duration::from_secs(timeout_secs),
        user_agent: Some(user_agent.to_string()),
        ..apex_crawl::http::ExternalClientOptions::default()
    })
    .map_err(|error| format!("failed to build HTTP client: {error}"))
}

/// Fetch a URL as text, mapping every failure onto an explicit outcome.
async fn fetch_text(
    client: &reqwest::Client,
    url: &str,
    context: &str,
) -> Result<String, AcquisitionOutcome<IngestedPost>> {
    match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let status = resp.status().as_u16();
            apex_crawl::http::read_capped(resp, MAX_FETCH_BODY_BYTES)
                .await
                .map_err(|error| {
                    AcquisitionOutcome::fetch_failed(
                        format!("{context} body read failed: {error}"),
                        Some(status),
                    )
                })
        }
        Ok(resp) => {
            let status = resp.status().as_u16();
            Err(AcquisitionOutcome::fetch_failed(
                format!("{context} returned HTTP {status}"),
                Some(status),
            ))
        }
        Err(error) => Err(AcquisitionOutcome::fetch_failed(
            format!("{context} network error: {error}"),
            None,
        )),
    }
}

/// Ingest posts from monitored subreddits using RSS feeds (Reddit blocks
/// the JSON API server-side with 403, but RSS feeds work without auth).
async fn ingest_reddit(
    _company_names: &[(uuid::Uuid, String)],
) -> AcquisitionOutcome<IngestedPost> {
    let client = match build_social_client("ApexIntel-Social/1.0 (research)", 15) {
        Ok(client) => client,
        Err(reason) => return AcquisitionOutcome::Unavailable { reason },
    };

    let mut acc = PlatformAccumulator::default();

    for subreddit in MONITORED_SUBREDDITS {
        // Reddit RSS feed endpoint — works without OAuth
        let url = format!("https://www.reddit.com/r/{subreddit}/.rss?limit=5");
        let context = format!("reddit r/{subreddit}");
        if let Some(xml) = acc.record_fetch(fetch_text(&client, &url, &context).await) {
            // Parse RSS XML — extract <item> entries
            for item_chunk in xml.split("<entry>").skip(1).take(5) {
                let title = extract_xml_tag(item_chunk, "title").unwrap_or_default();
                let content = extract_xml_tag(item_chunk, "content").unwrap_or_default();
                let author =
                    extract_xml_tag(item_chunk, "name").unwrap_or_else(|| "unknown".to_string());
                let link = extract_xml_tag(item_chunk, "id").unwrap_or_default();
                let published = extract_xml_tag(item_chunk, "published")
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                if title.is_empty() {
                    continue;
                }

                let clean_content = strip_html_tags(&content);
                let text = if clean_content.trim().is_empty() {
                    title.clone()
                } else {
                    format!(
                        "{title}\n\n{}",
                        clean_content.chars().take(2000).collect::<String>()
                    )
                };

                acc.push(IngestedPost {
                    platform: "reddit".to_string(),
                    text: text.chars().take(4000).collect(),
                    author,
                    url: link,
                    published_at: published,
                    engagement: 0,
                });
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    acc.finish()
}

/// Ingest from public Telegram channels via t.me/s/{channel}.
async fn ingest_telegram(
    _company_names: &[(uuid::Uuid, String)],
) -> AcquisitionOutcome<IngestedPost> {
    let client = match build_social_client("ApexIntel-Social/1.0", 15) {
        Ok(client) => client,
        Err(reason) => return AcquisitionOutcome::Unavailable { reason },
    };

    let mut acc = PlatformAccumulator::default();

    for channel in MONITORED_TELEGRAM_CHANNELS {
        let url = format!("https://t.me/s/{channel}");
        let context = format!("telegram {channel}");
        if let Some(html) = acc.record_fetch(fetch_text(&client, &url, &context).await) {
            // Parse the HTML for message text using simple string matching
            // (the Telegram public preview page has predictable structure).
            for chunk in html.split("tgme_widget_message_text").skip(1) {
                if let Some(text_start) = chunk.find('>') {
                    let rest = &chunk[text_start + 1..];
                    if let Some(text_end) = rest.find("</div>") {
                        let raw_text = &rest[..text_end];
                        // Strip HTML tags
                        let clean_text = strip_html_tags(raw_text);
                        if clean_text.trim().len() < 20 {
                            continue;
                        }

                        acc.push(IngestedPost {
                            platform: "telegram".to_string(),
                            text: clean_text.chars().take(4000).collect(),
                            author: channel.to_string(),
                            url: url.clone(),
                            // The public preview does not state a publication
                            // time; recording "now" would fabricate freshness.
                            published_at: None,
                            engagement: 0,
                        });
                    }
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }

    acc.finish()
}

/// Ingest from Hacker News via the free Algolia search API.
async fn ingest_hackernews(
    company_names: &[(uuid::Uuid, String)],
) -> AcquisitionOutcome<IngestedPost> {
    let client = match build_social_client("ApexIntel-Social/1.0", 15) {
        Ok(client) => client,
        Err(reason) => return AcquisitionOutcome::Unavailable { reason },
    };

    let mut acc = PlatformAccumulator::default();

    // Search for company names + general terms
    let mut queries: Vec<String> = HN_SEARCH_TERMS.iter().map(|s| s.to_string()).collect();
    // Add top company names as search queries
    for (_, name) in company_names.iter().take(10) {
        queries.push(name.clone());
    }

    for query in &queries {
        let url = format!(
            "https://hn.algolia.com/api/v1/search?query={}&tags=story&hitsPerPage=5",
            simple_url_encode(query)
        );
        let context = format!("hackernews query {query}");
        if let Some(json_body) = acc.record_fetch(fetch_text(&client, &url, &context).await) {
            match serde_json::from_str::<serde_json::Value>(&json_body) {
                Ok(json) => {
                    if let Some(hits) = json.get("hits").and_then(|h| h.as_array()) {
                        for hit in hits.iter().take(3) {
                            let title = hit.get("title").and_then(|v| v.as_str()).unwrap_or("");
                            let hit_url = hit.get("url").and_then(|v| v.as_str()).unwrap_or("");
                            let author = hit
                                .get("author")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            let points = hit.get("points").and_then(|v| v.as_u64()).unwrap_or(0);
                            let object_id =
                                hit.get("objectID").and_then(|v| v.as_str()).unwrap_or("");
                            let created = hit
                                .get("created_at_i")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);

                            if title.is_empty() {
                                continue;
                            }

                            acc.push(IngestedPost {
                                platform: "hackernews".to_string(),
                                text: title.chars().take(4000).collect(),
                                author: author.to_string(),
                                url: if hit_url.is_empty() {
                                    format!("https://news.ycombinator.com/item?id={object_id}")
                                } else {
                                    hit_url.to_string()
                                },
                                published_at: chrono::DateTime::from_timestamp(created, 0),
                                engagement: points,
                            });
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(query = %query, %error, "social_scan: HN JSON parse failed");
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    acc.finish()
}

/// Ingest from Twitter/X via Nitter instances (no bearer token needed).
async fn ingest_twitter_nitter(
    company_names: &[(uuid::Uuid, String)],
) -> AcquisitionOutcome<IngestedPost> {
    let client = match build_social_client("ApexIntel-Social/1.0", 10) {
        Ok(client) => client,
        Err(reason) => return AcquisitionOutcome::Unavailable { reason },
    };

    let nitter_instances = [
        "nitter.privacydev.net",
        "nitter.poast.org",
        "nitter.woodland.cafe",
    ];

    let mut acc = PlatformAccumulator::default();

    // Search for top company names
    for (_, name) in company_names.iter().take(5) {
        let query = simple_url_encode(name);
        for instance in &nitter_instances {
            let url = format!("https://{instance}/search?f=tweets&q={query}");
            let context = format!("nitter {instance} query {name}");
            if let Some(html) = acc.record_fetch(fetch_text(&client, &url, &context).await) {
                // Parse Nitter's HTML for tweet text
                for chunk in html.split("tweet-content").skip(1).take(3) {
                    if let Some(text_start) = chunk.find('>') {
                        let rest = &chunk[text_start + 1..];
                        if let Some(text_end) = rest.find("</div>") {
                            let clean_text = strip_html_tags(&rest[..text_end]);
                            if clean_text.trim().len() < 20 {
                                continue;
                            }
                            acc.push(IngestedPost {
                                platform: "twitter".to_string(),
                                text: clean_text.chars().take(4000).collect(),
                                author: name.clone(),
                                url: url.clone(),
                                // Nitter search HTML does not state a
                                // reliable publication time here.
                                published_at: None,
                                engagement: 0,
                            });
                        }
                    }
                }
                break; // Got results from this instance, don't try others
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    acc.finish()
}

/// Store a social post as a SocialPost observation.
async fn store_social_observation(
    store: &PgStore,
    post: &IngestedPost,
    source: &str,
    entity_id: Option<uuid::Uuid>,
) -> Result<(), sqlx::Error> {
    let (ts_utc, timestamp_basis) = match post.published_at {
        Some(published_at) => (published_at, "published_at"),
        None => (Utc::now(), "ingested_at"),
    };
    let obs = apex_core::entities::Observation::new(
        apex_core::entities::ObservationType::SocialPost,
        ts_utc,
        serde_json::json!({
            "platform": post.platform,
            "content": &post.text,
            "body_excerpt": &post.text,
            "author": &post.author,
            "url": &post.url,
            "engagement": post.engagement,
            "source": source,
        }),
        serde_json::json!({
            "source": source,
            "source_id": format!("{}_{}", post.platform, post.url),
            "source_domain": &post.platform,
            "url": &post.url,
            "timestamp_basis": timestamp_basis,
        }),
    );
    let mut obs = obs;
    obs.entity_id = entity_id;
    obs.entity_type = Some("company".to_string());
    obs.confidence = if entity_id.is_some() { 0.8 } else { 0.4 };
    // B326: stable ID per (platform, url, content) — posts that remain in a
    // feed across scans were previously re-inserted every 2h. Entity-linked
    // observations are scoped by entity so every matched company gets its own
    // row instead of colliding with the first one.
    //
    // Id-migration caveat: rows already stored under the legacy
    // entity-unscoped id can re-insert once after this switch, because
    // `observations` has no natural unique key besides `id` and the old row
    // cannot be addressed by the new entity-scoped id. The duplicate is
    // accepted as a one-time cost (the alternative is a data migration that
    // rewrites ids); every scan after that is idempotent.
    let stabilize_key = obs.stabilize_id("social");
    if let Some(entity_id) = entity_id {
        obs.id = apex_core::entities::Observation::deterministic_id(
            "social",
            &format!("{stabilize_key}|entity:{entity_id}"),
        );
    }
    store
        .insert_observation(&obs)
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("{e}")))
}

/// Entity targets for one post: every matched entity id, or a single `None`
/// when nothing matched (the post is still stored, just unlinked).
fn post_entity_targets(text: &str, matcher: &EntityMatcher) -> Vec<Option<uuid::Uuid>> {
    let matched = match_entities_in_text(text, "", matcher);
    if matched.is_empty() {
        vec![None]
    } else {
        matched.into_iter().map(Some).collect()
    }
}

/// Load tracked company names for entity linking.
async fn load_company_names(store: &PgStore) -> Result<Vec<(uuid::Uuid, String)>, sqlx::Error> {
    sqlx::query_as::<_, (uuid::Uuid, String)>(
        "SELECT id, name FROM companies WHERE name IS NOT NULL AND TRIM(name) != '' ORDER BY name",
    )
    .fetch_all(&store.pool)
    .await
}

/// Strip HTML tags from a string.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}

/// Simple URL query parameter encoder (avoids needing the urlencoding crate).
fn simple_url_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(ch),
            ' ' => result.push('+'),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    result.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    result
}

/// Extract the text content of an XML tag from an XML fragment.
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let after_open = &xml[start..];
    // Skip to the end of the opening tag (handle attributes)
    let content_start = after_open.find('>')? + 1;
    let content_end = after_open.find(&close)?;
    let raw = &after_open[content_start..content_end];
    Some(strip_html_tags(raw))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn matcher_for(names: &[(uuid::Uuid, &str)]) -> EntityMatcher {
        let index: Vec<(uuid::Uuid, String)> = names
            .iter()
            .map(|(id, name)| (*id, (*name).to_string()))
            .collect();
        EntityMatcher::from_index(&index).expect("test matcher builds")
    }

    #[test]
    fn post_entity_targets_links_every_word_boundary_match() {
        let first = uuid::Uuid::from_u128(1);
        let second = uuid::Uuid::from_u128(2);
        let matcher = matcher_for(&[(first, "Acme Batteries"), (second, "Northwind Traders")]);

        let targets = post_entity_targets(
            "Acme Batteries and Northwind Traders both announced results.",
            &matcher,
        );

        assert_eq!(
            targets.len(),
            2,
            "both entities must be linked: {targets:?}"
        );
        assert!(targets.contains(&Some(first)));
        assert!(targets.contains(&Some(second)));
    }

    #[test]
    fn post_entity_targets_rejects_substrings_and_short_single_mentions() {
        let ion = uuid::Uuid::from_u128(3);
        let matcher = matcher_for(&[(ion, "Ion")]);

        assert_eq!(
            post_entity_targets("the nation reported steady growth", &matcher),
            vec![None],
            "substring matches must not link"
        );
        assert_eq!(
            post_entity_targets("an ion engine was tested", &matcher),
            vec![None],
            "a lone short-name mention needs a title or a second mention"
        );
        assert_eq!(
            post_entity_targets("the ion drive and the ion engine shipped", &matcher),
            vec![Some(ion)],
            "two body mentions corroborate a short name"
        );
    }

    #[test]
    fn post_entity_targets_keeps_unlinked_posts() {
        let matcher = matcher_for(&[(uuid::Uuid::from_u128(4), "Globex Corporation")]);
        assert_eq!(
            post_entity_targets("unrelated content without names", &matcher),
            vec![None]
        );
    }
}
