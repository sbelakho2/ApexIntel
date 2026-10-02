//! Telegram open-channel archiver.
//!
//! Scrapes public Telegram channel posts via `t.me/s/{channel}` — the
//! web-preview endpoint that does not require authentication.
//!
//! # Key features
//! - Parses post text, timestamp, view count, and forward attribution
//! - Supports pagination through older posts via `?before={id}`
//! - Extracts embedded URLs and document names

use super::SocialPost;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use std::time::Duration;
use tracing::debug;

const TELEGRAM_WEB: &str = "https://t.me/s";
const MAX_PAGES: usize = 5;

/// Telegram channel scraper.
pub struct TelegramScraper {
    client: Client,
    base_url: String,
}

impl TelegramScraper {
    pub fn new(proxy_url: Option<&str>) -> Result<Self> {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(30),
            user_agent: Some("Mozilla/5.0 (compatible; ApexIntelOSINT/1.0)".to_string()),
            proxy: proxy_url
                .map(reqwest::Proxy::all)
                .transpose()
                .context("Invalid proxy")?,
            ..crate::http::ExternalClientOptions::default()
        })?;

        Ok(Self {
            client,
            base_url: TELEGRAM_WEB.to_string(),
        })
    }

    /// Fetch recent posts from a public Telegram channel.
    ///
    /// * `channel` — channel username without `@`, e.g. `"nexta_tv"`.
    /// * `max_posts` — maximum number of posts to return.
    pub async fn channel_posts(&self, channel: &str, max_posts: usize) -> Result<Vec<SocialPost>> {
        let mut all_posts = Vec::new();
        let mut before_id: Option<u64> = None;

        let encoded_channel = urlencoding::encode(channel);
        for page in 0..MAX_PAGES {
            let url = if let Some(id) = before_id {
                format!("{}/{}?before={}", self.base_url, encoded_channel, id)
            } else {
                format!("{}/{}", self.base_url, encoded_channel)
            };

            debug!(url=%url, page, "Fetching Telegram channel page");

            // A failure on the first page must surface as an error so callers
            // can tell "channel unreachable" apart from "channel has no posts";
            // later-page failures keep the partial result already collected.
            let resp = match self.client.get(&url).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) if page == 0 => {
                    anyhow::bail!("Telegram channel {channel} returned HTTP {}", r.status())
                }
                Ok(r) => {
                    tracing::warn!(page, status = %r.status(), "Telegram page non-success; returning partial results");
                    break;
                }
                Err(e) if page == 0 => {
                    return Err(anyhow::Error::new(e).context("Telegram channel fetch"));
                }
                Err(e) => {
                    tracing::warn!(page, error = %e, "Telegram fetch error; returning partial results");
                    break;
                }
            };
            let html = crate::http::read_capped(resp, crate::http::MAX_EXTERNAL_BODY_BYTES).await?;

            let posts = self.parse_channel_html(channel, &html);
            if posts.is_empty() {
                break;
            }

            // Track the lowest post ID seen for pagination
            before_id = posts
                .iter()
                .filter_map(|p| p.post_id.parse::<u64>().ok())
                .min()
                .map(|id| id.saturating_sub(1));

            all_posts.extend(posts);

            if all_posts.len() >= max_posts {
                break;
            }
        }

        all_posts.truncate(max_posts);
        Ok(all_posts)
    }

    // ── HTML parser ───────────────────────────────────────────────

    fn parse_channel_html(&self, channel: &str, html: &str) -> Vec<SocialPost> {
        let mut posts = Vec::new();

        for block in html.split("tgme_widget_message_wrap") {
            if !block.contains("tgme_widget_message_text") {
                continue;
            }

            let post_id = self
                .extract_message_id(block)
                .unwrap_or_else(|| "0".to_string());
            let text = self.extract_message_text(block);
            if text.trim().is_empty() {
                continue;
            }

            let published_at = self.extract_datetime(block);
            let view_count = self.extract_view_count(block);

            let mut post = SocialPost::minimal("telegram", &post_id, channel, &text, published_at);
            post.post_url = format!("https://t.me/{}/{}", channel, post_id);
            post.like_count = view_count;
            post.author_display_name = Some(format!("@{}", channel));

            posts.push(post);
        }

        posts
    }

    fn extract_message_id(&self, block: &str) -> Option<String> {
        // Look for data-post="channel/12345"
        let marker = "data-post=\"";
        let start = block.find(marker)? + marker.len();
        let end = block[start..].find('"')?;
        let id_part = &block[start..start + end];
        id_part.split('/').next_back().map(|s| s.to_string())
    }

    fn extract_message_text(&self, block: &str) -> String {
        let marker = "tgme_widget_message_text";
        let start = match block.find(marker) {
            Some(i) => i,
            None => return String::new(),
        };

        let inner = &block[start..];
        let tag_end = match inner.find('>') {
            Some(i) => i + 1,
            None => return String::new(),
        };

        // Extract up to closing div
        let content = &inner[tag_end..];
        let end = content.find("</div>").unwrap_or(content.len().min(2000));
        strip_html_tags(&content[..end])
    }

    fn extract_datetime(&self, block: &str) -> Option<DateTime<Utc>> {
        let marker = "datetime=\"";
        let start = block.find(marker)? + marker.len();
        let end = block[start..].find('"')?;
        let dt_str = &block[start..start + end];
        DateTime::parse_from_rfc3339(dt_str)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    fn extract_view_count(&self, block: &str) -> u64 {
        let marker = "tgme_widget_message_views";
        let start = match block.find(marker) {
            Some(i) => i,
            None => return 0,
        };
        let inner = &block[start..];
        let content_start = inner.find('>').map(|i| i + 1).unwrap_or(0);
        let content_end = inner.find("</span>").unwrap_or(content_start + 20);
        let text = &inner[content_start..content_end.min(inner.len())];
        let clean = text
            .trim()
            .replace(',', "")
            .replace('K', "000")
            .replace('M', "000000");
        clean.parse().unwrap_or(0)
    }
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    /// Serves `responses` in order, one per connection, and records each
    /// request line so pagination can be asserted.
    async fn scripted_server(responses: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut queue: VecDeque<String> = responses.into();
        let seen = requests.clone();
        tokio::spawn(async move {
            while let Some(response) = queue.pop_front() {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0_u8; 2048];
                let read = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..read]);
                seen.lock()
                    .await
                    .push(request.lines().next().unwrap_or_default().to_string());
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{addr}/s"), requests)
    }

    fn http_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    fn post_html(channel: &str, ids: &[u64]) -> String {
        ids.iter()
            .map(|id| {
                format!(
                    r#"<div class="tgme_widget_message_wrap"><div data-post="{channel}/{id}"><div class="tgme_widget_message_text">post {id}</div></div></div>"#
                )
            })
            .collect()
    }

    fn local_scraper(base_url: String) -> TelegramScraper {
        let client = crate::http::external_client_with(crate::http::ExternalClientOptions {
            timeout: Duration::from_secs(5),
            allow_private_targets: true,
            ..crate::http::ExternalClientOptions::default()
        })
        .expect("build client");
        TelegramScraper { client, base_url }
    }

    #[tokio::test]
    async fn first_page_http_error_is_surfaced() {
        let (base, _) = scripted_server(vec![http_response("503 Service Unavailable", "")]).await;
        let error = local_scraper(base)
            .channel_posts("chan", 10)
            .await
            .expect_err("an unreachable channel must not look like an empty one");
        assert!(error.to_string().contains("503"), "{error}");
    }

    #[tokio::test]
    async fn first_page_transport_error_is_surfaced() {
        // Bind then drop: the port refuses connections.
        let addr = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind")
            .local_addr()
            .expect("addr");
        let result = local_scraper(format!("http://{addr}/s"))
            .channel_posts("chan", 10)
            .await;
        assert!(result.is_err(), "a refused first page must be an error");
    }

    #[tokio::test]
    async fn later_page_error_keeps_partial_results_and_paginates() {
        let (base, requests) = scripted_server(vec![
            http_response("200 OK", &post_html("chan", &[105, 104])),
            http_response("500 Internal Server Error", ""),
        ])
        .await;
        let posts = local_scraper(base)
            .channel_posts("chan", 10)
            .await
            .expect("a later-page failure keeps the first page");
        let ids: Vec<&str> = posts.iter().map(|p| p.post_id.as_str()).collect();
        assert_eq!(ids, ["105", "104"]);
        let requests = requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /s/chan "), "{}", requests[0]);
        assert!(
            requests[1].starts_with("GET /s/chan?before=103 "),
            "{}",
            requests[1]
        );
    }

    #[test]
    fn scraper_builds() {
        assert!(TelegramScraper::new(None).is_ok());
    }

    #[test]
    fn parse_empty_html_returns_no_posts() {
        let s = TelegramScraper::new(None)
            .unwrap_or_else(|error| panic!("telegram scraper should build: {error}"));
        let posts = s.parse_channel_html("test", "<html></html>");
        assert!(posts.is_empty());
    }

    #[test]
    fn extract_datetime_valid() {
        let s = TelegramScraper::new(None)
            .unwrap_or_else(|error| panic!("telegram scraper should build: {error}"));
        let html = r#"some content datetime="2024-01-15T14:30:00+00:00" more"#;
        let dt = s.extract_datetime(html);
        assert!(dt.is_some());
    }
}
