use super::*;
use apex_core::analysis::{
    assess_evidence_quality, compare_temporal_windows, fuse_weak_signals,
    score_competing_hypotheses, source_group_from_url, EvidenceRecord, EvidenceStance,
    HypothesisInput, SignalFrame,
};

fn normalize_person_window(limit: i64, offset: i64) -> (i64, i64) {
    (clamp_limit(limit), offset.max(0))
}

fn normalize_person_seed_limit(limit: i64) -> i64 {
    clamp_limit(limit)
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
        sqlx::query(
            r#"INSERT INTO persons
               (id, name, name_ar, name_fr, primary_org_id, "current_role",
                role_family, region, country_code, priority_vector,
                influence_score, metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 name_ar = EXCLUDED.name_ar,
                 name_fr = EXCLUDED.name_fr,
                 primary_org_id = EXCLUDED.primary_org_id,
                 "current_role" = EXCLUDED."current_role",
                 role_family = EXCLUDED.role_family,
                 region = EXCLUDED.region,
                 country_code = EXCLUDED.country_code,
                 priority_vector = EXCLUDED.priority_vector,
                 influence_score = EXCLUDED.influence_score,
                 metadata = EXCLUDED.metadata,
                 updated_at = now()"#,
        )
        .bind(p.id)
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
                      current_role,
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
                    WHEN public_bio IS NULL OR length(public_bio) < 100 THEN $2
                    ELSE public_bio
                END,
                decision_style = COALESCE(decision_style, $3),
                communication_style = COALESCE(communication_style, $4),
                risk_tolerance = COALESCE(risk_tolerance, $5),
                change_appetite = COALESCE(change_appetite, $6),
                preferred_proof_type = COALESCE(preferred_proof_type, $7),
                trigger_topics = CASE
                    WHEN trigger_topics IS NULL OR array_length(trigger_topics, 1) IS NULL THEN $8
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
                    COALESCE(p.country_code, '') AS country_code,
                    COALESCE(p.role_family, 'Unknown') AS role_family,
                    COALESCE(p.influence_score, 0) AS priority_score,
                    COALESCE(p.metadata->>'engagement_status', 'untracked') AS engagement_status,
                    COALESCE(p.updated_at, p.created_at, now()) AS updated_at,
                    (SELECT COUNT(*) FROM poi_artifacts a WHERE a.person_id = p.id) AS artifact_count
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
        let rows = sqlx::query_as::<_, PersonListRow>(
            "SELECT p.id,
                    p.name,
                    COALESCE(p.\"current_role\", p.role_family, 'Unknown') AS role,
                    COALESCE(c.name, 'Independent') AS organization,
                    COALESCE(p.region, '') AS region,
                    COALESCE(p.country_code, '') AS country_code,
                    COALESCE(p.role_family, 'Unknown') AS role_family,
                    COALESCE(p.influence_score, 0) AS priority_score,
                    COALESCE(p.metadata->>'engagement_status', 'untracked') AS engagement_status,
                    COALESCE(p.updated_at, p.created_at, now()) AS updated_at,
                    (SELECT COUNT(*) FROM poi_artifacts a WHERE a.person_id = p.id) AS artifact_count
             FROM persons p
             LEFT JOIN companies c ON p.primary_org_id = c.id
             WHERE p.id != $1
               AND (p.role_family = $2 OR COALESCE(p.region, '') = $3)
             ORDER BY p.influence_score DESC NULLS LAST
             LIMIT $4",
        )
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
