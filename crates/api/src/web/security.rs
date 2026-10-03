//! Security handler — GET /security and POST /security/trigger-scan
//!
//! Covers: security posture page with DNS analysis, KEV tracking,
//! lookalike domain detection, and overall security scoring.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    Extension, Form,
};
use serde::Deserialize;

use super::{is_htmx_request, safe_href, PageContext};
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{PgStore, WarningListFilters, WarningOrderBy};

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct DnsPostureItem {
    pub domain: String,
    pub has_spf: bool,
    pub has_dkim: bool,
    pub has_dmarc: bool,
    pub has_dnssec: bool,
    /// Posture score (0–100), or `None` when a component lookup was
    /// indeterminate — an unknown posture must not render as 0.
    pub score: Option<i64>,
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
    /// Relevance from the catalog metadata; `None` = not recorded.
    pub relevance_score: Option<f64>,
    pub notes: Option<String>,
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

/// One security source's measured state, rendered as a badge. The label is
/// never a bare "0 findings": a clean result is only shown for a scan that
/// actually succeeded.
#[derive(Clone, Debug)]
pub struct SecuritySourceBadge {
    pub id: String,
    pub label: String,
    pub state: String,
    pub state_class: String,
    pub detail: String,
    pub findings: Option<u64>,
    pub last_scan_at: Option<String>,
}

/// Map a taxonomy state to its badge label and colour class. Every state has a
/// distinct label; the negative states never render as a clean result.
pub fn security_source_badge(
    state: crate::routes::security::SecuritySourceState,
) -> (&'static str, &'static str) {
    use crate::routes::security::SecuritySourceState as S;
    match state {
        S::FindingsReported => ("Findings reported", "apex-text-danger"),
        S::NoFindingsAfterSuccessfulScan => {
            ("No findings after successful scan", "apex-text-positive")
        }
        S::NotScanned => ("Not scanned", "text-rams-muted"),
        S::ScanFailed => ("Scan failed", "apex-text-danger"),
        S::AuthenticationUnavailable => ("Authentication unavailable", "apex-text-warning"),
        S::RateLimited => ("Rate limited", "apex-text-warning"),
        S::SourceUnavailable => ("Source unavailable", "apex-text-danger"),
        S::PartialScan => ("Partial scan", "apex-text-warning"),
        S::ScanSucceeded => ("Scan succeeded", "apex-text-positive"),
    }
}

fn security_source_badges(
    statuses: &[crate::routes::security::SecuritySourceStatus],
) -> Vec<SecuritySourceBadge> {
    statuses
        .iter()
        .map(|status| {
            let (label, state_class) = security_source_badge(status.state);
            SecuritySourceBadge {
                id: status.id.clone(),
                label: status.label.clone(),
                state: label.to_string(),
                state_class: state_class.to_string(),
                detail: status.detail.clone(),
                findings: status.findings,
                last_scan_at: status.last_scan_at.clone(),
            }
        })
        .collect()
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
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    pub overall_score: Option<i64>,
    pub dns_posture: Vec<DnsPostureItem>,
    pub kev_items: Vec<KevItem>,
    pub lookalike_domains: Vec<LookalikeDomain>,
    pub scores: Vec<SecurityScore>,
    pub last_scan_at: String,
    pub domains_monitored: i64,
    /// Per-source state taxonomy (never a bare "0 findings").
    pub source_states: Vec<SecuritySourceBadge>,
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
    pub degraded_notice: Option<String>,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /security — security posture overview.
pub async fn security_page(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(params): Query<SecurityQuery>,
) -> impl IntoResponse {
    let mut degraded_notice: Option<String> = None;
    let unack_state = DataState::from_result(
        store
            .count_warnings(&WarningListFilters {
                acknowledged: Some(false),
                ..Default::default()
            })
            .await,
        "count_warnings failed (web security page)",
        |_| false,
    );
    DegradedNotice::capture(&unack_state, &mut degraded_notice);
    let ctx = PageContext::from_session(&session, "/security", unack_state.into_loaded_or(0));

    // DNS posture observations
    let dns_state = DataState::from_result(
        store.get_dns_posture_entries(200).await,
        "get_dns_posture_entries failed (web security page)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&dns_state, &mut degraded_notice);
    let dns_obs = dns_state.into_items();
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
                    .map(|s| (s * 100.0) as i64),
            }
        })
        .collect();

    // KEV observations. A failed read is recorded as a degraded state (and the
    // source-state report keeps it as `None`) so it can never render as a
    // clean "no findings" scan.
    let kev_result = store.get_kev_relevance(200).await;
    let cve_findings = kev_result.as_ref().ok().map(|rows| rows.len() as u64);
    let kev_state = DataState::from_result(
        kev_result,
        "get_kev_relevance failed (web security page)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&kev_state, &mut degraded_notice);
    let kev_obs = kev_state.into_items();
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
                    .get("vulnerability_name")
                    .or_else(|| v.get("name"))
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
                    .get("affected_companies")
                    .or_else(|| v.get("relevant_companies"))
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                relevance_score: v.get("relevance_score").and_then(|s| s.as_f64()),
                notes: v
                    .get("notes")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string()),
            }
        })
        .collect();

    // Lookalike domains
    let lookalike_state = DataState::from_result(
        store.get_lookalike_domains(500).await,
        "get_lookalike_domains failed (web security page)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&lookalike_state, &mut degraded_notice);
    let lookalike_obs = lookalike_state.into_items();
    // #143: the same lookalike/target pair is written by every scan, so the
    // raw observation list repeats it; keep only the newest row per pair
    // (observations are ordered newest-first).
    let mut seen_lookalike_pairs = std::collections::HashSet::new();
    let lookalike_domains: Vec<LookalikeDomain> = lookalike_obs
        .iter()
        .map(|o| {
            let v = &o.value;
            // Data stores fields as: domain, original_domain, distance, threat_type, active
            let distance = v.get("distance").and_then(|s| s.as_i64()).unwrap_or(1);
            let threat_type = v
                .get("threat_type")
                .and_then(|s| s.as_str())
                .unwrap_or("typosquat");
            let risk_level = match threat_type {
                "homoglyph" => "medium",
                "tld_swap" => "medium",
                _ => {
                    // Lower distance = more risky
                    if distance <= 1 {
                        "medium"
                    } else {
                        "low"
                    }
                }
            };
            LookalikeDomain {
                domain: v
                    .get("domain")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                target_domain: v
                    .get("original_domain")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                similarity: (100.0 - (distance as f64).clamp(1.0, 10.0) * 10.0).max(0.0),
                registered_at: Some(o.ts_utc.format("%Y-%m-%d %H:%M").to_string()),
                is_active: v.get("active").and_then(|b| b.as_bool()).unwrap_or(false),
                risk_level: risk_level.to_string(),
            }
        })
        .filter(|view| {
            !view.domain.is_empty()
                && seen_lookalike_pairs.insert((
                    view.domain.to_ascii_lowercase(),
                    view.target_domain.to_ascii_lowercase(),
                ))
        })
        .collect();

    // Compute overall score from DNS posture, averaging only the domains whose
    // score was actually assertable (indeterminate lookups are not zeros).
    let known_scores: Vec<i64> = dns_posture.iter().filter_map(|d| d.score).collect();
    let overall_score = if known_scores.is_empty() {
        None
    } else {
        Some(known_scores.iter().sum::<i64>() / known_scores.len() as i64)
    };

    let domains_monitored = dns_posture.len() as i64;
    let last_scan_at = dns_obs
        .first()
        .map(|o| o.ts_utc.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into());

    let warning_findings_state = DataState::from_result(
        store
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
            .await,
        "list_warnings (security findings) failed (web security page)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&warning_findings_state, &mut degraded_notice);
    let warning_findings = warning_findings_state.into_items();

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
                    .and_then(|urls| urls.into_iter().find(|u| !u.is_empty()))
                    .map(|url| safe_href(&url))
                    .filter(|url| url != "#"),
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

    // Security-source negative-state taxonomy. The shared loader keeps this
    // page and /api/security in lockstep.
    let source_states = security_source_badges(
        &crate::routes::security::load_security_source_statuses(&store, cve_findings).await,
    );

    let tpl = SecurityPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        status_strip: crate::system_status::StatusStrip::current(),
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
        source_states,
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
        degraded_notice,
    };

    let _ = is_htmx_request(&headers);
    super::render_template(&tpl)
}

// ─── Trigger scan ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TriggerScanForm {
    pub job_kind: Option<String>,
}

/// POST /security/trigger-scan — trigger an on-demand security crawl job
/// (e.g. dns_posture_scan, lookalike_domain_scan) and return an HTMX HTML
/// fragment showing the result status.
pub async fn post_trigger_scan_html(
    Extension(store): Extension<Arc<PgStore>>,
    Form(form): Form<TriggerScanForm>,
) -> impl IntoResponse {
    let requested_kind = form
        .job_kind
        .unwrap_or_else(|| "dns_posture_scan".to_string());

    // Whitelist of security-relevant job kinds that can be triggered from the UI.
    let allowed = [
        "dns_posture_scan",
        "lookalike_domain_scan",
        "kev_catalog_fetch",
    ];
    if !allowed.contains(&requested_kind.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Html(format!(
                "<div class=\"rounded border border-rams-red/30 bg-rams-red/10 px-3 py-2 text-xs font-semibold text-rams-red\">Unknown scan type: {}</div>",
                super::escape_html(&requested_kind)
            )),
        );
    }

    match store.queue_job_trigger(&requested_kind).await {
        Ok(trigger_id) => {
            let short_id = &trigger_id[..trigger_id.len().min(8)];
            (
                StatusCode::ACCEPTED,
                Html(format!(
                    "<div class=\"apex-card p-4 border-rams-green/30 bg-rams-green/5\">\
                     <div class=\"flex items-center gap-2\">\
                     <span class=\"h-2 w-2 rounded-full bg-rams-green animate-pulse\"></span>\
                     <p class=\"text-sm font-bold text-rams-green\">{}</p>\
                     </div>\
                     <p class=\"mt-1 text-[11px] text-rams-green\">Trigger ID: {}</p>\
                     </div>",
                    requested_kind, short_id
                )),
            )
        }
        Err(err) => {
            tracing::error!(job_kind = %requested_kind, "queue_job_trigger failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("<div class=\"rounded border border-rams-red/30 bg-rams-red/10 px-3 py-2 text-xs font-semibold text-rams-red\">Failed to queue security scan</div>".to_string()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::routes::security::SecuritySourceState;

    #[test]
    fn every_security_source_state_renders_distinctly() {
        let states = [
            SecuritySourceState::FindingsReported,
            SecuritySourceState::NoFindingsAfterSuccessfulScan,
            SecuritySourceState::NotScanned,
            SecuritySourceState::ScanFailed,
            SecuritySourceState::AuthenticationUnavailable,
            SecuritySourceState::RateLimited,
            SecuritySourceState::SourceUnavailable,
            SecuritySourceState::PartialScan,
            SecuritySourceState::ScanSucceeded,
        ];
        let mut labels = std::collections::BTreeSet::new();
        for state in states {
            let (label, class) = security_source_badge(state);
            assert!(!label.is_empty());
            assert!(!class.is_empty());
            assert!(
                labels.insert(label),
                "duplicate badge label for {state:?}: {label}"
            );
        }

        // The clean states are never worded as a bare "0 findings".
        let (clean_label, _) =
            security_source_badge(SecuritySourceState::NoFindingsAfterSuccessfulScan);
        assert!(!clean_label.contains("0 findings"));
        let (not_scanned, _) = security_source_badge(SecuritySourceState::NotScanned);
        assert_ne!(not_scanned, clean_label);
    }
}
