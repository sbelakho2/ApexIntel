//! Security handler — GET /security
//!
//! Covers: security posture page with DNS analysis, KEV tracking,
//! lookalike domain detection, and overall security scoring.

use std::sync::Arc;

use askama::Template;
use axum::{extract::Query, http::HeaderMap, response::IntoResponse, Extension};
use serde::Deserialize;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_store::postgres::{PgStore, WarningListFilters, WarningOrderBy};

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DnsPostureItem {
    pub domain: String,
    pub has_spf: bool,
    pub has_dkim: bool,
    pub has_dmarc: bool,
    pub has_dnssec: bool,
    pub score: i64,
}

#[derive(Clone, Debug)]
pub struct KevItem {
    pub cve_id: String,
    pub name: String,
    pub vendor: String,
    pub product: String,
    pub date_added: String,
    pub due_date: String,
    pub relevant_companies: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct LookalikeDomain {
    pub domain: String,
    pub target_domain: String,
    pub similarity: f64,
    pub registered_at: Option<String>,
    pub is_active: bool,
    pub risk_level: String,
}

#[derive(Clone, Debug)]
pub struct SecurityScore {
    pub category: String,
    pub score: i64,
    pub max_score: i64,
    pub details: String,
}

#[derive(Clone, Debug)]
pub struct SecurityFinding {
    pub severity: String,
    pub title: String,
    pub summary: String,
    pub region: String,
    pub signal: String,
    pub created_at: String,
    pub acknowledged: bool,
    pub detail_href: String,
    pub source_href: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SecurityQuery {
    pub signal: Option<String>,
    pub risk: Option<String>,
}

fn classify_security_signal(title: &str, description: Option<&str>) -> String {
    let t = title.to_ascii_lowercase();
    let d = description.unwrap_or_default().to_ascii_lowercase();
    if t.contains("dns posture") || d.contains("dns posture") {
        "dns".to_string()
    } else if t.contains("lookalike") || d.contains("lookalike") || d.contains("typosquat") {
        "lookalike".to_string()
    } else if t.contains("kev") || t.contains("cisa") || d.contains("cve-") {
        "kev".to_string()
    } else {
        "security".to_string()
    }
}

fn signal_label(signal: &str) -> &'static str {
    match signal {
        "dns" => "DNS posture",
        "lookalike" => "Lookalike domain",
        "kev" => "Known exploited vulnerabilities",
        _ => "Security",
    }
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/security.html")]
pub struct SecurityPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub overall_score: i64,
    pub dns_posture: Vec<DnsPostureItem>,
    pub kev_items: Vec<KevItem>,
    pub lookalike_domains: Vec<LookalikeDomain>,
    pub scores: Vec<SecurityScore>,
    pub last_scan_at: String,
    pub domains_monitored: i64,
    pub findings: Vec<SecurityFinding>,
    pub findings_total: i64,
    pub active_findings: i64,
    pub acknowledged_findings: i64,
    pub critical_high_count: i64,
    pub dns_posture_pass: i64,
    pub posture_warnings: i64,
    pub active_signal: String,
    pub active_risk: String,
    pub risk_critical_count: i64,
    pub risk_high_count: i64,
    pub risk_medium_count: i64,
    pub risk_low_count: i64,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /security — security posture overview.
pub async fn security_page(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<SecurityQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/security", unack);

    // DNS posture observations
    let dns_obs = store
        .get_dns_posture_entries(100)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to load DNS posture: {e}");
            vec![]
        });
    let dns_posture: Vec<DnsPostureItem> = dns_obs
        .iter()
        .map(|o| {
            let v = &o.value;
            DnsPostureItem {
                domain: v
                    .get("domain")
                    .and_then(|d| d.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                has_spf: v.get("has_spf").and_then(|b| b.as_bool()).unwrap_or(false),
                has_dkim: v.get("has_dkim").and_then(|b| b.as_bool()).unwrap_or(false),
                has_dmarc: v
                    .get("has_dmarc")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
                has_dnssec: v
                    .get("has_dnssec")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false),
                score: v
                    .get("posture_score")
                    .and_then(|s| s.as_f64())
                    .map(|s| (s * 100.0) as i64)
                    .unwrap_or(0),
            }
        })
        .collect();

    // KEV observations
    let kev_obs = store.get_kev_relevance(50).await.unwrap_or_else(|e| {
        tracing::error!("Failed to load KEV data: {e}");
        vec![]
    });
    let kev_items: Vec<KevItem> = kev_obs
        .iter()
        .map(|o| {
            let v = &o.value;
            KevItem {
                cve_id: v
                    .get("cve_id")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: v
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
                vendor: v
                    .get("vendor")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                product: v
                    .get("product")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                date_added: v
                    .get("date_added")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                due_date: v
                    .get("due_date")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                relevant_companies: v
                    .get("relevant_companies")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();

    // Lookalike domains
    let lookalike_obs = store.get_lookalike_domains(50).await.unwrap_or_else(|e| {
        tracing::error!("Failed to load lookalike domains: {e}");
        vec![]
    });
    let lookalike_domains: Vec<LookalikeDomain> = lookalike_obs
        .iter()
        .map(|o| {
            let v = &o.value;
            LookalikeDomain {
                domain: v
                    .get("lookalike_domain")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                target_domain: v
                    .get("original_domain")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                similarity: v.get("similarity").and_then(|s| s.as_f64()).unwrap_or(0.0),
                registered_at: v
                    .get("registered_at")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string()),
                is_active: v.get("active").and_then(|b| b.as_bool()).unwrap_or(false),
                risk_level: v
                    .get("risk_level")
                    .and_then(|s| s.as_str())
                    .unwrap_or("low")
                    .to_string(),
            }
        })
        .collect();

    // Compute overall score from DNS posture
    let overall_score = if dns_posture.is_empty() {
        0
    } else {
        dns_posture.iter().map(|d| d.score).sum::<i64>() / dns_posture.len() as i64
    };

    let domains_monitored = dns_posture.len() as i64;
    let last_scan_at = dns_obs
        .first()
        .map(|o| o.ts_utc.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into());

    let warning_findings = store
        .list_warnings(
            &WarningListFilters {
                warning_types: vec!["security".to_string()],
                ..Default::default()
            },
            Some(WarningOrderBy::CreatedAt),
            true,
            100,
            0,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to load security warnings: {e}");
            vec![]
        });

    let mut findings: Vec<SecurityFinding> = warning_findings
        .into_iter()
        .map(|w| {
            let signal = classify_security_signal(&w.title, w.description.as_deref());
            SecurityFinding {
                severity: w.severity,
                title: w.title,
                summary: w
                    .description
                    .unwrap_or_else(|| "No additional details provided.".to_string()),
                region: w.region.unwrap_or_else(|| "Global".to_string()),
                signal: signal_label(&signal).to_string(),
                created_at: w.ts_utc.format("%Y-%m-%d %H:%M").to_string(),
                acknowledged: w.acknowledged,
                detail_href: format!("/warnings/{}", w.id),
                source_href: w
                    .source_urls
                    .and_then(|urls| urls.into_iter().find(|u| !u.is_empty())),
            }
        })
        .collect();

    // Fallback to synthesized findings if no security warnings exist yet.
    if findings.is_empty() {
        for item in &kev_items {
            findings.push(SecurityFinding {
                severity: "high".into(),
                title: format!("{} ({})", item.name, item.cve_id),
                summary: "Potentially relevant known exploited vulnerability.".into(),
                region: "Global".into(),
                signal: signal_label("kev").into(),
                created_at: item.date_added.clone(),
                acknowledged: false,
                detail_href: "/warnings".into(),
                source_href: None,
            });
        }

        for item in &lookalike_domains {
            findings.push(SecurityFinding {
                severity: item.risk_level.clone(),
                title: format!("Lookalike domain {}", item.domain),
                summary: format!("Similar to monitored domain {}", item.target_domain),
                region: "Global".into(),
                signal: signal_label("lookalike").into(),
                created_at: item.registered_at.clone().unwrap_or_else(|| "—".into()),
                acknowledged: false,
                detail_href: "/warnings".into(),
                source_href: None,
            });
        }

        for item in &dns_posture {
            let mut issues = 0;
            if !item.has_spf {
                issues += 1;
            }
            if !item.has_dkim {
                issues += 1;
            }
            if !item.has_dmarc {
                issues += 1;
            }
            if issues > 0 {
                let severity = if issues >= 3 {
                    "high"
                } else if issues == 2 {
                    "medium"
                } else {
                    "low"
                };
                findings.push(SecurityFinding {
                    severity: severity.into(),
                    title: format!("DNS posture issue on {}", item.domain),
                    summary: "Missing SPF, DKIM, or DMARC controls were detected.".into(),
                    region: "Global".into(),
                    signal: signal_label("dns").into(),
                    created_at: last_scan_at.clone(),
                    acknowledged: false,
                    detail_href: "/warnings".into(),
                    source_href: None,
                });
            }
        }
    }

    let active_signal = params.signal.unwrap_or_default();
    let active_risk = params.risk.unwrap_or_default();

    if !active_signal.is_empty() {
        findings.retain(|f| f.signal.eq_ignore_ascii_case(signal_label(&active_signal)));
    }

    if !active_risk.is_empty() {
        findings.retain(|f| f.severity.eq_ignore_ascii_case(&active_risk));
    }

    let findings_total = findings.len() as i64;
    let active_findings = findings.iter().filter(|f| !f.acknowledged).count() as i64;
    let acknowledged_findings = findings.iter().filter(|f| f.acknowledged).count() as i64;
    let critical_high_count = findings
        .iter()
        .filter(|f| {
            f.severity.eq_ignore_ascii_case("critical") || f.severity.eq_ignore_ascii_case("high")
        })
        .count() as i64;
    let risk_critical_count = findings
        .iter()
        .filter(|f| f.severity.eq_ignore_ascii_case("critical"))
        .count() as i64;
    let risk_high_count = findings
        .iter()
        .filter(|f| f.severity.eq_ignore_ascii_case("high"))
        .count() as i64;
    let risk_medium_count = findings
        .iter()
        .filter(|f| f.severity.eq_ignore_ascii_case("medium"))
        .count() as i64;
    let risk_low_count = findings
        .iter()
        .filter(|f| f.severity.eq_ignore_ascii_case("low"))
        .count() as i64;

    let dns_posture_pass = dns_posture
        .iter()
        .filter(|d| d.has_spf && d.has_dkim && d.has_dmarc)
        .count() as i64;
    let posture_warnings = dns_posture.len() as i64 - dns_posture_pass;

    let tpl = SecurityPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        overall_score,
        dns_posture,
        kev_items,
        lookalike_domains,
        scores: vec![],
        last_scan_at,
        domains_monitored,
        findings,
        findings_total,
        active_findings,
        acknowledged_findings,
        critical_high_count,
        dns_posture_pass,
        posture_warnings,
        active_signal,
        active_risk,
        risk_critical_count,
        risk_high_count,
        risk_medium_count,
        risk_low_count,
    };

    if is_htmx_request(&headers) {
        tpl.into_response()
    } else {
        tpl.into_response()
    }
}
