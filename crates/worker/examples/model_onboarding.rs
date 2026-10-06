//! Model onboarding — eval-gated LLM swaps without blind jumps.
//!
//! Probes a candidate LLM endpoint and scores it on the dimensions that
//! matter for this pipeline, so replacing the model is a measured change:
//!
//!   1. liveness and identity (`GET /v1/models`);
//!   2. citation compliance + fabrication resistance on fixed evidence sets
//!      (the analytical claim verifier scores the model's own output);
//!   3. latency profile over repeated calls.
//!
//! Output is JSON: the eval block to store in `llm_model_registry` and the
//! SQL to register/activate the model. Calibration is NOT inherited: the
//! weekly review refits a fresh curve per model from that model's resolved
//! predictions (see `docs/model-portability.md`).
//!
//! Usage:
//!   CANDIDATE_LLM_BASE_URL=http://host:8081 CANDIDATE_LLM_MODEL=Model.gguf \
//!     cargo run -q -p apex-worker --example model_onboarding

use apex_insights::analytical::verification::{verify_claims, VerificationConfig};
use apex_insights::analytical::EvidenceRecord;
use chrono::Utc;
use std::process::Command;
use std::time::Instant;
use uuid::Uuid;

fn main() {
    let base_url = std::env::var("CANDIDATE_LLM_BASE_URL").unwrap_or_else(|_| {
        eprintln!("CANDIDATE_LLM_BASE_URL is required");
        std::process::exit(2);
    });
    let model = std::env::var("CANDIDATE_LLM_MODEL").unwrap_or_else(|_| {
        eprintln!("CANDIDATE_LLM_MODEL is required");
        std::process::exit(2);
    });

    println!("model onboarding: {model} @ {base_url}");

    // 1. Liveness / identity.
    let listing = curl_get(&format!("{}/v1/models", base_url.trim_end_matches('/')));
    let reachable = listing.is_some();
    println!("reachable: {reachable}");

    // 2. Citation compliance + fabrication resistance.
    let evidence = onboarding_evidence();
    let evidence_block = evidence
        .iter()
        .enumerate()
        .map(|(index, record)| format!("[{}] {} — {}", index + 1, record.title, record.text))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "Write a 90-140 word intelligence assessment of the evidence below. Cite evidence as [n]. Use only figures \
that appear in the evidence. End with a one-sentence judgment.\n\nEvidence:\n{evidence_block}"
    );

    let mut latencies = Vec::new();
    let mut factuality_scores = Vec::new();
    let mut hard_violations = 0usize;
    let mut citation_uses = 0usize;
    for _ in 0..3 {
        let started = Instant::now();
        match chat(&base_url, &model, &prompt, 0.3) {
            Some(content) => {
                latencies.push(started.elapsed().as_secs_f64());
                citation_uses += content.matches('[').count();
                let refs: Vec<apex_insights::claims::ClaimEvidenceRef> = evidence
                    .iter()
                    .map(|record| {
                        apex_insights::claims::ClaimEvidenceRef::new(
                            record.evidence_id,
                            record.source_url.clone(),
                        )
                    })
                    .collect();
                let claims = apex_insights::claims::extract_claims(&content, "", &refs, 0.7, None);
                let verification = verify_claims(
                    &claims,
                    &evidence,
                    &VerificationConfig::default(),
                    Utc::now(),
                );
                factuality_scores.push(verification.factuality_score);
                hard_violations += verification.hard_violations.len();
            }
            None => {
                latencies.push(f64::INFINITY);
            }
        }
    }

    let mean = |values: &[f64]| {
        let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.is_empty() {
            None
        } else {
            Some(finite.iter().sum::<f64>() / finite.len() as f64)
        }
    };

    let eval = serde_json::json!({
        "reachable": reachable,
        "runs": latencies.len(),
        "mean_latency_secs": mean(&latencies),
        "mean_factuality": mean(&factuality_scores),
        "hard_violations": hard_violations,
        "citation_markers": citation_uses,
        "onboarded_at": Utc::now().to_rfc3339(),
    });
    println!(
        "eval: {}",
        serde_json::to_string_pretty(&eval).unwrap_or_default()
    );

    let passed = reachable
        && hard_violations == 0
        && mean(&factuality_scores).is_some_and(|score| score >= 0.75);
    println!(
        "\nverdict: {}",
        if passed {
            "PASS — candidate eligible for registry activation (calibration refits from this model's own outcomes)"
        } else {
            "FAIL — do not activate: fabricated content or thin grounding detected"
        }
    );
    println!(
        "-- register with:\nINSERT INTO llm_model_registry (id, provider, display_name, status, eval_scores)\n\
         VALUES ('{model}', 'local', '{model}', 'candidate', '{}');\n-- then activate: SELECT ... (see PgStore::activate_llm_model)",
        serde_json::to_string(&eval).unwrap_or_default()
    );
    if !passed {
        std::process::exit(1);
    }
}

fn onboarding_evidence() -> Vec<EvidenceRecord> {
    vec![
        EvidenceRecord {
            evidence_id: Uuid::from_u128(1),
            title: "Customs filing".into(),
            text: "Duty rates on HS 8534 rose 12% in 2025.".into(),
            source_name: "trade.gov".into(),
            source_url: Some("https://trade.gov.tn/filing".into()),
            signal_type: "government_registry".into(),
            observed_at: None,
            reliability: 0.9,
        },
        EvidenceRecord {
            evidence_id: Uuid::from_u128(2),
            title: "Wire report".into(),
            text: "Buyers re-quoted contracts after the tariff change in 2025.".into(),
            source_name: "reuters.com".into(),
            source_url: Some("https://reuters.com/a".into()),
            signal_type: "news".into(),
            observed_at: None,
            reliability: 0.8,
        },
    ]
}

fn curl_get(url: &str) -> Option<String> {
    let output = Command::new("curl")
        .args(["-s", "-m", "10", url])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
        .filter(|body| !body.trim().is_empty())
}

fn chat(base_url: &str, model: &str, prompt: &str, temperature: f64) -> Option<String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are a rigorous intelligence analyst. Never fabricate figures. Cite evidence as [n]."},
            {"role": "user", "content": prompt}
        ],
        "temperature": temperature,
        "max_tokens": 400
    })
    .to_string();
    let output = Command::new("curl")
        .args([
            "-s",
            "-m",
            "120",
            "-X",
            "POST",
            &format!("{}/v1/chat/completions", base_url.trim_end_matches('/')),
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ])
        .output()
        .ok()?;
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).ok()?;
    parsed["choices"][0]["message"]["content"]
        .as_str()
        .map(str::to_string)
}
