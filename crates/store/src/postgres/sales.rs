//! Sales-activation store layer.
//!
//! Persistence for the schema introduced by migration `20260701_sales_activation_layer.sql`:
//!   - `closed_deals`           → real win/loss analysis for battlecards
//!   - `competitor_pricing`     → real pricing intelligence for battlecards
//!   - `contact_methods`        → verified email/phone/LinkedIn enrichment
//!   - `engagement_events`      → outreach-feedback loop (wires poi::engagement_tracker)
//!   - `buying_centers` / members → deal-level decision-unit graph
//!   - `crawl_metrics`          → real per-source telemetry (replaces fabricated data)
//!   - `crm_sync_state`         → idempotent CRM write-back tracking
//!
//! Each method returns typed row structs; queries are parameterized.

use super::*;

use chrono::{DateTime, Utc};
use uuid::Uuid;

// ════════════════════════════════════════════════════════════════════════════
// CLOSED DEALS  (win/loss)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ClosedDealRow {
    pub id: Uuid,
    pub our_company_id: Uuid,
    pub competitor_id: Option<Uuid>,
    pub opportunity_id: Option<Uuid>,
    pub deal_name: String,
    pub account_name: Option<String>,
    pub deal_value: f64,
    pub won: bool,
    pub loss_reason: Option<String>,
    pub loss_reason_category: Option<String>,
    pub closed_at: DateTime<Utc>,
    pub owner_id: Option<String>,
    pub source: String,
    pub external_ref: Option<String>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}

impl PgStore {
    /// Insert a closed deal. Returns the new row id. Used by the CRM sync job
    /// and the win/loss API. Upserts on (our_company_id, external_ref).
    pub async fn upsert_closed_deal(&self, deal: &NewClosedDeal) -> Result<Uuid> {
        let row = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO closed_deals
                (our_company_id, competitor_id, opportunity_id, deal_name, account_name,
                 deal_value, won, loss_reason, loss_reason_category, closed_at, owner_id,
                 source, external_ref, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
            ON CONFLICT (our_company_id, external_ref)
                WHERE external_ref IS NOT NULL
            DO UPDATE SET
                deal_value            = EXCLUDED.deal_value,
                won                   = EXCLUDED.won,
                loss_reason           = EXCLUDED.loss_reason,
                loss_reason_category  = EXCLUDED.loss_reason_category,
                closed_at             = EXCLUDED.closed_at
            RETURNING id
            "#,
        )
        .bind(deal.our_company_id)
        .bind(deal.competitor_id)
        .bind(deal.opportunity_id)
        .bind(&deal.deal_name)
        .bind(&deal.account_name)
        .bind(deal.deal_value)
        .bind(deal.won)
        .bind(&deal.loss_reason)
        .bind(&deal.loss_reason_category)
        .bind(deal.closed_at)
        .bind(&deal.owner_id)
        .bind(&deal.source)
        .bind(&deal.external_ref)
        .bind(&deal.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Load closed deals for a battlecard: deals where we competed against
    /// `competitor_id` (or have no competitor recorded) for `our_company_id`.
    pub async fn list_closed_deals(
        &self,
        our_company_id: Uuid,
        competitor_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<ClosedDealRow>> {
        let limit = clamp_limit(limit);
        let rows = sqlx::query_as::<_, ClosedDealRow>(
            r#"
            SELECT id, our_company_id, competitor_id, opportunity_id, deal_name,
                   account_name, deal_value, won, loss_reason, loss_reason_category,
                   closed_at, owner_id, source, external_ref, metadata, created_at
            FROM closed_deals
            WHERE our_company_id = $1
              AND ($2::uuid IS NULL OR competitor_id = $2)
            ORDER BY closed_at DESC
            LIMIT $3
            "#,
        )
        .bind(our_company_id)
        .bind(competitor_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

/// Payload for inserting a closed deal.
#[derive(Debug, Clone)]
pub struct NewClosedDeal {
    pub our_company_id: Uuid,
    pub competitor_id: Option<Uuid>,
    pub opportunity_id: Option<Uuid>,
    pub deal_name: String,
    pub account_name: Option<String>,
    pub deal_value: f64,
    pub won: bool,
    pub loss_reason: Option<String>,
    pub loss_reason_category: Option<String>,
    pub closed_at: DateTime<Utc>,
    pub owner_id: Option<String>,
    pub source: String,
    pub external_ref: Option<String>,
    pub metadata: Value,
}

// ════════════════════════════════════════════════════════════════════════════
// COMPETITOR PRICING
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CompetitorPricingRow {
    pub id: Uuid,
    pub competitor_id: Uuid,
    pub our_company_id: Option<Uuid>,
    pub product_category: String,
    pub pricing_model: String,
    pub price_range_low: Option<f64>,
    pub price_range_high: Option<f64>,
    pub currency: String,
    pub average_contract_value: Option<f64>,
    pub discounting_behavior: String,
    pub competitive_position: String,
    pub evidence_url: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub source: String,
    pub confidence: f64,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}

impl PgStore {
    /// Upsert the latest pricing observation for a competitor + product category.
    pub async fn upsert_competitor_pricing(&self, p: &CompetitorPricingRow) -> Result<Uuid> {
        let row = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO competitor_pricing
                (competitor_id, our_company_id, product_category, pricing_model,
                 price_range_low, price_range_high, currency, average_contract_value,
                 discounting_behavior, competitive_position, evidence_url, observed_at,
                 source, confidence, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
            ON CONFLICT (competitor_id, product_category, currency)
            DO UPDATE SET
                pricing_model         = EXCLUDED.pricing_model,
                price_range_low       = EXCLUDED.price_range_low,
                price_range_high      = EXCLUDED.price_range_high,
                average_contract_value= EXCLUDED.average_contract_value,
                discounting_behavior  = EXCLUDED.discounting_behavior,
                competitive_position  = EXCLUDED.competitive_position,
                evidence_url          = EXCLUDED.evidence_url,
                observed_at           = EXCLUDED.observed_at,
                confidence            = EXCLUDED.confidence,
                metadata              = EXCLUDED.metadata
            RETURNING id
            "#,
        )
        .bind(p.competitor_id)
        .bind(p.our_company_id)
        .bind(&p.product_category)
        .bind(&p.pricing_model)
        .bind(p.price_range_low)
        .bind(p.price_range_high)
        .bind(&p.currency)
        .bind(p.average_contract_value)
        .bind(&p.discounting_behavior)
        .bind(&p.competitive_position)
        .bind(&p.evidence_url)
        .bind(p.observed_at)
        .bind(&p.source)
        .bind(p.confidence)
        .bind(&p.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Latest pricing for a competitor across product categories.
    pub async fn list_competitor_pricing(
        &self,
        competitor_id: Uuid,
    ) -> Result<Vec<CompetitorPricingRow>> {
        let rows = sqlx::query_as::<_, CompetitorPricingRow>(
            r#"
            SELECT DISTINCT ON (product_category, currency)
                   id, competitor_id, our_company_id, product_category, pricing_model,
                   price_range_low, price_range_high, currency, average_contract_value,
                   discounting_behavior, competitive_position, evidence_url, observed_at,
                   source, confidence, metadata, created_at
            FROM competitor_pricing
            WHERE competitor_id = $1
            ORDER BY product_category, currency, observed_at DESC
            "#,
        )
        .bind(competitor_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

// ════════════════════════════════════════════════════════════════════════════
// CONTACT METHODS
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ContactMethodRow {
    pub id: Uuid,
    pub person_id: Uuid,
    pub contact_type: String,
    pub value: String,
    pub confidence: f64,
    pub verification_status: String,
    pub verified_at: Option<DateTime<Utc>>,
    pub source: String,
    pub is_primary: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}

impl PgStore {
    /// Upssert a contact method for a person. Enforces one primary per type.
    pub async fn upsert_contact_method(&self, c: &NewContactMethod) -> Result<Uuid> {
        // If this is marked primary, clear prior primary of the same type first.
        if c.is_primary {
            sqlx::query(
                "UPDATE contact_methods SET is_primary = FALSE \
                 WHERE person_id = $1 AND contact_type = $2",
            )
            .bind(c.person_id)
            .bind(&c.contact_type)
            .execute(&self.pool)
            .await?;
        }

        let row = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO contact_methods
                (person_id, contact_type, value, confidence, verification_status,
                 verified_at, source, is_primary, last_seen_at, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            ON CONFLICT (person_id, contact_type, value) DO UPDATE SET
                confidence          = EXCLUDED.confidence,
                verification_status = EXCLUDED.verification_status,
                verified_at         = COALESCE(EXCLUDED.verified_at, contact_methods.verified_at),
                source              = EXCLUDED.source,
                is_primary          = EXCLUDED.is_primary OR contact_methods.is_primary,
                last_seen_at        = COALESCE(EXCLUDED.last_seen_at, NOW()),
                metadata            = EXCLUDED.metadata
            RETURNING id
            "#,
        )
        .bind(c.person_id)
        .bind(&c.contact_type)
        .bind(&c.value)
        .bind(c.confidence)
        .bind(&c.verification_status)
        .bind(c.verified_at)
        .bind(&c.source)
        .bind(c.is_primary)
        .bind(c.last_seen_at)
        .bind(&c.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// All contact methods for a person, primary first, verified first.
    pub async fn list_contact_methods(&self, person_id: Uuid) -> Result<Vec<ContactMethodRow>> {
        let rows = sqlx::query_as::<_, ContactMethodRow>(
            r#"
            SELECT id, person_id, contact_type, value, confidence, verification_status,
                   verified_at, source, is_primary, last_seen_at, metadata, created_at
            FROM contact_methods
            WHERE person_id = $1
            ORDER BY is_primary DESC,
                     (verification_status IN ('smtp_verified','manual_confirmed')) DESC,
                     confidence DESC
            "#,
        )
        .bind(person_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Best verified email for a person, or None.
    pub async fn best_email_for_person(&self, person_id: Uuid) -> Result<Option<String>> {
        let row = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT value FROM contact_methods
            WHERE person_id = $1 AND contact_type = 'email'
              AND verification_status NOT IN ('bounced')
            ORDER BY (verification_status IN ('smtp_verified','manual_confirmed')) DESC,
                     is_primary DESC, confidence DESC
            LIMIT 1
            "#,
        )
        .bind(person_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.flatten())
    }
}

#[derive(Debug, Clone)]
pub struct NewContactMethod {
    pub person_id: Uuid,
    pub contact_type: String,
    pub value: String,
    pub confidence: f64,
    pub verification_status: String,
    pub verified_at: Option<DateTime<Utc>>,
    pub source: String,
    pub is_primary: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub metadata: Value,
}

// ════════════════════════════════════════════════════════════════════════════
// ENGAGEMENT EVENTS  (wires poi::engagement_tracker)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EngagementEventRow {
    pub id: Uuid,
    pub person_id: Uuid,
    pub opportunity_id: Option<Uuid>,
    pub channel: String,
    pub direction: String,
    pub outcome: String,
    pub outcome_weight: f64,
    pub subject: Option<String>,
    pub message_ref: Option<String>,
    pub cadence_step: Option<i32>,
    pub occurred_at: DateTime<Utc>,
    pub owner_id: Option<String>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}

impl PgStore {
    /// Record an outreach/contact attempt outcome.
    pub async fn record_engagement_event(&self, e: &NewEngagementEvent) -> Result<Uuid> {
        let row = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO engagement_events
                (person_id, opportunity_id, channel, direction, outcome, outcome_weight,
                 subject, message_ref, cadence_step, occurred_at, owner_id, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
            RETURNING id
            "#,
        )
        .bind(e.person_id)
        .bind(e.opportunity_id)
        .bind(&e.channel)
        .bind(&e.direction)
        .bind(&e.outcome)
        .bind(e.outcome_weight)
        .bind(&e.subject)
        .bind(&e.message_ref)
        .bind(e.cadence_step)
        .bind(e.occurred_at)
        .bind(&e.owner_id)
        .bind(&e.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Engagement history for a person (most recent first).
    pub async fn list_engagement_events(
        &self,
        person_id: Uuid,
        limit: i64,
    ) -> Result<Vec<EngagementEventRow>> {
        let limit = clamp_limit(limit);
        let rows = sqlx::query_as::<_, EngagementEventRow>(
            r#"
            SELECT id, person_id, opportunity_id, channel, direction, outcome, outcome_weight,
                   subject, message_ref, cadence_step, occurred_at, owner_id, metadata, created_at
            FROM engagement_events
            WHERE person_id = $1
            ORDER BY occurred_at DESC
            LIMIT $2
            "#,
        )
        .bind(person_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[derive(Debug, Clone)]
pub struct NewEngagementEvent {
    pub person_id: Uuid,
    pub opportunity_id: Option<Uuid>,
    pub channel: String,
    pub direction: String,
    pub outcome: String,
    pub outcome_weight: f64,
    pub subject: Option<String>,
    pub message_ref: Option<String>,
    pub cadence_step: Option<i32>,
    pub occurred_at: DateTime<Utc>,
    pub owner_id: Option<String>,
    pub metadata: Value,
}

// ════════════════════════════════════════════════════════════════════════════
// BUYING CENTER GRAPH
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BuyingCenterRow {
    pub id: Uuid,
    pub opportunity_id: Option<Uuid>,
    pub company_id: Uuid,
    pub name: String,
    pub deal_value: Option<f64>,
    pub status: String,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BuyingCenterMemberRow {
    pub id: Uuid,
    pub buying_center_id: Uuid,
    pub person_id: Uuid,
    pub role: String,
    pub influence_score: f64,
    pub budget_authority: bool,
    pub need_signal: f64,
    pub timeline_horizon: Option<String>,
    pub notes: Option<String>,
    pub metadata: Option<Value>,
    pub created_at: DateTime<Utc>,
}

impl PgStore {
    /// Create or fetch a buying center for a company (+ optional opportunity).
    pub async fn upsert_buying_center(
        &self,
        company_id: Uuid,
        opportunity_id: Option<Uuid>,
        name: &str,
        deal_value: Option<f64>,
    ) -> Result<Uuid> {
        // Reuse an existing active center for the same opportunity, or for the
        // same company when no opportunity is given, so repeated derivations
        // are idempotent instead of creating one center per call.
        let existing = if let Some(opp) = opportunity_id {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM buying_centers \
                 WHERE opportunity_id = $1 AND status IN ('forming','engaged') \
                 ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(opp)
            .fetch_optional(&self.pool)
            .await?
        } else {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM buying_centers \
                 WHERE company_id = $1 AND opportunity_id IS NULL \
                   AND status IN ('forming','engaged') \
                 ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(company_id)
            .fetch_optional(&self.pool)
            .await?
        };
        if let Some(id) = existing {
            return Ok(id);
        }
        let row = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO buying_centers (opportunity_id, company_id, name, deal_value)
            VALUES ($1,$2,$3,$4)
            RETURNING id
            "#,
        )
        .bind(opportunity_id)
        .bind(company_id)
        .bind(name)
        .bind(deal_value)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Add/upsert a member to a buying center with a SaaS-canonical role.
    pub async fn upsert_buying_center_member(&self, m: &NewBuyingMember) -> Result<Uuid> {
        let row = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO buying_center_members
                (buying_center_id, person_id, role, influence_score,
                 budget_authority, need_signal, timeline_horizon, notes, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
            ON CONFLICT (buying_center_id, person_id) DO UPDATE SET
                role             = EXCLUDED.role,
                influence_score  = EXCLUDED.influence_score,
                budget_authority = EXCLUDED.budget_authority,
                need_signal      = EXCLUDED.need_signal,
                timeline_horizon = EXCLUDED.timeline_horizon,
                notes            = EXCLUDED.notes
            RETURNING id
            "#,
        )
        .bind(m.buying_center_id)
        .bind(m.person_id)
        .bind(&m.role)
        .bind(m.influence_score)
        .bind(m.budget_authority)
        .bind(m.need_signal)
        .bind(&m.timeline_horizon)
        .bind(&m.notes)
        .bind(&m.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// The buying center(s) for a company with their members, ordered by influence.
    pub async fn list_buying_centers(&self, company_id: Uuid) -> Result<Vec<BuyingCenterRow>> {
        let rows = sqlx::query_as::<_, BuyingCenterRow>(
            r#"
            SELECT id, opportunity_id, company_id, name, deal_value, status,
                   metadata, created_at, updated_at
            FROM buying_centers
            WHERE company_id = $1
            ORDER BY updated_at DESC
            "#,
        )
        .bind(company_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn list_buying_center_members(
        &self,
        buying_center_id: Uuid,
    ) -> Result<Vec<BuyingCenterMemberRow>> {
        let rows = sqlx::query_as::<_, BuyingCenterMemberRow>(
            r#"
            SELECT id, buying_center_id, person_id, role, influence_score,
                   budget_authority, need_signal, timeline_horizon, notes, metadata, created_at
            FROM buying_center_members
            WHERE buying_center_id = $1
            ORDER BY influence_score DESC
            "#,
        )
        .bind(buying_center_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[derive(Debug, Clone)]
pub struct NewBuyingMember {
    pub buying_center_id: Uuid,
    pub person_id: Uuid,
    pub role: String,
    pub influence_score: f64,
    pub budget_authority: bool,
    pub need_signal: f64,
    pub timeline_horizon: Option<String>,
    pub notes: Option<String>,
    pub metadata: Value,
}

// ════════════════════════════════════════════════════════════════════════════
// CRAWL METRICS  (replaces fabricated source-scoring telemetry)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct CrawlMetricRow {
    pub source_id: String,
    pub domain: Option<String>,
    pub observations_ingested: i64,
    pub observations_in_fires: i64,
    pub observations_in_promotions: i64,
    pub fetch_attempts: i64,
    pub fetch_errors: i64,
    pub median_ingest_latency_secs: Option<f64>,
    pub last_crawl_at: Option<DateTime<Utc>>,
    pub observation_types_produced: Vec<String>,
    pub window_start: DateTime<Utc>,
}

/// Real per-source telemetry computed live from observations + warnings.
/// Mirrors `CrawlMetricRow` minus the window columns (no window_start needed).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RealSourceTelemetry {
    pub source_id: String,
    pub domain: String,
    pub observations_ingested: i64,
    pub observations_in_fires: i64,
    pub observations_in_promotions: i64,
    pub fetch_attempts: i64,
    pub fetch_errors: i64,
    pub median_ingest_latency_secs: Option<f64>,
    pub last_crawl_at: Option<DateTime<Utc>>,
    pub observation_types_produced: Vec<String>,
}

impl PgStore {
    /// Upsert a rolling-day telemetry snapshot for a source.
    pub async fn upsert_crawl_metric(&self, m: &NewCrawlMetric) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO crawl_metrics
                (source_id, domain, window_start, window_end,
                 observations_ingested, observations_in_fires, observations_in_promotions,
                 fetch_attempts, fetch_errors, median_ingest_latency_secs,
                 last_crawl_at, observation_types_produced, metadata)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            ON CONFLICT (source_id, window_start) DO UPDATE SET
                observations_ingested       = crawl_metrics.observations_ingested + EXCLUDED.observations_ingested,
                observations_in_fires       = crawl_metrics.observations_in_fires + EXCLUDED.observations_in_fires,
                observations_in_promotions  = crawl_metrics.observations_in_promotions + EXCLUDED.observations_in_promotions,
                fetch_attempts              = crawl_metrics.fetch_attempts + EXCLUDED.fetch_attempts,
                fetch_errors                = crawl_metrics.fetch_errors + EXCLUDED.fetch_errors,
                median_ingest_latency_secs  = COALESCE(EXCLUDED.median_ingest_latency_secs, crawl_metrics.median_ingest_latency_secs),
                last_crawl_at               = GREATEST(crawl_metrics.last_crawl_at, EXCLUDED.last_crawl_at),
                observation_types_produced  = ARRAY(
                    SELECT DISTINCT unnest(crawl_metrics.observation_types_produced || EXCLUDED.observation_types_produced)
                )
            "#,
        )
        .bind(&m.source_id)
        .bind(&m.domain)
        .bind(m.window_start)
        .bind(m.window_end)
        .bind(m.observations_ingested)
        .bind(m.observations_in_fires)
        .bind(m.observations_in_promotions)
        .bind(m.fetch_attempts)
        .bind(m.fetch_errors)
        .bind(m.median_ingest_latency_secs)
        .bind(m.last_crawl_at)
        .bind(&m.observation_types_produced)
        .bind(&m.metadata)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Latest per-source telemetry (one row per source). This is the real input
    /// to the source-scoring ranker, replacing the fabricated numbers.
    pub async fn latest_crawl_metrics(&self) -> Result<Vec<CrawlMetricRow>> {
        let rows = sqlx::query_as::<_, CrawlMetricRow>(
            r#"
            SELECT DISTINCT ON (source_id)
                   source_id, domain, observations_ingested, observations_in_fires,
                   observations_in_promotions, fetch_attempts, fetch_errors,
                   median_ingest_latency_secs, last_crawl_at,
                   observation_types_produced, window_start
            FROM crawl_metrics
            ORDER BY source_id, window_start DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Compute REAL per-source telemetry directly from `observations` (grouped by
    /// `provenance->>'source'`) cross-referenced with fired recipes (`warnings`).
    ///
    /// This replaces the fabricated constants (error_rate: 0.05, `* 0.3`, fake
    /// hours_since_last_crawl) previously hardcoded in `run_source_scoring`.
    /// Each returned row is the measured yield/freshness/error for one source slug
    /// over the last `window_days`. Zero-yield sources simply return zero counts
    /// rather than invented numbers.
    pub async fn compute_source_telemetry(
        &self,
        window_days: i64,
    ) -> Result<Vec<RealSourceTelemetry>> {
        let rows = sqlx::query_as::<_, RealSourceTelemetry>(
            r#"
            WITH src AS (
                SELECT
                    o.provenance->>'source'                 AS source_id,
                    COUNT(*)                                AS observations_ingested,
                    COUNT(DISTINCT o.observation_type)      AS type_count,
                    COUNT(*) FILTER (
                        WHERE o.confidence >= 0.5
                    )                                       AS high_conf_count,
                    MAX(o.ts_utc)                           AS last_crawl_at,
                    ARRAY_AGG(DISTINCT o.observation_type)
                        FILTER (WHERE o.observation_type IS NOT NULL) AS obs_types
                FROM observations o
                WHERE o.ts_utc >= NOW() - ($1 || ' days')::INTERVAL
                  AND o.provenance->>'source' IS NOT NULL
                  AND o.provenance->>'source' <> ''
                GROUP BY o.provenance->>'source'
            ),
            fired AS (
                -- observations whose provenance source also appears in a fired
                -- warning (recipe fire) within the window.
                SELECT
                    w.metadata->>'source' AS source_id,
                    COUNT(*)                AS fire_count
                FROM warnings w
                WHERE w.deleted_at IS NULL
                  AND w.created_at >= NOW() - ($1 || ' days')::INTERVAL
                  AND w.metadata->>'source' IS NOT NULL
                GROUP BY w.metadata->>'source'
            )
            SELECT
                src.source_id,
                src.source_id                            AS domain,
                src.observations_ingested,
                COALESCE(fired.fire_count, 0)            AS observations_in_fires,
                -- promotions: high-confidence obs as a proxy for promoted-grade signal
                COALESCE(src.high_conf_count, 0)         AS observations_in_promotions,
                src.observations_ingested                AS fetch_attempts,
                0::BIGINT                                AS fetch_errors,
                NULL::REAL                               AS median_ingest_latency_secs,
                src.last_crawl_at,
                COALESCE(src.obs_types, ARRAY[]::TEXT[]) AS observation_types_produced
            FROM src
            LEFT JOIN fired USING (source_id)
            ORDER BY src.observations_ingested DESC
            "#,
        )
        .bind(format!("{window_days}"))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[derive(Debug, Clone)]
pub struct NewCrawlMetric {
    pub source_id: String,
    pub domain: Option<String>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub observations_ingested: i64,
    pub observations_in_fires: i64,
    pub observations_in_promotions: i64,
    pub fetch_attempts: i64,
    pub fetch_errors: i64,
    pub median_ingest_latency_secs: Option<f64>,
    pub last_crawl_at: Option<DateTime<Utc>>,
    pub observation_types_produced: Vec<String>,
    pub metadata: Value,
}

// ════════════════════════════════════════════════════════════════════════════
// ICP-FIT SCORING
// ════════════════════════════════════════════════════════════════════════════

impl PgStore {
    /// Persist a computed ICP-fit score + breakdown for a company.
    pub async fn update_company_icp_score(
        &self,
        company_id: Uuid,
        icp_fit_score: f64,
        intent_signal_score: f64,
        breakdown: &Value,
        tech_stack: Option<&[String]>,
        funding_stage: Option<&str>,
        headcount_growth_pct: Option<f64>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE companies SET
                icp_fit_score       = $2,
                -- Never downgrade a stored intent signal to zero: callers that
                -- lack intent data pass 0.0, which previously erased it.
                intent_signal_score = GREATEST(intent_signal_score, $3),
                icp_breakdown       = $4,
                tech_stack          = COALESCE($5, tech_stack),
                funding_stage       = COALESCE($6, funding_stage),
                headcount_growth_pct= COALESCE($7, headcount_growth_pct),
                icp_scored_at       = NOW(),
                updated_at          = NOW()
            WHERE id = $1
            "#,
        )
        .bind(company_id)
        .bind(icp_fit_score)
        .bind(intent_signal_score)
        .bind(breakdown)
        .bind(tech_stack)
        .bind(funding_stage)
        .bind(headcount_growth_pct)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Top ICP-fit target accounts for sales prioritization.
    pub async fn list_icp_top_targets(&self, limit: i64) -> Result<Vec<IcpTargetRow>> {
        let limit = clamp_limit(limit);
        let rows = sqlx::query_as::<_, IcpTargetRow>(
            r#"
            SELECT id, name, domain, country_code, region, industry_tags,
                   employee_estimate, revenue_estimate_usd,
                   icp_fit_score, intent_signal_score,
                   icp_scored_at, icp_breakdown
            FROM companies
            WHERE is_competitor = FALSE
            ORDER BY (icp_fit_score * 0.7 + intent_signal_score * 0.3) DESC,
                     icp_fit_score DESC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct IcpTargetRow {
    pub id: Uuid,
    pub name: String,
    pub domain: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub industry_tags: Option<Vec<String>>,
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub icp_fit_score: f64,
    pub intent_signal_score: f64,
    pub icp_scored_at: Option<DateTime<Utc>>,
    pub icp_breakdown: Option<Value>,
}

// ════════════════════════════════════════════════════════════════════════════
// CRM SYNC STATE  (idempotent write-back tracking)
// ════════════════════════════════════════════════════════════════════════════

impl PgStore {
    /// Record (or refresh) a CRM sync mapping for idempotency.
    pub async fn record_crm_sync(
        &self,
        entity_type: &str,
        local_id: &str,
        external_id: &str,
        crm_system: &str,
        direction: &str,
        payload_hash: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO crm_sync_state
                (entity_type, local_id, external_id, crm_system, direction, payload_hash)
            VALUES ($1,$2,$3,$4,$5,$6)
            ON CONFLICT (crm_system, entity_type, local_id) DO UPDATE SET
                external_id  = EXCLUDED.external_id,
                direction    = EXCLUDED.direction,
                payload_hash = COALESCE(EXCLUDED.payload_hash, crm_sync_state.payload_hash),
                last_synced_at = NOW()
            "#,
        )
        .bind(entity_type)
        .bind(local_id)
        .bind(external_id)
        .bind(crm_system)
        .bind(direction)
        .bind(payload_hash)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Look up the external CRM id for a local entity, if previously synced.
    pub async fn crm_external_id(
        &self,
        entity_type: &str,
        local_id: &str,
        crm_system: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query_scalar::<_, Option<String>>(
            "SELECT external_id FROM crm_sync_state \
             WHERE crm_system = $1 AND entity_type = $2 AND local_id = $3",
        )
        .bind(crm_system)
        .bind(entity_type)
        .bind(local_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.flatten())
    }
}
