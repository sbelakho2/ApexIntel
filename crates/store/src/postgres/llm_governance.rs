use super::*;

fn json_value<T: serde::Serialize>(value: T) -> serde_json::Value {
    serde_json::to_value(value)
        .unwrap_or_else(|error| panic!("value should serialize to JSON: {error}"))
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ReviewedWarningGoldenSetRow {
    id: Uuid,
    recipe_code: Option<String>,
    warning_type: String,
    title: String,
    description: Option<String>,
    severity: String,
    region: Option<String>,
    source_urls: Option<Vec<String>>,
    confidence: Option<f64>,
    review_outcome: String,
    reviewed_at: Option<DateTime<Utc>>,
    ts_utc: DateTime<Utc>,
}

fn serialize_quality_gate_examples_jsonl(
    examples: &[QualityGateGoldenSetExample],
) -> Result<String> {
    let mut lines = Vec::with_capacity(examples.len());
    for example in examples {
        lines.push(serde_json::to_string(example)?);
    }
    Ok(lines.join("\n"))
}

fn map_reviewed_warning_golden_example(
    row: ReviewedWarningGoldenSetRow,
) -> QualityGateGoldenSetExample {
    let historical_label = match row.review_outcome.as_str() {
        "true_positive" => HistoricalQualityGateLabel::Accepted,
        _ => HistoricalQualityGateLabel::Rejected,
    };

    QualityGateGoldenSetExample {
        source_id: row.id,
        source_kind: "warning".to_string(),
        content_type: row.warning_type.clone(),
        historical_label,
        title: row.title,
        body: row.description.unwrap_or_default(),
        region: row.region,
        confidence: row.confidence,
        source_urls: row.source_urls.unwrap_or_default(),
        reviewed_at: row.reviewed_at.unwrap_or(row.ts_utc),
        metadata: {
            let mut metadata = serde_json::Map::new();
            metadata.insert("severity".to_string(), row.severity.into());
            metadata.insert("recipe_code".to_string(), json_value(row.recipe_code));
            metadata.insert("review_outcome".to_string(), json_value(row.review_outcome));
            serde_json::Value::Object(metadata)
        },
    }
}

impl PgStore {
    pub async fn list_prompt_versions(&self, limit: i64) -> Result<Vec<PromptVersionRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, PromptVersionRecord>(
            r#"SELECT prompt_id, version, workflow, system_prompt, metadata, created_at
               FROM prompt_versions
               ORDER BY created_at DESC, prompt_id ASC, version DESC
               LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn ensure_prompt_version(
        &self,
        prompt_id: &str,
        version: &str,
        workflow: &str,
        system_prompt: &str,
        metadata: &Value,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO prompt_versions (prompt_id, version, workflow, system_prompt, metadata)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (prompt_id, version) DO UPDATE SET
                 workflow = EXCLUDED.workflow,
                 system_prompt = EXCLUDED.system_prompt,
                 metadata = EXCLUDED.metadata"#,
        )
        .bind(prompt_id)
        .bind(version)
        .bind(workflow)
        .bind(system_prompt)
        .bind(metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_llm_workflow_run(
        &self,
        workflow: &str,
        prompt_id: &str,
        prompt_version: &str,
        model_name: &str,
        request_payload: &Value,
        response_payload: &Value,
        validation_issues: &[String],
        quality_gate_passed: bool,
        duration_ms: i64,
    ) -> Result<LlmWorkflowRunRecord> {
        let validation_issues = serde_json::to_value(validation_issues)?;
        Ok(sqlx::query_as::<_, LlmWorkflowRunRecord>(
            r#"INSERT INTO llm_workflow_runs (
                   workflow, prompt_id, prompt_version, model_name, request_payload,
                   response_payload, validation_issues, quality_gate_passed, duration_ms
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, workflow, prompt_id, prompt_version, model_name, request_payload,
                         response_payload, validation_issues, quality_gate_passed, duration_ms,
                         created_at"#,
        )
        .bind(workflow)
        .bind(prompt_id)
        .bind(prompt_version)
        .bind(model_name)
        .bind(request_payload)
        .bind(response_payload)
        .bind(validation_issues)
        .bind(quality_gate_passed)
        .bind(duration_ms)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_llm_workflow_runs(
        &self,
        workflow: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LlmWorkflowRunRecord>> {
        let limit = clamp_limit(limit);
        match workflow.filter(|value| !value.trim().is_empty()) {
            Some(workflow) => Ok(sqlx::query_as::<_, LlmWorkflowRunRecord>(
                r#"SELECT id, workflow, prompt_id, prompt_version, model_name, request_payload,
                          response_payload, validation_issues, quality_gate_passed, duration_ms,
                          created_at
                   FROM llm_workflow_runs
                   WHERE workflow = $1
                   ORDER BY created_at DESC, id DESC
                   LIMIT $2"#,
            )
            .bind(workflow.trim())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?),
            None => Ok(sqlx::query_as::<_, LlmWorkflowRunRecord>(
                r#"SELECT id, workflow, prompt_id, prompt_version, model_name, request_payload,
                          response_payload, validation_issues, quality_gate_passed, duration_ms,
                          created_at
                   FROM llm_workflow_runs
                   ORDER BY created_at DESC, id DESC
                   LIMIT $1"#,
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await?),
        }
    }

    pub async fn record_llm_improvement_run(
        &self,
        run_kind: &str,
        run_key: &str,
        metrics: &Value,
        artifacts: &Value,
    ) -> Result<LlmImprovementRunRecord> {
        Ok(sqlx::query_as::<_, LlmImprovementRunRecord>(
            r#"INSERT INTO llm_improvement_runs (run_kind, run_key, metrics, artifacts)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (run_kind, run_key) DO UPDATE SET
                 metrics = EXCLUDED.metrics,
                 artifacts = EXCLUDED.artifacts
               RETURNING id, run_kind, run_key, metrics, artifacts, created_at"#,
        )
        .bind(run_kind)
        .bind(run_key)
        .bind(metrics)
        .bind(artifacts)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_llm_improvement_runs(
        &self,
        run_kind: Option<&str>,
        limit: i64,
    ) -> Result<Vec<LlmImprovementRunRecord>> {
        let limit = clamp_limit(limit);
        match run_kind.filter(|value| !value.trim().is_empty()) {
            Some(run_kind) => Ok(sqlx::query_as::<_, LlmImprovementRunRecord>(
                r#"SELECT id, run_kind, run_key, metrics, artifacts, created_at
                   FROM llm_improvement_runs
                   WHERE run_kind = $1
                   ORDER BY created_at DESC, id DESC
                   LIMIT $2"#,
            )
            .bind(run_kind.trim())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?),
            None => Ok(sqlx::query_as::<_, LlmImprovementRunRecord>(
                r#"SELECT id, run_kind, run_key, metrics, artifacts, created_at
                   FROM llm_improvement_runs
                   ORDER BY created_at DESC, id DESC
                   LIMIT $1"#,
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await?),
        }
    }

    pub async fn record_llm_training_dataset(
        &self,
        dataset_name: &str,
        dataset_version: &str,
        source_run_kind: &str,
        source_run_key: &str,
        manifest: &Value,
        example_count: i64,
        examples_jsonl: &str,
    ) -> Result<LlmTrainingDatasetRecord> {
        Ok(sqlx::query_as::<_, LlmTrainingDatasetRecord>(
            r#"INSERT INTO llm_training_datasets (
                   dataset_name, dataset_version, source_run_kind, source_run_key,
                   manifest, example_count, examples_jsonl
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (dataset_name, dataset_version) DO UPDATE SET
                 source_run_kind = EXCLUDED.source_run_kind,
                 source_run_key = EXCLUDED.source_run_key,
                 manifest = EXCLUDED.manifest,
                 example_count = EXCLUDED.example_count,
                 examples_jsonl = EXCLUDED.examples_jsonl
               RETURNING id, dataset_name, dataset_version, source_run_kind, source_run_key,
                         manifest, example_count, examples_jsonl, created_at"#,
        )
        .bind(dataset_name)
        .bind(dataset_version)
        .bind(source_run_kind)
        .bind(source_run_key)
        .bind(manifest)
        .bind(example_count)
        .bind(examples_jsonl)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_llm_training_datasets(
        &self,
        limit: i64,
    ) -> Result<Vec<LlmTrainingDatasetRecord>> {
        let limit = clamp_limit(limit);
        Ok(sqlx::query_as::<_, LlmTrainingDatasetRecord>(
            r#"SELECT id, dataset_name, dataset_version, source_run_kind, source_run_key,
                      manifest, example_count, examples_jsonl, created_at
               FROM llm_training_datasets
               ORDER BY created_at DESC, id DESC
               LIMIT $1"#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn export_quality_gate_reviewed_warning_golden_set(
        &self,
        accepted_limit: i64,
        rejected_limit: i64,
    ) -> Result<QualityGateGoldenSetExport> {
        let accepted_limit = clamp_limit(accepted_limit).max(1);
        let rejected_limit = clamp_limit(rejected_limit).max(1);

        let rows = sqlx::query_as::<_, ReviewedWarningGoldenSetRow>(
            r#"WITH ranked AS (
                   SELECT id, recipe_code, warning_type, title, description, severity, region,
                          source_urls, confidence, review_outcome, reviewed_at, ts_utc,
                          ROW_NUMBER() OVER (
                              PARTITION BY review_outcome
                              ORDER BY reviewed_at DESC NULLS LAST, ts_utc DESC, id DESC
                          ) AS outcome_rank
                   FROM warnings
                   WHERE review_outcome IN ('true_positive', 'false_positive')
                                         AND deleted_at IS NULL
                     AND coalesce(trim(title), '') <> ''
                     AND coalesce(trim(description), '') <> ''
               )
               SELECT id, recipe_code, warning_type, title, description, severity, region,
                      source_urls, confidence, review_outcome, reviewed_at, ts_utc
               FROM ranked
               WHERE (review_outcome = 'true_positive' AND outcome_rank <= $1)
                  OR (review_outcome = 'false_positive' AND outcome_rank <= $2)
               ORDER BY CASE review_outcome WHEN 'true_positive' THEN 0 ELSE 1 END,
                        reviewed_at DESC NULLS LAST,
                        ts_utc DESC,
                        id DESC"#,
        )
        .bind(accepted_limit)
        .bind(rejected_limit)
        .fetch_all(&self.pool)
        .await?;

        let examples = rows
            .into_iter()
            .map(map_reviewed_warning_golden_example)
            .collect::<Vec<_>>();
        let accepted_count = examples
            .iter()
            .filter(|example| example.historical_label == HistoricalQualityGateLabel::Accepted)
            .count();
        let rejected_count = examples
            .iter()
            .filter(|example| example.historical_label == HistoricalQualityGateLabel::Rejected)
            .count();
        let dataset_version = format!(
            "{}-{}a-{}r",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            accepted_count,
            rejected_count
        );
        let agreement_target = 0.95;
        let examples_jsonl = serialize_quality_gate_examples_jsonl(&examples)?;
        let dataset_manifest = {
            let mut manifest = serde_json::Map::new();
            manifest.insert(
                "dataset_name".to_string(),
                "quality_gate_reviewed_warning_golden_set".into(),
            );
            manifest.insert(
                "dataset_version".to_string(),
                dataset_version.clone().into(),
            );
            manifest.insert("schema".to_string(), "quality_gate_warning_jsonl_v1".into());
            manifest.insert("accepted_target".to_string(), json_value(accepted_limit));
            manifest.insert("rejected_target".to_string(), json_value(rejected_limit));
            manifest.insert("accepted_count".to_string(), json_value(accepted_count));
            manifest.insert("rejected_count".to_string(), json_value(rejected_count));
            manifest.insert("agreement_target".to_string(), json_value(agreement_target));
            manifest.insert("source_kind".to_string(), "reviewed_warnings".into());
            manifest.insert(
                "historical_labels".to_string(),
                json_value(["accepted", "rejected"]),
            );
            serde_json::Value::Object(manifest)
        };
        let dataset = self
            .record_llm_training_dataset(
                "quality_gate_reviewed_warning_golden_set",
                &dataset_version,
                "quality_gate_golden_set_export",
                &dataset_version,
                &dataset_manifest,
                examples.len() as i64,
                &examples_jsonl,
            )
            .await?;

        let export_metrics = {
            let mut metrics = serde_json::Map::new();
            metrics.insert("dataset_id".to_string(), json_value(dataset.id));
            metrics.insert(
                "dataset_version".to_string(),
                dataset.dataset_version.clone().into(),
            );
            metrics.insert("example_count".to_string(), json_value(examples.len()));
            metrics.insert("accepted_count".to_string(), json_value(accepted_count));
            metrics.insert("rejected_count".to_string(), json_value(rejected_count));
            metrics.insert("agreement_target".to_string(), json_value(agreement_target));
            serde_json::Value::Object(metrics)
        };
        let export_artifacts = {
            let preview_ids = examples
                .iter()
                .take(10)
                .map(|example| example.source_id.to_string())
                .collect::<Vec<_>>();
            let mut artifacts = serde_json::Map::new();
            artifacts.insert(
                "dataset_name".to_string(),
                dataset.dataset_name.clone().into(),
            );
            artifacts.insert(
                "dataset_version".to_string(),
                dataset.dataset_version.clone().into(),
            );
            artifacts.insert("preview_ids".to_string(), json_value(preview_ids));
            serde_json::Value::Object(artifacts)
        };
        self.record_llm_improvement_run(
            "quality_gate_golden_set_export",
            &dataset.dataset_version,
            &export_metrics,
            &export_artifacts,
        )
        .await?;

        Ok(QualityGateGoldenSetExport {
            dataset_id: dataset.id,
            dataset_name: dataset.dataset_name,
            dataset_version: dataset.dataset_version,
            example_count: dataset.example_count,
            accepted_count,
            rejected_count,
            agreement_target,
            examples,
        })
    }

    pub async fn list_audit_log(
        &self,
        actor: Option<&str>,
        event_type: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AuditLogRecord>> {
        let limit = clamp_limit(limit);
        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT id, event_type, actor, detail, created_at FROM audit_log",
        );

        let mut has_where = false;
        if let Some(actor) = actor.filter(|value| !value.trim().is_empty()) {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("actor = ").push_bind(actor.trim());
            has_where = true;
        }

        if let Some(event_type) = event_type.filter(|value| !value.trim().is_empty()) {
            qb.push(if has_where { " AND " } else { " WHERE " });
            qb.push("event_type = ").push_bind(event_type.trim());
        }

        qb.push(" ORDER BY created_at DESC, id DESC LIMIT ")
            .push_bind(limit);

        Ok(qb
            .build_query_as::<AuditLogRecord>()
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get_admin_llm_governance_overview(
        &self,
        limit: i64,
    ) -> Result<AdminLlmGovernanceOverview> {
        Ok(AdminLlmGovernanceOverview {
            prompt_versions: self.list_prompt_versions(limit).await?,
            workflow_runs: self.list_llm_workflow_runs(None, limit).await?,
            improvement_runs: self.list_llm_improvement_runs(None, limit).await?,
            training_datasets: self.list_llm_training_datasets(limit).await?,
        })
    }
}
