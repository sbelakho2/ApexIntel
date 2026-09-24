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

use apex_store::postgres::{PgStore, SourceRuntimeStateRow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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

impl Region {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::NorthAmerica => "north_america",
            Self::Europe => "europe",
            Self::MiddleEast => "middle_east",
            Self::Israel => "israel",
            Self::China => "china",
            Self::AsiaPacific => "asia_pacific",
            Self::LatinAmerica => "latin_america",
            Self::Africa => "africa",
            Self::EasternEurope => "eastern_europe",
            Self::Russia => "russia",
            Self::India => "india",
        }
    }
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

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::News => "news",
            Self::Defence => "defence",
            Self::Finance => "finance",
            Self::Trade => "trade",
            Self::Technology => "technology",
            Self::Patents => "patents",
            Self::Sanctions => "sanctions",
            Self::GovernmentRegistry => "government_registry",
            Self::Procurement => "procurement",
            Self::AcademicResearch => "academic_research",
            Self::SocialMedia => "social_media",
            Self::Forum => "forum",
            Self::GeopoliticsThinkTank => "geopolitics_think_tank",
            Self::SupplyChain => "supply_chain",
            Self::Cybersecurity => "cybersecurity",
            Self::EnergyResources => "energy_resources",
            Self::HealthcareLife => "healthcare_life",
            Self::LegalRegulatory => "legal_regulatory",
        }
    }
}

/// Access mechanism an API-backed source requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiAdapter {
    /// Stable adapter identifier, e.g. `sec_edgar` or `openalex`.
    pub id: String,
    /// Whether the adapter needs credentials before it can be called.
    pub requires_credentials: bool,
}

/// Explicit fetch strategy for a registered source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchStrategy {
    Rss,
    Html,
    JsonApi(ApiAdapter),
    Browser,
    Sitemap,
    Search,
}

/// Operational capability of a source. Only [`SourceCapability::Operational`]
/// sources are schedulable and counted in the admin source-coverage metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceCapability {
    #[default]
    Operational,
    RequiresCredentials,
    Blocked,
    TemporarilyFailed,
    Unsupported,
}

impl SourceCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::RequiresCredentials => "requires_credentials",
            Self::Blocked => "blocked",
            Self::TemporarilyFailed => "temporarily_failed",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn is_operational(self) -> bool {
        matches!(self, Self::Operational)
    }
}

/// Resolve the effective capability of a source by combining its declared
/// capability with live runtime state: a source whose circuit breaker is
/// currently open is `temporarily_failed` regardless of its declared state.
pub fn effective_capability(
    source: &Source,
    runtime: Option<&SourceRuntimeStateRow>,
    now: DateTime<Utc>,
) -> SourceCapability {
    if !source.capability.is_operational() {
        return source.capability;
    }
    if let Some(row) = runtime {
        if row
            .circuit_open_until
            .map(|until| until > now)
            .unwrap_or(false)
        {
            return SourceCapability::TemporarilyFailed;
        }
    }
    SourceCapability::Operational
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
    /// Explicit fetch strategy; inferred from URL shape/category when absent.
    #[serde(default)]
    pub fetch_strategy: Option<FetchStrategy>,
    /// Declared capability of the source. Sources that are not operational
    /// are excluded from scheduling and from the operational source count.
    #[serde(default)]
    pub capability: SourceCapability,
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
            fetch_strategy: None,
            capability: SourceCapability::Operational,
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

    fn capability(mut self, capability: SourceCapability) -> Self {
        self.capability = capability;
        self
    }

    /// Explicit strategy when configured, otherwise inferred from the URL
    /// shape and content category.
    pub fn strategy(&self) -> FetchStrategy {
        if let Some(explicit) = &self.fetch_strategy {
            return explicit.clone();
        }
        if self.rss_url.is_some() {
            return FetchStrategy::Rss;
        }
        if self.search_param.is_some() {
            return FetchStrategy::Search;
        }
        match self.category {
            Category::SocialMedia | Category::Forum => FetchStrategy::Browser,
            _ => FetchStrategy::Html,
        }
    }

    /// Host of the primary endpoint (RSS preferred), used for per-domain rate
    /// limiting.
    pub fn domain(&self) -> Option<String> {
        let endpoint = self.rss_url.as_deref().unwrap_or(self.url.as_str());
        url::Url::parse(endpoint)
            .ok()
            .and_then(|parsed| parsed.host_str().map(ToOwned::to_owned))
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
    let mut sources = Vec::with_capacity(680);

    // ── Verified daily-crawl front sources ───────────────────────
    // These feeds are validated-working RSS endpoints (HTTP 200, valid
    // RSS/Atom, items > 0) prioritised at the top of the registry because the
    // daily `crawl_cycle` deterministically selects the first enabled tier-1/2
    // sources.  They are aligned to Starz's market focus (Morocco / Tunisia /
    // Egypt primary, EU secondary) and the battery-storage product line, with a
    // base of reliable global wires.  Legacy premium wires further down
    // (Reuters/Bloomberg/FT/etc.) remain registered for historical metadata but
    // no longer occupy the daily crawl window.
    //
    // MENA / Africa — primary target-market coverage.
    sources.push(
        Source::new(
            "aljazeera_all",
            "Al Jazeera English",
            "https://www.aljazeera.com",
            Region::MiddleEast,
            Category::News,
            1,
        )
        .rss("https://www.aljazeera.com/xml/rss/all.xml")
        .interval(15)
        .notes("MENA-wide news desk; strong Maghreb and Egypt coverage"),
    );
    sources.push(
        Source::new(
            "france24_en",
            "France 24 English",
            "https://www.france24.com/en/",
            Region::Europe,
            Category::News,
            1,
        )
        .rss("https://www.france24.com/en/rss")
        .interval(20)
        .notes("Francophone Africa / Maghreb desk"),
    );
    sources.push(
        Source::new(
            "northafricapost",
            "The North Africa Post",
            "https://northafricapost.com",
            Region::Africa,
            Category::News,
            1,
        )
        .rss("https://northafricapost.com/feed")
        .interval(60)
        .notes("Morocco / Tunisia / Algeria political and economic news"),
    );
    sources.push(
        Source::new(
            "dailynewsegypt",
            "Daily News Egypt",
            "https://www.dailynewsegypt.com",
            Region::Africa,
            Category::News,
            1,
        )
        .rss("https://www.dailynewsegypt.com/feed/")
        .interval(60)
        .notes("Egypt business and energy news (primary target market)"),
    );
    sources.push(
        Source::new(
            "egypt_independent",
            "Egypt Independent",
            "https://www.egyptindependent.com",
            Region::Africa,
            Category::News,
            1,
        )
        .rss("https://www.egyptindependent.com/feed/")
        .interval(60)
        .notes("Egypt national news (primary target market)"),
    );
    sources.push(
        Source::new(
            "arabnews",
            "Arab News",
            "https://www.arabnews.com",
            Region::MiddleEast,
            Category::News,
            1,
        )
        .rss("https://www.arabnews.com/rss.xml")
        .interval(30)
        .notes("Pan-Arab business and regional coverage"),
    );
    sources.push(
        Source::new(
            "middleeasteye",
            "Middle East Eye",
            "https://www.middleeasteye.net",
            Region::MiddleEast,
            Category::News,
            2,
        )
        .rss("https://www.middleeasteye.net/rss")
        .interval(60)
        .notes("MENA political and economic coverage"),
    );
    sources.push(
        Source::new(
            "africanews",
            "Africanews",
            "https://www.africanews.com",
            Region::Africa,
            Category::News,
            2,
        )
        .rss("https://www.africanews.com/feed/rss")
        .interval(60)
        .notes("Pan-African news network"),
    );
    // Energy storage / battery — product-line intelligence.
    sources.push(
        Source::new(
            "energy_storage_news",
            "Energy-Storage.news",
            "https://www.energy-storage.news",
            Region::Global,
            Category::EnergyResources,
            1,
        )
        .rss("https://www.energy-storage.news/feed/")
        .interval(60)
        .notes("Battery energy storage systems (BESS) industry intelligence"),
    );
    sources.push(
        Source::new(
            "electrek",
            "Electrek",
            "https://electrek.co",
            Region::Global,
            Category::EnergyResources,
            1,
        )
        .rss("https://electrek.co/feed/")
        .interval(60)
        .notes("EV, battery and clean-energy industry news"),
    );
    sources.push(
        Source::new(
            "pv_magazine",
            "pv magazine",
            "https://www.pv-magazine.com",
            Region::Global,
            Category::EnergyResources,
            2,
        )
        .rss("https://www.pv-magazine.com/feed/")
        .interval(90)
        .notes("Solar and storage market coverage"),
    );
    sources.push(
        Source::new(
            "oilprice",
            "OilPrice.com",
            "https://oilprice.com",
            Region::Global,
            Category::EnergyResources,
            2,
        )
        .rss("https://oilprice.com/rss/main")
        .interval(90)
        .notes("Energy commodity and markets coverage"),
    );
    // EU / global wires — reliable baseline.
    sources.push(
        Source::new(
            "bbc_world",
            "BBC World News",
            "https://www.bbc.com/news/world",
            Region::Global,
            Category::News,
            1,
        )
        .rss("https://feeds.bbci.co.uk/news/world/rss.xml")
        .interval(15),
    );
    sources.push(
        Source::new(
            "dw_en",
            "Deutsche Welle (English)",
            "https://www.dw.com/en/",
            Region::Europe,
            Category::News,
            1,
        )
        .rss("https://rss.dw.com/rdf/rss-en-all")
        .interval(20),
    );
    sources.push(
        Source::new(
            "euronews",
            "Euronews",
            "https://www.euronews.com",
            Region::Europe,
            Category::News,
            2,
        )
        .rss("https://www.euronews.com/rss")
        .interval(30),
    );
    sources.push(
        Source::new(
            "guardian_world",
            "The Guardian — World",
            "https://www.theguardian.com/world",
            Region::Global,
            Category::News,
            1,
        )
        .rss("https://www.theguardian.com/world/rss")
        .interval(15),
    );
    sources.push(
        Source::new(
            "nyt_world",
            "New York Times — World",
            "https://www.nytimes.com/section/world",
            Region::NorthAmerica,
            Category::News,
            1,
        )
        .rss("https://rss.nytimes.com/services/xml/rss/nyt/World.xml")
        .interval(15),
    );
    // Finance / markets.
    sources.push(
        Source::new(
            "nyt_business",
            "New York Times — Business",
            "https://www.nytimes.com/section/business",
            Region::NorthAmerica,
            Category::Finance,
            2,
        )
        .rss("https://rss.nytimes.com/services/xml/rss/nyt/Business.xml")
        .interval(30),
    );
    sources.push(
        Source::new(
            "economist_finance",
            "The Economist — Finance & Economics",
            "https://www.economist.com/finance-and-economics",
            Region::Global,
            Category::Finance,
            2,
        )
        .rss("https://www.economist.com/finance-and-economics/rss.xml")
        .interval(60),
    );
    sources.push(
        Source::new(
            "marketwatch",
            "MarketWatch Top Stories",
            "https://www.marketwatch.com",
            Region::Global,
            Category::Finance,
            2,
        )
        .rss("https://feeds.marketwatch.com/marketwatch/topstories/")
        .interval(30),
    );
    // Supply chain / technology.
    sources.push(
        Source::new(
            "gcaptain",
            "gCaptain — Maritime",
            "https://gcaptain.com",
            Region::Global,
            Category::SupplyChain,
            2,
        )
        .rss("https://gcaptain.com/feed/")
        .interval(60)
        .notes("Maritime logistics and shipping disruption signal"),
    );
    sources.push(
        Source::new(
            "techcrunch",
            "TechCrunch",
            "https://techcrunch.com",
            Region::Global,
            Category::Technology,
            2,
        )
        .rss("https://techcrunch.com/feed/")
        .interval(45),
    );
    sources.push(
        Source::new(
            "theverge",
            "The Verge",
            "https://www.theverge.com",
            Region::Global,
            Category::Technology,
            2,
        )
        .rss("https://www.theverge.com/rss/index.xml")
        .interval(45),
    );

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
        .notes("Requires API key; free tier available")
        .capability(SourceCapability::RequiresCredentials),
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
        .notes("Requires subscription; scrape summaries only")
        .capability(SourceCapability::RequiresCredentials),
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
        .interval(10)
        .capability(SourceCapability::RequiresCredentials),
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
        .interval(60)
        .capability(SourceCapability::RequiresCredentials),
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

    // ── Battery & Energy Storage (BESS pivot) ─────────────────────
    // Trade press and supply-chain trackers for stationary storage (BESS),
    // battery cells, BMS, and PV-plus-storage. These feed company / POI /
    // procurement discovery for Starz's 5/10/15 kWh pack + BMS business.
    // (energy_storage_news and pv_magazine are registered in the verified
    // daily-crawl front block near the top of this registry.)
    sources.push(
        Source::new(
            "pv_magazine_australia",
            "pv magazine Australia",
            "https://www.pv-magazine-australia.com",
            Region::AsiaPacific,
            Category::EnergyResources,
            3,
        )
        .rss("https://www.pv-magazine-australia.com/feed/")
        .interval(180)
        .notes("Australia is a top residential/C&I BESS demand market."),
    );
    sources.push(
        Source::new(
            "pv_tech",
            "PV Tech",
            "https://www.pv-tech.org",
            Region::Global,
            Category::EnergyResources,
            2,
        )
        .rss("https://www.pv-tech.org/feed/")
        .interval(120)
        .notes("Solar manufacturing + storage supply chain (Solar Media)."),
    );
    sources.push(
        Source::new(
            "electrive_com",
            "electrive",
            "https://www.electrive.com",
            Region::Europe,
            Category::EnergyResources,
            3,
        )
        .rss("https://www.electrive.com/feed/")
        .interval(120)
        .notes("E-mobility & battery industry news (light EV pack demand)."),
    );
    sources.push(
        Source::new(
            "battery_news_de",
            "Battery-News.de",
            "https://battery-news.de",
            Region::Europe,
            Category::EnergyResources,
            3,
        )
        .rss("https://battery-news.de/feed/")
        .interval(180)
        .notes("Battery cell, pack and BMS industry news (DE/EU)."),
    );
    sources.push(
        Source::new(
            "cleantechnica",
            "CleanTechnica",
            "https://cleantechnica.com",
            Region::Global,
            Category::EnergyResources,
            4,
        )
        .rss("https://cleantechnica.com/feed/")
        .interval(180)
        .notes("Cleantech / battery / storage aggregator (secondary signal)."),
    );
    sources.push(
        Source::new(
            "benchmark_minerals",
            "Benchmark Mineral Intelligence",
            "https://www.benchmarkminerals.com",
            Region::Global,
            Category::SupplyChain,
            2,
        )
        .interval(240)
        .notes("Li-ion cell & raw-material price/supply benchmarks; cell-sourcing risk."),
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

// ─────────────────────────────────────────────────────────────────────────────
// Stateful weighted-fair due-source scheduler (P0 #1)
// ─────────────────────────────────────────────────────────────────────────────

/// Score weights for the weighted-fair scheduler.
pub const COVERAGE_DEBT_WEIGHT: f64 = 0.45;
pub const TIER_PRIORITY_WEIGHT: f64 = 0.20;
pub const SOURCE_QUALITY_WEIGHT: f64 = 0.15;
pub const REGIONAL_COVERAGE_DEBT_WEIGHT: f64 = 0.10;
pub const CATEGORY_COVERAGE_DEBT_WEIGHT: f64 = 0.10;

/// Additive score boost for forced sources: they win contested budget slots
/// while due, but do not receive permanent slots.
pub const FORCED_SOURCE_BOOST: f64 = 0.50;

/// Slugs that receive the forced-source score boost.
pub const FORCED_SOURCE_SLUGS: [&str; 4] = [
    "globes_il_tech",
    "reddit_worldnews",
    "reddit_geopolitics",
    "telegram_channels",
];

/// Runtime scheduling state consumed by the scheduler. Implemented for
/// [`PgStore`] and, in tests, by in-memory fakes.
#[async_trait]
pub trait SourceRuntimeStateProvider: Send + Sync {
    async fn load_runtime_states(&self) -> anyhow::Result<Vec<SourceRuntimeStateRow>>;
}

#[async_trait]
impl SourceRuntimeStateProvider for PgStore {
    async fn load_runtime_states(&self) -> anyhow::Result<Vec<SourceRuntimeStateRow>> {
        PgStore::load_source_runtime_states(self).await
    }
}

/// One scored scheduling candidate.
#[derive(Debug, Clone)]
pub struct SourceScheduleCandidate<'a> {
    pub source: &'a Source,
    pub coverage_debt: f64,
    pub priority: f64,
    pub next_due_at: DateTime<Utc>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub score: f64,
}

/// Outcome of one scheduling pass.
#[derive(Debug, Clone)]
pub struct SourceSelection {
    /// Sources selected for this pass, highest score first.
    pub selected: Vec<Source>,
    /// Sources eligible at selection time (`next_due_at <= now` with a closed
    /// circuit) — the full backlog for this pass.
    pub due: usize,
    /// Eligible sources left unscheduled after applying the budget.
    pub coverage_debt_remaining: usize,
}

/// True when a source may be attempted at `now`.
pub fn is_source_due(
    source: &Source,
    runtime: Option<&SourceRuntimeStateRow>,
    now: DateTime<Utc>,
) -> bool {
    if !source.enabled || !effective_capability(source, runtime, now).is_operational() {
        return false;
    }
    match runtime {
        None => true,
        Some(row) => {
            if row.next_due_at > now {
                return false;
            }
            !row.circuit_open_until
                .map(|until| until > now)
                .unwrap_or(false)
        }
    }
}

fn interval_seconds(source: &Source) -> f64 {
    f64::from(source.min_interval_minutes.max(1)) * 60.0
}

fn tier_priority(tier: u8) -> f64 {
    (5.0 - f64::from(tier.min(5))) / 4.0
}

fn coverage_ratio(
    last_success_at: Option<DateTime<Utc>>,
    source: &Source,
    now: DateTime<Utc>,
) -> f64 {
    match last_success_at {
        Some(last_success) => {
            let elapsed = (now - last_success).num_seconds().max(0) as f64;
            elapsed / interval_seconds(source)
        }
        None => f64::INFINITY,
    }
}

fn normalize_debt(raw: f64, scale: f64) -> f64 {
    if raw.is_finite() {
        (raw / scale).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn due_fraction(counts: &HashMap<&str, (usize, usize)>, key: &str) -> f64 {
    match counts.get(key) {
        Some((due, total)) if *total > 0 => *due as f64 / *total as f64,
        _ => 0.0,
    }
}

/// Score and rank eligible sources by coverage debt, tier priority, source
/// quality, and regional/category coverage debt. Forced slugs receive an
/// additive [`FORCED_SOURCE_BOOST`].
pub fn rank_due_sources<'a>(
    sources: &'a [Source],
    states: &HashMap<&str, &SourceRuntimeStateRow>,
    now: DateTime<Utc>,
    forced_slugs: &[&str],
) -> Vec<SourceScheduleCandidate<'a>> {
    let mut region_counts: HashMap<&str, (usize, usize)> = HashMap::new();
    let mut category_counts: HashMap<&str, (usize, usize)> = HashMap::new();
    for source in sources {
        if !source.enabled {
            continue;
        }
        let runtime = states.get(source.slug.as_str()).copied();
        if !effective_capability(source, runtime, now).is_operational() {
            continue;
        }
        let due = is_source_due(source, runtime, now);
        for (counts, key) in [
            (&mut region_counts, source.region.as_str()),
            (&mut category_counts, source.category.as_str()),
        ] {
            let entry = counts.entry(key).or_insert((0, 0));
            entry.1 += 1;
            if due {
                entry.0 += 1;
            }
        }
    }

    let eligible: Vec<(&Source, Option<&SourceRuntimeStateRow>)> = sources
        .iter()
        .filter(|source| source.enabled)
        .map(|source| (source, states.get(source.slug.as_str()).copied()))
        .filter(|(source, runtime)| is_source_due(source, *runtime, now))
        .collect();

    let max_ratio = eligible
        .iter()
        .map(|(source, runtime)| {
            coverage_ratio(runtime.and_then(|row| row.last_success_at), source, now)
        })
        .filter(|ratio| ratio.is_finite())
        .fold(1.0_f64, f64::max);

    let mut candidates: Vec<SourceScheduleCandidate<'a>> = eligible
        .into_iter()
        .map(|(source, runtime)| {
            let raw_debt = coverage_ratio(runtime.and_then(|row| row.last_success_at), source, now);
            let coverage_debt = normalize_debt(raw_debt, max_ratio);
            let priority = tier_priority(source.tier);
            let quality = runtime
                .and_then(|row| row.rolling_success_rate)
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let regional_debt = due_fraction(&region_counts, source.region.as_str());
            let category_debt = due_fraction(&category_counts, source.category.as_str());
            let mut score = coverage_debt * COVERAGE_DEBT_WEIGHT
                + priority * TIER_PRIORITY_WEIGHT
                + quality * SOURCE_QUALITY_WEIGHT
                + regional_debt * REGIONAL_COVERAGE_DEBT_WEIGHT
                + category_debt * CATEGORY_COVERAGE_DEBT_WEIGHT;
            if forced_slugs.contains(&source.slug.as_str()) {
                score += FORCED_SOURCE_BOOST;
            }
            SourceScheduleCandidate {
                source,
                coverage_debt,
                priority,
                next_due_at: runtime
                    .map(|row| row.next_due_at)
                    .unwrap_or(DateTime::<Utc>::MIN_UTC),
                last_attempt_at: runtime.and_then(|row| row.last_attempt_at),
                score,
            }
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.coverage_debt
                    .partial_cmp(&a.coverage_debt)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.last_attempt_at.cmp(&b.last_attempt_at))
            .then_with(|| a.source.slug.cmp(&b.source.slug))
    });
    candidates
}

/// Select up to `budget` due sources using persisted runtime state.
///
/// Only sources with `next_due_at <= now` and a closed circuit
/// (`circuit_open_until IS NULL OR <= now`) are eligible. Selected sources are
/// returned owned so callers (e.g. concurrent crawl loops) do not hold
/// registry borrows across awaits.
pub async fn select_due_sources<S: SourceRuntimeStateProvider + ?Sized>(
    store: &S,
    sources: &[Source],
    budget: usize,
    now: DateTime<Utc>,
) -> anyhow::Result<SourceSelection> {
    let states = store.load_runtime_states().await?;
    let state_by_slug: HashMap<&str, &SourceRuntimeStateRow> = states
        .iter()
        .map(|row| (row.source_slug.as_str(), row))
        .collect();
    let ranked = rank_due_sources(sources, &state_by_slug, now, &FORCED_SOURCE_SLUGS);
    let due = ranked.len();
    let selected = ranked
        .iter()
        .take(budget)
        .map(|candidate| candidate.source.clone())
        .collect();
    Ok(SourceSelection {
        selected,
        due,
        coverage_debt_remaining: due.saturating_sub(budget),
    })
}

/// Count operational sources that are still due after a crawl pass.
pub fn coverage_debt_remaining(
    sources: &[Source],
    states: &[SourceRuntimeStateRow],
    now: DateTime<Utc>,
) -> usize {
    let state_by_slug: HashMap<&str, &SourceRuntimeStateRow> = states
        .iter()
        .map(|row| (row.source_slug.as_str(), row))
        .collect();
    sources
        .iter()
        .filter(|source| {
            is_source_due(
                source,
                state_by_slug.get(source.slug.as_str()).copied(),
                now,
            )
        })
        .count()
}

/// Snapshot of the declared vs operational source universe for the admin
/// source-coverage metric. Only `operational` sources count toward the
/// product's source count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCoverageSummary {
    pub declared: usize,
    pub operational: usize,
    pub due: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub disabled: usize,
    pub never_crawled: usize,
}

/// Aggregate the registry + persisted runtime state into the admin
/// source-coverage metric.
pub fn source_coverage_summary(
    sources: &[Source],
    states: &[SourceRuntimeStateRow],
    now: DateTime<Utc>,
) -> SourceCoverageSummary {
    let state_by_slug: HashMap<&str, &SourceRuntimeStateRow> = states
        .iter()
        .map(|row| (row.source_slug.as_str(), row))
        .collect();
    let mut summary = SourceCoverageSummary {
        declared: sources.len(),
        ..SourceCoverageSummary::default()
    };
    for source in sources {
        if !source.enabled {
            summary.disabled += 1;
            continue;
        }
        let runtime = state_by_slug.get(source.slug.as_str()).copied();
        if !effective_capability(source, runtime, now).is_operational() {
            continue;
        }
        summary.operational += 1;
        if is_source_due(source, runtime, now) {
            summary.due += 1;
        }
        let degraded = runtime
            .map(|row| {
                row.consecutive_failures > 0
                    || row
                        .circuit_open_until
                        .map(|until| until > now)
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        if degraded {
            summary.degraded += 1;
        } else if runtime.and_then(|row| row.last_success_at).is_some() {
            summary.healthy += 1;
        } else {
            summary.never_crawled += 1;
        }
    }
    summary
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
        let file =
            NamedTempFile::new().unwrap_or_else(|error| panic!("test: create temp file: {error}"));
        fs::write(
            file.path(),
            "sources:\n  - slug: test_feed\n    name: Test Feed\n    url: https://example.com/feed\n    search_param: null\n    region: Global\n    category: News\n    tier: 1\n    needs_proxy: false\n    rss_url: https://example.com/rss\n    enabled: true\n    min_interval_minutes: 15\n    notes: runtime override\n",
        )
        .unwrap_or_else(|error| panic!("test: write source yaml: {error}"));

        let sources = load_sources_from_path(file.path())
            .unwrap_or_else(|error| panic!("test: load sources: {error}"));
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].slug, "test_feed");
    }

    #[test]
    fn load_sources_from_path_rejects_duplicate_slugs() {
        let file =
            NamedTempFile::new().unwrap_or_else(|error| panic!("test: create temp file: {error}"));
        fs::write(
            file.path(),
            "sources:\n  - slug: dup\n    name: One\n    url: https://example.com/one\n    search_param: null\n    region: Global\n    category: News\n    tier: 1\n    needs_proxy: false\n    rss_url: null\n    enabled: true\n    min_interval_minutes: 15\n    notes: null\n  - slug: dup\n    name: Two\n    url: https://example.com/two\n    search_param: null\n    region: Global\n    category: News\n    tier: 2\n    needs_proxy: false\n    rss_url: null\n    enabled: true\n    min_interval_minutes: 30\n    notes: null\n",
        )
        .unwrap_or_else(|error| panic!("test: write duplicate source yaml: {error}"));

        let error = load_sources_from_path(file.path()).unwrap_err();
        assert!(error.to_string().contains("duplicate source slug dup"));
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

#[cfg(test)]
mod scheduler_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use chrono::Duration;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRuntimeStore {
        rows: Mutex<HashMap<String, SourceRuntimeStateRow>>,
    }

    impl FakeRuntimeStore {
        fn insert_row(&self, row: SourceRuntimeStateRow) {
            self.rows
                .lock()
                .unwrap()
                .insert(row.source_slug.clone(), row);
        }

        fn record_success(&self, slug: &str, now: DateTime<Utc>, min_interval_minutes: i64) {
            let mut rows = self.rows.lock().unwrap();
            let row = rows
                .entry(slug.to_string())
                .or_insert_with(|| blank_row(slug, now));
            row.last_attempt_at = Some(now);
            row.last_success_at = Some(now);
            row.next_due_at = now + Duration::minutes(min_interval_minutes);
            row.consecutive_failures = 0;
            row.circuit_open_until = None;
            row.updated_at = now;
        }
    }

    #[async_trait]
    impl SourceRuntimeStateProvider for FakeRuntimeStore {
        async fn load_runtime_states(&self) -> anyhow::Result<Vec<SourceRuntimeStateRow>> {
            let rows = self.rows.lock().unwrap();
            let mut states: Vec<SourceRuntimeStateRow> = rows.values().cloned().collect();
            states.sort_by(|a, b| a.source_slug.cmp(&b.source_slug));
            Ok(states)
        }
    }

    fn blank_row(slug: &str, now: DateTime<Utc>) -> SourceRuntimeStateRow {
        SourceRuntimeStateRow {
            source_slug: slug.to_string(),
            last_attempt_at: None,
            last_success_at: None,
            next_due_at: now,
            consecutive_failures: 0,
            rolling_success_rate: None,
            rolling_latency_ms: None,
            last_http_status: None,
            circuit_open_until: None,
            etag: None,
            last_modified: None,
            last_error: None,
            updated_at: now,
        }
    }

    fn synthetic_source(slug: &str, region: Region, category: Category, tier: u8) -> Source {
        Source::new(
            slug,
            slug,
            "https://example.com/feed",
            region,
            category,
            tier,
        )
    }

    #[tokio::test]
    async fn due_scheduler_covers_all_sources_without_repeating_before_full_sweep() {
        let sources: Vec<Source> = (0..100)
            .map(|index| {
                synthetic_source(
                    &format!("synthetic_{index:03}"),
                    Region::Global,
                    Category::News,
                    1,
                )
            })
            .collect();
        let store = FakeRuntimeStore::default();
        let now = Utc::now();
        let interval_minutes = 60;
        let mut scheduled: Vec<String> = Vec::new();

        for cycle in 0..10 {
            let selection = select_due_sources(&store, &sources, 10, now).await.unwrap();
            assert_eq!(selection.selected.len(), 10, "cycle {cycle}");
            assert_eq!(selection.due, 100 - cycle * 10, "cycle {cycle}");
            assert_eq!(selection.coverage_debt_remaining, 100 - (cycle + 1) * 10);
            for source in selection.selected {
                let slug = source.slug.clone();
                assert!(
                    !scheduled.contains(&slug),
                    "source {slug} was scheduled twice before every source was scheduled once"
                );
                scheduled.push(slug.clone());
                store.record_success(&slug, now, interval_minutes);
            }
        }

        assert_eq!(scheduled.len(), 100);
        let unique: std::collections::HashSet<&String> = scheduled.iter().collect();
        assert_eq!(unique.len(), 100);
    }

    #[tokio::test]
    async fn circuit_open_source_is_ineligible_until_it_closes() {
        let sources = vec![
            synthetic_source("circuit_open", Region::Global, Category::News, 1),
            synthetic_source("healthy", Region::Global, Category::News, 1),
        ];
        let store = FakeRuntimeStore::default();
        let now = Utc::now();
        let mut open_row = blank_row("circuit_open", now);
        open_row.next_due_at = now - Duration::minutes(1);
        open_row.consecutive_failures = 3;
        open_row.circuit_open_until = Some(now + Duration::hours(1));
        open_row.last_attempt_at = Some(now - Duration::minutes(30));
        store.insert_row(open_row);

        let selection = select_due_sources(&store, &sources, 10, now).await.unwrap();
        assert_eq!(selection.due, 1);
        assert_eq!(selection.selected.len(), 1);
        assert_eq!(selection.selected[0].slug, "healthy");

        let after_circuit = now + Duration::hours(1) + Duration::minutes(1);
        let selection = select_due_sources(&store, &sources, 10, after_circuit)
            .await
            .unwrap();
        assert_eq!(selection.due, 2);
        assert!(selection
            .selected
            .iter()
            .any(|source| source.slug == "circuit_open"));
    }

    #[test]
    fn forced_sources_get_a_boost_but_not_a_permanent_slot() {
        let sources = vec![
            synthetic_source("forced", Region::Global, Category::News, 5),
            synthetic_source("rival", Region::Global, Category::News, 1),
        ];
        let now = Utc::now();
        let mut forced_row = blank_row("forced", now);
        forced_row.last_success_at = Some(now - Duration::hours(2));
        forced_row.last_attempt_at = Some(now - Duration::hours(2));
        forced_row.next_due_at = now - Duration::minutes(1);
        let states = [forced_row];
        let state_by_slug: HashMap<&str, &SourceRuntimeStateRow> = states
            .iter()
            .map(|row| (row.source_slug.as_str(), row))
            .collect();

        let unboosted = rank_due_sources(&sources, &state_by_slug, now, &[]);
        assert_eq!(unboosted[0].source.slug, "rival");
        let boosted = rank_due_sources(&sources, &state_by_slug, now, &["forced"]);
        assert_eq!(boosted[0].source.slug, "forced");
        assert!(boosted[0].score > boosted[1].score);
    }

    #[tokio::test]
    async fn not_due_forced_source_does_not_consume_a_slot() {
        let sources = vec![
            synthetic_source("forced", Region::Global, Category::News, 1),
            synthetic_source("a", Region::Global, Category::News, 2),
            synthetic_source("b", Region::Global, Category::News, 2),
        ];
        let store = FakeRuntimeStore::default();
        let now = Utc::now();
        let mut forced_row = blank_row("forced", now);
        forced_row.last_success_at = Some(now);
        forced_row.last_attempt_at = Some(now);
        forced_row.next_due_at = now + Duration::hours(1);
        store.insert_row(forced_row);

        let selection = select_due_sources(&store, &sources, 1, now).await.unwrap();
        assert_eq!(selection.due, 2);
        assert_eq!(selection.selected.len(), 1);
        assert_ne!(selection.selected[0].slug, "forced");
    }

    #[test]
    fn coverage_summary_counts_only_operational_sources() {
        let mut requires_credentials =
            synthetic_source("needs_credentials", Region::Global, Category::News, 2);
        requires_credentials.capability = SourceCapability::RequiresCredentials;
        let mut blocked = synthetic_source("blocked", Region::Global, Category::News, 2);
        blocked.capability = SourceCapability::Blocked;
        let mut unsupported = synthetic_source("unsupported", Region::Global, Category::News, 2);
        unsupported.capability = SourceCapability::Unsupported;
        let mut disabled = synthetic_source("disabled", Region::Global, Category::News, 2);
        disabled.enabled = false;

        let sources = vec![
            requires_credentials,
            blocked,
            unsupported,
            disabled,
            synthetic_source("healthy", Region::Global, Category::News, 1),
            synthetic_source("never_crawled", Region::Global, Category::News, 1),
            synthetic_source("failing", Region::Global, Category::News, 1),
        ];

        let now = Utc::now();
        let mut healthy_row = blank_row("healthy", now);
        healthy_row.last_success_at = Some(now - Duration::minutes(30));
        healthy_row.last_attempt_at = Some(now - Duration::minutes(30));
        healthy_row.next_due_at = now + Duration::minutes(30);
        let mut failing_row = blank_row("failing", now);
        failing_row.last_success_at = Some(now - Duration::minutes(90));
        failing_row.last_attempt_at = Some(now - Duration::minutes(10));
        failing_row.consecutive_failures = 2;
        failing_row.next_due_at = now - Duration::minutes(1);
        let states = vec![healthy_row, failing_row];

        let summary = source_coverage_summary(&sources, &states, now);
        assert_eq!(summary.declared, 7);
        assert_eq!(summary.operational, 3);
        assert_eq!(summary.healthy, 1);
        assert_eq!(summary.degraded, 1);
        assert_eq!(summary.never_crawled, 1);
        assert_eq!(summary.disabled, 1);
        assert_eq!(summary.due, 2);
    }

    #[test]
    fn covered_source_is_not_due_before_its_interval_elapses() {
        let sources = vec![synthetic_source("daily", Region::Global, Category::News, 1)];
        let now = Utc::now();
        let mut row = blank_row("daily", now);
        row.last_success_at = Some(now - Duration::minutes(30));
        row.next_due_at = now + Duration::minutes(30);
        let states = vec![row];

        assert_eq!(coverage_debt_remaining(&sources, &states, now), 0);
        let later = now + Duration::minutes(31);
        assert_eq!(coverage_debt_remaining(&sources, &states, later), 1);
    }
}
