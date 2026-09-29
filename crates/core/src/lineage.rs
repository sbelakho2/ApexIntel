//! Evidence lineage graph (audit evidence-graph item).
//!
//! Every artifact in the intelligence pipeline is a node in one chain:
//!
//! `source_document -> extraction -> observation -> entity_link -> feature ->
//! claim -> insight -> warning -> triage -> notification -> analyst_action ->
//! outcome`
//!
//! Each node records the stage that produced it (`LineageStage`), the
//! producer and its version, when it was created, the digest of its input,
//! and the confidence assigned at that stage. Edges connect a node to the
//! node produced from it, carrying the transformation name.
//!
//! The types here are pure data; persistence lives in `apex-store` and is
//! exposed to the UI through the lineage read API so an insight can always be
//! traced back to the source document that started the chain.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Pipeline stage / transformation name. Order matters: the enum is the
/// canonical chain order used when rendering a trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageStage {
    SourceDocument,
    Extraction,
    Observation,
    EntityLink,
    Feature,
    Claim,
    Insight,
    Warning,
    Triage,
    Notification,
    AnalystAction,
    Outcome,
}

impl LineageStage {
    pub const ALL: [Self; 12] = [
        Self::SourceDocument,
        Self::Extraction,
        Self::Observation,
        Self::EntityLink,
        Self::Feature,
        Self::Claim,
        Self::Insight,
        Self::Warning,
        Self::Triage,
        Self::Notification,
        Self::AnalystAction,
        Self::Outcome,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceDocument => "source_document",
            Self::Extraction => "extraction",
            Self::Observation => "observation",
            Self::EntityLink => "entity_link",
            Self::Feature => "feature",
            Self::Claim => "claim",
            Self::Insight => "insight",
            Self::Warning => "warning",
            Self::Triage => "triage",
            Self::Notification => "notification",
            Self::AnalystAction => "analyst_action",
            Self::Outcome => "outcome",
        }
    }

    /// Parse a persisted stage name; unknown values map to `None` instead of
    /// guessing a stage.
    pub fn from_db(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "source_document" => Some(Self::SourceDocument),
            "extraction" => Some(Self::Extraction),
            "observation" => Some(Self::Observation),
            "entity_link" => Some(Self::EntityLink),
            "feature" => Some(Self::Feature),
            "claim" => Some(Self::Claim),
            "insight" => Some(Self::Insight),
            "warning" => Some(Self::Warning),
            "triage" => Some(Self::Triage),
            "notification" => Some(Self::Notification),
            "analyst_action" => Some(Self::AnalystAction),
            "outcome" => Some(Self::Outcome),
            _ => None,
        }
    }

    /// Position in the canonical chain (0-based), used to sort traces.
    pub fn order(self) -> u8 {
        Self::ALL
            .iter()
            .position(|stage| *stage == self)
            .unwrap_or(0) as u8
    }
}

impl std::fmt::Display for LineageStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A node in the evidence lineage graph: one artifact at one pipeline stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineageNode {
    /// Persisted row id when the node was loaded from the graph; `None` for
    /// in-memory nodes that have not been written yet. Edges reference nodes
    /// by this id, so a loaded trace is directly traversable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub stage: LineageStage,
    /// Stable external reference: artifact UUID for database rows, URL for
    /// source documents, recipe/slug for other producers.
    pub reference: String,
    pub producer: String,
    pub producer_version: String,
    pub created_at: DateTime<Utc>,
    /// Digest of the input this stage consumed (content hash, payload hash).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

impl LineageNode {
    pub fn new(
        stage: LineageStage,
        reference: impl Into<String>,
        producer: impl Into<String>,
        producer_version: impl Into<String>,
    ) -> Self {
        Self {
            id: None,
            stage,
            reference: reference.into(),
            producer: producer.into(),
            producer_version: producer_version.into(),
            created_at: Utc::now(),
            input_digest: None,
            confidence: None,
        }
    }

    pub fn with_input_digest(mut self, input_digest: impl Into<String>) -> Self {
        self.input_digest = Some(input_digest.into());
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence.clamp(0.0, 1.0));
        self
    }

    /// Set confidence from an optional measurement. `None` stays unknown —
    /// lineage must never invent a confidence value.
    pub fn with_confidence_opt(mut self, confidence: Option<f64>) -> Self {
        self.confidence = confidence.map(|value| value.clamp(0.0, 1.0));
        self
    }
}

/// A directed edge: `source` was transformed into `target` by
/// `transformation` (always the stage of `target`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineageEdge {
    pub transformation: LineageStage,
    pub source_node: String,
    pub target_node: String,
    pub producer: String,
    pub producer_version: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// A source document recovered from a lineage trace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceDocumentRef {
    pub url: String,
    /// Digest of the fetched document when recorded (`content_hash`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<DateTime<Utc>>,
}

/// Full trace returned by the lineage read API: every node reachable
/// upstream of an insight plus the edges connecting them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineageTrace {
    pub root_stage: LineageStage,
    pub root_reference: String,
    pub nodes: Vec<LineageNode>,
    pub edges: Vec<LineageEdge>,
    /// Source documents the chain starts from, ordered by stage order.
    pub source_documents: Vec<SourceDocumentRef>,
}

impl LineageTrace {
    pub fn contains_reference(&self, stage: LineageStage, reference: &str) -> bool {
        self.nodes
            .iter()
            .any(|node| node.stage == stage && node.reference == reference)
    }
}

/// Deterministic reference for a source document node: the URL is the stable
/// identity the crawler and the UI can both resolve.
pub fn source_document_reference(url: &str) -> String {
    url.trim().to_string()
}

/// Sha-256 hex digest of a lineage input payload.
pub fn lineage_digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_round_trips_and_orders() {
        for stage in LineageStage::ALL {
            assert_eq!(LineageStage::from_db(stage.as_str()), Some(stage));
        }
        assert_eq!(LineageStage::from_db("nonsense"), None);
        assert!(
            LineageStage::SourceDocument.order() < LineageStage::Insight.order(),
            "source document must sort before the insight that cites it"
        );
    }

    #[test]
    fn node_builder_clamps_confidence_and_keeps_digest() {
        let digest = lineage_digest(b"payload");
        let node = LineageNode::new(LineageStage::Observation, "obs-1", "crawl_ingest", "1.0.0")
            .with_input_digest(digest.clone())
            .with_confidence(1.5);

        assert_eq!(node.confidence, Some(1.0));
        assert_eq!(node.input_digest.as_deref(), Some(digest.as_str()));
        assert_eq!(node.stage, LineageStage::Observation);
    }

    #[test]
    fn trace_reference_lookup() {
        let trace = LineageTrace {
            root_stage: LineageStage::Insight,
            root_reference: "insight-1".to_string(),
            nodes: vec![LineageNode::new(
                LineageStage::Insight,
                "insight-1",
                "insight_generation",
                "1.0.0",
            )],
            edges: Vec::new(),
            source_documents: Vec::new(),
        };

        assert!(trace.contains_reference(LineageStage::Insight, "insight-1"));
        assert!(!trace.contains_reference(LineageStage::Observation, "obs-1"));
    }
}
