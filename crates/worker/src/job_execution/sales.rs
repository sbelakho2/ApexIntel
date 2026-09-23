//! Sales-activation worker jobs.
//!
//! - `run_contact_enrichment` — discovers verified email/phone/LinkedIn for
//!   persons that lack contact data, via the [`ContactEnricher`] waterfall.
//! - `run_icp_scoring` — batch-scores all non-competitor companies against the
//!   ICP definition and persists fit scores + breakdowns.
//! - `run_engagement_refresh` — recomputes engagement summaries from
//!   `engagement_events` (wires the previously-orphaned engagement tracker).
//! - `run_buying_center_derivation` — auto-derives the buying committee per
//!   account from each person's role, so "which person at the company" is
//!   populated without manual POSTs.
//! - `run_person_mention_materialization` — turns `PersonMention` observations
//!   (e.g. OpenAlex author lists) into real `persons` rows, bridging the gap
//!   between "we observed 2775 people" and "0 new persons were ever created".

use std::sync::Arc;

use apex_core::entities::RoleFamily;
use apex_crawl::contact_enrichment::ContactEnricher;
use apex_insights::icp_scorer::{IcpDefinition, IcpInput, IcpScorer};
use apex_poi::buying_center::{role_to_buying_center, BuyingCenterRole};
use apex_store::postgres::{NewBuyingMember, NewContactMethod, PgStore};

use crate::*;

/// Enrich contact data for persons that have none yet.
///
/// Selects up to `batch` persons with no `contact_methods` rows, looks up their
/// company domain, runs the enrichment waterfall, and persists results.
pub(super) async fn run_contact_enrichment(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let batch: i64 = 50;
    // Find persons lacking any contact method, joining companies for the domain.
    let rows = match sqlx::query_as::<_, (uuid::Uuid, String, Option<String>)>(
        r#"
        SELECT p.id, p.name, c.domain
        FROM persons p
        LEFT JOIN companies c ON c.id = p.primary_org_id
        WHERE p.primary_org_id IS NOT NULL
          AND NOT EXISTS (SELECT 1 FROM contact_methods cm WHERE cm.person_id = p.id)
        ORDER BY p.influence_score DESC NULLS LAST
        LIMIT $1
        "#,
    )
    .bind(batch)
    .fetch_all(&store.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            run.fail(&format!("contact_enrichment: query persons failed: {e}"));
            return run;
        }
    };

    if rows.is_empty() {
        run.skip("contact_enrichment: no persons lacking contacts");
        return run;
    }

    let enricher = ContactEnricher::from_env();
    let mut enriched = 0u64;

    for (person_id, name, domain) in &rows {
        let domain = match domain {
            Some(d) if !d.trim().is_empty() => d.trim(),
            _ => continue,
        };
        match enricher.enrich(name, domain).await {
            Ok(contacts) if !contacts.is_empty() => {
                for c in &contacts {
                    let new_cm = NewContactMethod {
                        person_id: *person_id,
                        contact_type: c.contact_type.clone(),
                        value: c.value.clone(),
                        confidence: c.confidence as f64,
                        verification_status: c.verification_status.clone(),
                        verified_at: None,
                        source: c.source.clone(),
                        is_primary: false,
                        last_seen_at: Some(chrono::Utc::now()),
                        metadata: serde_json::json!({"name": name}),
                    };
                    if let Err(e) = store.upsert_contact_method(&new_cm).await {
                        tracing::warn!(error = %e, "contact_enrichment: upsert failed");
                    }
                }
                enriched += 1;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(error = %e, name = %name, "contact_enrichment: provider error")
            }
        }
    }

    run.succeed(
        enriched,
        &format!(
            "contact_enrichment: checked {} persons, enriched {} with contact data",
            rows.len(),
            enriched
        ),
    );
    run
}

/// Batch-score all non-competitor companies against the ICP definition.
pub(super) async fn run_icp_scoring(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let rows = match sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            Option<i32>,
            Option<i64>,
            Option<Vec<String>>,
            Option<String>,
            Option<String>,
            Option<f64>,
            Option<Vec<String>>,
            Option<f64>,
            Option<String>,
            Option<f64>,
        ),
    >(
        r#"
        SELECT id, employee_estimate, revenue_estimate_usd, industry_tags,
               region, country_code, strategic_relevance,
               tech_stack, intent_signal_score, funding_stage, headcount_growth_pct
        FROM companies
        WHERE is_competitor IS DISTINCT FROM TRUE
        "#,
    )
    .fetch_all(&store.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            run.fail(&format!("icp_scoring: query companies failed: {e}"));
            return run;
        }
    };

    if rows.is_empty() {
        run.skip("icp_scoring: no companies to score");
        return run;
    }

    let def = IcpDefinition::default();
    let mut scored = 0u64;

    for (id, emp, rev, tags, region, country, strategic, tech, intent, funding, growth) in &rows {
        let input = IcpInput {
            employee_estimate: *emp,
            revenue_estimate_usd: *rev,
            industry_tags: tags.clone().unwrap_or_default(),
            tech_stack: tech.clone().unwrap_or_default(),
            region: region.clone(),
            country_code: country.clone(),
            intent_signal_score: intent.unwrap_or(0.0).clamp(0.0, 1.0),
            strategic_relevance: strategic.unwrap_or(0.0),
            funding_stage: funding.clone(),
            headcount_growth_pct: *growth,
        };
        let score = IcpScorer::score(&input, &def);
        let breakdown = serde_json::to_value(&score.components).unwrap_or(serde_json::json!([]));
        if let Err(e) = store
            .update_company_icp_score(
                *id,
                score.icp_fit_score,
                score.intent_signal_score,
                &breakdown,
                tech.as_deref(),
                funding.as_deref(),
                *growth,
            )
            .await
        {
            tracing::warn!(error = %e, "icp_scoring: persist failed");
        } else {
            scored += 1;
        }
    }

    run.succeed(
        scored,
        &format!("icp_scoring: scored {} companies against ICP", scored),
    );
    run
}

/// Recompute engagement summaries from the `engagement_events` history.
///
/// This materializes the outreach-feedback loop: per-person response rates and
/// next-best-channel are derived from real contact outcomes. The summary is
/// stored on the person's engagement profile metadata so the API can surface
/// "this person responds to email 60% of the time" without recomputing per call.
pub(super) async fn run_engagement_refresh(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    let rows = match sqlx::query_as::<_, (uuid::Uuid,)>(
        r#"
        SELECT DISTINCT person_id
        FROM engagement_events
        WHERE occurred_at >= NOW() - INTERVAL '90 days'
        "#,
    )
    .fetch_all(&store.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            run.fail(&format!("engagement_refresh: query failed: {e}"));
            return run;
        }
    };

    if rows.is_empty() {
        run.skip("engagement_refresh: no recent engagement events");
        return run;
    }

    let mut updated = 0u64;
    for (person_id,) in &rows {
        // Aggregate real outcome counts + response rate per person.
        let summary = match sqlx::query_as::<_, (i64, i64, Option<f64>)>(
            r#"
            SELECT
                COUNT(*) AS total,
                COUNT(*) FILTER (WHERE outcome IN ('positive','reply','meeting_booked')) AS responses,
                AVG(outcome_weight) AS avg_weight
            FROM engagement_events
            WHERE person_id = $1 AND occurred_at >= NOW() - INTERVAL '90 days'
            "#,
        )
        .bind(person_id)
        .fetch_one(&store.pool)
        .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "engagement_refresh: aggregate failed");
                continue;
            }
        };

        let (total, responses, avg_weight) = summary;
        let response_rate = if total > 0 {
            responses as f64 / total as f64
        } else {
            0.0
        };

        let meta = serde_json::json!({
            "engagement_summary": {
                "total_contacts": total,
                "responses": responses,
                "response_rate": response_rate,
                "avg_outcome_weight": avg_weight,
            }
        });

        // Persist the summary onto the person's engagement profile, creating one
        // only when none exists. Both statements are checked: a silent failure
        // previously reported success while storing nothing.
        let update = sqlx::query(
            "UPDATE engagement_profiles SET metadata = $2, updated_at = NOW() WHERE person_id = $1",
        )
        .bind(person_id)
        .bind(&meta)
        .execute(&store.pool)
        .await;

        let persisted = match update {
            Ok(result) if result.rows_affected() > 0 => Ok(()),
            Ok(_) => sqlx::query(
                r#"
                INSERT INTO engagement_profiles
                    (person_id, talking_points, opening_topics, avoid_topics, best_channel, best_timing, proof_pack, metadata)
                VALUES ($1, ARRAY[]::TEXT[], ARRAY[]::TEXT[], ARRAY[]::TEXT[], 'email', NULL, '{}'::JSONB, $2)
                "#,
            )
            .bind(person_id)
            .bind(&meta)
            .execute(&store.pool)
            .await
            .map(|_| ()),
            Err(e) => Err(e),
        };

        match persisted {
            Ok(()) => updated += 1,
            Err(e) => {
                tracing::warn!(error = %e, person = %person_id, "engagement_refresh: persist failed")
            }
        }
    }

    run.succeed(
        updated,
        &format!(
            "engagement_refresh: updated {} person engagement summaries",
            updated
        ),
    );
    run
}

// ════════════════════════════════════════════════════════════════════════════
// Buying-center auto-derivation (R6 fix)
// ════════════════════════════════════════════════════════════════════════════

/// Map the academic [`BuyingCenterRole`] to the SaaS-canonical role string
/// stored in `buying_center_members.role` (champion / economic_buyer / …).
fn buying_role_to_saas(role: &BuyingCenterRole) -> &'static str {
    match role {
        BuyingCenterRole::Initiator => "influencer",
        BuyingCenterRole::Decider => "decision_maker",
        BuyingCenterRole::Buyer => "economic_buyer",
        BuyingCenterRole::Influencer => "influencer",
        BuyingCenterRole::User => "user",
        BuyingCenterRole::Gatekeeper => "gatekeeper",
    }
}

/// Auto-derive the buying committee for each account from the roles of the
/// persons already tracked at that company. Previously buying centers were
/// only populated via manual `POST /api/companies/:id/buying-center/members`,
/// so the "which person at the company" layer was always empty. This job runs
/// the existing `role_to_buying_center` classifier over every person and
/// persists the result, so every account gets a populated committee.
#[cfg(feature = "llm")]
pub(super) async fn run_buying_center_derivation(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    // All persons with a known company + role_family.
    let rows: Vec<(uuid::Uuid, String, Option<String>, Option<uuid::Uuid>)> = match sqlx::query_as(
        "SELECT id, name, role_family, primary_org_id FROM persons \
         WHERE primary_org_id IS NOT NULL AND role_family IS NOT NULL AND role_family <> ''",
    )
    .fetch_all(&store.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            run.fail(&format!(
                "buying_center_derivation: query persons failed: {e}"
            ));
            return run;
        }
    };

    if rows.is_empty() {
        run.skip("buying_center_derivation: no persons with org+role");
        return run;
    }

    let mut members_added = 0u64;
    let mut companies_covered: std::collections::HashSet<uuid::Uuid> =
        std::collections::HashSet::new();
    let mut center_by_company: std::collections::HashMap<uuid::Uuid, uuid::Uuid> =
        std::collections::HashMap::new();

    for (person_id, name, role_family_str, org_id) in &rows {
        let org_id = match org_id {
            Some(o) => *o,
            None => continue,
        };
        let rf = RoleFamily::from_str(role_family_str.as_deref().unwrap_or("other"));
        let bc_role = role_to_buying_center(&rf);
        let saas_role = buying_role_to_saas(&bc_role);
        let influence = bc_role.influence_score();

        // Ensure exactly one buying center exists per company (idempotent).
        let bc_id = match center_by_company.get(&org_id).copied() {
            Some(id) => id,
            None => match store
                .upsert_buying_center(org_id, None, "Auto-derived Buying Center", None)
                .await
            {
                Ok(id) => {
                    center_by_company.insert(org_id, id);
                    id
                }
                Err(e) => {
                    tracing::warn!(error = %e, person = %name, "buying_center_derivation: upsert center failed");
                    continue;
                }
            },
        };

        let member = NewBuyingMember {
            buying_center_id: bc_id,
            person_id: *person_id,
            role: saas_role.to_string(),
            influence_score: influence,
            budget_authority: matches!(
                bc_role,
                BuyingCenterRole::Decider | BuyingCenterRole::Buyer
            ),
            need_signal: 0.0,
            timeline_horizon: None,
            notes: Some(format!("Auto-derived from role_family: {}", rf.as_str())),
            metadata: serde_json::json!({"source": "auto_derivation", "role_family": rf.as_str()}),
        };
        match store.upsert_buying_center_member(&member).await {
            Ok(_) => {
                members_added += 1;
                companies_covered.insert(org_id);
            }
            Err(e) => {
                tracing::warn!(error = %e, person = %name, "buying_center_derivation: member upsert failed")
            }
        }
    }

    run.succeed(
        members_added,
        &format!(
            "buying_center_derivation: {} members across {} companies",
            members_added,
            companies_covered.len()
        ),
    );
    run
}

#[cfg(not(feature = "llm"))]
pub(super) async fn run_buying_center_derivation(kind: &JobKind, _store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.skip("buying_center_derivation: requires the `llm` feature (apex-poi)");
    run
}

// ════════════════════════════════════════════════════════════════════════════
// PersonMention → persons materialization (R2 fix)
// ════════════════════════════════════════════════════════════════════════════

/// Materialize real people from `PersonMention` observations into the `persons`
/// table. Each PersonMention (e.g. from OpenAlex author lists, news mentions,
/// social posts) carries named individuals in its `value` JSON. Previously
/// thousands of these observations existed but **no code path ever created a
/// `persons` row from them** — the authors were effectively dead data.
///
/// This job extracts author/mention names, dedups against existing persons, and
/// inserts the new ones linked to the company the observation was about.
pub(super) async fn run_person_mention_materialization(
    kind: &JobKind,
    store: &Arc<PgStore>,
) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();

    // Pull PersonMention observations with the entity (company) they relate to.
    let rows: Vec<(uuid::Uuid, serde_json::Value, Option<uuid::Uuid>)> = match sqlx::query_as(
        "SELECT id, value, entity_id FROM observations \
         WHERE observation_type = 'PersonMention' \
           AND ts_utc > NOW() - INTERVAL '90 days' \
         ORDER BY ts_utc DESC LIMIT 5000",
    )
    .fetch_all(&store.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            run.fail(&format!(
                "person_mention_materialization: query failed: {e}"
            ));
            return run;
        }
    };

    if rows.is_empty() {
        run.skip("person_mention_materialization: no PersonMention observations");
        return run;
    }

    let mut created = 0u64;
    let mut skipped_dup = 0u64;

    for (obs_id, value, company_id) in &rows {
        // Extract candidate names from the observation JSON.
        // OpenAlex stores authors under value.authors[]; other sources may use
        // value.person or value.mentions[]. Be tolerant of shape.
        let names = extract_person_names(value);
        if names.is_empty() {
            continue;
        }

        for name in names {
            let clean = clean_person_name(&name);
            if clean.len() < 4 || clean.split_whitespace().count() < 2 {
                continue; // need at least "First Last"
            }

            // Dedup against existing persons by normalized name + org.
            let name_key = normalize_name_key(&clean);
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM persons WHERE lower(regexp_replace(name, '\\s+', ' ', 'g')) = lower($1))",
            )
            .bind(&name_key)
            .fetch_one(&store.pool)
            .await
            .unwrap_or(true); // on error, assume exists to avoid dup risk
            if exists {
                skipped_dup += 1;
                continue;
            }

            // Insert the new person. role_family is unknown until enrichment;
            // the PoiRoleReclassify job will classify them later from any bio.
            let res = sqlx::query(
                "INSERT INTO persons (name, primary_org_id, region, public_bio, influence_score, role_family, metadata) \
                 VALUES ($1, $2, NULL, $3, 0.1, 'Other', $4) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(&clean)
            .bind(company_id)
            .bind(format!("Mentioned in observation {}", obs_id))
            .bind(serde_json::json!({"source": "person_mention_materialization", "observation_id": obs_id.to_string()}))
            .execute(&store.pool)
            .await;
            match res {
                Ok(r) if r.rows_affected() > 0 => created += 1,
                Ok(_) => skipped_dup += 1,
                Err(e) => {
                    tracing::debug!(error = %e, name = %clean, "person_mention: insert failed")
                }
            }
        }
    }

    run.succeed(
        created,
        &format!(
            "person_mention_materialization: {} observations → {} persons created, {} duplicates skipped",
            rows.len(),
            created,
            skipped_dup
        ),
    );
    run
}

/// Extract person names from a PersonMention observation's JSON value.
fn extract_person_names(value: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    // OpenAlex: value.authors = ["Name1", "Name2", ...]
    if let Some(authors) = value.get("authors").and_then(|a| a.as_array()) {
        for a in authors {
            if let Some(n) = a.as_str().filter(|s| !s.is_empty()) {
                names.push(n.to_string());
            }
        }
    }
    // Generic: value.person or value.name
    if names.is_empty() {
        if let Some(n) = value
            .get("person")
            .or_else(|| value.get("name"))
            .and_then(|v| v.as_str())
        {
            names.push(n.to_string());
        }
    }
    // Generic: value.mentions = [{"name": "..."}, ...]
    if names.is_empty() {
        if let Some(mentions) = value.get("mentions").and_then(|m| m.as_array()) {
            for m in mentions {
                if let Some(n) = m.get("name").and_then(|v| v.as_str()) {
                    names.push(n.to_string());
                }
            }
        }
    }
    names
}

/// Strip titles/affiliations OpenAlex sometimes appends, leaving a clean name.
fn clean_person_name(raw: &str) -> String {
    raw.split(',')
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != ' ' && c != '-' && c != '\'')
        .to_string()
}

/// Normalize a name for dedup: lowercase, collapse whitespace.
fn normalize_name_key(name: &str) -> String {
    name.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
