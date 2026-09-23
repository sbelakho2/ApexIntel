use super::*;
use apex_core::analysis::{
    assess_evidence_quality, compare_temporal_windows, fuse_weak_signals,
    score_competing_hypotheses, source_group_from_url, EvidenceRecord, EvidenceStance,
    HypothesisInput, SignalFrame,
};

fn normalize_person_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

const GET_PERSON_PEERS_QUERY: &str = "SELECT p.id,
                                        p.name,
                                        COALESCE(p.\"current_role\", p.role_family, 'Unknown') AS role,
                                        COALESCE(c.name, 'Independent') AS organization,
                                        COALESCE(p.region, '') AS region,
                                        COALESCE(p.country_code, '') AS country,
                                        COALESCE(p.role_family, 'Unknown') AS role_family,
                                        COALESCE(p.influence_score, 0) AS priority_score,
                                        COALESCE(p.pain_index, 0) AS pain_index,
                                        COALESCE(p.change_risk, 0) AS change_risk,
                                        COALESCE(p.role_drift_score, 0) AS role_drift_score,
                                        COALESCE(p.metadata->>'engagement_status', 'untracked') AS engagement_status,
                                        COALESCE(p.updated_at, p.created_at, now()) AS updated_at
                         FROM persons p
                         LEFT JOIN companies c ON p.primary_org_id = c.id
                         WHERE p.id != $1
                             AND (p.role_family = $2 OR COALESCE(p.region, '') = $3)
                         ORDER BY p.influence_score DESC NULLS LAST
                         LIMIT $4";

fn normalize_person_seed_limit(limit: i64) -> i64 {
    clamp_limit(limit)
}

fn normalize_person_identity_name(name: &str) -> String {
    name.split_whitespace()
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn person_identity_dedup_key(name: &str, primary_org_id: Option<Uuid>) -> Option<String> {
    let normalized_name = normalize_person_identity_name(name);
    if normalized_name.is_empty() {
        return None;
    }

    Some(match primary_org_id {
        Some(org_id) => format!("{normalized_name}|{org_id}"),
        None => format!("{normalized_name}|no-org"),
    })
}

fn person_engagement_status(person: &PersonRow) -> String {
    person
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("engagement_status"))
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "untracked".to_string())
}

fn person_temporal_delta(
    role_history: &[RoleHistoryRow],
    dossier_entries: &[DossierEntryRow],
    recent_changes: &[PersonChangeRow],
) -> apex_core::analysis::TemporalDelta {
    let mut timestamps: Vec<DateTime<Utc>> = role_history
        .iter()
        .filter_map(|role| role.updated_at.or(role.start_date))
        .collect();
    timestamps.extend(dossier_entries.iter().filter_map(|entry| entry.created_at));
    timestamps.extend(
        recent_changes
            .iter()
            .filter_map(|change| change.detected_at.or(change.created_at)),
    );

    if timestamps.is_empty() {
        return compare_temporal_windows(0.0, 0.0);
    }

    timestamps.sort();
    let midpoint = timestamps[0]
        + chrono::Duration::seconds(
            (timestamps[timestamps.len() - 1] - timestamps[0]).num_seconds() / 2,
        );
    let early = timestamps.iter().filter(|ts| **ts <= midpoint).count() as f64;
    let late = timestamps.iter().filter(|ts| **ts > midpoint).count() as f64;
    compare_temporal_windows(late, early)
}

fn build_person_dossier_analysis(
    artifacts: &[ArtifactRow],
    observations: &[ObservationRow],
    role_history: &[RoleHistoryRow],
    dossier_entries: &[DossierEntryRow],
    recent_changes: &[PersonChangeRow],
) -> DossierAnalysis {
    let now = Utc::now();
    let mut evidence_records: Vec<EvidenceRecord> = artifacts
        .iter()
        .map(|artifact| {
            let mut record = EvidenceRecord::new(0.65, EvidenceStance::Supports)
                .with_source_url(artifact.url.clone())
                .with_source_type(artifact.artifact_type.clone())
                .with_observed_at(artifact.ts_utc);
            if let Some(domain) = artifact.source_domain.as_deref() {
                record = record.with_source_id(domain.to_string());
            }
            record
        })
        .collect();
    evidence_records.extend(role_history.iter().filter_map(|role| {
        role.source_url.as_ref().map(|url| {
            let mut record =
                EvidenceRecord::new(role.confidence.unwrap_or(0.6), EvidenceStance::Supports)
                    .with_source_url(url.clone())
                    .with_source_type("role_history");
            if let Some(updated_at) = role.updated_at.or(role.start_date) {
                record = record.with_observed_at(updated_at);
            }
            record
        })
    }));
    evidence_records.extend(dossier_entries.iter().flat_map(|entry| {
        entry
            .source_urls
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(move |url| {
                let mut record =
                    EvidenceRecord::new(entry.confidence.unwrap_or(0.6), EvidenceStance::Supports)
                        .with_source_url(url)
                        .with_source_type(entry.category.clone());
                if let Some(created_at) = entry.created_at {
                    record = record.with_observed_at(created_at);
                }
                record
            })
    }));
    evidence_records.extend(recent_changes.iter().filter_map(|change| {
        change.source_url.as_ref().map(|url| {
            let mut record =
                EvidenceRecord::new(change.confidence.unwrap_or(0.6), EvidenceStance::Supports)
                    .with_source_url(url.clone())
                    .with_source_type(change.change_type.clone());
            if let Some(detected_at) = change.detected_at.or(change.created_at) {
                record = record.with_observed_at(detected_at);
            }
            record
        })
    }));
    let evidence_quality = assess_evidence_quality(&evidence_records, now);

    let temporal_delta = person_temporal_delta(role_history, dossier_entries, recent_changes);

    let mut signal_frames = Vec::new();
    for change in recent_changes {
        signal_frames.push(SignalFrame {
            id: change.id.to_string(),
            theme: change.change_type.clone(),
            category: change.field_name.clone(),
            region: None,
            entity: Some(change.person_id.to_string()),
            confidence: change.confidence.unwrap_or(0.55),
            impact: 0.55,
            source_group: change.source_url.as_deref().and_then(source_group_from_url),
        });
    }
    for role in role_history {
        signal_frames.push(SignalFrame {
            id: role.id.to_string(),
            theme: role
                .role_family
                .clone()
                .unwrap_or_else(|| "role".to_string()),
            category: Some(role.title.clone()),
            region: None,
            entity: Some(role.person_id.to_string()),
            confidence: role.confidence.unwrap_or(0.55),
            impact: 0.5,
            source_group: role.source_url.as_deref().and_then(source_group_from_url),
        });
    }
    let correlated_signals = fuse_weak_signals(&signal_frames);

    let artifact_signal = (artifacts.len() as f64 / 8.0).clamp(0.0, 1.0);
    let observation_signal = (observations.len() as f64 / 12.0).clamp(0.0, 1.0);
    let change_signal = (recent_changes.len() as f64 / 6.0).clamp(0.0, 1.0);
    let role_transition_signal = role_history
        .iter()
        .filter(|role| role.end_date.is_some())
        .count() as f64
        / role_history.len().max(1) as f64;
    let competing_hypotheses = score_competing_hypotheses(&[
        HypothesisInput {
            hypothesis: "Role expansion or mandate growth".to_string(),
            support_score: (artifact_signal * 0.45
                + observation_signal * 0.35
                + change_signal * 0.2)
                .clamp(0.0, 1.0),
            contradiction_score: role_transition_signal * 0.4,
            prior: 0.45,
        },
        HypothesisInput {
            hypothesis: "Transition or churn risk".to_string(),
            support_score: (role_transition_signal + change_signal * 0.3).clamp(0.0, 1.0),
            contradiction_score: artifact_signal * 0.25,
            prior: 0.30,
        },
        HypothesisInput {
            hypothesis: "Active market signalling and outreach".to_string(),
            support_score: (artifact_signal * 0.6 + observation_signal * 0.2).clamp(0.0, 1.0),
            contradiction_score: role_transition_signal * 0.15,
            prior: 0.35,
        },
    ]);

    let summary = format!(
        "Evidence posture is {} ({:.2}) with {} independent sources. Activity is {} and {} correlated weak-signal cluster(s) are being fused for analyst review.",
        evidence_quality.quality_label,
        evidence_quality.overall_score,
        evidence_quality.independent_source_count,
        temporal_delta.label,
        correlated_signals.len(),
    );

    DossierAnalysis {
        evidence_quality,
        temporal_delta,
        correlated_signals,
        competing_hypotheses,
        summary,
    }
}

fn append_person_filters(qb: &mut QueryBuilder<Postgres>, filters: &PersonListFilters) {
    let mut has_where = false;

    if !filters.regions.is_empty() {
        let regions: Vec<String> = filters
            .regions
            .iter()
            .map(|region| region.to_lowercase())
            .collect();
        qb.push(" WHERE ");
        qb.push("LOWER(COALESCE(p.region, '')) = ANY(");
        qb.push_bind(regions);
        qb.push(")");
        has_where = true;
    }

    if !filters.roles.is_empty() {
        let roles: Vec<String> = filters
            .roles
            .iter()
            .map(|role| role.to_lowercase())
            .collect();
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("LOWER(COALESCE(p.role_family, p.\"current_role\", '')) = ANY(");
        qb.push_bind(roles);
        qb.push(")");
        has_where = true;
    }

    if let Some(min_priority) = filters.min_priority {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("COALESCE(p.influence_score, 0) >= ");
        qb.push_bind(min_priority);
        has_where = true;
    }

    if let Some(max_priority) = filters.max_priority {
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("COALESCE(p.influence_score, 0) < ");
        qb.push_bind(max_priority);
        has_where = true;
    }

    if let Some(search) = &filters.search {
        let pattern = ilike_pattern(search);
        qb.push(if has_where { " AND " } else { " WHERE " });
        qb.push("(p.name ILIKE ");
        qb.push_bind(pattern.clone());
        qb.push(" OR c.name ILIKE ");
        qb.push_bind(pattern);
        qb.push(")");
    }
}

impl PgStore {
    pub async fn insert_person(&self, p: &Person) -> Result<()> {
        let pv_json = serde_json::to_value(&p.priority_vector)?;
        let normalized_name = normalize_person_identity_name(&p.name);
        let dedup_key = person_identity_dedup_key(&p.name, p.primary_org_id)
            .ok_or_else(|| anyhow::anyhow!("person name must not be empty"))?;
        sqlx::query(
            r#"WITH identity_lock AS (
                   SELECT pg_advisory_xact_lock(hashtext($16), 0)
               ),
               existing AS (
                   SELECT persons.id
                   FROM persons, identity_lock
                   WHERE lower(regexp_replace(trim(persons.name), '\s+', ' ', 'g')) = $2
                     AND persons.primary_org_id IS NOT DISTINCT FROM $6
                   ORDER BY persons.created_at ASC NULLS LAST, persons.id ASC
                   LIMIT 1
               )
               INSERT INTO persons
               (id, name, name_ar, name_fr, primary_org_id, "current_role",
                role_family, region, country_code, priority_vector,
                influence_score, metadata, created_at, updated_at)
               VALUES (
                   COALESCE((SELECT id FROM existing), $1),
                   $3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15
               )
               ON CONFLICT (id) DO UPDATE SET
                 name = COALESCE(NULLIF(EXCLUDED.name, ''), persons.name),
                 name_ar = COALESCE(EXCLUDED.name_ar, persons.name_ar),
                 name_fr = COALESCE(EXCLUDED.name_fr, persons.name_fr),
                 primary_org_id = COALESCE(EXCLUDED.primary_org_id, persons.primary_org_id),
                 "current_role" = COALESCE(EXCLUDED."current_role", persons."current_role"),
                 role_family = COALESCE(EXCLUDED.role_family, persons.role_family),
                 region = COALESCE(EXCLUDED.region, persons.region),
                 country_code = COALESCE(EXCLUDED.country_code, persons.country_code),
                 priority_vector = COALESCE(EXCLUDED.priority_vector, persons.priority_vector),
                 influence_score = GREATEST(COALESCE(persons.influence_score, 0), COALESCE(EXCLUDED.influence_score, 0)),
                 metadata = COALESCE(persons.metadata, '{}'::jsonb) || COALESCE(EXCLUDED.metadata, '{}'::jsonb),
                 updated_at = now()"#,
        )
        .bind(p.id)
        .bind(&normalized_name)
        .bind(&p.name)
        .bind(&p.name_ar)
        .bind(&p.name_fr)
        .bind(p.primary_org_id)
        .bind(&p.current_role)
        .bind(p.role_family.as_str())
        .bind(&p.region)
        .bind(&p.country_code)
        .bind(&pv_json)
        .bind(p.influence_score)
        .bind(&p.metadata)
        .bind(p.created_at)
        .bind(p.updated_at)
        .bind(&dedup_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_person_names_by_company_ids(
        &self,
        company_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, String)>> {
        if company_ids.is_empty() {
            return Ok(vec![]);
        }
        let rows: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT primary_org_id, name FROM persons WHERE primary_org_id = ANY($1) ORDER BY influence_score DESC NULLS LAST",
        )
        .bind(company_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_person(&self, id: Uuid) -> Result<Option<PersonRow>> {
        let row = sqlx::query_as::<_, PersonRow>(
            "SELECT id, name, name_ar, name_fr, primary_org_id, \"current_role\",
                    role_family, region, country_code, public_bio, public_email,
                    priority_vector, influence_score, trigger_topics, decision_style,
                    risk_tolerance, change_appetite, communication_style,
                    decision_mode, preferred_proof_type, pain_index, change_risk,
                    role_drift_score,
                    metadata, created_at, updated_at
             FROM persons WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn update_person_contacts(
        &self,
        person_id: Uuid,
        email: Option<&str>,
        phone: Option<&str>,
        linkedin: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE persons
               SET public_email = COALESCE($2, public_email),
                   metadata = jsonb_set(
                       jsonb_set(
                           COALESCE(metadata, '{}'::jsonb),
                           '{phone}', to_jsonb($3::TEXT)
                       ),
                       '{linkedin_url}', to_jsonb($4::TEXT)
                   ),
                   updated_at = now()
               WHERE id = $1"#,
        )
        .bind(person_id)
        .bind(email)
        .bind(phone)
        .bind(linkedin)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_expansion_seeds(&self, limit: i64) -> Result<Vec<ExpansionSeedRow>> {
        let limit = normalize_person_seed_limit(limit);
        let rows = sqlx::query_as::<_, ExpansionSeedRow>(
            r#"WITH ranked AS (
                   SELECT p.id,
                          p.name,
                          COALESCE(p."current_role", p.role_family, '') AS current_role,
                          COALESCE(p.role_family, 'Other') AS role_family,
                          COALESCE(p.region, '') AS region,
                          COALESCE(p.country_code, '') AS country_code,
                          p.primary_org_id,
                          COALESCE(c.name, '') AS org_name,
                         c.domain AS org_domain,
                         COALESCE((c.metadata->>'is_competitor')::boolean, false) AS is_competitor,
                          ROW_NUMBER() OVER (
                              PARTITION BY COALESCE(NULLIF(p.region, ''), 'GLOBAL')
                              ORDER BY
                                  CASE
                                      WHEN LOWER(COALESCE(p.role_family, '')) IN (
                                          'government', 'procurement', 'operations', 'engineering',
                                          'quality', 'legal', 'security', 'logistics', 'finance'
                                      ) THEN 0
                                      WHEN LOWER(COALESCE(p.role_family, '')) = 'executive' THEN 2
                                      ELSE 1
                                  END,
                                  COALESCE(p.influence_score, 0) DESC,
                                  p.updated_at DESC NULLS LAST
                          ) AS region_rank
                   FROM persons p
                   LEFT JOIN companies c ON p.primary_org_id = c.id
                   WHERE COALESCE(p.name, '') <> ''
               )
               SELECT id,
                      name,
                      "current_role",
                      role_family,
                      region,
                      country_code,
                      primary_org_id,
                      org_name,
                 org_domain,
                 is_competitor
               FROM ranked
               WHERE region_rank <= GREATEST(2, LEAST(8, $1 / 6))
               ORDER BY
                   CASE
                       WHEN LOWER(role_family) IN ('government', 'procurement', 'operations', 'engineering', 'quality', 'legal', 'security', 'logistics', 'finance') THEN 0
                       WHEN LOWER(role_family) = 'executive' THEN 2
                       ELSE 1
                   END,
                   region,
                   name
               LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_persons_by_org(&self, org_id: Uuid) -> Result<Vec<PersonRow>> {
        let rows = sqlx::query_as::<_, PersonRow>(
            "SELECT id, name, name_ar, name_fr, primary_org_id, \"current_role\",
                    role_family, region, country_code, public_bio, public_email,
                    priority_vector, influence_score, trigger_topics, decision_style,
                    risk_tolerance, change_appetite, communication_style,
                    decision_mode, preferred_proof_type, pain_index, change_risk,
                    role_drift_score,
                    metadata, created_at, updated_at
             FROM persons WHERE primary_org_id = $1 ORDER BY name",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn update_person_influence_score(&self, id: Uuid, score: f64) -> Result<()> {
        sqlx::query(
            r#"UPDATE persons
               SET influence_score = $2,
                   updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(score.clamp(0.0, 1.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Update computed scores: pain_index, change_risk, and role_drift_score.
    pub async fn update_person_computed_scores(
        &self,
        id: Uuid,
        pain_index: f64,
        change_risk: f64,
        role_drift_score: f64,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE persons
               SET pain_index = $2,
                   change_risk = $3,
                   role_drift_score = $4,
                   updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(pain_index.clamp(0.0, 1.0))
        .bind(change_risk.clamp(0.0, 1.0))
        .bind(role_drift_score.clamp(0.0, 1.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Query persons that need psychological profile enrichment.
    /// Returns persons where psychographic fields are NULL or stale (>7 days).
    pub async fn get_persons_needing_psych_enrichment(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PersonRow>> {
        let (limit, offset) = normalize_person_window(limit, offset);
        sqlx::query_as::<_, PersonRow>(
            r#"SELECT id, name, name_ar, name_fr, primary_org_id, "current_role",
                      role_family, region, country_code, public_bio, public_email,
                      priority_vector, influence_score, trigger_topics, decision_style,
                      risk_tolerance, change_appetite, communication_style,
                      decision_mode, preferred_proof_type, pain_index, change_risk,
                      role_drift_score,
                      metadata, created_at, updated_at
               FROM persons
               WHERE decision_style IS NULL
                  OR risk_tolerance IS NULL
                  OR change_appetite IS NULL
                  OR communication_style IS NULL
                  OR (pain_index IS NULL AND influence_score IS NULL)
               ORDER BY influence_score DESC NULLS LAST
               LIMIT $1 OFFSET $2"#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Update psychological profile fields on a person record.
    /// Only non-None fields are updated (COALESCE pattern).
    /// Also stamps last_psych_enrichment_at.
    pub async fn update_person_psych_profile(
        &self,
        person_id: Uuid,
        priority_vector: Option<&str>,
        decision_style: Option<&str>,
        risk_tolerance: Option<&str>,
        change_appetite: Option<&str>,
        communication_style: Option<&str>,
        pain_index: Option<f64>,
        influence_score: Option<f64>,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE persons SET
                 priority_vector = COALESCE($2::jsonb, priority_vector),
                 decision_style = COALESCE($3, decision_style),
                 risk_tolerance = COALESCE($4, risk_tolerance),
                 change_appetite = COALESCE($5, change_appetite),
                 communication_style = COALESCE($6, communication_style),
                 pain_index = COALESCE($7, pain_index),
                 influence_score = COALESCE($8, influence_score),
                 last_psych_enrichment_at = now(),
                 updated_at = now()
               WHERE id = $1"#,
        )
        .bind(person_id)
        .bind(priority_vector)
        .bind(decision_style)
        .bind(risk_tolerance)
        .bind(change_appetite)
        .bind(communication_style)
        .bind(pain_index.map(|v| v.clamp(0.0, 1.0)))
        .bind(influence_score.map(|v| v.clamp(0.0, 1.0)))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Update a person's company affiliation and role.
    /// Creates a role_history entry if the company changes.
    pub async fn update_person_company_and_role(
        &self,
        person_id: Uuid,
        company_id: Option<Uuid>,
        current_role: Option<&str>,
        role_family: Option<&str>,
        confidence: f64,
    ) -> Result<()> {
        // Check current org to detect changes
        let current_org: Option<Uuid> =
            sqlx::query_scalar("SELECT primary_org_id FROM persons WHERE id = $1")
                .bind(person_id)
                .fetch_optional(&self.pool)
                .await?
                .flatten();

        let org_changed = current_org != company_id;

        sqlx::query(
            r#"UPDATE persons SET
                 primary_org_id = COALESCE($2, primary_org_id),
                 "current_role" = COALESCE($3, "current_role"),
                 role_family = COALESCE($4, role_family),
                 updated_at = now()
               WHERE id = $1"#,
        )
        .bind(person_id)
        .bind(company_id)
        .bind(current_role)
        .bind(role_family)
        .execute(&self.pool)
        .await?;

        // If organization changed, create a role_history entry
        if org_changed {
            if let Some(company_id) = company_id {
                let org_name: Option<String> =
                    sqlx::query_scalar("SELECT name FROM companies WHERE id = $1")
                        .bind(company_id)
                        .fetch_optional(&self.pool)
                        .await?
                        .flatten();

                let role_text = current_role
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                let family_text = role_family
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "Unknown".to_string());
                let org_name_text = org_name.unwrap_or_else(|| "Unknown".to_string());

                self.insert_role_history(
                    person_id,
                    Some(company_id),
                    &org_name_text,
                    &role_text,
                    Some(&family_text),
                    Some(Utc::now()),
                    None,
                    None,
                    confidence.clamp(0.0, 1.0),
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn update_person_llm_enrichment(
        &self,
        id: Uuid,
        bio: &str,
        decision_style: Option<&str>,
        communication_style: Option<&str>,
        risk_tolerance: Option<&str>,
        change_appetite: Option<&str>,
        preferred_proof_type: Option<&str>,
        trigger_topics: &[String],
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE persons SET
                public_bio = CASE
                    WHEN $2 IS NOT NULL AND length($2) > 0 THEN $2
                    ELSE public_bio
                END,
                decision_style = COALESCE($3, decision_style),
                communication_style = COALESCE($4, communication_style),
                risk_tolerance = COALESCE($5, risk_tolerance),
                change_appetite = COALESCE($6, change_appetite),
                preferred_proof_type = COALESCE($7, preferred_proof_type),
                trigger_topics = CASE
                    WHEN $8::TEXT[] IS NOT NULL AND array_length($8, 1) IS NOT NULL THEN $8
                    ELSE trigger_topics
                END,
                updated_at = now()
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(bio)
        .bind(decision_style)
        .bind(communication_style)
        .bind(risk_tolerance)
        .bind(change_appetite)
        .bind(preferred_proof_type)
        .bind(trigger_topics)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn count_persons(&self, filters: &PersonListFilters) -> Result<i64> {
        let mut qb = QueryBuilder::new(
            "SELECT COUNT(*) FROM persons p LEFT JOIN companies c ON p.primary_org_id = c.id",
        );
        append_person_filters(&mut qb, filters);
        let query = qb.build_query_as::<(i64,)>();
        let (count,) = query.fetch_one(&self.pool).await?;
        Ok(count)
    }

    pub async fn list_persons(
        &self,
        filters: &PersonListFilters,
        order_by: Option<PersonOrderBy>,
        desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PersonListRow>> {
        let (limit, offset) = normalize_person_window(limit, offset);
        let mut qb = QueryBuilder::new(
            "SELECT p.id,
                    p.name,
                    COALESCE(p.\"current_role\", p.role_family, 'Unknown') AS role,
                    COALESCE(c.name, 'Independent') AS organization,
                    COALESCE(p.region, '') AS region,
                    COALESCE(p.country_code, '') AS country,
                    COALESCE(p.role_family, 'Unknown') AS role_family,
                    COALESCE(p.influence_score, 0) AS priority_score,
                    COALESCE(p.pain_index, 0) AS pain_index,
                    COALESCE(p.change_risk, 0) AS change_risk,
                    COALESCE(p.role_drift_score, 0) AS role_drift_score,
                    COALESCE(p.metadata->>'engagement_status', 'untracked') AS engagement_status,
                    COALESCE(p.updated_at, p.created_at, now()) AS updated_at
             FROM persons p
             LEFT JOIN companies c ON p.primary_org_id = c.id",
        );
        append_person_filters(&mut qb, filters);

        let order_clause = match order_by.unwrap_or(PersonOrderBy::UpdatedAt) {
            PersonOrderBy::Name => "p.name",
            PersonOrderBy::Priority => "COALESCE(p.influence_score, 0)",
            PersonOrderBy::Region => "COALESCE(p.region, '')",
            PersonOrderBy::UpdatedAt => "COALESCE(p.updated_at, p.created_at)",
        };
        qb.push(" ORDER BY ");
        qb.push(order_clause);
        if desc {
            qb.push(" DESC");
        }
        qb.push(", p.id ASC");
        qb.push(" LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        let query = qb.build_query_as::<PersonListRow>();
        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn get_person_dossier(&self, person_id: Uuid) -> Result<Option<PersonDossier>> {
        let person = match self.get_person(person_id).await? {
            Some(person) => person,
            None => return Ok(None),
        };
        let artifacts = self
            .get_artifacts_for_person(person_id, 200)
            .await
            .unwrap_or_default();
        let observations = self
            .get_observations_by_entity(person_id, 200)
            .await
            .unwrap_or_default();
        let edges = self
            .get_edges_from(person_id, "person")
            .await
            .unwrap_or_default();
        let role_history = self
            .get_role_history(person_id, 100)
            .await
            .unwrap_or_default();
        let dossier_entries = self
            .get_dossier_entries("person", person_id, None, 200)
            .await
            .unwrap_or_default();
        let recent_changes = self
            .get_person_changes(person_id, 50)
            .await
            .unwrap_or_default();
        let analysis = build_person_dossier_analysis(
            &artifacts,
            &observations,
            &role_history,
            &dossier_entries,
            &recent_changes,
        );
        Ok(Some(PersonDossier {
            person,
            artifacts,
            observations,
            edges,
            role_history,
            dossier_entries,
            recent_changes,
            analysis,
        }))
    }

    pub async fn get_person_engagement(&self, person_id: Uuid) -> Result<Option<PersonEngagement>> {
        let person = match self.get_person(person_id).await? {
            Some(person) => person,
            None => return Ok(None),
        };
        let co_appearances = self
            .get_edges_from(person_id, "person")
            .await
            .unwrap_or_default();
        let recent_observations = sqlx::query_as::<_, ObservationRow>(
            "SELECT * FROM observations WHERE entity_id = $1 ORDER BY ts_utc DESC LIMIT 20",
        )
        .bind(person_id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        let influence = person.influence_score.unwrap_or(0.0);
        Ok(Some(PersonEngagement {
            person_id,
            name: person.name.clone(),
            role: person.current_role.clone(),
            priority_score: influence,
            engagement_status: person_engagement_status(&person),
            co_appearances,
            recent_observations,
        }))
    }

    pub async fn get_person_peers(
        &self,
        person_id: Uuid,
        role_family: &str,
        region: &str,
        limit: i64,
    ) -> Result<Vec<PersonListRow>> {
        let (limit, _) = normalize_person_window(limit, 0);
        let rows = sqlx::query_as::<_, PersonListRow>(GET_PERSON_PEERS_QUERY)
            .bind(person_id)
            .bind(role_family)
            .bind(region)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn test_normalize_person_window_clamps_limit_and_offset() {
        assert_eq!(normalize_person_window(0, -4), (1, 0));
        assert_eq!(normalize_person_window(9999, 7), (500, 7));
    }

    #[test]
    fn test_normalize_person_seed_limit_clamps_large_values() {
        assert_eq!(normalize_person_seed_limit(0), 1);
        assert_eq!(normalize_person_seed_limit(9999), 500);
    }

    #[test]
    fn test_normalize_person_identity_name_collapses_whitespace_and_case() {
        assert_eq!(
            normalize_person_identity_name("  Jane   SMITH  "),
            "jane smith"
        );
    }

    #[test]
    fn test_get_person_peers_query_selects_computed_score_columns() {
        assert!(GET_PERSON_PEERS_QUERY.contains("AS pain_index"));
        assert!(GET_PERSON_PEERS_QUERY.contains("AS change_risk"));
        assert!(GET_PERSON_PEERS_QUERY.contains("AS role_drift_score"));
    }

    #[test]
    fn test_person_identity_dedup_key_includes_org_scope() {
        let org_id = Uuid::nil();
        assert_eq!(
            person_identity_dedup_key("Jane Smith", Some(org_id)).as_deref(),
            Some("jane smith|00000000-0000-0000-0000-000000000000")
        );
        assert_eq!(
            person_identity_dedup_key("Jane Smith", None).as_deref(),
            Some("jane smith|no-org")
        );
    }

    #[test]
    fn test_person_engagement_status_defaults_to_untracked() {
        let row = PersonRow {
            id: Uuid::new_v4(),
            name: "Test Person".to_string(),
            name_ar: None,
            name_fr: None,
            primary_org_id: None,
            current_role: None,
            role_family: Some("operations".to_string()),
            region: Some("mena".to_string()),
            country_code: Some("AE".to_string()),
            public_bio: None,
            public_email: None,
            priority_vector: None,
            influence_score: Some(0.5),
            trigger_topics: None,
            decision_style: None,
            risk_tolerance: None,
            change_appetite: None,
            communication_style: None,
            decision_mode: None,
            preferred_proof_type: None,
            pain_index: None,
            change_risk: None,
            role_drift_score: None,
            metadata: Some(json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };

        assert_eq!(person_engagement_status(&row), "untracked");
    }
}
