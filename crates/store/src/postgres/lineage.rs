//! Evidence lineage graph persistence + read API (migration 079, audit
//! evidence-graph item).
//!
//! Writers record a [`LineageNode`] per pipeline stage and a transition per
//! produced artifact; readers call [`PgStore::load_lineage_trace`] to walk the
//! graph upstream from an insight back to the source document that started
//! the chain. Both operations are idempotent: `(stage, reference)` and
//! `(transformation, source_node, target_node)` are unique, so re-running a
//! stage re-affirms its rows instead of duplicating the graph.
//!
//! Node/edge insertion on the hot path is best-effort for the pipeline
//! (see `insert_insight_claims`): lineage never blocks claim persistence, but
//! a failed lineage write is logged loudly and leaves a readable gap.

use super::*;

use apex_core::lineage::{
    lineage_digest, LineageEdge, LineageNode, LineageStage, LineageTrace, SourceDocumentRef,
};

/// Default producer label for lineage rows written by the store itself.
pub const LINEAGE_PRODUCER: &str = "apex-store:evidence-lineage";
/// Producer version recorded when the caller does not supply one.
pub const LINEAGE_PRODUCER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Default upstream traversal depth (the canonical chain has 12 stages).
pub const LINEAGE_MAX_DEPTH: i32 = 16;

/// Persisted lineage node row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LineageNodeRow {
    pub id: Uuid,
    pub stage: String,
    pub reference: String,
    pub producer: String,
    pub producer_version: String,
    pub created_at: DateTime<Utc>,
    pub input_digest: Option<String>,
    pub confidence: Option<f64>,
    pub metadata: Value,
}

impl LineageNodeRow {
    /// Convert a persisted row to the shared model; unknown stage names are
    /// skipped rather than guessed.
    pub fn to_node(&self) -> Option<LineageNode> {
        let stage = LineageStage::from_db(&self.stage)?;
        Some(LineageNode {
            id: Some(self.id.to_string()),
            stage,
            reference: self.reference.clone(),
            producer: self.producer.clone(),
            producer_version: self.producer_version.clone(),
            created_at: self.created_at,
            input_digest: self.input_digest.clone(),
            confidence: self.confidence,
        })
    }
}

/// Persisted lineage edge row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LineageEdgeRow {
    pub id: Uuid,
    pub transformation: String,
    pub source_node: Uuid,
    pub target_node: Uuid,
    pub producer: String,
    pub producer_version: String,
    pub created_at: DateTime<Utc>,
    pub input_digest: Option<String>,
    pub confidence: Option<f64>,
    pub metadata: Value,
}

/// One observation's lineage-relevant metadata captured while claims are
/// persisted, so the document -> observation edges can be recorded after the
/// claim transaction commits.
#[derive(Debug, Clone)]
pub struct ObservationLineageSeed {
    pub observation_id: Uuid,
    pub confidence: f64,
    pub document_url: Option<String>,
    pub content_digest: Option<String>,
    pub extractor_version: Option<String>,
}

impl PgStore {
    /// Upsert one lineage node, returning its row id. Re-recording a node
    /// refreshes producer/version and fills in missing digest/confidence
    /// without rewriting recorded history.
    pub async fn upsert_lineage_node(&self, node: &LineageNode) -> Result<Uuid> {
        sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO evidence_lineage_nodes
                   (stage, reference, producer, producer_version, created_at,
                    input_digest, confidence)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (stage, reference) DO UPDATE SET
                   producer = EXCLUDED.producer,
                   producer_version = EXCLUDED.producer_version,
                   input_digest = COALESCE(
                       EXCLUDED.input_digest, evidence_lineage_nodes.input_digest),
                   confidence = COALESCE(
                       EXCLUDED.confidence, evidence_lineage_nodes.confidence)
               RETURNING id"#,
        )
        .bind(node.stage.as_str())
        .bind(&node.reference)
        .bind(&node.producer)
        .bind(&node.producer_version)
        .bind(node.created_at)
        .bind(&node.input_digest)
        .bind(node.confidence)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Upsert one edge between already-persisted nodes. `transformation` is
    /// the stage of the target node.
    pub async fn record_lineage_edge(
        &self,
        transformation: LineageStage,
        source_node: Uuid,
        target_node: Uuid,
        producer: &str,
        producer_version: &str,
        input_digest: Option<&str>,
        confidence: Option<f64>,
    ) -> Result<Uuid> {
        sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO evidence_lineage_edges
                   (transformation, source_node, target_node, producer,
                    producer_version, input_digest, confidence)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               ON CONFLICT (transformation, source_node, target_node) DO UPDATE SET
                   input_digest = COALESCE(
                       EXCLUDED.input_digest, evidence_lineage_edges.input_digest),
                   confidence = COALESCE(
                       EXCLUDED.confidence, evidence_lineage_edges.confidence)
               RETURNING id"#,
        )
        .bind(transformation.as_str())
        .bind(source_node)
        .bind(target_node)
        .bind(producer)
        .bind(producer_version)
        .bind(input_digest)
        .bind(confidence)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Persist one stage transition: upsert both nodes, then the edge.
    pub async fn record_lineage_transition(
        &self,
        source: &LineageNode,
        target: &LineageNode,
        input_digest: Option<&str>,
        confidence: Option<f64>,
    ) -> Result<()> {
        let source_id = self.upsert_lineage_node(source).await?;
        let target_id = self.upsert_lineage_node(target).await?;
        self.record_lineage_edge(
            target.stage,
            source_id,
            target_id,
            &target.producer,
            &target.producer_version,
            input_digest,
            confidence,
        )
        .await?;
        Ok(())
    }

    /// Record the lineage of one persisted claim: observation -> claim and
    /// claim -> insight, plus document -> observation for each evidence row
    /// whose provenance names the fetched document. Best-effort at the call
    /// site; errors here never roll back claim persistence.
    pub async fn record_claim_lineage(
        &self,
        insight_id: Uuid,
        claim_id: Uuid,
        claim_text: &str,
        claim_confidence: Option<f64>,
        evidence_ids: &[Uuid],
        observations: &[ObservationLineageSeed],
        producer: &str,
        producer_version: &str,
        insight_confidence: Option<f64>,
    ) -> Result<()> {
        let insight_node = LineageNode::new(
            LineageStage::Insight,
            insight_id.to_string(),
            producer,
            producer_version,
        )
        .with_confidence(insight_confidence.unwrap_or(0.5));

        let evidence_digest = lineage_digest(
            evidence_ids
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(",")
                .as_bytes(),
        );
        let claim_node = LineageNode::new(
            LineageStage::Claim,
            claim_id.to_string(),
            producer,
            producer_version,
        )
        .with_input_digest(evidence_digest)
        .with_confidence(claim_confidence.unwrap_or(0.5));

        // claim -> insight
        self.record_lineage_transition(
            &claim_node,
            &insight_node,
            Some(&lineage_digest(claim_text.as_bytes())),
            claim_confidence,
        )
        .await?;

        let observation_by_id: std::collections::HashMap<Uuid, &ObservationLineageSeed> =
            observations
                .iter()
                .map(|seed| (seed.observation_id, seed))
                .collect();

        for evidence_id in evidence_ids {
            let seed = observation_by_id.get(evidence_id).copied();
            let observation_node = LineageNode::new(
                LineageStage::Observation,
                evidence_id.to_string(),
                "crawl_ingest",
                seed.and_then(|s| s.extractor_version.as_deref())
                    .unwrap_or("unknown"),
            )
            .with_confidence(seed.map(|s| s.confidence).unwrap_or(0.5));
            // observation -> claim
            self.record_lineage_transition(
                &observation_node,
                &claim_node,
                seed.and_then(|s| s.content_digest.as_deref()),
                claim_confidence,
            )
            .await?;

            // document -> observation (the extraction stage)
            let Some(seed) = seed else { continue };
            let Some(url) = seed.document_url.as_deref() else {
                continue;
            };
            let document_node = LineageNode::new(
                LineageStage::SourceDocument,
                apex_core::lineage::source_document_reference(url),
                "crawl_fetch",
                seed.extractor_version.as_deref().unwrap_or("unknown"),
            );
            let mut document_node = document_node;
            if let Some(digest) = seed.content_digest.as_deref() {
                document_node = document_node.with_input_digest(digest);
            }
            self.record_lineage_transition(
                &document_node,
                &observation_node,
                seed.content_digest.as_deref(),
                Some(seed.confidence),
            )
            .await?;
        }
        Ok(())
    }

    /// Walk the lineage graph upstream from a root artifact and return every
    /// reachable node plus the connecting edges. `None` when the root was
    /// never recorded.
    pub async fn load_lineage_trace(
        &self,
        root_stage: LineageStage,
        root_reference: &str,
        max_depth: i32,
    ) -> Result<Option<LineageTrace>> {
        let root_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM evidence_lineage_nodes WHERE stage = $1 AND reference = $2",
        )
        .bind(root_stage.as_str())
        .bind(root_reference)
        .fetch_optional(&self.pool)
        .await?;
        let Some(root_id) = root_id else {
            return Ok(None);
        };

        let depth = max_depth.clamp(1, 64);
        let rows: Vec<LineageNodeRow> = sqlx::query_as(
            r#"WITH RECURSIVE upstream AS (
                   SELECT $1::uuid AS node_id, 0 AS depth
                   UNION ALL
                   SELECT e.source_node, u.depth + 1
                   FROM upstream u
                   JOIN evidence_lineage_edges e ON e.target_node = u.node_id
                   WHERE u.depth < $2
               )
               SELECT DISTINCT n.id, n.stage, n.reference, n.producer,
                      n.producer_version, n.created_at, n.input_digest,
                      n.confidence, n.metadata
               FROM upstream u
               JOIN evidence_lineage_nodes n ON n.id = u.node_id"#,
        )
        .bind(root_id)
        .bind(depth)
        .fetch_all(&self.pool)
        .await?;

        let node_ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let edge_rows: Vec<LineageEdgeRow> = sqlx::query_as(
            r#"SELECT id, transformation, source_node, target_node, producer,
                      producer_version, created_at, input_digest, confidence,
                      metadata
               FROM evidence_lineage_edges
               WHERE source_node = ANY($1) AND target_node = ANY($1)"#,
        )
        .bind(&node_ids)
        .fetch_all(&self.pool)
        .await?;

        let mut nodes: Vec<LineageNode> = rows.iter().filter_map(|row| row.to_node()).collect();
        nodes.sort_by(|a, b| {
            a.stage
                .order()
                .cmp(&b.stage.order())
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.reference.cmp(&b.reference))
        });

        let mut edges: Vec<LineageEdge> = edge_rows
            .iter()
            .filter_map(|row| {
                Some(LineageEdge {
                    transformation: LineageStage::from_db(&row.transformation)?,
                    source_node: row.source_node.to_string(),
                    target_node: row.target_node.to_string(),
                    producer: row.producer.clone(),
                    producer_version: row.producer_version.clone(),
                    created_at: row.created_at,
                    input_digest: row.input_digest.clone(),
                    confidence: row.confidence,
                })
            })
            .collect();
        edges.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.source_node.cmp(&b.source_node))
        });

        let source_documents: Vec<SourceDocumentRef> = rows
            .iter()
            .filter(|row| row.stage == LineageStage::SourceDocument.as_str())
            .map(|row| SourceDocumentRef {
                url: row.reference.clone(),
                content_digest: row.input_digest.clone(),
                observed_at: Some(row.created_at),
            })
            .collect();

        Ok(Some(LineageTrace {
            root_stage,
            root_reference: root_reference.to_string(),
            nodes,
            edges,
            source_documents,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_row_skips_unknown_stage() {
        let row = LineageNodeRow {
            id: Uuid::new_v4(),
            stage: "not_a_stage".to_string(),
            reference: "x".to_string(),
            producer: "p".to_string(),
            producer_version: "1".to_string(),
            created_at: Utc::now(),
            input_digest: None,
            confidence: None,
            metadata: Value::Null,
        };
        assert!(row.to_node().is_none());
    }

    #[test]
    fn known_stage_row_round_trips() {
        let row = LineageNodeRow {
            id: Uuid::new_v4(),
            stage: "observation".to_string(),
            reference: "obs-1".to_string(),
            producer: "crawl_ingest".to_string(),
            producer_version: "1.2.3".to_string(),
            created_at: Utc::now(),
            input_digest: Some("digest".to_string()),
            confidence: Some(0.8),
            metadata: Value::Null,
        };
        let node = row.to_node().expect("known stage converts");
        assert_eq!(node.stage, LineageStage::Observation);
        assert_eq!(node.reference, "obs-1");
    }
}
