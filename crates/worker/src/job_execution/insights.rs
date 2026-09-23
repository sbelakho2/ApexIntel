//! Dedicated InsightGeneration worker handler.
//!
//! Runs every 6 hours: loads recent observations (rolling 7-day window),
//! groups by company, and generates **grounded LLM insights** by routing
//! through the canonical `generate_llm_insight()` pipeline (the same one
//! `RecipeFire` uses nightly). Each insight carries real evidence signals
//! with `[1][2]` citations, real POI contact recommendations via the buying
//! center model, and is validated against the anti-hallucination grounding
//! layer before storage.
//!
//! Previously this job used `DeepInsightGenerator` — a pure string-template
//! function with NO LLM call, NO grounding, and a broken dedup — which
//! produced formulaic "Intelligence Analysis: {co} (N observations)" noise
//! with empty evidence URLs. That path has been removed.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration, Utc};

use crate::{JobKind, JobRun, PgStore};

#[cfg(feature = "llm")]
use {
    crate::{EntityContext, EvidenceSignal, InferenceLlmClient},
    apex_llm::inference::InferenceConfig,
    apex_poi::model::RoleFamily as PoiRoleFamily,
};
/// How far back to look for observations to generate insights from.
/// A rolling 7-day window (vs the old 24h) lets more companies qualify for
/// insight generation and captures weekly-rhythm signals, matching the
/// philosophy of RecipeFire's 30-day evaluation window while staying fresh.
const OBSERVATION_LOOKBACK_HOURS: i64 = 168; // 7 days

/// Maximum number of companies to process per invocation (B358).
///
/// Each company costs one LLM call; on local CPU inference (Qwen3-30B) a
/// call is minutes, not seconds, so the cap must fit the job's declared
/// 2-hour timeout. Override with `LLM_INSIGHT_MAX_COMPANIES`; runs pick up
/// where the last one left off (only companies with fresh observations are
/// loaded), so a lower cap just spreads work across the 6-hour cadence.
static MAX_COMPANIES_PER_RUN: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
    std::env::var("LLM_INSIGHT_MAX_COMPANIES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(10)
});

/// Minimum text length for an observation to be considered for insight generation.
const MIN_OBSERVATION_TEXT_LEN: usize = 40;

/// Maximum evidence signals to feed the LLM (matches RecipeFire's cap).
const MAX_EVIDENCE_SIGNALS: usize = 12;

/// Load recent observations grouped by company, generate insights, store them.
pub(super) async fn run_insight_generation(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    #[cfg(feature = "llm")]
    {
        let total_start = Instant::now();

        // 1. Determine the lookback window
        let since = Utc::now() - Duration::hours(OBSERVATION_LOOKBACK_HOURS);

        // 2. Load recent observations grouped by company
        let company_observations = match load_recent_observations_by_company(store, since).await {
            Ok(data) => data,
            Err(e) => {
                run.fail(&format!(
                    "insight_generation: failed to load observations: {e}"
                ));
                return run;
            }
        };

        if company_observations.is_empty() {
            run.succeed(
                0,
                "insight_generation: no recent observations with sufficient text found",
            );
            return run;
        }

        tracing::info!(
            companies_with_obs = company_observations.len(),
            "insight_generation: loaded recent observations"
        );

        // 3. Process companies (limit to MAX_COMPANIES_PER_RUN, prioritize by observation count)
        let mut sorted: Vec<_> = company_observations.iter().collect();
        sorted.sort_by_key(|a| std::cmp::Reverse(a.1.len()));
        sorted.truncate(*MAX_COMPANIES_PER_RUN);

        let mut total_insights_generated: u64 = 0;
        let mut companies_processed: u64 = 0;
        let mut companies_skipped: u64 = 0;

        for (company_id, observations) in &sorted {
            if observations.is_empty() {
                companies_skipped += 1;
                continue;
            }

            // Get company name for context
            let company_name = match store.get_company(**company_id).await {
                Ok(Some(c)) => c.name,
                Ok(None) | Err(_) => {
                    tracing::warn!(company_id = %company_id, "insight_generation: company not found");
                    companies_skipped += 1;
                    continue;
                }
            };

            // Load POIs associated with this company for stakeholder mapping
            let company_pois = load_company_pois(store, company_id).await;

            match generate_insights_for_company(
                store,
                company_id,
                &company_name,
                observations,
                &company_pois,
            )
            .await
            {
                Ok(count) => {
                    total_insights_generated += count;
                    companies_processed += 1;
                    tracing::info!(
                        company = %company_name,
                        insights_generated = count,
                        "insight_generation: company processed"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        company = %company_name,
                        error = %e,
                        "insight_generation: failed for company"
                    );
                }
            }
        }

        let elapsed = total_start.elapsed();
        run.succeed(
            total_insights_generated,
            &format!(
                "insight_generation: {} companies processed, {} skipped, {} insights generated in {:.1}s",
                companies_processed,
                companies_skipped,
                total_insights_generated,
                elapsed.as_secs_f64(),
            ),
        );
    }

    #[cfg(not(feature = "llm"))]
    {
        run.skip("insight_generation: requires 'llm' feature");
    }

    run
}

#[cfg(feature = "llm")]
/// An observation's text plus its source URL (when the provenance carries one).
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
pub(super) struct ObservationText {
    pub text: String,
    pub url: Option<String>,
}

async fn load_recent_observations_by_company(
    store: &PgStore,
    since: chrono::DateTime<Utc>,
) -> Result<HashMap<uuid::Uuid, Vec<ObservationText>>, sqlx::Error> {
    use sqlx::FromRow;

    #[derive(FromRow)]
    struct ObsCompanyRow {
        company_id: uuid::Uuid,
        content: String,
        url: Option<String>,
    }

    // B335: the observations table has no `content` column — crawled text
    // lives in the JSONB `value` under several keys. The previous query
    // errored at runtime, so LLM insight generation never saw any crawl
    // text at all. B336: carry the provenance URL so stored insights keep
    // verifiable evidence links instead of `[1][2]` markers with no sources.
    // B357: read `observations.entity_id` directly — the previous join went
    // through `observation_entity_graph`, a table with NO writer anywhere in
    // the codebase (0 rows in production), so the join always came back
    // empty and insight generation silently skipped every run.
    let rows = sqlx::query_as::<_, ObsCompanyRow>(
        r#"SELECT DISTINCT ON (o.id)
                   o.entity_id AS company_id,
                   COALESCE(o.value->>'content', o.value->>'body', o.value->>'body_excerpt',
                            o.value->>'text', o.value->>'description', o.value->>'title', '') AS content,
                   o.provenance->>'url' AS url
            FROM observations o
            WHERE o.created_at >= $1
              AND o.entity_id IS NOT NULL
              AND o.entity_type = 'company'
              AND LENGTH(COALESCE(o.value->>'content', o.value->>'body', o.value->>'body_excerpt',
                            o.value->>'text', o.value->>'description', '')) >= $2
            ORDER BY o.id, o.created_at DESC
            LIMIT 5000"#,
    )
    .bind(since)
    .bind(MIN_OBSERVATION_TEXT_LEN as i32)
    .fetch_all(&store.pool)
    .await?;

    let mut map: HashMap<uuid::Uuid, Vec<ObservationText>> = HashMap::new();
    for row in rows {
        map.entry(row.company_id)
            .or_default()
            .push(ObservationText {
                text: row.content,
                url: row.url.filter(|u| u.starts_with("http")),
            });
    }
    Ok(map)
}

/// A simplified POI reference used during insight generation for stakeholder-aware recommendations.
#[cfg(feature = "llm")]
#[derive(Debug, Clone)]
struct CompanyPoiRef {
    name: String,
    role: String,
    role_family: String,
    org: String,
    is_buyer_relevant: bool,
}

#[cfg(feature = "llm")]
async fn load_company_pois(store: &PgStore, company_id: &uuid::Uuid) -> Vec<CompanyPoiRef> {
    let rows = sqlx::query(
        r#"SELECT
               p.name,
               COALESCE(p.\"current_role\", 'Unknown') AS role,
               COALESCE(p.metadata->>'role_family', 'unknown') AS role_family,
               COALESCE(c.name, p.primary_org_id::text) AS org
           FROM persons p
           LEFT JOIN companies c ON p.primary_org_id = c.id
           WHERE p.primary_org_id = $1
           ORDER BY p.name ASC
           LIMIT 100"#,
    )
    .bind(*company_id)
    .fetch_all(&store.pool)
    .await
    .unwrap_or_default();

    rows.iter()
        .map(|row| {
            use sqlx::Row;
            let name: String = row.try_get("name").unwrap_or_default();
            let role: String = row.try_get("role").unwrap_or_default();
            let role_family: String = row.try_get("role_family").unwrap_or_default();
            let org: String = row.try_get("org").unwrap_or_default();
            let role_lower = role.to_lowercase();
            let family_lower = role_family.to_lowercase();
            let is_buyer_relevant = family_lower.contains("procurement")
                || family_lower.contains("supply")
                || family_lower.contains("quality")
                || family_lower.contains("operations")
                || family_lower.contains("executive")
                || role_lower.contains("buyer")
                || role_lower.contains("purchasing")
                || role_lower.contains("sourcing")
                || role_lower.contains("director")
                || role_lower.contains("vp")
                || role_lower.contains("head")
                || role_lower.contains("manager");

            CompanyPoiRef {
                name,
                role,
                role_family,
                org,
                is_buyer_relevant,
            }
        })
        .collect()
}

/// Build a buying-center-aware contact recommendation block from the POIs
/// attached to a company. Uses `role_classifier::classify_role` to determine
/// each POI's true functional role family (NOT the seeded 'C-Suite' default),
/// then maps it to a buying-center role and prioritizes procurement/supply
/// chain contacts over executives — eliminating the CEO-recommendation bias.
///
/// Returns a markdown block ready to append to an insight's recommendation.
#[cfg(feature = "llm")]
fn build_buying_center_recommendation(
    company_name: &str,
    company_pois: &[CompanyPoiRef],
) -> String {
    if company_pois.is_empty() {
        return format!(
            "\n\n**Recommended contacts at {company_name}:** No tracked personnel with buyer-relevant roles yet. Prioritize discovering the procurement or supply-chain lead before outreach."
        );
    }

    // Classify each POI's real role family from their title (bypassing any
    // stale seeded role_family) and map to a buying-center role.
    let mut ranked: Vec<(
        usize,
        &CompanyPoiRef,
        PoiRoleFamily,
        apex_poi::BuyingCenterRole,
    )> = company_pois
        .iter()
        .enumerate()
        .filter_map(|(idx, poi)| {
            if !poi.is_buyer_relevant {
                return None;
            }
            let family = apex_poi::role_classifier::classify_role(&poi.role);
            let bc = apex_poi::role_to_buying_center(&family);
            Some((idx, poi, family, bc))
        })
        .collect();

    // Sort by buying-center priority (Buyer=1 first, Decider=5 last).
    ranked.sort_by_key(|(_, _, _, bc)| bc.priority());

    if ranked.is_empty() {
        return format!(
            "\n\n**Recommended contacts at {company_name}:** No buyer-relevant personnel currently tracked. Enrich POI discovery to find procurement, supply-chain, or quality contacts."
        );
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "**Recommended contacts at {company_name}** (prioritized by buying-center role):"
    ));
    for (_, poi, family, bc) in ranked.iter().take(3) {
        let family_label = match family {
            PoiRoleFamily::Procurement => "Procurement",
            PoiRoleFamily::SupplierQuality | PoiRoleFamily::Quality => "Quality",
            PoiRoleFamily::Engineering => "Engineering",
            PoiRoleFamily::Operations => "Operations",
            PoiRoleFamily::Security => "Security",
            PoiRoleFamily::Finance => "Finance",
            PoiRoleFamily::Legal => "Legal",
            PoiRoleFamily::Government => "Government",
            PoiRoleFamily::Executive => "Executive",
            PoiRoleFamily::Logistics | PoiRoleFamily::PortLogistics => "Logistics",
            other => return_type_label(other),
        };
        lines.push(format!(
            "- **{name}** ({bc_label}, priority {prio}) — {role} · {family} · {org}. {strategy}",
            name = poi.name,
            bc_label = bc.label(),
            prio = bc.priority(),
            role = poi.role,
            family = family_label,
            org = poi.org,
            strategy = bc.outreach_strategy(),
        ));
    }
    format!("\n\n{}", lines.join("\n"))
}

/// Render an arbitrary RoleFamily variant as a short label.
#[cfg(feature = "llm")]
fn return_type_label(family: &PoiRoleFamily) -> &'static str {
    match family {
        PoiRoleFamily::FreeZoneAuthority => "Free Zone Authority",
        PoiRoleFamily::CertificationBody => "Certification Body",
        PoiRoleFamily::IndustryAssociation => "Industry Association",
        PoiRoleFamily::Distributor => "Distributor",
        PoiRoleFamily::Military => "Military",
        PoiRoleFamily::Intelligence => "Intelligence",
        _ => "Other",
    }
}

/// Generate a single grounded LLM insight for a company from its recent
/// observations. Routes through the canonical `generate_llm_insight()`
/// pipeline (shared with RecipeFire) so every insight carries real evidence
/// citations `[1][2]`, real company context, and real POI contact data.
///
/// This replaces the previous templated `DeepInsightGenerator` path which
/// produced formulaic, un-grounded, de-duplication-broken output.
#[cfg(feature = "llm")]
async fn generate_insights_for_company(
    store: &PgStore,
    company_id: &uuid::Uuid,
    company_name: &str,
    observations: &[ObservationText],
    company_pois: &[CompanyPoiRef],
) -> Result<u64, String> {
    use sqlx::Row;

    // Plain-text view of the corpus (existing keyword logic unchanged).
    let observation_texts: Vec<String> = observations.iter().map(|o| o.text.clone()).collect();

    // ── 1. Build the LLM client (same construction as RecipeFire) ──────────
    let llm_client = {
        let base_url =
            std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".into());
        let api_key = std::env::var("LLM_API_KEY").ok();
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
        let mut config = InferenceConfig::default();
        config.model = model;
        config.max_tokens = 2048;
        config.timeout = crate::config::llm_timeout();
        InferenceLlmClient::new(base_url, api_key, config)
    };

    // ── 2. Build the EntityContext from company + POI data ─────────────────
    let company_row = sqlx::query(
        r#"SELECT region, country_code, industry_tags, revenue_estimate_usd,
                  employee_estimate, threat_score, overlap_score,
                  strategic_relevance, is_competitor, domain
             FROM companies WHERE id = $1"#,
    )
    .bind(*company_id)
    .fetch_optional(&store.pool)
    .await
    .map_err(|e| format!("db error loading company context: {e}"))?;

    let (region, industry_tags, key_persons, is_competitor) = if let Some(ref row) = company_row {
        let region: String = row
            .try_get::<Option<String>, _>("region")
            .ok()
            .flatten()
            .unwrap_or_else(|| "Unknown".into());
        let industry_tags: Vec<String> = row
            .try_get::<Option<Vec<String>>, _>("industry_tags")
            .ok()
            .flatten()
            .unwrap_or_default();
        let is_competitor: bool = row
            .try_get::<Option<bool>, _>("is_competitor")
            .ok()
            .flatten()
            .unwrap_or(false);
        // Build key-persons strings: "Name — Role (role_family)" — real POI
        // names/titles, so the LLM can recommend the RIGHT contact instead of
        // defaulting to "contact the CEO".
        let key_persons: Vec<String> = company_pois
            .iter()
            .take(6)
            .map(|p| {
                if p.role_family.is_empty() || p.role_family == "unknown" {
                    format!("{} — {}", p.name, p.role)
                } else {
                    format!("{} — {} ({})", p.name, p.role, p.role_family)
                }
            })
            .collect();
        (region, industry_tags, key_persons, is_competitor)
    } else {
        ("Unknown".to_string(), Vec::new(), Vec::new(), false)
    };

    let entity_ctx = EntityContext {
        name: company_name.to_string(),
        region: region.clone(),
        entity_type: Some(if is_competitor {
            "competitor".to_string()
        } else {
            "company".to_string()
        }),
        is_competitor,
        industry_tags,
        certifications: Vec::new(),
        capabilities: Vec::new(),
        key_persons,
        recent_changes: Vec::new(),
        threat_score: company_row
            .as_ref()
            .and_then(|r| r.try_get::<Option<f64>, _>("threat_score").ok())
            .flatten(),
        overlap_score: company_row
            .as_ref()
            .and_then(|r| r.try_get::<Option<f64>, _>("overlap_score").ok())
            .flatten(),
        strategic_relevance: company_row
            .as_ref()
            .and_then(|r| r.try_get::<Option<f64>, _>("strategic_relevance").ok())
            .flatten(),
        revenue_estimate_usd: company_row
            .as_ref()
            .and_then(|r| r.try_get::<Option<i64>, _>("revenue_estimate_usd").ok())
            .flatten(),
        employee_estimate: company_row
            .as_ref()
            .and_then(|r| r.try_get::<Option<i32>, _>("employee_estimate").ok())
            .flatten(),
        competitor_names: Vec::new(),
        sites_summary: Vec::new(),
        competitor_events: Vec::new(),
        domain: company_row
            .as_ref()
            .and_then(|r| r.try_get::<Option<String>, _>("domain").ok())
            .flatten(),
    };

    // ── 3. Build EvidenceSignals from the raw observations ─────────────────
    let mut evidence_signals: Vec<EvidenceSignal> = observations
        .iter()
        .take(MAX_EVIDENCE_SIGNALS)
        .enumerate()
        .map(|(i, obs)| EvidenceSignal {
            title: format!(
                "Observation {}: {}",
                i + 1,
                truncate_for_title(&obs.text, 120)
            ),
            description: obs.text.clone(),
            // B336: real provenance URL when available — insights previously
            // shipped citation markers with zero sources.
            source_url: obs.url.clone().unwrap_or_default(),
            signal_type: "observation".to_string(),
            extracted_facts: Vec::new(),
            date_context: None,
            relevance_score: 0.7,
        })
        .collect();

    // Inject POI stakeholder evidence so the LLM grounds its contact
    // recommendations in REAL tracked personnel, not invented names.
    for poi in company_pois.iter().filter(|p| p.is_buyer_relevant).take(4) {
        evidence_signals.push(EvidenceSignal {
            title: format!("Known stakeholder: {} — {} at {}", poi.name, poi.role, poi.org),
            description: format!(
                "{} holds the role '{}' at {} (role family: {}). This is a tracked contact relevant to procurement/supply-chain engagement.",
                poi.name, poi.role, poi.org, poi.role_family
            ),
            source_url: "person_record".to_string(),
            signal_type: "poi".to_string(),
            extracted_facts: vec![
                format!("Name: {}", poi.name),
                format!("Role: {}", poi.role),
                format!("Role family: {}", poi.role_family),
            ],
            date_context: None,
            relevance_score: 0.9,
        });
    }

    if evidence_signals.is_empty() {
        return Ok(0);
    }

    // ── 4. Pick the best-fit category from the observation corpus ─────────
    let category = infer_best_category(&observation_texts);

    // ── 5. Call the real grounded LLM insight pipeline ─────────────────────
    let (headline, narrative, recommendation, llm_confidence, insight_metadata) =
        match crate::generate_llm_insight(&llm_client, &entity_ctx, &category, &evidence_signals)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(
                    company = %company_name,
                    category = %category,
                    error = %e,
                    "insight_generation: LLM generation failed; skipping (no template fallback)"
                );
                return Ok(0);
            }
        };

    // ── 6. Anti-hallucination grounding validation ────────────────────────
    // Verify that the generated narrative's claims are grounded in the
    // evidence signals. Build a SourceGroundingValidator from the evidence
    // and check that cited entities appear in the source material.
    let grounding_ratio = validate_grounding_with_entity(
        &narrative,
        &recommendation,
        &evidence_signals,
        company_name,
    );
    if grounding_ratio < 0.3 {
        tracing::warn!(
            company = %company_name,
            grounding_ratio,
            "insight_generation: grounding validation failed (<0.3); skipping ungrounded insight"
        );
        return Ok(0);
    }

    // ── 7. Append buying-center-aware contact recommendations ─────────────
    let contact_block = build_buying_center_recommendation(company_name, company_pois);
    let full_recommendation = format!("{recommendation}{contact_block}");

    // ── 8. Assemble + store via insert_insight (has built-in dedup) ────────
    let summary = format!("{narrative}\n\n{full_recommendation}");
    let evidence_urls: Vec<String> = evidence_signals
        .iter()
        .filter_map(|s| {
            if s.source_url.starts_with("http") {
                Some(s.source_url.clone())
            } else {
                None
            }
        })
        .take(8)
        .collect();
    let entity_ids = vec![*company_id];
    let tags = vec![category.clone(), "insight_generation".to_string()];

    match store
        .insert_insight(
            &headline,
            &summary,
            Some(&category),
            Some(&region),
            Some(llm_confidence),
            if evidence_urls.is_empty() {
                None
            } else {
                Some(evidence_urls)
            },
            Some(entity_ids),
            Some(tags),
            Some(insight_metadata),
        )
        .await
    {
        Ok(insight_id) => {
            // Link insight → company and insight → relevant POIs.
            if let Err(e) = store
                .link_insight_to_entity(insight_id, company_id, "company")
                .await
            {
                tracing::warn!(
                    insight_id = %insight_id,
                    company_id = %company_id,
                    error = %e,
                    "insight_generation: failed to link insight to company"
                );
            }
            for poi in company_pois.iter().filter(|p| p.is_buyer_relevant).take(5) {
                if let Ok(Some(row)) = sqlx::query(
                    "SELECT id FROM persons WHERE primary_org_id = $1 AND name ILIKE $2 LIMIT 1",
                )
                .bind(*company_id)
                .bind(&poi.name)
                .fetch_optional(&store.pool)
                .await
                {
                    if let Ok(person_id) = row.try_get::<uuid::Uuid, _>("id") {
                        let _ = store
                            .link_insight_to_entity(insight_id, &person_id, "person")
                            .await;
                    }
                }
            }

            // Surface the generated insight in the activity feed.
            let activity_logger =
                apex_worker::activity_logger::ActivityLogger::new(store.pool.clone());
            activity_logger
                .log_insight_generated(
                    company_name,
                    &headline,
                    llm_confidence,
                    &category,
                    Some(&insight_id.to_string()),
                )
                .await;

            tracing::info!(
                company = %company_name,
                category = %category,
                grounding_ratio,
                confidence = llm_confidence,
                "insight_generation: grounded insight generated and stored"
            );
            Ok(1)
        }
        Err(e) => {
            tracing::warn!(
                company = %company_name,
                headline = %headline,
                error = %e,
                "insight_generation: failed to store insight"
            );
            Ok(0)
        }
    }
}

/// Truncate a string to `max` chars for use as a title, adding an ellipsis.
#[cfg(feature = "llm")]
fn truncate_for_title(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Infer the best-fit insight category from the observation corpus so the
/// LLM gets the right per-category prompting guidance.
#[cfg(feature = "llm")]
fn infer_best_category(observations: &[String]) -> String {
    let corpus: String = observations.join(" ").to_lowercase();
    let mut scores: Vec<(&str, usize)> = Vec::new();

    let count = |needle: &str| corpus.matches(needle).count();
    let score = |needles: &[&str]| needles.iter().map(|n| count(n)).sum::<usize>();

    // Use precise compound terms to avoid false positives. Generic words like
    // "capacity", "expansion", "supply", "chain", "product" appear in academic
    // / governance text and must NOT trigger procurement/supply categories.
    scores.push((
        "supply_chain_risk",
        score(&[
            "supply chain",
            "shortage",
            "lead time",
            "supplier disruption",
            "sourcing risk",
            "procurement delay",
        ]),
    ));
    scores.push((
        "demand_procurement",
        score(&[
            "contract award",
            "rfp",
            "tender",
            "purchase order",
            "procurement signal",
            "rfq",
            "bidding",
        ]),
    ));
    scores.push((
        "competitor_market",
        score(&[
            "competitor",
            "market share",
            "product launch",
            "win loss",
            "competitive landscape",
        ]),
    ));
    scores.push((
        "security_compliance",
        score(&[
            "data breach",
            "cyber",
            "vulnerab",
            "compliance",
            "audit",
            "certif",
            "iso ",
            "security incident",
        ]),
    ));
    scores.push((
        "geopolitical_analysis",
        score(&[
            "sanction",
            "tariff",
            "export control",
            "geopolitical",
            "embargo",
            "trade war",
            "government",
            "ministry",
            "policy",
            "regime",
            "institutional",
            "governance",
            "neoliberal",
            "urban",
            "sovereign",
        ]),
    ));
    scores.push((
        "regulatory_policy",
        score(&[
            "regulation",
            "directive",
            "mandate",
            "restriction",
            "legislation",
            "compliance framework",
        ]),
    ));

    scores.sort_by_key(|a| std::cmp::Reverse(a.1));
    if scores.first().map(|(_, s)| *s).unwrap_or(0) == 0 {
        "competitor_market".to_string() // neutral default — avoids forcing
                                        // academic/institutional content into procurement
    } else {
        scores[0].0.to_string()
    }
}

/// Validate that an LLM-generated narrative + recommendation are grounded in
/// the provided evidence signals. Returns a grounding ratio (0.0–1.0): the
/// fraction of evidence-grounded claims vs. total checks.
///
/// Two layers:
/// 1. **Word-presence grounding** — checks that substantive words in the output
///    appear in the source evidence (via `SourceGroundingValidator`).
/// 2. **Proper-noun hallucination check** — extracts proper-noun phrases
///    (multi-word capitalized entities, part numbers like "S32K3xx") from the
///    output and verifies each appears in the evidence or entity profile.
///    Invented company names, customer relationships, and part numbers are the
///    most common LLM hallucination; the word-presence layer misses them
///    because individual words ("Siemens", "Munich") happen to appear.
#[cfg(feature = "llm")]
pub(crate) fn validate_grounding(
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
) -> f64 {
    validate_grounding_with_entity(narrative, recommendation, evidence_signals, "")
}

/// Validate grounding with an additional entity-name corpus. The entity being
/// analyzed is always a legitimate mention, so its name (and profile words)
/// must be in the known set — otherwise every insight gets penalized for
/// "hallucinating" the entity it's written about.
#[cfg(feature = "llm")]
pub(crate) fn validate_grounding_with_entity(
    narrative: &str,
    recommendation: &str,
    evidence_signals: &[EvidenceSignal],
    entity_corpus: &str,
) -> f64 {
    use apex_llm::anti_hallucination::SourceGroundingValidator;
    use std::collections::HashSet;

    let mut validator = SourceGroundingValidator::new();

    // Register each evidence signal as a source text.
    for sig in evidence_signals {
        validator.add_source(&sig.description, None);
    }

    // Cross-reference dictionary: real entity names that may appear in output.
    let mut entity_dict: HashSet<String> = HashSet::new();
    for sig in evidence_signals {
        for fact in &sig.extracted_facts {
            if let Some(name) = fact.strip_prefix("Name: ") {
                entity_dict.insert(name.to_lowercase());
            }
        }
    }
    if !entity_dict.is_empty() {
        validator.add_dictionary("entities", entity_dict);
    }

    let combined = format!("{narrative} {recommendation}");
    let words: Vec<&str> = combined
        .split_whitespace()
        .filter(|w| w.len() > 3) // skip short stopwords
        .collect();

    if words.is_empty() {
        return 0.0;
    }

    // Sample-check up to 40 substantive words for grounding.
    let sample_size = words.len().min(40);
    let mut grounded = 0usize;
    for word in words.iter().take(sample_size) {
        let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric());
        if cleaned.is_empty() {
            continue;
        }
        let result = validator.validate_field("claim", cleaned);
        if apex_llm::anti_hallucination::SourceGroundingValidator::is_grounded(&result) {
            grounded += 1;
        }
    }
    let word_ratio = grounded as f64 / sample_size as f64;

    // ── Layer 2: proper-noun hallucination detection ──────────────────────
    // Build the set of ALL known proper nouns from evidence + entity context.
    // This is the ground truth: any proper noun in the output that isn't here
    // is a fabrication. The entity_corpus includes the entity name and profile
    // so legitimate mentions of the analyzed entity don't get penalized.
    let known_corpus: String = {
        let evidence_text: String = evidence_signals
            .iter()
            .map(|s| format!("{} {}", s.title, s.description))
            .collect::<Vec<_>>()
            .join(" ");
        if entity_corpus.is_empty() {
            evidence_text
        } else {
            format!("{} {}", entity_corpus, evidence_text)
        }
    };

    let known_proper_nouns = extract_known_proper_nouns(&known_corpus);
    let output_proper_nouns = extract_proper_noun_phrases(&combined);

    // Count how many output proper-noun phrases are NOT supported by evidence.
    let mut unsupported_count = 0usize;
    let mut supported_count = 0usize;
    for phrase in &output_proper_nouns {
        let lower = phrase.to_lowercase();
        // A phrase is supported if it (or a token overlap) appears in the known set.
        let is_supported = known_proper_nouns
            .iter()
            .any(|k| k == &lower || k.contains(&lower) || lower.contains(k));
        if is_supported {
            supported_count += 1;
        } else {
            unsupported_count += 1;
            tracing::warn!(
                unsupported_phrase = %phrase,
                "insight_grounding: proper-noun phrase not found in evidence (possible hallucination)"
            );
        }
    }

    let total_proper_nouns = supported_count + unsupported_count;
    let proper_noun_ratio = if total_proper_nouns == 0 {
        1.0 // no proper nouns to check — neutral
    } else {
        supported_count as f64 / total_proper_nouns as f64
    };

    // Weighted blend: word-presence (40%) + proper-noun support (60%).
    // The proper-noun check is weighted higher because invented names/figures
    // are the most damaging hallucination type for intelligence credibility.
    word_ratio.mul_add(0.4, proper_noun_ratio * 0.6)
}

/// Extract known proper-noun phrases from the evidence corpus (ground truth).
/// These are sequences of Capitalized words and part-number patterns.
#[cfg(feature = "llm")]
fn extract_known_proper_nouns(corpus: &str) -> Vec<String> {
    let mut nouns = Vec::new();
    for token in corpus.split_whitespace() {
        let clean = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '.');
        if clean.is_empty() {
            continue;
        }
        // Part numbers: alphanumeric with mixed case digits (e.g. S32K3xx, STM32H7)
        let is_part_number = clean.chars().any(|c| c.is_ascii_digit())
            && clean.chars().any(|c| c.is_ascii_alphabetic())
            && clean.len() >= 3;
        // Capitalized words (skip sentence-initial common words handled by caller)
        let is_capitalized = clean
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
        if is_part_number || is_capitalized {
            nouns.push(clean.to_lowercase());
        }
    }
    nouns
}

/// Extract proper-noun phrases from the LLM output that need validation.
/// Captures multi-word capitalized sequences ("Siemens Munich") and part numbers.
#[cfg(feature = "llm")]
fn extract_proper_noun_phrases(text: &str) -> Vec<String> {
    use std::collections::HashSet;
    let mut phrases = Vec::new();
    let mut current_phrase: Vec<String> = Vec::new();
    let common_words: HashSet<&str> = HashSet::from([
        "The",
        "A",
        "An",
        "This",
        "That",
        "These",
        "Those",
        "Because",
        "If",
        "When",
        "While",
        "For",
        "With",
        "Without",
        "However",
        "Therefore",
        "Additionally",
        "Moreover",
        "Furthermore",
        "Meanwhile",
        "Although",
        "Since",
        "Unless",
        "Until",
        "Where",
        "Which",
        "Who",
        "What",
        "Why",
        "How",
        "Each",
        "Every",
        "Some",
        "Any",
        "All",
        "No",
        "Not",
        "But",
        "And",
        "Or",
        "In",
        "On",
        "At",
        "By",
        "To",
        "Of",
        "As",
        "Is",
        "Are",
        "Was",
        "Were",
        "Be",
        "Been",
        "Being",
        "Has",
        "Have",
        "Had",
        "Do",
        "Does",
        "Did",
        "Will",
        "Would",
        "Could",
        "Should",
        "May",
        "Might",
        "Can",
        "Must",
        "Shall",
        "It",
        "We",
        "They",
        "He",
        "She",
        "Our",
        "Their",
        "His",
        "Her",
        "Its",
        "Your",
        "About",
        "Into",
        "Through",
        "From",
        "Over",
        "Under",
        "After",
        "Before",
        "During",
        "Between",
        "Starz",
        "Recommend",
        "Contact",
        "Approach",
        "Initiate",
        "Begin",
        "Conduct",
        "Identify",
        "Assess",
        "Review",
        "Establish",
        "Develop",
        // Sentence-initial common words that get capitalized mid-output
        "Three",
        "Four",
        "Five",
        "Six",
        "Seven",
        "Eight",
        "Nine",
        "Ten",
        "One",
        "Two",
        "First",
        "Second",
        "Third",
        "Both",
        "Several",
        "Many",
        "Few",
        "Most",
        "Other",
        "Another",
        "Such",
        "Same",
        "New",
        "Old",
        "Given",
        "Based",
        "According",
        "Following",
        "Despite",
        "Given",
        "Insufficient",
        "Evidence",
        "No",
        "Yes",
        "True",
        "False",
        "Source",
        "Sources",
        "Note",
        "Note:",
        "Based",
        "Tier",
        "Tier-2",
        "Tier-3",
        "Tier-1",
        "PCBA",
        "EMS",
        "BMS",
        "BESS",
        "OEM",
        "OEMs",
        "RFQ",
        "RFQs",
        "BOM",
        "BOMs",
        "ISO",
        "CEO",
        "CFO",
        "CTO",
        "VP",
        "PR",
        "IR",
        "EU",
        "US",
        "UK",
        "North",
        "South",
        "East",
        "West",
        "Central",
        "Open",
        "Close",
        "High",
        "Low",
        "Medium",
        "Recent",
        "Current",
        "Previous",
        "Prior",
        "Next",
        "Last",
    ]);

    for token in text.split_whitespace() {
        let clean = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '.');
        if clean.is_empty() {
            continue;
        }
        let is_capitalized = clean
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
        let is_part_number = clean.chars().any(|c| c.is_ascii_digit())
            && clean.chars().any(|c| c.is_ascii_alphabetic())
            && clean.len() >= 3;

        if (is_capitalized && !common_words.contains(clean)) || is_part_number {
            current_phrase.push(clean.to_string());
        } else {
            // Flush the current phrase
            if !current_phrase.is_empty() {
                let joined = current_phrase.join(" ");
                current_phrase.clear();
                // Keep phrases with substance: multi-word, part number, OR a
                // single long capitalized word that looks like a proper noun
                // (e.g. "Medtronic", "MedImmune", "Nemco"). This catches
                // invented single-word company names that the multi-word
                // filter would otherwise miss.
                let word_count = joined.split_whitespace().count();
                let has_part_number = joined.chars().any(|c| c.is_ascii_digit())
                    && joined.chars().any(|c| c.is_ascii_alphabetic());
                let is_meaningful = joined.len() >= 4
                    && (has_part_number
                        || word_count >= 2
                        || (word_count == 1 && joined.len() >= 5));
                if is_meaningful {
                    phrases.push(joined.to_lowercase());
                }
            }
        }
    }
    // Flush trailing phrase
    if !current_phrase.is_empty() {
        let joined = current_phrase.join(" ");
        let word_count = joined.split_whitespace().count();
        let is_meaningful =
            joined.len() >= 4 && (word_count >= 2 || (word_count == 1 && joined.len() >= 5));
        if is_meaningful {
            phrases.push(joined.to_lowercase());
        }
    }
    phrases.sort();
    phrases.dedup();
    phrases
}
