//! Agentic hypothesis-generation wiring tests.
//!
//! Covers the manual `JobKind::HypothesisGeneration` path added in W3:
//!
//! * **(a)** the agent path invokes `run_agent_loop` and advertises the live
//!   tool catalog in the system prompt;
//! * **(b)** an invalid final answer becomes a `ValidationFailed` and stages
//!   nothing;
//! * **(c)** a valid final answer stages exactly one `staging` recipe through
//!   the production staging/namespacing code;
//! * **(d)** the scheduled batch mode still calls the batch generator
//!   (`generate_json`), never the multi-turn agent loop.
//!
//! Pure, in-process tests use a scripted OpenAI-compatible HTTP endpoint and
//! need no live model; the PostgreSQL-backed tests are `#[ignore]`d and run
//! from `scripts/ci/run_pg_integration_suites.sh` against an isolated database
//! (e.g. `apexintel_ci_w3`).
//!
//! `job_execution` lives in the `apex-worker` *binary*, so the production
//! module is included via `#[path]`; its code is unmodified production source
//! and it resolves everything through absolute crate paths.
#![cfg(feature = "llm")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use apex_learning::generate::HypothesisResult;
use apex_learning::miner::PatternCandidate;
use apex_llm::function_calling::{
    FunctionSpec, ParamSchema, ParamType, Tool, ToolError, ToolRegistry,
};
use apex_llm::LlmClient;
use apex_store::postgres::PgStore;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[path = "../src/job_execution/agent_tools.rs"]
mod agent_tools;

use agent_tools::{
    build_store_tool_registry, generate_hypotheses_for_mode, stage_hypothesis_results,
    HypothesisGenerationMode,
};

// ─── Fixtures ────────────────────────────────────────────────────────────────

/// Candidate matching [`valid_recipe_json`] (stats permit min_effect 2.0).
fn candidate() -> PatternCandidate {
    PatternCandidate {
        outcome: "RfQPosted".to_string(),
        signals: vec!["WebChange.portal".to_string()],
        best_lag_days: 30,
        effect_size: 3.2,
        odds_ratio_ci_low: Some(1.8),
        odds_ratio_ci_high: Some(5.7),
        minimum_detectable_effect: 1.6,
        p_value: 0.004,
        q_value: 0.02,
        stability: 0.8,
        entity_coverage: 0.45,
        segments: Vec::new(),
        contingency: (24, 4, 4, 18),
    }
}

/// A recipe that passes `validate_hypothesis` against [`candidate`].
fn valid_recipe_json() -> String {
    json!({
        "id": "agentic_w3_recipe",
        "join": "Entity",
        "outcome": "RfQPosted",
        "signals": ["WebChange.portal"],
        "transforms": [{"type": "Lag", "days": 30}],
        "test": {"type": "FisherExact"},
        "thresholds": {
            "min_effect": 2.0,
            "max_p_value": 0.01,
            "min_stability": 0.7,
            "max_false_alarm_rate": 0.05,
        },
        "narrative_template": "Tender activity: {{evidence:WebChange.portal}}; review procurement.",
        "action_playbook": ["Contact the procurement lead"],
        "applicability": {"geos": ["TN"], "industries": ["EMS"], "notes": ""},
    })
    .to_string()
}

/// Minimal in-memory tool, standing in for a store tool in pure tests.
struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn spec(&self) -> FunctionSpec {
        FunctionSpec {
            name: "echo".to_string(),
            description: "Echo a message back.".to_string(),
            parameters: [(
                "msg".to_string(),
                ParamSchema {
                    kind: ParamType::String,
                    description: Some("message".to_string()),
                    enum_values: None,
                },
            )]
            .into_iter()
            .collect(),
            required: vec!["msg".to_string()],
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<Value, ToolError> {
        Ok(json!({ "echoed": arguments.get("msg").cloned().unwrap_or(Value::Null) }))
    }
}

// ─── Scripted OpenAI-compatible endpoint (W1 pattern) ────────────────────────

fn chat_completion(content: &str) -> String {
    json!({
        "choices": [{
            "message": { "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
    })
    .to_string()
}

async fn read_http_request_body(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    use tokio::io::AsyncReadExt;

    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(header_end) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            while buf.len() < body_start + content_length {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..read]);
            }
            return buf[body_start..(body_start + content_length).min(buf.len())].to_vec();
        }
        let read = socket.read(&mut chunk).await.unwrap();
        if read == 0 {
            return buf;
        }
        buf.extend_from_slice(&chunk[..read]);
    }
}

async fn mock_llm_server(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::AsyncWriteExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut captured = Vec::with_capacity(responses.len());
        for response in responses {
            let accepted =
                tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept()).await;
            let Ok(Ok((mut socket, _))) = accepted else {
                break;
            };
            let body = read_http_request_body(&mut socket).await;
            captured.push(String::from_utf8_lossy(&body).to_string());
            let payload = response.as_bytes();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            socket.write_all(head.as_bytes()).await.unwrap();
            socket.write_all(payload).await.unwrap();
            socket.flush().await.unwrap();
            let _ = socket.shutdown().await;
        }
        captured
    });
    (format!("http://{addr}"), handle)
}

fn test_client(base_url: &str) -> apex_llm::inference::LlmClient {
    apex_llm::inference::LlmClient::new(
        base_url,
        None,
        apex_llm::inference::InferenceConfig {
            max_retries: 0,
            timeout: std::time::Duration::from_secs(10),
            ..Default::default()
        },
    )
}

/// In-process client that records which generation entry point was used.
struct ModeRecordingClient {
    json_calls: AtomicUsize,
    multi_turn_calls: AtomicUsize,
}

impl ModeRecordingClient {
    fn new() -> Self {
        Self {
            json_calls: AtomicUsize::new(0),
            multi_turn_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmClient for ModeRecordingClient {
    async fn generate_json(&self, _system: &str, _user: &str) -> anyhow::Result<String> {
        self.json_calls.fetch_add(1, Ordering::SeqCst);
        Ok(valid_recipe_json())
    }

    async fn generate_text(&self, _system: &str, _user: &str) -> anyhow::Result<String> {
        anyhow::bail!("generate_text must not be used by either generation path")
    }

    async fn complete_messages(
        &self,
        _messages: Vec<apex_llm::inference::ChatMessage>,
        _config: &apex_llm::inference::InferenceConfig,
    ) -> anyhow::Result<String> {
        self.multi_turn_calls.fetch_add(1, Ordering::SeqCst);
        Ok(valid_recipe_json())
    }
}

// ─── (a) + (c): agent loop advertises the tool catalog and accepts an answer ─

#[tokio::test]
async fn agentic_path_advertises_tool_catalog_and_accepts_valid_answer() {
    let tool_call = r#"{"tool_calls":[{"name":"echo","arguments":{"msg":"hi"}}]}"#;
    let (url, server) = mock_llm_server(vec![
        chat_completion(tool_call),
        chat_completion(&valid_recipe_json()),
    ])
    .await;
    let client = test_client(&url);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let results = generate_hypotheses_for_mode(
        HypothesisGenerationMode::Agentic,
        &client,
        &registry,
        &[candidate()],
        &[],
    )
    .await;

    match &results[0] {
        HypothesisResult::Success(hyp) => assert_eq!(hyp.id, "agentic_w3_recipe"),
        other => panic!("expected Success, got {other:?}"),
    }

    let captured = server.await.unwrap();
    assert_eq!(captured.len(), 2, "one tool turn plus the final answer");
    assert!(
        captured[0].contains("Available tools"),
        "the system prompt must advertise the tool catalog: {}",
        captured[0]
    );
    assert!(
        captured[0].contains("echo"),
        "the registered tool must be named in the system prompt: {}",
        captured[0]
    );
    assert!(
        captured[1].contains("[tool_result]"),
        "the post-tool request must carry the tool result turn: {}",
        captured[1]
    );
}

// ─── (b): invalid final answer is a validation failure ───────────────────────

#[tokio::test]
async fn agentic_invalid_final_answer_is_a_validation_failure() {
    let (url, server) = mock_llm_server(vec![chat_completion("I could not find a pattern.")]).await;
    let client = test_client(&url);
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(EchoTool));

    let results = generate_hypotheses_for_mode(
        HypothesisGenerationMode::Agentic,
        &client,
        &registry,
        &[candidate()],
        &[],
    )
    .await;

    match &results[0] {
        HypothesisResult::ValidationFailed {
            candidate_outcome,
            issues,
        } => {
            assert_eq!(candidate_outcome, "RfQPosted");
            assert!(
                issues.iter().any(|issue| issue.contains("Parse error")),
                "parse failures must be reported as validation issues: {issues:?}"
            );
        }
        other => panic!("expected ValidationFailed, got {other:?}"),
    }
    assert_eq!(server.await.unwrap().len(), 1);
}

// ─── (d): scheduled batch mode keeps the batch generator ─────────────────────

#[tokio::test]
async fn scheduled_batch_mode_never_uses_the_agent_loop() {
    let client = ModeRecordingClient::new();
    let registry = ToolRegistry::new();

    let results = generate_hypotheses_for_mode(
        HypothesisGenerationMode::Batch,
        &client,
        &registry,
        &[candidate()],
        &[],
    )
    .await;

    assert!(matches!(&results[0], HypothesisResult::Success(_)));
    assert_eq!(
        client.json_calls.load(Ordering::SeqCst),
        1,
        "the scheduled batch path must call the single-completion generator"
    );
    assert_eq!(
        client.multi_turn_calls.load(Ordering::SeqCst),
        0,
        "the scheduled batch path must never enter the multi-turn agent loop"
    );
}

#[tokio::test]
async fn agentic_mode_never_uses_the_single_completion_generator() {
    let client = ModeRecordingClient::new();
    let registry = ToolRegistry::new();

    let results = generate_hypotheses_for_mode(
        HypothesisGenerationMode::Agentic,
        &client,
        &registry,
        &[candidate()],
        &[],
    )
    .await;

    assert!(matches!(&results[0], HypothesisResult::Success(_)));
    assert_eq!(
        client.multi_turn_calls.load(Ordering::SeqCst),
        1,
        "the agentic path must call the multi-turn loop"
    );
    assert_eq!(
        client.json_calls.load(Ordering::SeqCst),
        0,
        "the agentic path must not fall back to the single-completion generator"
    );
}

// ─── PostgreSQL-backed staging tests ─────────────────────────────────────────

#[cfg(test)]
mod pg {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    /// Delete recipes staged by these tests so re-runs stay deterministic.
    const W3_RECIPE_PREFIX: &str = "mined_agentic_w3";

    async fn connect() -> sqlx::PgPool {
        let url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("TEST_DATABASE_URL or DATABASE_URL must be set");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("connect to postgres");
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        sqlx::query("DELETE FROM recipes WHERE code LIKE $1")
            .bind(format!("{W3_RECIPE_PREFIX}%"))
            .execute(&pool)
            .await
            .expect("clear w3 recipes");
        pool
    }

    async fn staged_count(pool: &sqlx::PgPool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM recipes WHERE code LIKE $1")
            .bind(format!("{W3_RECIPE_PREFIX}%"))
            .fetch_one(pool)
            .await
            .expect("count w3 recipes")
    }

    /// (a) + (c): the real registry is advertised and used, the valid final
    /// answer is staged exactly once through the production staging code.
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn agentic_valid_answer_stages_exactly_one_staging_recipe() {
        let pool = connect().await;
        let store = Arc::new(PgStore::from_pool(pool.clone()));
        let registry = build_store_tool_registry(store.clone());
        let candidate = candidate();

        // The scripted tool call executes against the real store: the agent
        // must be able to read mining stats before answering.
        let tool_call =
            r#"{"tool_calls":[{"name":"get_mining_stats","arguments":{"since_hours":24}}]}"#;
        let (url, server) = mock_llm_server(vec![
            chat_completion(tool_call),
            chat_completion(&valid_recipe_json()),
        ])
        .await;
        let client = test_client(&url);

        let results = generate_hypotheses_for_mode(
            HypothesisGenerationMode::Agentic,
            &client,
            &registry,
            std::slice::from_ref(&candidate),
            &[],
        )
        .await;
        assert!(
            matches!(&results[0], HypothesisResult::Success(_)),
            "expected a validated hypothesis, got {:?}",
            results[0]
        );

        let captured = server.await.unwrap();
        let first_request: Value =
            serde_json::from_str(&captured[0]).expect("captured request must be JSON");
        let system_prompt = first_request["messages"][0]["content"]
            .as_str()
            .unwrap_or_default();
        assert!(
            system_prompt.contains("get_mining_stats")
                && system_prompt.contains("get_company_dossier"),
            "the production registry must be advertised to the model"
        );
        assert!(
            captured[1].contains("[tool_result]") && captured[1].contains("candidates_found"),
            "the real store tool result must be fed back: {}",
            captured[1]
        );

        let existing_codes = store.list_recipe_codes().await.expect("list codes");
        let outcome =
            stage_hypothesis_results(&store, &[candidate], &results, &existing_codes).await;
        assert_eq!(outcome.submitted, 1);
        assert_eq!(outcome.generated, 1);
        assert_eq!(outcome.staged, 1, "errors: {:?}", outcome.errors);
        assert!(outcome.errors.is_empty());

        let (status, definition): (String, Value) = sqlx::query_as(
            "SELECT status, definition FROM recipes WHERE code = 'mined_agentic_w3_recipe'",
        )
        .fetch_one(&pool)
        .await
        .expect("staged recipe row");
        assert_eq!(status, "staging");
        assert_eq!(definition["category"], "mined");
        assert_eq!(definition["provenance"]["source"], "pattern_mining");
        assert_eq!(definition["outcome"], "RfQPosted");
        assert_eq!(staged_count(&pool).await, 1, "exactly one recipe staged");
    }

    /// (b): an invalid final answer stages nothing, and the failure is not
    /// hidden (generated == 0, failed == 1, zero rows written).
    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn agentic_invalid_answer_stages_nothing() {
        let pool = connect().await;
        let store = Arc::new(PgStore::from_pool(pool.clone()));
        let registry = build_store_tool_registry(store.clone());
        let candidate = candidate();

        let (url, _server) =
            mock_llm_server(vec![chat_completion("no JSON here, just prose")]).await;
        let client = test_client(&url);

        let results = generate_hypotheses_for_mode(
            HypothesisGenerationMode::Agentic,
            &client,
            &registry,
            std::slice::from_ref(&candidate),
            &[],
        )
        .await;
        assert!(
            matches!(&results[0], HypothesisResult::ValidationFailed { .. }),
            "an invalid answer must be a validation failure, got {:?}",
            results[0]
        );

        let existing_codes = store.list_recipe_codes().await.expect("list codes");
        let outcome =
            stage_hypothesis_results(&store, &[candidate], &results, &existing_codes).await;
        assert_eq!(outcome.submitted, 1);
        assert_eq!(outcome.generated, 0);
        assert_eq!(outcome.staged, 0);
        assert_eq!(outcome.failed, 1);
        assert_eq!(
            staged_count(&pool).await,
            0,
            "an invalid answer must not stage any recipe"
        );
    }
}
