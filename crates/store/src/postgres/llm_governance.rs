use super::*;

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
