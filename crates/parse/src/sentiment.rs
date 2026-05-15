//! Multi-language sentiment analyzer with keyword lists for business signals.
//!
//! Covers: English, French, Arabic, Hebrew, Chinese (Simplified), Japanese,
//! Korean, and German — each with positive/negative business signal words.
//!
//! Extends the basic `simple_sentiment` function in parse/multilingual.rs
//! with proper per-language keyword dictionaries.

use serde::Serialize;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SentimentResult {
    pub text: String,
    pub language: String,
    pub score: f64,     // -1.0 to +1.0
    pub magnitude: f64, // 0.0 to 1.0 (strength)
    pub label: SentimentLabel,
    pub positive_hits: Vec<String>,
    pub negative_hits: Vec<String>,
    pub aspect_scores: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum SentimentLabel {
    VeryPositive,
    Positive,
    Neutral,
    Negative,
    VeryNegative,
}

#[derive(Debug, Clone, Copy)]
struct KeywordDescriptor {
    term: &'static str,
    polarity: f64,
    aspect: &'static str,
    idf_weight: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Keyword dictionaries
// ─────────────────────────────────────────────────────────────────────────────

fn get_positive_keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "growth",
            "expansion",
            "expand",
            "profit",
            "profitable",
            "revenue",
            "innovation",
            "partnership",
            "award",
            "milestone",
            "record",
            "success",
            "investment",
            "acquisition",
            "upgrade",
            "breakthrough",
            "leading",
            "certification",
            "contract",
            "approved",
            "launch",
            "strategic",
        ],
        "fr" => &[
            "croissance",
            "expansion",
            "bénéfice",
            "chiffre d'affaires",
            "innovation",
            "partenariat",
            "récompense",
            "record",
            "succès",
            "investissement",
            "acquisition",
            "modernisation",
            "percée",
            "leader",
            "certification",
            "contrat",
            "approuvé",
            "lancement",
            "stratégique",
            "développement",
        ],
        "ar" => &[
            "نمو",
            "توسع",
            "ربح",
            "إيرادات",
            "ابتكار",
            "شراكة",
            "جائزة",
            "إنجاز",
            "نجاح",
            "استثمار",
            "استحواذ",
            "تطوير",
            "اختراق",
            "ريادة",
            "شهادة",
            "عقد",
            "موافقة",
            "إطلاق",
            "تعاون",
            "تقدم",
        ],
        "he" => &[
            "צמיחה",
            "הרחבה",
            "רווח",
            "רווחי",
            "הכנסות",
            "חדשנות",
            "שותפות",
            "פרס",
            "אבן דרך",
            "הצלחה",
            "השקעה",
            "רכישה",
            "שדרוג",
            "פריצת דרך",
            "מוביל",
            "הסמכה",
            "חוזה",
            "אישור",
            "השקה",
            "אסטרטגי",
            "פיתוח",
        ],
        "zh" => &[
            "增长",
            "扩张",
            "利润",
            "收入",
            "创新",
            "合作",
            "获奖",
            "里程碑",
            "成功",
            "投资",
            "收购",
            "升级",
            "突破",
            "领先",
            "认证",
            "合同",
            "批准",
            "上市",
            "战略",
            "发展",
        ],
        "ja" => &[
            "成長", "拡大", "利益", "収益", "革新", "提携", "受賞", "達成", "成功", "投資", "買収",
            "進化", "躍進", "先進", "認証", "契約", "承認", "発売", "戦略", "発展",
        ],
        "ko" => &[
            "성장",
            "확장",
            "이익",
            "매출",
            "혁신",
            "파트너십",
            "수상",
            "달성",
            "성공",
            "투자",
            "인수",
            "업그레이드",
            "돌파",
            "선도",
            "인증",
            "계약",
            "승인",
            "출시",
            "전략",
            "발전",
        ],
        "de" => &[
            "Wachstum",
            "Expansion",
            "Gewinn",
            "Umsatz",
            "Innovation",
            "Partnerschaft",
            "Auszeichnung",
            "Meilenstein",
            "Erfolg",
            "Investition",
            "Übernahme",
            "Modernisierung",
            "Durchbruch",
            "führend",
            "Zertifizierung",
            "Auftrag",
            "Genehmigung",
            "Markteinführung",
            "strategisch",
            "Entwicklung",
        ],
        _ => &[],
    }
}

fn get_negative_keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "decline",
            "loss",
            "layoff",
            "closure",
            "bankruptcy",
            "recall",
            "lawsuit",
            "penalty",
            "violation",
            "downgrade",
            "delay",
            "shortage",
            "dispute",
            "sanction",
            "fraud",
            "default",
            "investigation",
            "restructuring",
            "warning",
            "risk",
        ],
        "fr" => &[
            "déclin",
            "perte",
            "licenciement",
            "fermeture",
            "faillite",
            "rappel",
            "procès",
            "amende",
            "violation",
            "dégradation",
            "retard",
            "pénurie",
            "litige",
            "sanction",
            "fraude",
            "défaut",
            "enquête",
            "restructuration",
            "avertissement",
            "risque",
        ],
        "ar" => &[
            "تراجع",
            "خسارة",
            "لم تتحقق",
            "تسريح",
            "إغلاق",
            "إفلاس",
            "سحب",
            "دعوى",
            "غرامة",
            "انتهاك",
            "تخفيض",
            "تأخير",
            "نقص",
            "نزاع",
            "عقوبة",
            "احتيال",
            "تخلف",
            "تحقيق",
            "إعادة هيكلة",
            "تحذير",
            "مخاطر",
        ],
        "he" => &[
            "ירידה",
            "הפסד",
            "לא רווחי",
            "פיטורים",
            "סגירה",
            "פשיטת רגל",
            "ריקול",
            "תביעה",
            "קנס",
            "הפרה",
            "הורדת דירוג",
            "עיכוב",
            "מחסור",
            "סכסוך",
            "סנקציה",
            "הונאה",
            "חדלות פירעון",
            "חקירה",
            "ארגון מחדש",
            "אזהרה",
            "סיכון",
        ],
        "zh" => &[
            "下降", "亏损", "裁员", "关闭", "破产", "召回", "诉讼", "罚款", "违规", "降级", "延迟",
            "短缺", "纠纷", "制裁", "欺诈", "违约", "调查", "重组", "警告", "风险",
        ],
        "ja" => &[
            "減少",
            "損失",
            "解雇",
            "閉鎖",
            "倒産",
            "リコール",
            "訴訟",
            "罰金",
            "違反",
            "格下げ",
            "遅延",
            "不足",
            "紛争",
            "制裁",
            "不正",
            "債務不履行",
            "調査",
            "再編",
            "警告",
            "リスク",
        ],
        "ko" => &[
            "감소",
            "손실",
            "해고",
            "폐쇄",
            "파산",
            "리콜",
            "소송",
            "벌금",
            "위반",
            "하향",
            "지연",
            "부족",
            "분쟁",
            "제재",
            "사기",
            "부도",
            "조사",
            "구조조정",
            "경고",
            "위험",
        ],
        "de" => &[
            "Rückgang",
            "Verlust",
            "Entlassung",
            "Schließung",
            "Insolvenz",
            "Rückruf",
            "Klage",
            "Strafe",
            "Verstoß",
            "Herabstufung",
            "Verzögerung",
            "Engpass",
            "Streit",
            "Sanktion",
            "Betrug",
            "Ausfall",
            "Ermittlung",
            "Umstrukturierung",
            "Warnung",
            "Risiko",
        ],
        _ => &[],
    }
}

fn domain_positive_keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "awarded contract",
            "expand capacity",
            "expanded capacity",
            "passed audit",
            "tier 1 qualified",
            "tier-1 qualified",
            "production ramp",
        ],
        _ => &[],
    }
}

fn domain_negative_keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "en" => &[
            "force majeure",
            "supply disruption",
            "debarred",
            "dpas rated shortage",
            "dpas-rated shortage",
            "lot rejection",
        ],
        _ => &[],
    }
}

fn negation_terms() -> &'static [&'static str] {
    &[
        "not", "no", "never", "n't", "without", "lack", "lack of", "sans", "без", "لا", "لم", "לא",
    ]
}

fn hedge_terms() -> &'static [&'static str] {
    &[
        "might",
        "could",
        "possibly",
        "uncertain",
        "allegedly",
        "reportedly",
    ]
}

fn keyword_aspect(term: &str) -> &'static str {
    match term {
        "profit" | "revenue" | "growth" | "loss" | "bankruptcy" | "default" | "investment" => {
            "financial"
        }
        "expansion" | "delay" | "shortage" | "expanded capacity" | "production ramp"
        | "supply disruption" | "lot rejection" | "passed audit" => "operational",
        "penalty"
        | "violation"
        | "sanction"
        | "approved"
        | "certification"
        | "debarred"
        | "investigation"
        | "dpas rated shortage"
        | "dpas-rated shortage" => "regulatory",
        _ => "reputational",
    }
}

fn keyword_idf_weight(term: &str) -> f64 {
    match term {
        "growth" | "success" | "risk" | "warning" | "contract" => 0.65,
        "award" | "partnership" | "delay" | "shortage" => 0.8,
        "awarded contract"
        | "passed audit"
        | "production ramp"
        | "force majeure"
        | "supply disruption"
        | "debarred"
        | "dpas rated shortage"
        | "lot rejection" => 1.25,
        _ => 1.0,
    }
}

fn keyword_catalog(language: &str) -> Vec<KeywordDescriptor> {
    let positives = get_positive_keywords(language)
        .iter()
        .map(|term| KeywordDescriptor {
            term,
            polarity: 1.0,
            aspect: keyword_aspect(term),
            idf_weight: keyword_idf_weight(term),
        });
    let negatives = get_negative_keywords(language)
        .iter()
        .map(|term| KeywordDescriptor {
            term,
            polarity: -1.0,
            aspect: keyword_aspect(term),
            idf_weight: keyword_idf_weight(term),
        });
    let domain_positives =
        domain_positive_keywords(language)
            .iter()
            .map(|term| KeywordDescriptor {
                term,
                polarity: 1.0,
                aspect: keyword_aspect(term),
                idf_weight: keyword_idf_weight(term),
            });
    let domain_negatives =
        domain_negative_keywords(language)
            .iter()
            .map(|term| KeywordDescriptor {
                term,
                polarity: -1.0,
                aspect: keyword_aspect(term),
                idf_weight: keyword_idf_weight(term),
            });

    positives
        .chain(negatives)
        .chain(domain_positives)
        .chain(domain_negatives)
        .collect()
}

fn tokenize_lower(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '\''))
        .filter(|token| !token.is_empty())
        .map(|token| token.to_string())
        .collect()
}

fn phrase_tokens(term: &str) -> Vec<String> {
    term.to_lowercase()
        .split_whitespace()
        .map(|token| token.to_string())
        .collect()
}

fn phrase_positions(tokens: &[String], phrase: &[String]) -> Vec<usize> {
    if phrase.is_empty() || tokens.len() < phrase.len() {
        return Vec::new();
    }

    tokens
        .windows(phrase.len())
        .enumerate()
        .filter_map(|(index, window)| if window == phrase { Some(index) } else { None })
        .collect()
}

fn has_window_term(tokens: &[String], keyword_start: usize, window_terms: &[&str]) -> bool {
    let window_start = keyword_start.saturating_sub(3);
    let context = tokens[window_start..keyword_start].join(" ");
    window_terms.iter().any(|term| {
        let normalized = term.to_lowercase();
        if normalized.contains(' ') {
            context.contains(&normalized)
        } else {
            tokens[window_start..keyword_start]
                .iter()
                .any(|token| token == &normalized)
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Sentiment analysis engine
// ─────────────────────────────────────────────────────────────────────────────

/// Detect the language of a text (simple heuristic based on character ranges).
pub fn detect_language(text: &str) -> &'static str {
    if text
        .chars()
        .any(|ch| matches!(ch, '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}'))
    {
        return "ja";
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();

    for ch in text.chars() {
        let lang = match ch {
            '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' => "ar",
            '\u{0590}'..='\u{05FF}' => "he",
            '\u{3040}'..='\u{309F}' | '\u{30A0}'..='\u{30FF}' => "ja",
            '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}' => "zh",
            '\u{AC00}'..='\u{D7AF}' | '\u{1100}'..='\u{11FF}' => "ko",
            // German-specific chars (ß, ä, ö, ü) — check before broader Latin range
            '\u{00DF}' | '\u{00E4}' | '\u{00F6}' | '\u{00FC}' => "de",
            '\u{00C0}'..='\u{00DE}'
            | '\u{00E0}'..='\u{00E3}'
            | '\u{00E5}'..='\u{00F5}'
            | '\u{00F7}'..='\u{00FB}'
            | '\u{00FD}'..='\u{00FF}' => "fr", // accented Latin (excluding German chars)
            _ => continue,
        };
        *counts.entry(lang).or_default() += 1;
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(lang, _)| lang)
        .unwrap_or("en")
}

/// Analyze sentiment of text in a given language.
pub fn analyze_sentiment(text: &str, language: &str) -> SentimentResult {
    let text_lower = text.to_lowercase();
    let tokens = tokenize_lower(text);
    let token_count = tokens.len().max(1) as f64;
    let catalog = keyword_catalog(language);

    let mut pos_hits = Vec::new();
    let mut neg_hits = Vec::new();
    let mut aspect_scores: HashMap<String, f64> = HashMap::new();
    let mut weighted_score = 0.0;
    let mut weighted_magnitude = 0.0;

    for descriptor in catalog {
        let phrase = phrase_tokens(descriptor.term);
        let mut positions = if !tokens.is_empty() {
            phrase_positions(&tokens, &phrase)
        } else {
            Vec::new()
        };
        if positions.is_empty() && text_lower.contains(&descriptor.term.to_lowercase()) {
            positions.push(0);
        }

        if positions.is_empty() {
            continue;
        }

        let tf = positions.len() as f64 / token_count;
        let tfidf_weight = (1.0 + tf).ln() * descriptor.idf_weight;

        for position in positions {
            let negated = has_window_term(&tokens, position, negation_terms());
            let hedged = has_window_term(&tokens, position, hedge_terms());
            let hedge_scale = if hedged { 0.5 } else { 1.0 };
            let mut signed_weight = descriptor.polarity * tfidf_weight * hedge_scale;
            if negated {
                signed_weight *= -1.0;
            }

            weighted_score += signed_weight;
            weighted_magnitude += tfidf_weight * hedge_scale;
            *aspect_scores
                .entry(descriptor.aspect.to_string())
                .or_default() += signed_weight;

            if signed_weight >= 0.0 {
                if !pos_hits.iter().any(|hit| hit == descriptor.term) {
                    pos_hits.push(descriptor.term.to_string());
                }
            } else if !neg_hits.iter().any(|hit| hit == descriptor.term) {
                neg_hits.push(descriptor.term.to_string());
            }
        }
    }

    let (score, magnitude) = if weighted_magnitude <= f64::EPSILON {
        (0.0, 0.0)
    } else {
        let raw = weighted_score / weighted_magnitude;
        let mag = weighted_magnitude / token_count;
        (raw.clamp(-1.0, 1.0), mag.clamp(0.0, 1.0))
    };

    let label = if score > 0.5 {
        SentimentLabel::VeryPositive
    } else if score > 0.1 {
        SentimentLabel::Positive
    } else if score < -0.5 {
        SentimentLabel::VeryNegative
    } else if score < -0.1 {
        SentimentLabel::Negative
    } else {
        SentimentLabel::Neutral
    };

    SentimentResult {
        text: text.to_string(),
        language: language.into(),
        score,
        magnitude,
        label,
        positive_hits: pos_hits,
        negative_hits: neg_hits,
        aspect_scores,
    }
}

/// Auto-detect language and analyze sentiment.
pub fn analyze_sentiment_auto(text: &str) -> SentimentResult {
    let lang = detect_language(text);
    analyze_sentiment(text, lang)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_positive() {
        let result = analyze_sentiment("Company announces record growth and major expansion", "en");
        assert!(result.score > 0.0);
        assert!(
            result.label == SentimentLabel::Positive
                || result.label == SentimentLabel::VeryPositive
        );
        assert!(!result.positive_hits.is_empty());
    }

    #[test]
    fn english_negative() {
        let result = analyze_sentiment(
            "Company faces bankruptcy after massive layoff and fraud investigation",
            "en",
        );
        assert!(result.score < 0.0);
        assert!(!result.negative_hits.is_empty());
    }

    #[test]
    fn french_positive() {
        let result = analyze_sentiment(
            "L'entreprise annonce une croissance record et un partenariat stratégique",
            "fr",
        );
        assert!(result.score > 0.0);
    }

    #[test]
    fn arabic_detection() {
        let lang = detect_language("الشركة تعلن عن نمو كبير في الإيرادات");
        assert_eq!(lang, "ar");
    }

    #[test]
    fn hebrew_detection() {
        let lang = detect_language("החברה מכריזה על צמיחה משמעותית");
        assert_eq!(lang, "he");
    }

    #[test]
    fn chinese_detection() {
        let lang = detect_language("公司宣布收入增长创下新纪录");
        assert_eq!(lang, "zh");
    }

    #[test]
    fn japanese_detection() {
        let lang = detect_language("会社は記録的な成長を発表しました");
        assert_eq!(lang, "ja");
    }

    #[test]
    fn korean_detection() {
        let lang = detect_language("회사는 기록적인 성장을 발표했습니다");
        assert_eq!(lang, "ko");
    }

    #[test]
    fn neutral_text() {
        let result = analyze_sentiment("The meeting is scheduled for next Tuesday at 3pm", "en");
        assert_eq!(result.label, SentimentLabel::Neutral);
    }

    #[test]
    fn auto_detection_works() {
        let result = analyze_sentiment_auto("L'entreprise annonce une croissance record");
        assert!(result.score > 0.0);
    }

    #[test]
    fn german_keywords() {
        let result = analyze_sentiment(
            "Das Unternehmen meldet Wachstum und eine neue Partnerschaft",
            "de",
        );
        assert!(result.score > 0.0);
    }

    #[test]
    fn negation_flips_polarity() {
        let result = analyze_sentiment("The company did not report growth after the warning", "en");
        assert!(result.score < 0.0);
        assert!(result.negative_hits.iter().any(|hit| hit == "growth"));
    }

    #[test]
    fn hedging_reduces_magnitude() {
        let direct = analyze_sentiment("The company announced an awarded contract", "en");
        let hedged = analyze_sentiment("The company might announce an awarded contract", "en");
        assert!(hedged.magnitude < direct.magnitude);
    }

    #[test]
    fn negation_polarity_inversion() {
        let not_profitable = analyze_sentiment("The company is not profitable", "en");
        let profits_declined = analyze_sentiment("Profits declined after the warning", "en");
        assert!(not_profitable.score <= 0.0);
        assert!(profits_declined.score < 0.0);
    }

    #[test]
    fn hedging_magnitude_reduction() {
        let direct = analyze_sentiment("The company will expand capacity", "en");
        let hedged = analyze_sentiment("The company might expand capacity", "en");
        assert!(hedged.magnitude < direct.magnitude);
    }

    #[test]
    fn multilingual_negation() {
        let arabic = analyze_sentiment("مكاسب لم تتحقق", "ar");
        let hebrew = analyze_sentiment("לא רווחי", "he");
        assert!(arabic.score <= 0.0);
        assert!(hebrew.score < 0.0);
    }

    #[test]
    fn domain_specific_lexicon_applies() {
        let result = analyze_sentiment(
            "The supplier passed audit and began a production ramp after an awarded contract",
            "en",
        );
        assert!(result.score > 0.0);
        assert!(result.positive_hits.iter().any(|hit| hit == "passed audit"));
    }

    #[test]
    fn aspect_scores_separate_dimensions() {
        let result = analyze_sentiment(
            "Revenue growth offset reputational risk after an investigation and passed audit",
            "en",
        );
        assert!(result.aspect_scores.contains_key("financial"));
        assert!(result.aspect_scores.contains_key("regulatory"));
        assert!(result.aspect_scores.contains_key("operational"));
    }
}
