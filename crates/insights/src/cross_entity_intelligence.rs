//! # Cross-Entity Intelligence Module
//!
//! Advanced cross-entity analysis for ApexIntel OSINT platform.
//!
//! This module provides sophisticated intelligence capabilities:
//!
//! ## 2.2.1 Relationship Discovery
//! - Person-to-company linking (ownership, employment)
//! - Company-to-company linking (supplier, competitor, JV)
//! - Event-to-entity linking (tenders, certifications, news)
//! - Temporal relationship tracking
//!
//! ## 2.2.2 Network Analysis
//! - Supply chain graph analysis
//! - Leadership network mapping
//! - Financial relationship extraction
//! - Cross-border entity clustering
//!
//! ## 2.2.3 Correlation Engine
//! - Time-series correlation across entities
//! - Geographic clustering
//! - Industry vertical grouping
//! - Anomaly detection (deviation from patterns)
//!
//! ## 2.2.4 Intelligence Synthesis
//! - Cross-entity narrative generation
//! - Risk propagation modeling
//! - Opportunity identification
//! - Threat actor attribution

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum IntelligenceError {
    #[error("Entity not found: {0}")]
    EntityNotFound(String),

    #[error("Invalid relationship: {0}")]
    InvalidRelationship(String),

    #[error("Graph operation failed: {0}")]
    GraphError(String),

    #[error("Insufficient data: {0}")]
    InsufficientData(String),

    #[error("Analysis error: {0}")]
    AnalysisError(String),
}

// ============================================================================
// 2.2.1 Relationship Discovery
// ============================================================================

/// Types of entity relationships
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationshipKind {
    // Person-Company relationships
    /// Person is owner/stockholder of company
    Owner,
    /// Person is employee of company
    Employee,
    /// Person is executive/board member
    Executive,
    /// Person is founder of company
    Founder,
    /// Person is investor in company
    Investor,

    // Company-Company relationships
    /// Company is supplier of components/materials
    Supplier,
    /// Company is customer of products
    Customer,
    /// Companies are joint venture partners
    JointVenture,
    /// Companies are direct competitors
    Competitor,
    /// One company is subsidiary of another
    Subsidiary,
    /// Companies share common parent
    SisterCompany,
    /// Companies have licensed technology
    LicensedTo,
    /// Companies license technology from
    LicensedFrom,

    // Event-Entity relationships
    /// Company/Person participated in tender
    TenderParticipant,
    /// Company received certification
    Certified,
    /// Entity mentioned in news
    NewsSubject,
    /// Entity involved in regulatory action
    RegulatorySubject,

    // Temporal relationships
    /// Historical relationship (now ended)
    Former,
    /// Current active relationship
    Current,
    /// Pending/under negotiation
    Pending,
}

impl fmt::Display for RelationshipKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelationshipKind::Owner => write!(f, "Owner"),
            RelationshipKind::Employee => write!(f, "Employee"),
            RelationshipKind::Executive => write!(f, "Executive"),
            RelationshipKind::Founder => write!(f, "Founder"),
            RelationshipKind::Investor => write!(f, "Investor"),
            RelationshipKind::Supplier => write!(f, "Supplier"),
            RelationshipKind::Customer => write!(f, "Customer"),
            RelationshipKind::JointVenture => write!(f, "JointVenture"),
            RelationshipKind::Competitor => write!(f, "Competitor"),
            RelationshipKind::Subsidiary => write!(f, "Subsidiary"),
            RelationshipKind::SisterCompany => write!(f, "SisterCompany"),
            RelationshipKind::LicensedTo => write!(f, "LicensedTo"),
            RelationshipKind::LicensedFrom => write!(f, "LicensedFrom"),
            RelationshipKind::TenderParticipant => write!(f, "TenderParticipant"),
            RelationshipKind::Certified => write!(f, "Certified"),
            RelationshipKind::NewsSubject => write!(f, "NewsSubject"),
            RelationshipKind::RegulatorySubject => write!(f, "RegulatorySubject"),
            RelationshipKind::Former => write!(f, "Former"),
            RelationshipKind::Current => write!(f, "Current"),
            RelationshipKind::Pending => write!(f, "Pending"),
        }
    }
}

/// Category of entity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityCategory {
    Person,
    Company,
    Organization,
    Event,
    Location,
    Unknown,
}

impl fmt::Display for EntityCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityCategory::Person => write!(f, "Person"),
            EntityCategory::Company => write!(f, "Company"),
            EntityCategory::Organization => write!(f, "Organization"),
            EntityCategory::Event => write!(f, "Event"),
            EntityCategory::Location => write!(f, "Location"),
            EntityCategory::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Core entity representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub normalized_name: String,
    pub category: EntityCategory,
    pub metadata: HashMap<String, String>,
    pub aliases: Vec<String>,
    pub country_codes: Vec<String>,
    pub industry_codes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Entity {
    pub fn new(id: &str, name: &str, category: EntityCategory) -> Self {
        let normalized = name.to_lowercase();
        Self {
            id: id.to_string(),
            name: name.to_string(),
            normalized_name: normalized.clone(),
            category,
            metadata: HashMap::new(),
            aliases: vec![],
            country_codes: vec![],
            industry_codes: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn company(id: &str, name: &str) -> Self {
        Self::new(id, name, EntityCategory::Company)
    }

    pub fn person(id: &str, name: &str) -> Self {
        Self::new(id, name, EntityCategory::Person)
    }
}

/// Relationship between entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub kind: RelationshipKind,
    pub confidence: f64,
    pub evidence: Vec<RelationshipEvidence>,
    pub temporal: TemporalInfo,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipEvidence {
    pub source: String,
    pub source_type: EvidenceSourceType,
    pub snippet: String,
    pub url: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidenceSourceType {
    NewsArticle,
    RegulatoryFiling,
    CompanyWebsite,
    LinkedIn,
    Crunchbase,
    LinkedData,
    UserProvided,
    Inferred,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalInfo {
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub is_current: bool,
    pub is_estimated: bool,
}

impl TemporalInfo {
    pub fn current() -> Self {
        Self {
            start_date: None,
            end_date: None,
            is_current: true,
            is_estimated: false,
        }
    }

    pub fn historical(start: NaiveDate, end: NaiveDate) -> Self {
        Self {
            start_date: Some(start),
            end_date: Some(end),
            is_current: false,
            is_estimated: false,
        }
    }

    pub fn with_start(start: NaiveDate) -> Self {
        Self {
            start_date: Some(start),
            end_date: None,
            is_current: true,
            is_estimated: false,
        }
    }
}

/// Relationship discovery engine
pub struct RelationshipDiscovery {
    entities: HashMap<String, Entity>,
    relationships: Vec<Relationship>,
    entity_aliases: HashMap<String, HashSet<String>>,
}

impl Default for RelationshipDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

impl RelationshipDiscovery {
    pub fn new() -> Self {
        Self {
            entities: HashMap::new(),
            relationships: Vec::new(),
            entity_aliases: HashMap::new(),
        }
    }

    /// Register an entity
    pub fn register_entity(&mut self, entity: Entity) {
        let id = entity.id.clone();
        self.entities.insert(id, entity);
    }

    /// Register a relationship
    pub fn register_relationship(
        &mut self,
        relationship: Relationship,
    ) -> Result<(), IntelligenceError> {
        // Validate entities exist
        if !self.entities.contains_key(&relationship.source_id) {
            return Err(IntelligenceError::EntityNotFound(
                relationship.source_id.clone(),
            ));
        }
        if !self.entities.contains_key(&relationship.target_id) {
            return Err(IntelligenceError::EntityNotFound(
                relationship.target_id.clone(),
            ));
        }

        self.relationships.push(relationship);
        Ok(())
    }

    /// Add entity alias for resolution
    pub fn add_alias(&mut self, entity_id: &str, alias: &str) {
        self.entity_aliases
            .entry(entity_id.to_string())
            .or_default()
            .insert(alias.to_lowercase());
    }

    /// Resolve entity by name or alias
    pub fn resolve_entity(&self, name: &str) -> Option<&Entity> {
        let normalized = name.to_lowercase();

        // Direct lookup by normalized name
        for entity in self.entities.values() {
            if entity.normalized_name == normalized {
                return Some(entity);
            }
        }

        // Alias lookup
        for (entity_id, aliases) in &self.entity_aliases {
            if aliases.contains(&normalized) {
                return self.entities.get(entity_id);
            }
        }

        None
    }

    /// Link person to company relationship
    pub fn link_person_company(
        &mut self,
        person_id: &str,
        company_id: &str,
        role: RelationshipKind,
        evidence: Vec<RelationshipEvidence>,
        temporal: TemporalInfo,
    ) -> Result<String, IntelligenceError> {
        // Validate relationship kind
        let valid_roles = [
            RelationshipKind::Owner,
            RelationshipKind::Employee,
            RelationshipKind::Executive,
            RelationshipKind::Founder,
            RelationshipKind::Investor,
        ];
        if !valid_roles.contains(&role) {
            return Err(IntelligenceError::InvalidRelationship(format!(
                "Invalid person-company role: {:?}",
                role
            )));
        }

        let relationship = Relationship {
            id: Uuid::new_v4().to_string(),
            source_id: person_id.to_string(),
            target_id: company_id.to_string(),
            kind: role,
            confidence: 0.8,
            evidence,
            temporal,
            metadata: HashMap::new(),
        };

        let id = relationship.id.clone();
        self.register_relationship(relationship)?;
        Ok(id)
    }

    /// Link company to company relationship
    pub fn link_company_company(
        &mut self,
        company_a_id: &str,
        company_b_id: &str,
        kind: RelationshipKind,
        evidence: Vec<RelationshipEvidence>,
        temporal: TemporalInfo,
    ) -> Result<String, IntelligenceError> {
        // Validate relationship kind
        let valid_kinds = [
            RelationshipKind::Supplier,
            RelationshipKind::Customer,
            RelationshipKind::JointVenture,
            RelationshipKind::Competitor,
            RelationshipKind::Subsidiary,
            RelationshipKind::SisterCompany,
            RelationshipKind::LicensedTo,
            RelationshipKind::LicensedFrom,
        ];
        if !valid_kinds.contains(&kind) {
            return Err(IntelligenceError::InvalidRelationship(format!(
                "Invalid company-company kind: {:?}",
                kind
            )));
        }

        let relationship = Relationship {
            id: Uuid::new_v4().to_string(),
            source_id: company_a_id.to_string(),
            target_id: company_b_id.to_string(),
            kind,
            confidence: 0.8,
            evidence,
            temporal,
            metadata: HashMap::new(),
        };

        let id = relationship.id.clone();
        self.register_relationship(relationship)?;
        Ok(id)
    }

    /// Link event to entity
    pub fn link_event_entity(
        &mut self,
        event_id: &str,
        entity_id: &str,
        role: RelationshipKind,
        evidence: Vec<RelationshipEvidence>,
    ) -> Result<String, IntelligenceError> {
        let valid_roles = [
            RelationshipKind::TenderParticipant,
            RelationshipKind::Certified,
            RelationshipKind::NewsSubject,
            RelationshipKind::RegulatorySubject,
        ];
        if !valid_roles.contains(&role) {
            return Err(IntelligenceError::InvalidRelationship(format!(
                "Invalid event-entity role: {:?}",
                role
            )));
        }

        let relationship = Relationship {
            id: Uuid::new_v4().to_string(),
            source_id: event_id.to_string(),
            target_id: entity_id.to_string(),
            kind: role,
            confidence: 0.7,
            evidence,
            temporal: TemporalInfo::current(),
            metadata: HashMap::new(),
        };

        let id = relationship.id.clone();
        self.register_relationship(relationship)?;
        Ok(id)
    }

    /// Get all relationships for an entity
    pub fn get_relationships(&self, entity_id: &str) -> Vec<&Relationship> {
        self.relationships
            .iter()
            .filter(|r| r.source_id == entity_id || r.target_id == entity_id)
            .collect()
    }

    /// Get relationships by kind
    pub fn get_relationships_by_kind(&self, kind: RelationshipKind) -> Vec<&Relationship> {
        self.relationships
            .iter()
            .filter(|r| r.kind == kind)
            .collect()
    }

    /// Get entities in relationship with given entity
    pub fn get_related_entities(&self, entity_id: &str) -> Vec<(EntityRef, RelationshipKind)> {
        self.get_relationships(entity_id)
            .iter()
            .map(|r| {
                let other_id = if r.source_id == entity_id {
                    r.target_id.clone()
                } else {
                    r.source_id.clone()
                };
                let kind = r.kind;
                (EntityRef { id: other_id }, kind)
            })
            .collect()
    }

    /// Track temporal changes for an entity
    pub fn get_temporal_timeline(&self, entity_id: &str) -> TemporalTimeline {
        let mut timeline = TemporalTimeline {
            entity_id: entity_id.to_string(),
            events: Vec::new(),
        };

        for rel in self.get_relationships(entity_id) {
            timeline.events.push(TemporalEvent {
                date: rel.temporal.start_date.or(rel.temporal.end_date),
                event_type: rel.kind,
                related_entity: if rel.source_id == entity_id {
                    rel.target_id.clone()
                } else {
                    rel.source_id.clone()
                },
                is_start: rel.temporal.start_date.is_some(),
                is_end: rel.temporal.end_date.is_some(),
                is_current: rel.temporal.is_current,
            });
        }

        timeline.events.sort_by(|a, b| match (a.date, b.date) {
            (Some(d1), Some(d2)) => d1.cmp(&d2),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        timeline
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRef {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalTimeline {
    pub entity_id: String,
    pub events: Vec<TemporalEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalEvent {
    pub date: Option<NaiveDate>,
    pub event_type: RelationshipKind,
    pub related_entity: String,
    pub is_start: bool,
    pub is_end: bool,
    pub is_current: bool,
}

// ============================================================================
// 2.2.2 Network Analysis
// ============================================================================

/// Supply chain link with full context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainLink {
    pub supplier_id: String,
    pub customer_id: String,
    pub component_category: String,
    pub component_name: String,
    pub estimated_value_share: Option<f64>,
    pub contract_type: ContractType,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContractType {
    LongTerm,
    Spot,
    Framework,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Leadership position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadershipPosition {
    pub person_id: String,
    pub company_id: String,
    pub title: String,
    pub title_level: u8,
    pub is_board_member: bool,
    pub compensation: Option<CompensationInfo>,
    pub tenure_start: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompensationInfo {
    pub base_salary: Option<f64>,
    pub bonus: Option<f64>,
    pub equity_value: Option<f64>,
    pub currency: String,
}

/// Financial relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialRelationship {
    pub investor_id: String,
    pub investee_id: String,
    pub investment_type: InvestmentType,
    pub amount: Option<f64>,
    pub currency: String,
    pub round_type: Option<String>,
    pub valuation: Option<f64>,
    pub ownership_percentage: Option<f64>,
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InvestmentType {
    Equity,
    Debt,
    Grant,
    RevenueShare,
    ConvertibleNote,
    Unknown,
}

/// Cross-border clustering result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderCluster {
    pub cluster_id: String,
    pub countries: Vec<String>,
    pub entities: Vec<String>,
    pub cluster_type: ClusterType,
    pub strength: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClusterType {
    SupplyChain,
    Investment,
    Regulatory,
    Ownership,
    RegulatoryCapture,
}

/// Network analysis engine
pub struct NetworkAnalyzer {
    supply_chains: HashMap<String, Vec<SupplyChainLink>>,
    leadership: HashMap<String, Vec<LeadershipPosition>>,
    financials: Vec<FinancialRelationship>,
    border_clusters: Vec<BorderCluster>,
}

impl Default for NetworkAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkAnalyzer {
    pub fn new() -> Self {
        Self {
            supply_chains: HashMap::new(),
            leadership: HashMap::new(),
            financials: Vec::new(),
            border_clusters: Vec::new(),
        }
    }

    /// Add supply chain link
    pub fn add_supply_chain_link(&mut self, link: SupplyChainLink) {
        self.supply_chains
            .entry(link.customer_id.clone())
            .or_default()
            .push(link);
    }

    /// Get supply chain depth analysis
    pub fn analyze_supply_chain_depth(
        &self,
        company_id: &str,
        max_depth: usize,
    ) -> SupplyChainAnalysis {
        let mut suppliers: Vec<SupplierNode> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut risk_score = 0.0;

        self.collect_suppliers(
            company_id,
            &mut suppliers,
            &mut visited,
            0,
            max_depth,
            &mut risk_score,
        );

        SupplyChainAnalysis {
            company_id: company_id.to_string(),
            direct_supplier_count: suppliers.iter().filter(|s| s.depth == 1).count(),
            total_supplier_count: suppliers.len(),
            max_depth_reached: suppliers.iter().map(|s| s.depth).max().unwrap_or(0),
            risk_score,
            critical_suppliers: suppliers
                .iter()
                .filter(|s| s.is_critical)
                .cloned()
                .collect(),
            geographic_diversity: self.calculate_supplier_diversity(&suppliers),
        }
    }

    fn collect_suppliers(
        &self,
        company_id: &str,
        suppliers: &mut Vec<SupplierNode>,
        visited: &mut HashSet<String>,
        current_depth: usize,
        max_depth: usize,
        risk_score: &mut f64,
    ) {
        if current_depth >= max_depth || visited.contains(company_id) {
            return;
        }
        visited.insert(company_id.to_string());

        if let Some(links) = self.supply_chains.get(company_id) {
            for link in links {
                let node = SupplierNode {
                    supplier_id: link.supplier_id.clone(),
                    component: link.component_name.clone(),
                    depth: current_depth + 1,
                    is_critical: matches!(link.risk_level, RiskLevel::Critical | RiskLevel::High),
                    sole_source: links.len() == 1,
                };
                suppliers.push(node);

                if matches!(link.risk_level, RiskLevel::Critical | RiskLevel::High) {
                    *risk_score += 0.2;
                }

                self.collect_suppliers(
                    &link.supplier_id,
                    suppliers,
                    visited,
                    current_depth + 1,
                    max_depth,
                    risk_score,
                );
            }
        }
    }

    fn calculate_supplier_diversity(&self, suppliers: &[SupplierNode]) -> f64 {
        if suppliers.is_empty() {
            return 1.0;
        }

        // Simplified diversity calculation
        // In production, would consider actual country codes
        let unique_suppliers = suppliers
            .iter()
            .map(|s| &s.supplier_id)
            .collect::<HashSet<_>>()
            .len();
        (unique_suppliers as f64 / suppliers.len() as f64).min(1.0)
    }

    /// Add leadership position
    pub fn add_leadership_position(&mut self, position: LeadershipPosition) {
        self.leadership
            .entry(position.company_id.clone())
            .or_default()
            .push(position);
    }

    /// Map leadership network between companies
    pub fn map_leadership_network(&self, company_ids: &[String]) -> LeadershipNetwork {
        let mut connections: Vec<LeadershipConnection> = Vec::new();
        let mut person_companies: HashMap<String, Vec<String>> = HashMap::new();

        for company_id in company_ids {
            if let Some(positions) = self.leadership.get(company_id) {
                for pos in positions {
                    person_companies
                        .entry(pos.person_id.clone())
                        .or_default()
                        .push(company_id.clone());
                }
            }
        }

        for (person_id, companies) in &person_companies {
            if companies.len() > 1 {
                for i in 0..companies.len() {
                    for j in (i + 1)..companies.len() {
                        connections.push(LeadershipConnection {
                            person_id: person_id.clone(),
                            company_a: companies[i].clone(),
                            company_b: companies[j].clone(),
                            connection_type: LeadershipConnectionType::SharedBoardMember,
                        });
                    }
                }
            }
        }

        let mut company_connections: HashMap<(String, String), usize> = HashMap::new();
        for conn in &connections {
            let key = if conn.company_a < conn.company_b {
                (conn.company_a.clone(), conn.company_b.clone())
            } else {
                (conn.company_b.clone(), conn.company_a.clone())
            };
            *company_connections.entry(key).or_insert(0) += 1;
        }

        let shared_person_count = company_connections.len();
        let total_connections = connections.len();

        LeadershipNetwork {
            company_ids: company_ids.to_vec(),
            connections,
            shared_person_count,
            total_connections,
            network_density: if !company_ids.is_empty() {
                (total_connections as f64)
                    / (company_ids.len() as f64 * (company_ids.len() - 1) as f64 / 2.0)
            } else {
                0.0
            },
        }
    }

    /// Add financial relationship
    pub fn add_financial_relationship(&mut self, fr: FinancialRelationship) {
        self.financials.push(fr);
    }

    /// Extract financial relationship graph
    pub fn extract_financial_graph(&self) -> FinancialGraph {
        let mut nodes: HashMap<String, FinancialNode> = HashMap::new();
        let mut edges: Vec<FinancialEdge> = Vec::new();

        for fr in &self.financials {
            // Add investor node
            nodes
                .entry(fr.investor_id.clone())
                .or_insert_with(|| FinancialNode {
                    entity_id: fr.investor_id.clone(),
                    node_type: FinancialNodeType::Investor,
                    total_investments: 0.0,
                    total_investees: 0,
                });

            // Add investee node
            nodes
                .entry(fr.investee_id.clone())
                .or_insert_with(|| FinancialNode {
                    entity_id: fr.investee_id.clone(),
                    node_type: FinancialNodeType::Investee,
                    total_investments: 0.0,
                    total_investees: 0,
                });

            // Add edge
            if let Some(amount) = fr.amount {
                edges.push(FinancialEdge {
                    investor_id: fr.investor_id.clone(),
                    investee_id: fr.investee_id.clone(),
                    amount,
                    investment_type: fr.investment_type,
                    ownership_percentage: fr.ownership_percentage,
                });

                if let Some(node) = nodes.get_mut(&fr.investor_id) {
                    node.total_investments += amount;
                    node.total_investees += 1;
                }
            }
        }

        FinancialGraph {
            nodes: nodes.into_values().collect(),
            edges,
        }
    }

    /// Add border cluster
    pub fn add_border_cluster(&mut self, cluster: BorderCluster) {
        self.border_clusters.push(cluster);
    }

    /// Find cross-border clusters for an entity
    pub fn find_entity_clusters(&self, entity_id: &str) -> Vec<&BorderCluster> {
        self.border_clusters
            .iter()
            .filter(|c| c.entities.contains(&entity_id.to_string()))
            .collect()
    }

    /// Perform cross-border clustering analysis
    pub fn analyze_cross_border_clusters(&self, min_countries: usize) -> CrossBorderAnalysis {
        let relevant_clusters: Vec<&BorderCluster> = self
            .border_clusters
            .iter()
            .filter(|c| c.countries.len() >= min_countries)
            .collect();

        let mut by_type: HashMap<ClusterType, Vec<&BorderCluster>> = HashMap::new();
        for cluster in &relevant_clusters {
            by_type
                .entry(cluster.cluster_type)
                .or_default()
                .push(cluster);
        }

        CrossBorderAnalysis {
            total_clusters: relevant_clusters.len(),
            clusters_by_type: by_type.into_iter().map(|(k, v)| (k, v.len())).collect(),
            highest_strength: relevant_clusters
                .iter()
                .map(|c| c.strength)
                .fold(0.0, f64::max),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplierNode {
    pub supplier_id: String,
    pub component: String,
    pub depth: usize,
    pub is_critical: bool,
    pub sole_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupplyChainAnalysis {
    pub company_id: String,
    pub direct_supplier_count: usize,
    pub total_supplier_count: usize,
    pub max_depth_reached: usize,
    pub risk_score: f64,
    pub critical_suppliers: Vec<SupplierNode>,
    pub geographic_diversity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadershipConnection {
    pub person_id: String,
    pub company_a: String,
    pub company_b: String,
    pub connection_type: LeadershipConnectionType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LeadershipConnectionType {
    SharedBoardMember,
    SharedExecutive,
    SharedInvestor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadershipNetwork {
    pub company_ids: Vec<String>,
    pub connections: Vec<LeadershipConnection>,
    pub shared_person_count: usize,
    pub total_connections: usize,
    pub network_density: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialNode {
    pub entity_id: String,
    pub node_type: FinancialNodeType,
    pub total_investments: f64,
    pub total_investees: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FinancialNodeType {
    Investor,
    Investee,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialEdge {
    pub investor_id: String,
    pub investee_id: String,
    pub amount: f64,
    pub investment_type: InvestmentType,
    pub ownership_percentage: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialGraph {
    pub nodes: Vec<FinancialNode>,
    pub edges: Vec<FinancialEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossBorderAnalysis {
    pub total_clusters: usize,
    pub clusters_by_type: HashMap<ClusterType, usize>,
    pub highest_strength: f64,
}

// ============================================================================
// 2.2.3 Correlation Engine
// ============================================================================

/// Time series data point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
    pub entity_id: String,
    pub metric_type: String,
}

/// Geographic location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoLocation {
    pub entity_id: String,
    pub country_code: String,
    pub region: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// Industry classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndustryClassification {
    pub entity_id: String,
    pub industry_code: String,
    pub industry_name: String,
    pub level: u8,
    pub is_primary: bool,
}

/// Anomaly detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyResult {
    pub entity_id: String,
    pub metric_type: String,
    pub anomaly_type: AnomalyType,
    pub severity: AnomalySeverity,
    pub deviation_score: f64,
    pub description: String,
    pub detected_at: DateTime<Utc>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnomalyType {
    /// Value spike compared to history
    Spike,
    /// Unusual pattern break
    PatternBreak,
    /// Value below expected
    Drop,
    /// Geographic deviation
    GeographicShift,
    /// Industry change
    IndustryChange,
    /// Relationship anomaly
    RelationshipChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnomalySeverity {
    Low,
    Medium,
    High,
    Critical,
}

/// Correlation engine
pub struct CorrelationEngine {
    time_series: HashMap<String, Vec<TimeSeriesPoint>>,
    geo_locations: HashMap<String, GeoLocation>,
    industry_classes: HashMap<String, Vec<IndustryClassification>>,
    anomaly_history: Vec<AnomalyResult>,
}

impl Default for CorrelationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CorrelationEngine {
    pub fn new() -> Self {
        Self {
            time_series: HashMap::new(),
            geo_locations: HashMap::new(),
            industry_classes: HashMap::new(),
            anomaly_history: Vec::new(),
        }
    }

    /// Add time series data point
    pub fn add_time_series_point(&mut self, point: TimeSeriesPoint) {
        let key = format!("{}:{}", point.entity_id, point.metric_type);
        self.time_series.entry(key).or_default().push(point);
    }

    /// Perform time-series correlation between entities
    pub fn correlate_time_series(
        &self,
        entity_a: &str,
        entity_b: &str,
        metric_type: &str,
    ) -> CorrelationResult {
        let key_a = format!("{}:{}", entity_a, metric_type);
        let key_b = format!("{}:{}", entity_b, metric_type);

        let series_a = self.time_series.get(&key_a);
        let series_b = self.time_series.get(&key_b);

        match (series_a, series_b) {
            (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() => {
                let correlation = self.calculate_correlation(a, b);
                let lag = self.calculate_lag(a, b);
                let is_statistically_significant = correlation.abs() > 0.7 && a.len() >= 5;

                CorrelationResult {
                    entity_a: entity_a.to_string(),
                    entity_b: entity_b.to_string(),
                    metric_type: metric_type.to_string(),
                    correlation,
                    lag_hours: lag,
                    is_significant: is_statistically_significant,
                    sample_size: a.len().min(b.len()),
                    interpretation: self.interpret_correlation(correlation),
                }
            }
            _ => CorrelationResult {
                entity_a: entity_a.to_string(),
                entity_b: entity_b.to_string(),
                metric_type: metric_type.to_string(),
                correlation: 0.0,
                lag_hours: 0,
                is_significant: false,
                sample_size: 0,
                interpretation: "Insufficient data".to_string(),
            },
        }
    }

    fn calculate_correlation(&self, a: &[TimeSeriesPoint], b: &[TimeSeriesPoint]) -> f64 {
        // Simplified Pearson correlation
        // In production, would need proper time alignment
        let n = a.len().min(b.len()).min(100);
        if n < 2 {
            return 0.0;
        }

        let mean_a: f64 = a.iter().take(n).map(|p| p.value).sum::<f64>() / n as f64;
        let mean_b: f64 = b.iter().take(n).map(|p| p.value).sum::<f64>() / n as f64;

        let mut numerator = 0.0;
        let mut denom_a = 0.0;
        let mut denom_b = 0.0;

        for i in 0..n {
            let diff_a = a[i].value - mean_a;
            let diff_b = b[i].value - mean_b;
            numerator += diff_a * diff_b;
            denom_a += diff_a * diff_a;
            denom_b += diff_b * diff_b;
        }

        let denominator = (denom_a * denom_b).sqrt();
        if denominator == 0.0 {
            return 0.0;
        }

        (numerator / denominator).clamp(-1.0, 1.0)
    }

    fn calculate_lag(&self, _a: &[TimeSeriesPoint], _b: &[TimeSeriesPoint]) -> i64 {
        // Simplified lag calculation
        // Would use cross-correlation in production
        0
    }

    fn interpret_correlation(&self, corr: f64) -> String {
        let abs_corr = corr.abs();
        if abs_corr > 0.9 {
            if corr > 0.0 {
                "Very strong positive correlation - entities move together tightly".to_string()
            } else {
                "Very strong negative correlation - entities move in opposite directions"
                    .to_string()
            }
        } else if abs_corr > 0.7 {
            if corr > 0.0 {
                "Strong positive correlation".to_string()
            } else {
                "Strong negative correlation".to_string()
            }
        } else if abs_corr > 0.5 {
            if corr > 0.0 {
                "Moderate positive correlation".to_string()
            } else {
                "Moderate negative correlation".to_string()
            }
        } else if abs_corr > 0.3 {
            if corr > 0.0 {
                "Weak positive correlation".to_string()
            } else {
                "Weak negative correlation".to_string()
            }
        } else {
            "No significant correlation".to_string()
        }
    }

    /// Add geographic location
    pub fn add_geo_location(&mut self, location: GeoLocation) {
        self.geo_locations
            .insert(location.entity_id.clone(), location);
    }

    /// Perform geographic clustering
    pub fn cluster_by_geography(&self, min_entities: usize) -> Vec<GeoCluster> {
        let mut by_country: HashMap<String, Vec<String>> = HashMap::new();

        for (entity_id, loc) in &self.geo_locations {
            by_country
                .entry(loc.country_code.clone())
                .or_default()
                .push(entity_id.clone());
        }

        by_country
            .into_iter()
            .filter(|(_, entities)| entities.len() >= min_entities)
            .map(|(country, entities)| GeoCluster {
                country_code: country,
                entities,
                centroid: None,
            })
            .collect()
    }

    /// Add industry classification
    pub fn add_industry_classification(&mut self, classification: IndustryClassification) {
        self.industry_classes
            .entry(classification.entity_id.clone())
            .or_default()
            .push(classification);
    }

    /// Perform industry vertical grouping
    pub fn group_by_industry(&self) -> HashMap<String, IndustryGroup> {
        let mut groups: HashMap<String, IndustryGroup> = HashMap::new();

        for (entity_id, classes) in &self.industry_classes {
            for class in classes {
                groups
                    .entry(class.industry_code.clone())
                    .or_insert_with(|| IndustryGroup {
                        industry_code: class.industry_code.clone(),
                        industry_name: class.industry_name.clone(),
                        entities: Vec::new(),
                        primary_entity_count: 0,
                    })
                    .entities
                    .push(entity_id.clone());

                if class.is_primary {
                    if let Some(group) = groups.get_mut(&class.industry_code) {
                        group.primary_entity_count += 1;
                    }
                }
            }
        }

        groups
    }

    /// Detect anomalies
    pub fn detect_anomalies(&self, entity_id: &str, _lookback_days: u32) -> Vec<AnomalyResult> {
        let mut anomalies = Vec::new();

        // Get all time series for this entity
        let entity_series: Vec<(&String, &Vec<TimeSeriesPoint>)> = self
            .time_series
            .iter()
            .filter(|(key, _)| key.starts_with(&format!("{}:", entity_id)))
            .collect();

        for (key, series) in entity_series {
            let metric = key.split(':').nth(1).unwrap_or("");
            if series.len() >= 3 {
                if let Some(anomaly) = self.detect_statistical_anomaly(series, metric) {
                    anomalies.push(anomaly);
                }
            }
        }

        // Check for geographic anomalies
        if let Some(loc) = self.geo_locations.get(entity_id) {
            let same_country_count = self
                .geo_locations
                .values()
                .filter(|l| l.country_code == loc.country_code)
                .count();
            if same_country_count < 3 {
                anomalies.push(AnomalyResult {
                    entity_id: entity_id.to_string(),
                    metric_type: "geography".to_string(),
                    anomaly_type: AnomalyType::GeographicShift,
                    severity: AnomalySeverity::Medium,
                    deviation_score: 0.6,
                    description: format!(
                        "Entity located in {} with limited regional presence",
                        loc.country_code
                    ),
                    detected_at: Utc::now(),
                    confidence: 0.7,
                });
            }
        }

        anomalies
    }

    fn detect_statistical_anomaly(
        &self,
        series: &[TimeSeriesPoint],
        metric: &str,
    ) -> Option<AnomalyResult> {
        let values: Vec<f64> = series.iter().map(|p| p.value).collect();
        let n = values.len();

        if n < 3 {
            return None;
        }

        let mean: f64 = values.iter().sum::<f64>() / n as f64;
        let std_dev = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64).sqrt();

        if std_dev == 0.0 {
            return None;
        }

        // Check the most recent point
        let last = values.last()?;
        let z_score = (last - mean).abs() / std_dev;

        if z_score > 2.0 {
            let severity = if z_score > 3.0 {
                AnomalySeverity::Critical
            } else if z_score > 2.5 {
                AnomalySeverity::High
            } else {
                AnomalySeverity::Medium
            };

            return Some(AnomalyResult {
                entity_id: series[0].entity_id.clone(),
                metric_type: metric.to_string(),
                anomaly_type: if *last > mean {
                    AnomalyType::Spike
                } else {
                    AnomalyType::Drop
                },
                severity,
                deviation_score: z_score,
                description: format!(
                    "{} deviation from historical mean (z-score: {:.2})",
                    if *last > mean { "Spike" } else { "Drop" },
                    z_score
                ),
                detected_at: Utc::now(),
                confidence: (z_score / 4.0).min(0.99),
            });
        }

        None
    }

    /// Get recent anomaly history
    pub fn get_anomaly_history(&self, entity_id: &str) -> Vec<&AnomalyResult> {
        self.anomaly_history
            .iter()
            .filter(|a| a.entity_id == entity_id)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationResult {
    pub entity_a: String,
    pub entity_b: String,
    pub metric_type: String,
    pub correlation: f64,
    pub lag_hours: i64,
    pub is_significant: bool,
    pub sample_size: usize,
    pub interpretation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoCluster {
    pub country_code: String,
    pub entities: Vec<String>,
    pub centroid: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndustryGroup {
    pub industry_code: String,
    pub industry_name: String,
    pub entities: Vec<String>,
    pub primary_entity_count: usize,
}

// ============================================================================
// 2.2.4 Intelligence Synthesis
// ============================================================================

/// Synthesized intelligence report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelligenceReport {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub key_findings: Vec<KeyFinding>,
    pub risk_assessment: RiskAssessment,
    pub opportunities: Vec<Opportunity>,
    pub recommended_actions: Vec<RecommendedAction>,
    pub confidence: f64,
    pub generated_at: DateTime<Utc>,
    pub valid_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFinding {
    pub finding_id: String,
    pub title: String,
    pub description: String,
    pub evidence_strength: f64,
    pub related_entities: Vec<String>,
    pub category: FindingCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FindingCategory {
    SupplyChain,
    Leadership,
    Financial,
    Regulatory,
    Geopolitical,
    Competitive,
    Operational,
    Reputational,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub overall_risk_level: RiskLevel,
    pub risk_factors: Vec<RiskFactor>,
    pub risk_score: f64,
    pub risk_trend: RiskTrend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub factor_type: RiskFactorType,
    pub severity: RiskLevel,
    pub description: String,
    pub affected_entities: Vec<String>,
    pub mitigation_suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskFactorType {
    SupplyChainDisruption,
    KeyPersonDependency,
    FinancialExposure,
    RegulatoryRisk,
    GeopoliticalRisk,
    CompetitiveThreat,
    ReputationalRisk,
    TechnologyRisk,
    OperationalRisk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RiskTrend {
    Increasing,
    Stable,
    Decreasing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub opportunity_id: String,
    pub title: String,
    pub description: String,
    pub opportunity_type: OpportunityType,
    pub potential_value: Option<f64>,
    pub confidence: f64,
    pub related_entities: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OpportunityType {
    MarketExpansion,
    Partnership,
    Acquisition,
    Investment,
    SupplyChainOptimization,
    TechnologyAdoption,
    RegulatoryAdvantage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedAction {
    pub action_id: String,
    pub action: String,
    pub priority: ActionPriority,
    pub rationale: String,
    pub estimated_impact: Option<String>,
    pub related_findings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionPriority {
    Critical,
    High,
    Medium,
    Low,
}

/// Threat actor profile
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatActor {
    pub actor_id: String,
    pub name: String,
    pub actor_type: ThreatActorType,
    pub sophistication_level: SophisticationLevel,
    pub attributed_incidents: Vec<AttributedIncident>,
    pub associated_entities: Vec<String>,
    pub motivation: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThreatActorType {
    NationState,
    CyberCriminal,
    Hacktivist,
    Insider,
    Competitor,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SophisticationLevel {
    Low,
    Medium,
    High,
    Advanced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributedIncident {
    pub incident_id: String,
    pub date: NaiveDate,
    pub incident_type: String,
    pub target_entities: Vec<String>,
    pub impact_description: String,
    pub attribution_confidence: f64,
}

/// Intelligence synthesis engine
pub struct IntelligenceSynthesizer {
    relationship_discovery: RelationshipDiscovery,
    network_analyzer: NetworkAnalyzer,
    correlation_engine: CorrelationEngine,
}

impl Default for IntelligenceSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

impl IntelligenceSynthesizer {
    pub fn new() -> Self {
        Self {
            relationship_discovery: RelationshipDiscovery::new(),
            network_analyzer: NetworkAnalyzer::new(),
            correlation_engine: CorrelationEngine::new(),
        }
    }

    /// Generate cross-entity narrative
    pub fn generate_narrative(&self, entity_ids: &[String]) -> NarrativeResult {
        let mut narratives: Vec<EntityNarrative> = Vec::new();

        for entity_id in entity_ids {
            let relationships = self.relationship_discovery.get_relationships(entity_id);
            let clusters = self.network_analyzer.find_entity_clusters(entity_id);
            let anomalies = self.correlation_engine.detect_anomalies(entity_id, 30);

            let narrative =
                self.build_entity_narrative(entity_id, &relationships, &clusters, &anomalies);
            narratives.push(narrative);
        }

        // Cross-entity narrative
        let cross_entity = self.build_cross_entity_narrative(entity_ids, &narratives);

        NarrativeResult {
            narratives,
            cross_entity_summary: cross_entity,
        }
    }

    fn build_entity_narrative(
        &self,
        entity_id: &str,
        relationships: &[&Relationship],
        clusters: &[&BorderCluster],
        anomalies: &[AnomalyResult],
    ) -> EntityNarrative {
        let mut narrative = EntityNarrative {
            entity_id: entity_id.to_string(),
            summary: String::new(),
            relationship_summary: String::new(),
            risk_summary: String::new(),
            opportunities_summary: String::new(),
            key_events: Vec::new(),
            derived_insights: Vec::new(),
        };

        // Summarize relationships
        let mut relationship_counts: HashMap<RelationshipKind, usize> = HashMap::new();
        for rel in relationships {
            *relationship_counts.entry(rel.kind).or_insert(0) += 1;
        }

        let rel_summary_parts: Vec<String> = relationship_counts
            .iter()
            .map(|(kind, count)| format!("{}: {}", kind, count))
            .collect();
        narrative.relationship_summary = if rel_summary_parts.is_empty() {
            "No known relationships".to_string()
        } else {
            rel_summary_parts.join(", ")
        };

        // Summarize risks from anomalies
        let critical_anomalies = anomalies
            .iter()
            .filter(|a| {
                matches!(
                    a.severity,
                    AnomalySeverity::High | AnomalySeverity::Critical
                )
            })
            .count();

        narrative.risk_summary = if anomalies.is_empty() {
            "No anomalies detected".to_string()
        } else if critical_anomalies > 0 {
            format!(
                "{} critical anomalies detected requiring attention",
                critical_anomalies
            )
        } else {
            format!("{} minor anomalies detected", anomalies.len())
        };

        // Generate derived insights
        if !clusters.is_empty() {
            narrative.derived_insights.push(format!(
                "Entity participates in {} cross-border clusters",
                clusters.len()
            ));
        }

        if relationships.len() > 5 {
            narrative
                .derived_insights
                .push("Entity has extensive network of relationships".to_string());
        }

        if critical_anomalies > 0 {
            narrative
                .derived_insights
                .push("Entity exhibits anomalous behavior patterns".to_string());
        }

        narrative.summary = format!(
            "Entity {} has {} relationships and {} detected anomalies",
            entity_id,
            relationships.len(),
            anomalies.len()
        );

        narrative
    }

    fn build_cross_entity_narrative(
        &self,
        entity_ids: &[String],
        narratives: &[EntityNarrative],
    ) -> String {
        if entity_ids.is_empty() {
            return "No entities to analyze".to_string();
        }

        let total_relationships: usize = narratives
            .iter()
            .map(|n| {
                let parts: Vec<&str> = n
                    .relationship_summary
                    .split(", ")
                    .filter(|s| s.contains(':'))
                    .collect();
                parts.len()
            })
            .sum();

        let total_anomalies = narratives
            .iter()
            .filter(|n| n.risk_summary.contains("critical"))
            .count();

        format!(
            "Analysis of {} entities reveals {} total relationship connections and {} critical anomalies requiring investigation.",
            entity_ids.len(),
            total_relationships,
            total_anomalies
        )
    }

    /// Model risk propagation
    pub fn model_risk_propagation(
        &self,
        source_entity_id: &str,
        risk_type: RiskFactorType,
    ) -> RiskPropagationResult {
        let mut propagation_chain: Vec<PropagatedRisk> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();

        self.propagate_risk(
            source_entity_id,
            risk_type,
            &mut visited,
            &mut propagation_chain,
            0,
            3,
        );

        let affected_entities: Vec<String> = propagation_chain
            .iter()
            .map(|r| r.entity_id.clone())
            .collect();
        let max_severity =
            propagation_chain
                .iter()
                .map(|r| r.severity)
                .fold(RiskLevel::Low, |acc, s| {
                    let acc_val = match acc {
                        RiskLevel::Low => 0,
                        RiskLevel::Medium => 1,
                        RiskLevel::High => 2,
                        RiskLevel::Critical => 3,
                    };
                    let s_val = match s {
                        RiskLevel::Low => 0,
                        RiskLevel::Medium => 1,
                        RiskLevel::High => 2,
                        RiskLevel::Critical => 3,
                    };
                    if s_val > acc_val {
                        s
                    } else {
                        acc
                    }
                });

        RiskPropagationResult {
            source_entity: source_entity_id.to_string(),
            risk_type,
            propagation_chain,
            total_affected: affected_entities.len(),
            max_severity,
            confidence: 0.8,
        }
    }

    fn propagate_risk(
        &self,
        entity_id: &str,
        _risk_type: RiskFactorType,
        visited: &mut HashSet<String>,
        chain: &mut Vec<PropagatedRisk>,
        depth: usize,
        max_depth: usize,
    ) {
        if depth >= max_depth || visited.contains(entity_id) {
            return;
        }
        visited.insert(entity_id.to_string());

        let relationships = self.relationship_discovery.get_relationships(entity_id);

        for rel in relationships {
            let next_entity = if rel.source_id == entity_id {
                rel.target_id.clone()
            } else {
                rel.source_id.clone()
            };

            if !visited.contains(&next_entity) {
                chain.push(PropagatedRisk {
                    entity_id: next_entity.clone(),
                    source_entity: entity_id.to_string(),
                    relationship: rel.kind,
                    severity: self.assess_propagated_severity(&rel.kind, depth as u8),
                    depth: depth + 1,
                    propagation_path: vec![],
                });

                self.propagate_risk(
                    &next_entity,
                    _risk_type,
                    visited,
                    chain,
                    depth + 1,
                    max_depth,
                );
            }
        }
    }

    fn assess_propagated_severity(&self, kind: &RelationshipKind, depth: u8) -> RiskLevel {
        let base_risk = match kind {
            RelationshipKind::Supplier => RiskLevel::Medium,
            RelationshipKind::Customer => RiskLevel::Medium,
            RelationshipKind::Subsidiary => RiskLevel::High,
            RelationshipKind::Executive => RiskLevel::High,
            RelationshipKind::Owner => RiskLevel::Critical,
            _ => RiskLevel::Low,
        };

        // Reduce risk with depth
        match depth {
            0 => base_risk,
            1 => match base_risk {
                RiskLevel::Critical => RiskLevel::High,
                _ => RiskLevel::Medium,
            },
            _ => RiskLevel::Low,
        }
    }

    /// Identify opportunities
    pub fn identify_opportunities(&self, entity_ids: &[String]) -> Vec<Opportunity> {
        let mut opportunities = Vec::new();

        for entity_id in entity_ids {
            let relationships = self.relationship_discovery.get_relationships(entity_id);

            // Look for partnership opportunities
            let has_supplier = relationships
                .iter()
                .any(|r| matches!(r.kind, RelationshipKind::Supplier) && r.temporal.is_current);
            let has_customer = relationships
                .iter()
                .any(|r| matches!(r.kind, RelationshipKind::Customer) && r.temporal.is_current);

            if has_supplier && has_customer {
                opportunities.push(Opportunity {
                    opportunity_id: Uuid::new_v4().to_string(),
                    title: "Supply Chain Integration Potential".to_string(),
                    description: format!(
                        "Entity {} shows supply chain integration potential with existing supplier and customer relationships",
                        entity_id
                    ),
                    opportunity_type: OpportunityType::SupplyChainOptimization,
                    potential_value: None,
                    confidence: 0.75,
                    related_entities: vec![entity_id.to_string()],
                });
            }

            // Look for expansion opportunities based on network analysis
            let clusters = self.network_analyzer.find_entity_clusters(entity_id);
            if clusters.len() > 2 {
                opportunities.push(Opportunity {
                    opportunity_id: Uuid::new_v4().to_string(),
                    title: "Cross-Border Market Expansion".to_string(),
                    description: format!(
                        "Entity {} participates in {} cross-border clusters suggesting expansion potential",
                        entity_id,
                        clusters.len()
                    ),
                    opportunity_type: OpportunityType::MarketExpansion,
                    potential_value: None,
                    confidence: 0.7,
                    related_entities: vec![entity_id.to_string()],
                });
            }
        }

        // Remove duplicates
        opportunities.sort_by(|a, b| a.title.cmp(&b.title));
        opportunities.dedup_by(|a, b| a.title == b.title);

        opportunities
    }

    /// Attribute threat actors
    pub fn attribute_threat_actors(
        &self,
        incident_patterns: &[IncidentPattern],
    ) -> Vec<ThreatActor> {
        let mut attributed: Vec<ThreatActor> = Vec::new();

        // Pattern-based attribution (simplified)
        for pattern in incident_patterns {
            let actor_type = self.infer_actor_type(pattern);
            if let Some(actor_type) = actor_type {
                attributed.push(ThreatActor {
                    actor_id: Uuid::new_v4().to_string(),
                    name: format!("Attributed Actor - {:?}", actor_type),
                    actor_type,
                    sophistication_level: SophisticationLevel::Medium,
                    attributed_incidents: vec![AttributedIncident {
                        incident_id: pattern.incident_id.clone(),
                        date: pattern.date,
                        incident_type: pattern.incident_type.clone(),
                        target_entities: pattern.target_entities.clone(),
                        impact_description: pattern.description.clone(),
                        attribution_confidence: pattern.confidence,
                    }],
                    associated_entities: Vec::new(),
                    motivation: self.infer_motivation(&actor_type),
                    confidence: pattern.confidence,
                });
            }
        }

        attributed
    }

    fn infer_actor_type(&self, pattern: &IncidentPattern) -> Option<ThreatActorType> {
        if pattern.confidence < 0.5 {
            return None;
        }

        match pattern.incident_type.to_lowercase().as_str() {
            t if t.contains("cyber") || t.contains("apt") => Some(ThreatActorType::NationState),
            t if t.contains("ransomware") || t.contains("financial") => {
                Some(ThreatActorType::CyberCriminal)
            }
            t if t.contains("hacktivist") || t.contains("protest") => {
                Some(ThreatActorType::Hacktivist)
            }
            t if t.contains("insider") || t.contains("employee") => Some(ThreatActorType::Insider),
            t if t.contains("competitive") || t.contains("corporate") => {
                Some(ThreatActorType::Competitor)
            }
            _ => None,
        }
    }

    fn infer_motivation(&self, actor_type: &ThreatActorType) -> String {
        match actor_type {
            ThreatActorType::NationState => {
                "Strategic intelligence gathering, disruption".to_string()
            }
            ThreatActorType::CyberCriminal => "Financial gain".to_string(),
            ThreatActorType::Hacktivist => "Ideological activism, publicity".to_string(),
            ThreatActorType::Insider => {
                "Personal grievance, financial incentive, coercion".to_string()
            }
            ThreatActorType::Competitor => "Market advantage, competitive intelligence".to_string(),
            ThreatActorType::Unknown => "Unknown motivation".to_string(),
        }
    }

    /// Generate comprehensive intelligence report
    pub fn generate_report(
        &self,
        entity_ids: &[String],
    ) -> Result<IntelligenceReport, IntelligenceError> {
        if entity_ids.is_empty() {
            return Err(IntelligenceError::InsufficientData(
                "No entities provided".to_string(),
            ));
        }

        let narrative_result = self.generate_narrative(entity_ids);
        let opportunities = self.identify_opportunities(entity_ids);

        // Build risk assessment
        let mut all_anomalies: Vec<AnomalyResult> = Vec::new();
        for entity_id in entity_ids {
            let anomalies = self.correlation_engine.detect_anomalies(entity_id, 30);
            all_anomalies.extend(anomalies);
        }

        let critical_count = all_anomalies
            .iter()
            .filter(|a| {
                matches!(
                    a.severity,
                    AnomalySeverity::High | AnomalySeverity::Critical
                )
            })
            .count();

        let overall_risk = if critical_count > 5 {
            RiskLevel::Critical
        } else if critical_count > 2 {
            RiskLevel::High
        } else if critical_count > 0 {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        };

        let risk_factors: Vec<RiskFactor> = all_anomalies
            .iter()
            .filter(|a| {
                matches!(
                    a.severity,
                    AnomalySeverity::High | AnomalySeverity::Critical
                )
            })
            .map(|a| RiskFactor {
                factor_type: match a.anomaly_type {
                    AnomalyType::Spike | AnomalyType::Drop => RiskFactorType::OperationalRisk,
                    AnomalyType::GeographicShift => RiskFactorType::GeopoliticalRisk,
                    AnomalyType::PatternBreak => RiskFactorType::CompetitiveThreat,
                    _ => RiskFactorType::TechnologyRisk,
                },
                severity: match a.severity {
                    AnomalySeverity::Critical => RiskLevel::Critical,
                    AnomalySeverity::High => RiskLevel::High,
                    _ => RiskLevel::Medium,
                },
                description: a.description.clone(),
                affected_entities: vec![a.entity_id.clone()],
                mitigation_suggestion: None,
            })
            .collect();

        let risk_score = (critical_count as f64 * 0.2).min(1.0);

        // Build key findings
        let key_findings: Vec<KeyFinding> = narrative_result
            .narratives
            .iter()
            .enumerate()
            .map(|(i, n)| KeyFinding {
                finding_id: format!("FINDING-{:03}", i + 1),
                title: format!("Entity {} Analysis", n.entity_id),
                description: n.summary.clone(),
                evidence_strength: 0.8,
                related_entities: vec![n.entity_id.clone()],
                category: FindingCategory::Operational,
            })
            .collect();

        // Build recommended actions
        let recommended_actions: Vec<RecommendedAction> = if critical_count > 0 {
            vec![
                RecommendedAction {
                    action_id: "ACT-001".to_string(),
                    action: "Investigate detected anomalies".to_string(),
                    priority: ActionPriority::Critical,
                    rationale: format!(
                        "{} critical anomalies require immediate attention",
                        critical_count
                    ),
                    estimated_impact: Some("High".to_string()),
                    related_findings: vec!["FINDING-001".to_string()],
                },
                RecommendedAction {
                    action_id: "ACT-002".to_string(),
                    action: "Review relationship network".to_string(),
                    priority: ActionPriority::High,
                    rationale: "Entity relationships may indicate hidden risks".to_string(),
                    estimated_impact: Some("Medium".to_string()),
                    related_findings: vec!["FINDING-001".to_string()],
                },
            ]
        } else {
            vec![RecommendedAction {
                action_id: "ACT-001".to_string(),
                action: "Continue monitoring".to_string(),
                priority: ActionPriority::Low,
                rationale: "No critical risks detected".to_string(),
                estimated_impact: None,
                related_findings: Vec::new(),
            }]
        };

        Ok(IntelligenceReport {
            id: Uuid::new_v4().to_string(),
            title: format!(
                "Cross-Entity Intelligence Report - {} Entities",
                entity_ids.len()
            ),
            summary: narrative_result.cross_entity_summary.clone(),
            key_findings,
            risk_assessment: RiskAssessment {
                overall_risk_level: overall_risk,
                risk_factors,
                risk_score,
                risk_trend: RiskTrend::Unknown,
            },
            opportunities,
            recommended_actions,
            confidence: 0.75,
            generated_at: Utc::now(),
            valid_until: Some(Utc::now() + chrono::Duration::days(7)),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityNarrative {
    pub entity_id: String,
    pub summary: String,
    pub relationship_summary: String,
    pub risk_summary: String,
    pub opportunities_summary: String,
    pub key_events: Vec<String>,
    pub derived_insights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeResult {
    pub narratives: Vec<EntityNarrative>,
    pub cross_entity_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropagatedRisk {
    pub entity_id: String,
    pub source_entity: String,
    pub relationship: RelationshipKind,
    pub severity: RiskLevel,
    pub depth: usize,
    pub propagation_path: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskPropagationResult {
    pub source_entity: String,
    pub risk_type: RiskFactorType,
    pub propagation_chain: Vec<PropagatedRisk>,
    pub total_affected: usize,
    pub max_severity: RiskLevel,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentPattern {
    pub incident_id: String,
    pub date: NaiveDate,
    pub incident_type: String,
    pub target_entities: Vec<String>,
    pub description: String,
    pub confidence: f64,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn test_entity_creation() {
        let entity = Entity::company("ent-001", "Acme Corporation");
        assert_eq!(entity.name, "Acme Corporation");
        assert_eq!(entity.category, EntityCategory::Company);
        assert_eq!(entity.normalized_name, "acme corporation");
    }

    #[test]
    fn test_relationship_discovery_registration() {
        let mut discovery = RelationshipDiscovery::new();

        let person = Entity::person("p-001", "John Smith");
        let company = Entity::company("c-001", "Acme Corp");

        discovery.register_entity(person);
        discovery.register_entity(company);

        let evidence = vec![RelationshipEvidence {
            source: "LinkedIn".to_string(),
            source_type: EvidenceSourceType::LinkedIn,
            snippet: "John Smith is CEO of Acme Corp".to_string(),
            url: None,
            timestamp: Utc::now(),
            weight: 0.9,
        }];

        let result = discovery.link_person_company(
            "p-001",
            "c-001",
            RelationshipKind::Executive,
            evidence,
            TemporalInfo::current(),
        );

        assert!(result.is_ok());

        let relationships = discovery.get_relationships("p-001");
        assert_eq!(relationships.len(), 1);
        assert_eq!(relationships[0].kind, RelationshipKind::Executive);
    }

    #[test]
    fn test_relationship_discovery_invalid_role() {
        let mut discovery = RelationshipDiscovery::new();

        let person = Entity::person("p-001", "John Smith");
        let company = Entity::company("c-001", "Acme Corp");

        discovery.register_entity(person);
        discovery.register_entity(company);

        let result = discovery.link_person_company(
            "p-001",
            "c-001",
            RelationshipKind::Supplier, // Invalid for person-company
            vec![],
            TemporalInfo::current(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_temporal_timeline() {
        let mut discovery = RelationshipDiscovery::new();

        let company = Entity::company("c-001", "Acme Corp");
        let company2 = Entity::company("c-002", "Partner Corp");

        discovery.register_entity(company.clone());
        discovery.register_entity(company2.clone());

        // Add current relationship
        discovery
            .link_company_company(
                "c-001",
                "c-002",
                RelationshipKind::Supplier,
                vec![],
                TemporalInfo::current(),
            )
            .unwrap();

        let timeline = discovery.get_temporal_timeline("c-001");
        assert!(!timeline.events.is_empty());
    }

    #[test]
    fn test_network_analyzer_supply_chain() {
        let mut analyzer = NetworkAnalyzer::new();

        analyzer.add_supply_chain_link(SupplyChainLink {
            supplier_id: "s-001".to_string(),
            customer_id: "c-001".to_string(),
            component_category: "Electronics".to_string(),
            component_name: "GPU".to_string(),
            estimated_value_share: Some(0.3),
            contract_type: ContractType::LongTerm,
            risk_level: RiskLevel::High,
        });

        let analysis = analyzer.analyze_supply_chain_depth("c-001", 3);
        assert_eq!(analysis.direct_supplier_count, 1);
        assert!(analysis.risk_score > 0.0);
    }

    #[test]
    fn test_network_analyzer_leadership_network() {
        let mut analyzer = NetworkAnalyzer::new();

        analyzer.add_leadership_position(LeadershipPosition {
            person_id: "p-001".to_string(),
            company_id: "c-001".to_string(),
            title: "CEO".to_string(),
            title_level: 1,
            is_board_member: true,
            compensation: None,
            tenure_start: None,
        });

        analyzer.add_leadership_position(LeadershipPosition {
            person_id: "p-001".to_string(),
            company_id: "c-002".to_string(),
            title: "Board Member".to_string(),
            title_level: 2,
            is_board_member: true,
            compensation: None,
            tenure_start: None,
        });

        let network = analyzer.map_leadership_network(&["c-001".to_string(), "c-002".to_string()]);
        assert_eq!(network.shared_person_count, 1);
    }

    #[test]
    fn test_correlation_engine_time_series() {
        let mut engine = CorrelationEngine::new();

        let now = Utc::now();
        for i in 0..10 {
            engine.add_time_series_point(TimeSeriesPoint {
                timestamp: now + chrono::Duration::hours(i),
                value: 100.0 + i as f64 * 10.0,
                entity_id: "e-001".to_string(),
                metric_type: "revenue".to_string(),
            });

            engine.add_time_series_point(TimeSeriesPoint {
                timestamp: now + chrono::Duration::hours(i),
                value: 50.0 + i as f64 * 5.0,
                entity_id: "e-002".to_string(),
                metric_type: "revenue".to_string(),
            });
        }

        let result = engine.correlate_time_series("e-001", "e-002", "revenue");
        assert!(result.is_significant);
        assert!(result.correlation > 0.9);
    }

    #[test]
    fn test_correlation_engine_geographic_clustering() {
        let mut engine = CorrelationEngine::new();

        engine.add_geo_location(GeoLocation {
            entity_id: "e-001".to_string(),
            country_code: "US".to_string(),
            region: Some("California".to_string()),
            city: None,
            latitude: None,
            longitude: None,
        });

        engine.add_geo_location(GeoLocation {
            entity_id: "e-002".to_string(),
            country_code: "US".to_string(),
            region: Some("Texas".to_string()),
            city: None,
            latitude: None,
            longitude: None,
        });

        engine.add_geo_location(GeoLocation {
            entity_id: "e-003".to_string(),
            country_code: "DE".to_string(),
            region: None,
            city: None,
            latitude: None,
            longitude: None,
        });

        let clusters = engine.cluster_by_geography(2);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].country_code, "US");
    }

    #[test]
    fn test_correlation_engine_anomaly_detection() {
        let mut engine = CorrelationEngine::new();

        let now = Utc::now();
        // Normal values around 100
        for i in 0..10 {
            engine.add_time_series_point(TimeSeriesPoint {
                timestamp: now + chrono::Duration::hours(i),
                value: 100.0 + (i % 2) as f64 * 5.0,
                entity_id: "e-001".to_string(),
                metric_type: "price".to_string(),
            });
        }
        // Add spike at end
        engine.add_time_series_point(TimeSeriesPoint {
            timestamp: now + chrono::Duration::hours(10),
            value: 300.0, // Significant spike
            entity_id: "e-001".to_string(),
            metric_type: "price".to_string(),
        });

        let anomalies = engine.detect_anomalies("e-001", 30);
        assert!(!anomalies.is_empty());
        assert_eq!(anomalies[0].anomaly_type, AnomalyType::Spike);
    }

    #[test]
    fn test_intelligence_synthesizer_narrative_generation() {
        let synthesizer = IntelligenceSynthesizer::new();

        let narratives = synthesizer.generate_narrative(&["e-001".to_string()]);
        assert_eq!(narratives.narratives.len(), 1);
    }

    #[test]
    fn test_intelligence_synthesizer_opportunity_identification() {
        let mut synthesizer = IntelligenceSynthesizer::new();

        // Add supplier and customer relationships
        let company1 = Entity::company("c-001", "Manufacturer");
        let company2 = Entity::company("c-002", "Supplier");
        let company3 = Entity::company("c-003", "Customer");

        synthesizer
            .relationship_discovery
            .register_entity(company1.clone());
        synthesizer
            .relationship_discovery
            .register_entity(company2.clone());
        synthesizer
            .relationship_discovery
            .register_entity(company3.clone());

        synthesizer
            .relationship_discovery
            .link_company_company(
                "c-002",
                "c-001",
                RelationshipKind::Supplier,
                vec![],
                TemporalInfo::current(),
            )
            .unwrap();

        synthesizer
            .relationship_discovery
            .link_company_company(
                "c-001",
                "c-003",
                RelationshipKind::Customer,
                vec![],
                TemporalInfo::current(),
            )
            .unwrap();

        let opportunities = synthesizer.identify_opportunities(&["c-001".to_string()]);
        assert!(!opportunities.is_empty());
    }

    #[test]
    fn test_intelligence_synthesizer_threat_attribution() {
        let synthesizer = IntelligenceSynthesizer::new();

        let patterns = vec![
            IncidentPattern {
                incident_id: "INC-001".to_string(),
                date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
                incident_type: "Cyber Attack".to_string(),
                target_entities: vec!["e-001".to_string()],
                description: "APT-style intrusion detected".to_string(),
                confidence: 0.8,
            },
            IncidentPattern {
                incident_id: "INC-002".to_string(),
                date: NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
                incident_type: "Ransomware".to_string(),
                target_entities: vec!["e-002".to_string()],
                description: "Financial motivation evident".to_string(),
                confidence: 0.7,
            },
        ];

        let actors = synthesizer.attribute_threat_actors(&patterns);
        assert_eq!(actors.len(), 2);
    }

    #[test]
    fn test_intelligence_synthesizer_report_generation() {
        let synthesizer = IntelligenceSynthesizer::new();

        let report = synthesizer.generate_report(&["e-001".to_string()]);
        assert!(report.is_ok());

        let report = report.unwrap();
        assert!(!report.title.is_empty());
        assert!(!report.key_findings.is_empty());
    }

    #[test]
    fn test_risk_propagation() {
        let mut synthesizer = IntelligenceSynthesizer::new();

        // Set up a simple network
        let c1 = Entity::company("c-001", "Main Corp");
        let c2 = Entity::company("c-002", "Supplier");
        let c3 = Entity::company("c-003", "Sub-supplier");

        synthesizer.relationship_discovery.register_entity(c1);
        synthesizer.relationship_discovery.register_entity(c2);
        synthesizer.relationship_discovery.register_entity(c3);

        synthesizer
            .relationship_discovery
            .link_company_company(
                "c-002",
                "c-001",
                RelationshipKind::Supplier,
                vec![],
                TemporalInfo::current(),
            )
            .unwrap();

        synthesizer
            .relationship_discovery
            .link_company_company(
                "c-003",
                "c-002",
                RelationshipKind::Supplier,
                vec![],
                TemporalInfo::current(),
            )
            .unwrap();

        let result =
            synthesizer.model_risk_propagation("c-001", RiskFactorType::SupplyChainDisruption);

        assert_eq!(result.source_entity, "c-001");
        assert!(result.total_affected >= 1);
    }

    #[test]
    fn test_temporal_info_variants() {
        let current = TemporalInfo::current();
        assert!(current.is_current);
        assert!(current.start_date.is_none());

        let historical = TemporalInfo::historical(
            NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2023, 12, 31).unwrap(),
        );
        assert!(!historical.is_current);
        assert!(historical.start_date.is_some());
        assert!(historical.end_date.is_some());
    }

    #[test]
    fn test_border_cluster_analysis() {
        let mut analyzer = NetworkAnalyzer::new();

        analyzer.add_border_cluster(BorderCluster {
            cluster_id: "cl-001".to_string(),
            countries: vec!["US".to_string(), "DE".to_string(), "JP".to_string()],
            entities: vec![
                "e-001".to_string(),
                "e-002".to_string(),
                "e-003".to_string(),
            ],
            cluster_type: ClusterType::SupplyChain,
            strength: 0.9,
        });

        let analysis = analyzer.analyze_cross_border_clusters(2);
        assert_eq!(analysis.total_clusters, 1);
    }

    #[test]
    fn test_financial_graph_extraction() {
        let mut analyzer = NetworkAnalyzer::new();

        analyzer.add_financial_relationship(FinancialRelationship {
            investor_id: "inv-001".to_string(),
            investee_id: "co-001".to_string(),
            investment_type: InvestmentType::Equity,
            amount: Some(10_000_000.0),
            currency: "USD".to_string(),
            round_type: Some("Series A".to_string()),
            valuation: Some(50_000_000.0),
            ownership_percentage: Some(20.0),
            date: Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()),
        });

        let graph = analyzer.extract_financial_graph();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
    }

    #[test]
    fn test_industry_grouping() {
        let mut engine = CorrelationEngine::new();

        engine.add_industry_classification(IndustryClassification {
            entity_id: "e-001".to_string(),
            industry_code: "TECH".to_string(),
            industry_name: "Technology".to_string(),
            level: 1,
            is_primary: true,
        });

        engine.add_industry_classification(IndustryClassification {
            entity_id: "e-002".to_string(),
            industry_code: "TECH".to_string(),
            industry_name: "Technology".to_string(),
            level: 1,
            is_primary: true,
        });

        engine.add_industry_classification(IndustryClassification {
            entity_id: "e-001".to_string(),
            industry_code: "SEMI".to_string(),
            industry_name: "Semiconductors".to_string(),
            level: 2,
            is_primary: false,
        });

        let groups = engine.group_by_industry();
        assert!(groups.contains_key("TECH"));

        let tech_group = groups.get("TECH").unwrap();
        assert_eq!(tech_group.entities.len(), 2);
        assert_eq!(tech_group.primary_entity_count, 2);
    }
}
