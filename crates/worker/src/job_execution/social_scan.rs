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

/// Run the social media scan.
pub(super) async fn run_social_scan(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let start = Instant::now();

    let mut total_posts: u64 = 0;
    let mut total_linked: u64 = 0;

    // Load tracked company names for entity linking.
    let company_names = load_company_names(store).await;
    if company_names.is_empty() {
        run.skip("social_scan: no companies to match against");
        return run;
    }

    // ── 1. Reddit ──────────────────────────────────────────────────────────
    let reddit_posts = ingest_reddit(&company_names).await;
    for post in &reddit_posts {
        let entity_id = link_post_to_entity(&post.text, &company_names, store).await;
        if let Err(e) = store_social_observation(store, post, "reddit", entity_id).await {
            tracing::warn!(error = %e, "social_scan: failed to store Reddit post");
        } else {
            total_posts += 1;
            if entity_id.is_some() {
                total_linked += 1;
            }
        }
    }
    tracing::info!(count = reddit_posts.len(), "social_scan: Reddit ingested");

    // ── 2. Telegram ────────────────────────────────────────────────────────
    let telegram_posts = ingest_telegram(&company_names).await;
    for post in &telegram_posts {
        let entity_id = link_post_to_entity(&post.text, &company_names, store).await;
        if let Err(e) = store_social_observation(store, post, "telegram", entity_id).await {
            tracing::warn!(error = %e, "social_scan: failed to store Telegram post");
        } else {
            total_posts += 1;
            if entity_id.is_some() {
                total_linked += 1;
            }
        }
    }
    tracing::info!(
        count = telegram_posts.len(),
        "social_scan: Telegram ingested"
    );

    // ── 3. Hacker News (via Algolia API — free, no key) ────────────────────
    let hn_posts = ingest_hackernews(&company_names).await;
    for post in &hn_posts {
        let entity_id = link_post_to_entity(&post.text, &company_names, store).await;
        if let Err(e) = store_social_observation(store, post, "hackernews", entity_id).await {
            tracing::warn!(error = %e, "social_scan: failed to store HN post");
        } else {
            total_posts += 1;
            if entity_id.is_some() {
                total_linked += 1;
            }
        }
    }
    tracing::info!(count = hn_posts.len(), "social_scan: Hacker News ingested");

    // ── 4. Twitter via Nitter (no bearer token needed) ─────────────────────
    let twitter_posts = ingest_twitter_nitter(&company_names).await;
    for post in &twitter_posts {
        let entity_id = link_post_to_entity(&post.text, &company_names, store).await;
        if let Err(e) = store_social_observation(store, post, "twitter", entity_id).await {
            tracing::warn!(error = %e, "social_scan: failed to store Twitter post");
        } else {
            total_posts += 1;
            if entity_id.is_some() {
                total_linked += 1;
            }
        }
    }
    tracing::info!(
        count = twitter_posts.len(),
        "social_scan: Twitter/Nitter ingested"
    );

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
    run.succeed(
        total_posts,
        &format!(
            "social_scan: {} posts ingested ({} linked to entities) from Reddit+Telegram+HN+Twitter in {:.1}s",
            total_posts,
            total_linked,
            elapsed.as_secs_f64(),
        ),
    );
    run
}

/// A lightweight post struct (avoids depending on the full SocialPost type
/// which requires the full social module to compile).
struct IngestedPost {
    platform: String,
    text: String,
    author: String,
    url: String,
    published_at: chrono::DateTime<Utc>,
    engagement: u64,
}

/// Ingest posts from monitored subreddits using RSS feeds (Reddit blocks
/// the JSON API server-side with 403, but RSS feeds work without auth).
async fn ingest_reddit(_company_names: &[(uuid::Uuid, String)]) -> Vec<IngestedPost> {
    let client = reqwest::Client::builder()
        .user_agent("ApexIntel-Social/1.0 (research)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let mut posts = Vec::new();

    for subreddit in MONITORED_SUBREDDITS {
        // Reddit RSS feed endpoint — works without OAuth
        let url = format!("https://www.reddit.com/r/{subreddit}/.rss?limit=5");
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(xml) = resp.text().await {
                    // Parse RSS XML — extract <item> entries
                    for item_chunk in xml.split("<entry>").skip(1).take(5) {
                        let title = extract_xml_tag(item_chunk, "title").unwrap_or_default();
                        let content = extract_xml_tag(item_chunk, "content").unwrap_or_default();
                        let author = extract_xml_tag(item_chunk, "name")
                            .unwrap_or_else(|| "unknown".to_string());
                        let link = extract_xml_tag(item_chunk, "id").unwrap_or_default();
                        let published = extract_xml_tag(item_chunk, "published")
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(Utc::now);

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

                        posts.push(IngestedPost {
                            platform: "reddit".to_string(),
                            text: text.chars().take(4000).collect(),
                            author,
                            url: link,
                            published_at: published,
                            engagement: 0,
                        });
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(
                    subreddit,
                    status = resp.status().as_u16(),
                    "social_scan: Reddit RSS fetch failed"
                );
            }
            Err(e) => {
                tracing::warn!(subreddit, error = %e, "social_scan: Reddit network error");
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    posts.truncate(MAX_POSTS_PER_PLATFORM);
    posts
}

/// Ingest from public Telegram channels via t.me/s/{channel}.
async fn ingest_telegram(_company_names: &[(uuid::Uuid, String)]) -> Vec<IngestedPost> {
    let client = reqwest::Client::builder()
        .user_agent("ApexIntel-Social/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let mut posts = Vec::new();

    for channel in MONITORED_TELEGRAM_CHANNELS {
        let url = format!("https://t.me/s/{channel}");
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(html) = resp.text().await {
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

                                posts.push(IngestedPost {
                                    platform: "telegram".to_string(),
                                    text: clean_text.chars().take(4000).collect(),
                                    author: channel.to_string(),
                                    url: url.clone(),
                                    published_at: Utc::now(),
                                    engagement: 0,
                                });
                            }
                        }
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(
                    channel,
                    status = resp.status().as_u16(),
                    "social_scan: Telegram fetch failed"
                );
            }
            Err(e) => {
                tracing::warn!(channel, error = %e, "social_scan: Telegram network error");
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    }

    posts.truncate(MAX_POSTS_PER_PLATFORM);
    posts
}

/// Ingest from Hacker News via the free Algolia search API.
async fn ingest_hackernews(company_names: &[(uuid::Uuid, String)]) -> Vec<IngestedPost> {
    let client = reqwest::Client::builder()
        .user_agent("ApexIntel-Social/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();

    let mut posts = Vec::new();

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
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(hits) = json.get("hits").and_then(|h| h.as_array()) {
                        for hit in hits.iter().take(3) {
                            let title = hit.get("title").and_then(|v| v.as_str()).unwrap_or("");
                            let url = hit.get("url").and_then(|v| v.as_str()).unwrap_or("");
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

                            posts.push(IngestedPost {
                                platform: "hackernews".to_string(),
                                text: title.chars().take(4000).collect(),
                                author: author.to_string(),
                                url: if url.is_empty() {
                                    format!("https://news.ycombinator.com/item?id={object_id}")
                                } else {
                                    url.to_string()
                                },
                                published_at: chrono::DateTime::from_timestamp(created, 0)
                                    .unwrap_or_else(Utc::now),
                                engagement: points,
                            });
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(query = %query, error = %e, "social_scan: HN fetch failed");
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    posts.truncate(MAX_POSTS_PER_PLATFORM);
    posts
}

/// Ingest from Twitter/X via Nitter instances (no bearer token needed).
async fn ingest_twitter_nitter(company_names: &[(uuid::Uuid, String)]) -> Vec<IngestedPost> {
    let client = reqwest::Client::builder()
        .user_agent("ApexIntel-Social/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let nitter_instances = [
        "nitter.privacydev.net",
        "nitter.poast.org",
        "nitter.woodland.cafe",
    ];

    let mut posts = Vec::new();

    // Search for top company names
    for (_, name) in company_names.iter().take(5) {
        let query = simple_url_encode(name);
        for instance in &nitter_instances {
            let url = format!("https://{instance}/search?f=tweets&q={query}");
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(html) = resp.text().await {
                        // Parse Nitter's HTML for tweet text
                        for chunk in html.split("tweet-content").skip(1).take(3) {
                            if let Some(text_start) = chunk.find('>') {
                                let rest = &chunk[text_start + 1..];
                                if let Some(text_end) = rest.find("</div>") {
                                    let clean_text = strip_html_tags(&rest[..text_end]);
                                    if clean_text.trim().len() < 20 {
                                        continue;
                                    }
                                    posts.push(IngestedPost {
                                        platform: "twitter".to_string(),
                                        text: clean_text.chars().take(4000).collect(),
                                        author: name.clone(),
                                        url: url.clone(),
                                        published_at: Utc::now(),
                                        engagement: 0,
                                    });
                                }
                            }
                        }
                        break; // Got results from this instance, don't try others
                    }
                }
                _ => continue, // Try next instance
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    posts.truncate(MAX_POSTS_PER_PLATFORM);
    posts
}

/// Store a social post as a SocialPost observation.
async fn store_social_observation(
    store: &PgStore,
    post: &IngestedPost,
    source: &str,
    entity_id: Option<uuid::Uuid>,
) -> Result<(), sqlx::Error> {
    let obs = apex_core::entities::Observation::new(
        apex_core::entities::ObservationType::SocialPost,
        post.published_at,
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
        }),
    );
    let mut obs = obs;
    obs.entity_id = entity_id;
    obs.entity_type = Some("company".to_string());
    obs.confidence = if entity_id.is_some() { 0.8 } else { 0.4 };
    // B326: stable ID per (platform, url, content) — posts that remain in a
    // feed across scans were previously re-inserted every 2h.
    obs.stabilize_id("social");
    store
        .insert_observation(&obs)
        .await
        .map_err(|e| sqlx::Error::Protocol(format!("{e}")))
}

/// Link a post to a tracked company by name matching.
async fn link_post_to_entity(
    text: &str,
    company_names: &[(uuid::Uuid, String)],
    _store: &PgStore,
) -> Option<uuid::Uuid> {
    let lower = text.to_lowercase();
    for (id, name) in company_names {
        if lower.contains(&name.to_lowercase()) {
            return Some(*id);
        }
    }
    None
}

/// Load tracked company names for entity linking.
async fn load_company_names(store: &PgStore) -> Vec<(uuid::Uuid, String)> {
    sqlx::query_as::<_, (uuid::Uuid, String)>(
        "SELECT id, name FROM companies WHERE name IS NOT NULL AND TRIM(name) != '' ORDER BY name",
    )
    .fetch_all(&store.pool)
    .await
    .unwrap_or_default()
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
