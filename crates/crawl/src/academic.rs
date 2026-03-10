//! Academic publication tracker.
//!
//! Monitor Google Scholar, arXiv, IEEE Xplore, and PubMed for publications
//! by POI-linked researchers. Detect new patents, papers, and technical
//! disclosures that may signal R&D direction shifts.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Data model ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PublicationSource {
    GoogleScholar,
    ArXiv,
    IeeeXplore,
    PubMed,
    Scopus,
    SemanticScholar,
}

impl PublicationSource {
    pub fn as_str(&self) -> &str {
        match self {
            Self::GoogleScholar => "google_scholar",
            Self::ArXiv => "arxiv",
            Self::IeeeXplore => "ieee_xplore",
            Self::PubMed => "pubmed",
            Self::Scopus => "scopus",
            Self::SemanticScholar => "semantic_scholar",
        }
    }

    pub fn rss_template(&self) -> Option<&str> {
        match self {
            Self::GoogleScholar => Some("https://scholar.google.com/scholar?as_sdt=0,5&q=author:\"{}\"&hl=en&as_vis=1&output=rss"),
            Self::ArXiv => Some("https://export.arxiv.org/api/query?search_query=au:\"{}\"&sortBy=submittedDate&sortOrder=descending&max_results=20"),
            Self::PubMed => Some("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pubmed&term={}[author]&retmax=20&sort=date"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcademicPublication {
    pub title: String,
    pub authors: Vec<String>,
    pub source: PublicationSource,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
    pub url: String,
    pub abstract_text: Option<String>,
    pub published_date: Option<NaiveDate>,
    pub venue: Option<String>,
    pub citation_count: Option<u32>,
    pub keywords: Vec<String>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorProfile {
    pub display_name: String,
    /// Name variants for search (transliterated forms, maiden names)
    pub name_variants: Vec<String>,
    /// Linked POI entity ID (if any)
    pub poi_entity_id: Option<i64>,
    /// Linked company entity ID (if any)
    pub company_entity_id: Option<i64>,
    /// Known affiliation
    pub affiliation: Option<String>,
    /// Research areas of interest
    pub research_areas: Vec<String>,
    /// Google Scholar profile ID (if known)
    pub scholar_id: Option<String>,
}

// ─── Research topic classification ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResearchDomain {
    Electronics,
    WireHarness,
    Semiconductor,
    Materials,
    Manufacturing,
    AI,
    Cybersecurity,
    DefenseTech,
    Energy,
    Telecom,
    Other,
}

impl ResearchDomain {
    pub fn classify(title: &str, abstract_text: Option<&str>) -> Self {
        let text = format!(
            "{} {}",
            title.to_lowercase(),
            abstract_text.unwrap_or("").to_lowercase()
        );

        if text.contains("wire harness")
            || text.contains("wiring")
            || text.contains("cable assembly")
            || text.contains("connector")
        {
            Self::WireHarness
        } else if text.contains("semiconductor")
            || text.contains("wafer")
            || text.contains("chip")
            || text.contains("transistor")
            || text.contains("cmos")
        {
            Self::Semiconductor
        } else if text.contains("pcb")
            || text.contains("circuit board")
            || text.contains("smt")
            || text.contains("electronic")
            || text.contains("capacitor")
            || text.contains("resistor")
        {
            Self::Electronics
        } else if text.contains("polymer")
            || text.contains("composite")
            || text.contains("alloy")
            || text.contains("coating")
            || text.contains("material")
        {
            Self::Materials
        } else if text.contains("manufacturing")
            || text.contains("quality control")
            || text.contains("lean")
            || text.contains("six sigma")
            || text.contains("automation")
        {
            Self::Manufacturing
        } else if text.contains("machine learning")
            || text.contains("neural network")
            || text.contains("deep learning")
            || text.contains("nlp")
            || text.contains("artificial intelligence")
        {
            Self::AI
        } else if text.contains("cyber")
            || text.contains("cryptograph")
            || text.contains("malware")
            || text.contains("intrusion")
        {
            Self::Cybersecurity
        } else if text.contains("defense")
            || text.contains("defence")
            || text.contains("militar")
            || text.contains("radar")
            || text.contains("missile")
        {
            Self::DefenseTech
        } else if text.contains("solar")
            || text.contains("battery")
            || text.contains("energy storage")
            || text.contains("wind turbine")
        {
            Self::Energy
        } else if text.contains("5g")
            || text.contains("antenna")
            || text.contains("rf ")
            || text.contains("wireless")
            || text.contains("telecom")
        {
            Self::Telecom
        } else {
            Self::Other
        }
    }
}

// ─── Alert generation ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct PublicationAlert {
    pub author: String,
    pub poi_entity_id: Option<i64>,
    pub company_entity_id: Option<i64>,
    pub publication: AcademicPublication,
    pub domain: ResearchDomain,
    pub significance: Significance,
    pub signal_description: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum Significance {
    High,
    Medium,
    Low,
}

pub fn assess_significance(
    pub_: &AcademicPublication,
    author: &AuthorProfile,
    domain: &ResearchDomain,
) -> Significance {
    let mut score = 0u32;

    // Domain relevance to EMS industry
    match domain {
        ResearchDomain::WireHarness | ResearchDomain::Electronics => score += 3,
        ResearchDomain::Semiconductor | ResearchDomain::Manufacturing => score += 2,
        ResearchDomain::Materials | ResearchDomain::DefenseTech => score += 2,
        ResearchDomain::Energy | ResearchDomain::Telecom => score += 1,
        _ => {}
    }

    // Author is linked to a monitored entity
    if author.poi_entity_id.is_some() {
        score += 2;
    }
    if author.company_entity_id.is_some() {
        score += 1;
    }

    // High-profile venue
    if let Some(ref venue) = pub_.venue {
        let v = venue.to_lowercase();
        if v.contains("nature") || v.contains("science") || v.contains("ieee transactions") {
            score += 2;
        }
    }

    // Recent publication
    if let Some(date) = pub_.published_date {
        let days_ago = (Utc::now().date_naive() - date).num_days();
        if days_ago < 7 {
            score += 2;
        } else if days_ago < 30 {
            score += 1;
        }
    }

    // Citation count (for older papers that are trending)
    if let Some(citations) = pub_.citation_count {
        if citations > 50 {
            score += 2;
        } else if citations > 10 {
            score += 1;
        }
    }

    if score >= 7 {
        Significance::High
    } else if score >= 4 {
        Significance::Medium
    } else {
        Significance::Low
    }
}

pub fn generate_signal_description(
    pub_: &AcademicPublication,
    author: &AuthorProfile,
    domain: &ResearchDomain,
) -> String {
    let venue_str = pub_
        .venue
        .as_deref()
        .map(|v| format!(" in {}", v))
        .unwrap_or_default();

    format!(
        "{} (affiliated with {}) published \"{}\"{} in the {:?} domain. \
         This may indicate R&D investment direction for {} and related entities.",
        author.display_name,
        author.affiliation.as_deref().unwrap_or("unknown"),
        pub_.title,
        venue_str,
        domain,
        author
            .affiliation
            .as_deref()
            .unwrap_or(&author.display_name),
    )
}

// ─── Batch processing ───────────────────────────────────────────────────

pub fn process_publications(
    publications: &[AcademicPublication],
    authors: &[AuthorProfile],
) -> Vec<PublicationAlert> {
    let mut author_map: HashMap<String, &AuthorProfile> = HashMap::new();
    for author in authors {
        let key = author.display_name.to_lowercase();
        author_map.insert(key.clone(), author);
        for variant in &author.name_variants {
            author_map.insert(variant.to_lowercase(), author);
        }
    }

    let mut alerts = Vec::new();
    for pub_ in publications {
        for pub_author in &pub_.authors {
            let key = pub_author.to_lowercase();
            if let Some(author) = author_map.get(&key) {
                let domain = ResearchDomain::classify(&pub_.title, pub_.abstract_text.as_deref());
                let significance = assess_significance(pub_, author, &domain);
                let signal = generate_signal_description(pub_, author, &domain);

                alerts.push(PublicationAlert {
                    author: author.display_name.clone(),
                    poi_entity_id: author.poi_entity_id,
                    company_entity_id: author.company_entity_id,
                    publication: pub_.clone(),
                    domain,
                    significance,
                    signal_description: signal,
                });
                break; // one alert per publication
            }
        }
    }

    alerts
}

// ─── SQL helpers ────────────────────────────────────────────────────────

pub fn sql_create_publications_table() -> &'static str {
    "CREATE TABLE IF NOT EXISTS academic_publications (
        id            BIGSERIAL PRIMARY KEY,
        title         TEXT NOT NULL,
        authors       TEXT[] NOT NULL,
        source        TEXT NOT NULL,
        doi           TEXT,
        arxiv_id      TEXT,
        url           TEXT NOT NULL,
        abstract_text TEXT,
        published_date DATE,
        venue         TEXT,
        citation_count INTEGER,
        keywords      TEXT[],
        domain        TEXT,
        fetched_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
        UNIQUE(doi),
        UNIQUE(arxiv_id)
    )"
}

pub fn sql_create_author_profiles_table() -> &'static str {
    "CREATE TABLE IF NOT EXISTS author_profiles (
        id              BIGSERIAL PRIMARY KEY,
        display_name    TEXT NOT NULL,
        name_variants   TEXT[] NOT NULL DEFAULT '{}',
        poi_entity_id   BIGINT REFERENCES entities(id),
        company_entity_id BIGINT REFERENCES entities(id),
        affiliation     TEXT,
        research_areas  TEXT[] NOT NULL DEFAULT '{}',
        scholar_id      TEXT,
        UNIQUE(display_name)
    )"
}

pub fn sql_insert_publication() -> &'static str {
    "INSERT INTO academic_publications
     (title, authors, source, doi, arxiv_id, url, abstract_text,
      published_date, venue, citation_count, keywords, domain)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
     ON CONFLICT DO NOTHING
     RETURNING id"
}

pub fn sql_recent_by_author() -> &'static str {
    "SELECT * FROM academic_publications
     WHERE $1 = ANY(authors)
     ORDER BY published_date DESC NULLS LAST
     LIMIT 50"
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_author() -> AuthorProfile {
        AuthorProfile {
            display_name: "Dr. Ahmed Ben Ali".into(),
            name_variants: vec!["Ahmed Benali".into(), "A. Ben Ali".into()],
            poi_entity_id: Some(42),
            company_entity_id: Some(100),
            affiliation: Some("Telnet Holding".into()),
            research_areas: vec!["electronics".into(), "wire harness".into()],
            scholar_id: None,
        }
    }

    fn sample_publication() -> AcademicPublication {
        AcademicPublication {
            title: "Novel Wire Harness Routing Algorithm for Electric Vehicles".into(),
            authors: vec!["Dr. Ahmed Ben Ali".into(), "J. Smith".into()],
            source: PublicationSource::IeeeXplore,
            doi: Some("10.1109/example.2026".into()),
            arxiv_id: None,
            url: "https://ieeexplore.ieee.org/document/12345".into(),
            abstract_text: Some(
                "We present a new algorithm for optimizing wire harness routing in EVs.".into(),
            ),
            published_date: Some(Utc::now().date_naive()),
            venue: Some("IEEE Transactions on Industrial Electronics".into()),
            citation_count: Some(5),
            keywords: vec!["wire harness".into(), "EV".into(), "routing".into()],
            fetched_at: Utc::now(),
        }
    }

    #[test]
    fn test_domain_classification() {
        assert_eq!(
            ResearchDomain::classify("Wire Harness Routing", None),
            ResearchDomain::WireHarness
        );
        assert_eq!(
            ResearchDomain::classify("CMOS chip design", None),
            ResearchDomain::Semiconductor
        );
        assert_eq!(
            ResearchDomain::classify("Deep learning for NLP", None),
            ResearchDomain::AI
        );
        assert_eq!(
            ResearchDomain::classify("Abstract painting", None),
            ResearchDomain::Other
        );
    }

    #[test]
    fn test_significance_high_for_relevant_linked_pub() {
        let pub_ = sample_publication();
        let author = sample_author();
        let domain = ResearchDomain::classify(&pub_.title, pub_.abstract_text.as_deref());
        let sig = assess_significance(&pub_, &author, &domain);
        assert_eq!(sig, Significance::High);
    }

    #[test]
    fn test_significance_low_for_unlinked_other() {
        let mut pub_ = sample_publication();
        pub_.title = "Impressionist art in the 20th century".into();
        pub_.abstract_text = None;
        pub_.venue = None;
        pub_.citation_count = None;
        pub_.published_date = None;

        let mut author = sample_author();
        author.poi_entity_id = None;
        author.company_entity_id = None;

        let domain = ResearchDomain::classify(&pub_.title, pub_.abstract_text.as_deref());
        let sig = assess_significance(&pub_, &author, &domain);
        assert_eq!(sig, Significance::Low);
    }

    #[test]
    fn test_process_publications_matches_author() {
        let pubs = vec![sample_publication()];
        let authors = vec![sample_author()];
        let alerts = process_publications(&pubs, &authors);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].author, "Dr. Ahmed Ben Ali");
    }

    #[test]
    fn test_process_publications_handles_variant() {
        let mut pub_ = sample_publication();
        pub_.authors = vec!["Ahmed Benali".into()]; // variant name
        let pubs = vec![pub_];
        let authors = vec![sample_author()];
        let alerts = process_publications(&pubs, &authors);
        assert_eq!(alerts.len(), 1);
    }

    #[test]
    fn test_source_rss_template() {
        assert!(PublicationSource::GoogleScholar.rss_template().is_some());
        assert!(PublicationSource::ArXiv.rss_template().is_some());
        assert!(PublicationSource::IeeeXplore.rss_template().is_none());
    }

    #[test]
    fn test_signal_description_generated() {
        let pub_ = sample_publication();
        let author = sample_author();
        let domain = ResearchDomain::WireHarness;
        let desc = generate_signal_description(&pub_, &author, &domain);
        assert!(desc.contains("Ahmed Ben Ali"));
        assert!(desc.contains("Telnet Holding"));
        assert!(desc.contains("Wire Harness"));
    }
}
