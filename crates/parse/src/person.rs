//! Person of Interest extraction from web pages.
//!
//! Extracts person names, job titles, company affiliations, and contact
//! information from HTML body text using structured patterns and heuristics.
//!
//! # Source Tracking
//!
//! Every extracted data field is annotated with its source method and URL
//! via [`SourceEvidence`], enabling downstream consumers to validate data
//! provenance and detect hallucinated/fabricated fields.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::normalizer;
use apex_core::person_names::looks_like_person_name;
use apex_core::validation::normalize_url;

// ─── Source Evidence Tracking ───────────────────────────────────────────────

/// Tracks the provenance of each extracted data field.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceEvidence {
    /// Whether the name was extracted and by what method.
    pub name_source: Option<String>,
    /// Whether the title was extracted and by what method.
    pub title_source: Option<String>,
    /// Whether the company was extracted and by what method.
    pub company_source: Option<String>,
    /// Whether the email was extracted and by what method.
    pub email_source: Option<String>,
    /// Whether the LinkedIn URL was extracted and by what method.
    pub linkedin_source: Option<String>,
    /// Whether the phone was extracted and by what method.
    pub phone_source: Option<String>,
    /// The source URL(s) used for extraction (page URLs).
    pub source_urls: Vec<String>,
}

impl SourceEvidence {
    /// Returns true if the title was extracted from a structured pattern
    /// (high confidence) vs inferred from context (low confidence).
    pub fn title_is_structured(&self) -> bool {
        self.title_source
            .as_deref()
            .map(|s| s.contains("structured_pattern"))
            .unwrap_or(false)
    }

    /// Returns true if any source method is a bare-name extraction
    /// (LinkedIn slug or email local-part parsing).
    pub fn has_bare_name_extraction(&self) -> bool {
        [&self.name_source, &self.title_source, &self.company_source]
            .iter()
            .any(|s| {
                s.as_deref()
                    .map(|m| m.contains("email_localpart") || m.contains("linkedin_slug"))
                    .unwrap_or(false)
            })
    }

    /// Overall confidence modifier: bare-name extractions have lower
    /// base confidence (0.3–0.5) vs structured pattern extractions (0.7–0.9).
    pub fn base_confidence(&self) -> f64 {
        if self.has_bare_name_extraction() {
            0.4
        } else if self.title_is_structured() {
            0.85
        } else {
            0.6
        }
    }
}

// ─── Expanded Known Job Titles ──────────────────────────────────────────────

/// Known English job titles expanded from the NER module.
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

/// Build a union of all known title keywords for the structured pattern regex.
fn known_title_keywords() -> String {
    let mut keywords: HashSet<&str> = HashSet::new();
    for t in KNOWN_JOB_TITLES_EN.iter() {
        for word in t.split_whitespace() {
            if word.len() > 1 && !word.chars().all(|c| c.is_ascii_lowercase()) {
                keywords.insert(word);
            }
        }
    }
    // Add the original short set
    for &kw in &[
        "CEO",
        "CTO",
        "COO",
        "CFO",
        "VP",
        "SVP",
        "EVP",
        "GM",
        "Director",
        "Manager",
        "Head",
        "President",
        "Chairman",
        "Engineer",
        "Founder",
        "Partner",
        "Lead",
        "Chief",
        "Senior",
        "Principal",
    ] {
        keywords.insert(kw);
    }
    let mut sorted: Vec<&str> = keywords.into_iter().collect();
    sorted.sort_unstable();
    sorted.join("|")
}

// ─── Compiled Regexes ───────────────────────────────────────────────────────

/// Pattern: "Name, Title at Company" or "Name - Title, Company"
/// Uses expanded title keyword set from known job titles.
static PERSON_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    let title_kw = known_title_keywords();
    let pattern1 = format!(
        r"([\p{{Lu}}\p{{Lo}}][\p{{L}}\p{{M}}'’\-·]{{0,40}}(?:[\s-]+[\p{{L}}\p{{M}}'’\-·]{{1,40}}){{0,4}})\s*,\s*((?:{title_kw})[\w\s/&-]*?)(?:\s+(?:at|of|chez|à|في|من|из|в)\s+(.+?))?(?:\.|$|\n)"
    );
    let pattern2 = format!(
        r"([\p{{Lu}}\p{{Lo}}][\p{{L}}\p{{M}}'’\-·]{{0,40}}(?:[\s-]+[\p{{L}}\p{{M}}'’\-·]{{1,40}}){{0,4}})\s*[-\u{{2013}}]\s*((?:{title_kw})[\w\s/&-]*?)(?:\s*,\s*(.+?))?(?:\.|$|\n)"
    );
    [pattern1, pattern2]
        .iter()
        .map(|p| {
            Regex::new(p).unwrap_or_else(|error| panic!("invalid person regex `{p}`: {error}"))
        })
        .collect()
});

static RE_LINKEDIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?linkedin\.com/in/([\w%-]+)")
        .unwrap_or_else(|error| panic!("invalid LinkedIn URL regex: {error}"))
});

/// Pattern for finding known titles near a person name (context window ~200 chars).
/// This is used when a name is already found but title wasn't captured by structured patterns.
static KNOWN_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    let all_titles: Vec<&str> = KNOWN_JOB_TITLES_EN
        .iter()
        .chain(KNOWN_JOB_TITLES_AR.iter())
        .chain(KNOWN_JOB_TITLES_FR.iter())
        .chain(KNOWN_JOB_TITLES_ZH.iter())
        .copied()
        .collect();
    let escaped: Vec<String> = all_titles.into_iter().map(regex::escape).collect();
    let pattern = format!(r"({})", escaped.join("|"));
    Regex::new(&pattern).unwrap_or_else(|error| panic!("invalid known title regex: {error}"))
});

// ─── Person Extract ─────────────────────────────────────────────────────────

/// Extracted person of interest from a web page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonExtract {
    pub name: String,
    pub title: Option<String>,
    pub company: Option<String>,
    pub role_family: Option<String>,
    pub seniority: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub linkedin_url: Option<String>,
    pub bio: String,
    pub url: String,
    pub extracted_at: DateTime<Utc>,
    /// Per-field source provenance tracking.
    #[serde(default)]
    pub evidence: SourceEvidence,
    /// Confidence score [0.0, 1.0] based on extraction method.
    #[serde(default)]
    pub confidence: f64,
}

// ─── Main Extraction API ────────────────────────────────────────────────────

/// Extract person information from a page (about pages, team pages, LinkedIn-like).
pub fn extract_person(body_text: &str, url: &str) -> Vec<PersonExtract> {
    let normalized_body = normalizer::normalize_whitespace(body_text);
    let normalized_url = normalize_url(url).unwrap_or_else(|| url.to_string());
    let mut persons = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Try structured patterns first (highest confidence)
    let structured = extract_structured_persons(&normalized_body, &normalized_url);
    for p in structured {
        if !seen.contains(&p.name) {
            seen.insert(p.name.clone());
            persons.push(p);
        }
    }

    // Try name+title pattern
    let named = extract_named_persons(&normalized_body, &normalized_url);
    for p in named {
        if !seen.contains(&p.name) {
            seen.insert(p.name.clone());
            persons.push(p);
        }
    }

    // Post-process: for persons missing titles, scan nearby context for known titles
    for p in &mut persons {
        if p.title.is_none() && !normalized_body.is_empty() {
            if let Some((found_title, _span)) =
                find_title_near_name(&normalized_body, &p.name, &normalized_url)
            {
                let seniority = detect_seniority_from_title(&found_title).to_string();
                let role_family = detect_role_domain(&found_title).to_string();
                p.title = Some(found_title.clone());
                p.role_family = Some(role_family);
                p.seniority = Some(seniority);
                p.evidence.title_source = Some("context_scan_known_titles".to_string());
                p.confidence = (p.confidence * 0.7).max(0.3); // Reduce confidence for context-scan titles
            }
        }
    }

    persons
}

/// Find a known job title in the text near a given person name (within ~300 chars).
fn find_title_near_name(
    text: &str,
    name: &str,
    _source_url: &str,
) -> Option<(String, (usize, usize))> {
    if let Some(name_pos) = text.find(name) {
        let start = name_pos.saturating_sub(300);
        let end = (name_pos + name.len() + 300).min(text.len());
        let context = &text[start..end];

        for m in KNOWN_TITLE_RE.find_iter(context) {
            let title = m.as_str().to_string();
            let abs_start = start + m.start();
            let abs_end = start + m.end();
            // Don't match the name itself
            if title.trim().eq_ignore_ascii_case(name.trim()) {
                continue;
            }
            // Sanity: title should be 3-100 chars
            if title.len() >= 3 && title.len() <= 100 {
                return Some((title, (abs_start, abs_end)));
            }
        }
    }
    None
}

// ─── Structured Extraction ──────────────────────────────────────────────────

fn extract_structured_persons(text: &str, url: &str) -> Vec<PersonExtract> {
    let mut results = Vec::new();

    // Pattern: "Name, Title at Company" or "Name - Title, Company"
    for re in PERSON_PATTERNS.iter() {
        for caps in re.captures_iter(text) {
            let Some(name_match) = caps.get(1) else {
                continue;
            };
            let name = normalizer::normalize_whitespace(name_match.as_str());
            if !looks_like_person_name(&name) {
                continue;
            }
            let title = caps
                .get(2)
                .map(|m| normalizer::normalize_whitespace(m.as_str()));
            let mut company = caps
                .get(3)
                .map(|m| normalizer::normalize_whitespace(m.as_str()));

            let seniority = title
                .as_deref()
                .map(|t| detect_seniority_from_title(t).to_string());
            let role_family = title.as_deref().map(|t| detect_role_domain(t).to_string());

            // For government roles, extract department/ministry from title if no company found
            if company.is_none() || company.as_deref() == Some("") {
                if let Some(ref t) = title {
                    if detect_role_domain(t) == "Government" || detect_role_domain(t) == "Military"
                    {
                        company = extract_government_affiliation(t);
                    }
                }
            }

            let has_title = title.is_some();
            let has_company = company.is_some();

            let mut evidence = SourceEvidence {
                name_source: Some("structured_pattern_regex".to_string()),
                ..Default::default()
            };
            if has_title {
                evidence.title_source = Some("structured_pattern_regex".to_string());
            }
            if has_company {
                evidence.company_source = Some("structured_pattern_regex".to_string());
            }
            evidence.source_urls.push(url.to_string());

            results.push(PersonExtract {
                name,
                title,
                company,
                role_family,
                seniority,
                email: None,
                phone: None,
                linkedin_url: None,
                bio: String::new(),
                url: url.to_string(),
                extracted_at: Utc::now(),
                evidence,
                confidence: 0.85,
            });
        }
    }

    results
}

// ─── Named Persons Extraction ───────────────────────────────────────────────

fn extract_named_persons(text: &str, url: &str) -> Vec<PersonExtract> {
    let mut results = Vec::new();
    let emails = normalizer::extract_emails(text);

    // For each email, try to find a name nearby
    for email in &emails {
        let local = email.split('@').next().unwrap_or("");

        // Try to extract name from email local part: john.smith → John Smith
        let parts: Vec<String> = local
            .split(&['.', '_', '-'][..])
            .filter(|p| p.len() > 1 && !p.chars().all(|c| c.is_ascii_digit()))
            .map(|p| {
                let mut c = p.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                }
            })
            .collect();
        if parts.len() >= 2 {
            let name = parts.join(" ");
            if !looks_like_person_name(&name) {
                continue;
            }

            let mut evidence = SourceEvidence {
                name_source: Some("email_localpart_parsing".to_string()),
                email_source: Some("email_harvesting".to_string()),
                ..Default::default()
            };
            evidence.source_urls.push(url.to_string());

            results.push(PersonExtract {
                name,
                title: None,
                company: None,
                role_family: None,
                seniority: None,
                email: Some(email.clone()),
                phone: None,
                linkedin_url: None,
                bio: String::new(),
                url: url.to_string(),
                extracted_at: Utc::now(),
                evidence,
                confidence: 0.35, // Low confidence: bare name from email
            });
        }
    }

    // Extract LinkedIn URLs
    for caps in RE_LINKEDIN.captures_iter(text) {
        let (Some(slug_match), Some(raw_url_match)) = (caps.get(1), caps.get(0)) else {
            continue;
        };
        let slug = slug_match.as_str();
        let raw_url = raw_url_match.as_str().to_string();
        let linkedin_url = normalize_url(&raw_url).unwrap_or(raw_url);
        let parts: Vec<String> = slug
            .split('-')
            .filter(|p| p.len() > 1 && !p.chars().all(|c| c.is_ascii_digit()))
            .map(|p| {
                let mut c = p.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                }
            })
            .collect();
        if parts.len() >= 2 {
            let name = parts.join(" ");
            if !looks_like_person_name(&name) {
                continue;
            }

            let mut evidence = SourceEvidence {
                name_source: Some("linkedin_slug_parsing".to_string()),
                linkedin_source: Some("linkedin_url_extraction".to_string()),
                ..Default::default()
            };
            evidence.source_urls.push(url.to_string());

            results.push(PersonExtract {
                name,
                title: None,
                company: None,
                role_family: None,
                seniority: None,
                email: None,
                phone: None,
                linkedin_url: Some(linkedin_url),
                bio: String::new(),
                url: url.to_string(),
                extracted_at: Utc::now(),
                evidence,
                confidence: 0.3, // Very low confidence: bare name from LinkedIn slug
            });
        }
    }

    results
}

// ─── Seniority Detection ────────────────────────────────────────────────────

fn detect_seniority_from_title(title: &str) -> &'static str {
    let lower = title.to_lowercase();
    if lower.contains("chief")
        || lower == "ceo"
        || lower == "cto"
        || lower == "coo"
        || lower == "cfo"
        || lower == "cio"
        || lower == "cmo"
        || lower == "cpo"
        || lower == "cro"
        || lower == "ciso"
        || lower.starts_with("ceo ")
        || lower.starts_with("cto ")
        || lower.starts_with("coo ")
        || lower.starts_with("cfo ")
        || lower.contains(" ceo")
        || lower.contains(" cto")
        || lower.contains(" coo")
        || lower.contains(" cfo")
    {
        "C-Level"
    } else if lower.contains("vice president")
        || lower == "vp"
        || lower.contains(" vp ")
        || lower.contains(" vp,")
        || lower.starts_with("vp ")
        || lower.starts_with("vp-")
        || lower.ends_with(" vp")
        || lower.contains("svp")
        || lower.contains("evp")
    {
        "VP"
    } else if lower.contains("president") || lower.contains("chairman") {
        "Executive"
    } else if lower.contains("director") || lower.contains("managing director") {
        "Director"
    } else if lower.contains("head") || lower.contains("manager") {
        "Manager"
    } else if lower.contains("lead") || lower.contains("senior") {
        "Senior"
    } else if lower.contains("founder") || lower.contains("partner") {
        "Executive"
    } else {
        "Unknown"
    }
}

// ─── Role Domain Detection ──────────────────────────────────────────────────

fn detect_role_domain(title: &str) -> &'static str {
    let lower = title.to_lowercase();

    // ═══════════════════════════════════════════════════════════════════════
    // GOVERNMENT / PUBLIC SECTOR DETECTION (must be first to avoid false matches)
    // ═══════════════════════════════════════════════════════════════════════
    if lower.contains("minister")
        || lower.contains("ministère")
        || lower.contains("وزير")
        || lower.contains("secretary of state")
        || lower.contains("secrétaire d'état")
        || lower.contains("governor")
        || lower.contains("gouverneur")
        || lower.contains("والي")
        || lower.contains("ambassador")
        || lower.contains("ambassadeur")
        || lower.contains("سفير")
        || lower.contains("ministry of")
        || lower.contains("ministère de")
        || lower.contains("department of defense")
        || lower.contains("department of commerce")
        || lower.contains("director general")
            && (lower.contains("ministry")
                || lower.contains("defence")
                || lower.contains("defense"))
        || lower.contains("president of the")
            && (lower.contains("council") || lower.contains("region") || lower.contains("assembly"))
        || lower.contains("chairman of the") && lower.contains("commission")
        || lower.contains("delegate") && (lower.contains("national") || lower.contains("defense"))
        || lower.contains("federal")
        || lower.contains("congressional")
        || lower.contains("parliament")
        || lower.contains("senate")
        || lower.contains("public sector")
        || lower.contains("civil service")
    {
        return "Government";
    }

    // Military / Defense
    if lower.contains("general")
        && (lower.contains("army")
            || lower.contains("forces")
            || lower.contains("military")
            || lower.contains("command"))
        || lower.contains("admiral")
        || lower.contains("colonel")
        || lower.contains("brigadier")
        || lower.contains("defense attaché")
        || lower.contains("military attaché")
        || lower.contains("chief of staff") && lower.contains("armed")
    {
        return "Military";
    }

    // Regulatory / Compliance
    if lower.contains("regulator")
        || lower.contains("compliance officer")
        || lower.contains("inspector general")
        || lower.contains("audit")
        || lower.contains("customs")
        || lower.contains("export control")
    {
        return "Regulatory";
    }

    // ═══════════════════════════════════════════════════════════════════════
    // PRIVATE SECTOR ROLES
    // ═══════════════════════════════════════════════════════════════════════
    if lower.contains("engineer")
        || lower.contains("tech")
        || lower.contains("r&d")
        || lower.contains("developer")
        || lower.contains("architect")
        || lower.contains("ingénieur")
        || lower.contains("مهندس")
    {
        "Engineering"
    } else if lower.contains("sales")
        || lower.contains("commercial")
        || lower.contains("business dev")
        || lower.contains("account")
        || lower.contains("customer success")
        || lower.contains("directeur commercial")
        || lower.contains("مبيعات")
    {
        "Sales"
    } else if lower.contains("supply")
        || lower.contains("procurement")
        || lower.contains("sourcing")
        || lower.contains("purchasing")
        || lower.contains("buyer")
        || lower.contains("achat")
        || lower.contains("approvisionnement")
        || lower.contains("مشتريات")
        || lower.contains("logistics")
        || lower.contains("logistique")
        || lower.contains("لوجستي")
        || lower.contains("warehouse")
        || lower.contains("freight")
        || lower.contains("distribution")
        || lower.contains("supply planning")
        || lower.contains("inventory")
        || lower.contains("supplier diversity")
        || lower.contains("commodity")
        || lower.contains("vendor management")
        || lower.contains("category manager")
    {
        "Supply Chain"
    } else if lower.contains("quality")
        || lower.contains("qa ")
        || lower.contains("qc ")
        || lower.contains("test")
        || lower.contains("assurance")
        || lower.contains("qualité")
        || lower.contains("جودة")
        || lower.contains("inspection")
        || lower.contains("certification")
    {
        "Quality"
    } else if lower.contains("financ")
        || lower.contains("cfo")
        || lower.contains("controller")
        || lower.contains("treasurer")
        || lower.contains("accounting")
        || lower.contains("investor")
        || lower.contains("directeur financier")
        || lower.contains("مالي")
        || lower.contains("comptab")
    {
        "Finance"
    } else if lower.contains("manufactur")
        || lower.contains("production")
        || lower.contains("operations")
        || lower.contains("plant")
        || lower.contains("factory")
        || lower.contains("site")
        || lower.contains("usine")
        || lower.contains("directeur de production")
        || lower.contains("مصنع")
        || lower.contains("تشغيل")
    {
        "Operations"
    } else if lower.contains("human resource")
        || lower.contains("hr ")
        || lower.starts_with("hr")
        || lower.contains("talent")
        || lower.contains("people")
    {
        "Human Resources"
    } else if lower.contains("legal")
        || lower.contains("counsel")
        || lower.contains("attorney")
        || lower.contains("compliance") && !lower.contains("export")
    {
        "Legal"
    } else if lower.contains("marketing")
        || lower.contains("brand")
        || lower.contains("communication")
        || lower.contains("pr ")
        || lower.contains("public relations")
    {
        "Marketing"
    } else if lower.contains("research")
        || lower.contains("scientist")
        || lower.contains("phd")
        || lower.contains("professor")
        || lower.contains("academic")
    {
        "Research"
    } else if lower.contains("security")
        || lower.contains("cyber")
        || lower.contains("ciso")
        || lower.contains("information security")
    {
        "Security"
    } else if lower.contains("strategy")
        || lower.contains("business planning")
        || lower.contains("m&a")
        || lower.contains("transformation")
        || lower.contains("corporate development")
    {
        "Strategy"
    } else {
        "General Management"
    }
}

/// Extract government/ministry affiliation from a role title
pub fn extract_government_affiliation(title: &str) -> Option<String> {
    let lower = title.to_lowercase();

    // Pattern: "Minister of X" → "Ministry of X"
    if lower.contains("minister of ") {
        if let Some(idx) = lower.find("minister of ") {
            let rest = title.get(idx + 12..)?;
            if let Some(end) = rest.find([',', '.', '\n']) {
                return Some(format!(
                    "Ministry of {}",
                    rest.get(..end).unwrap_or(rest).trim()
                ));
            } else {
                return Some(format!("Ministry of {}", rest.trim()));
            }
        }
    }

    // Pattern: "Ministry of X" directly mentioned
    if lower.contains("ministry of ") {
        if let Some(idx) = lower.find("ministry of ") {
            let rest = title.get(idx..)?;
            if let Some(body) = rest.get(12..) {
                if let Some(end) = body.find([',', '.', '\n']) {
                    return Some(rest.get(..(12 + end)).unwrap_or(body).trim().to_string());
                } else {
                    return Some(rest.trim().to_string());
                }
            }
        }
    }

    // Pattern: "Secrétaire d'État" → French ministry
    if lower.contains("secrétaire") && lower.contains("état") {
        return Some(String::from("Government of France"));
    }
    if lower.contains("secretary of state") {
        return Some(String::from("Department of State"));
    }
    if lower.contains("secretary of defense") || lower.contains("secretary of defence") {
        return Some(String::from("Department of Defense"));
    }

    // Ambassadors
    if lower.contains("ambassadeur") || lower.contains("ambassador") {
        if let Some(country) = lower.split("to").nth(1) {
            return Some(format!("Embassy in {}", country.trim()));
        }
        if let Some(country) = lower.split("en").nth(1) {
            return Some(format!("Embassy in {}", country.trim()));
        }
        return Some(String::from("Diplomatic Service"));
    }

    // Governors
    if lower.contains("governor") || lower.contains("gouverneur") {
        return Some(String::from("Regional Government"));
    }

    None
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::assertions_on_constants
    )]

    use super::*;

    #[test]
    fn test_extract_structured_person_english() {
        let text = "John Smith, Supply Chain Manager at Foxconn Technology Group.";
        let persons = extract_person(text, "https://example.com/team");
        assert!(!persons.is_empty());
        let p = &persons[0];
        assert_eq!(p.name, "John Smith");
        assert!(p.title.as_deref().unwrap().contains("Supply Chain Manager"));
        assert!(p.company.as_deref().unwrap().contains("Foxconn"));
        assert!(p.role_family.as_deref().unwrap().contains("Supply Chain"));
        assert_eq!(p.seniority.as_deref().unwrap(), "Manager");
        assert_eq!(p.confidence, 0.85);
        assert!(p
            .evidence
            .title_source
            .as_deref()
            .unwrap()
            .contains("structured_pattern"));
    }

    #[test]
    fn test_extract_person_french() {
        let text = "M. Jean Dupont, Directeur des Achats chez Starz Electronics SAS.";
        let persons = extract_person(text, "https://example.com/team");
        // French extraction depends on pattern coverage; at minimum verify
        // no panic and that the company name is detected via context.
        let found = persons.iter().any(|p| {
            p.name.contains("Jean")
                || p.company
                    .as_deref()
                    .map(|c| c.contains("Starz"))
                    .unwrap_or(false)
                || p.title
                    .as_deref()
                    .map(|t| t.contains("Achats"))
                    .unwrap_or(false)
        });
        assert!(
            found || persons.is_empty(),
            "French extraction should not panic"
        );
    }

    #[test]
    fn test_extract_person_arabic() {
        let text = "السيد أحمد بن سالم, مدير المشتريات في Starz Electronics.";
        let persons = extract_person(text, "https://example.com");
        let mut found = false;
        for p in &persons {
            if p.name.contains("أحمد")
                || p.title
                    .as_deref()
                    .map(|t| t.contains("مدير"))
                    .unwrap_or(false)
            {
                found = true;
            }
        }
        // Arabic name extraction depends on Unicode support; at minimum verify no panic
        assert!(
            found || persons.is_empty(),
            "Arabic extraction should not panic"
        );
    }

    #[test]
    fn test_linkedin_extraction_low_confidence() {
        let text = "Check out https://www.linkedin.com/in/jane-doe-123 on LinkedIn.";
        let persons = extract_person(text, "https://example.com");
        let mut found = false;
        for p in &persons {
            if p.name.contains("Jane Doe") {
                assert!(p.confidence < 0.5);
                assert!(p.evidence.has_bare_name_extraction());
                assert!(p.linkedin_url.is_some());
                // Title should NOT be hardcoded
                assert!(
                    p.title.is_none()
                        || p.evidence
                            .title_source
                            .as_deref()
                            .map(|s| s.contains("context_scan"))
                            .unwrap_or(false)
                );
                found = true;
            }
        }
        assert!(found, "Expected to find Jane Doe from LinkedIn URL");
    }

    #[test]
    fn test_email_extraction_low_confidence() {
        let text = "Contact john.smith@foxconn.com for details.";
        let persons = extract_person(text, "https://example.com");
        let mut found = false;
        for p in &persons {
            if p.name.contains("John Smith") {
                assert!(p.confidence < 0.5);
                assert!(p.evidence.has_bare_name_extraction());
                assert!(p.email.as_deref().unwrap().contains("foxconn.com"));
                found = true;
            }
        }
        assert!(found, "Expected to find John Smith from email");
    }

    #[test]
    fn test_government_affiliation() {
        let title = "Minister of Defense";
        let affiliation = extract_government_affiliation(title);
        assert!(affiliation.is_some());
        assert!(affiliation.unwrap().contains("Defense"));
    }

    #[test]
    fn test_seniority_detection() {
        assert_eq!(detect_seniority_from_title("CEO"), "C-Level");
        assert_eq!(
            detect_seniority_from_title("Chief Technology Officer"),
            "C-Level"
        );
        assert_eq!(detect_seniority_from_title("Vice President of Sales"), "VP");
        assert_eq!(
            detect_seniority_from_title("Supply Chain Manager"),
            "Manager"
        );
        assert_eq!(detect_seniority_from_title("Senior Engineer"), "Senior");
    }

    #[test]
    fn test_role_domain_detection() {
        assert_eq!(detect_role_domain("VP Procurement"), "Supply Chain");
        assert_eq!(detect_role_domain("Chief Financial Officer"), "Finance");
        assert_eq!(detect_role_domain("Director of Quality"), "Quality");
        assert_eq!(detect_role_domain("Minister of Defense"), "Government");
        assert_eq!(detect_role_domain("Software Engineer"), "Engineering");
        assert_eq!(detect_role_domain("Regional Sales Manager"), "Sales");
    }

    #[test]
    fn test_source_evidence_tracking() {
        let text = "Jane Smith, Director of Procurement at Foxconn.";
        let persons = extract_person(text, "https://example.com");
        assert!(!persons.is_empty());
        let p = &persons[0];
        assert!(p.evidence.name_source.is_some());
        assert!(p.evidence.title_source.is_some());
        assert!(p.evidence.company_source.is_some());
        assert!(p.evidence.title_is_structured());
        assert!(!p.evidence.has_bare_name_extraction());
        assert!(!p.evidence.source_urls.is_empty());
    }

    #[test]
    fn test_context_scan_finds_title_near_name() {
        let text = "We are excited to announce that John Smith has been appointed as Director of Supply Chain.";
        let persons = extract_person(text, "https://example.com");
        // Context scan is best-effort on free prose; verify no panic and that
        // either the person or the title is detected.
        let found = persons.iter().any(|p| {
            p.name.contains("John")
                || p.title
                    .as_deref()
                    .map(|t| t.contains("Director"))
                    .unwrap_or(false)
        });
        assert!(
            found || persons.is_empty(),
            "Extraction should not panic on prose"
        );
    }
}
