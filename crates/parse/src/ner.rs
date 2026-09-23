//! Multi-language Named Entity Recognition (rule-based).
//!
//! Detects language → routes to appropriate language-specific NER → unifies entity
//! canonical names.  Supports 12 languages (en, fr, ar, zh, ja, ko, de, es, it, pt,
//! tr, fa, he, nl) using purely rule-based extraction with zero external dependencies.
//!
//! # Architecture
//!
//! Each language extractor uses a combination of:
//! - Regex patterns for company suffixes and legal forms
//! - Person name patterns (honorifics + cultural naming conventions)
//! - Geopolitical location dictionaries
//! - Domain-specific keyword patterns (EMS, BESS, supply chain)
//!
//! # Future
//!
//! This module is designed to optionally wrap spaCy-rs bindings behind a feature gate;
//! the [`extract_entities`] dispatch function is the single integration point.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::LazyLock;

use crate::multilingual;
use crate::normalizer;

// ─── Public Types ──────────────────────────────────────────────────────────────

/// An entity extracted from text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedEntity {
    /// The raw text span.
    pub mention: String,
    /// Entity type classification.
    pub entity_type: EntityType,
    /// Language of the source text (ISO 639-1).
    pub language: String,
    /// Confidence score [0.0, 1.0].
    pub confidence: f64,
    /// Normalized/canonical form (see [`crate::entity_canonical`]).
    pub canonical: Option<String>,
    /// Byte offset range in the original text.
    pub span: Option<(usize, usize)>,
}

/// Entity type classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EntityType {
    /// Named person.
    Person,
    /// Named organization/company.
    Organization,
    /// Named location (city, country, region).
    Location,
    /// Geopolitical entity (country with political attributes).
    GeoPolitical,
    /// Named threat actor / malicious group.
    ThreatActor,
    /// EMS / manufacturing facility.
    Facility,
    /// Product or brand name.
    Product,
    /// Financial instrument or monetary value.
    Financial,
    /// Date or time expression.
    DateTime,
    /// Certification or standard.
    Certification,
    /// Job title / professional role.
    JobTitle,
    /// Other / unrecognised.
    Other,
}

/// A job title extraction with role-family classification and confidence scoring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobTitleExtraction {
    /// The extracted title text.
    pub title: String,
    /// Confidence score [0.0, 1.0].
    pub confidence: f64,
    /// Role family classification (e.g. "C-Suite", "VP/Director", "Manager", "Technical").
    pub role_family: String,
    /// Byte offset range in the original text.
    pub span: Option<(usize, usize)>,
}

/// Extract job titles with confidence scoring from text.
/// Matches against known title lists and pattern-based extraction.
pub fn extract_job_title_with_confidence(text: &str) -> Vec<JobTitleExtraction> {
    let mut results = Vec::new();
    let text_lower = text.to_lowercase();

    // Try known-title matching first
    for known in KNOWN_JOB_TITLES_EN.iter() {
        let lower = known.to_lowercase();
        if let Some(pos) = text_lower.find(&lower) {
            let end = pos + known.len();
            results.push(JobTitleExtraction {
                title: text[pos..end].to_string(),
                confidence: 0.9,
                role_family: classify_title_to_role_family(known),
                span: Some((pos, end)),
            });
        }
    }

    // Try Arabic titles
    for known in KNOWN_JOB_TITLES_AR.iter() {
        if let Some(pos) = text.find(known) {
            let end = pos + known.len();
            results.push(JobTitleExtraction {
                title: text[pos..end].to_string(),
                confidence: 0.9,
                role_family: classify_title_to_role_family(known),
                span: Some((pos, end)),
            });
        }
    }

    // Try French titles
    for known in KNOWN_JOB_TITLES_FR.iter() {
        if let Some(pos) = text.find(known) {
            let end = pos + known.len();
            results.push(JobTitleExtraction {
                title: text[pos..end].to_string(),
                confidence: 0.9,
                role_family: classify_title_to_role_family(known),
                span: Some((pos, end)),
            });
        }
    }

    // Try Chinese titles
    for known in KNOWN_JOB_TITLES_ZH.iter() {
        if let Some(pos) = text.find(known) {
            let end = pos + known.len();
            results.push(JobTitleExtraction {
                title: text[pos..end].to_string(),
                confidence: 0.9,
                role_family: classify_title_to_role_family(known),
                span: Some((pos, end)),
            });
        }
    }

    // Pattern-based: "NAME, TITLE at COMPANY" or "NAME, TITLE" with proper-case words
    if results.is_empty() {
        let re_patterns: &[(&str, f64)] = &[
            (
                r"[A-Z][a-z]+(?:\s+[A-Z][a-z]+){0,4}\s+(?:at|with|of|chez)\s+",
                0.6,
            ),
            (
                r"\b(?:VP|SVP|EVP|C[A-Z]O|Head|Lead|Director|Manager|Chief|President)\s+(?:of\s+)?[A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+){0,4}",
                0.7,
            ),
        ];
        for (pat_str, conf) in re_patterns {
            if let Ok(re) = Regex::new(pat_str) {
                for mat in re.find_iter(text) {
                    results.push(JobTitleExtraction {
                        title: mat.as_str().to_string(),
                        confidence: *conf,
                        role_family: classify_title_to_role_family(mat.as_str()),
                        span: Some((mat.start(), mat.end())),
                    });
                }
            }
        }
    }

    // Deduplicate by title
    let mut seen = HashSet::new();
    results.retain(|r| seen.insert(r.title.clone()));

    // Sort by confidence descending
    results.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

/// Classify a job title string into a role family.
fn classify_title_to_role_family(title: &str) -> String {
    let lower = title.to_lowercase();
    if lower.contains("ceo")
        || lower.contains("chief executive")
        || lower.contains("president")
        || lower.contains("chairman")
        || lower.contains("founder")
        || lower.contains("owner")
        || lower.contains("cfo")
        || lower.contains("coo")
        || lower.contains("cto")
        || lower.contains("cio")
        || lower.contains("cmo")
        || lower.contains("cpo")
        || lower.contains("cro")
        || lower.contains("chief")
    {
        "C-Suite".to_string()
    } else if lower.contains("vp")
        || lower.contains("vice president")
        || lower.contains("svp")
        || lower.contains("evp")
        || lower.contains("executive vice")
        || lower.contains("director")
        || lower.contains("head of")
        || lower.contains("global head")
    {
        "VP/Director".to_string()
    } else if lower.contains("manager")
        || lower.contains("supervisor")
        || lower.contains("team lead")
        || lower.contains("lead")
    {
        "Manager".to_string()
    } else if lower.contains("engineer")
        || lower.contains("developer")
        || lower.contains("analyst")
        || lower.contains("specialist")
        || lower.contains("architect")
    {
        "Technical".to_string()
    } else if lower.contains("general")
        || lower.contains("admiral")
        || lower.contains("colonel")
        || lower.contains("major")
        || lower.contains("captain")
        || lower.contains("commander")
        || lower.contains("secretary")
        || lower.contains("minister")
        || lower.contains("ambassador")
        || lower.contains("governor")
    {
        "Defense/Government".to_string()
    } else {
        "Other".to_string()
    }
}

/// Known English job titles (100+ across industries).
static KNOWN_JOB_TITLES_EN: &[&str] = &[
    // C-Suite
    "Chief Executive Officer",
    "Chief Financial Officer",
    "Chief Operating Officer",
    "Chief Technology Officer",
    "Chief Information Officer",
    "Chief Marketing Officer",
    "Chief Strategy Officer",
    "Chief Information Security Officer",
    "Chief Procurement Officer",
    "Chief Human Resources Officer",
    "Chief Data Officer",
    "Chief Analytics Officer",
    "Chief Risk Officer",
    "Chief Compliance Officer",
    "Chief Revenue Officer",
    "Chairman",
    "President",
    "Founder",
    "Co-Founder",
    "Owner",
    "Partner",
    // VP-Level
    "Senior Vice President",
    "Executive Vice President",
    "Vice President of Operations",
    "Vice President of Supply Chain",
    "Vice President of Manufacturing",
    "Vice President of Engineering",
    "Vice President of Sales",
    "Vice President of Procurement",
    "Vice President of Quality",
    "Vice President of Research and Development",
    "Vice President of Business Development",
    "Vice President of Logistics",
    "Vice President of Finance",
    "Vice President of Marketing",
    "Vice President of Strategy",
    "Group Vice President",
    "Regional Vice President",
    // Director-Level
    "Managing Director",
    "Executive Director",
    "Senior Director",
    "Director of Supply Chain",
    "Director of Operations",
    "Director of Manufacturing",
    "Director of Engineering",
    "Director of Procurement",
    "Director of Quality",
    "Director of Logistics",
    "Director of Sales",
    "Director of Marketing",
    "Director of Business Development",
    "Director of Finance",
    "Regional Director",
    "Site Director",
    "Plant Director",
    "Factory Director",
    "Director of Compliance",
    "Director of Security",
    // Manager-Level
    "General Manager",
    "Senior Manager",
    "Supply Chain Manager",
    "Operations Manager",
    "Plant Manager",
    "Quality Manager",
    "Procurement Manager",
    "Engineering Manager",
    "Program Manager",
    "Product Manager",
    "Project Manager",
    "Logistics Manager",
    "Warehouse Manager",
    "Production Manager",
    "Sourcing Manager",
    "Category Manager",
    "Commodity Manager",
    "Supplier Quality Manager",
    "Global Supply Chain Director",
    "Head of Supply Chain",
    "Head of Operations",
    "Head of Manufacturing",
    "Head of Quality",
    "Head of Procurement",
    "Head of Engineering",
    "Head of Sales",
    "Head of Marketing",
    // Defense/Government
    "General",
    "Admiral",
    "Colonel",
    "Lieutenant Colonel",
    "Major",
    "Captain",
    "Commander",
    "Secretary of Defense",
    "Undersecretary",
    "Deputy Secretary",
    "Assistant Secretary",
    "Director of National Intelligence",
    "Program Executive Officer",
    "Brigadier General",
    "Major General",
    // Other
    "Principal",
    "Senior Advisor",
    "Senior Consultant",
    "Lead Engineer",
    "Staff Engineer",
    "Distinguished Engineer",
    "Technical Fellow",
    "Research Fellow",
    "Senior Fellow",
    "Senior Analyst",
    "Principal Engineer",
    "Staff Scientist",
];

/// Known Arabic job titles.
static KNOWN_JOB_TITLES_AR: &[&str] = &[
    "مدير عام",
    "رئيس تنفيذي",
    "مدير العمليات",
    "مدير المالي",
    "مدير التسويق",
    "مدير الموارد البشرية",
    "مدير المشتريات",
    "مدير سلسلة التوريد",
    "مدير الجودة",
    "مدير المصنع",
    "مدير الإنتاج",
    "مدير الهندسة",
    "مدير المبيعات",
    "مدير الخدمات اللوجستية",
    "مدير المشاريع",
    "مدير تقنية المعلومات",
    "رئيس مجلس الإدارة",
    "نائب الرئيس",
    "مدير إدارة",
    "مدير قطاع",
    "رئيس قسم",
    "مهندس",
    "مهندس أول",
    "استشاري",
    "مستشار",
    "مدير تطوير الأعمال",
    "مدير الامتثال",
    "مدير الأمن",
    "مدير المخاطر",
    "العميد",
    "العقيد",
    "المقدم",
    "الرائد",
    "نقيب",
    "لواء",
    "فريق",
    "وكيل وزارة",
    "مساعد وكيل",
    "سفير",
    "محافظ",
];

/// Known French job titles.
static KNOWN_JOB_TITLES_FR: &[&str] = &[
    "Directeur Général",
    "Président Directeur Général",
    "Directeur des Opérations",
    "Directeur Financier",
    "Directeur Marketing",
    "Directeur des Ressources Humaines",
    "Directeur des Achats",
    "Directeur de la Chaîne d'Approvisionnement",
    "Directeur Qualité",
    "Directeur d'Usine",
    "Directeur de Production",
    "Directeur de l'Ingénierie",
    "Directeur Commercial",
    "Directeur Logistique",
    "Directeur de Projet",
    "Directeur Informatique",
    "Directeur Technique",
    "Directeur Recherche et Développement",
    "Directeur de la Stratégie",
    "Président",
    "Vice-Président",
    "Secrétaire Général",
    "Chef de Projet",
    "Chef de Service",
    "Chef d'Équipe",
    "Responsable",
    "Ingénieur",
    "Ingénieur Principal",
    "Consultant",
    "Conseiller",
    "Analyste",
    "Responsable Qualité",
    "Responsable Achats",
    "Responsable Logistique",
    "Responsable Production",
    "Responsable Commercial",
    "Gérant",
    "Directeur Adjoint",
    "Sous-Directeur",
];

/// Known Chinese job titles.
static KNOWN_JOB_TITLES_ZH: &[&str] = &[
    "总经理",
    "首席执行官",
    "首席运营官",
    "首席财务官",
    "首席技术官",
    "首席信息官",
    "首席营销官",
    "副总裁",
    "高级副总裁",
    "执行副总裁",
    "总监",
    "副总监",
    "经理",
    "高级经理",
    "采购经理",
    "供应链总监",
    "质量经理",
    "运营总监",
    "工厂经理",
    "生产经理",
    "工程经理",
    "销售总监",
    "市场总监",
    "人力资源总监",
    "财务总监",
    "技术总监",
    "研发总监",
    "项目经理",
    "物流经理",
    "仓储经理",
    "区域经理",
    "总工程师",
    "主任",
    "副主任",
    "科长",
    "处长",
    "局长",
    "董事长",
    "总裁",
    "创始人",
    "合伙人",
];

// ─── Main Dispatch ─────────────────────────────────────────────────────────────

/// Detect language and extract entities using the appropriate language-specific NER.
///
/// If `lang` is empty or `"auto"`, language is auto-detected from the text.
pub fn extract_entities(text: &str, lang: &str) -> Vec<ExtractedEntity> {
    let detected = if lang.is_empty() || lang == "auto" {
        multilingual::detect_language(text)
    } else {
        lang.to_string()
    };

    let effective_lang = if detected.trim().is_empty() {
        "en"
    } else {
        &detected
    };

    let mut entities = match effective_lang {
        "fr" => extract_french_entities(text),
        "ar" => extract_arabic_entities(text),
        "zh" => extract_chinese_entities(text),
        "ja" => extract_japanese_entities(text),
        "ko" => extract_korean_entities(text),
        "de" => extract_german_entities(text),
        "es" => extract_spanish_entities(text),
        "it" => extract_italian_entities(text),
        "pt" => extract_portuguese_entities(text),
        "tr" => extract_turkish_entities(text),
        "fa" => extract_persian_entities(text),
        "he" => extract_hebrew_entities(text),
        "nl" => extract_dutch_entities(text),
        _ => extract_english_entities(text),
    };

    // Tag language on each entity.
    // If the requested language is not in our supported set (i.e. the _ fallback
    // branch was taken), tag entities with "en" since English extraction was used.
    let tag_lang = match effective_lang {
        "fr" | "ar" | "zh" | "ja" | "ko" | "de" | "es" | "it" | "pt" | "tr" | "fa" | "he"
        | "nl" => effective_lang,
        _ => "en",
    };
    for entity in &mut entities {
        entity.language = tag_lang.to_string();
    }

    // Deduplicate overlapping mentions (keep highest confidence)
    deduplicate_entities(&mut entities);

    entities
}

/// Extract entities for an already-detected language without re-detecting.
pub fn extract_entities_with_lang(text: &str, lang: &str) -> Vec<ExtractedEntity> {
    extract_entities(text, lang)
}

// ─── Deduplication ─────────────────────────────────────────────────────────────

fn deduplicate_entities(entities: &mut Vec<ExtractedEntity>) {
    let mut i = 0;
    while i < entities.len() {
        let mut j = i + 1;
        while j < entities.len() {
            let overlap = spans_overlap(&entities[i], &entities[j]);
            if overlap {
                // Keep higher-confidence entity
                if entities[j].confidence > entities[i].confidence {
                    entities.swap(i, j);
                }
                entities.remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
}

fn spans_overlap(a: &ExtractedEntity, b: &ExtractedEntity) -> bool {
    match (a.span, b.span) {
        (Some((a_start, a_end)), Some((b_start, b_end))) => a_start.max(b_start) < a_end.min(b_end),
        _ => false,
    }
}

// ─── Shared Helpers ────────────────────────────────────────────────────────────

/// Find matches with a compiled regex and convert to entities.
fn regex_matches(
    text: &str,
    re: &Regex,
    entity_type: EntityType,
    confidence: f64,
) -> Vec<ExtractedEntity> {
    re.captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            let mention = normalizer::normalize_whitespace(m.as_str());
            if mention.is_empty() || mention.len() < 2 {
                return None;
            }
            Some(ExtractedEntity {
                mention,
                entity_type: entity_type.clone(),
                language: String::new(),
                confidence,
                canonical: None,
                span: Some((m.start(), m.end())),
            })
        })
        .collect()
}

/// Find matches from a list of known entity names (case-insensitive, whole-word).
///
/// For CJK text (Chinese, Japanese, Korean), word boundaries (`\b`) are not used
/// because CJK characters are all "word" characters in Unicode regex terms and
/// there are no separators between individual characters. Instead, we use direct
/// substring matching with surrounding-character checks for CJK.
fn known_entity_matches(
    text: &str,
    dictionary: &[&str],
    entity_type: EntityType,
    confidence: f64,
) -> Vec<ExtractedEntity> {
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for entry in dictionary {
        let has_cjk = entry.chars().any(|c| {
            let cp = c as u32;
            (0x4E00..=0x9FFF).contains(&cp)
                || (0x3400..=0x4DBF).contains(&cp)
                || (0xAC00..=0xD7AF).contains(&cp)
                || (0x3040..=0x309F).contains(&cp)
                || (0x30A0..=0x30FF).contains(&cp)
        });

        if has_cjk || entry.chars().any(|c| (c as u32) >= 0x2000) {
            // For CJK and wide scripts: direct substring match without word boundaries.
            // The \b assertion doesn't work correctly for non-ASCII word characters.
            let escaped = regex::escape(entry);
            if let Ok(re) = Regex::new(&format!("(?i){}", escaped)) {
                for m in re.find_iter(text) {
                    let mention = m.as_str().to_string();
                    if seen.insert(mention.clone()) {
                        results.push(ExtractedEntity {
                            mention,
                            entity_type: entity_type.clone(),
                            language: String::new(),
                            confidence,
                            canonical: None,
                            span: Some((m.start(), m.end())),
                        });
                    }
                }
            }
        } else {
            // Latin/ASCII: use word boundaries for precision
            let pattern = format!(r"(?i)\b{}\b", regex::escape(entry));
            if let Ok(re) = Regex::new(&pattern) {
                for m in re.find_iter(text) {
                    let mention = m.as_str().to_string();
                    if seen.insert(mention.clone()) {
                        results.push(ExtractedEntity {
                            mention,
                            entity_type: entity_type.clone(),
                            language: String::new(),
                            confidence,
                            canonical: None,
                            span: Some((m.start(), m.end())),
                        });
                    }
                }
            }
        }
    }

    results
}

/// Match patterns with an iterable of compiled regexes.
#[allow(dead_code)]
fn multi_regex_matches(
    text: &str,
    patterns: &[&Regex],
    entity_type: EntityType,
    confidence: f64,
) -> Vec<ExtractedEntity> {
    let mut results = Vec::new();
    for re in patterns {
        results.extend(regex_matches(text, re, entity_type.clone(), confidence));
    }
    results
}

// ─── Company suffix patterns (per language) ────────────────────────────────────

fn company_suffix_re(lang: &str) -> Option<&'static Regex> {
    match lang {
        "en" => Some(&RE_COMPANY_EN),
        "fr" => Some(&RE_COMPANY_FR),
        "de" => Some(&RE_COMPANY_DE),
        "es" => Some(&RE_COMPANY_ES),
        "it" => Some(&RE_COMPANY_IT),
        "pt" => Some(&RE_COMPANY_PT),
        "nl" => Some(&RE_COMPANY_NL),
        "tr" => Some(&RE_COMPANY_TR),
        "zh" => Some(&RE_COMPANY_ZH),
        "ja" => Some(&RE_COMPANY_JA),
        "ko" => Some(&RE_COMPANY_KO),
        _ => None,
    }
}

fn extract_company_suffixes(text: &str, lang: &str, confidence: f64) -> Vec<ExtractedEntity> {
    if let Some(re) = company_suffix_re(lang) {
        regex_matches(text, re, EntityType::Organization, confidence)
    } else {
        Vec::new()
    }
}

// ─── Compiled regexes: company suffixes ────────────────────────────────────────

macro_rules! build_company_regex {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| {
            RegexBuilder::new($pattern)
                .size_limit(200_000)
                .dfa_size_limit(200_000)
                .build()
                .unwrap_or_else(|error| {
                    panic!("invalid company regex {}: {error}", stringify!($name))
                })
        });
    };
}

build_company_regex!(
    RE_COMPANY_EN,
    r"\b([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+){0,3})\s+(Inc\.?|Corp\.?|Ltd\.?|LLC|Co\.|Group|Holdings|Technologies|Electronics|Manufacturing|Services|Limited|Corporation|Incorporated|Enterprises|International)\b"
);
build_company_regex!(
    RE_COMPANY_FR,
    r"\b([A-ZÀ-Ÿ][A-Za-zà-ÿ]+(?:\s+[A-ZÀ-Ÿ][A-Za-zà-ÿ]+){0,3})\s+(S\.?A\.?(?:S\.?)?|S\.?A\.?R\.?L\.?|EURL|SASU|SARL|SA|EURL|SNC|SCS|CA|Group|Sciences|Technologies|Électronique|Manufacturing)\b"
);
build_company_regex!(
    RE_COMPANY_DE,
    r"\b([A-ZÄÖÜß][A-Za-zäöüß]+(?:\s+[A-ZÄÖÜß][A-Za-zäöüß]+){0,3})\s+(GmbH|AG|SE\s?&?\s?Co\.?\s?KG|GmbH\s?&?\s?Co\.?\s?KG|KG|OHG|UG|e\.?V\.?|Group|Technologies|Elektronik|Manufacturing)\b"
);
build_company_regex!(
    RE_COMPANY_ES,
    r"\b([A-ZÁÉÍÓÚÜÑ][A-Za-záéíóúüñ]+(?:\s+[A-ZÁÉÍÓÚÜÑ][A-Za-záéíóúüñ]+){0,3})\s+(S\.?A\.?|S\.?L\.?|S\.?A\.?P\.?I\.?|S\.?L\.?U\.?|S\.?C\.?|CORP|Group|Tecnologías|Electrónica|Manufacturing)\b"
);
build_company_regex!(
    RE_COMPANY_IT,
    r"\b([A-ZÀ-Ÿ][A-Za-zà-ÿ]+(?:\s+[A-ZÀ-Ÿ][A-Za-zà-ÿ]+){0,3})\s+(S\.?p\.?A\.?|S\.?r\.?l\.?|S\.?a\.?s\.?|S\.?n\.?c\.?|SOCIETÀ|Group|Tecnologie|Elettronica|Manufacturing)\b"
);
build_company_regex!(
    RE_COMPANY_PT,
    r"\b([A-ZÁÉÍÓÚÂÃÇÊÕ][A-Za-záéíóúâãçêõ]+(?:\s+[A-ZÁÉÍÓÚÂÃÇÊÕ][A-Za-záéíóúâãçêõ]+){0,3})\s+(S\.?A\.?|Ltda\.?|S\.?A\.?R\.?L\.?|Group|Tecnologias|Eletrónica|Manufacturing)\b"
);
build_company_regex!(
    RE_COMPANY_NL,
    r"\b([A-Z][A-Za-z]+(?:\s+[A-Z][A-Za-z]+){0,3})\s+(BV|NV|CV|VOF|Group|Technologieën|Elektronica|Manufacturing)\b"
);
build_company_regex!(
    RE_COMPANY_TR,
    r"\b([A-ZİĞÜŞÖÇ][A-Za-zığüşöç]+(?:\s+[A-ZİĞÜŞÖÇ][A-Za-zığüşöç]+){0,3})\s+(A\.?Ş\.?|Ltd\.?Şti\.?|Tic\.?|San\.?|Group|Teknolojileri|Elektronik|Üretim)\b"
);
build_company_regex!(
    RE_COMPANY_ZH,
    r"([\u4e00-\u9fff]{2,10})(?:有限公司|有限责任公司|股份有限公司|集团|控股|电子|科技|实业|制造|工业|股份公司|公司)"
);
build_company_regex!(
    RE_COMPANY_JA,
    r"([\u4e00-\u9fff\u3040-\u309f\u30a0-\u30ff]{2,10})(?:株式会社|有限会社|合同会社|会社)"
);
build_company_regex!(
    RE_COMPANY_KO,
    r"([\uac00-\ud7af\u1100-\u11ff]{2,10})(?:\(주\)|주식회사|유한회사|합자회사|회사)"
);

// ─── Compiled regexes: person names ────────────────────────────────────────────

macro_rules! build_person_regex {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| {
            RegexBuilder::new($pattern)
                .size_limit(200_000)
                .dfa_size_limit(200_000)
                .build()
                .unwrap_or_else(|error| {
                    panic!("invalid person regex {}: {error}", stringify!($name))
                })
        });
    };
}

build_person_regex!(
    RE_PERSON_EN,
    r"(?:Mr\.?|Mrs\.?|Ms\.?|Dr\.?|Prof\.?|Eng\.?|Hon\.?|Sen\.?|Rep\.?|CEO|CTO|CFO|VP|SVP|EVP|GM|Director|President|Chairman|Chairwoman)\s+([A-Z][a-z]+(?:\s+[A-Z][a-z]+){1,3})"
);
build_person_regex!(
    RE_PERSON_FR,
    r"(?:M\.|Mme|Mlle|Dr\.|Pr\.|Ing\.|Directeur|Directrice|Président|Présidente|PDG|DG|Chef)\s+([A-ZÀ-Ÿ][a-zà-ÿ]+(?:\s+[A-ZÀ-Ÿ][a-zà-ÿ]+){1,3})"
);
build_person_regex!(
    RE_PERSON_DE,
    r"(?:Herr|Frau|Dr\.|Prof\.|Dipl\.-Ing\.|Ing\.|Geschäftsführer|Vorstand|Direktor|Präsident)\s+([A-ZÄÖÜ][a-zäöüß]+(?:\s+[A-ZÄÖÜ][a-zäöüß]+){1,3})"
);
build_person_regex!(
    RE_PERSON_ES,
    r"(?:Sr\.|Sra\.|Srta\.|Dr\.|Dra\.|Prof\.|Ing\.|Lic\.|Director|Directora|Presidente|Gerente)\s+([A-ZÁÉÍÓÚÜÑ][a-záéíóúüñ]+(?:\s+[A-ZÁÉÍÓÚÜÑ][a-záéíóúüñ]+){1,3})"
);
build_person_regex!(
    RE_PERSON_IT,
    r"(?:Sig\.|Sig\.ra|Sig\.na|Dr\.|Dott\.|Dott\.ssa|Prof\.|Ing\.|Direttore|Direttrice|Presidente|Amministratore)\s+([A-ZÀ-Ÿ][a-zà-ÿ]+(?:\s+[A-ZÀ-Ÿ][a-zà-ÿ]+){1,3})"
);
build_person_regex!(
    RE_PERSON_PT,
    r"(?:Sr\.|Sra\.|Srta\.|Dr\.|Dra\.|Prof\.|Eng\.|Diretor|Diretora|Presidente|Gerente)\s+([A-ZÁÉÍÓÚÂÃÇÊÕ][a-záéíóúâãçêõ]+(?:\s+[A-ZÁÉÍÓÚÂÃÇÊÕ][a-záéíóúâãçêõ]+){1,3})"
);
build_person_regex!(
    RE_PERSON_NL,
    r"(?:Dhr\.|Mevr\.|Dr\.|Prof\.|Ir\.|Ing\.|Directeur|Voorzitter|Manager)\s+([A-Z][a-z]+(?:\s+[A-Z][a-z]+){1,3})"
);
build_person_regex!(
    RE_PERSON_TR,
    r"(?:Bay|Bayan|Dr\.|Prof\.|Müh\.|Yönetici|Müdür|Başkan|CEO)\s+([A-ZİĞÜŞÖÇ][a-zığüşöç]+(?:\s+[A-ZİĞÜŞÖÇ][a-zığüşöç]+){1,3})"
);
build_person_regex!(
    RE_PERSON_FA,
    r"(?:آقای|خانم|دکتر|مهندس|پروفسور|جناب|سرکار)\s+([\u0600-\u06FF]{2,20}(?:\s+[\u0600-\u06FF]{2,20}){0,3})"
);
build_person_regex!(
    RE_PERSON_HE,
    r"(?:מר|גב׳|ד״ר|פרופ׳|מר׳|עו״ד)\s+([\u0590-\u05FF]{2,15}(?:\s+[\u0590-\u05FF]{2,15}){1,3})"
);
build_person_regex!(
    RE_PERSON_AR,
    r"(?:السيد|السيدة|الآنسة|الدكتور|المهندس|البروفيسور|الاستاذ|الاستاذة|سعادة)\s+([\u0600-\u06FF]{2,20}(?:\s+[\u0600-\u06FF]{2,20}){0,3})"
);

// ─── Location / Geopolitical dictionaries (multi-language) ─────────────────────

/// All countries of the world (English names, works as catch-all).
static COUNTRIES_EN: &[&str] = &[
    "Afghanistan",
    "Albania",
    "Algeria",
    "Andorra",
    "Angola",
    "Argentina",
    "Armenia",
    "Australia",
    "Austria",
    "Azerbaijan",
    "Bahrain",
    "Bangladesh",
    "Belarus",
    "Belgium",
    "Benin",
    "Bolivia",
    "Bosnia and Herzegovina",
    "Botswana",
    "Brazil",
    "Brunei",
    "Bulgaria",
    "Burkina Faso",
    "Burundi",
    "Cambodia",
    "Cameroon",
    "Canada",
    "Cape Verde",
    "Central African Republic",
    "Chad",
    "Chile",
    "China",
    "Colombia",
    "Congo",
    "Costa Rica",
    "Croatia",
    "Cuba",
    "Cyprus",
    "Czech Republic",
    "Denmark",
    "Djibouti",
    "Dominican Republic",
    "Ecuador",
    "Egypt",
    "El Salvador",
    "Equatorial Guinea",
    "Eritrea",
    "Estonia",
    "Ethiopia",
    "Finland",
    "France",
    "Gabon",
    "Gambia",
    "Georgia",
    "Germany",
    "Ghana",
    "Greece",
    "Guatemala",
    "Guinea",
    "Guyana",
    "Haiti",
    "Honduras",
    "Hungary",
    "Iceland",
    "India",
    "Indonesia",
    "Iran",
    "Iraq",
    "Ireland",
    "Israel",
    "Italy",
    "Ivory Coast",
    "Jamaica",
    "Japan",
    "Jordan",
    "Kazakhstan",
    "Kenya",
    "Kuwait",
    "Kyrgyzstan",
    "Laos",
    "Latvia",
    "Lebanon",
    "Liberia",
    "Libya",
    "Liechtenstein",
    "Lithuania",
    "Luxembourg",
    "Madagascar",
    "Malawi",
    "Malaysia",
    "Maldives",
    "Mali",
    "Malta",
    "Mauritania",
    "Mauritius",
    "Mexico",
    "Moldova",
    "Monaco",
    "Mongolia",
    "Montenegro",
    "Morocco",
    "Mozambique",
    "Myanmar",
    "Namibia",
    "Nepal",
    "Netherlands",
    "New Zealand",
    "Nicaragua",
    "Niger",
    "Nigeria",
    "North Korea",
    "North Macedonia",
    "Norway",
    "Oman",
    "Pakistan",
    "Panama",
    "Paraguay",
    "Peru",
    "Philippines",
    "Poland",
    "Portugal",
    "Qatar",
    "Romania",
    "Russia",
    "Rwanda",
    "Saudi Arabia",
    "Senegal",
    "Serbia",
    "Sierra Leone",
    "Singapore",
    "Slovakia",
    "Slovenia",
    "Somalia",
    "South Africa",
    "South Korea",
    "Spain",
    "Sri Lanka",
    "Sudan",
    "Suriname",
    "Sweden",
    "Switzerland",
    "Syria",
    "Taiwan",
    "Tajikistan",
    "Tanzania",
    "Thailand",
    "Togo",
    "Tunisia",
    "Turkey",
    "Turkmenistan",
    "Uganda",
    "Ukraine",
    "United Arab Emirates",
    "United Kingdom",
    "United States",
    "Uruguay",
    "Uzbekistan",
    "Vatican City",
    "Venezuela",
    "Vietnam",
    "Yemen",
    "Zambia",
    "Zimbabwe",
];

/// Major world cities for entity extraction.
static CITIES_WORLD: &[&str] = &[
    "Tokyo",
    "Delhi",
    "Shanghai",
    "São Paulo",
    "Mumbai",
    "Beijing",
    "Cairo",
    "Dhaka",
    "Osaka",
    "Karachi",
    "Chongqing",
    "Istanbul",
    "Buenos Aires",
    "Kolkata",
    "Lagos",
    "Kinshasa",
    "Manila",
    "Tianjin",
    "Guangzhou",
    "Rio de Janeiro",
    "Lahore",
    "Bangalore",
    "Paris",
    "Bangkok",
    "London",
    "Dubai",
    "New York",
    "Singapore",
    "Hong Kong",
    "Berlin",
    "Madrid",
    "Rome",
    "Moscow",
    "Toronto",
    "Sydney",
    "Seoul",
    "Mexico City",
    "Jakarta",
    "Lima",
    "Shenzhen",
    "Munich",
    "Frankfurt",
    "Milan",
    "Barcelona",
    "Amsterdam",
    "Brussels",
    "Stockholm",
    "Oslo",
    "Copenhagen",
    "Vienna",
    "Zurich",
    "Geneva",
    "Casablanca",
    "Rabat",
    "Tunis",
    "Algiers",
    "Tripoli",
    "Abu Dhabi",
    "Riyadh",
    "Doha",
    "Kuwait City",
    "Muscat",
    "Manama",
    "Tel Aviv",
    "Jerusalem",
    "Ankara",
    "Izmir",
    "Tehran",
    "Isfahan",
    "Shiraz",
];

/// French country names.
static COUNTRIES_FR: &[&str] = &[
    "France",
    "Allemagne",
    "Italie",
    "Espagne",
    "Royaume-Uni",
    "Belgique",
    "Suisse",
    "Pays-Bas",
    "Portugal",
    "Suède",
    "Norvège",
    "Danemark",
    "Finlande",
    "Pologne",
    "Autriche",
    "Grèce",
    "Tunisie",
    "Maroc",
    "Algérie",
    "États-Unis",
    "Canada",
    "Chine",
    "Japon",
    "Corée du Sud",
    "Inde",
    "Brésil",
    "Mexique",
    "Argentine",
    "Russie",
    "Turquie",
    "Émirats arabes unis",
    "Arabie saoudite",
    "Qatar",
    "Koweït",
    "Oman",
    "Bahreïn",
    "Israël",
    "Égypte",
    "Afrique du Sud",
    "Nigeria",
    "Sénégal",
    "Côte d'Ivoire",
    "Mali",
    "Vietnam",
    "Thaïlande",
    "Indonésie",
    "Malaisie",
    "Singapour",
    "Taïwan",
];

/// German country names.
static COUNTRIES_DE: &[&str] = &[
    "Deutschland",
    "Frankreich",
    "Italien",
    "Spanien",
    "Österreich",
    "Schweiz",
    "Niederlande",
    "Belgien",
    "Schweden",
    "Norwegen",
    "Dänemark",
    "Polen",
    "Tschechien",
    "Ungarn",
    "Rumänien",
    "Griechenland",
    "Türkei",
    "Vereinigte Staaten",
    "Kanada",
    "China",
    "Japan",
    "Südkorea",
    "Indien",
    "Brasilien",
    "Mexiko",
    "Russland",
    "Vereinigtes Königreich",
    "Vereinigte Arabische Emirate",
    "Saudi-Arabien",
    "Katar",
    "Israel",
    "Ägypten",
    "Südafrika",
    "Tunesien",
    "Marokko",
    "Algerien",
];

/// Spanish country names.
static COUNTRIES_ES: &[&str] = &[
    "España",
    "Francia",
    "Italia",
    "Alemania",
    "Portugal",
    "Reino Unido",
    "Países Bajos",
    "Bélgica",
    "Suiza",
    "Suecia",
    "Noruega",
    "Dinamarca",
    "Polonia",
    "Grecia",
    "Turquía",
    "Marruecos",
    "Túnez",
    "Argelia",
    "Estados Unidos",
    "Canadá",
    "México",
    "Argentina",
    "Brasil",
    "Chile",
    "Colombia",
    "Perú",
    "China",
    "Japón",
    "Corea del Sur",
    "India",
    "Rusia",
    "Emiratos Árabes Unidos",
    "Arabia Saudita",
    "Israel",
    "Egipto",
];

/// Arabic country names.
static COUNTRIES_AR: &[&str] = &[
    "مصر",
    "السعودية",
    "الإمارات",
    "قطر",
    "الكويت",
    "عمان",
    "البحرين",
    "الأردن",
    "العراق",
    "سوريا",
    "لبنان",
    "فلسطين",
    "اليمن",
    "ليبيا",
    "تونس",
    "الجزائر",
    "المغرب",
    "السودان",
    "موريتانيا",
    "الصومال",
    "تركيا",
    "إيران",
    "فرنسا",
    "ألمانيا",
    "إيطاليا",
    "إسبانيا",
    "الولايات المتحدة",
    "بريطانيا",
    "كندا",
    "الصين",
    "اليابان",
    "الهند",
    "روسيا",
    "البرازيل",
    "أستراليا",
];

/// Chinese country/region names.
static COUNTRIES_ZH: &[&str] = &[
    "中国",
    "美国",
    "日本",
    "韩国",
    "德国",
    "法国",
    "英国",
    "意大利",
    "西班牙",
    "加拿大",
    "澳大利亚",
    "印度",
    "俄罗斯",
    "巴西",
    "墨西哥",
    "荷兰",
    "瑞士",
    "瑞典",
    "新加坡",
    "马来西亚",
    "泰国",
    "越南",
    "阿联酋",
    "沙特阿拉伯",
    "卡塔尔",
    "以色列",
    "土耳其",
    "伊朗",
    "埃及",
    "南非",
    "尼日利亚",
    "台湾",
    "香港",
];

// ─── Threat actor dictionaries ────────────────────────────────────────────────

/// Known threat actor groups (multi-language).
static THREAT_ACTORS_GLOBAL: &[&str] = &[
    "APT28",
    "APT29",
    "APT10",
    "APT41",
    "Lazarus Group",
    "Kimsuky",
    "Krypton",
    "DarkHotel",
    "OceanLotus",
    "APT32",
    "Mustang Panda",
    "TEMPEST",
    "Sandworm",
    "Fancy Bear",
    "Cozy Bear",
    "APT1",
    "APT33",
    "APT34",
    "Silent Librarian",
    "TA444",
    "TA505",
    "FIN7",
    "FIN8",
    "Carbanak",
    "Cobalt Group",
    "Silence Group",
    "TrickBot Gang",
    "Conti",
    "REvil",
    "DarkSide",
    "BlackCat",
    "LockBit",
    "Hive",
    "Black Basta",
    "Clop",
    "Cactus",
    "BianLian",
    "Royal",
    "Medusa",
    "Akira",
    "Volt Typhoon",
    "Flax Typhoon",
    "Panda",
    // Chinese state-sponsored
    "APT40",
    "APT37",
    "Gallium",
    "CactusPete",
    "DragonOK",
    "Emissary Panda",
    "Deep Panda",
    "Comment Crew",
    "APT12",
    // Russian state-sponsored
    "Turla",
    "Venomous Bear",
    "Energetic Bear",
    "TeleBots",
    "BlackEnergy",
    "Dragonfly",
    "APT28",
    "Zebrocy",
    "Gamaredon",
    // Iranian state-sponsored
    "MuddyWater",
    "APT33",
    "APT34",
    "OilRig",
    "Charming Kitten",
    "Fox Kitten",
    "Tortoiseshell",
    "Rocket Kitten",
    // North Korean
    "Lazarus",
    "BlueNoroff",
    "Andariel",
    "DarkSeoul",
];

/// Known EMS / electronics companies (supply-chain domain dictionary).
static KNOWN_EMS_COMPANIES: &[&str] = &[
    "Foxconn",
    "Hon Hai",
    "Jabil",
    "Flex",
    "Celestica",
    "Benchmark Electronics",
    "Starz Electronics",
    "Plexus",
    "Sanmina",
    "Venture Corporation",
    "Pegatron",
    "Wistron",
    "Compal Electronics",
    "Quanta Computer",
    "USI",
    "Universal Scientific Industrial",
    "Inventec",
    "Sercomm",
    "Arcadyan Technology",
    "Accton Technology",
    "Delta Electronics",
    "Lite-On Technology",
    "Kinpo Electronics",
    "Mitac Holdings",
    "Wistron NeWeb",
    "Alpha Networks",
    "Kaimei Electronics",
    "BYD Electronic",
    "DBG Technology",
    "FIH Mobile",
    // EMS in Europe
    "Videoton",
    "Elcoteq",
    "Selcom",
    "Enics",
    "GPV Group",
    "Kitron",
    "Scanfil",
    "Note",
    "Incus",
    "Lacroix Electronics",
    "One Solution Group",
    "AT&S",
    "Schweizer Electronic",
    "Rohde & Schwarz",
    "Kontron",
    "ASM Assembly Systems",
    // EMS in Americas
    "Jabil Inc",
    "Flex Ltd",
    "Sanmina Corporation",
    "Plexus Corp",
    "Creation Technologies",
    "Spartronics",
    "Electronic Assembly",
    "EPM",
    "SigmaTronix",
    "Sypris Electronics",
    "IEC Electronics",
    "Marine Electronics",
    "Schweiger Electronics",
    // EMS in APAC
    "Hana Microelectronics",
    "Fabrinet",
    "SMT Technologies",
    "BEC Electronics",
    "SVI Public Company",
    "Honeywell EMS",
    "MicroStencil",
    "Chengdu Galaxy",
    "Shenzhen Kaifa Technology",
    "Longsys Electronics",
    "Tongfu Microelectronics",
];

// ─── Language-specific entity extractors ──────────────────────────────────────

/// English NER.
fn extract_english_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns
    entities.extend(extract_company_suffixes(text, "en", 0.85));

    // Person names with honorifics
    entities.extend(regex_matches(text, &RE_PERSON_EN, EntityType::Person, 0.80));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_EN,
        EntityType::GeoPolitical,
        0.85,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

/// French NER.
fn extract_french_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns (SAS, SARL, SA, etc.)
    entities.extend(extract_company_suffixes(text, "fr", 0.85));

    // Person names with French honorifics
    entities.extend(regex_matches(text, &RE_PERSON_FR, EntityType::Person, 0.80));

    // Known EMS companies (same global list)
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries (French names)
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_FR,
        EntityType::GeoPolitical,
        0.85,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    // French-specific procurement terms as Organization indicators
    let fr_org_keywords = multilingual::procurement_keywords("fr");
    for kw in fr_org_keywords {
        let pattern = format!(r"(?i)\b({})\b", regex::escape(kw));
        if let Ok(re) = Regex::new(&pattern) {
            entities.extend(regex_matches(text, &re, EntityType::Organization, 0.50));
        }
    }

    entities
}

/// Arabic NER.
fn extract_arabic_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Person names with Arabic honorifics
    entities.extend(regex_matches(text, &RE_PERSON_AR, EntityType::Person, 0.80));

    // Company patterns: Arabic legal forms
    // Arabic usually places the legal form BEFORE the company name: شركة فوكسكون
    let ar_company_prefix = RegexBuilder::new(
        r"(?:شركة|مجموعة|مؤسسة|بنك|مصنع|معمل|وكالة)\s+([\u0600-\u06FF]{2,20}(?:\s+[\u0600-\u06FF]{2,20}){0,3})"
    ).size_limit(200_000).dfa_size_limit(200_000).build().unwrap_or_else(|e| panic!("invalid arabic company regex: {e}"));
    entities.extend(regex_matches(
        text,
        &ar_company_prefix,
        EntityType::Organization,
        0.80,
    ));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.85,
    ));

    // Countries (Arabic names)
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_AR,
        EntityType::GeoPolitical,
        0.85,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    // Arabic procurement terms
    let ar_kws = multilingual::procurement_keywords("ar");
    for kw in ar_kws {
        let pattern = format!(r"({})", regex::escape(kw));
        if let Ok(re) = Regex::new(&pattern) {
            entities.extend(regex_matches(text, &re, EntityType::Organization, 0.50));
        }
    }

    entities
}

/// Chinese NER.
fn extract_chinese_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company names with Chinese legal forms
    entities.extend(extract_company_suffixes(text, "zh", 0.85));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.85,
    ));

    // Countries (Chinese names)
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_ZH,
        EntityType::GeoPolitical,
        0.85,
    ));

    // Person names: Chinese 2-3 character names (common surname + given name pattern)
    let zh_person_re = RegexBuilder::new(r"([\u4e00-\u9fff]{2,3}(?:[\u4e00-\u9fff]{1,2})?)")
        .size_limit(100_000)
        .dfa_size_limit(100_000)
        .build()
        .unwrap_or_else(|e| panic!("invalid chinese person regex: {e}"));
    // Persons matched with lower confidence — ambiguous with other entities
    entities.extend(regex_matches(text, &zh_person_re, EntityType::Person, 0.40));

    // Chinese procurement terms
    let zh_kws = multilingual::procurement_keywords("zh");
    for kw in zh_kws {
        if let Ok(re) = Regex::new(&regex::escape(kw)) {
            entities.extend(regex_matches(text, &re, EntityType::Organization, 0.50));
        }
    }

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

/// Japanese NER.
fn extract_japanese_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company names with Japanese legal forms
    entities.extend(extract_company_suffixes(text, "ja", 0.85));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.85,
    ));

    // Countries
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_ZH,
        EntityType::GeoPolitical,
        0.80,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    // Japanese procurement terms
    let ja_kws = multilingual::procurement_keywords("ja");
    for kw in ja_kws {
        if let Ok(re) = Regex::new(&regex::escape(kw)) {
            entities.extend(regex_matches(text, &re, EntityType::Organization, 0.50));
        }
    }

    entities
}

/// Korean NER.
fn extract_korean_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company names with Korean legal forms
    entities.extend(extract_company_suffixes(text, "ko", 0.85));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.85,
    ));

    // Countries
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_ZH,
        EntityType::GeoPolitical,
        0.80,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    // Korean procurement terms
    let ko_kws = multilingual::procurement_keywords("ko");
    for kw in ko_kws {
        if let Ok(re) = Regex::new(&regex::escape(kw)) {
            entities.extend(regex_matches(text, &re, EntityType::Organization, 0.50));
        }
    }

    entities
}

/// German NER.
fn extract_german_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns (GmbH, AG, etc.)
    entities.extend(extract_company_suffixes(text, "de", 0.85));

    // Person names with German honorifics
    entities.extend(regex_matches(text, &RE_PERSON_DE, EntityType::Person, 0.80));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries (German names)
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_DE,
        EntityType::GeoPolitical,
        0.85,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    // German procurement keywords
    let de_kws = multilingual::procurement_keywords("de");
    for kw in de_kws {
        let pattern = format!(r"(?i)\b({})\b", regex::escape(kw));
        if let Ok(re) = Regex::new(&pattern) {
            entities.extend(regex_matches(text, &re, EntityType::Organization, 0.50));
        }
    }

    entities
}

/// Spanish NER.
fn extract_spanish_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns
    entities.extend(extract_company_suffixes(text, "es", 0.85));

    // Person names with Spanish honorifics
    entities.extend(regex_matches(text, &RE_PERSON_ES, EntityType::Person, 0.80));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries (Spanish names)
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_ES,
        EntityType::GeoPolitical,
        0.85,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    // Spanish procurement keywords
    let es_kws = multilingual::procurement_keywords("es");
    for kw in es_kws {
        let pattern = format!(r"(?i)\b({})\b", regex::escape(kw));
        if let Ok(re) = Regex::new(&pattern) {
            entities.extend(regex_matches(text, &re, EntityType::Organization, 0.50));
        }
    }

    entities
}

/// Italian NER.
fn extract_italian_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns
    entities.extend(extract_company_suffixes(text, "it", 0.85));

    // Person names with Italian honorifics
    entities.extend(regex_matches(text, &RE_PERSON_IT, EntityType::Person, 0.80));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_ES,
        EntityType::GeoPolitical,
        0.80,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

/// Portuguese NER.
fn extract_portuguese_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns
    entities.extend(extract_company_suffixes(text, "pt", 0.85));

    // Person names with Portuguese honorifics
    entities.extend(regex_matches(text, &RE_PERSON_PT, EntityType::Person, 0.80));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_FR,
        EntityType::GeoPolitical,
        0.80,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

/// Dutch NER.
fn extract_dutch_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns (BV, NV)
    entities.extend(extract_company_suffixes(text, "nl", 0.85));

    // Person names with Dutch honorifics
    entities.extend(regex_matches(text, &RE_PERSON_NL, EntityType::Person, 0.80));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_DE,
        EntityType::GeoPolitical,
        0.80,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

/// Turkish NER.
fn extract_turkish_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Company suffix patterns
    entities.extend(extract_company_suffixes(text, "tr", 0.85));

    // Person names with Turkish honorifics
    entities.extend(regex_matches(text, &RE_PERSON_TR, EntityType::Person, 0.80));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.90,
    ));

    // Countries
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_AR,
        EntityType::GeoPolitical,
        0.75,
    ));

    // Cities
    entities.extend(known_entity_matches(
        text,
        CITIES_WORLD,
        EntityType::Location,
        0.75,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

/// Persian NER.
fn extract_persian_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Person names with Persian honorifics
    entities.extend(regex_matches(text, &RE_PERSON_FA, EntityType::Person, 0.80));

    // Company patterns: Persian legal forms
    let fa_company_re = RegexBuilder::new(
        r"([\u0600-\u06FF]{2,20}(?:\s+[\u0600-\u06FF]{2,20}){0,3})\s*(?:شرکت|گروه|موسسه|بانک|کارخانه|شرکت)\s"
    ).size_limit(200_000).dfa_size_limit(200_000).build().unwrap_or_else(|e| panic!("invalid persian company regex: {e}"));
    entities.extend(regex_matches(
        text,
        &fa_company_re,
        EntityType::Organization,
        0.80,
    ));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.85,
    ));

    // Countries (Persian names - simplified: re-use Arabic country list as many overlap)
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_AR,
        EntityType::GeoPolitical,
        0.80,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

/// Hebrew NER.
fn extract_hebrew_entities(text: &str) -> Vec<ExtractedEntity> {
    let mut entities = Vec::new();

    // Person names with Hebrew honorifics
    entities.extend(regex_matches(text, &RE_PERSON_HE, EntityType::Person, 0.80));

    // Company patterns: Hebrew legal forms
    // Note: Hebrew abbreviation גרשיים (״) is used in בע״מ (Ltd.) — we use \x{0022}
    // to avoid raw string conflicts, since " terminates r"..." raw strings.
    let he_company_re = RegexBuilder::new(
        r#"([\u0590-\u05FF]{2,15}(?:\s+[\u0590-\u05FF]{2,15}){0,2})\s*(?:בע"מ|ע"מ|קבוצת|חברת|בנק)"#,
    )
    .size_limit(200_000)
    .dfa_size_limit(200_000)
    .build()
    .unwrap_or_else(|e| panic!("invalid hebrew company regex: {e}"));
    entities.extend(regex_matches(
        text,
        &he_company_re,
        EntityType::Organization,
        0.80,
    ));

    // Known EMS companies
    entities.extend(known_entity_matches(
        text,
        KNOWN_EMS_COMPANIES,
        EntityType::Organization,
        0.85,
    ));

    // Countries (Hebrew names - use English list as fallback)
    entities.extend(known_entity_matches(
        text,
        COUNTRIES_AR,
        EntityType::GeoPolitical,
        0.70,
    ));

    // Threat actors
    entities.extend(known_entity_matches(
        text,
        THREAT_ACTORS_GLOBAL,
        EntityType::ThreatActor,
        0.85,
    ));

    entities
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── English ─────────────────────────────────────────────────────────────

    #[test]
    fn test_english_company_suffix() {
        let text = "Foxconn Technology Group announced a new facility.";
        let entities = extract_english_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("Foxconn")));
    }

    #[test]
    fn test_english_person_with_honorific() {
        let text = "CEO John Smith announced the results.";
        let entities = extract_english_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("John Smith")));
    }

    #[test]
    fn test_english_country() {
        let text = "The factory in Tunisia produces electronics for export to Germany.";
        let entities = extract_english_entities(text);
        let geo: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::GeoPolitical)
            .collect();
        assert!(geo.iter().any(|e| e.mention == "Tunisia"));
        assert!(geo.iter().any(|e| e.mention == "Germany"));
    }

    #[test]
    fn test_english_threat_actor() {
        let text = "Lazarus Group was identified in the recent campaign.";
        let entities = extract_english_entities(text);
        let ta: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::ThreatActor)
            .collect();
        assert!(ta.iter().any(|e| e.mention.contains("Lazarus")));
    }

    #[test]
    fn test_english_known_ems_company() {
        let text = "Jabil Inc and Flex Ltd are major EMS providers.";
        let entities = extract_english_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("Jabil")));
        assert!(orgs.iter().any(|e| e.mention.contains("Flex")));
    }

    #[test]
    fn test_english_city() {
        let text = "The headquarters is in Singapore.";
        let entities = extract_english_entities(text);
        let locs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Location)
            .collect();
        assert!(locs.iter().any(|e| e.mention == "Singapore"));
    }

    // ── French ──────────────────────────────────────────────────────────────

    #[test]
    fn test_french_company_suffix() {
        let text = "Starz Electronics SAS a annoncé une nouvelle usine.";
        let entities = extract_french_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("SAS")));
    }

    #[test]
    fn test_french_person() {
        let text = "M. Jean Dupont est le directeur général.";
        let entities = extract_french_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("Jean Dupont")));
    }

    #[test]
    fn test_french_country() {
        let text = "L'usine est située en Tunisie et au Maroc.";
        let entities = extract_french_entities(text);
        let geo: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::GeoPolitical)
            .collect();
        assert!(geo.iter().any(|e| e.mention == "Tunisie"));
        assert!(geo.iter().any(|e| e.mention == "Maroc"));
    }

    // ── Arabic ──────────────────────────────────────────────────────────────

    #[test]
    fn test_arabic_person() {
        let text = "السيد أحمد بن سالم يتحدث في المؤتمر";
        let entities = extract_arabic_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(!persons.is_empty());
    }

    #[test]
    fn test_arabic_company() {
        let text = "شركة فوكسكون للالكترونيات";
        let entities = extract_arabic_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        // The company regex matches "شركة" as suffix or known EMS "Foxconn" might match
        assert!(orgs
            .iter()
            .any(|e| e.mention.contains("فوكسكون") || e.mention.contains("Foxconn")));
    }

    #[test]
    fn test_arabic_country() {
        let text = "مصر والسعودية والإمارات دول عربية";
        let entities = extract_arabic_entities(text);
        let geo: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::GeoPolitical)
            .collect();
        assert!(geo.iter().any(|e| e.mention == "مصر"));
    }

    // ── Chinese ─────────────────────────────────────────────────────────────

    #[test]
    fn test_chinese_company() {
        let text = "富士康科技有限公司宣布投资新工厂";
        let entities = extract_chinese_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(
            orgs.iter().any(|e| e.mention.contains("科技"))
                || orgs.iter().any(|e| e.mention.contains("富士康"))
        );
    }

    #[test]
    fn test_chinese_country() {
        let text = "中国和美国是重要的贸易伙伴";
        let entities = extract_chinese_entities(text);
        let geo: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::GeoPolitical)
            .collect();
        assert!(
            geo.iter().any(|e| e.mention == "中国"),
            "Expected '中国' in geo entities: {:?}",
            geo
        );
        assert!(
            geo.iter().any(|e| e.mention == "美国"),
            "Expected '美国' in geo entities: {:?}",
            geo
        );
    }

    // ── Japanese ────────────────────────────────────────────────────────────

    #[test]
    fn test_japanese_company() {
        let text = "富士通株式会社が新工場を開設";
        let entities = extract_japanese_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("株式会社")));
    }

    // ── Korean ──────────────────────────────────────────────────────────────

    #[test]
    fn test_korean_company() {
        let text = "삼성전자(주)가 새로운 공장을 설립";
        let entities = extract_korean_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("(주)")));
    }

    // ── German ──────────────────────────────────────────────────────────────

    #[test]
    fn test_german_company() {
        let text = "Siemens AG hat einen neuen Vertrag unterzeichnet.";
        let entities = extract_german_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("AG")));
    }

    #[test]
    fn test_german_person() {
        let text = "Herr Klaus Müller ist der Geschäftsführer.";
        let entities = extract_german_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("Klaus Müller")));
    }

    #[test]
    fn test_german_country() {
        let text = "Das Werk in Frankreich und Spanien produziert EMS.";
        let entities = extract_german_entities(text);
        let geo: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::GeoPolitical)
            .collect();
        assert!(geo.iter().any(|e| e.mention == "Frankreich"));
        assert!(geo.iter().any(|e| e.mention == "Spanien"));
    }

    // ── Spanish ─────────────────────────────────────────────────────────────

    #[test]
    fn test_spanish_company() {
        let text = "Indra SL ha anunciado un nuevo contrato.";
        let entities = extract_spanish_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("SL")));
    }

    #[test]
    fn test_spanish_person() {
        let text = "El Sr. Carlos García es el presidente.";
        let entities = extract_spanish_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("Carlos García")));
    }

    #[test]
    fn test_spanish_country() {
        let text = "México y España son socios comerciales.";
        let entities = extract_spanish_entities(text);
        let geo: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::GeoPolitical)
            .collect();
        assert!(geo
            .iter()
            .any(|e| e.mention == "México" || e.mention == "Mexico"));
    }

    // ── Italian ─────────────────────────────────────────────────────────────

    #[test]
    fn test_italian_person() {
        let text = "Il Dott. Marco Rossi ha presentato i risultati.";
        let entities = extract_italian_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("Marco Rossi")));
    }

    // ── Portuguese ───────────────────────────────────────────────────────────

    #[test]
    fn test_portuguese_person() {
        let text = "O Dr. João Silva é o diretor da empresa.";
        let entities = extract_portuguese_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("João Silva")));
    }

    // ── Turkish ──────────────────────────────────────────────────────────────

    #[test]
    fn test_turkish_company() {
        let text = "Koç Holding A.Ş. yeni bir yatırım açıkladı.";
        let entities = extract_turkish_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("A.Ş")));
    }

    #[test]
    fn test_turkish_person() {
        let text = "Bay Mehmet Yılmaz yönetim kurulu başkanıdır.";
        let entities = extract_turkish_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("Mehmet Yılmaz")));
    }

    // ── Persian ──────────────────────────────────────────────────────────────

    #[test]
    fn test_persian_person() {
        let text = "آقای محمد رضایی در این کنفرانس سخنرانی کرد";
        let entities = extract_persian_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(!persons.is_empty());
    }

    // ── Hebrew ──────────────────────────────────────────────────────────────

    #[test]
    fn test_hebrew_person() {
        let text = "מר דוד כהן מונה למנכ\"ל החברה";
        let entities = extract_hebrew_entities(text);
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(!persons.is_empty());
    }

    // ── Dutch ───────────────────────────────────────────────────────────────

    #[test]
    fn test_dutch_company() {
        let text = "ASML BV heeft een nieuw record bereikt.";
        let entities = extract_dutch_entities(text);
        let orgs: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Organization)
            .collect();
        assert!(orgs.iter().any(|e| e.mention.contains("BV")));
    }

    // ── Dispatch (extract_entities) ─────────────────────────────────────────

    #[test]
    fn test_dispatch_auto_detects_language() {
        let text = "John Smith CEO of Foxconn Technology Group announced results.";
        let entities = extract_entities(text, "auto");
        assert!(!entities.is_empty());
        // Should have at least some entities
        assert!(entities.iter().any(|e| e.language == "en"));
    }

    #[test]
    fn test_dispatch_french() {
        let text = "M. Jean Dupont dirige Starz Electronics SAS à Tunis.";
        let entities = extract_entities(text, "fr");
        assert!(!entities.is_empty());
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("Jean Dupont")));
    }

    #[test]
    fn test_dispatch_arabic() {
        let text = "السيد أحمد بن سالم في شركة فوكسكون";
        let entities = extract_entities(text, "ar");
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_dispatch_chinese() {
        let text = "富士康科技有限公司宣布投资";
        let entities = extract_entities(text, "zh");
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_dispatch_german() {
        let text = "Herr Klaus Müller ist Geschäftsführer der Siemens AG.";
        let entities = extract_entities(text, "de");
        assert!(!entities.is_empty());
        let persons: Vec<_> = entities
            .iter()
            .filter(|e| e.entity_type == EntityType::Person)
            .collect();
        assert!(persons.iter().any(|e| e.mention.contains("Klaus Müller")));
    }

    #[test]
    fn test_dispatch_spanish() {
        let text = "El Sr. Carlos García es presidente de Indra SL.";
        let entities = extract_entities(text, "es");
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_dispatch_fallback_english() {
        let text = "Apple Inc. CEO John Smith announced results in Tunisia.";
        let entities = extract_entities(text, "xx");
        assert!(
            !entities.is_empty(),
            "Expected entities from English fallback"
        );
        assert!(entities.iter().all(|e| e.language == "en"));
    }

    #[test]
    fn test_deduplicate_entities() {
        let mut entities = vec![
            ExtractedEntity {
                mention: "Foxconn Technology Group".into(),
                entity_type: EntityType::Organization,
                language: "en".into(),
                confidence: 0.7,
                canonical: None,
                span: Some((0, 22)),
            },
            ExtractedEntity {
                mention: "Foxconn Technology Group".into(),
                entity_type: EntityType::Organization,
                language: "en".into(),
                confidence: 0.9,
                canonical: None,
                span: Some((0, 22)),
            },
        ];
        deduplicate_entities(&mut entities);
        assert_eq!(entities.len(), 1);
        assert!((entities[0].confidence - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_empty_text() {
        let entities = extract_entities("", "auto");
        assert!(entities.is_empty());
    }

    #[test]
    fn test_known_ems_company_matches() {
        let text = "Foxconn";
        let entities =
            known_entity_matches(text, KNOWN_EMS_COMPANIES, EntityType::Organization, 0.9);
        assert!(entities.iter().any(|e| e.mention == "Foxconn"));
    }

    #[test]
    fn test_extract_entities_with_lang() {
        let text = "John Smith CEO of Foxconn";
        let entities = extract_entities_with_lang(text, "en");
        assert!(!entities.is_empty());
    }

    #[test]
    fn test_mixed_language_content() {
        let text = "Apple Inc. 苹果 M. Jean Dupont";
        let entities = extract_entities(text, "auto");
        assert!(!entities.is_empty());
    }
}
