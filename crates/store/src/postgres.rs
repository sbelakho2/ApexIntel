use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

use apex_core::entities::*;


/// PostgreSQL connection pool wrapper with all CRUD operations.
#[derive(Clone)]
pub struct PgStore {
    pub pool: PgPool,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    // ─── Companies ───────────────────────────────────────────────────────

    pub async fn insert_company(&self, c: &Company) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO companies
               (id, name, legal_name, domain, country_code, region, company_type,
                industry_tags, employee_estimate, revenue_estimate_usd,
                risk_score, threat_score, overlap_score, strategic_relevance,
                metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 updated_at = now()"#,
        )
        .bind(c.id)
        .bind(&c.name)
        .bind(&c.legal_name)
        .bind(&c.domain)
        .bind(&c.country_code)
        .bind(&c.region)
        .bind(c.company_type.as_str())
        .bind(&c.industry_tags)
        .bind(c.employee_estimate)
        .bind(c.revenue_estimate_usd)
        .bind(c.risk_score)
        .bind(c.threat_score)
        .bind(c.overlap_score)
        .bind(c.strategic_relevance)
        .bind(&c.metadata)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_company(&self, id: Uuid) -> Result<Option<CompanyRow>> {
        let row = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    metadata, created_at, updated_at
             FROM companies WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_companies_by_region(&self, region: &str) -> Result<Vec<CompanyRow>> {
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    metadata, created_at, updated_at
             FROM companies WHERE region = $1 ORDER BY name",
        )
        .bind(region)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_companies_by_type(&self, company_type: &str) -> Result<Vec<CompanyRow>> {
        let rows = sqlx::query_as::<_, CompanyRow>(
            "SELECT id, name, legal_name, domain, country_code, region, company_type,
                    industry_tags, employee_estimate, revenue_estimate_usd,
                    risk_score, threat_score, overlap_score, strategic_relevance,
                    metadata, created_at, updated_at
             FROM companies WHERE company_type = $1 ORDER BY name",
        )
        .bind(company_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Sites ───────────────────────────────────────────────────────────

    pub async fn insert_site(&self, s: &Site) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO sites
               (id, company_id, name, address, city, country_code, region,
                lat, lon, site_type, capabilities, certifications,
                employee_estimate, free_zone, metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
               ON CONFLICT (id) DO UPDATE SET
                 name = EXCLUDED.name,
                 updated_at = now()"#,
        )
        .bind(s.id)
        .bind(s.company_id)
        .bind(&s.name)
        .bind(&s.address)
        .bind(&s.city)
        .bind(&s.country_code)
        .bind(&s.region)
        .bind(s.lat)
        .bind(s.lon)
        .bind(s.site_type.as_str())
        .bind(&s.capabilities)
        .bind(&s.certifications)
        .bind(s.employee_estimate)
        .bind(&s.free_zone)
        .bind(&s.metadata)
        .bind(s.created_at)
        .bind(s.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_sites_for_company(&self, company_id: Uuid) -> Result<Vec<SiteRow>> {
        let rows = sqlx::query_as::<_, SiteRow>(
            "SELECT id, company_id, name, address, city, country_code, region,
                    lat, lon, site_type, capabilities, certifications,
                    employee_estimate, free_zone, metadata, created_at, updated_at
             FROM sites WHERE company_id = $1 ORDER BY name",
        )
        .bind(company_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Persons (POI) ──────────────────────────────────────────────────

    pub async fn insert_person(&self, p: &Person) -> Result<()> {
        let pv_json = serde_json::to_value(&p.priority_vector)?;
        sqlx::query(
            r#"INSERT INTO persons
               (id, name, name_ar, name_fr, primary_org_id, current_role,
                role_family, region, country_code, priority_vector,
                influence_score, metadata, created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
               ON CONFLICT (id) DO UPDATE SET
                 current_role = EXCLUDED.current_role,
                 updated_at = now()"#,
        )
        .bind(p.id)
        .bind(&p.name)
        .bind(&p.name_ar)
        .bind(&p.name_fr)
        .bind(p.primary_org_id)
        .bind(&p.current_role)
        .bind(p.role_family.as_str())
        .bind(&p.region)
        .bind(&p.country_code)
        .bind(&pv_json)
        .bind(p.influence_score)
        .bind(&p.metadata)
        .bind(p.created_at)
        .bind(p.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_person(&self, id: Uuid) -> Result<Option<PersonRow>> {
        let row = sqlx::query_as::<_, PersonRow>(
            "SELECT id, name, name_ar, name_fr, primary_org_id, current_role,
                    role_family, region, country_code, priority_vector,
                    influence_score, metadata, created_at, updated_at
             FROM persons WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_persons_by_org(&self, org_id: Uuid) -> Result<Vec<PersonRow>> {
        let rows = sqlx::query_as::<_, PersonRow>(
            "SELECT id, name, name_ar, name_fr, primary_org_id, current_role,
                    role_family, region, country_code, priority_vector,
                    influence_score, metadata, created_at, updated_at
             FROM persons WHERE primary_org_id = $1 ORDER BY name",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Observations ────────────────────────────────────────────────────

    pub async fn insert_observation(&self, o: &Observation) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO observations
               (id, observation_type, entity_id, entity_type, ts_utc,
                value, provenance, confidence, created_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(o.id)
        .bind(o.observation_type.as_str())
        .bind(o.entity_id)
        .bind(&o.entity_type)
        .bind(o.ts_utc)
        .bind(&o.value)
        .bind(&o.provenance)
        .bind(o.confidence)
        .bind(o.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_observations_by_entity(
        &self,
        entity_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc,
                    value, provenance, confidence, created_at
             FROM observations
             WHERE entity_id = $1
             ORDER BY ts_utc DESC
             LIMIT $2",
        )
        .bind(entity_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_observations_by_type(
        &self,
        obs_type: &str,
        since: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<ObservationRow>> {
        let rows = sqlx::query_as::<_, ObservationRow>(
            "SELECT id, observation_type, entity_id, entity_type, ts_utc,
                    value, provenance, confidence, created_at
             FROM observations
             WHERE observation_type = $1 AND ts_utc >= $2
             ORDER BY ts_utc DESC
             LIMIT $3",
        )
        .bind(obs_type)
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Graph Edges ─────────────────────────────────────────────────────

    pub async fn upsert_edge(&self, e: &GraphEdge) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO graph_edges
               (id, source_id, source_type, target_id, target_type,
                edge_type, weight, confidence, evidence_ids, metadata,
                first_seen, last_seen)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
               ON CONFLICT (source_id, source_type, target_id, target_type, edge_type)
               DO UPDATE SET
                 weight = EXCLUDED.weight,
                 confidence = EXCLUDED.confidence,
                 last_seen = now()"#,
        )
        .bind(e.id)
        .bind(e.source_id)
        .bind(&e.source_type)
        .bind(e.target_id)
        .bind(&e.target_type)
        .bind(e.edge_type.as_str())
        .bind(e.weight)
        .bind(e.confidence)
        .bind(&e.evidence_ids)
        .bind(&e.metadata)
        .bind(e.first_seen)
        .bind(e.last_seen)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_edges_from(
        &self,
        source_id: Uuid,
        source_type: &str,
    ) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE source_id = $1 AND source_type = $2
             ORDER BY weight DESC",
        )
        .bind(source_id)
        .bind(source_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_edges_to(
        &self,
        target_id: Uuid,
        target_type: &str,
    ) -> Result<Vec<EdgeRow>> {
        let rows = sqlx::query_as::<_, EdgeRow>(
            "SELECT id, source_id, source_type, target_id, target_type,
                    edge_type, weight, confidence, evidence_ids, metadata,
                    first_seen, last_seen
             FROM graph_edges
             WHERE target_id = $1 AND target_type = $2
             ORDER BY weight DESC",
        )
        .bind(target_id)
        .bind(target_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Certifications ──────────────────────────────────────────────────

    pub async fn insert_certification(&self, c: &Certification) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO certifications
               (id, company_id, site_id, standard, status, issuing_body,
                valid_from, valid_until, scope, evidence_url, metadata,
                created_at, updated_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
               ON CONFLICT (id) DO UPDATE SET
                 status = EXCLUDED.status,
                 updated_at = now()"#,
        )
        .bind(c.id)
        .bind(c.company_id)
        .bind(c.site_id)
        .bind(&c.standard)
        .bind(c.status.as_str())
        .bind(&c.issuing_body)
        .bind(c.valid_from)
        .bind(c.valid_until)
        .bind(&c.scope)
        .bind(&c.evidence_url)
        .bind(&c.metadata)
        .bind(c.created_at)
        .bind(c.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_certifications_for_company(
        &self,
        company_id: Uuid,
    ) -> Result<Vec<CertificationRow>> {
        let rows = sqlx::query_as::<_, CertificationRow>(
            "SELECT id, company_id, site_id, standard, status, issuing_body,
                    valid_from, valid_until, scope, evidence_url, metadata,
                    created_at, updated_at
             FROM certifications WHERE company_id = $1 ORDER BY standard",
        )
        .bind(company_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── POI Artifacts ───────────────────────────────────────────────────

    pub async fn insert_poi_artifact(&self, a: &PoiArtifact) -> Result<()> {
        let prov_json = serde_json::to_value(&a.provenance)?;
        sqlx::query(
            r#"INSERT INTO poi_artifacts
               (id, person_id, artifact_type, title, content_summary,
                url, source_domain, language, topics, sentiment_score,
                key_phrases, ts_utc, provenance, metadata, created_at)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(a.id)
        .bind(a.person_id)
        .bind(a.artifact_type.as_str())
        .bind(&a.title)
        .bind(&a.content_summary)
        .bind(&a.url)
        .bind(&a.source_domain)
        .bind(&a.language)
        .bind(&a.topics)
        .bind(a.sentiment_score)
        .bind(&a.key_phrases)
        .bind(a.ts_utc)
        .bind(&prov_json)
        .bind(&a.metadata)
        .bind(a.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_artifacts_for_person(
        &self,
        person_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ArtifactRow>> {
        let rows = sqlx::query_as::<_, ArtifactRow>(
            "SELECT id, person_id, artifact_type, title, content_summary,
                    url, source_domain, language, topics, sentiment_score,
                    key_phrases, ts_utc, provenance, metadata, created_at
             FROM poi_artifacts
             WHERE person_id = $1
             ORDER BY ts_utc DESC
             LIMIT $2",
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ─── Capabilities ────────────────────────────────────────────────────

    pub async fn insert_capability(&self, cap: &Capability) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO capabilities
               (id, company_id, site_id, capability, proof_grade,
                evidence_urls, first_seen, last_confirmed, metadata)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT (id) DO UPDATE SET
                 last_confirmed = now()"#,
        )
        .bind(cap.id)
        .bind(cap.company_id)
        .bind(cap.site_id)
        .bind(&cap.capability)
        .bind(cap.proof_grade.as_str())
        .bind(&cap.evidence_urls)
        .bind(cap.first_seen)
        .bind(cap.last_confirmed)
        .bind(&cap.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ─── Logistics Nodes ─────────────────────────────────────────────────

    pub async fn insert_logistics_node(&self, n: &LogisticsNode) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO logistics_nodes
               (id, name, node_type, country_code, lat, lon, metadata)
               VALUES ($1,$2,$3,$4,$5,$6,$7)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(n.id)
        .bind(&n.name)
        .bind(&n.node_type)
        .bind(&n.country_code)
        .bind(n.lat)
        .bind(n.lon)
        .bind(&n.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ─── Schema Migration ────────────────────────────────────────────────

    /// Run the full schema creation. Idempotent via IF NOT EXISTS.
    pub async fn run_migrations(&self) -> Result<()> {
        sqlx::query(include_str!("../migrations/init.sql"))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

// ─── Row Types (sqlx::FromRow) ──────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CompanyRow {
    pub id: Uuid,
    pub name: String,
    pub legal_name: Option<String>,
    pub domain: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub company_type: Option<String>,
    pub industry_tags: Option<Vec<String>>,
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub risk_score: Option<f64>,
    pub threat_score: Option<f64>,
    pub overlap_score: Option<f64>,
    pub strategic_relevance: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct SiteRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub name: String,
    pub address: Option<String>,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub site_type: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub certifications: Option<Vec<String>>,
    pub employee_estimate: Option<i32>,
    pub free_zone: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PersonRow {
    pub id: Uuid,
    pub name: String,
    pub name_ar: Option<String>,
    pub name_fr: Option<String>,
    pub primary_org_id: Option<Uuid>,
    pub current_role: Option<String>,
    pub role_family: Option<String>,
    pub region: Option<String>,
    pub country_code: Option<String>,
    pub priority_vector: Option<serde_json::Value>,
    pub influence_score: Option<f64>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ObservationRow {
    pub id: Uuid,
    pub observation_type: String,
    pub entity_id: Option<Uuid>,
    pub entity_type: Option<String>,
    pub ts_utc: DateTime<Utc>,
    pub value: serde_json::Value,
    pub provenance: serde_json::Value,
    pub confidence: Option<f64>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EdgeRow {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_type: String,
    pub target_id: Uuid,
    pub target_type: String,
    pub edge_type: String,
    pub weight: Option<f64>,
    pub confidence: Option<f64>,
    pub evidence_ids: Option<Vec<Uuid>>,
    pub metadata: Option<serde_json::Value>,
    pub first_seen: Option<DateTime<Utc>>,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CertificationRow {
    pub id: Uuid,
    pub company_id: Option<Uuid>,
    pub site_id: Option<Uuid>,
    pub standard: String,
    pub status: Option<String>,
    pub issuing_body: Option<String>,
    pub valid_from: Option<NaiveDate>,
    pub valid_until: Option<NaiveDate>,
    pub scope: Option<String>,
    pub evidence_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ArtifactRow {
    pub id: Uuid,
    pub person_id: Option<Uuid>,
    pub artifact_type: String,
    pub title: Option<String>,
    pub content_summary: Option<String>,
    pub url: String,
    pub source_domain: Option<String>,
    pub language: Option<String>,
    pub topics: Option<Vec<String>>,
    pub sentiment_score: Option<f64>,
    pub key_phrases: Option<Vec<String>>,
    pub ts_utc: DateTime<Utc>,
    pub provenance: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests verify query construction and row type structure.
    /// Full integration tests require a running PostgreSQL instance.

    #[test]
    fn test_company_row_fields() {
        // Verify CompanyRow has all expected fields via construction
        let row = CompanyRow {
            id: Uuid::new_v4(),
            name: "Starz Electronics".into(),
            legal_name: Some("Starz Electronics SARL".into()),
            domain: Some("starz-electronics.com".into()),
            country_code: Some("TN".into()),
            region: Some("TN".into()),
            company_type: Some("ems".into()),
            industry_tags: Some(vec!["automotive".into(), "industrial".into()]),
            employee_estimate: Some(500),
            revenue_estimate_usd: Some(50_000_000),
            risk_score: Some(0.3),
            threat_score: Some(0.1),
            overlap_score: Some(0.8),
            strategic_relevance: Some(0.9),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Starz Electronics");
        assert_eq!(row.country_code.as_deref(), Some("TN"));

        // Verify serialization
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["name"], "Starz Electronics");
        assert!(json["industry_tags"].is_array());
    }

    #[test]
    fn test_observation_row_fields() {
        let row = ObservationRow {
            id: Uuid::new_v4(),
            observation_type: "JobPost".into(),
            entity_id: Some(Uuid::new_v4()),
            entity_type: Some("company".into()),
            ts_utc: Utc::now(),
            value: serde_json::json!({"role": "SQE", "role_family": "quality"}),
            provenance: serde_json::json!({"url": "https://jobs.example.com"}),
            confidence: Some(0.95),
            created_at: Some(Utc::now()),
        };
        assert_eq!(row.observation_type, "JobPost");
        assert_eq!(row.value["role"], "SQE");
    }

    #[test]
    fn test_edge_row_fields() {
        let row = EdgeRow {
            id: Uuid::new_v4(),
            source_id: Uuid::new_v4(),
            source_type: "company".into(),
            target_id: Uuid::new_v4(),
            target_type: "company".into(),
            edge_type: "competitor".into(),
            weight: Some(0.8),
            confidence: Some(0.9),
            evidence_ids: Some(vec![Uuid::new_v4()]),
            metadata: Some(serde_json::json!({})),
            first_seen: Some(Utc::now()),
            last_seen: Some(Utc::now()),
        };
        assert_eq!(row.edge_type, "competitor");
        assert!(row.weight.unwrap() > 0.5);
    }

    #[test]
    fn test_person_row_fields() {
        let row = PersonRow {
            id: Uuid::new_v4(),
            name: "Ahmed Ben Ali".into(),
            name_ar: Some("أحمد بن علي".into()),
            name_fr: Some("Ahmed Ben Ali".into()),
            primary_org_id: Some(Uuid::new_v4()),
            current_role: Some("VP Procurement".into()),
            role_family: Some("procurement".into()),
            region: Some("TN".into()),
            country_code: Some("TN".into()),
            priority_vector: Some(serde_json::json!({"cost":0.8,"quality":0.6})),
            influence_score: Some(0.7),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Ahmed Ben Ali");
        assert_eq!(row.role_family.as_deref(), Some("procurement"));
    }

    #[test]
    fn test_certification_row_fields() {
        let row = CertificationRow {
            id: Uuid::new_v4(),
            company_id: Some(Uuid::new_v4()),
            site_id: None,
            standard: "IATF_16949".into(),
            status: Some("active".into()),
            issuing_body: Some("TUV".into()),
            valid_from: Some(NaiveDate::from_ymd_opt(2023, 1, 1).unwrap()),
            valid_until: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            scope: Some("Automotive EMS".into()),
            evidence_url: Some("https://example.com/cert".into()),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.standard, "IATF_16949");
        assert!(row.valid_until.unwrap() > row.valid_from.unwrap());
    }

    #[test]
    fn test_artifact_row_fields() {
        let row = ArtifactRow {
            id: Uuid::new_v4(),
            person_id: Some(Uuid::new_v4()),
            artifact_type: "press_quote".into(),
            title: Some("Industry 4.0 Panel".into()),
            content_summary: Some("Discussed manufacturing automation".into()),
            url: "https://example.com/article".into(),
            source_domain: Some("example.com".into()),
            language: Some("en".into()),
            topics: Some(vec!["automation".into(), "industry_4_0".into()]),
            sentiment_score: Some(0.7),
            key_phrases: Some(vec!["smart factory".into()]),
            ts_utc: Utc::now(),
            provenance: serde_json::json!({"url": "https://example.com/article"}),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
        };
        assert_eq!(row.artifact_type, "press_quote");
        assert!(row.topics.as_ref().unwrap().contains(&"automation".into()));
    }

    #[test]
    fn test_site_row_fields() {
        let row = SiteRow {
            id: Uuid::new_v4(),
            company_id: Some(Uuid::new_v4()),
            name: "Sousse Plant".into(),
            address: Some("Zone Industrielle".into()),
            city: Some("Sousse".into()),
            country_code: Some("TN".into()),
            region: Some("TN".into()),
            lat: Some(35.8256),
            lon: Some(10.6369),
            site_type: Some("plant".into()),
            capabilities: Some(vec!["SMT".into(), "THT".into(), "AOI".into()]),
            certifications: Some(vec!["ISO_9001".into(), "IATF_16949".into()]),
            employee_estimate: Some(300),
            free_zone: Some("TAC".into()),
            metadata: Some(serde_json::json!({})),
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
        };
        assert_eq!(row.name, "Sousse Plant");
        assert!(row.lat.unwrap() > 35.0);
        assert!(row.capabilities.as_ref().unwrap().contains(&"SMT".into()));
    }
}
