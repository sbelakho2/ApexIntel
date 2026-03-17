use std::sync::Arc;

#[cfg(feature = "llm")]
use std::collections::HashSet;

#[cfg(feature = "llm")]
use apex_core::env::parse_truthy_flag;

use crate::*;

pub(super) async fn run_poi_refresh(kind: &JobKind, store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(kind.clone());
    run.start();
    #[cfg(feature = "llm")]
    {
        let filters = PersonListFilters {
            regions: vec![],
            roles: vec![],
            search: None,
            min_priority: None,
            max_priority: None,
        };
        let persons = match store
            .list_persons(&filters, Some(PersonOrderBy::Priority), true, 200, 0)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                run.fail(&format!("poi_refresh: failed to load persons: {e}"));
                return run;
            }
        };

        if persons.is_empty() {
            run.skip("poi_refresh: no persons in database");
            return run;
        }

        let now_utc = Utc::now().timestamp();
        let mut refreshed: u64 = 0;
        let mut unchanged: u64 = 0;
        let mut enriched_pois: u64 = 0;
        let mut role_history_backfilled: u64 = 0;

        for row in &persons {
            let mut profile = PoiProfile {
                person_id: row.id.to_string(),
                name: row.name.clone(),
                name_variants: vec![],
                org: row.organization.clone(),
                org_id: None,
                current_role: row.role.clone(),
                role_family: RoleFamily::Other(row.role.clone()),
                region: row.region.clone(),
                country_code: String::new(),
                public_bio: String::new(),
                public_email: None,
                artifacts: vec![],
                priority_vector: PoiPriorityVector::zero(),
                psychological: PsychProfile::default_profile(),
                influence: InfluenceProfile {
                    influence_score: row.priority_score,
                    graph_centrality: 0.0,
                    public_recurrence: 0.0,
                    role_seniority_score: 0.0,
                    network_size: 0,
                },
                engagement: None,
                role_history: vec![],
                last_updated_utc: 0,
                profile_completeness: 0.0,
            };

            let report = refresh_profile(&mut profile, now_utc);
            if !report.fields_updated.is_empty() || report.role_changed {
                refreshed += 1;
                tracing::debug!(
                    person = %row.name,
                    completeness = profile.profile_completeness,
                    fields = ?report.fields_updated,
                    "poi_refresh: profile updated"
                );
            } else {
                unchanged += 1;
            }
            if (profile.influence.influence_score - row.priority_score).abs() > 1e-6 {
                if let Err(e) = store
                    .update_person_influence_score(row.id, profile.influence.influence_score)
                    .await
                {
                    tracing::warn!(
                        person = %row.name,
                        error = %e,
                        "poi_refresh: failed to write-back influence score"
                    );
                }
            }
        }

        #[derive(sqlx::FromRow)]
        struct MissingRoleHistoryRow {
            id: Uuid,
            org_id: Option<Uuid>,
            org_name: String,
            current_role: String,
            role_family: String,
        }

        let role_history_backfill_limit = std::env::var("POI_ROLE_HISTORY_BACKFILL_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(100)
            .clamp(0, 500);

        if role_history_backfill_limit > 0 {
            let missing_role_history = sqlx::query_as::<_, MissingRoleHistoryRow>(
                r#"SELECT p.id,
                          p.primary_org_id AS org_id,
                          COALESCE(c.name, 'Independent') AS org_name,
                          COALESCE(p.current_role, p.role_family, 'Unknown') AS current_role,
                          COALESCE(p.role_family, 'Unknown') AS role_family
                   FROM persons p
                   LEFT JOIN companies c ON p.primary_org_id = c.id
                   WHERE NOT EXISTS (
                       SELECT 1 FROM role_history rh WHERE rh.person_id = p.id
                   )
                   ORDER BY COALESCE(p.updated_at, p.created_at) DESC
                   LIMIT $1"#,
            )
            .bind(role_history_backfill_limit)
            .fetch_all(&store.pool)
            .await
            .unwrap_or_default();

            for row in missing_role_history {
                if store
                    .insert_role_history(
                        row.id,
                        row.org_id,
                        &row.org_name,
                        &row.current_role,
                        Some(&row.role_family),
                        Some(Utc::now()),
                        None,
                        None,
                        0.6,
                    )
                    .await
                    .is_ok()
                {
                    role_history_backfilled += 1;
                }
            }
        }

        #[derive(sqlx::FromRow)]
        struct ThinPersonRow {
            id: Uuid,
            name: String,
            org: String,
            current_role: String,
        }
        let enrichment_limit = std::env::var("POI_LLM_ENRICH_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(25)
            .clamp(0, 100);
        let thin_persons: Vec<ThinPersonRow> = sqlx::query_as::<_, ThinPersonRow>(
            r#"SELECT p.id,
                      p.name,
                      COALESCE(c.name, '') AS org,
                      COALESCE(p.current_role, 'Executive') AS current_role
               FROM persons p
               LEFT JOIN companies c ON p.primary_org_id = c.id
               WHERE (p.public_bio IS NULL OR length(COALESCE(p.public_bio, '')) < 250)
                  OR p.decision_style IS NULL
               ORDER BY COALESCE(p.influence_score, 0) DESC
             LIMIT $1"#,
        )
         .bind(enrichment_limit)
        .fetch_all(&store.pool)
        .await
        .unwrap_or_default();

        if !thin_persons.is_empty() {
            let poi_llm_client = {
                let base_url = std::env::var("LLM_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:8080".into());
                let api_key = std::env::var("LLM_API_KEY").ok();
                let model =
                    std::env::var("LLM_MODEL").unwrap_or_else(|_| "Qwen3-30B-A3B-Q4_K_M".into());
                let mut cfg = apex_llm::inference::InferenceConfig::default();
                cfg.model = model;
                cfg.max_tokens = 900;
                cfg.temperature = 0.35;
                cfg.json_mode = true;
                cfg.suppress_thinking = false;
                cfg.timeout = std::time::Duration::from_secs(90);
                InferenceLlmClient::new(base_url, api_key, cfg)
            };

            for thin in &thin_persons {
                let prompt = format!(
                    "Generate a structured intelligence profile for {name}, {role} at {org}.\n\
Return ONLY valid JSON (no markdown) with these exact keys:\n\
{{\"bio\":\"3-4 sentences of professional background for this specific person and role\",\
\"decision_style\":\"one of: Analytical/Decisive/Collaborative/Consensus-driven\",\
\"communication_style\":\"one of: Direct/Consultative/Data-driven/Relationship-focused\",\
\"risk_tolerance\":\"one of: Risk-averse/Moderate/Risk-tolerant\",\
\"change_appetite\":\"one of: Conservative/Moderate/Aggressive\",\
\"preferred_proof_type\":\"one of: ROI metrics/Case studies/Peer references/Technical specs\",\
\"trigger_topics\":[\"topic1\",\"topic2\",\"topic3\"]}}",
                    name = thin.name,
                    role = thin.current_role,
                    org = thin.org,
                );
                use apex_llm::inference::{ChatMessage, InferenceConfig};
                let messages = vec![
                    ChatMessage::system(
                        "You are an executive intelligence analyst. \
You have comprehensive knowledge of global industry executives. \
Produce a concise structured JSON profile. Return only valid JSON, no markdown, no extra text.",
                    ),
                    ChatMessage::user(&prompt),
                ];
                let enrich_config = InferenceConfig {
                    max_tokens: 900,
                    temperature: 0.35,
                    json_mode: true,
                    suppress_thinking: false,
                    timeout: std::time::Duration::from_secs(90),
                    ..Default::default()
                };
                match poi_llm_client
                    .complete_with_config(messages, &enrich_config)
                    .await
                {
                    Ok(resp) => {
                        #[derive(serde::Deserialize)]
                        struct PoiEnrichResp {
                            bio: Option<String>,
                            decision_style: Option<String>,
                            communication_style: Option<String>,
                            risk_tolerance: Option<String>,
                            change_appetite: Option<String>,
                            preferred_proof_type: Option<String>,
                            #[serde(default)]
                            trigger_topics: Vec<String>,
                        }
                        match resp.parse_json::<PoiEnrichResp>() {
                            Ok(data) => {
                                let bio = data.bio.as_deref().unwrap_or_default();
                                if !bio.is_empty() {
                                    match store
                                        .update_person_llm_enrichment(
                                            thin.id,
                                            bio,
                                            data.decision_style.as_deref(),
                                            data.communication_style.as_deref(),
                                            data.risk_tolerance.as_deref(),
                                            data.change_appetite.as_deref(),
                                            data.preferred_proof_type.as_deref(),
                                            &data.trigger_topics,
                                        )
                                        .await
                                    {
                                        Ok(()) => {
                                            tracing::info!(
                                                person = %thin.name,
                                                bio_len = bio.len(),
                                                "poi_refresh: LLM profile enrichment applied"
                                            );
                                            enriched_pois += 1;
                                        }
                                        Err(e) => tracing::warn!(
                                            person = %thin.name,
                                            error = %e,
                                            "poi_refresh: failed to write LLM enrichment"
                                        ),
                                    }
                                }
                            }
                            Err(e) => tracing::warn!(
                                person = %thin.name,
                                error = %e,
                                "poi_refresh: LLM enrichment JSON parse failed"
                            ),
                        }
                    }
                    Err(e) => tracing::warn!(
                        person = %thin.name,
                        error = %e,
                        "poi_refresh: LLM enrichment call failed"
                    ),
                }
            }
        }

        run.succeed(
            refreshed,
            &format!(
                "poi_refresh: {} persons processed — {} updated, {} unchanged, {} role-history backfilled, {} LLM-enriched",
                persons.len(),
                refreshed,
                unchanged,
                role_history_backfilled,
                enriched_pois,
            ),
        );
    }
    #[cfg(not(feature = "llm"))]
    {
        // Without the LLM feature we can still do the role-history backfill
        // and basic influence-score refresh using the database directly.
        let filters = PersonListFilters {
            regions: vec![],
            roles: vec![],
            search: None,
            min_priority: None,
            max_priority: None,
        };
        let persons = match store
            .list_persons(&filters, Some(PersonOrderBy::Priority), true, 200, 0)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                run.fail(&format!("poi_refresh(no-llm): failed to load persons: {e}"));
                return run;
            }
        };

        if persons.is_empty() {
            run.skip("poi_refresh(no-llm): no persons in database");
            return run;
        }

        let role_history_backfill_limit = std::env::var("POI_ROLE_HISTORY_BACKFILL_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(100)
            .clamp(0, 500);

        let mut role_history_backfilled: u64 = 0;
        if role_history_backfill_limit > 0 {
            #[derive(sqlx::FromRow)]
            struct MissingRoleHistoryRow {
                id: Uuid,
                org_id: Option<Uuid>,
                org_name: String,
                current_role: String,
                role_family: String,
            }

            let missing = sqlx::query_as::<_, MissingRoleHistoryRow>(
                r#"SELECT p.id,
                          p.primary_org_id AS org_id,
                          COALESCE(c.name, 'Independent') AS org_name,
                          COALESCE(p.current_role, p.role_family, 'Unknown') AS current_role,
                          COALESCE(p.role_family, 'Unknown') AS role_family
                   FROM persons p
                   LEFT JOIN companies c ON p.primary_org_id = c.id
                   WHERE NOT EXISTS (
                       SELECT 1 FROM role_history rh WHERE rh.person_id = p.id
                   )
                   ORDER BY COALESCE(p.updated_at, p.created_at) DESC
                   LIMIT $1"#,
            )
            .bind(role_history_backfill_limit)
            .fetch_all(&store.pool)
            .await
            .unwrap_or_default();

            for row in missing {
                if store
                    .insert_role_history(
                        row.id,
                        row.org_id,
                        &row.org_name,
                        &row.current_role,
                        Some(&row.role_family),
                        Some(Utc::now()),
                        None,
                        None,
                        0.6,
                    )
                    .await
                    .is_ok()
                {
                    role_history_backfilled += 1;
                }
            }
        }

        run.succeed(
            role_history_backfilled,
            &format!(
                "poi_refresh(no-llm): {} persons in DB, {} role-history entries backfilled",
                persons.len(),
                role_history_backfilled,
            ),
        );
    }
    run
}

pub(super) async fn run_poi_discovery(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::PoiDiscovery);
    run.start();
    #[cfg(feature = "llm")]
    {
        let seed_limit = std::env::var("POI_DISCOVERY_SEED_LIMIT")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(120)
            .clamp(20, 400);

        let seed_rows = match store.list_expansion_seeds(seed_limit).await {
            Ok(r) => r,
            Err(e) => {
                run.fail(&format!("poi_discovery: failed to load seed persons: {e}"));
                return run;
            }
        };

        if seed_rows.is_empty() {
            run.skip("poi_discovery: no seed persons in database");
            return run;
        }

        let min_competitor_seeds = std::env::var("POI_DISCOVERY_MIN_COMPETITOR_SEEDS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(25)
            .clamp(0, seed_rows.len());
        let min_partner_seeds = std::env::var("POI_DISCOVERY_MIN_PARTNER_SEEDS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(25)
            .clamp(0, seed_rows.len());

        let mut selected_seed_rows: Vec<&apex_store::postgres::ExpansionSeedRow> = Vec::new();
        let mut selected_ids: HashSet<String> = HashSet::new();

        for row in seed_rows
            .iter()
            .filter(|r| r.is_competitor)
            .take(min_competitor_seeds)
        {
            selected_ids.insert(row.id.to_string());
            selected_seed_rows.push(row);
        }
        for row in seed_rows
            .iter()
            .filter(|r| !r.is_competitor)
            .take(min_partner_seeds)
        {
            if selected_ids.insert(row.id.to_string()) {
                selected_seed_rows.push(row);
            }
        }
        for row in &seed_rows {
            if selected_seed_rows.len() >= seed_limit as usize {
                break;
            }
            if selected_ids.insert(row.id.to_string()) {
                selected_seed_rows.push(row);
            }
        }

        let seeds: Vec<SeedPoi> = selected_seed_rows
            .iter()
            .map(|r| SeedPoi {
                id: r.id.to_string(),
                name: r.name.clone(),
                organization: r.org_name.clone(),
                org_website: r
                    .org_domain
                    .as_deref()
                    .filter(|d| !d.is_empty())
                    .map(|d| format!("https://www.{d}")),
                region: Some(r.region.clone()).filter(|s| !s.is_empty()),
                role_family: r.role_family.clone(),
            })
            .collect();

        let seed_lookup: std::collections::HashMap<
            String,
            &apex_store::postgres::ExpansionSeedRow,
        > = selected_seed_rows
            .iter()
            .map(|r| (r.id.to_string(), *r))
            .collect();

        tracing::info!(
            seeds_total = seeds.len(),
            competitor_seeds = selected_seed_rows
                .iter()
                .filter(|r| r.is_competitor)
                .count(),
            partner_seeds = selected_seed_rows
                .iter()
                .filter(|r| !r.is_competitor)
                .count(),
            with_website = seeds.iter().filter(|s| s.org_website.is_some()).count(),
            "poi_discovery: loaded expansion seeds"
        );

        let filters = PersonListFilters {
            regions: vec![],
            roles: vec![],
            search: None,
            min_priority: None,
            max_priority: None,
        };
        let all_persons = match store.list_persons(&filters, None, true, 1000, 0).await {
            Ok(p) => p,
            Err(e) => {
                run.fail(&format!("poi_discovery: failed to load all persons: {e}"));
                return run;
            }
        };
        let known_names: HashSet<String> =
            all_persons.iter().map(|p| p.name.to_lowercase()).collect();

        let proxy_rotator = build_proxy_rotator_from_env().map(|r| Arc::new(Mutex::new(r)));
        if let Some(rotator) = proxy_rotator.as_ref() {
            if let Ok(guard) = rotator.lock() {
                tracing::info!(
                    proxy_health = %guard.health_summary(),
                    "poi_discovery: proxy rotation enabled"
                );
            }
        }

        let engine = match PoiExpansionEngine::new(proxy_rotator) {
            Ok(e) => e,
            Err(e) => {
                run.fail(&format!(
                    "poi_discovery: failed to create expansion engine: {e}"
                ));
                return run;
            }
        };

        let discoveries = engine.expand_from_seeds(&seeds, &known_names, 50).await;

        let discovered_total = discoveries.len();
        let min_confidence = 0.35;
        let mut discoveries: Vec<_> = discoveries
            .into_iter()
            .filter(|d| {
                if !looks_like_person_name(&d.name) {
                    tracing::debug!(
                        name = %d.name,
                        method = %d.discovery_method,
                        "poi_discovery: skipping non-person-like candidate"
                    );
                    return false;
                }

                if d.discovery_method == "gdelt_co_mention" && d.confidence < 0.55 {
                    tracing::debug!(
                        name = %d.name,
                        confidence = d.confidence,
                        "poi_discovery: skipping low-confidence gdelt co-mention"
                    );
                    return false;
                }

                if d.confidence < min_confidence {
                    tracing::debug!(
                        name = %d.name,
                        confidence = d.confidence,
                        method = %d.discovery_method,
                        "poi_discovery: skipping low-confidence discovery"
                    );
                    false
                } else {
                    true
                }
            })
            .collect();

        discoveries.sort_by(|a, b| {
            discovery_method_priority(&b.discovery_method)
                .cmp(&discovery_method_priority(&a.discovery_method))
                .then_with(|| {
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        const MAX_LLM_CANDIDATES: usize = 25;
        if discoveries.len() > MAX_LLM_CANDIDATES {
            discoveries.truncate(MAX_LLM_CANDIDATES);
        }

        tracing::info!(
            discovered_total,
            above_confidence = discoveries.len(),
            min_confidence,
            llm_cap = MAX_LLM_CANDIDATES,
            "poi_discovery: candidates prepared for LLM validation"
        );

        if discoveries.is_empty() {
            run.succeed(0, "poi_discovery: no new persons discovered");
            return run;
        }

        tracing::info!(
            count = discoveries.len(),
            "poi_discovery: running LLM validation on candidates"
        );

        let mut llm_config = ModelConfig::llamacpp_lightweight();
        if let Ok(base_url) = std::env::var("LLM_BASE_URL") {
            llm_config.base_url = base_url;
        }
        let llm = OpenAiCompatibleClient::new(llm_config);
        let mut validated_discoveries: Vec<DiscoveredPoi> = Vec::new();

        for disc in discoveries {
            match validate_person_via_llm(&llm, &disc).await {
                Ok(Some(validated)) => {
                    tracing::debug!(
                        name = %validated.name,
                        role = ?validated.inferred_role,
                        org = ?validated.inferred_org,
                        "poi_discovery: LLM validated as real person"
                    );
                    validated_discoveries.push(validated);
                }
                Ok(None) => {
                    tracing::info!(
                        name = %disc.name,
                        method = %disc.discovery_method,
                        "poi_discovery: LLM rejected as not a real person"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        name = %disc.name,
                        error = %e,
                        "poi_discovery: LLM validation failed, skipping"
                    );
                }
            }
        }

        let discoveries = validated_discoveries;
        tracing::info!(
            count = discoveries.len(),
            names = ?discoveries.iter().map(|d| d.name.clone()).collect::<Vec<_>>(),
            "poi_discovery: LLM validation passed {} candidates",
            discoveries.len()
        );

        if discoveries.is_empty() {
            run.succeed(0, "poi_discovery: no candidates passed LLM validation");
            return run;
        }

        let mut inserted: u64 = 0;
        let mut artifacts_ingested: u64 = 0;
        let mut skipped_dup: u64 = 0;
        let mut errors: Vec<String> = vec![];
        let now = Utc::now();

        let enrichment_proxy = build_paid_proxy_url_from_env();
        let person_scraper = PersonOsintScraper::new(enrichment_proxy.as_deref())
            .map_err(|e| {
                tracing::warn!(error = %e, "poi_discovery: failed to init person scraper, continuing without deep artifact enrichment");
                e
            })
            .ok();

        let onion_enrich_enabled = std::env::var("POI_ONION_ENRICH_ENABLED")
            .ok()
            .map(|v| parse_truthy_flag(&v))
            .unwrap_or(true);
        let max_onion_people = std::env::var("POI_ONION_ENRICH_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(12)
            .clamp(0, 200);
        let tor_client = if onion_enrich_enabled {
            Some(TorClient::new().await)
        } else {
            None
        };
        let mut onion_enriched_people = 0usize;

        let mut inserted_names: HashSet<String> = HashSet::new();

        for disc in &discoveries {
            let normalized = disc
                .name
                .split_whitespace()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            let duplicate_known_exact = known_names.contains(&disc.name.to_lowercase());
            let duplicate_known_normalized = known_names.contains(&normalized);
            let duplicate_in_run = inserted_names.contains(&normalized);

            if duplicate_known_exact || duplicate_known_normalized || duplicate_in_run {
                tracing::info!(
                    name = %disc.name,
                    method = %disc.discovery_method,
                    duplicate_known_exact,
                    duplicate_known_normalized,
                    duplicate_in_run,
                    "poi_discovery: skipping duplicate"
                );
                skipped_dup += 1;
                continue;
            }
            inserted_names.insert(normalized);

            let role_family = classify_role_family(disc.inferred_role.as_deref());
            let parent_seed = seed_lookup.get(&disc.seed_person_id);

            let primary_org_id = match resolve_discovered_company_id(
                &store,
                disc,
                parent_seed.copied(),
                now,
            )
            .await
            {
                Ok(company_id) => company_id,
                Err(error) => {
                    tracing::warn!(
                        name = %disc.name,
                        error = %error,
                        "poi_discovery: failed to resolve company, falling back to seed org"
                    );
                    parent_seed.and_then(|s| s.primary_org_id)
                }
            };

            let person = Person {
                id: Uuid::new_v4(),
                name: disc.name.clone(),
                name_ar: None,
                name_fr: None,
                primary_org_id,
                current_role: Some(
                    disc.inferred_role
                        .clone()
                        .unwrap_or_else(|| format!("Discovered ({})", disc.discovery_method)),
                ),
                role_family,
                region: parent_seed
                    .map(|s| s.region.clone())
                    .filter(|s| !s.is_empty()),
                country_code: parent_seed
                    .map(|s| s.country_code.clone())
                    .filter(|s| !s.is_empty()),
                public_bio: None,
                public_email: disc.contact_email.clone(),
                phone: None,
                personal_email: None,
                photo_hash: None,
                priority_vector: PriorityVector::default(),
                decision_mode: None,
                influence_score: 0.0,
                role_drift_score: 0.0,
                change_risk: 0.0,
                pain_index: 0.0,
                preferred_proof_type: None,
                trigger_topics: vec![],
                decision_style: None,
                risk_tolerance: None,
                change_appetite: None,
                communication_style: None,
                metadata: serde_json::json!({
                    "engagement_status": "untracked",
                    "discovery_method": disc.discovery_method,
                    "source_url": disc.source_url,
                    "seed_person_id": disc.seed_person_id,
                    "linkedin_url": disc.contact_linkedin,
                    "inferred_org": disc.inferred_org,
                    "confidence": disc.confidence,
                    "seed_is_competitor": parent_seed.map(|s| s.is_competitor).unwrap_or(false),
                    "discovery_track": if parent_seed.map(|s| s.is_competitor).unwrap_or(false) { "competitor" } else { "partner_or_prospect" },
                }),
                created_at: now,
                updated_at: now,
            };

            match store.insert_person(&person).await {
                Ok(()) => {
                    inserted += 1;
                    tracing::debug!(
                        name = %disc.name,
                        method = %disc.discovery_method,
                        "poi_discovery: inserted new person"
                    );

                    if let Some(seed) = parent_seed {
                        let role_family_label = person.role_family.as_str().to_string();
                        let _ = store
                            .insert_role_history(
                                person.id,
                                person.primary_org_id,
                                &seed.org_name,
                                person.current_role.as_deref().unwrap_or("Unknown"),
                                Some(&role_family_label),
                                Some(now),
                                None,
                                Some(&disc.source_url),
                                disc.confidence as f64,
                            )
                            .await;
                    }

                    if let Some(scraper) = person_scraper.as_ref() {
                        let org_hint = parent_seed.map(|s| s.org_name.as_str()).unwrap_or("");
                        let mut raw_artifacts = scraper.aggregate(&person.name, org_hint).await;

                        if let Some(tor) = tor_client.as_ref() {
                            let org_domain = parent_seed.and_then(|s| s.org_domain.as_deref());
                            if tor.is_available() && onion_enriched_people < max_onion_people {
                                let dark_web = tor
                                    .aggregate_dark_web_contacts(&person.name, org_domain)
                                    .await;
                                let onion_raw = dark_web_to_raw_artifacts(&dark_web, org_domain);
                                if !onion_raw.is_empty() {
                                    onion_enriched_people += 1;
                                }
                                raw_artifacts.extend(onion_raw);
                            }
                        }

                        let mut inserted_for_person = 0u64;
                        for raw in raw_artifacts.into_iter().take(120) {
                            if !is_high_quality_raw_artifact(&raw) {
                                continue;
                            }
                            if let Some(artifact) =
                                raw_to_poi_artifact(person.id, raw, &disc.source_url, now)
                            {
                                if store.insert_poi_artifact(&artifact).await.is_ok() {
                                    inserted_for_person += 1;
                                }
                            }
                        }
                        artifacts_ingested += inserted_for_person;
                        tracing::info!(
                            person = %person.name,
                            artifacts = inserted_for_person,
                            "poi_discovery: deep profile artifacts ingested"
                        );
                    }
                }
                Err(e) => {
                    errors.push(format!("{}: {}", disc.name, e));
                    tracing::warn!(
                        name = %disc.name,
                        error = %e,
                        "poi_discovery: failed to insert person"
                    );
                }
            }
        }

        if errors.is_empty() {
            run.succeed(
                inserted,
                &format!(
                    "poi_discovery: {} seeds ({} competitor / {} partner) → {} validated → {} inserted, {} artifacts, {} duplicates skipped",
                    seeds.len(),
                    selected_seed_rows.iter().filter(|r| r.is_competitor).count(),
                    selected_seed_rows.iter().filter(|r| !r.is_competitor).count(),
                    discoveries.len(),
                    inserted,
                    artifacts_ingested,
                    skipped_dup
                ),
            );
        } else {
            run.succeed(
                inserted,
                &format!(
                    "poi_discovery: {} inserted, {} artifacts, {} duplicates, {} errors: {}",
                    inserted,
                    artifacts_ingested,
                    skipped_dup,
                    errors.len(),
                    errors.join("; ")
                ),
            );
        }
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = store;
        run.skip("poi_discovery: requires the `llm` feature");
    }
    run
}
