-- ApexIntel Schema Migration – LLM governance, prompt registry, and improvement datasets

CREATE TABLE IF NOT EXISTS prompt_versions (
    prompt_id TEXT NOT NULL,
    version TEXT NOT NULL,
    workflow TEXT NOT NULL,
    system_prompt TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (prompt_id, version)
);

CREATE INDEX IF NOT EXISTS idx_prompt_versions_workflow
    ON prompt_versions (workflow, created_at DESC);

CREATE TABLE IF NOT EXISTS llm_workflow_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow TEXT NOT NULL,
    prompt_id TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    model_name TEXT NOT NULL,
    request_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    response_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    validation_issues JSONB NOT NULL DEFAULT '[]'::jsonb,
    quality_gate_passed BOOLEAN NOT NULL DEFAULT FALSE,
    duration_ms BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (prompt_id, prompt_version) REFERENCES prompt_versions(prompt_id, version)
);

CREATE INDEX IF NOT EXISTS idx_llm_workflow_runs_workflow_created_at
    ON llm_workflow_runs (workflow, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_llm_workflow_runs_prompt
    ON llm_workflow_runs (prompt_id, prompt_version, created_at DESC);

CREATE TABLE IF NOT EXISTS llm_improvement_runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_kind TEXT NOT NULL,
    run_key TEXT NOT NULL,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    artifacts JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (run_kind, run_key)
);

CREATE INDEX IF NOT EXISTS idx_llm_improvement_runs_kind_created_at
    ON llm_improvement_runs (run_kind, created_at DESC);

CREATE TABLE IF NOT EXISTS llm_training_datasets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_name TEXT NOT NULL,
    dataset_version TEXT NOT NULL,
    source_run_kind TEXT NOT NULL,
    source_run_key TEXT NOT NULL,
    manifest JSONB NOT NULL DEFAULT '{}'::jsonb,
    example_count BIGINT NOT NULL DEFAULT 0,
    examples_jsonl TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (dataset_name, dataset_version)
);

CREATE INDEX IF NOT EXISTS idx_llm_training_datasets_name_created_at
    ON llm_training_datasets (dataset_name, created_at DESC);