//! Email digest scheduling, rendering, and delivery.

use anyhow::Result;
use apex_core::env::parse_truthy_flag;
use apex_store::postgres::{InsightListFilters, PgStore};
use chrono::{Datelike, Timelike, Utc};
use chrono_tz::Europe::Berlin;
use lettre::message::{header::ContentType, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::sync::Arc;

use crate::config;
use crate::digest_filtering::{
    canonical_digest_key, digest_tokens, expand_digest_categories, is_digest_insight_quality,
    token_jaccard_similarity,
};
use crate::prompts::clean_rendered_text;
pub(crate) fn parse_digest_recipients(raw: &str) -> Vec<String> {
    raw.split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn parse_hhmm(raw: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = raw.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hour = parts[0].parse::<u32>().ok()?;
    let minute = parts[1].parse::<u32>().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

pub(crate) fn weekday_matches(weekday: &str, now_weekday: chrono::Weekday) -> bool {
    match weekday {
        "Mon" => now_weekday == chrono::Weekday::Mon,
        "Tue" => now_weekday == chrono::Weekday::Tue,
        "Wed" => now_weekday == chrono::Weekday::Wed,
        "Thu" => now_weekday == chrono::Weekday::Thu,
        "Fri" => now_weekday == chrono::Weekday::Fri,
        "Sat" => now_weekday == chrono::Weekday::Sat,
        "Sun" => now_weekday == chrono::Weekday::Sun,
        _ => false,
    }
}

pub(crate) fn is_digest_due(
    now_cet: chrono::DateTime<chrono_tz::Tz>,
    prefs: &apex_store::postgres::UserSettingsPrefs,
) -> bool {
    if !prefs.email_digest_enabled {
        return false;
    }
    let Some((target_hour, target_minute)) = parse_hhmm(&prefs.email_digest_time_cet) else {
        return false;
    };
    if now_cet.hour() < target_hour
        || (now_cet.hour() == target_hour && now_cet.minute() < target_minute)
    {
        return false;
    }

    let last_sent_cet = prefs
        .email_digest_last_sent_at
        .map(|dt| dt.with_timezone(&Berlin));

    if prefs.notification_frequency.eq_ignore_ascii_case("weekly") {
        if !weekday_matches(&prefs.email_digest_weekday, now_cet.weekday()) {
            return false;
        }
        if let Some(last) = last_sent_cet {
            let now_week = now_cet.iso_week();
            let last_week = last.iso_week();
            if now_week.year() == last_week.year() && now_week.week() == last_week.week() {
                return false;
            }
        }
        true
    } else {
        if let Some(last) = last_sent_cet {
            if last.date_naive() == now_cet.date_naive() {
                return false;
            }
        }
        true
    }
}

pub(crate) fn build_digest_html(
    base_url: &str,
    insights: &[apex_store::postgres::InsightRow],
    category_label: &str,
) -> String {
    let mut cards = String::new();
    for insight in insights {
        let id = insight.id;
        let confidence = ((insight.confidence.unwrap_or(0.0) * 100.0).round() as i64).clamp(0, 100);
        let category = insight
            .insight_type
            .clone()
            .unwrap_or_else(|| "general".to_string())
            .replace('_', " ");
        let updated = insight
            .updated_at
            .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "n/a".to_string());
        let summary = insight
            .summary
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\n', "<br/>");
        let title = insight.title.replace('<', "&lt;").replace('>', "&gt;");
        cards.push_str(&format!(
                        "<div style=\"margin:0 0 14px;padding:14px 16px 12px;border:1px solid #BCBCBC;border-radius:2px;background:#FFFFFF;box-shadow:inset 0 1px 0 rgba(255,255,255,.75);\">\
                         <div style=\"margin:0 0 6px;font-size:11px;line-height:1.35;color:#606060;font-weight:700;text-transform:uppercase;letter-spacing:.08em;\">{category} • {confidence}% confidence</div>\
                         <a href=\"{base_url}/insights/{id}\" style=\"display:block;margin:0 0 9px;color:#101010;font-size:17px;line-height:1.35;font-weight:700;text-decoration:none;\">{title}</a>\
                         <p style=\"margin:0 0 10px;color:#303030;font-size:14px;line-height:1.55;\">{summary}</p>\
                         <a href=\"{base_url}/insights/{id}\" style=\"display:inline-block;padding:8px 12px;border-radius:4px;background:#FFBE00;color:#121212;font-size:12px;font-weight:700;text-decoration:none;\">Open insight</a>\
                         <span style=\"float:right;padding-top:8px;color:#606060;font-size:12px;\">Updated: {updated}</span>\
                         </div>"
        ));
    }

    let generated_at = Utc::now().format("%Y-%m-%d %H:%M UTC");

    format!(
                "<!doctype html><html><body style=\"margin:0;padding:0;background:#F2F2F2;font-family:Inter,Segoe UI,Arial,sans-serif;color:#101010;\">\
                     <div style=\"max-width:760px;margin:24px auto;padding:0 12px;\">\
                         <div style=\"background:#101010;padding:18px 20px;border-radius:4px 4px 0 0;\">\
                             <div style=\"font-size:11px;letter-spacing:.12em;text-transform:uppercase;font-weight:800;color:#FFBE00;\">ApexIntel</div>\
                             <h1 style=\"margin:6px 0 0;font-size:24px;line-height:1.2;color:#F4F6FA;\">Top Insights Digest</h1>\
                             <p style=\"margin:8px 0 0;color:#B6BCC7;font-size:13px;line-height:1.45;\">Generated {generated_at} • Categories: {category_label} • {insights_len} insights</p>\
                         </div>\
                         <div style=\"background:#FFFFFF;padding:16px;border:1px solid #BCBCBC;border-top:none;border-radius:0 0 4px 4px;\">\
                             {cards}\
                             <div style=\"margin-top:12px;padding-top:10px;border-top:1px solid #D9D9D9;\">\
                                 <a href=\"{base_url}/insights\" style=\"display:inline-block;padding:10px 14px;border-radius:4px;background:#111111;color:#F3F4F8;font-size:12px;font-weight:700;text-decoration:none;\">View all insights</a>\
                             </div>\
                         </div>\
                     </div>\
                 </body></html>",
                insights_len = insights.len()
    )
}

pub(crate) fn build_digest_text(
    base_url: &str,
    insights: &[apex_store::postgres::InsightRow],
    category_label: &str,
) -> String {
    let mut out = format!(
        "ApexIntel Top Insights Digest\nCategories: {}\n\n",
        category_label
    );
    for (idx, insight) in insights.iter().enumerate() {
        let confidence = ((insight.confidence.unwrap_or(0.0) * 100.0).round() as i64).clamp(0, 100);
        out.push_str(&format!(
            "{}. {} ({}%)\n{}\n{}/insights/{}\n\n",
            idx + 1,
            insight.title,
            confidence,
            insight.summary,
            base_url,
            insight.id
        ));
    }
    out
}

pub(crate) async fn send_digest_email(
    recipients: &[String],
    subject: &str,
    html_body: String,
    text_body: String,
) -> Result<()> {
    let from_address = "contact@apexmediation.ee";
    let smtp_host = std::env::var("EMAIL_DIGEST_SMTP_HOST")
        .unwrap_or_else(|_| "mail.apexmediation.ee".to_string());
    let smtp_port: u16 = std::env::var("EMAIL_DIGEST_SMTP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(25);
    let smtp_user = std::env::var("EMAIL_DIGEST_SMTP_USER").unwrap_or_default();
    let smtp_pass = std::env::var("EMAIL_DIGEST_SMTP_PASS").unwrap_or_default();
    let smtp_starttls = std::env::var("EMAIL_DIGEST_SMTP_STARTTLS")
        .ok()
        .map(|v| parse_truthy_flag(&v))
        .unwrap_or(false);

    let mut builder = Message::builder()
        .from(from_address.parse::<Mailbox>()?)
        .subject(subject);
    for to in recipients {
        builder = builder.to(to.parse::<Mailbox>()?);
    }

    let email = builder.multipart(
        MultiPart::alternative()
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(text_body),
            )
            .singlepart(
                SinglePart::builder()
                    .header(ContentType::TEXT_HTML)
                    .body(html_body),
            ),
    )?;

    let mailer = if smtp_starttls {
        // Submission on 587 expects STARTTLS upgrade, not implicit TLS.
        let mut transport =
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp_host)?.port(smtp_port);
        if !smtp_user.trim().is_empty() {
            transport = transport.credentials(Credentials::new(smtp_user, smtp_pass));
        }
        transport.build()
    } else {
        let mut transport =
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp_host).port(smtp_port);
        if !smtp_user.trim().is_empty() {
            transport = transport.credentials(Credentials::new(smtp_user, smtp_pass));
        }
        transport.build()
    };

    mailer.send(email).await?;
    Ok(())
}

pub(crate) async fn run_update_email_digest_job(store: &Arc<PgStore>) -> Result<(u64, u64)> {
    let subscribers = store.list_user_settings_prefs_for_email_digest().await?;
    if subscribers.is_empty() {
        return Ok((0, 0));
    }

    let now_utc = Utc::now();
    let now_cet = now_utc.with_timezone(&Berlin);
    let base_url = std::env::var("EMAIL_DIGEST_BASE_URL")
        .unwrap_or_else(|_| "https://starzerp.fi".to_string())
        .trim_end_matches('/')
        .to_string();

    let mut sent_count: u64 = 0;
    let mut users_due: u64 = 0;

    for (user_id, prefs) in subscribers {
        if !is_digest_due(now_cet, &prefs) {
            continue;
        }
        users_due += 1;

        let recipients = parse_digest_recipients(&prefs.email_digest_recipients);
        if recipients.is_empty() {
            tracing::warn!(user_id = %user_id, "email digest enabled but recipients empty");
            continue;
        }

        let since = if prefs.notification_frequency.eq_ignore_ascii_case("weekly") {
            now_utc - chrono::Duration::days(7)
        } else {
            now_utc - chrono::Duration::days(1)
        };

        let mut filters = InsightListFilters {
            date_from: Some(since),
            ..Default::default()
        };
        let mapped_insight_types = expand_digest_categories(&prefs.email_digest_categories);
        if !mapped_insight_types.is_empty() {
            filters.insight_types = mapped_insight_types;
        }

        let mut rows = store.list_insights(&filters, 200, 0).await?;
        rows.retain(|r| {
            !r.insight_type
                .as_deref()
                .map(|t| t.trim().to_ascii_lowercase().starts_with("llm_"))
                .unwrap_or(false)
        });
        if prefs.critical_only_enabled {
            rows.retain(|r| r.confidence.unwrap_or(0.0) >= 0.7);
        }
        rows.sort_by(|a, b| {
            b.confidence
                .unwrap_or(0.0)
                .total_cmp(&a.confidence.unwrap_or(0.0))
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });

        let mut curated = Vec::new();
        let mut seen_title_keys: Vec<String> = Vec::new();
        let mut seen_summary_keys: Vec<String> = Vec::new();
        let mut seen_signature_tokens: Vec<(Vec<String>, Vec<String>)> = Vec::new();
        for mut row in rows {
            row.title = clean_rendered_text(&row.title);
            row.summary = clean_rendered_text(&row.summary);

            if !is_digest_insight_quality(&row.title, &row.summary) {
                continue;
            }

            let title_key = canonical_digest_key(&row.title, 10);
            if title_key.is_empty() {
                continue;
            }
            if seen_title_keys.iter().any(|k| k == &title_key) {
                continue;
            }

            let summary_key = canonical_digest_key(&row.summary, 14);
            if !summary_key.is_empty() && seen_summary_keys.iter().any(|k| k == &summary_key) {
                continue;
            }

            // Allow multiple updates per company, but suppress near-duplicate variants.
            let title_tokens = digest_tokens(&row.title, 20);
            let summary_tokens = digest_tokens(&row.summary, 80);
            let near_duplicate = seen_signature_tokens
                .iter()
                .any(|(seen_title, seen_summary)| {
                    let title_sim = token_jaccard_similarity(&title_tokens, seen_title);
                    let summary_sim = token_jaccard_similarity(&summary_tokens, seen_summary);
                    title_sim >= *config::DEDUP_TITLE_THRESHOLD
                        && summary_sim >= *config::DEDUP_SUMMARY_THRESHOLD
                });
            if near_duplicate {
                continue;
            }

            seen_title_keys.push(title_key);
            if !summary_key.is_empty() {
                seen_summary_keys.push(summary_key);
            }
            seen_signature_tokens.push((title_tokens, summary_tokens));
            curated.push(row);
        }

        let category_label = if prefs.email_digest_categories.is_empty() {
            "All".to_string()
        } else {
            prefs.email_digest_categories.join(", ")
        };
        let top: Vec<_> = curated.into_iter().take(8).collect();
        if top.is_empty() {
            tracing::info!(user_id = %user_id, "digest due but no matching insights");
            continue;
        }

        let subject = format!(
            "ApexIntel Update: {} top insights ({})",
            top.len(),
            now_cet.format("%Y-%m-%d")
        );
        let html = build_digest_html(&base_url, &top, &category_label);
        let text = build_digest_text(&base_url, &top, &category_label);

        // B333: one failing recipient previously aborted the whole loop —
        // every user after the failure lost their digest that cycle. Log and
        // continue; only a send that succeeded marks the digest as sent.
        if let Err(error) = send_digest_email(&recipients, &subject, html, text).await {
            tracing::error!(user_id = %user_id, %error, "email digest send failed");
            continue;
        }
        if let Err(error) = store.mark_email_digest_sent(&user_id, now_utc).await {
            tracing::warn!(user_id = %user_id, %error, "mark_email_digest_sent failed (digest may re-send)");
        }
        sent_count += 1;
        tracing::info!(user_id = %user_id, recipients = recipients.len(), insights = top.len(), "email digest sent");
    }

    Ok((sent_count, users_due))
}
