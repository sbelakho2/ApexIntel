use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ────────────────────────────────────────────
// Company
// ────────────────────────────────────────────

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
            name: name.into(),
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
            name: name.into(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RoleFamily {
    Procurement,
    Quality,
    Engineering,
    Operations,
    Security,
    Executive,
    Government,
    Logistics,
    Finance,
    Military,
    Intelligence,
    Other(String),
}

impl RoleFamily {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Procurement => "procurement",
            Self::Quality => "quality",
            Self::Engineering => "engineering",
            Self::Operations => "operations",
            Self::Security => "security",
            Self::Executive => "executive",
            Self::Government => "government",
            Self::Logistics => "logistics",
            Self::Finance => "finance",
            Self::Military => "military",
            Self::Intelligence => "intelligence",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "procurement" => Self::Procurement,
            "quality" => Self::Quality,
            "engineering" => Self::Engineering,
            "operations" => Self::Operations,
            "security" => Self::Security,
            "executive" => Self::Executive,
            "government" => Self::Government,
            "logistics" => Self::Logistics,
            "finance" => Self::Finance,
            "military" => Self::Military,
            "intelligence" => Self::Intelligence,
            other => Self::Other(other.to_string()),
        }
    }
}

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
        let mut best = pairs[0];
        for &pair in &pairs[1..] {
            if pair.1 > best.1 {
                best = pair;
            }
        }
        best.0
    }
}

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
            name: name.into(),
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
        }
    }

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
            _ => None,
        }
    }
}

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
        match self.status {
            CertStatus::Active => {
                if let Some(until) = self.valid_until {
                    until >= Utc::now().date_naive()
                } else {
                    true
                }
            }
            _ => false,
        }
    }
}

// ────────────────────────────────────────────
// Logistics Node
// ────────────────────────────────────────────

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
}

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
    pub fn new(
        company_id: Uuid,
        capability: impl Into<String>,
        proof_grade: ProofGrade,
    ) -> Self {
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
    pub fn new(entity_id: impl Into<String>, entity_type: impl Into<String>, time_bucket: i64, bucket_size_days: i32) -> Self {
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
