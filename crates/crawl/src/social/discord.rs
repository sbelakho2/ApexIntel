//! Discord public server monitor.
//!
//! Scrapes publicly-visible server metadata via the Discord widget JSON
//! endpoint and invite preview API.  **No bot token or login required.**
//!
//! Use cases:
//! - Track community size changes for competitors / industry groups
//! - Detect growth/decline signals in EMS/electronics Discord servers

use super::SocialPost;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::debug;

const DISCORD_WIDGET_URL: &str = "https://discord.com/api/guilds";
const DISCORD_INVITE_URL: &str = "https://discord.com/api/v10/invites";

/// Discord community intelligence scraper.
pub struct DiscordScraper {
    client: Client,
}

impl DiscordScraper {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("ApexIntel/1.0 (+https://apexintel.io) OSINT Collector")
            .build()
            .context("building Discord HTTP client")?;
        Ok(Self { client })
    }

    // ── Widget endpoint ─────────────────────────────────────────

    /// Fetch server widget data (requires server to have widget enabled).
    pub async fn fetch_server_widget(&self, guild_id: &str) -> Result<ServerSnapshot> {
        let url = format!("{}/{}/widget.json", DISCORD_WIDGET_URL, guild_id);
        debug!(guild_id, "Fetching Discord widget");

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Discord widget GET")?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Discord widget returned HTTP {} for guild {}",
                resp.status(),
                guild_id
            );
        }

        let widget: DiscordWidget = resp.json().await.context("parsing Discord widget JSON")?;

        Ok(ServerSnapshot {
            guild_id: guild_id.to_string(),
            name: widget.name,
            member_count: widget.presence_count,
            channel_count: widget.channels.len(),
            invite_url: widget.instant_invite,
            channels: widget
                .channels
                .into_iter()
                .map(|c| ChannelInfo {
                    id: c.id,
                    name: c.name,
                    position: c.position,
                })
                .collect(),
            fetched_at: Utc::now(),
        })
    }

    // ── Invite preview (public, no auth) ────────────────────────

    /// Fetch server info via invite code.
    pub async fn fetch_invite_preview(&self, invite_code: &str) -> Result<InvitePreview> {
        let url = format!(
            "{}/{}?with_counts=true&with_expiration=true",
            DISCORD_INVITE_URL, invite_code
        );

        debug!(invite_code, "Fetching Discord invite preview");

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Discord invite GET")?;

        if !resp.status().is_success() {
            anyhow::bail!("Discord invite returned HTTP {}", resp.status());
        }

        let invite: DiscordInviteResp =
            resp.json().await.context("parsing Discord invite JSON")?;

        Ok(InvitePreview {
            guild_name: invite
                .guild
                .as_ref()
                .map(|g| g.name.clone())
                .unwrap_or_default(),
            guild_id: invite
                .guild
                .as_ref()
                .map(|g| g.id.clone())
                .unwrap_or_default(),
            description: invite.guild.as_ref().and_then(|g| g.description.clone()),
            member_count: invite.approximate_member_count.unwrap_or(0),
            online_count: invite.approximate_presence_count.unwrap_or(0),
            channel_name: invite
                .channel
                .as_ref()
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            fetched_at: Utc::now(),
        })
    }

    // ── Change detection ────────────────────────────────────────

    /// Compare two snapshots and emit intelligence signals for significant changes.
    pub fn detect_changes(
        &self,
        previous: &ServerSnapshot,
        current: &ServerSnapshot,
    ) -> Vec<SocialPost> {
        let mut signals = Vec::new();
        let member_delta = current.member_count as i64 - previous.member_count as i64;
        let pct_change = if previous.member_count > 0 {
            (member_delta as f64 / previous.member_count as f64 * 100.0).abs()
        } else {
            0.0
        };

        // Emit signal if member change > 10% OR > 50 absolute
        if pct_change > 10.0 || member_delta.unsigned_abs() > 50 {
            let direction = if member_delta > 0 {
                "growth"
            } else {
                "decline"
            };

            let text = format!(
                "Discord server '{}' member {}: {} → {} ({:+} / {:.1}%)",
                current.name,
                direction,
                previous.member_count,
                current.member_count,
                member_delta,
                pct_change
            );

            let mut post = SocialPost::minimal(
                "discord",
                &format!(
                    "discord:delta:{}:{}",
                    current.guild_id,
                    Utc::now().timestamp()
                ),
                &current.name,
                &text,
                Utc::now(),
            );
            post.post_url = current
                .invite_url
                .clone()
                .unwrap_or_else(|| format!("https://discord.com/servers/{}", current.guild_id));

            signals.push(post);
        }

        // Channel count change
        let ch_delta = current.channel_count as i64 - previous.channel_count as i64;
        if ch_delta.unsigned_abs() >= 3 {
            let text = format!(
                "Discord server '{}' channel change: {} → {} ({:+})",
                current.name, previous.channel_count, current.channel_count, ch_delta
            );

            signals.push(SocialPost::minimal(
                "discord",
                &format!(
                    "discord:ch_delta:{}:{}",
                    current.guild_id,
                    Utc::now().timestamp()
                ),
                &current.name,
                &text,
                Utc::now(),
            ));
        }

        signals
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of a Discord server's public state.
#[derive(Debug, Clone)]
pub struct ServerSnapshot {
    pub guild_id: String,
    pub name: String,
    pub member_count: u64,
    pub channel_count: usize,
    pub invite_url: Option<String>,
    pub channels: Vec<ChannelInfo>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub position: i32,
}

/// Summary of a server obtained via invite code.
#[derive(Debug, Clone)]
pub struct InvitePreview {
    pub guild_name: String,
    pub guild_id: String,
    pub description: Option<String>,
    pub member_count: u64,
    pub online_count: u64,
    pub channel_name: String,
    pub fetched_at: DateTime<Utc>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Discord API response shapes
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct DiscordWidget {
    name: String,
    presence_count: u64,
    channels: Vec<WidgetChannel>,
    instant_invite: Option<String>,
}

#[derive(Deserialize)]
struct WidgetChannel {
    id: String,
    name: String,
    position: i32,
}

#[derive(Deserialize)]
struct DiscordInviteResp {
    guild: Option<InviteGuild>,
    channel: Option<InviteChannel>,
    approximate_member_count: Option<u64>,
    approximate_presence_count: Option<u64>,
}

#[derive(Deserialize)]
struct InviteGuild {
    id: String,
    name: String,
    description: Option<String>,
}

#[derive(Deserialize)]
struct InviteChannel {
    name: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(name: &str, members: u64, channels: usize) -> ServerSnapshot {
        ServerSnapshot {
            guild_id: "12345".into(),
            name: name.into(),
            member_count: members,
            channel_count: channels,
            invite_url: None,
            channels: vec![],
            fetched_at: Utc::now(),
        }
    }

    #[test]
    fn detect_member_growth() {
        let scraper = DiscordScraper::new().unwrap();
        let prev = make_snapshot("TestServer", 100, 5);
        let curr = make_snapshot("TestServer", 200, 5);
        let signals = scraper.detect_changes(&prev, &curr);
        assert_eq!(signals.len(), 1);
        assert!(signals[0].text.contains("growth"));
    }

    #[test]
    fn detect_member_decline() {
        let scraper = DiscordScraper::new().unwrap();
        let prev = make_snapshot("TestServer", 200, 5);
        let curr = make_snapshot("TestServer", 100, 5);
        let signals = scraper.detect_changes(&prev, &curr);
        assert_eq!(signals.len(), 1);
        assert!(signals[0].text.contains("decline"));
    }

    #[test]
    fn no_signal_for_small_change() {
        let scraper = DiscordScraper::new().unwrap();
        let prev = make_snapshot("TestServer", 1000, 10);
        let curr = make_snapshot("TestServer", 1005, 10);
        let signals = scraper.detect_changes(&prev, &curr);
        assert!(signals.is_empty());
    }

    #[test]
    fn channel_change_signal() {
        let scraper = DiscordScraper::new().unwrap();
        let prev = make_snapshot("TestServer", 100, 5);
        let curr = make_snapshot("TestServer", 100, 10);
        let signals = scraper.detect_changes(&prev, &curr);
        assert_eq!(signals.len(), 1);
        assert!(signals[0].text.contains("channel change"));
    }

    #[test]
    fn scraper_builds() {
        assert!(DiscordScraper::new().is_ok());
    }
}
