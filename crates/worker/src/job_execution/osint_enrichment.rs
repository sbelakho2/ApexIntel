//! OSINT Enrichment worker handler.
//!
//! Runs every 6 hours: invokes structured-source fetchers against tracked
//! companies and domains, producing typed observations with full provenance.
//! This complements the generic RSS/web crawl (`run_crawl_cycle`) with
//! source-specific parsers that extract structured intelligence.
//!
//! # Sources invoked
//! - **SEC EDGAR** — public-company filings (8-K, 10-K, DEF 14A, Form 3/4/5)
//!   for US-listed companies identified by ticker.
//! - **RDAP** — domain registration data (registrar, dates, abuse contacts).
//! - **OpenAlex** — academic publications mentioning the company / its tech.
//! - **NVD/CVE** — recent vulnerability disclosures (global feed, matched to
//!   company tech stacks where possible).
//! - **DNS posture** — SPF/DKIM/DMARC posture for company domains.
//!
//! Each source degrades gracefully: network errors are logged at `warn!` and
//! the job continues with the remaining sources. No source failure can abort
//! the whole enrichment cycle.

use std::sync::Arc;
use std::time::Instant;

use crate::{JobKind, JobRun, PgStore};

/// Maximum number of companies to enrich per invocation.
/// Set high — every company with a domain should get enriched.
const MAX_COMPANIES_PER_RUN: usize = 500;

/// Load tracked companies, invoke all OSINT source fetchers, store observations.
pub(super) async fn run_osint_enrichment(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    let start = Instant::now();

    // ── 1. Load tracked companies with domain + ticker metadata ────────────
    #[derive(sqlx::FromRow)]
    struct CompanyRow {
        id: uuid::Uuid,
        name: String,
        domain: Option<String>,
        region: Option<String>,
    }

    let companies: Vec<CompanyRow> = match sqlx::query_as::<_, CompanyRow>(
        r#"SELECT id, name, domain, region
             FROM companies
            WHERE domain IS NOT NULL AND TRIM(domain) != ''
            ORDER BY updated_at DESC NULLS LAST
            LIMIT $1"#,
    )
    .bind(MAX_COMPANIES_PER_RUN as i64)
    .fetch_all(&store.pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            run.fail(&format!("osint_enrichment: failed to load companies: {e}"));
            return run;
        }
    };

    if companies.is_empty() {
        run.skip("osint_enrichment: no companies with domains found");
        return run;
    }

    let mut total_observations: u64 = 0;
    let mut companies_enriched: u64 = 0;
    let activity_logger = apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());

    // ── 2. Global CVE fetch (not company-specific) ─────────────────────────
    let cve_client = apex_crawl::cve::CveClient::new();
    match cve_client.fetch_recent(7).await {
        Ok(cves) if !cves.is_empty() => {
            let count = cves.len();
            for cve in &cves {
                let mut obs = cve.to_observation(None);
                // B326: content-derived ID — re-fetching the same CVE every
                // 6h no longer inserts a duplicate row.
                obs.stabilize_id("cve");
                if let Err(e) = store.insert_observation(&obs).await {
                    tracing::warn!(error = %e, "osint_enrichment: failed to insert CVE observation");
                }
            }
            total_observations += count as u64;
            tracing::info!(cve_count = count, "osint_enrichment: fetched recent CVEs");
        }
        Ok(_) => {
            tracing::info!("osint_enrichment: no recent CVEs returned");
        }
        Err(e) => {
            tracing::warn!(error = %e, "osint_enrichment: CVE fetch failed (continuing)");
        }
    }

    // ── 3. Per-company enrichment ──────────────────────────────────────────
    for company in &companies {
        let mut company_obs: u64 = 0;

        // ── 3a. RDAP domain registration lookup ────────────────────────────
        if let Some(ref domain) = company.domain {
            let rdap_client = apex_crawl::rdap::RdapClient::new();
            match rdap_client.lookup(domain).await {
                Ok(Some(record)) => {
                    let mut obs = record.to_observation(Some(company.id));
                    // B326: stable ID per (company, registration data).
                    obs.stabilize_id("rdap");
                    if let Err(e) = store.insert_observation(&obs).await {
                        tracing::warn!(error = %e, "osint_enrichment: RDAP insert failed");
                    } else {
                        company_obs += 1;
                    }
                }
                Ok(None) => {} // domain not found in RDAP (common for private TLDs)
                Err(e) => {
                    tracing::warn!(domain = %domain, error = %e, "osint_enrichment: RDAP lookup failed");
                }
            }
        }

        // ── 3b. OpenAlex academic publications ─────────────────────────────
        let openalex_client = apex_crawl::openalex::OpenAlexClient::new();
        match openalex_client.search_works(&company.name, 5).await {
            Ok(works) if !works.is_empty() => {
                for work in &works {
                    let mut obs = work.to_observation(Some(company.id));
                    // B326: stable ID per (company, work) — the same five
                    // OpenAlex works were re-inserted every 6h.
                    obs.stabilize_id("openalex");
                    if let Err(e) = store.insert_observation(&obs).await {
                        tracing::warn!(error = %e, "osint_enrichment: OpenAlex insert failed");
                    } else {
                        company_obs += 1;
                    }
                }
            }
            Ok(_) => {} // no publications found
            Err(e) => {
                tracing::warn!(company = %company.name, error = %e, "osint_enrichment: OpenAlex search failed");
            }
        }

        // ── 3c. SEC EDGAR filings (if US public company) ───────────────────
        // Look up the ticker from company metadata if available.
        let ticker: Option<String> =
            sqlx::query_scalar(r#"SELECT metadata->>'sec_ticker' FROM companies WHERE id = $1"#)
                .bind(company.id)
                .fetch_optional(&store.pool)
                .await
                .ok()
                .flatten()
                .filter(|t: &String| !t.trim().is_empty());

        if let Some(ref ticker) = ticker {
            let edgar_client = apex_crawl::sec_edgar::SecEdgarClient::new();
            match edgar_client
                .fetch_filings_as_observations(ticker, Some(company.id), 10)
                .await
            {
                Ok(mut filings) if !filings.is_empty() => {
                    let count = filings.len();
                    for filing in filings.iter_mut() {
                        // B326: stable ID per filing accession number.
                        filing.stabilize_id("sec_edgar");
                        if let Err(e) = store.insert_observation(filing).await {
                            tracing::warn!(error = %e, "osint_enrichment: SEC filing insert failed");
                        }
                    }
                    company_obs += count as u64;
                }
                Ok(_) => {} // no tracked filings
                Err(e) => {
                    tracing::warn!(ticker = %ticker, error = %e, "osint_enrichment: SEC EDGAR fetch failed");
                }
            }
        }

        // ── 3d. DNS posture check ──────────────────────────────────────────
        if let Some(ref domain) = company.domain {
            let dns_checker = apex_crawl::dns::DnsChecker::new();
            match dns_checker.check_posture(domain).await {
                Ok(posture) => {
                    let obs = apex_core::entities::Observation::new(
                        apex_core::entities::ObservationType::DnsPosture,
                        chrono::Utc::now(),
                        serde_json::to_value(&posture).unwrap_or(serde_json::Value::Null),
                        serde_json::json!({
                            "source": "dns_posture",
                            "domain": domain,
                        }),
                    );
                    let mut obs = obs;
                    obs.entity_id = Some(company.id);
                    obs.entity_type = Some("company".to_string());
                    // B326: stable ID per (domain, posture snapshot) — a new
                    // row only when the posture actually changes.
                    obs.stabilize_id("dns_posture");
                    if let Err(e) = store.insert_observation(&obs).await {
                        tracing::warn!(error = %e, "osint_enrichment: DNS posture insert failed");
                    } else {
                        company_obs += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(domain = %domain, error = %e, "osint_enrichment: DNS posture check failed");
                }
            }
        }

        if company_obs > 0 {
            total_observations += company_obs;
            companies_enriched += 1;
            tracing::info!(
                company = %company.name,
                observations = company_obs,
                "osint_enrichment: company enriched"
            );
        }
    }

    // ── 4. Log crawl-completion activity event ─────────────────────────────
    activity_logger
        .log_crawl_completed(
            "osint_enrichment",
            companies.len() as u32,
            total_observations as u32,
            start.elapsed().as_secs_f64(),
        )
        .await;

    let elapsed = start.elapsed();
    run.succeed(
        total_observations,
        &format!(
            "osint_enrichment: {} companies loaded, {} enriched, {} observations inserted in {:.1}s",
            companies.len(),
            companies_enriched,
            total_observations,
            elapsed.as_secs_f64(),
        ),
    );
    run
}
