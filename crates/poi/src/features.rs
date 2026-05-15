//! POI feature computation from artifacts.

use crate::model::*;

const MAX_KEYWORD_HITS_PER_TERM: usize = 5;

/// Keyword categories used for priority vector computation.
/// Covers English, Arabic, French, German, Spanish, Hebrew, Korean,
/// Japanese, Chinese and Russian signals in a single pass.
const KEYWORD_CATEGORIES: &[(&str, &[&str])] = &[
    (
        "cost",
        &[
            // EN
            "cost",
            "price",
            "budget",
            "savings",
            "tco",
            "should-cost",
            "capex",
            "opex",
            "spend",
            "expenditure",
            "affordable",
            "cheap",
            "overrun",
            "invoice",
            "roi",
            "payback",
            "margin",
            "profitability",
            "financial",
            "fiscal",
            // AR
            "تكلفة",
            "سعر",
            "ميزانية",
            "وفورات",
            "إنفاق",
            // FR
            "coût",
            "prix",
            "budget",
            "économies",
            "dépense",
            // DE
            "kosten",
            "preis",
            "budget",
            "einsparung",
            // ES
            "costo",
            "precio",
            "presupuesto",
            "ahorro",
            // HE
            "עלות",
            "מחיר",
            "תקציב",
            // KO
            "비용",
            "가격",
            "예산",
            // JA
            "コスト",
            "価格",
            "予算",
            // ZH
            "成本",
            "价格",
            "预算",
            // RU
            "стоимость",
            "бюджет",
        ],
    ),
    (
        "quality",
        &[
            // EN
            "quality",
            "ppm",
            "defect",
            "yield",
            "zero defects",
            "six sigma",
            "iso 9001",
            "inspection",
            "reliability",
            "durability",
            "tolerance",
            "precision",
            "accuracy",
            "validation",
            "testing",
            "audit trail",
            "traceability",
            "asq",
            "apqp",
            "ppap",
            // AR
            "جودة",
            "عيب",
            "موثوقية",
            // FR
            "qualité",
            "défaut",
            "rendement",
            "fiabilité",
            // DE
            "qualität",
            "fehler",
            "zuverlässigkeit",
            // ES
            "calidad",
            "defecto",
            "confiabilidad",
            // HE
            "איכות",
            "פגם",
            // KO
            "품질",
            "결함",
            "수율",
            // JA
            "品質",
            "不良",
            "歩留まり",
            // ZH
            "质量",
            "缺陷",
            "良率",
        ],
    ),
    (
        "speed",
        &[
            // EN
            "speed",
            "lead time",
            "fast",
            "agile",
            "npi",
            "time-to-market",
            "delivery",
            "turnaround",
            "velocity",
            "throughput",
            "expedite",
            "urgent",
            "asap",
            "rapid",
            "quick",
            "swift",
            "on-time",
            "runway",
            "sprint",
            // AR
            "سرعة",
            "وقت التسليم",
            "عاجل",
            // FR
            "rapidité",
            "délai",
            "livraison",
            "urgent",
            // DE
            "schnelligkeit",
            "lieferzeit",
            "dringend",
            // ES
            "velocidad",
            "entrega",
            "urgente",
            // KO
            "속도",
            "납기",
            "긴급",
            // JA
            "スピード",
            "リードタイム",
            "緊急",
            // ZH
            "速度",
            "交期",
            "紧急",
        ],
    ),
    (
        "resilience",
        &[
            // EN
            "resilience",
            "risk",
            "disruption",
            "continuity",
            "dual source",
            "buffer stock",
            "backup",
            "redundancy",
            "contingency",
            "recovery",
            "bcp",
            "drp",
            "disaster",
            "shortage",
            "scarcity",
            "geopolitical",
            "vulnerability",
            "exposure",
            "volatility",
            "diversification",
            "nearshoring",
            "reshoring",
            "friendshoring",
            // AR
            "مرونة",
            "مخاطر",
            "استمرارية",
            "نقص",
            // FR
            "résilience",
            "risque",
            "continuité",
            "pénurie",
            "rupture",
            // DE
            "resilienz",
            "risiko",
            "kontinuität",
            "versorgungsengpass",
            // ES
            "resiliencia",
            "riesgo",
            "continuidad",
            // HE
            "חוסן",
            "סיכון",
            // KO
            "회복력",
            "위험",
            "연속성",
            // ZH
            "弹性",
            "风险",
            "供应中断",
        ],
    ),
    (
        "compliance",
        &[
            // EN
            "compliance",
            "audit",
            "regulation",
            "standard",
            "certification",
            "gdpr",
            "sox",
            "hipaa",
            "iso",
            "itar",
            "ear",
            "sanctions",
            "aml",
            "kyc",
            "esg",
            "csrd",
            "due diligence",
            "reporting",
            "transparency",
            "governance",
            "fiduciary",
            "regulatory",
            "licensing",
            "accreditation",
            // AR
            "امتثال",
            "تدقيق",
            "تنظيم",
            "شهادة",
            // FR
            "conformité",
            "audit",
            "réglementation",
            "certification",
            // DE
            "compliance",
            "regulierung",
            "zertifizierung",
            // ES
            "cumplimiento",
            "auditoría",
            "regulación",
            // HE
            "ציות",
            "ביקורת",
            "רגולציה",
            // KO
            "준수",
            "감사",
            "규정",
            // ZH
            "合规",
            "审计",
            "监管",
        ],
    ),
    (
        "security",
        &[
            // EN
            "security",
            "cyber",
            "dmarc",
            "breach",
            "zero trust",
            "soc2",
            "pentest",
            "ransomware",
            "phishing",
            "intrusion",
            "vulnerability",
            "patch",
            "cve",
            "ciso",
            "siem",
            "iam",
            "encryption",
            "privacy",
            "data loss",
            "dlp",
            "national security",
            "defense",
            "classified",
            "clearance",
            "intelligence",
            // AR
            "أمن",
            "سيبراني",
            "اختراق",
            "دفاع",
            // FR
            "sécurité",
            "cyber",
            "violation",
            "défense",
            // DE
            "sicherheit",
            "cyber",
            "datenschutz",
            // ES
            "seguridad",
            "ciberseguridad",
            "privacidad",
            // HE
            "אבטחה",
            "סייבר",
            "ביטחון",
            // KO
            "보안",
            "사이버",
            "방어",
            // ZH
            "安全",
            "网络安全",
            "防御",
        ],
    ),
];

/// Locale-specific keyword extensions for priority vector (B130).
/// Callers may provide additional keyword sets keyed by (category, locale).
pub type LocaleKeywords = Vec<(&'static str, &'static str, &'static [&'static str])>;

/// Get default locale extensions for common markets.
pub fn default_locale_keywords() -> LocaleKeywords {
    vec![
        ("cost", "ko", &["비용", "가격", "예산"]),
        ("quality", "ko", &["품질", "결함", "수율"]),
        ("cost", "ja", &["コスト", "価格", "予算"]),
        ("quality", "ja", &["品質", "不良", "歩留まり"]),
        ("cost", "zh", &["成本", "价格", "预算"]),
        ("quality", "zh", &["质量", "缺陷", "良率"]),
        ("cost", "he", &["עלות", "מחיר", "תקציב"]),
        ("quality", "he", &["איכות", "פגם"]),
    ]
}

/// Compute priority vector from artifact text analysis.
pub fn compute_priority_vector(artifacts: &[PoiArtifact]) -> PriorityVector {
    if artifacts.is_empty() {
        return PriorityVector::zero();
    }

    let total_text: String = artifacts
        .iter()
        .map(|a| format!("{} {}", a.title, a.content_summary))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    let mut scores = [0.0_f64; 6];
    let mut total = 0.0;

    for (i, (_, keywords)) in KEYWORD_CATEGORIES.iter().enumerate() {
        for kw in *keywords {
            scores[i] += bounded_keyword_hits(&total_text, kw) as f64;
        }
        total += scores[i];
    }

    if total > 0.0 {
        for s in &mut scores {
            *s /= total;
        }
    }

    PriorityVector {
        cost: scores[0],
        quality: scores[1],
        speed: scores[2],
        resilience: scores[3],
        compliance: scores[4],
        security: scores[5],
        confidence: (artifacts.len() as f64 / 20.0).min(1.0),
    }
}

/// Compute priority vector with optional locale-specific keyword extensions (B130).
pub fn compute_priority_vector_with_locale(
    artifacts: &[PoiArtifact],
    locale_keywords: &LocaleKeywords,
) -> PriorityVector {
    if artifacts.is_empty() {
        return PriorityVector::zero();
    }

    let total_text: String = artifacts
        .iter()
        .map(|a| format!("{} {}", a.title, a.content_summary))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    let category_names = [
        "cost",
        "quality",
        "speed",
        "resilience",
        "compliance",
        "security",
    ];
    let mut scores = [0.0_f64; 6];
    let mut total = 0.0;

    // Base keywords
    for (i, (_, keywords)) in KEYWORD_CATEGORIES.iter().enumerate() {
        for kw in *keywords {
            scores[i] += bounded_keyword_hits(&total_text, kw) as f64;
        }
    }

    // Locale extensions
    for (cat, _locale, keywords) in locale_keywords {
        if let Some(idx) = category_names.iter().position(|c| c == cat) {
            for kw in *keywords {
                scores[idx] += bounded_keyword_hits(&total_text, kw) as f64;
            }
        }
    }

    for s in &scores {
        total += s;
    }

    if total > 0.0 {
        for s in &mut scores {
            *s /= total;
        }
    }

    PriorityVector {
        cost: scores[0],
        quality: scores[1],
        speed: scores[2],
        resilience: scores[3],
        compliance: scores[4],
        security: scores[5],
        confidence: (artifacts.len() as f64 / 20.0).min(1.0),
    }
}

fn bounded_keyword_hits(text: &str, keyword: &str) -> usize {
    text.matches(keyword).count().min(MAX_KEYWORD_HITS_PER_TERM)
}

/// Return a role-family-aware default priority vector when no artifacts exist.
/// These defaults reflect what each function cares about most.
pub fn default_priority_vector_for_role(role_family: &str) -> PriorityVector {
    let lower = role_family.to_lowercase();
    match lower.as_str() {
        "procurement" | "sourcing" | "purchasing" | "supply_chain" | "supply chain" => {
            PriorityVector {
                cost: 0.35,
                quality: 0.25,
                speed: 0.15,
                resilience: 0.15,
                compliance: 0.05,
                security: 0.05,
                confidence: 0.3,
            }
        }
        "engineering" | "technology" | "r&d" | "research" => PriorityVector {
            cost: 0.10,
            quality: 0.30,
            speed: 0.20,
            resilience: 0.10,
            compliance: 0.10,
            security: 0.20,
            confidence: 0.3,
        },
        "quality" | "supplier_quality" | "supplier quality" | "compliance" => PriorityVector {
            cost: 0.05,
            quality: 0.40,
            speed: 0.05,
            resilience: 0.15,
            compliance: 0.30,
            security: 0.05,
            confidence: 0.3,
        },
        "operations" | "manufacturing" | "logistics" => PriorityVector {
            cost: 0.20,
            quality: 0.20,
            speed: 0.30,
            resilience: 0.20,
            compliance: 0.05,
            security: 0.05,
            confidence: 0.3,
        },
        "security" | "cyber" | "information security" => PriorityVector {
            cost: 0.05,
            quality: 0.10,
            speed: 0.05,
            resilience: 0.20,
            compliance: 0.20,
            security: 0.40,
            confidence: 0.3,
        },
        "finance" | "accounting" => PriorityVector {
            cost: 0.40,
            quality: 0.10,
            speed: 0.10,
            resilience: 0.10,
            compliance: 0.25,
            security: 0.05,
            confidence: 0.3,
        },
        "government" | "military" | "regulatory" => PriorityVector {
            cost: 0.10,
            quality: 0.15,
            speed: 0.05,
            resilience: 0.20,
            compliance: 0.35,
            security: 0.15,
            confidence: 0.3,
        },
        _ => PriorityVector::zero(),
    }
}

/// Infer decision style from priority vector.
pub fn infer_decision_style(pv: &PriorityVector) -> DecisionStyle {
    let dominant = pv.dominant();
    let max_val = match dominant {
        "cost" => pv.cost,
        "quality" => pv.quality,
        "speed" => pv.speed,
        "resilience" => pv.resilience,
        "compliance" => pv.compliance,
        "security" => pv.security,
        _ => 0.0,
    };

    // If no dimension dominates strongly, they're balanced
    if !max_val.is_finite() || max_val < 0.25 {
        return DecisionStyle::BalancedAnalytical;
    }

    match dominant {
        "cost" => DecisionStyle::CostFirst,
        "quality" => DecisionStyle::QualityFirst,
        "speed" => DecisionStyle::SpeedFirst,
        "resilience" | "security" => DecisionStyle::RiskFirst,
        "compliance" => DecisionStyle::ComplianceFirst,
        _ => DecisionStyle::BalancedAnalytical,
    }
}

/// Compute influence score: 0.35*centrality + 0.25*seniority + 0.40*recurrence.
/// Clamps to [0, 100] and guards against negative inputs (B112).
pub fn compute_influence_score(
    graph_centrality: f64,
    role_seniority: f64,
    public_recurrence: f64,
) -> f64 {
    let gc = graph_centrality.max(0.0);
    let rs = role_seniority.max(0.0);
    let pr = public_recurrence.max(0.0);
    let raw = 0.35 * gc + 0.25 * rs + 0.40 * pr;
    raw.clamp(0.0, 100.0)
}

/// Map role title to seniority score (0-100).
pub fn role_seniority_score(title: &str) -> f64 {
    let lower = title.to_lowercase();

    // C-level outranks director. Use token-level matching for 3-letter acronyms
    // to avoid false positives: "director" contains "cto" as a substring.
    let tokens: std::collections::HashSet<&str> = lower.split_whitespace().collect();
    if tokens.contains("ceo")
        || tokens.contains("cto")
        || tokens.contains("cfo")
        || tokens.contains("coo")
        || tokens.contains("cpo")
        || lower.contains("chief")
    {
        return 95.0;
    }
    // VP outranks Director; check first so "VP & Director" gets 85, not 70.
    if tokens.contains("vp") || lower.contains("vice president") {
        return 85.0;
    }
    if lower.contains("director") {
        return 70.0;
    }
    if lower.contains("senior manager") {
        return 60.0;
    }
    // Non-exec buyer/procurement roles with decision authority (Fix 8)
    if lower.contains("head of procurement")
        || lower.contains("head of sourcing")
        || lower.contains("head of purchasing")
        || lower.contains("head of supply chain")
        || lower.contains("head of quality")
        || lower.contains("head of engineering")
        || lower.contains("head of operations")
        || lower.contains("head of logistics")
    {
        return 65.0;
    }
    if lower.contains("category manager")
        || lower.contains("commodity manager")
        || lower.contains("strategic buyer")
        || lower.contains("senior buyer")
        || lower.contains("quality manager")
        || lower.contains("plant manager")
        || lower.contains("engineering manager")
        || lower.contains("supply chain manager")
    {
        return 55.0;
    }
    if lower.contains("manager") {
        return 50.0;
    }
    if lower.contains("buyer")
        || lower.contains("purchaser")
        || lower.contains("procurement specialist")
        || lower.contains("sourcing specialist")
        || lower.contains("quality engineer")
        || lower.contains("process engineer")
        || lower.contains("supplier quality")
    {
        return 45.0;
    }
    if lower.contains("lead") {
        return 40.0;
    }
    if lower.contains("senior") {
        return 30.0;
    }
    if lower.contains("engineer")
        || lower.contains("analyst")
        || lower.contains("specialist")
        || lower.contains("coordinator")
        || lower.contains("planner")
    {
        return 25.0;
    }
    20.0
}

/// Compute pain index from artifacts — higher if recent disruption/complaint mentions.
pub fn compute_pain_index(artifacts: &[PoiArtifact], now_utc: i64) -> f64 {
    let pain_keywords = [
        "problem",
        "issue",
        "delay",
        "shortage",
        "failure",
        "complaint",
        "disruption",
        "late",
        "defect",
        "recall",
        "crisis",
        "مشكلة",
        "problème",
    ];

    let mut pain_score = 0.0;
    for artifact in artifacts {
        let text = format!("{} {}", artifact.title, artifact.content_summary).to_lowercase();
        if artifact.ts_utc > now_utc {
            continue; // skip future-dated artifacts — they get no recency weight
        }
        let age_days = ((now_utc - artifact.ts_utc) as f64 / 86400.0).max(1.0);
        let recency_weight = 1.0 / (1.0 + age_days / 90.0);

        for kw in &pain_keywords {
            if text.contains(kw) {
                pain_score += recency_weight;
            }
        }
    }

    // Normalize to 0-1 range
    (pain_score / 5.0).min(1.0)
}

/// Compute change risk: how likely this person is to change roles or orgs soon.
/// Based on tenure patterns, recent changes, and career velocity.
pub fn compute_change_risk(role_history: &[RoleHistoryEntry], now_utc: i64) -> f64 {
    if role_history.is_empty() {
        return 0.1; // low confidence default
    }

    // Average tenure in years
    let tenures: Vec<f64> = role_history
        .iter()
        .map(|r| {
            let end = r.end_ts.unwrap_or(now_utc);
            ((end - r.start_ts) as f64 / (365.25 * 86_400.0)).max(0.0)
        })
        .collect();
    let avg_tenure = tenures.iter().sum::<f64>() / tenures.len() as f64;

    // Current tenure
    let current_tenure = role_history
        .last()
        .map(|r| ((now_utc - r.start_ts) as f64 / (365.25 * 86_400.0)).max(0.0))
        .unwrap_or(0.0);

    // Risk increases when current tenure exceeds average (overdue for change)
    let tenure_ratio = if avg_tenure > 0.5 {
        (current_tenure / avg_tenure).min(2.0) / 2.0
    } else {
        0.3
    };

    // More moves = higher base risk
    let move_count = role_history.len() as f64;
    let velocity_risk = (move_count / 6.0).min(1.0);

    let raw = 0.6 * tenure_ratio + 0.4 * velocity_risk;
    raw.clamp(0.0, 1.0)
}

/// Compute role drift score: how far the person's current role diverges from
/// their career trajectory (e.g., a procurement person moving to operations).
pub fn compute_role_drift_score(role_history: &[RoleHistoryEntry]) -> f64 {
    if role_history.len() < 2 {
        return 0.0;
    }

    let families: Vec<String> = role_history
        .iter()
        .map(|r| r.role_family.canonical_label().to_string())
        .collect();

    // Count family changes
    let family_changes = families.windows(2).filter(|w| w[0] != w[1]).count();
    let drift_ratio = family_changes as f64 / (families.len() - 1) as f64;

    drift_ratio.clamp(0.0, 1.0)
}

/// Infer change appetite from role history.
/// Accounts for overlapping roles — concurrent roles count as one move (B127).
pub fn infer_change_appetite(role_history: &[RoleHistoryEntry]) -> ChangeAppetite {
    // Count distinct non-overlapping role transitions
    let distinct_moves = if role_history.len() < 2 {
        role_history.len()
    } else {
        let mut moves = 1usize;
        for i in 1..role_history.len() {
            let prev = &role_history[i - 1];
            let curr = &role_history[i];
            // If current starts after prev ends, it's a new distinct move
            let prev_end = prev.end_ts.unwrap_or(i64::MAX);
            if curr.start_ts >= prev_end || curr.org != prev.org {
                moves += 1;
            }
        }
        moves
    };

    if distinct_moves >= 4 {
        ChangeAppetite::EarlyAdopter
    } else if distinct_moves >= 2 {
        ChangeAppetite::Pragmatist
    } else if distinct_moves == 1 {
        ChangeAppetite::Conservative
    } else {
        ChangeAppetite::Laggard
    }
}

// ─── Seniority mapping helper ──────────────────────────────────────────────

fn seniority_level_for_title(title: &str) -> u8 {
    let t = title.to_lowercase();
    if t.contains("chief")
        || t.contains("ceo")
        || t.contains("coo")
        || t.contains("cfo")
        || t.contains("cto")
        || t.contains("ciso")
        || t.contains("president")
        || t.contains("founder")
    {
        9
    } else if t.contains("evp") || t.contains("svp") || t.contains("executive vice") {
        8
    } else if t.contains("vp")
        || t.contains("vice president")
        || t.contains("principal")
        || t.contains("fellow")
        || t.contains("partner")
    {
        7
    } else if t.contains("director")
        || t.contains("managing director")
        || t.contains("head of")
        || t.contains("gm")
        || t.contains("general manager")
    {
        6
    } else if t.contains("senior manager") || t.contains("senior director") {
        5
    } else if t.contains("manager") || t.contains("lead") {
        4
    } else if t.contains("senior") || t.contains("principal engineer") {
        3
    } else if t.contains("engineer") || t.contains("analyst") || t.contains("specialist") {
        2
    } else {
        1
    }
}

/// Compute career velocity: average seniority-level gain per year of career history.
/// Returns a value in [0, 4] where ≥1.0 is fast-track, ≥0.5 is steady growth.
pub fn compute_career_velocity(role_history: &[RoleHistoryEntry], now_utc: i64) -> f64 {
    if role_history.len() < 2 {
        return 0.0;
    }

    // Use earliest start as career start
    let career_start = role_history
        .iter()
        .map(|r| r.start_ts)
        .min()
        .unwrap_or(now_utc);
    let career_years = ((now_utc - career_start) as f64 / (365.25 * 86_400.0)).max(0.25);

    // Calculate max seniority reached minus lowest seniority held
    let seniorities: Vec<u8> = role_history
        .iter()
        .map(|r| seniority_level_for_title(&r.title))
        .collect();
    let min_s = *seniorities.iter().min().unwrap_or(&1) as f64;
    let max_s = *seniorities.iter().max().unwrap_or(&1) as f64;
    let seniority_gain = (max_s - min_s).max(0.0);

    (seniority_gain / career_years).min(4.0)
}

/// Count distinct artifact mentions in the trailing `window_days`, normalised to [0, 1].
/// 100 or more mentions in the window scores 1.0.
pub fn compute_public_recurrence_from_artifacts(
    artifacts: &[PoiArtifact],
    now_utc: i64,
    window_days: i64,
) -> f64 {
    let cutoff = now_utc - window_days * 86_400;
    let count = artifacts
        .iter()
        .filter(|a| a.ts_utc >= cutoff && a.ts_utc <= now_utc + 86_400)
        .count();
    (count as f64 / 100.0).min(1.0)
}

/// Return the top pain themes as `(topic_label, weight)` pairs, sorted descending by weight.
/// At most 6 themes are returned; weights are normalised so the best category = 1.0.
pub fn infer_pain_themes(artifacts: &[PoiArtifact], now_utc: i64) -> Vec<(String, f64)> {
    // theme keyword sets → same decay logic as compute_pain_index but per category
    const THEMES: &[(&str, &[&str])] = &[
        (
            "supply_disruption",
            &[
                "shortage",
                "disruption",
                "stockout",
                "out of stock",
                "delay",
                "backlog",
            ],
        ),
        (
            "financial_stress",
            &[
                "budget cut",
                "layoff",
                "downturn",
                "loss",
                "deficit",
                "cash flow",
            ],
        ),
        (
            "quality_crisis",
            &[
                "defect",
                "recall",
                "failure",
                "reject",
                "ppm spike",
                "complaint",
            ],
        ),
        (
            "talent_gap",
            &[
                "talent",
                "hiring",
                "attrition",
                "skills gap",
                "understaffed",
                "headcount",
            ],
        ),
        (
            "regulatory_burden",
            &[
                "fine",
                "penalty",
                "audit finding",
                "non-compliance",
                "litigation",
                "sec",
                "gdpr",
            ],
        ),
        (
            "cyber_threat",
            &[
                "breach",
                "ransomware",
                "attack",
                "vulnerability",
                "data leak",
                "phishing",
            ],
        ),
    ];

    let half_life_secs = 90.0 * 86_400.0_f64; // 90-day half-life
    let mut scores: Vec<f64> = vec![0.0; THEMES.len()];

    for artifact in artifacts {
        let age = (now_utc - artifact.ts_utc).max(0) as f64;
        if age > 365.0 * 86_400.0 {
            continue;
        }
        let decay = (-age / half_life_secs).exp();
        let text = format!(
            "{} {}",
            artifact.title.to_lowercase(),
            artifact.content_summary.to_lowercase()
        );
        for (i, (_label, kws)) in THEMES.iter().enumerate() {
            for kw in *kws {
                if text.contains(kw) {
                    scores[i] += decay;
                    break; // only one hit per artifact per theme
                }
            }
        }
    }

    let max_score = scores.iter().cloned().fold(0.0_f64, f64::max).max(1e-9);
    let mut themes: Vec<(String, f64)> = THEMES
        .iter()
        .zip(scores.iter())
        .filter(|(_, &s)| s > 0.0)
        .map(|((label, _), &s)| (label.to_string(), (s / max_score).min(1.0)))
        .collect();
    themes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    themes.truncate(6);
    themes
}

/// Count distinct priority-vector categories that carry significant weight (≥10%).
pub fn compute_topic_diversity(pv: &PriorityVector) -> u8 {
    let weights = [
        pv.cost,
        pv.quality,
        pv.speed,
        pv.resilience,
        pv.compliance,
        pv.security,
    ];
    weights.iter().filter(|&&w| w >= 0.10).count() as u8
}

// Assertive/directive language markers
const DIRECTIVE_MARKERS: &[&str] = &[
    "we will",
    "we must",
    "i will",
    "i expect",
    "we are committed",
    "non-negotiable",
    "mandatory",
    "zero tolerance",
    "immediate",
    "take action",
    "drive",
    "execute",
    "deliver",
    "require",
];
// Hedging/passive language markers
const HEDGING_MARKERS: &[&str] = &[
    "perhaps",
    "maybe",
    "consider",
    "might",
    "could be worth",
    "it depends",
    "we'll see",
    "hopefully",
    "try to",
    "look into",
    "it's complicated",
    "nuanced",
    "in theory",
    "ideally",
];

/// Infer communication assertiveness from quote / speech artifacts.
/// Returns a score in [0, 1]: 1.0 = very assertive/directive, 0.0 = very passive/hedging.
pub fn infer_communication_assertiveness(artifacts: &[PoiArtifact]) -> f64 {
    let mut directive_hits = 0usize;
    let mut hedging_hits = 0usize;
    let mut quote_count = 0usize;

    for artifact in artifacts {
        if artifact.artifact_type != "quote" && artifact.artifact_type != "speech" {
            continue;
        }
        quote_count += 1;
        let text = format!(
            "{} {}",
            artifact.title.to_lowercase(),
            artifact.content_summary.to_lowercase()
        );
        for m in DIRECTIVE_MARKERS {
            if text.contains(m) {
                directive_hits += 1;
            }
        }
        for m in HEDGING_MARKERS {
            if text.contains(m) {
                hedging_hits += 1;
            }
        }
    }

    if quote_count == 0 {
        return 0.5; // neutral default when no quotes
    }

    let total = (directive_hits + hedging_hits) as f64;
    if total < 1.0 {
        return 0.5;
    }
    (directive_hits as f64 / total).clamp(0.0, 1.0)
}

/// Detect the number of distinct organisations the person appears to hold
/// board-level roles in, based on artifact text (a proxy for cross-board influence).
pub fn infer_cross_board_count(artifacts: &[PoiArtifact]) -> u32 {
    const BOARD_SIGNALS: &[&str] = &[
        "board member",
        "board of directors",
        "director at",
        "advisory board",
        "non-executive",
        "independent director",
        "trustee",
        "governor",
    ];
    use std::collections::HashSet;
    let mut orgs: HashSet<String> = HashSet::new();
    for artifact in artifacts {
        let text = format!(
            "{} {}",
            artifact.title.to_lowercase(),
            artifact.content_summary.to_lowercase()
        );
        let has_board_signal = BOARD_SIGNALS.iter().any(|s| text.contains(s));
        if has_board_signal {
            // Heuristic: find the word after "at" / "of" / "for" near the board keyword
            if let Some(pos) = BOARD_SIGNALS.iter().find_map(|s| text.find(s)) {
                let snippet = &text[pos..pos.min(text.len())];
                // Just record the artifact source as a distinct board seat proxy
                orgs.insert(
                    artifact
                        .source_url
                        .clone()
                        .unwrap_or_else(|| snippet.to_string()),
                );
            }
        }
    }
    orgs.len() as u32
}

/// Classify a person into a buying-center role based on their title and role family.
///
/// Returns one of: "Decider", "Influencer", "Buyer", "Gatekeeper", "User", "Initiator".
pub fn classify_buying_center_role(title: &str, role_family: &str) -> &'static str {
    let lower = title.to_lowercase();
    let role_family_lower = role_family.to_lowercase();

    // C-suite and VPs are Deciders
    if lower.contains("ceo")
        || lower.contains("coo")
        || lower.contains("cfo")
        || lower == "cto"
        || lower.starts_with("cto ")
        || lower.contains(" cto")
        || lower.contains("cpo")
        || lower.contains("chief")
        || lower.contains("president")
        || lower.contains("general manager")
        || lower.contains("managing director")
    {
        return "Decider";
    }

    // Procurement / Purchasing are Buyers
    if matches!(
        role_family_lower.as_str(),
        "supply chain" | "supply_chain" | "procurement" | "sourcing" | "purchasing"
    ) || lower.contains("buyer")
        || lower.contains("purchas")
        || lower.contains("procurement")
        || lower.contains("sourcing")
        || lower.contains("supply chain")
        || lower.contains("category manager")
        || lower.contains("commodity")
        || lower.contains("vendor management")
        || lower.contains("approvisionnement")
        || lower.contains("achat")
    {
        return "Buyer";
    }

    // Quality / Compliance / Legal / Regulatory are Gatekeepers
    if matches!(
        role_family_lower.as_str(),
        "quality" | "regulatory" | "legal" | "compliance"
    ) || lower.contains("compliance")
        || lower.contains("quality")
        || lower.contains("audit")
        || lower.contains("inspector")
        || lower.contains("certification")
    {
        return "Gatekeeper";
    }

    // Engineering / R&D / Operations are Users (they use the product/service)
    if role_family == "Engineering"
        || role_family == "Operations"
        || lower.contains("engineer")
        || lower.contains("manufactur")
        || lower.contains("production")
        || lower.contains("plant manager")
    {
        return "User";
    }

    // VP / Director level are Influencers
    if lower.contains("vp")
        || lower.contains("vice president")
        || lower.contains("director")
        || lower.contains("head of")
        || lower.contains("senior manager")
    {
        return "Influencer";
    }

    // Strategy / Innovation roles are Initiators
    if role_family == "Strategy"
        || role_family == "Research"
        || lower.contains("strategy")
        || lower.contains("innovation")
        || lower.contains("transformation")
        || lower.contains("business development")
    {
        return "Initiator";
    }

    "Influencer"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_artifact(title: &str, summary: &str, ts: i64) -> PoiArtifact {
        PoiArtifact {
            artifact_type: "article".to_string(),
            title: title.to_string(),
            content_summary: summary.to_string(),
            source_url: None,
            ts_utc: ts,
        }
    }

    #[test]
    fn test_compute_priority_vector_cost_dominant() {
        let artifacts = vec![
            make_artifact(
                "Cost reduction strategies",
                "Budget savings and TCO analysis",
                1700000000,
            ),
            make_artifact(
                "Price negotiation",
                "Cost optimization approach with price benchmarking",
                1700000000,
            ),
        ];
        let pv = compute_priority_vector(&artifacts);
        assert_eq!(pv.dominant(), "cost");
        assert!(pv.cost > 0.3);
    }

    #[test]
    fn test_compute_priority_vector_quality_dominant() {
        let artifacts = vec![
            make_artifact(
                "Quality management",
                "PPM defect analysis and yield improvement",
                1700000000,
            ),
            make_artifact("Zero defects", "Quality control excellence", 1700000000),
        ];
        let pv = compute_priority_vector(&artifacts);
        assert_eq!(pv.dominant(), "quality");
    }

    #[test]
    fn test_compute_priority_vector_empty() {
        let pv = compute_priority_vector(&[]);
        assert!((pv.confidence - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_infer_decision_style() {
        let pv = PriorityVector {
            cost: 0.5,
            quality: 0.2,
            speed: 0.1,
            resilience: 0.1,
            compliance: 0.05,
            security: 0.05,
            confidence: 0.8,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::CostFirst);
    }

    #[test]
    fn test_infer_decision_style_balanced() {
        let pv = PriorityVector {
            cost: 0.18,
            quality: 0.17,
            speed: 0.16,
            resilience: 0.17,
            compliance: 0.16,
            security: 0.16,
            confidence: 0.5,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::BalancedAnalytical);
    }

    #[test]
    fn test_infer_decision_style_all_zero_values() {
        let pv = PriorityVector::zero();
        assert_eq!(infer_decision_style(&pv), DecisionStyle::BalancedAnalytical);
    }

    #[test]
    fn test_infer_decision_style_nan_values_defaults_balanced() {
        let pv = PriorityVector {
            cost: f64::NAN,
            quality: f64::NAN,
            speed: f64::NAN,
            resilience: f64::NAN,
            compliance: f64::NAN,
            security: f64::NAN,
            confidence: 0.0,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::BalancedAnalytical);
    }

    #[test]
    fn test_infer_decision_style_tie_prefers_stable_primary_dimension() {
        let pv = PriorityVector {
            cost: 0.40,
            quality: 0.40,
            speed: 0.05,
            resilience: 0.05,
            compliance: 0.05,
            security: 0.05,
            confidence: 0.9,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::CostFirst);
    }

    #[test]
    fn test_infer_decision_style_tie_risk_dimensions() {
        let pv = PriorityVector {
            cost: 0.05,
            quality: 0.05,
            speed: 0.05,
            resilience: 0.40,
            compliance: 0.05,
            security: 0.40,
            confidence: 0.9,
        };
        assert_eq!(infer_decision_style(&pv), DecisionStyle::RiskFirst);
    }

    #[test]
    fn test_compute_influence_score() {
        let score = compute_influence_score(80.0, 90.0, 70.0);
        // 0.35*80 + 0.25*90 + 0.40*70 = 28 + 22.5 + 28 = 78.5
        assert!((score - 78.5).abs() < 0.01);
    }

    #[test]
    fn test_compute_influence_score_negative_inputs() {
        let score = compute_influence_score(-10.0, -20.0, -30.0);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_role_seniority_score() {
        assert!((role_seniority_score("CEO") - 95.0).abs() < 0.01);
        assert!((role_seniority_score("VP Procurement") - 85.0).abs() < 0.01);
        assert!((role_seniority_score("Director of Engineering") - 70.0).abs() < 0.01);
        assert!((role_seniority_score("Manager") - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_pain_index_high() {
        let now = 1700100000_i64;
        let artifacts = vec![
            make_artifact(
                "Supply chain crisis",
                "Major shortage and delays causing failure",
                now - 86400,
            ),
            make_artifact(
                "Quality problem report",
                "Defect recall and complaint escalation",
                now - 43200,
            ),
        ];
        let pain = compute_pain_index(&artifacts, now);
        assert!(
            pain > 0.5,
            "Recent pain artifacts should yield high pain index, got {}",
            pain
        );
    }

    #[test]
    fn test_compute_pain_index_low() {
        let now = 1700100000_i64;
        let artifacts = vec![make_artifact(
            "Annual report",
            "Growth and expansion plans",
            now - 86400,
        )];
        let pain = compute_pain_index(&artifacts, now);
        assert!(
            pain < 0.2,
            "expected low pain from non-problematic artifact, got {}",
            pain
        );
    }

    #[test]
    fn test_compute_pain_index_empty_is_zero() {
        let now = 1700100000_i64;
        let pain = compute_pain_index(&[], now);
        assert_eq!(pain, 0.0);
    }

    #[test]
    fn test_classify_buying_center_role() {
        assert_eq!(classify_buying_center_role("CEO", "Executive"), "Decider");
        assert_eq!(classify_buying_center_role("CFO", "Finance"), "Decider");
        assert_eq!(
            classify_buying_center_role("Chief Procurement Officer", "Supply Chain"),
            "Decider"
        );
        assert_eq!(
            classify_buying_center_role("Senior Buyer", "Supply Chain"),
            "Buyer"
        );
        assert_eq!(
            classify_buying_center_role("Category Manager, Packaging", "Supply Chain"),
            "Buyer"
        );
        assert_eq!(
            classify_buying_center_role("Head of Supply Chain", "procurement"),
            "Buyer"
        );
        assert_eq!(
            classify_buying_center_role("Quality Manager", "Quality"),
            "Gatekeeper"
        );
        assert_eq!(
            classify_buying_center_role("Quality Manager", "quality"),
            "Gatekeeper"
        );
        assert_eq!(
            classify_buying_center_role("Compliance Officer", "Regulatory"),
            "Gatekeeper"
        );
        assert_eq!(
            classify_buying_center_role("Plant Engineer", "Engineering"),
            "User"
        );
        assert_eq!(
            classify_buying_center_role("Production Manager", "operations"),
            "User"
        );
        assert_eq!(
            classify_buying_center_role("Production Manager", "Operations"),
            "User"
        );
        assert_eq!(
            classify_buying_center_role("VP of Sales", "Sales"),
            "Influencer"
        );
        assert_eq!(
            classify_buying_center_role("Director of Marketing", "Marketing"),
            "Influencer"
        );
        assert_eq!(
            classify_buying_center_role("Strategy Lead", "Strategy"),
            "Initiator"
        );
        assert_eq!(
            classify_buying_center_role("Innovation Director", "Strategy"),
            "Influencer"
        );
    }

    #[test]
    fn test_infer_change_appetite() {
        let history_4 = vec![
            RoleHistoryEntry {
                org: "A".to_string(),
                title: "Eng".to_string(),
                role_family: RoleFamily::Engineering,
                start_ts: 0,
                end_ts: Some(100),
            },
            RoleHistoryEntry {
                org: "B".to_string(),
                title: "Eng".to_string(),
                role_family: RoleFamily::Engineering,
                start_ts: 100,
                end_ts: Some(200),
            },
            RoleHistoryEntry {
                org: "C".to_string(),
                title: "Eng".to_string(),
                role_family: RoleFamily::Engineering,
                start_ts: 200,
                end_ts: Some(300),
            },
            RoleHistoryEntry {
                org: "D".to_string(),
                title: "Eng".to_string(),
                role_family: RoleFamily::Engineering,
                start_ts: 300,
                end_ts: None,
            },
        ];
        assert_eq!(
            infer_change_appetite(&history_4),
            ChangeAppetite::EarlyAdopter
        );
        assert_eq!(
            infer_change_appetite(&history_4[..2]),
            ChangeAppetite::Pragmatist
        );
        assert_eq!(
            infer_change_appetite(&history_4[..1]),
            ChangeAppetite::Conservative
        );
        assert_eq!(infer_change_appetite(&[]), ChangeAppetite::Laggard);
    }

    // B112: Guard against negative influence_score
    #[test]
    fn test_influence_score_negative_inputs() {
        let score = compute_influence_score(-10.0, -5.0, -20.0);
        assert!(
            score >= 0.0,
            "Negative inputs should be clamped: got {}",
            score
        );
    }

    // B119: Future-dated artifacts beyond threshold
    #[test]
    fn test_pain_index_future_artifacts_ignored() {
        let now = 1700100000_i64;
        let artifacts = vec![make_artifact(
            "Future crisis",
            "Major problem and failure",
            now + 86400 * 365,
        )];
        let pain = compute_pain_index(&artifacts, now);
        assert!(
            pain < 0.01,
            "Future artifacts should not contribute to pain, got {}",
            pain
        );
    }

    // B120: Empty role family / unknown values
    #[test]
    fn test_role_seniority_unknown_title() {
        let score = role_seniority_score("");
        assert!(score >= 0.0);
        assert!(score <= 100.0);
    }

    // B124: Pain index uses bounded recency weights
    #[test]
    fn test_pain_index_bounded() {
        let now = 1700100000_i64;
        // Many pain artifacts should still be bounded to [0,1]
        let artifacts: Vec<PoiArtifact> = (0..100)
            .map(|i| {
                make_artifact(
                    "crisis failure problem",
                    "shortage delay defect",
                    now - i * 3600,
                )
            })
            .collect();
        let pain = compute_pain_index(&artifacts, now);
        assert!(
            pain <= 1.0,
            "Pain index should be bounded to 1.0, got {}",
            pain
        );
        assert!(pain >= 0.0);
    }

    // B128: Network size validation (handled in model, tested here for completeness)
    #[test]
    fn test_influence_score_clamps_to_100() {
        let score = compute_influence_score(200.0, 200.0, 200.0);
        assert!((score - 100.0).abs() < 0.01);
    }

    // B128: Network size upper bound clamp
    #[test]
    fn test_network_size_clamp() {
        let mut inf = InfluenceProfile {
            influence_score: 50.0,
            graph_centrality: 50.0,
            public_recurrence: 50.0,
            role_seniority_score: 50.0,
            network_size: 100_000,
        };
        inf.clamp_network_size();
        assert_eq!(inf.network_size, MAX_NETWORK_SIZE);
    }

    // B130: Locale-specific keyword sets for priority vector
    #[test]
    fn test_priority_vector_with_locale_keywords() {
        let artifacts = vec![make_artifact(
            "비용 절감 보고서",
            "예산 초과 및 가격 인상",
            1700000000,
        )];
        let locale_kws = default_locale_keywords();
        let pv = compute_priority_vector_with_locale(&artifacts, &locale_kws);
        // Korean cost keywords should boost cost dimension
        assert!(
            pv.cost > 0.0,
            "Korean cost keywords should register, got cost={}",
            pv.cost
        );
    }

    #[test]
    fn test_compute_priority_vector_repeated_keywords_bounded() {
        let repeated_cost = "cost ".repeat(200);
        let artifacts = vec![make_artifact(
            "Cost storm",
            &format!("{} quality", repeated_cost),
            1700000000,
        )];

        let pv = compute_priority_vector(&artifacts);
        assert!(
            pv.cost < 0.9,
            "repeated keyword should be bounded, got {}",
            pv.cost
        );
        assert!(
            pv.cost > pv.quality,
            "cost should still dominate but not saturate"
        );
    }

    #[test]
    fn test_compute_priority_vector_repeated_keywords_cap_is_stable() {
        let artifacts_low = vec![make_artifact(
            "A",
            &"cost ".repeat(MAX_KEYWORD_HITS_PER_TERM),
            1700000000,
        )];
        let artifacts_high = vec![make_artifact(
            "A",
            &"cost ".repeat(MAX_KEYWORD_HITS_PER_TERM * 20),
            1700000000,
        )];

        let pv_low = compute_priority_vector(&artifacts_low);
        let pv_high = compute_priority_vector(&artifacts_high);

        assert!((pv_low.cost - pv_high.cost).abs() < 1e-9);
    }
}
