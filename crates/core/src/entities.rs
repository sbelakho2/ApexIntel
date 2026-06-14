use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Maximum length for entity name fields.
const MAX_NAME_LENGTH: usize = 500;

/// Truncate a string to fit within a maximum byte length, preserving UTF-8.
fn truncate_to(input: String, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        input
    } else {
        let mut end = max_bytes;
        while !input.is_char_boundary(end) {
            end -= 1;
        }
        let mut truncated: String = input[..end].to_string();
        truncated.push('…');
        truncated
    }
}

// ────────────────────────────────────────────
// Company
// ────────────────────────────────────────────

/// Broad classification of a company in the supply-chain graph.
///
/// `Other(String)` admits arbitrary strings from external data sources;
/// use `as_str`/`from_str` for lossless round-trips.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CompanyType {
    Oem,
    Ems,
    Distributor,
    Tier1,
    Tier2,
    Authority,
    Research,
    Other(String),
}

impl CompanyType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Oem => "oem",
            Self::Ems => "ems",
            Self::Distributor => "distributor",
            Self::Tier1 => "tier1",
            Self::Tier2 => "tier2",
            Self::Authority => "authority",
            Self::Research => "research",
            Self::Other(s) => s.as_str(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "oem" => Self::Oem,
            "ems" => Self::Ems,
            "distributor" => Self::Distributor,
            "tier1" => Self::Tier1,
            "tier2" => Self::Tier2,
            "authority" => Self::Authority,
            "research" => Self::Research,
            other => Self::Other(other.to_string()),
        }
    }
}

/// A company node in the ApexIntel supply-chain knowledge graph.
///
/// Score fields (`risk_score`, `threat_score`, `overlap_score`,
/// `strategic_relevance`) are real-valued in `[0, 1]` and recomputed
/// by the graph-risk module each nightly run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub id: Uuid,
    pub name: String,
    pub legal_name: Option<String>,
    pub domain: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub company_type: CompanyType,
    pub industry_tags: Vec<String>,
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub risk_score: f64,
    pub threat_score: f64,
    pub overlap_score: f64,
    pub strategic_relevance: f64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Company {
    pub fn new(name: impl Into<String>, company_type: CompanyType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: truncate_to(name.into(), MAX_NAME_LENGTH),
            legal_name: None,
            domain: None,
            country_code: None,
            region: None,
            company_type,
            industry_tags: Vec::new(),
            employee_estimate: None,
            revenue_estimate_usd: None,
            risk_score: 0.0,
            threat_score: 0.0,
            overlap_score: 0.0,
            strategic_relevance: 0.0,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }
}

// ────────────────────────────────────────────
// Site
// ────────────────────────────────────────────

/// Physical facility type for a [`Site`] belonging to a [`Company`].
///
/// `Other(String)` admits values from external data feeds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SiteType {
    Plant,
    Warehouse,
    Office,
    Lab,
    Hq,
    Other(String),
}

impl SiteType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Plant => "plant",
            Self::Warehouse => "warehouse",
            Self::Office => "office",
            Self::Lab => "lab",
            Self::Hq => "hq",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// A physical site (factory, warehouse, R&D lab, etc.) linked to a [`Company`].
///
/// Optional geo fields (`lat`, `lon`) enable distance-based risk queries.
/// `free_zone` carries the economic zone designation where applicable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: Uuid,
    pub company_id: Uuid,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub site_type: SiteType,
    pub capabilities: Vec<String>,
    pub certifications: Vec<String>,
    pub employee_estimate: Option<i32>,
    pub free_zone: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Site {
    pub fn new(company_id: Uuid, name: impl Into<String>, site_type: SiteType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            company_id,
            name: truncate_to(name.into(), MAX_NAME_LENGTH),
            address: None,
            city: None,
            country_code: None,
            region: None,
            lat: None,
            lon: None,
            site_type,
            capabilities: Vec::new(),
            certifications: Vec::new(),
            employee_estimate: None,
            free_zone: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }
}

// ────────────────────────────────────────────
// Person (POI)
// ────────────────────────────────────────────

/// High-level function a Person-of-Interest (POI) serves in their organisation.
///
/// Used by the engagement module to select appropriate outreach templates
/// and by the features module to compute `role_seniority_score`.
///
/// This is the **canonical** role-family enum shared across all crates.
/// The POI crate re-exports this type rather than defining its own.
///
/// `Other(String)` preserves unrecognised categories from external sources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RoleFamily {
    Procurement,
    Quality,
    /// Supplier quality — more specific than `Quality` for SQE roles.
    SupplierQuality,
    Engineering,
    Operations,
    Security,
    Executive,
    Government,
    /// Free-zone authority officials (TFZ, TMSA, etc.).
    FreeZoneAuthority,
    Logistics,
    /// Port / logistics authority roles (Tanger-Med, etc.).
    PortLogistics,
    /// Certification body auditors (UL, TÜV, etc.).
    CertificationBody,
    /// Industry association representatives (IPC, SMTA, etc.).
    IndustryAssociation,
    /// Component / module distributors.
    Distributor,
    Finance,
    Legal,
    Military,
    Intelligence,
    Other(String),
}

impl RoleFamily {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Procurement => "procurement",
            Self::Quality => "quality",
            Self::SupplierQuality => "supplier_quality",
            Self::Engineering => "engineering",
            Self::Operations => "operations",
            Self::Security => "security",
            Self::Executive => "executive",
            Self::Government => "government",
            Self::FreeZoneAuthority => "free_zone_authority",
            Self::Logistics => "logistics",
            Self::PortLogistics => "port_logistics",
            Self::CertificationBody => "certification_body",
            Self::IndustryAssociation => "industry_association",
            Self::Distributor => "distributor",
            Self::Finance => "finance",
            Self::Legal => "legal",
            Self::Military => "military",
            Self::Intelligence => "intelligence",
            Self::Other(s) => s.as_str(),
        }
    }

    /// Canonical lower_snake_case label — identical to `as_str` for named
    /// variants, normalised for `Other`.
    pub fn canonical_label(&self) -> String {
        match self {
            Self::Other(v) => v.trim().to_ascii_lowercase().replace(' ', "_"),
            other => other.as_str().to_string(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "procurement" => Self::Procurement,
            "quality" => Self::Quality,
            "supplier_quality" => Self::SupplierQuality,
            "engineering" => Self::Engineering,
            "operations" => Self::Operations,
            "security" => Self::Security,
            "executive" => Self::Executive,
            "government" => Self::Government,
            "free_zone_authority" => Self::FreeZoneAuthority,
            "logistics" => Self::Logistics,
            "port_logistics" => Self::PortLogistics,
            "certification_body" => Self::CertificationBody,
            "industry_association" => Self::IndustryAssociation,
            "distributor" => Self::Distributor,
            "finance" => Self::Finance,
            "legal" => Self::Legal,
            "military" => Self::Military,
            "intelligence" => Self::Intelligence,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Six-dimensional vector describing a person's (or organisation's) stated
/// and inferred decision-making priorities.
///
/// Values are real-valued weights in `[0, 1]`; they need not sum to 1.
/// [`PriorityVector::dominant`] returns the name of the highest-weighted
/// dimension, with `"cost"` as the tiebreaker.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PriorityVector {
    pub cost: f64,
    pub quality: f64,
    pub speed: f64,
    pub resilience: f64,
    pub compliance: f64,
    pub security: f64,
}

impl PriorityVector {
    pub fn dominant(&self) -> &str {
        let pairs = [
            ("cost", self.cost),
            ("quality", self.quality),
            ("speed", self.speed),
            ("resilience", self.resilience),
            ("compliance", self.compliance),
            ("security", self.security),
        ];
        if pairs.iter().all(|(_, value)| !value.is_finite()) {
            return "unknown";
        }
        let mut best = pairs[0];
        for &pair in &pairs[1..] {
            // Use partial_cmp so that NaN never silently wins the comparison.
            // If best is NaN, any finite value replaces it; if pair is NaN it
            // is skipped (Greater never matches).
            if matches!(
                pair.1.partial_cmp(&best.1),
                Some(std::cmp::Ordering::Greater)
            ) || best.1.is_nan()
            {
                best = pair;
            }
        }
        best.0
    }
}

/// A Person-of-Interest (POI) — a key decision maker tracked by ApexIntel.
///
/// Linked to a [`Company`] via the graph layer.  Score fields
/// (`pain_index`, `influence_score`) are recomputed by the POI-refresh
/// stage of the nightly pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
    pub name_ar: Option<String>,
    pub name_fr: Option<String>,
    pub primary_org_id: Option<Uuid>,
    pub current_role: Option<String>,
    pub role_family: RoleFamily,
    pub region: Option<String>,
    pub country_code: Option<String>,
    pub public_bio: Option<String>,
    pub public_email: Option<String>,
    pub phone: Option<String>,
    pub personal_email: Option<String>,
    pub photo_hash: Option<String>,

    // Derived features
    pub priority_vector: PriorityVector,
    pub decision_mode: Option<String>,
    pub influence_score: f64,
    pub role_drift_score: f64,
    pub change_risk: f64,
    pub pain_index: f64,
    pub preferred_proof_type: Option<String>,
    pub trigger_topics: Vec<String>,

    // Psychological / professional profile
    pub decision_style: Option<String>,
    pub risk_tolerance: Option<String>,
    pub change_appetite: Option<String>,
    pub communication_style: Option<String>,

    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Person {
    pub fn new(name: impl Into<String>, role_family: RoleFamily) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: truncate_to(name.into(), MAX_NAME_LENGTH),
            name_ar: None,
            name_fr: None,
            primary_org_id: None,
            current_role: None,
            role_family,
            region: None,
            country_code: None,
            public_bio: None,
            public_email: None,
            phone: None,
            personal_email: None,
            photo_hash: None,
            priority_vector: PriorityVector::default(),
            decision_mode: None,
            influence_score: 0.0,
            role_drift_score: 0.0,
            change_risk: 0.0,
            pain_index: 0.0,
            preferred_proof_type: None,
            trigger_topics: Vec::new(),
            decision_style: None,
            risk_tolerance: None,
            change_appetite: None,
            communication_style: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }
}

// ────────────────────────────────────────────
// POI Artifact
// ────────────────────────────────────────────

/// The type of digital artifact linked to a [`Person`].
///
/// Used to weight evidence in [`compute_priority_vector`](crate::features::compute_priority_vector)
/// and to build the `ArtifactSummarySection` in a POI dossier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ArtifactType {
    PressQuote,
    SpeakerBio,
    Patent,
    StandardsRole,
    Interview,
    Podcast,
    Article,
    RoleChange,
    SocialPost,
    Other(String),
}

impl ArtifactType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::PressQuote => "press_quote",
            Self::SpeakerBio => "speaker_bio",
            Self::Patent => "patent",
            Self::StandardsRole => "standards_role",
            Self::Interview => "interview",
            Self::Podcast => "podcast",
            Self::Article => "article",
            Self::RoleChange => "role_change",
            Self::SocialPost => "social_post",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// A digital artifact (patent, publication, talk, etc.) associated with a POI.
///
/// `url` is the canonical source; `published_at` drives freshness scoring
/// in [`compute_pain_index`](crate::features::compute_pain_index).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiArtifact {
    pub id: Uuid,
    pub person_id: Uuid,
    pub artifact_type: ArtifactType,
    pub title: Option<String>,
    pub content_summary: Option<String>,
    pub url: String,
    pub source_domain: Option<String>,
    pub language: Option<String>,
    pub topics: Vec<String>,
    pub sentiment_score: Option<f64>,
    pub key_phrases: Vec<String>,
    pub ts_utc: DateTime<Utc>,
    pub provenance: serde_json::Value,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl PoiArtifact {
    pub fn new(
        person_id: Uuid,
        artifact_type: ArtifactType,
        url: impl Into<String>,
        ts_utc: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            person_id,
            artifact_type,
            title: None,
            content_summary: None,
            url: url.into(),
            source_domain: None,
            language: None,
            topics: Vec::new(),
            sentiment_score: None,
            key_phrases: Vec::new(),
            ts_utc,
            provenance: serde_json::json!({}),
            metadata: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }
}

// ────────────────────────────────────────────
// Observation (time-stamped atomic fact)
// ────────────────────────────────────────────

/// Discriminant for the type of real-world signal captured in an [`Observation`].
///
/// Determines how downstream modules interpret `payload` and which recipes
/// are eligible to fire on this observation.  Hash-safe for use as a map key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ObservationType {
    JobPost,
    TenderPosted,
    WebChange,
    CertificationUpdate,
    PatentPublished,
    PortMetric,
    CommodityPrice,
    FxRate,
    DnsPosture,
    NewDomain,
    VulnNotice,
    PersonMention,
    RoleChange,
    SpeakerAppearance,
    ProcurementSignal,
    CompetitorEvent,
    /// Dark web forum post or paste matching monitored keywords.
    DarkWebPost,
}

impl ObservationType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::JobPost => "JobPost",
            Self::TenderPosted => "TenderPosted",
            Self::WebChange => "WebChange",
            Self::CertificationUpdate => "CertificationUpdate",
            Self::PatentPublished => "PatentPublished",
            Self::PortMetric => "PortMetric",
            Self::CommodityPrice => "CommodityPrice",
            Self::FxRate => "FxRate",
            Self::DnsPosture => "DnsPosture",
            Self::NewDomain => "NewDomain",
            Self::VulnNotice => "VulnNotice",
            Self::PersonMention => "PersonMention",
            Self::RoleChange => "RoleChange",
            Self::SpeakerAppearance => "SpeakerAppearance",
            Self::ProcurementSignal => "ProcurementSignal",
            Self::CompetitorEvent => "CompetitorEvent",
            Self::DarkWebPost => "DarkWebPost",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "JobPost" => Some(Self::JobPost),
            "TenderPosted" => Some(Self::TenderPosted),
            "WebChange" => Some(Self::WebChange),
            "CertificationUpdate" => Some(Self::CertificationUpdate),
            "PatentPublished" => Some(Self::PatentPublished),
            "PortMetric" => Some(Self::PortMetric),
            "CommodityPrice" => Some(Self::CommodityPrice),
            "FxRate" => Some(Self::FxRate),
            "DnsPosture" => Some(Self::DnsPosture),
            "NewDomain" => Some(Self::NewDomain),
            "VulnNotice" => Some(Self::VulnNotice),
            "PersonMention" => Some(Self::PersonMention),
            "RoleChange" => Some(Self::RoleChange),
            "SpeakerAppearance" => Some(Self::SpeakerAppearance),
            "ProcurementSignal" => Some(Self::ProcurementSignal),
            "CompetitorEvent" => Some(Self::CompetitorEvent),
            "DarkWebPost" => Some(Self::DarkWebPost),
            _ => None,
        }
    }
}

/// A single real-world signal captured during the crawl stage.
///
/// `payload` holds the raw extracted data (e.g. job-post fields, price tick).
/// `source_metadata` carries provenance info (URL, fetch timestamp, extractor
/// version) for auditability.  `confidence` defaults to 1.0 for direct crawls
/// and decreases for inferred/interpolated observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: Uuid,
    pub observation_type: ObservationType,
    pub entity_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub ts_utc: DateTime<Utc>,
    pub value: serde_json::Value,
    pub provenance: serde_json::Value,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
}

impl Observation {
    pub fn new(
        observation_type: ObservationType,
        ts_utc: DateTime<Utc>,
        value: serde_json::Value,
        provenance: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            observation_type,
            entity_id: None,
            entity_type: None,
            ts_utc,
            value,
            provenance,
            confidence: 1.0,
            created_at: Utc::now(),
        }
    }
}

// ────────────────────────────────────────────
// Graph Edge
// ────────────────────────────────────────────

/// The semantic relationship type for a directed edge in the supply-chain graph.
///
/// Used by the graph-risk module's propagation and PageRank algorithms.
/// Add new variants to `as_str`/`from_str` in tandem to preserve
/// serde round-trips.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EdgeType {
    CompanySite,
    CompanyPerson,
    PersonPerson,
    CompanyCapability,
    CompanyCompany,
    SiteLogistics,
    VulnProduct,
    ProductCompany,
    PersonEvent,
    CompanyRegulation,
    PersonPatent,
    PersonStandard,
}

impl EdgeType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::CompanySite => "company_site",
            Self::CompanyPerson => "company_person",
            Self::PersonPerson => "person_person",
            Self::CompanyCapability => "company_capability",
            Self::CompanyCompany => "company_company",
            Self::SiteLogistics => "site_logistics",
            Self::VulnProduct => "vuln_product",
            Self::ProductCompany => "product_company",
            Self::PersonEvent => "person_event",
            Self::CompanyRegulation => "company_regulation",
            Self::PersonPatent => "person_patent",
            Self::PersonStandard => "person_standard",
        }
    }
}

/// A directed weighted edge in the supply-chain knowledge graph.
///
/// `source_id` → `target_id` with `weight` in `(0, ∞)` (defaults to 1.0).
/// Both `source_type` and `target_type` are free-form strings matching the
/// entity type labels used by [`AdjacencyGraph`](apex_graph::AdjacencyGraph).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_type: String,
    pub target_id: Uuid,
    pub target_type: String,
    pub edge_type: EdgeType,
    pub weight: f64,
    pub confidence: f64,
    pub evidence_ids: Vec<Uuid>,
    pub metadata: serde_json::Value,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

impl GraphEdge {
    pub fn new(
        source_id: Uuid,
        source_type: impl Into<String>,
        target_id: Uuid,
        target_type: impl Into<String>,
        edge_type: EdgeType,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            source_id,
            source_type: source_type.into(),
            target_id,
            target_type: target_type.into(),
            edge_type,
            weight: 1.0,
            confidence: 1.0,
            evidence_ids: Vec::new(),
            metadata: serde_json::json!({}),
            first_seen: now,
            last_seen: now,
        }
    }
}

// ────────────────────────────────────────────
// Certification
// ────────────────────────────────────────────

/// Lifecycle status of a quality or compliance [`Certification`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CertStatus {
    Active,
    Expired,
    Suspended,
    Pending,
}

impl CertStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Suspended => "suspended",
            Self::Pending => "pending",
        }
    }
}

/// A quality or compliance certification held by a [`Company`].
///
/// `is_valid()` returns `true` only when `status == Active` AND either
/// `valid_until` is `None` (no expiry recorded) or is a future date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certification {
    pub id: Uuid,
    pub company_id: Uuid,
    pub site_id: Option<Uuid>,
    pub standard: String,
    pub status: CertStatus,
    pub issuing_body: Option<String>,
    pub valid_from: Option<NaiveDate>,
    pub valid_until: Option<NaiveDate>,
    pub scope: Option<String>,
    pub evidence_url: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Certification {
    pub fn new(company_id: Uuid, standard: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            company_id,
            site_id: None,
            standard: standard.into(),
            status: CertStatus::Active,
            issuing_body: None,
            valid_from: None,
            valid_until: None,
            scope: None,
            evidence_url: None,
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn is_valid(&self) -> bool {
        let today = Utc::now().date_naive();
        match self.status {
            CertStatus::Active => {
                // Not yet effective
                if let Some(from) = self.valid_from {
                    if from > today {
                        return false;
                    }
                }
                // Already expired
                if let Some(until) = self.valid_until {
                    until >= today
                } else {
                    true
                }
            }
            _ => false,
        }
    }

    /// Returns true if the certification expires within the given number of days.
    pub fn is_expiring_within_days(&self, days: i64) -> bool {
        if !self.is_valid() {
            return false;
        }
        if let Some(until) = self.valid_until {
            let today = Utc::now().date_naive();
            let deadline = today + chrono::Duration::days(days);
            until <= deadline
        } else {
            false
        }
    }
}

// ────────────────────────────────────────────
// Logistics Node
// ────────────────────────────────────────────

/// A logistics node (port, hub, airport, rail terminal) in the supply-chain graph.
///
/// `node_type` is a free-form string; common values are `"port"`, `"airport"`,
/// `"rail_hub"`, `"ftz"` (free-trade zone).  Linked to companies and sites via
/// [`GraphEdge`] entries in the adjacency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogisticsNode {
    pub id: Uuid,
    pub name: String,
    pub node_type: String,
    pub country_code: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub metadata: serde_json::Value,
}

impl LogisticsNode {
    pub fn new(name: impl Into<String>, node_type: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            node_type: node_type.into(),
            country_code: None,
            lat: None,
            lon: None,
            metadata: serde_json::json!({}),
        }
    }
}

// ────────────────────────────────────────────
// Capability
// ────────────────────────────────────────────

/// Evidence quality grade for a [`Capability`] claim.
///
/// `A` = confirmed by audited cert or direct inspection.
/// `B` = supported by multiple secondary sources.
/// `C` = inferred from job posts or indirect signals.
/// `D` = unverified / single-source claim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProofGrade {
    A, // cert/registry
    B, // capability page
    C, // marketing claim
    D, // inferred
}

impl ProofGrade {
    pub fn as_str(&self) -> &str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
        }
    }

    /// Parse a proof grade from a free-text string, returning `None` for unknown values.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_uppercase().as_str() {
            "A" => Some(Self::A),
            "B" => Some(Self::B),
            "C" => Some(Self::C),
            "D" => Some(Self::D),
            _ => None,
        }
    }

    /// Parse with fallback: unknown values get the lowest grade (D).
    pub fn from_str_or_default(s: &str) -> Self {
        Self::from_str_opt(s).unwrap_or(Self::D)
    }
}

/// A specific manufacturing or service capability claimed and graded for a [`Company`].
///
/// `capability` is a short slug, e.g. `"SMT"`, `"die_casting"`, `"painting"`.
/// `proof_grade` reflects how well the claim is evidenced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    pub id: Uuid,
    pub company_id: Uuid,
    pub site_id: Option<Uuid>,
    pub capability: String,
    pub proof_grade: ProofGrade,
    pub evidence_urls: Vec<String>,
    pub first_seen: DateTime<Utc>,
    pub last_confirmed: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl Capability {
    pub fn new(company_id: Uuid, capability: impl Into<String>, proof_grade: ProofGrade) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            company_id,
            site_id: None,
            capability: capability.into(),
            proof_grade,
            evidence_urls: Vec::new(),
            first_seen: now,
            last_confirmed: now,
            metadata: serde_json::json!({}),
        }
    }
}

// ────────────────────────────────────────────
// Feature Store Row
// ────────────────────────────────────────────

/// A time-bucketed feature vector for an entity, used as ML model input.
///
/// Rows are keyed by `(entity_id, entity_type, time_bucket)` where
/// `time_bucket` is the Unix epoch of the bucket start divided by
/// `bucket_size_days * 86_400`.  `signal_counts` holds raw event counts
/// keyed by signal name; numeric feature slots are stored separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRow {
    pub entity_id: String,
    pub entity_type: String,
    pub time_bucket: i64,
    pub bucket_size_days: i32,
    pub signal_counts: HashMap<String, f64>,
    pub diffs: HashMap<String, f64>,
    pub pct_changes: HashMap<String, f64>,
    pub regime_flags: HashMap<String, bool>,
    pub volatility: HashMap<String, f64>,
    pub topic_drift: HashMap<String, f64>,
    pub neighbor_agg_1hop: HashMap<String, f64>,
    pub neighbor_agg_2hop: HashMap<String, f64>,
    pub poi_pain_index: Option<f64>,
    pub poi_role_drift: Option<f64>,
    pub poi_influence_delta: Option<f64>,
}

impl FeatureRow {
    pub fn new(
        entity_id: impl Into<String>,
        entity_type: impl Into<String>,
        time_bucket: i64,
        bucket_size_days: i32,
    ) -> Self {
        Self {
            entity_id: entity_id.into(),
            entity_type: entity_type.into(),
            time_bucket,
            bucket_size_days,
            signal_counts: HashMap::new(),
            diffs: HashMap::new(),
            pct_changes: HashMap::new(),
            regime_flags: HashMap::new(),
            volatility: HashMap::new(),
            topic_drift: HashMap::new(),
            neighbor_agg_1hop: HashMap::new(),
            neighbor_agg_2hop: HashMap::new(),
            poi_pain_index: None,
            poi_role_drift: None,
            poi_influence_delta: None,
        }
    }
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_company_new() {
        let c = Company::new("Starz Electronics", CompanyType::Ems);
        assert_eq!(c.name, "Starz Electronics");
        assert_eq!(c.company_type, CompanyType::Ems);
        assert_eq!(c.risk_score, 0.0);
        assert!(c.domain.is_none());
    }

    #[test]
    fn test_company_type_roundtrip() {
        for ct in &[
            CompanyType::Oem,
            CompanyType::Ems,
            CompanyType::Distributor,
            CompanyType::Authority,
        ] {
            let s = ct.as_str();
            let back = CompanyType::from_str(s);
            assert_eq!(ct, &back);
        }
        let custom = CompanyType::Other("foundry".into());
        assert_eq!(CompanyType::from_str("foundry"), custom);
    }

    #[test]
    fn test_site_new() {
        let company_id = Uuid::new_v4();
        let s = Site::new(company_id, "Tunis Plant", SiteType::Plant);
        assert_eq!(s.company_id, company_id);
        assert_eq!(s.name, "Tunis Plant");
        assert_eq!(s.site_type, SiteType::Plant);
    }

    #[test]
    fn test_person_new() {
        let p = Person::new("Ahmed Ben Salah", RoleFamily::Procurement);
        assert_eq!(p.name, "Ahmed Ben Salah");
        assert_eq!(p.role_family, RoleFamily::Procurement);
        assert_eq!(p.pain_index, 0.0);
        assert_eq!(p.influence_score, 0.0);
    }

    #[test]
    fn test_role_family_roundtrip() {
        for rf in &[
            RoleFamily::Procurement,
            RoleFamily::Quality,
            RoleFamily::Engineering,
            RoleFamily::Military,
            RoleFamily::Intelligence,
        ] {
            let s = rf.as_str();
            let back = RoleFamily::from_str(s);
            assert_eq!(rf, &back);
        }
    }

    #[test]
    fn test_priority_vector_dominant() {
        let pv = PriorityVector {
            cost: 0.3,
            quality: 0.9,
            speed: 0.1,
            resilience: 0.2,
            compliance: 0.4,
            security: 0.5,
        };
        assert_eq!(pv.dominant(), "quality");

        let pv2 = PriorityVector::default();
        // all zero, first wins
        assert_eq!(pv2.dominant(), "cost");
    }

    #[test]
    fn test_observation_new() {
        let now = Utc::now();
        let obs = Observation::new(
            ObservationType::JobPost,
            now,
            serde_json::json!({"role": "SQE", "role_family": "quality"}),
            serde_json::json!({"url": "https://example.com/job/123", "fetch_ts": now.to_rfc3339()}),
        );
        assert_eq!(obs.observation_type, ObservationType::JobPost);
        assert_eq!(obs.confidence, 1.0);
        assert!(obs.entity_id.is_none());
    }

    #[test]
    fn test_observation_type_roundtrip() {
        let types = vec![
            ObservationType::JobPost,
            ObservationType::TenderPosted,
            ObservationType::WebChange,
            ObservationType::CommodityPrice,
            ObservationType::FxRate,
            ObservationType::DnsPosture,
            ObservationType::CompetitorEvent,
        ];
        for t in types {
            let s = t.as_str();
            let back = ObservationType::from_str(s).unwrap();
            assert_eq!(t, back);
        }
        assert!(ObservationType::from_str("UnknownType").is_none());
    }

    #[test]
    fn test_graph_edge_new() {
        let src = Uuid::new_v4();
        let tgt = Uuid::new_v4();
        let edge = GraphEdge::new(src, "company", tgt, "person", EdgeType::CompanyPerson);
        assert_eq!(edge.source_id, src);
        assert_eq!(edge.target_id, tgt);
        assert_eq!(edge.weight, 1.0);
        assert_eq!(edge.edge_type, EdgeType::CompanyPerson);
    }

    #[test]
    fn test_certification_is_valid() {
        let company_id = Uuid::new_v4();
        let mut cert = Certification::new(company_id, "ISO_9001");
        assert!(cert.is_valid()); // active with no expiry => valid

        cert.valid_until = Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap());
        assert!(!cert.is_valid()); // expired date

        cert.valid_until = Some(NaiveDate::from_ymd_opt(2099, 12, 31).unwrap());
        assert!(cert.is_valid()); // future date

        cert.status = CertStatus::Suspended;
        assert!(!cert.is_valid()); // suspended
    }

    #[test]
    fn test_capability_new() {
        let company_id = Uuid::new_v4();
        let cap = Capability::new(company_id, "SMT", ProofGrade::A);
        assert_eq!(cap.capability, "SMT");
        assert_eq!(cap.proof_grade, ProofGrade::A);
    }

    #[test]
    fn test_poi_artifact_new() {
        let person_id = Uuid::new_v4();
        let now = Utc::now();
        let art = PoiArtifact::new(
            person_id,
            ArtifactType::Patent,
            "https://patents.google.com/patent/US12345",
            now,
        );
        assert_eq!(art.person_id, person_id);
        assert_eq!(art.artifact_type, ArtifactType::Patent);
        assert!(art.topics.is_empty());
    }

    #[test]
    fn test_logistics_node_new() {
        let node = LogisticsNode::new("Tanger Med", "port");
        assert_eq!(node.name, "Tanger Med");
        assert_eq!(node.node_type, "port");
    }

    #[test]
    fn test_feature_row_new() {
        let fr = FeatureRow::new("company-123", "company", 1700000000, 7);
        assert_eq!(fr.entity_id, "company-123");
        assert_eq!(fr.bucket_size_days, 7);
        assert!(fr.signal_counts.is_empty());
        assert!(fr.poi_pain_index.is_none());
    }

    // ── B270: SiteType / RoleFamily::Other uncovered branches ─────────────

    #[test]
    fn test_site_type_as_str_all_variants() {
        let cases = [
            (SiteType::Plant, "plant"),
            (SiteType::Warehouse, "warehouse"),
            (SiteType::Office, "office"),
            (SiteType::Lab, "lab"),
            (SiteType::Hq, "hq"),
        ];
        for (variant, expected) in &cases {
            assert_eq!(
                variant.as_str(),
                *expected,
                "SiteType::{:?} should be {expected}",
                variant
            );
        }
        // Other variant returns the inner string verbatim
        let custom = SiteType::Other("depot".to_string());
        assert_eq!(custom.as_str(), "depot");
    }

    #[test]
    fn test_role_family_other_variant_roundtrip() {
        // Unrecognised strings must produce Other(s) and survive as_str
        let rf = RoleFamily::from_str("custom_role");
        assert_eq!(rf, RoleFamily::Other("custom_role".to_string()));
        assert_eq!(rf.as_str(), "custom_role");

        // "legal" is now a named variant (unified with POI crate)
        assert_eq!(RoleFamily::from_str("legal"), RoleFamily::Legal);
        assert_eq!(RoleFamily::Legal.as_str(), "legal");

        // Known variants must NOT become Other
        assert_ne!(
            RoleFamily::from_str("finance"),
            RoleFamily::Other("finance".to_string())
        );
        assert_eq!(RoleFamily::Finance.as_str(), "finance");

        // New POI-originated variants
        assert_eq!(
            RoleFamily::from_str("supplier_quality"),
            RoleFamily::SupplierQuality
        );
        assert_eq!(
            RoleFamily::from_str("free_zone_authority"),
            RoleFamily::FreeZoneAuthority
        );
        assert_eq!(
            RoleFamily::from_str("port_logistics"),
            RoleFamily::PortLogistics
        );
        assert_eq!(
            RoleFamily::from_str("certification_body"),
            RoleFamily::CertificationBody
        );
        assert_eq!(
            RoleFamily::from_str("industry_association"),
            RoleFamily::IndustryAssociation
        );
        assert_eq!(RoleFamily::from_str("distributor"), RoleFamily::Distributor);
    }

    #[test]
    fn test_priority_vector_all_fields_matter_for_dominant() {
        // Security highest
        let pv = PriorityVector {
            cost: 0.1,
            quality: 0.2,
            speed: 0.3,
            resilience: 0.4,
            compliance: 0.5,
            security: 0.9,
        };
        assert_eq!(pv.dominant(), "security");
        // Cost highest (explicitly)
        let pv2 = PriorityVector {
            cost: 1.0,
            quality: 0.0,
            speed: 0.0,
            resilience: 0.0,
            compliance: 0.0,
            security: 0.0,
        };
        assert_eq!(pv2.dominant(), "cost");
        // Exact tie: first encountered (cost) wins
        let pv3 = PriorityVector::default(); // all zero
        assert_eq!(pv3.dominant(), "cost");
    }

    #[test]
    fn test_company_serialize_deserialize() {
        let c = Company::new("Test Corp", CompanyType::Oem);
        let json = serde_json::to_string(&c).unwrap();
        let c2: Company = serde_json::from_str(&json).unwrap();
        assert_eq!(c.id, c2.id);
        assert_eq!(c.name, c2.name);
    }

    #[test]
    fn test_observation_serialize_deserialize() {
        let obs = Observation::new(
            ObservationType::FxRate,
            Utc::now(),
            serde_json::json!({"pair": "EUR/USD", "rate": 1.0856}),
            serde_json::json!({"source": "ECB"}),
        );
        let json = serde_json::to_string(&obs).unwrap();
        let obs2: Observation = serde_json::from_str(&json).unwrap();
        assert_eq!(obs.id, obs2.id);
        assert_eq!(obs.observation_type, obs2.observation_type);
    }
}
