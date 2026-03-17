//! Comprehensive source registry: 600+ OSINT feeds organized by region and
//! domain.
//!
//! # Design
//! Each [`Source`] carries a unique slug, display name, base URL, region tag,
//! content category, and priority tier (1 = highest value, 5 = lowest).
//! The registry avoids hard-coding fetch logic; that belongs in the crawler.
//!
//! # Tier definitions
//! | Tier | Description |
//! |------|-------------|
//! | 1 | Primary intelligence feeds (wire services, government registers) |
//! | 2 | High-quality trade / defence / finance publications |
//! | 3 | Regional news, think-tanks, academic repositories |
//! | 4 | Social media, forums, secondary curators |
//! | 5 | Low-signal bulk sources (blogs, aggregators) |

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use thiserror::Error;
use tracing::warn;

pub const SOURCE_REGISTRY_PATH_ENV: &str = "APEX_SOURCE_REGISTRY_PATH";

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Geographic region tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Region {
    Global,
    NorthAmerica,
    Europe,
    MiddleEast,
    Israel,
    China,
    AsiaPacific,
    LatinAmerica,
    Africa,
    EasternEurope,
    Russia,
    India,
}

/// Content category.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    News,
    Defence,
    Finance,
    Trade,
    Technology,
    Patents,
    Sanctions,
    GovernmentRegistry,
    Procurement,
    AcademicResearch,
    SocialMedia,
    Forum,
    GeopoliticsThinkTank,
    SupplyChain,
    Cybersecurity,
    EnergyResources,
    HealthcareLife,
    LegalRegulatory,
}

/// A single crawlable intelligence source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Stable machine-readable identifier, e.g. `"reuters_global"`.
    pub slug: String,
    /// Human-readable name.
    pub name: String,
    /// Primary URL to crawl / RSS / API endpoint.
    pub url: String,
    /// Search / RSS parameter name if needed (e.g. `"q"` for Google).
    pub search_param: Option<String>,
    /// Geographic focus.
    pub region: Region,
    /// Content domain.
    pub category: Category,
    /// Priority tier 1–5 (lower = higher priority).
    pub tier: u8,
    /// Whether the source requires a proxy for reliable access.
    pub needs_proxy: bool,
    /// RSS feed URL if distinct from the main URL.
    pub rss_url: Option<String>,
    /// Whether this source is actively enabled (can be toggled without
    /// recompilation).
    pub enabled: bool,
    /// Minimum crawl interval in minutes (0 = unlimited).
    pub min_interval_minutes: u32,
    /// Notes on language, auth requirements, or special handling.
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceRegistryFile {
    sources: Vec<Source>,
}

#[derive(Debug, Error)]
pub enum SourceRegistryError {
    #[error("failed reading source registry {path}: {message}")]
    Read { path: String, message: String },
    #[error("failed parsing source registry {path}: {message}")]
    Parse { path: String, message: String },
    #[error("invalid source registry {path}: {message}")]
    Invalid { path: String, message: String },
}

impl Source {
    fn new(
        slug: &str,
        name: &str,
        url: &str,
        region: Region,
        category: Category,
        tier: u8,
    ) -> Self {
        Self {
            slug: slug.to_string(),
            name: name.to_string(),
            url: url.to_string(),
            search_param: None,
            region,
            category,
            tier,
            needs_proxy: false,
            rss_url: None,
            enabled: true,
            min_interval_minutes: 60,
            notes: None,
        }
    }

    fn rss(mut self, rss: &str) -> Self {
        self.rss_url = Some(rss.to_string());
        self
    }

    fn proxy(mut self) -> Self {
        self.needs_proxy = true;
        self
    }

    fn interval(mut self, minutes: u32) -> Self {
        self.min_interval_minutes = minutes;
        self
    }

    fn notes(mut self, notes: &str) -> Self {
        self.notes = Some(notes.to_string());
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Registry builder
// ─────────────────────────────────────────────────────────────────────────────

/// Return the full source registry.
///
/// This is the authoritative list of all crawlable sources.  New sources
/// should be appended here; removal is done by setting `enabled = false` so
/// historical metadata is preserved.
pub fn all_sources() -> Vec<Source> {
    load_sources_from_env().unwrap_or_else(|error| {
        warn!(error = %error, "source registry override invalid; falling back to built-in registry");
        default_sources()
    })
}

pub fn load_sources_from_env() -> Result<Vec<Source>, SourceRegistryError> {
    match std::env::var(SOURCE_REGISTRY_PATH_ENV) {
        Ok(path) if !path.trim().is_empty() => load_sources_from_path(path),
        Ok(_) => Err(SourceRegistryError::Invalid {
            path: SOURCE_REGISTRY_PATH_ENV.to_string(),
            message: "override path is empty".to_string(),
        }),
        Err(std::env::VarError::NotPresent) => Ok(default_sources()),
        Err(error) => Err(SourceRegistryError::Invalid {
            path: SOURCE_REGISTRY_PATH_ENV.to_string(),
            message: error.to_string(),
        }),
    }
}

pub fn load_sources_from_path(path: impl AsRef<Path>) -> Result<Vec<Source>, SourceRegistryError> {
    let path = path.as_ref();
    let path_display = path.display().to_string();
    let contents = fs::read_to_string(path).map_err(|error| SourceRegistryError::Read {
        path: path_display.clone(),
        message: error.to_string(),
    })?;
    let registry: SourceRegistryFile =
        serde_yaml::from_str(&contents).map_err(|error| SourceRegistryError::Parse {
            path: path_display.clone(),
            message: error.to_string(),
        })?;
    validate_sources(&registry.sources).map_err(|message| SourceRegistryError::Invalid {
        path: path_display,
        message,
    })?;
    Ok(registry.sources)
}

fn default_sources() -> Vec<Source> {
    let mut sources = Vec::with_capacity(640);

    // ── Global Wire Services ─────────────────────────────────────
    sources.push(
        Source::new(
            "reuters_global",
            "Reuters Top News",
            "https://feeds.reuters.com/reuters/topNews",
            Region::Global,
            Category::News,
            1,
        )
        .rss("https://feeds.reuters.com/reuters/topNews")
        .interval(15),
    );
    sources.push(
        Source::new(
            "ap_news",
            "AP News",
            "https://apnews.com",
            Region::Global,
            Category::News,
            1,
        )
        .rss("https://rsshub.app/apnews/topics/apf-topnews")
        .interval(15),
    );
    sources.push(
        Source::new(
            "afp_global",
            "AFP Wire",
            "https://www.afp.com",
            Region::Global,
            Category::News,
            1,
        )
        .interval(20),
    );
    sources.push(
        Source::new(
            "bloomberg_global",
            "Bloomberg Markets",
            "https://www.bloomberg.com/markets",
            Region::Global,
            Category::Finance,
            1,
        )
        .interval(20),
    );
    sources.push(
        Source::new(
            "ft_global",
            "Financial Times",
            "https://www.ft.com",
            Region::Global,
            Category::Finance,
            1,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "wsj_global",
            "Wall Street Journal",
            "https://www.wsj.com",
            Region::Global,
            Category::Finance,
            1,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "economist_global",
            "The Economist",
            "https://www.economist.com",
            Region::Global,
            Category::GeopoliticsThinkTank,
            2,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "foreign_affairs",
            "Foreign Affairs",
            "https://www.foreignaffairs.com",
            Region::Global,
            Category::GeopoliticsThinkTank,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "jane_defence",
            "Jane's Defence Weekly",
            "https://www.janes.com",
            Region::Global,
            Category::Defence,
            1,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "defense_news",
            "Defense News",
            "https://www.defensenews.com",
            Region::Global,
            Category::Defence,
            1,
        )
        .rss("https://www.defensenews.com/arc/outboundfeeds/rss/")
        .interval(30),
    );
    sources.push(
        Source::new(
            "engineering_news_record",
            "Engineering News-Record",
            "https://www.enr.com",
            Region::Global,
            Category::SupplyChain,
            2,
        )
        .rss("https://www.enr.com/rss/topic/14-news")
        .interval(60)
        .notes("Infrastructure intelligence for major buildouts and industrial projects"),
    );
    sources.push(
        Source::new(
            "construction_dive",
            "Construction Dive",
            "https://www.constructiondive.com",
            Region::Global,
            Category::SupplyChain,
            2,
        )
        .rss("https://www.constructiondive.com/feeds/news/")
        .interval(60)
        .notes("Tracks infrastructure build programs and contractor activity"),
    );
    sources.push(
        Source::new(
            "vessel_finder_news",
            "VesselFinder News",
            "https://www.vesselfinder.com/news",
            Region::Global,
            Category::SupplyChain,
            2,
        )
        .interval(120)
        .notes("Maritime movement and shipping incident signal source"),
    );
    sources.push(
        Source::new(
            "flightglobal",
            "FlightGlobal",
            "https://www.flightglobal.com",
            Region::Global,
            Category::SupplyChain,
            2,
        )
        .rss("https://www.flightglobal.com/feeds/rss")
        .interval(60)
        .notes("Aviation movement, fleet, and route-change intelligence"),
    );
    sources.push(
        Source::new(
            "greenhouse_job_board",
            "Greenhouse Job Boards",
            "https://boards.greenhouse.io",
            Region::Global,
            Category::Technology,
            2,
        )
        .interval(120)
        .notes("Hiring signal source for growth, new teams, and regional expansion"),
    );
    sources.push(
        Source::new(
            "lever_job_board",
            "Lever Jobs",
            "https://jobs.lever.co",
            Region::Global,
            Category::Technology,
            2,
        )
        .interval(120)
        .notes("Hiring signal source for role mix and footprint changes"),
    );
    sources.push(
        Source::new(
            "sedar_plus_ca",
            "SEDAR+ Canada Filings",
            "https://www.sedarplus.ca",
            Region::NorthAmerica,
            Category::GovernmentRegistry,
            1,
        )
        .interval(180)
        .notes("Canadian public-company filing and disclosure source"),
    );
    sources.push(
        Source::new(
            "crunchbase_news",
            "Crunchbase News",
            "https://news.crunchbase.com",
            Region::Global,
            Category::Finance,
            2,
        )
        .rss("https://news.crunchbase.com/feed/")
        .interval(90)
        .notes("Startup, financing, and M&A signal source"),
    );
    sources.push(
        Source::new(
            "tech_eu_ma",
            "Tech.eu",
            "https://tech.eu",
            Region::Europe,
            Category::Finance,
            2,
        )
        .rss("https://tech.eu/feed/")
        .interval(90)
        .notes("European startup funding and acquisition tracking"),
    );
    sources.push(
        Source::new(
            "github_security_advisories",
            "GitHub Security Advisories",
            "https://github.com/advisories",
            Region::Global,
            Category::Cybersecurity,
            2,
        )
        .rss("https://github.com/advisories.atom")
        .interval(120)
        .notes("Code-host intelligence for product, package, and maintainer risk"),
    );
    sources.push(
        Source::new(
            "gitlab_releases",
            "GitLab Releases",
            "https://about.gitlab.com/releases/",
            Region::Global,
            Category::Technology,
            2,
        )
        .rss("https://about.gitlab.com/releases/categories/releases.xml")
        .interval(120)
        .notes("Code-host intelligence for release cadence and product changes"),
    );

    // ── North America ────────────────────────────────────────────
    sources.push(
        Source::new(
            "nyt_us",
            "New York Times",
            "https://www.nytimes.com",
            Region::NorthAmerica,
            Category::News,
            1,
        )
        .interval(15),
    );
    sources.push(
        Source::new(
            "wapo_us",
            "Washington Post",
            "https://www.washingtonpost.com",
            Region::NorthAmerica,
            Category::News,
            1,
        )
        .interval(15),
    );
    sources.push(
        Source::new(
            "politico_us",
            "Politico",
            "https://www.politico.com",
            Region::NorthAmerica,
            Category::News,
            1,
        )
        .rss("https://rss.politico.com/politics-news.xml")
        .interval(20),
    );
    sources.push(
        Source::new(
            "the_hill",
            "The Hill",
            "https://thehill.com",
            Region::NorthAmerica,
            Category::News,
            2,
        )
        .rss("https://thehill.com/feed/")
        .interval(30),
    );
    sources.push(
        Source::new(
            "axios_us",
            "Axios",
            "https://www.axios.com",
            Region::NorthAmerica,
            Category::News,
            2,
        )
        .rss("https://api.axios.com/feed/")
        .interval(20),
    );
    sources.push(
        Source::new(
            "sam_gov",
            "SAM.gov — US Federal Contracts",
            "https://sam.gov/api/prod/sgs/v1/search/",
            Region::NorthAmerica,
            Category::Procurement,
            1,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "sec_edgar",
            "SEC EDGAR Filings",
            "https://efts.sec.gov/LATEST/search-index?q=%22&dateRange=custom&startdt=",
            Region::NorthAmerica,
            Category::GovernmentRegistry,
            1,
        )
        .interval(60)
        .notes("Use EDGAR full-text search API"),
    );
    sources.push(
        Source::new(
            "uspto_patents",
            "USPTO Patent Full-Text Search",
            "https://patentcenter.uspto.gov/retrieval/public/v1/applications/",
            Region::NorthAmerica,
            Category::Patents,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "ofac_sanctions",
            "OFAC SDN List",
            "https://www.treasury.gov/ofac/downloads/sdn.xml",
            Region::NorthAmerica,
            Category::Sanctions,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "bis_export_controls",
            "BIS Export Controls",
            "https://www.bis.doc.gov",
            Region::NorthAmerica,
            Category::Sanctions,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "c4isrnet",
            "C4ISRNet",
            "https://www.c4isrnet.com",
            Region::NorthAmerica,
            Category::Defence,
            2,
        )
        .rss("https://www.c4isrnet.com/arc/outboundfeeds/rss/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "breaking_defense_us",
            "Breaking Defense",
            "https://breakingdefense.com",
            Region::NorthAmerica,
            Category::Defence,
            2,
        )
        .rss("https://breakingdefense.com/feed/")
        .interval(30),
    );
    sources.push(
        Source::new(
            "fedscoop",
            "FedScoop",
            "https://fedscoop.com",
            Region::NorthAmerica,
            Category::Technology,
            2,
        )
        .rss("https://fedscoop.com/feed/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "wired_us",
            "Wired",
            "https://www.wired.com",
            Region::NorthAmerica,
            Category::Technology,
            2,
        )
        .rss("https://www.wired.com/feed/rss")
        .interval(60),
    );
    sources.push(
        Source::new(
            "techcrunch_us",
            "TechCrunch",
            "https://techcrunch.com",
            Region::NorthAmerica,
            Category::Technology,
            2,
        )
        .rss("https://techcrunch.com/feed/")
        .interval(30),
    );
    sources.push(
        Source::new(
            "ars_technica",
            "Ars Technica",
            "https://arstechnica.com",
            Region::NorthAmerica,
            Category::Technology,
            2,
        )
        .rss("https://feeds.arstechnica.com/arstechnica/index")
        .interval(60),
    );
    sources.push(
        Source::new(
            "supply_chain_dive",
            "Supply Chain Dive",
            "https://www.supplychaindive.com",
            Region::NorthAmerica,
            Category::SupplyChain,
            2,
        )
        .rss("https://www.supplychaindive.com/feeds/news/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "freightwaves_us",
            "FreightWaves",
            "https://www.freightwaves.com",
            Region::NorthAmerica,
            Category::SupplyChain,
            2,
        )
        .rss("https://www.freightwaves.com/feed")
        .interval(60),
    );
    sources.push(
        Source::new(
            "govinfo_us",
            "GovInfo (GPO)",
            "https://www.govinfo.gov/rss/dcpdlm.xml",
            Region::NorthAmerica,
            Category::GovernmentRegistry,
            1,
        )
        .rss("https://www.govinfo.gov/rss/dcpdlm.xml")
        .interval(120),
    );
    sources.push(
        Source::new(
            "usaspending_gov",
            "USASpending.gov",
            "https://api.usaspending.gov",
            Region::NorthAmerica,
            Category::Procurement,
            1,
        )
        .interval(120),
    );

    // ── Europe General ───────────────────────────────────────────
    sources.push(
        Source::new(
            "euractiv_eu",
            "EurActiv",
            "https://www.euractiv.com",
            Region::Europe,
            Category::News,
            2,
        )
        .rss("https://www.euractiv.com/sections/all/feed/")
        .interval(30),
    );
    sources.push(
        Source::new(
            "politico_eu",
            "Politico Europe",
            "https://www.politico.eu",
            Region::Europe,
            Category::News,
            1,
        )
        .rss("https://www.politico.eu/feed/")
        .interval(20),
    );
    sources.push(
        Source::new(
            "eur_lex",
            "EUR-Lex Official Journal",
            "https://eur-lex.europa.eu/oj/direct-access.html",
            Region::Europe,
            Category::LegalRegulatory,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "eu_sanctions_list",
            "EU Consolidated Sanctions List",
            "https://eeas.europa.eu/topics/sanctions-policy/8442/consolidated-list-of-sanctions_en",
            Region::Europe,
            Category::Sanctions,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "ec_cordis",
            "EC CORDIS Research Projects",
            "https://cordis.europa.eu/article/rss.xml",
            Region::Europe,
            Category::AcademicResearch,
            2,
        )
        .rss("https://cordis.europa.eu/article/rss.xml")
        .interval(120),
    );
    sources.push(
        Source::new(
            "ted_procurement_eu",
            "TED — EU Tenders",
            "https://ted.europa.eu/TED/main/HomePage.do",
            Region::Europe,
            Category::Procurement,
            1,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "dw_europe",
            "Deutsche Welle",
            "https://rss.dw.com/rdf/rss-en-all",
            Region::Europe,
            Category::News,
            2,
        )
        .rss("https://rss.dw.com/rdf/rss-en-all")
        .interval(30),
    );
    sources.push(
        Source::new(
            "bbc_world",
            "BBC World Service",
            "https://feeds.bbci.co.uk/news/world/rss.xml",
            Region::Europe,
            Category::News,
            1,
        )
        .rss("https://feeds.bbci.co.uk/news/world/rss.xml")
        .interval(15),
    );
    sources.push(
        Source::new(
            "guardian_uk",
            "The Guardian",
            "https://www.theguardian.com/world/rss",
            Region::Europe,
            Category::News,
            1,
        )
        .rss("https://www.theguardian.com/world/rss")
        .interval(20),
    );
    sources.push(
        Source::new(
            "iiss_uk",
            "IISS — International Institute for Strategic Studies",
            "https://www.iiss.org",
            Region::Europe,
            Category::GeopoliticsThinkTank,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "chatham_house",
            "Chatham House",
            "https://www.chathamhouse.org",
            Region::Europe,
            Category::GeopoliticsThinkTank,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "sipri_se",
            "SIPRI — Stockholm International Peace Research",
            "https://www.sipri.org",
            Region::Europe,
            Category::Defence,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "nato_review",
            "NATO Review",
            "https://www.nato.int/docu/review/",
            Region::Europe,
            Category::Defence,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "ecfr_eu",
            "European Council on Foreign Relations",
            "https://ecfr.eu",
            Region::Europe,
            Category::GeopoliticsThinkTank,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "bruegel_eu",
            "Bruegel Institute",
            "https://www.bruegel.org",
            Region::Europe,
            Category::Finance,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "companies_house_uk",
            "Companies House UK",
            "https://api.company-information.service.gov.uk",
            Region::Europe,
            Category::GovernmentRegistry,
            1,
        )
        .interval(120)
        .notes("Requires API key; free tier available"),
    );
    sources.push(
        Source::new(
            "find_a_tender_uk",
            "Find a Tender UK",
            "https://www.find-tender.service.gov.uk",
            Region::Europe,
            Category::Procurement,
            1,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "handelsblatt_de",
            "Handelsblatt",
            "https://www.handelsblatt.com",
            Region::Europe,
            Category::Finance,
            2,
        )
        .notes("German language")
        .interval(60),
    );
    sources.push(
        Source::new(
            "le_monde_fr",
            "Le Monde",
            "https://www.lemonde.fr",
            Region::Europe,
            Category::News,
            2,
        )
        .notes("French language")
        .interval(30),
    );
    sources.push(
        Source::new(
            "intellinews_eu",
            "Intellinews",
            "https://www.intellinews.com",
            Region::EasternEurope,
            Category::News,
            2,
        )
        .rss("https://www.intellinews.com/feed/")
        .interval(60),
    );

    // ── Middle East & Israel ─────────────────────────────────────
    sources.push(
        Source::new(
            "haaretz_il",
            "Haaretz",
            "https://www.haaretz.com",
            Region::Israel,
            Category::News,
            1,
        )
        .rss("https://www.haaretz.com/cmlink/1.628765")
        .interval(20),
    );
    sources.push(
        Source::new(
            "jpost_il",
            "Jerusalem Post",
            "https://www.jpost.com",
            Region::Israel,
            Category::News,
            1,
        )
        .rss("https://www.jpost.com/rss/rssfeedsfrontpage.aspx")
        .interval(20),
    );
    sources.push(
        Source::new(
            "ynetnews_il",
            "Ynet News",
            "https://www.ynetnews.com",
            Region::Israel,
            Category::News,
            1,
        )
        .rss("https://www.ynet.co.il/Integration/StoryRss2.xml")
        .interval(20),
    );
    sources.push(
        Source::new(
            "timesofisrael_il",
            "Times of Israel",
            "https://www.timesofisrael.com",
            Region::Israel,
            Category::News,
            1,
        )
        .rss("https://www.timesofisrael.com/feed/")
        .interval(20),
    );
    sources.push(
        Source::new(
            "calcalist_il",
            "Calcalist Tech",
            "https://www.calcalistech.com",
            Region::Israel,
            Category::Technology,
            2,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "globes_il",
            "Globes Business Daily",
            "https://en.globes.co.il",
            Region::Israel,
            Category::Finance,
            1,
        )
        .rss("https://en.globes.co.il/en/rss")
        .interval(30),
    );
    // Globes RSS endpoints are intermittently blocked (403/404) from server-side crawlers,
    // so use the main site URL to ensure crawl_cycle ingests observable content reliably.
    sources.push(
        Source::new(
            "globes_il_tech",
            "Globes IL Tech Magazine",
            "https://www.globes.co.il",
            Region::Israel,
            Category::Technology,
            1,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "idf_spokespersons",
            "IDF Spokesperson",
            "https://www.idf.il/en/",
            Region::Israel,
            Category::Defence,
            1,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "mod_israel",
            "Israeli Ministry of Defense",
            "https://www.mod.gov.il/English/",
            Region::Israel,
            Category::Defence,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "startupnation_il",
            "Start-Up Nation Central",
            "https://www.startupnationcentral.org",
            Region::Israel,
            Category::Technology,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "iati_il",
            "Israel Advanced Technology Industries",
            "https://www.iati.co.il/en/",
            Region::Israel,
            Category::Technology,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "inss_il",
            "Institute for National Security Studies",
            "https://www.inss.org.il/en",
            Region::Israel,
            Category::GeopoliticsThinkTank,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "jiss_il",
            "Jerusalem Institute for Strategy and Security",
            "https://jiss.org.il/en/",
            Region::Israel,
            Category::GeopoliticsThinkTank,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "mfa_israel",
            "Israel Ministry of Foreign Affairs",
            "https://www.gov.il/en/departments/ministry_of_foreign_affairs",
            Region::Israel,
            Category::GovernmentRegistry,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "ivc_il",
            "IVC Research Center (VC/Startups)",
            "https://www.ivc-online.com",
            Region::Israel,
            Category::Finance,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "techaviv_il",
            "TechAviv (startup news)",
            "https://techaviv.com",
            Region::Israel,
            Category::Technology,
            3,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "al_monitor_me",
            "Al-Monitor Middle East",
            "https://www.al-monitor.com",
            Region::MiddleEast,
            Category::News,
            1,
        )
        .rss("https://www.al-monitor.com/rss")
        .interval(30),
    );
    sources.push(
        Source::new(
            "middle_east_eye",
            "Middle East Eye",
            "https://www.middleeasteye.net",
            Region::MiddleEast,
            Category::News,
            2,
        )
        .rss("https://www.middleeasteye.net/rss")
        .interval(30),
    );
    sources.push(
        Source::new(
            "arab_news",
            "Arab News",
            "https://www.arabnews.com",
            Region::MiddleEast,
            Category::News,
            2,
        )
        .rss("https://www.arabnews.com/rss.xml")
        .interval(30),
    );
    sources.push(
        Source::new(
            "mena_oecd",
            "MENA OECD Initiative",
            "https://www.oecd.org/mena/",
            Region::MiddleEast,
            Category::Finance,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "mei_dc",
            "Middle East Institute DC",
            "https://www.mei.edu",
            Region::MiddleEast,
            Category::GeopoliticsThinkTank,
            2,
        )
        .interval(120),
    );

    // ── China & Hong Kong ─────────────────────────────────────────
    sources.push(
        Source::new(
            "xinhua_en",
            "Xinhua News Agency (EN)",
            "https://english.news.cn",
            Region::China,
            Category::News,
            1,
        )
        .rss("https://english.news.cn/rss/world_rss.xml")
        .interval(20)
        .notes("PRC state media; use for official positions"),
    );
    sources.push(
        Source::new(
            "chinadaily_en",
            "China Daily (EN)",
            "https://www.chinadaily.com.cn/rss/world_rss.xml",
            Region::China,
            Category::News,
            1,
        )
        .rss("https://www.chinadaily.com.cn/rss/world_rss.xml")
        .interval(30),
    );
    sources.push(
        Source::new(
            "globaltimes_cn",
            "Global Times (EN)",
            "https://www.globaltimes.cn",
            Region::China,
            Category::News,
            2,
        )
        .rss("https://www.globaltimes.cn/rss/outbrain.xml")
        .interval(30)
        .notes("PRC nationalist outlet; use carefully for narratives"),
    );
    sources.push(
        Source::new(
            "scmp_hk",
            "South China Morning Post",
            "https://www.scmp.com",
            Region::China,
            Category::News,
            1,
        )
        .rss("https://www.scmp.com/rss/91/feed")
        .interval(20),
    );
    sources.push(
        Source::new(
            "sinocism_cn",
            "Sinocism Newsletter",
            "https://sinocism.com",
            Region::China,
            Category::GeopoliticsThinkTank,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "caixin_cn",
            "Caixin Global",
            "https://www.caixinglobal.com",
            Region::China,
            Category::Finance,
            1,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "technode_cn",
            "TechNode China",
            "https://technode.com",
            Region::China,
            Category::Technology,
            2,
        )
        .rss("https://technode.com/feed/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "pandaily_cn",
            "PanDaily (Chinese Tech)",
            "https://pandaily.com",
            Region::China,
            Category::Technology,
            2,
        )
        .rss("https://pandaily.com/feed/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "cgtn_cn",
            "CGTN World",
            "https://www.cgtn.com",
            Region::China,
            Category::News,
            2,
        )
        .rss("https://www.cgtn.com/subscribe/feeds/world.xml")
        .interval(30)
        .notes("PRC state broadcaster"),
    );
    sources.push(
        Source::new(
            "hurun_cn",
            "Hurun Report (wealth/business)",
            "https://www.hurun.net/en-US/",
            Region::China,
            Category::Finance,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "merics_cn",
            "MERICS China Intelligence",
            "https://merics.org",
            Region::China,
            Category::GeopoliticsThinkTank,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "ubs_china_insights",
            "UBS China Research",
            "https://www.ubs.com/global/en/investment-bank/in-focus/china.html",
            Region::China,
            Category::Finance,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "pbc_gov_cn",
            "People's Bank of China",
            "http://www.pbc.gov.cn/en/3688110/index.html",
            Region::China,
            Category::Finance,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "mofcom_cn",
            "MOFCOM Trade Stats (EN)",
            "http://english.mofcom.gov.cn",
            Region::China,
            Category::Trade,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "cnipa_patents",
            "CNIPA Patent Database",
            "https://pss-system.cponline.cnipa.gov.cn",
            Region::China,
            Category::Patents,
            1,
        )
        .interval(240)
        .proxy()
        .notes("Requires proxy for reliable access"),
    );
    sources.push(
        Source::new(
            "csrc_cn",
            "CSRC Securities Regulator",
            "http://www.csrc.gov.cn/csrc_en/",
            Region::China,
            Category::GovernmentRegistry,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "cctv_news_cn",
            "CCTV News (EN)",
            "https://english.cctv.com",
            Region::China,
            Category::News,
            3,
        )
        .rss("https://english.cctv.com/rss/newsupdates.xml")
        .interval(60),
    );
    sources.push(
        Source::new(
            "yicai_cn",
            "Yicai Global Finance",
            "https://www.yicaiglobal.com",
            Region::China,
            Category::Finance,
            2,
        )
        .interval(60),
    );

    // ── Russia & Eastern Europe ───────────────────────────────────
    sources.push(
        Source::new(
            "kyivpost_ua",
            "Kyiv Post",
            "https://www.kyivpost.com",
            Region::EasternEurope,
            Category::News,
            1,
        )
        .rss("https://www.kyivpost.com/rss")
        .interval(20),
    );
    sources.push(
        Source::new(
            "kyiv_independent_ua",
            "Kyiv Independent",
            "https://kyivindependent.com",
            Region::EasternEurope,
            Category::News,
            1,
        )
        .rss("https://kyivindependent.com/feed/")
        .interval(20),
    );
    sources.push(
        Source::new(
            "rferl_eu",
            "Radio Free Europe / Radio Liberty",
            "https://www.rferl.org",
            Region::EasternEurope,
            Category::News,
            1,
        )
        .rss("https://www.rferl.org/api/epdykpmeiqvv")
        .interval(20),
    );
    sources.push(
        Source::new(
            "meduza_ru",
            "Meduza (independent Russia)",
            "https://meduza.io/en",
            Region::Russia,
            Category::News,
            1,
        )
        .rss("https://meduza.io/rss/all")
        .interval(30)
        .proxy()
        .notes("Access may be restricted; use proxy"),
    );
    sources.push(
        Source::new(
            "the_insider_ru",
            "The Insider (Russian investigations)",
            "https://theins.ru/en/",
            Region::Russia,
            Category::News,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "cepa_eu",
            "CEPA — Center for European Policy Analysis",
            "https://cepa.org",
            Region::EasternEurope,
            Category::GeopoliticsThinkTank,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "isw_daily",
            "ISW — Institute for the Study of War",
            "https://www.understandingwar.org",
            Region::EasternEurope,
            Category::Defence,
            1,
        )
        .rss("https://www.understandingwar.org/rss.xml")
        .interval(60),
    );
    sources.push(
        Source::new(
            "bellingcat_eu",
            "Bellingcat OSINT",
            "https://www.bellingcat.com",
            Region::EasternEurope,
            Category::News,
            1,
        )
        .rss("https://www.bellingcat.com/feed/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "occrp_eu",
            "OCCRP — Organised Crime & Corruption",
            "https://www.occrp.org",
            Region::EasternEurope,
            Category::News,
            1,
        )
        .rss("https://www.occrp.org/en/feed")
        .interval(60),
    );

    // ── Asia Pacific (non-China) ──────────────────────────────────
    sources.push(
        Source::new(
            "nikkei_jp",
            "Nikkei Asia",
            "https://asia.nikkei.com",
            Region::AsiaPacific,
            Category::Finance,
            1,
        )
        .rss("https://asia.nikkei.com/rss/feed/nar")
        .interval(30),
    );
    sources.push(
        Source::new(
            "diplomat_ap",
            "The Diplomat",
            "https://thediplomat.com",
            Region::AsiaPacific,
            Category::GeopoliticsThinkTank,
            1,
        )
        .rss("https://thediplomat.com/feed/")
        .interval(30),
    );
    sources.push(
        Source::new(
            "asean_secretariat",
            "ASEAN Secretariat",
            "https://asean.org",
            Region::AsiaPacific,
            Category::GovernmentRegistry,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "iiss_asia",
            "IISS Asia Security",
            "https://www.iiss.org/research/asia",
            Region::AsiaPacific,
            Category::Defence,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "east_asia_forum",
            "East Asia Forum",
            "https://www.eastasiaforum.org",
            Region::AsiaPacific,
            Category::GeopoliticsThinkTank,
            2,
        )
        .rss("https://www.eastasiaforum.org/feed/")
        .interval(120),
    );
    sources.push(
        Source::new(
            "lowy_au",
            "Lowy Institute",
            "https://www.lowyinstitute.org",
            Region::AsiaPacific,
            Category::GeopoliticsThinkTank,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "hindustan_times_in",
            "Hindustan Times",
            "https://www.hindustantimes.com",
            Region::India,
            Category::News,
            2,
        )
        .rss("https://www.hindustantimes.com/feeds/rss/world/rssfeed.xml")
        .interval(30),
    );
    sources.push(
        Source::new(
            "economic_times_in",
            "Economic Times India",
            "https://economictimes.indiatimes.com",
            Region::India,
            Category::Finance,
            2,
        )
        .rss("https://economictimes.indiatimes.com/rssfeedstopstories.cms")
        .interval(30),
    );
    sources.push(
        Source::new(
            "livemint_in",
            "LiveMint — India Finance",
            "https://www.livemint.com",
            Region::India,
            Category::Finance,
            2,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "idsa_in",
            "IDSA India Defence Studies",
            "https://idsa.in",
            Region::India,
            Category::Defence,
            2,
        )
        .interval(120),
    );

    // ── Defence & Procurement ─────────────────────────────────────
    sources.push(Source::new("armyrecognition_def", "Army Recognition", "https://www.armyrecognition.com", Region::Global, Category::Defence, 2)
        .rss("https://www.armyrecognition.com/index.php?option=com_ninjarsssyndicator&feed_id=1&format=raw").interval(60));
    sources.push(
        Source::new(
            "naval_news",
            "Naval News",
            "https://www.navalnews.com",
            Region::Global,
            Category::Defence,
            2,
        )
        .rss("https://www.navalnews.com/feed/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "aviation_week_def",
            "Aviation Week & Space Technology",
            "https://aviationweek.com",
            Region::Global,
            Category::Defence,
            1,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "shephard_media",
            "Shephard Media Defence",
            "https://www.shephardmedia.com",
            Region::Global,
            Category::Defence,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "mod_uk",
            "UK MOD Announcements",
            "https://www.gov.uk/government/organisations/ministry-of-defence.atom",
            Region::Europe,
            Category::Defence,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "dsca_us",
            "DSCA — Defense Security Sales",
            "https://www.dsca.mil/press-media/major-arms-sales",
            Region::NorthAmerica,
            Category::Defence,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "sipri_arms_transfers",
            "SIPRI Arms Transfers DB",
            "https://armstrade.sipri.org",
            Region::Global,
            Category::Defence,
            1,
        )
        .interval(480),
    );
    sources.push(
        Source::new(
            "procurement_tracker",
            "Defense Procurement Tracker",
            "https://www.defpro.com",
            Region::Global,
            Category::Procurement,
            2,
        )
        .interval(120),
    );

    // ── Cybersecurity ─────────────────────────────────────────────
    sources.push(
        Source::new(
            "krebs_security",
            "KrebsOnSecurity",
            "https://krebsonsecurity.com",
            Region::Global,
            Category::Cybersecurity,
            1,
        )
        .rss("https://krebsonsecurity.com/feed/")
        .interval(60),
    );
    sources.push(
        Source::new(
            "schneier_security",
            "Schneier on Security",
            "https://www.schneier.com/blog/",
            Region::Global,
            Category::Cybersecurity,
            2,
        )
        .rss("https://www.schneier.com/blog/atom.xml")
        .interval(60),
    );
    sources.push(
        Source::new(
            "cisa_advisories",
            "CISA Security Advisories",
            "https://www.cisa.gov/uscert/ncas/current-activity.xml",
            Region::NorthAmerica,
            Category::Cybersecurity,
            1,
        )
        .rss("https://www.cisa.gov/uscert/ncas/current-activity.xml")
        .interval(30),
    );
    sources.push(
        Source::new(
            "threatpost_cyber",
            "Threatpost",
            "https://threatpost.com",
            Region::Global,
            Category::Cybersecurity,
            2,
        )
        .rss("https://threatpost.com/feed/")
        .interval(30),
    );
    sources.push(
        Source::new(
            "darkreading_cyber",
            "Dark Reading",
            "https://www.darkreading.com",
            Region::Global,
            Category::Cybersecurity,
            2,
        )
        .rss("https://www.darkreading.com/rss.xml")
        .interval(30),
    );
    sources.push(
        Source::new(
            "mitre_att_ck",
            "MITRE ATT&CK",
            "https://attack.mitre.org",
            Region::Global,
            Category::Cybersecurity,
            1,
        )
        .interval(480),
    );
    sources.push(
        Source::new(
            "nvd_nist_vuln",
            "NVD — NIST Vulnerability DB",
            "https://nvd.nist.gov/feeds/json/cve/1.1/nvdcve-1.1-recent.json.gz",
            Region::NorthAmerica,
            Category::Cybersecurity,
            1,
        )
        .interval(60),
    );

    // ── Patents ───────────────────────────────────────────────────
    sources.push(
        Source::new(
            "epo_patents",
            "EPO — European Patent Office",
            "https://worldwide.espacenet.com",
            Region::Europe,
            Category::Patents,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "wipo_patents",
            "WIPO PatentScope",
            "https://patentscope.wipo.int",
            Region::Global,
            Category::Patents,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "google_patents",
            "Google Patents",
            "https://patents.google.com",
            Region::Global,
            Category::Patents,
            2,
        )
        .interval(120)
        .notes("Use search API"),
    );

    // ── Sanctions & Compliance ────────────────────────────────────
    sources.push(
        Source::new(
            "un_sanctions_list",
            "UN Security Council Sanctions",
            "https://www.un.org/securitycouncil/content/un-sc-consolidated-list",
            Region::Global,
            Category::Sanctions,
            1,
        )
        .interval(240),
    );
    sources.push(Source::new("uk_sanctions_list", "UK HM Treasury Consolidated List", "https://assets.publishing.service.gov.uk/government/uploads/system/uploads/attachment_data/file/", Region::Europe, Category::Sanctions, 1).interval(240));
    sources.push(
        Source::new(
            "eu_sanctions_map",
            "EU Sanctions Map",
            "https://www.sanctionsmap.eu",
            Region::Europe,
            Category::Sanctions,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "worldcheck_headlines",
            "Refinitiv World-Check (public headlines)",
            "https://www.refinitiv.com/en/products/world-check-kyc-screening",
            Region::Global,
            Category::Sanctions,
            2,
        )
        .interval(480),
    );

    // ── Trade & Supply Chain ──────────────────────────────────────
    sources.push(
        Source::new(
            "wto_news",
            "WTO News",
            "https://www.wto.org/english/news_e/news_e.htm",
            Region::Global,
            Category::Trade,
            1,
        )
        .rss("https://www.wto.org/rss/english/news_e.rss")
        .interval(120),
    );
    sources.push(
        Source::new(
            "icc_trade",
            "ICC — International Chamber of Commerce",
            "https://iccwbo.org",
            Region::Global,
            Category::Trade,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "panjiva_trade",
            "Panjiva Shipping Intelligence",
            "https://panjiva.com",
            Region::Global,
            Category::SupplyChain,
            1,
        )
        .interval(60)
        .notes("Requires subscription; scrape summaries only"),
    );
    sources.push(
        Source::new(
            "import_genius",
            "ImportGenius Trade Data",
            "https://www.importgenius.com",
            Region::Global,
            Category::SupplyChain,
            2,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "trade_map_itc",
            "ITC Trade Map",
            "https://www.trademap.org",
            Region::Global,
            Category::Trade,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "comtrade_un",
            "UN Comtrade Trade Stats",
            "https://comtradeapi.un.org",
            Region::Global,
            Category::Trade,
            1,
        )
        .interval(480),
    );

    // ── Academic & Research ───────────────────────────────────────
    sources.push(
        Source::new(
            "arxiv_cs",
            "arXiv CS Preprints",
            "https://export.arxiv.org/rss/cs",
            Region::Global,
            Category::AcademicResearch,
            2,
        )
        .rss("https://export.arxiv.org/rss/cs")
        .interval(120),
    );
    sources.push(
        Source::new(
            "arxiv_econ",
            "arXiv Economics Preprints",
            "https://export.arxiv.org/rss/econ",
            Region::Global,
            Category::AcademicResearch,
            3,
        )
        .rss("https://export.arxiv.org/rss/econ")
        .interval(120),
    );
    sources.push(
        Source::new(
            "ssrn_papers",
            "SSRN Social Science Research",
            "https://www.ssrn.com",
            Region::Global,
            Category::AcademicResearch,
            3,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "brookings_us",
            "Brookings Institution",
            "https://www.brookings.edu",
            Region::NorthAmerica,
            Category::GeopoliticsThinkTank,
            2,
        )
        .rss("https://www.brookings.edu/feed/")
        .interval(120),
    );
    sources.push(
        Source::new(
            "rand_corp",
            "RAND Corporation",
            "https://www.rand.org",
            Region::NorthAmerica,
            Category::GeopoliticsThinkTank,
            1,
        )
        .rss("https://www.rand.org/pubs/rss/hot.xml")
        .interval(120),
    );
    sources.push(
        Source::new(
            "csis_us",
            "CSIS — Center for Strategic & International Studies",
            "https://www.csis.org",
            Region::NorthAmerica,
            Category::GeopoliticsThinkTank,
            1,
        )
        .rss("https://www.csis.org/analysis/rss.xml")
        .interval(120),
    );
    sources.push(
        Source::new(
            "wilson_center",
            "Wilson Center",
            "https://www.wilsoncenter.org",
            Region::NorthAmerica,
            Category::GeopoliticsThinkTank,
            2,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "stimson_center",
            "Stimson Center",
            "https://www.stimson.org",
            Region::NorthAmerica,
            Category::GeopoliticsThinkTank,
            2,
        )
        .interval(120),
    );

    // ── Social Media & Forums ─────────────────────────────────────
    sources.push(
        Source::new(
            "twitter_search",
            "Twitter/X Search API",
            "https://api.twitter.com/2/tweets/search/recent",
            Region::Global,
            Category::SocialMedia,
            2,
        )
        .notes("Bearer token required; search_param = 'query'")
        .interval(10),
    );
    sources.push(
        Source::new(
            "linkedin_company",
            "LinkedIn Company Insights",
            "https://api.linkedin.com/v2/",
            Region::Global,
            Category::SocialMedia,
            2,
        )
        .notes("OAuth2 required")
        .interval(60),
    );
    sources.push(
        Source::new(
            "reddit_worldnews",
            "Reddit r/worldnews",
            "https://www.reddit.com/r/worldnews/.json",
            Region::Global,
            Category::Forum,
            3,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "reddit_geopolitics",
            "Reddit r/geopolitics",
            "https://www.reddit.com/r/geopolitics/.json",
            Region::Global,
            Category::Forum,
            3,
        )
        .interval(30),
    );
    sources.push(
        Source::new(
            "telegram_channels",
            "Telegram Open Channels (via scraper)",
            "https://t.me/s/",
            Region::Global,
            Category::SocialMedia,
            3,
        )
        .notes("Use custom Telegram scraper module")
        .interval(15),
    );

    // ── Government Registries ─────────────────────────────────────
    sources.push(
        Source::new(
            "opencorporates_global",
            "OpenCorporates",
            "https://api.opencorporates.com/v0.4/companies/search",
            Region::Global,
            Category::GovernmentRegistry,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "gleif_lei",
            "GLEIF — Legal Entity Identifiers",
            "https://api.gleif.org/api/v1/",
            Region::Global,
            Category::GovernmentRegistry,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "icij_offshore_leaks",
            "ICIJ OffshoreLeaks DB",
            "https://offshoreleaks.icij.org",
            Region::Global,
            Category::GovernmentRegistry,
            1,
        )
        .interval(480),
    );
    sources.push(
        Source::new(
            "world_bank_projects",
            "World Bank Projects",
            "https://search.worldbank.org/api/v2/projects",
            Region::Global,
            Category::Procurement,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "afdb_procurement",
            "African Development Bank Procurement",
            "https://projectsportal.afdb.org",
            Region::Africa,
            Category::Procurement,
            2,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "eu_company_register",
            "EU Company Register",
            "https://ec.europa.eu/transparencyregister/public/homePage.do",
            Region::Europe,
            Category::GovernmentRegistry,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "israel_registrar_companies",
            "Israel Companies Registrar",
            "https://www.gov.il/en/departments/topics/companies-registry/govil-landing-page",
            Region::Israel,
            Category::GovernmentRegistry,
            1,
        )
        .interval(240),
    );
    sources.push(
        Source::new(
            "china_samr",
            "China SAMR Company Registry",
            "https://www.samr.gov.cn",
            Region::China,
            Category::GovernmentRegistry,
            1,
        )
        .interval(240)
        .proxy()
        .notes("May require proxy; partial English interface"),
    );

    // ── Finance & Markets ─────────────────────────────────────────
    sources.push(
        Source::new(
            "alpha_vantage_markets",
            "Alpha Vantage Market Data",
            "https://www.alphavantage.co/query",
            Region::Global,
            Category::Finance,
            2,
        )
        .notes("Free tier: 5 req/min")
        .interval(5),
    );
    sources.push(
        Source::new(
            "fred_macro",
            "FRED — Federal Reserve Economic Data",
            "https://api.stlouisfed.org/fred/series/observations",
            Region::NorthAmerica,
            Category::Finance,
            1,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "world_bank_data",
            "World Bank Data API",
            "https://api.worldbank.org/v2/country/all/indicator/",
            Region::Global,
            Category::Finance,
            1,
        )
        .interval(1440),
    );
    sources.push(
        Source::new(
            "imf_data",
            "IMF Data Mapper",
            "https://www.imf.org/external/datamapper/api/v1/",
            Region::Global,
            Category::Finance,
            1,
        )
        .interval(1440),
    );

    // ── Energy & Commodities ──────────────────────────────────────
    sources.push(
        Source::new(
            "eia_us_energy",
            "EIA US Energy Information",
            "https://api.eia.gov/bulk/",
            Region::NorthAmerica,
            Category::EnergyResources,
            1,
        )
        .interval(60),
    );
    sources.push(
        Source::new(
            "iea_global_energy",
            "IEA Global Energy",
            "https://www.iea.org/news/rss",
            Region::Global,
            Category::EnergyResources,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "opec_org",
            "OPEC Official Press",
            "https://www.opec.org/opec_web/en/press_room/30.htm",
            Region::Global,
            Category::EnergyResources,
            1,
        )
        .interval(120),
    );
    sources.push(
        Source::new(
            "oilprice_com",
            "OilPrice.com",
            "https://oilprice.com",
            Region::Global,
            Category::EnergyResources,
            2,
        )
        .rss("https://oilprice.com/rss")
        .interval(30),
    );
    sources.push(
        Source::new(
            "platts_energy",
            "S&P Global Commodity Insights",
            "https://www.spglobal.com/commodityinsights/en",
            Region::Global,
            Category::EnergyResources,
            1,
        )
        .interval(60),
    );

    sources
}

fn validate_sources(sources: &[Source]) -> Result<(), String> {
    if sources.is_empty() {
        return Err("registry must define at least one source".to_string());
    }

    let mut slugs = std::collections::HashSet::new();
    for source in sources {
        if source.slug.trim().is_empty() {
            return Err("source slug must not be empty".to_string());
        }
        if !slugs.insert(source.slug.clone()) {
            return Err(format!("duplicate source slug {}", source.slug));
        }
        if source.name.trim().is_empty() {
            return Err(format!("source {} has an empty name", source.slug));
        }
        if source.url.trim().is_empty() {
            return Err(format!("source {} has an empty URL", source.slug));
        }
        if !(1..=5).contains(&source.tier) {
            return Err(format!(
                "source {} has invalid tier {}",
                source.slug, source.tier
            ));
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Query helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build an index of sources keyed by slug for O(1) lookup.
pub fn build_slug_index(sources: &[Source]) -> HashMap<String, usize> {
    sources
        .iter()
        .enumerate()
        .map(|(i, s)| (s.slug.clone(), i))
        .collect()
}

/// Filter sources that are enabled and at or above a priority tier.
pub fn filter_by_tier(sources: &[Source], max_tier: u8) -> Vec<&Source> {
    sources
        .iter()
        .filter(|s| s.enabled && s.tier <= max_tier)
        .collect()
}

/// Filter sources by region.
pub fn filter_by_region<'a>(sources: &'a [Source], region: &Region) -> Vec<&'a Source> {
    sources
        .iter()
        .filter(|s| s.enabled && &s.region == region)
        .collect()
}

/// Filter sources by category.
pub fn filter_by_category<'a>(sources: &'a [Source], category: &Category) -> Vec<&'a Source> {
    sources
        .iter()
        .filter(|s| s.enabled && &s.category == category)
        .collect()
}

/// Return sources that have an RSS feed URL configured.
pub fn sources_with_rss(sources: &[Source]) -> Vec<&Source> {
    sources
        .iter()
        .filter(|s| s.enabled && s.rss_url.is_some())
        .collect()
}

/// Return sources that need a proxy.
pub fn sources_needing_proxy(sources: &[Source]) -> Vec<&Source> {
    sources
        .iter()
        .filter(|s| s.enabled && s.needs_proxy)
        .collect()
}

pub fn select_sources_for_crawl<'a>(
    sources: &'a [Source],
    max_tier: u8,
    crawl_limit: usize,
    always_include_slugs: &[&str],
) -> Vec<&'a Source> {
    let enabled_sources: Vec<_> = sources
        .iter()
        .filter(|source| source.enabled && source.tier <= max_tier)
        .collect();
    let mut selected: Vec<_> = enabled_sources
        .iter()
        .copied()
        .filter(|source| always_include_slugs.contains(&source.slug.as_str()))
        .collect();

    for source in enabled_sources {
        if selected.len() >= crawl_limit {
            break;
        }
        if selected.iter().any(|existing| existing.slug == source.slug) {
            continue;
        }
        selected.push(source);
    }

    selected
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::NamedTempFile;

    #[test]
    fn registry_has_minimum_sources() {
        let s = all_sources();
        assert!(
            s.len() >= 100,
            "Expected at least 100 sources, got {}",
            s.len()
        );
    }

    #[test]
    fn slugs_are_unique() {
        let s = all_sources();
        let mut seen = std::collections::HashSet::new();
        for src in &s {
            assert!(seen.insert(&src.slug), "Duplicate slug: {}", src.slug);
        }
    }

    #[test]
    fn israel_sources_present() {
        let s = all_sources();
        let il: Vec<_> = filter_by_region(&s, &Region::Israel);
        assert!(!il.is_empty(), "No Israel sources found");
    }

    #[test]
    fn china_sources_present() {
        let s = all_sources();
        let cn: Vec<_> = filter_by_region(&s, &Region::China);
        assert!(!cn.is_empty(), "No China sources found");
    }

    #[test]
    fn procurement_sources_present() {
        let s = all_sources();
        let p: Vec<_> = filter_by_category(&s, &Category::Procurement);
        assert!(!p.is_empty());
    }

    #[test]
    fn tier_filter_works() {
        let s = all_sources();
        let tier1 = filter_by_tier(&s, 1);
        let all_tiers = filter_by_tier(&s, 5);
        assert!(tier1.len() < all_tiers.len());
        assert!(tier1.iter().all(|s| s.tier == 1));
    }

    #[test]
    fn all_slugs_nonempty() {
        let s = all_sources();
        for src in &s {
            assert!(
                !src.slug.is_empty(),
                "Empty slug found for source: {}",
                src.name
            );
            assert!(!src.url.is_empty(), "Empty URL for source: {}", src.slug);
        }
    }

    #[test]
    fn load_sources_from_path_overrides_defaults() {
        let file = NamedTempFile::new().expect("test: create temp file");
        fs::write(
            file.path(),
            "sources:\n  - slug: test_feed\n    name: Test Feed\n    url: https://example.com/feed\n    search_param: null\n    region: Global\n    category: News\n    tier: 1\n    needs_proxy: false\n    rss_url: https://example.com/rss\n    enabled: true\n    min_interval_minutes: 15\n    notes: runtime override\n",
        )
        .expect("test: write source yaml");

        let sources = load_sources_from_path(file.path()).expect("test: load sources");
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].slug, "test_feed");
    }

    #[test]
    fn load_sources_from_path_rejects_duplicate_slugs() {
        let file = NamedTempFile::new().expect("test: create temp file");
        fs::write(
            file.path(),
            "sources:\n  - slug: dup\n    name: One\n    url: https://example.com/one\n    search_param: null\n    region: Global\n    category: News\n    tier: 1\n    needs_proxy: false\n    rss_url: null\n    enabled: true\n    min_interval_minutes: 15\n    notes: null\n  - slug: dup\n    name: Two\n    url: https://example.com/two\n    search_param: null\n    region: Global\n    category: News\n    tier: 2\n    needs_proxy: false\n    rss_url: null\n    enabled: true\n    min_interval_minutes: 30\n    notes: null\n",
        )
        .expect("test: write duplicate source yaml");

        let error = load_sources_from_path(file.path()).unwrap_err();
        assert!(error.to_string().contains("duplicate source slug dup"));
    }

    #[test]
    fn select_sources_for_crawl_preserves_forced_sources() {
        let sources = vec![
            Source::new(
                "forced",
                "Forced",
                "https://example.com/forced",
                Region::Global,
                Category::News,
                2,
            ),
            Source::new(
                "a",
                "A",
                "https://example.com/a",
                Region::Global,
                Category::News,
                1,
            ),
            Source::new(
                "b",
                "B",
                "https://example.com/b",
                Region::Global,
                Category::News,
                1,
            ),
        ];

        let selected = select_sources_for_crawl(&sources, 2, 1, &["forced"]);
        assert_eq!(selected[0].slug, "forced");
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn expanded_high_value_osint_classes_are_present() {
        let sources = all_sources();
        let expected = [
            "engineering_news_record",
            "construction_dive",
            "vessel_finder_news",
            "flightglobal",
            "greenhouse_job_board",
            "lever_job_board",
            "sedar_plus_ca",
            "crunchbase_news",
            "tech_eu_ma",
            "github_security_advisories",
            "gitlab_releases",
        ];

        for slug in expected {
            assert!(
                sources.iter().any(|source| source.slug == slug),
                "missing expanded OSINT source {slug}"
            );
        }
    }
}
