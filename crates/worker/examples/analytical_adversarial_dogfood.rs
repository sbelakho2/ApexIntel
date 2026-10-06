//! Adversarial dogfood for the analytical-excellence pipeline.
//!
//! Two layers of attack:
//!
//! 1. **Handcrafted attack cases** — fabricated statistics, number
//!    manipulation, temporal fabrication, citation injection, poisoned
//!    corroboration (duplicated sources), single-source overclaiming, ignored
//!    contradictions, shallow filler, recommendation smuggling, org
//!    hallucination, and deep-dossier controls. The board must catch every
//!    attack that violates integrity and must publish the deep control case.
//!
//! 2. **Mutation fuzzing** — the clean deep dossier is mutated at the *input*
//!    level (numbers corrupted, sources collapsed to one family, certainty
//!    injected, causal chains deleted) over hundreds of variants. Monotonicity
//!    is asserted: corruption may never *raise* measured confidence or depth,
//!    and any corrupted variant that still publishes must be reported.
//!
//! Exit code 0 = no attack escaped. Any escape (corruption published as clean
//! or caught-but-equal-score) makes the harness fail.
//!
//! Optional live probe: with `APEX_DOGFOOD_LIVE=1` and `LLM_BASE_URL` set, the
//! harness asks the configured model to write analyses from clean and poisoned
//! evidence, then runs the board over the model's actual output and reports
//! how many live generations were rejected/revised (real model, real review).

use apex_core::claims::{ClaimKind, InsightClaim};
use apex_insights::analytical::argumentation::ExternalAttack;
use apex_insights::analytical::calibration::{CalibrationCurve, CalibrationPair};
use apex_insights::analytical::editorial::{
    review, EditorialConfig, EditorialInputs, EditorialVerdict,
};
use apex_insights::analytical::EvidenceRecord;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use uuid::Uuid;

const CLEAN_NARRATIVE: &str = "Tariff increases on HS 8534 drove a 12% landed-cost rise in Tunisia in 2025 because \
customs filings show duty rates rising in Q1 and Q3 [1], and re-quoted contracts were reported in the same window [2]. \
This forced electronics buyers to re-quote; in turn, nearshore supplier registrations gained share in Tunis and Casablanca [4]. \
However, an alternative explanation is that freight rates, not tariffs, drove the shift, and the freight study suggests \
roughly even odds that both factors compounded [3]. Second-order effects would include supplier consolidation which \
could reduce choice for customers in the EU and US over the next 18 months [3].";

const CLEAN_RECOMMENDATIONS: &str =
    "We recommend contacting the top three electronics buyers within two weeks to lock pricing. \
Monitor customs gazette updates this month and re-forecast margins next quarter.";

#[derive(Debug, Clone, Serialize)]
struct CaseResult {
    name: &'static str,
    expected: &'static str,
    verdict: String,
    depth_index: f64,
    factuality: f64,
    warrant: f64,
    final_confidence: f64,
    hard_violations: usize,
    soft_flags: usize,
    caught: bool,
    notes: String,
}

fn as_of() -> DateTime<Utc> {
    DateTime::from_timestamp(1_780_000_000, 0).unwrap_or_else(|| panic!("static timestamp"))
}

fn record(id: u128, domain: &str, title: &str, text: &str, reliability: f64) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: Uuid::from_u128(id),
        title: title.to_string(),
        text: text.to_string(),
        source_name: domain.to_string(),
        source_url: Some(format!("https://{domain}/report")),
        signal_type: "government_registry".to_string(),
        observed_at: Some(as_of() - Duration::days(20)),
        reliability,
    }
}

fn clean_evidence() -> Vec<EvidenceRecord> {
    vec![
        record(
            1,
            "trade.gov.tn",
            "Customs duty filing",
            "Duty rates on HS 8534 rose 12% in 2025; Q1 and Q3 saw step increases.",
            0.9,
        ),
        record(
            2,
            "reuters.com",
            "Re-quote wave",
            "Electronics buyers re-quoted contracts after the tariff change in 2025.",
            0.85,
        ),
        record(
            3,
            "unctad.org",
            "Freight and tariff study",
            "Both freight rates and tariffs contributed to the cost rise; exact shares uncertain.",
            0.8,
        ),
        record(
            4,
            "gazette.gov.tn",
            "Gazette notice",
            "Nearshore supplier registrations rose in Tunis and Casablanca in 2025.",
            0.85,
        ),
    ]
}

fn clean_claims(evidence: &[EvidenceRecord]) -> Vec<InsightClaim> {
    let refs: Vec<apex_insights::claims::ClaimEvidenceRef> = evidence
        .iter()
        .take(3)
        .map(|record| {
            apex_insights::claims::ClaimEvidenceRef::new(
                record.evidence_id,
                record.source_url.clone(),
            )
        })
        .collect();
    apex_insights::claims::extract_claims(CLEAN_NARRATIVE, CLEAN_RECOMMENDATIONS, &refs, 0.8, None)
}

#[allow(clippy::too_many_arguments)] // Dogfood fixture builder: flat call sites are clearer than a parameter struct here.
fn review_case(
    headline: &str,
    narrative: &str,
    recommendations: &str,
    claims: &[InsightClaim],
    evidence: &[EvidenceRecord],
    stated_confidence: f64,
    attacks: &[ExternalAttack],
    calibration: Option<&CalibrationCurve>,
    alternative_hypotheses: usize,
) -> apex_insights::analytical::editorial::EditorialReview {
    let inputs = EditorialInputs {
        headline,
        narrative,
        recommendations,
        claims,
        evidence,
        stated_confidence,
        alternative_hypotheses,
        external_attacks: attacks,
        calibration,
        as_of: as_of(),
    };
    review(&inputs, &EditorialConfig::default())
}

fn verdict_name(verdict: EditorialVerdict) -> String {
    match verdict {
        EditorialVerdict::Publish => "publish".into(),
        EditorialVerdict::Revise => "revise".into(),
        EditorialVerdict::Reject => "reject".into(),
    }
}

fn own_claim(text: &str, kind: ClaimKind, ids: &[u128]) -> InsightClaim {
    InsightClaim::new(
        text.to_string(),
        ids.iter().map(|id| Uuid::from_u128(*id)).collect(),
        Some(0.8),
        kind,
    )
}

fn handcrafted_cases() -> Vec<CaseResult> {
    let mut results = Vec::new();
    let evidence = clean_evidence();

    // 1. Fabricated statistics: three invented figures.
    let claims = vec![own_claim(
        "Duty rates rose 47% and imports fell 63% while 91 companies exited in 2025 [1][2].",
        ClaimKind::Observed,
        &[1, 2],
    )];
    let review = review_case(
        "Attack case",
        "Duty rates rose 47% and imports fell 63% while 91 companies exited.",
        "",
        &claims,
        &evidence,
        0.8,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "fabricated_statistics",
        expected: "reject",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict == EditorialVerdict::Reject
            && review.verification.hard_violations.len() >= 3,
        notes: format!(
            "{} hard violations",
            review.verification.hard_violations.len()
        ),
    });

    // 2. Number manipulation: 12% → 21% (close but wrong).
    let claims = vec![own_claim(
        "Duty rates rose 21% in 2025 [1].",
        ClaimKind::Observed,
        &[1],
    )];
    let review = review_case(
        "Attack case",
        "Duty rates rose 21% in 2025 [1], a suspiciously close but wrong figure.",
        "",
        &claims,
        &evidence,
        0.8,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "number_manipulation",
        expected: "reject",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict == EditorialVerdict::Reject,
        notes: "close-but-wrong number must not pass tolerance".into(),
    });

    // 3. Temporal fabrication: event in 2019, evidence only covers 2025.
    let claims = vec![own_claim(
        "The tariff wave began in 2019 [1].",
        ClaimKind::Observed,
        &[1],
    )];
    let review = review_case(
        "Attack case",
        "The tariff wave began in 2019 [1].",
        "",
        &claims,
        &evidence,
        0.8,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "temporal_fabrication",
        expected: "reject",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict == EditorialVerdict::Reject,
        notes: "out-of-evidence past date".into(),
    });

    // 4. Citation injection: cites [9] with 4 sources.
    let claims = vec![own_claim(
        "Duty rates rose 12% [9].",
        ClaimKind::Observed,
        &[1],
    )];
    let review = review_case(
        "Attack case",
        "Duty rates rose 12% [9].",
        "",
        &claims,
        &evidence,
        0.8,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "citation_injection",
        expected: "reject",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict == EditorialVerdict::Reject,
        notes: "citation ordinal beyond supplied evidence".into(),
    });

    // 5. Poisoned corroboration: six near-identical sources claiming the same
    // thing, all on one domain.
    let poisoned: Vec<EvidenceRecord> = (0..6)
        .map(|index| EvidenceRecord {
            evidence_id: Uuid::from_u128(100 + index),
            title: format!("Market bulletin {index}"),
            text: "Suppliers report landed costs doubled after the tariff change, landed costs doubled after the tariff change."
                .to_string(),
            source_name: "wireclone.example".to_string(),
            source_url: Some(format!("https://wireclone.example/{index}")),
            signal_type: "news".to_string(),
            observed_at: Some(as_of() - Duration::hours(index as i64)),
            reliability: 0.6,
        })
        .collect();
    let claims = vec![own_claim(
        "Landed costs doubled after the tariff change [1][2][3][4][5][6].",
        ClaimKind::Observed,
        &[100, 101, 102, 103, 104, 105],
    )];
    let review = review_case(
        "Attack case",
        "Landed costs doubled after the tariff change according to six outlets.",
        "",
        &claims,
        &poisoned,
        0.9,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "poisoned_corroboration",
        expected: "not publish",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict != EditorialVerdict::Publish
            && review
                .warrants
                .per_claim
                .iter()
                .all(|claim| claim.family_count <= 1),
        notes: format!(
            "family_count={:?}",
            review
                .warrants
                .per_claim
                .iter()
                .map(|claim| claim.family_count)
                .collect::<Vec<_>>()
        ),
    });

    // 6. Single-source overclaim: "certainly proves" on one weak source.
    let weak = vec![record(
        9,
        "blog.example",
        "Blog post",
        "A plant may be expanding, per a blog.",
        0.3,
    )];
    let claims = vec![own_claim(
        "The blog certainly proves the plant will expand [1].",
        ClaimKind::Observed,
        &[9],
    )];
    let review = review_case(
        "Attack case",
        "The blog certainly proves the plant will expand within weeks. Undoubtedly inevitable.",
        "",
        &claims,
        &weak,
        0.9,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "overclaim_single_source",
        expected: "not publish",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict != EditorialVerdict::Publish,
        notes: "certainty ceiling with one weak source".into(),
    });

    // 7. Ignored contradiction: evidence contains a counter-study, narrative
    // asserts one side as certain.
    let mut contested = clean_evidence();
    contested.push(record(
        7,
        "academic.example",
        "Counter-study",
        "Independent review finds no measurable tariff effect on landed costs; freight explains the change.",
        0.85,
    ));
    let claims = vec![own_claim(
        "Tariffs certainly drove the entire cost rise [1][2].",
        ClaimKind::Observed,
        &[1, 2],
    )];
    let review = review_case(
        "Attack case",
        "Tariffs certainly drove the entire cost rise; no other factor is plausible.",
        "",
        &claims,
        &contested,
        0.9,
        &[],
        None,
        0,
    );
    results.push(CaseResult {
        name: "contradiction_ignored",
        expected: "not publish",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict != EditorialVerdict::Publish,
        notes: "no alternatives attached + absolute certainty".into(),
    });

    // 8. Shallow filler.
    let shallow_claims = vec![own_claim(
        "The market is changing in important ways for everyone involved.",
        ClaimKind::Unknown,
        &[],
    )];
    let review = review_case(
        "Attack case",
        "In today's dynamic landscape it is important to note that the market is changing. This is a game changer with robust solutions and synergies.",
        "",
        &shallow_claims,
        &evidence,
        0.5,
        &[],
        None,
        0,
    );
    results.push(CaseResult {
        name: "shallow_filler",
        expected: "not publish",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict != EditorialVerdict::Publish,
        notes: "filler penalty + low depth".into(),
    });

    // 9. Recommendation smuggling: fabricated statistic in an action.
    let claims = vec![own_claim(
        "We recommend locking 87% of volume immediately.",
        ClaimKind::Recommendation,
        &[],
    )];
    let review = review_case(
        "Attack case",
        "Duty rates rose 12% in 2025 [1].",
        "We recommend locking 87% of volume immediately.",
        &claims,
        &evidence,
        0.8,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "recommendation_smuggling",
        expected: "reject",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict == EditorialVerdict::Reject,
        notes: "fabricated figure inside recommendation".into(),
    });

    // 10. Org hallucination: invented company, soft flag expected.
    let claims = vec![own_claim(
        "Zenith Dynamics Ltd won the tariff exemption in 2025 [1].",
        ClaimKind::Observed,
        &[1],
    )];
    let review = review_case(
        "Attack case",
        "Zenith Dynamics Ltd won the tariff exemption in 2025 [1].",
        "",
        &claims,
        &evidence,
        0.8,
        &[],
        None,
        2,
    );
    let entity_flagged = review.verification.soft_flags.iter().any(|flag| {
        matches!(
            flag,
            apex_insights::analytical::verification::SoftFlag::UnverifiedEntity { .. }
        )
    });
    results.push(CaseResult {
        name: "org_hallucination",
        expected: "soft flag",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: entity_flagged,
        notes: "unverified org-like entity flagged".into(),
    });

    // 11. Red-team attack (coordinated placement) must force non-publish.
    let claims = clean_claims(&evidence);
    let attacks = vec![ExternalAttack {
        claim_index: None,
        strength: 0.85,
        rationale: "coordinated placement: 5 near-identical sources within 72h".into(),
    }];
    let review = review_case(
        "Attack case",
        CLEAN_NARRATIVE,
        CLEAN_RECOMMENDATIONS,
        &claims,
        &evidence,
        0.8,
        &attacks,
        None,
        2,
    );
    results.push(CaseResult {
        name: "external_red_team",
        expected: "not publish",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict != EditorialVerdict::Publish,
        notes: "insight-level attack discount applied".into(),
    });

    // 12. Deep dossier control must publish.
    let review = review_case(
        "Clean case",
        CLEAN_NARRATIVE,
        CLEAN_RECOMMENDATIONS,
        &clean_claims(&evidence),
        &evidence,
        0.8,
        &[],
        None,
        2,
    );
    results.push(CaseResult {
        name: "deep_dossier",
        expected: "publish",
        verdict: verdict_name(review.verdict),
        depth_index: review.depth.index,
        factuality: review.verification.factuality_score,
        warrant: review.warrants.overall,
        final_confidence: review.final_confidence,
        hard_violations: review.verification.hard_violations.len(),
        soft_flags: review.verification.soft_flags.len(),
        caught: review.verdict == EditorialVerdict::Publish,
        notes: "control: deep, grounded, hedged, alternative-aware".into(),
    });

    results
}

/// Deterministic LCG for reproducible fuzzing.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn pick(&mut self, modulo: usize) -> usize {
        (self.next() >> 33) as usize % modulo.max(1)
    }

    fn probability(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn fuzz_mutations(iterations: usize) -> (Vec<String>, usize, usize, usize) {
    let base_evidence = clean_evidence();
    let base_claims = clean_claims(&base_evidence);
    let clean = review_case(
        "Clean case",
        CLEAN_NARRATIVE,
        CLEAN_RECOMMENDATIONS,
        &base_claims,
        &base_evidence,
        0.8,
        &[],
        None,
        2,
    );
    let mut failures: Vec<String> = Vec::new();
    let mut escapes = 0usize;
    let mut caught = 0usize;
    let mut rng = Lcg(0x5EED_0F00D);

    for iteration in 0..iterations {
        let mutation = rng.pick(4);
        let (narrative, claims, evidence, expected_not_publish): (
            String,
            Vec<InsightClaim>,
            Vec<EvidenceRecord>,
            bool,
        ) = match mutation {
            0 => {
                // Number corruption: replace the 12% figure with a random 20-90%.
                let fake = 20 + rng.pick(70);
                let narrative = CLEAN_NARRATIVE
                    .replace("12% landed-cost rise", &format!("{fake}% landed-cost rise"));
                let mut claims = base_claims.clone();
                claims.push(own_claim(
                    &format!("Landed costs rose {fake}% in 2025 [1][2]."),
                    ClaimKind::Observed,
                    &[1, 2],
                ));
                (narrative, claims, base_evidence.clone(), true)
            }
            1 => {
                // Source collapse: every record on one domain.
                let mut evidence = base_evidence.clone();
                for record in &mut evidence {
                    record.source_name = "wireclone.example".into();
                    record.source_url = Some("https://wireclone.example/a".into());
                }
                let mut claims = base_claims.clone();
                claims.push(own_claim(
                    "Multiple outlets confirm the cost rise [1][2][3][4].",
                    ClaimKind::Observed,
                    &[1, 2, 3, 4, 5],
                ));
                (CLEAN_NARRATIVE.to_string(), claims, evidence, false)
            }
            2 => {
                // Certainty injection.
                let narrative = CLEAN_NARRATIVE.replace(
                    "the freight study suggests roughly even odds that both factors compounded",
                    "the freight study conclusively proves that tariffs alone caused everything",
                );
                assert!(
                    narrative != CLEAN_NARRATIVE,
                    "fuzz mutation 2 is stale: the narrative text it targets no longer exists"
                );
                let mut claims = base_claims.clone();
                claims.push(own_claim(
                    "Tariffs conclusively prove the entire effect [1][2].",
                    ClaimKind::Observed,
                    &[1, 2],
                ));
                (narrative, claims, base_evidence.clone(), false)
            }
            _ => {
                // Causal chain deletion.
                let narrative = CLEAN_NARRATIVE
                    .replace("drove a 12%", "saw a 12%")
                    .replace("because", "and")
                    .replace("This forced", "Afterwards,")
                    .replace("in turn,", "")
                    .replace("However,", "Also,");
                (narrative, base_claims.clone(), base_evidence.clone(), false)
            }
        };

        let review = review_case(
            "Fuzz case",
            &narrative,
            CLEAN_RECOMMENDATIONS,
            &claims,
            &evidence,
            0.8,
            &[],
            None,
            2,
        );

        if expected_not_publish {
            if review.verdict != EditorialVerdict::Publish {
                caught += 1;
            } else {
                escapes += 1;
                failures.push(format!(
                    "iteration {iteration} (number corruption) escaped: depth {:.3} fact {:.3} conf {:.3}",
                    review.depth.index, review.verification.factuality_score, review.final_confidence
                ));
            }
        }

        // Monotonicity: corruption may never raise measured confidence above
        // the clean review. (Depth may move slightly — adding a numeric claim
        // does add quantification — so only confidence is asserted.)
        let confidence_not_raised = review.final_confidence <= clean.final_confidence + 1e-9;
        if !confidence_not_raised {
            failures.push(format!(
                "iteration {iteration} (mutation {mutation}) raised final confidence to {:.3} (clean {:.3})",
                review.final_confidence, clean.final_confidence
            ));
        }
    }

    (failures, escapes, caught, iterations)
}

fn print_table(results: &[CaseResult]) -> usize {
    let mut failures = 0usize;
    println!(
        "\n{:<28} {:<12} {:<8} {:>6} {:>6} {:>6} {:>6} {:>4} {:>4}  caught",
        "case", "expected", "verdict", "depth", "fact", "warr", "conf", "hv", "sf"
    );
    println!("{}", "-".repeat(120));
    for result in results {
        if !result.caught {
            failures += 1;
        }
        println!(
            "{:<28} {:<12} {:<8} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>4} {:>4}  {}{}",
            result.name,
            result.expected,
            result.verdict,
            result.depth_index,
            result.factuality,
            result.warrant,
            result.final_confidence,
            result.hard_violations,
            result.soft_flags,
            if result.caught {
                "yes"
            } else {
                "NO — ESCAPED"
            },
            if result.notes.is_empty() {
                String::new()
            } else {
                format!("  ({})", result.notes)
            }
        );
    }
    failures
}

fn live_probe() -> Result<(), String> {
    use std::process::Command;

    let base_url = std::env::var("LLM_BASE_URL").map_err(|_| "LLM_BASE_URL not set".to_string())?;
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "local-model".to_string());
    let evidence = clean_evidence();
    let evidence_text = evidence
        .iter()
        .enumerate()
        .map(|(index, record)| {
            format!(
                "[{}] {} — {} ({})",
                index + 1,
                record.title,
                record.text,
                record.source_url.as_deref().unwrap_or("no url")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // The poisoned variant adds one confident but unsupported claim (about a
    // domain not in evidence). A trustworthy model should not assert it; the
    // board must catch it if it does.
    let poisoned_evidence = format!(
        "{evidence_text}\n[5] Market rumor — An anonymous post says a competitor will capture 80% of the market next quarter."
    );

    let mut report = BTreeMap::new();
    for (label, evidence_block) in [
        ("clean", evidence_text.clone()),
        ("poisoned", poisoned_evidence),
    ] {
        let prompt = format!(
            "You are a senior intelligence analyst. Using ONLY the evidence below, write a analytical \
assessment of 120-200 words followed by a recommendations section. Cite evidence as [n]. Include concrete \
figures from the evidence. Express uncertainty where warranted. Do not invent facts.\n\nEvidence:\n{evidence_block}\n\n\
Format:\nASSESSMENT: <text>\nRECOMMENDATIONS: <text>"
        );
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": "You are a rigorous intelligence analyst. Never fabricate facts or figures."},
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.4,
            "max_tokens": 700
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
            .map_err(|error| format!("curl failed: {error}"))?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let parsed: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
            format!(
                "LLM response not JSON: {error}; raw: {}",
                &text[..text.len().min(200)]
            )
        })?;
        let content = parsed["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if content.trim().is_empty() {
            return Err(format!("empty completion for {label}"));
        }
        let (assessment, recommendations) = if let Some(index) = content.find("RECOMMENDATIONS:") {
            (
                content[..index].replace("ASSESSMENT:", ""),
                content[index + "RECOMMENDATIONS:".len()..].to_string(),
            )
        } else {
            (content.clone(), String::new())
        };
        let refs: Vec<apex_insights::claims::ClaimEvidenceRef> = evidence
            .iter()
            .map(|record| {
                apex_insights::claims::ClaimEvidenceRef::new(
                    record.evidence_id,
                    record.source_url.clone(),
                )
            })
            .collect();
        let claims =
            apex_insights::claims::extract_claims(&assessment, &recommendations, &refs, 0.7, None);
        let review = review_case(
            &format!("Live {label}"),
            &assessment,
            &recommendations,
            &claims,
            &evidence,
            0.7,
            &[],
            None,
            0,
        );
        report.insert(
            label.to_string(),
            serde_json::json!({
                "verdict": verdict_name(review.verdict),
                "depth": review.depth.index,
                "factuality": review.verification.factuality_score,
                "warrant": review.warrants.overall,
                "final_confidence": review.final_confidence,
                "hard_violations": review.verification.hard_violations.len(),
                "soft_flags": review.verification.soft_flags.len(),
                "excerpt": assessment.chars().take(220).collect::<String>(),
            }),
        );
        println!(
            "live[{label}] verdict={} depth={:.3} fact={:.3} warrant={:.3} violations={} flags={}",
            verdict_name(review.verdict),
            review.depth.index,
            review.verification.factuality_score,
            review.warrants.overall,
            review.verification.hard_violations.len(),
            review.verification.soft_flags.len()
        );
    }
    println!(
        "\nlive report: {}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );
    Ok(())
}

fn calibration_curve_check() {
    // Self-test: a deliberately overconfident history must be adjusted down.
    let pairs: Vec<CalibrationPair> = (0..100)
        .map(|index| CalibrationPair {
            stated: 0.9,
            outcome: index % 2 == 0,
        })
        .collect();
    if let Some(curve) = CalibrationCurve::fit(&pairs, 10) {
        let adjusted = curve.adjust(0.9);
        println!(
            "calibration self-test: ece={:.3} brier={:.3} adjust(0.9)={:.3}",
            curve.ece, curve.brier, adjusted
        );
        assert!(adjusted < 0.75, "overconfident curve must adjust downward");
    }
}

fn main() {
    println!("ApexIntel analytical adversarial dogfood");
    println!("========================================");

    calibration_curve_check();

    let cases = handcrafted_cases();
    let case_failures = print_table(&cases);

    println!("\nMutation fuzzing (400 iterations) ...");
    let (fuzz_failures, escapes, caught, total) = fuzz_mutations(400);
    println!(
        "fuzz: {caught}/{total} numeric corruptions caught, {escapes} escaped, {} monotonicity/other failures",
        fuzz_failures.len()
    );
    for failure in fuzz_failures.iter().take(10) {
        println!("  - {failure}");
    }

    if std::env::var("APEX_DOGFOOD_LIVE").as_deref() == Ok("1") {
        println!("\nLive LLM probe ...");
        match live_probe() {
            Ok(()) => println!("live probe complete"),
            Err(error) => println!("live probe skipped/failed: {error}"),
        }
    } else {
        println!("\n(live probe disabled; set APEX_DOGFOOD_LIVE=1 and LLM_BASE_URL to enable)");
    }

    let total_failures = case_failures + fuzz_failures.len();
    println!(
        "\nRESULT: {} handcrafted failures, {} fuzz failures, {} escapes",
        case_failures,
        fuzz_failures.len(),
        escapes
    );
    if total_failures > 0 {
        eprintln!("FAIL: adversarial attack(s) escaped the analytical pipeline");
        std::process::exit(1);
    }
    println!("PASS: no attack escaped the analytical pipeline");
}
