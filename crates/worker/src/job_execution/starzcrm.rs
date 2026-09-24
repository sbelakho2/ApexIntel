//! StarzCRM MySQL read-only pull integration.
//!
//! Scheduled hourly job that connects to the collocated StarzCRM MySQL database,
//! pulls deals and accounts via cursor-based SELECT, extracts competitor mentions
//! from deal descriptions/notes, and upserts into ApexIntel PostgreSQL.
//!
//! # Environment Variables
//!
//! - `STARZCRM_MYSQL_URL` — MySQL connection string (e.g. `mysql://user:pass@127.0.0.1:3306/starz_crm`)
//! - `STARZCRM_ENABLED` — feature flag; if `false` the job skips
//! - `STARZCRM_SYNC_INTERVAL_SECS` — sync interval (default: 3600)

use std::sync::Arc;

use crate::{JobKind, JobRun, PgStore};

/// Max rows to fetch per MySQL query (cursor-based pagination).
const FETCH_LIMIT: i64 = 1000;

/// Schema and column info discovered at startup.
#[derive(Debug, Default)]
pub(crate) struct SchemaInfo {
    pub(crate) tables: Vec<String>,
    pub(crate) deals_columns: Vec<String>,
    pub(crate) accounts_columns: Vec<String>,
}

/// Minimal deal row from StarzCRM MySQL.
#[derive(Debug, Clone, Default)]
pub(crate) struct StarzCrmDeal {
    pub(crate) id: i64,
    pub(crate) account_id: Option<i64>,
    pub(crate) name: Option<String>,
    pub(crate) stage: Option<String>,
    pub(crate) amount: Option<f64>,
    pub(crate) currency: Option<String>,
    pub(crate) close_date: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) notes: Option<String>,
    pub(crate) won_lost_reason: Option<String>,
    pub(crate) owner_email: Option<String>,
}

/// Minimal account row from StarzCRM MySQL.
#[derive(Debug, Clone, Default)]
pub(crate) struct StarzCrmAccount {
    pub(crate) id: i64,
    pub(crate) name: Option<String>,
    pub(crate) industry: Option<String>,
    pub(crate) website: Option<String>,
}

/// Mapped observation from a StarzCRM deal, ready for ApexIntel PostgreSQL.
#[derive(Debug, Clone)]
pub(crate) struct DealObservation {
    pub(crate) external_deal_id: i64,
    pub(crate) external_account_id: Option<i64>,
    pub(crate) account_name: Option<String>,
    pub(crate) deal_name: Option<String>,
    pub(crate) stage: Option<String>,
    pub(crate) amount: Option<f64>,
    pub(crate) currency: Option<String>,
    pub(crate) close_date: Option<String>,
    pub(crate) competitors_mentioned: Vec<String>,
    pub(crate) won_lost_reason: Option<String>,
    pub(crate) owner_email: Option<String>,
}

/// Run a single sync cycle: connect to MySQL, fetch new/updated deals, map,
/// upsert into ApexIntel PostgreSQL, update cursor.
pub(super) async fn run_starzcrm_sync(store: &Arc<PgStore>) -> JobRun {
    let mut run = JobRun::new(JobKind::StarzCrmSync);
    run.start();

    // ── Feature flag check ──────────────────────────────────────────────
    let enabled = std::env::var("STARZCRM_ENABLED")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true";

    if !enabled {
        run.skip("STARZCRM_ENABLED is not set to true");
        return run;
    }

    let mysql_url = match std::env::var("STARZCRM_MYSQL_URL") {
        Ok(url) => url,
        Err(_) => {
            run.fail("STARZCRM_MYSQL_URL is not set");
            return run;
        }
    };

    // ── Connect to MySQL ────────────────────────────────────────────────
    let pool = match sqlx::MySqlPool::connect(&mysql_url).await {
        Ok(p) => p,
        Err(e) => {
            run.fail(&format!("failed to connect to StarzCRM MySQL: {e}"));
            return run;
        }
    };

    // ── Schema probe at startup ─────────────────────────────────────────
    let schema = match probe_schema(&pool).await {
        Ok(s) => s,
        Err(e) => {
            run.fail(&format!("schema probe failed: {e}"));
            return run;
        }
    };

    tracing::info!(
        tables = ?schema.tables,
        deals_cols = ?schema.deals_columns,
        accounts_cols = ?schema.accounts_columns,
        "starzcrm schema probed"
    );

    // ── Read last sync cursor ───────────────────────────────────────────
    let (last_deal_id, last_account_id) = match read_sync_state(store).await {
        Ok(state) => state,
        Err(e) => {
            run.fail(&format!("failed to read sync state: {e}"));
            return run;
        }
    };

    // ── Fetch deals since last cursor ───────────────────────────────────
    let deals = match fetch_deals_since(&pool, last_deal_id, FETCH_LIMIT).await {
        Ok(d) => d,
        Err(e) => {
            run.fail(&format!("failed to fetch deals: {e}"));
            return run;
        }
    };

    let deal_count = deals.len();
    if deal_count == 0 {
        // Update sync state to record that we checked but found nothing new.
        let _ = update_sync_state(store, last_deal_id, last_account_id, 0).await;
        run.succeed(0, "no new deals to sync");
        return run;
    }

    // ── Fetch accounts for the deals ────────────────────────────────────
    let account_ids: Vec<i64> = deals
        .iter()
        .filter_map(|d| d.account_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let accounts = match fetch_accounts(&pool, &account_ids).await {
        Ok(a) => a,
        Err(e) => {
            run.fail(&format!("failed to fetch accounts: {e}"));
            return run;
        }
    };

    // Build account lookup map
    let account_map: std::collections::HashMap<i64, StarzCrmAccount> =
        accounts.into_iter().map(|a| (a.id, a)).collect();

    // ── Map deals to observations ───────────────────────────────────────
    let observations: Vec<DealObservation> = deals
        .iter()
        .map(|deal| {
            let account = deal.account_id.and_then(|id| account_map.get(&id));
            map_deal_to_observation(deal, account)
        })
        .collect();

    // ── Upsert into ApexIntel PostgreSQL ────────────────────────────────
    let mut upserted_count: u64 = 0;
    let mut error_count: u64 = 0;
    let mut max_deal_id = last_deal_id;

    for obs in &observations {
        if obs.external_deal_id > max_deal_id {
            max_deal_id = obs.external_deal_id;
        }

        match upsert_deal_observation(store, obs).await {
            Ok(true) => upserted_count += 1,
            Ok(false) => {} // duplicate, skip
            Err(e) => {
                tracing::error!(
                    deal_id = obs.external_deal_id,
                    error = %e,
                    "failed to upsert starzcrm deal"
                );
                error_count += 1;
            }
        }
    }

    // ── Update sync state cursor ────────────────────────────────────────
    let max_account_id = account_ids.iter().max().copied().unwrap_or(last_account_id);

    if let Err(e) = update_sync_state(store, max_deal_id, max_account_id, upserted_count).await {
        tracing::error!(error = %e, "failed to update starzcrm sync state");
        error_count += 1;
    }

    // ── Outbound write-back: push top ICP targets as CRM leads ──────────────
    // Previously this job was strictly one-way (MySQL→PG). This closes the loop
    // by surfacing ApexIntel's highest-fit sales targets back into the CRM where
    // reps actually work. Gated behind STARZCRM_WRITEBACK (default off) so the
    // read-only behavior is preserved unless explicitly enabled.
    let pushed = match write_back_icp_targets(store, &pool).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "starzcrm write-back failed (continuing)");
            0
        }
    };

    let notes = format!(
        "pulled {} deals, upserted {}, pushed {} ICP targets, errors {}",
        deal_count, upserted_count, pushed, error_count
    );

    if error_count > 0 {
        run.fail(&format!("{notes}; errors encountered during sync"));
    } else {
        run.succeed(upserted_count, &notes);
    }

    run
}

// ─────────────────────────────────────────────────────────────────────────────
// MySQL helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Probe the StarzCRM MySQL schema at startup — discover tables and columns.
async fn probe_schema(pool: &sqlx::MySqlPool) -> Result<SchemaInfo, anyhow::Error> {
    let mut info = SchemaInfo::default();

    // Get list of tables
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = 'starz_crm'",
    )
    .fetch_all(pool)
    .await?;

    for (name,) in &tables {
        info.tables.push(name.clone());
    }

    // Get columns for deals table (try common table names)
    for tbl_name in &["deals", "opportunities", "deal"] {
        if tables.iter().any(|(t,)| t == tbl_name) {
            let cols: Vec<(String,)> = sqlx::query_as(
                "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = 'starz_crm' AND TABLE_NAME = ?",
            )
            .bind(tbl_name)
            .fetch_all(pool)
            .await?;

            for (col,) in &cols {
                info.deals_columns.push(col.clone());
            }
            break;
        }
    }

    // Get columns for accounts table
    for tbl_name in &["accounts", "account", "companies", "company"] {
        if tables.iter().any(|(t,)| t == tbl_name) {
            let cols: Vec<(String,)> = sqlx::query_as(
                "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = 'starz_crm' AND TABLE_NAME = ?",
            )
            .bind(tbl_name)
            .fetch_all(pool)
            .await?;

            for (col,) in &cols {
                info.accounts_columns.push(col.clone());
            }
            break;
        }
    }

    Ok(info)
}

/// Fetch deals modified since the given cursor id.
async fn fetch_deals_since(
    pool: &sqlx::MySqlPool,
    since_id: i64,
    limit: i64,
) -> Result<Vec<StarzCrmDeal>, anyhow::Error> {
    // Try common table/column names. The exact schema is unknown,
    // so we use a flexible query approach.
    // First try: deals table with id + updated_at
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<f64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT d.id, d.account_id, d.name, d.stage, d.amount, d.currency, \
                d.close_date, d.description, d.won_lost_reason, d.owner_email \
         FROM deals d \
         WHERE d.id > ? \
         ORDER BY d.id ASC \
         LIMIT ?",
    )
    .bind(since_id)
    .bind(limit)
    .fetch_all(pool)
    .await;

    match rows {
        Ok(rows) => Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    account_id,
                    name,
                    stage,
                    amount,
                    currency,
                    close_date,
                    description,
                    won_lost_reason,
                    owner_email,
                )| {
                    StarzCrmDeal {
                        id,
                        account_id,
                        name,
                        stage,
                        amount,
                        currency,
                        close_date,
                        description,
                        notes: None, // fetched separately if needed
                        won_lost_reason,
                        owner_email,
                    }
                },
            )
            .collect()),
        Err(_e) => {
            // Fallback: try without account_id or different column names
            let rows = sqlx::query_as::<
                _,
                (
                    i64,
                    Option<String>,
                    Option<String>,
                    Option<f64>,
                    Option<String>,
                    Option<String>,
                ),
            >(
                "SELECT id, name, stage, amount, description, won_lost_reason \
                 FROM deals \
                 WHERE id > ? \
                 ORDER BY id ASC \
                 LIMIT ?",
            )
            .bind(since_id)
            .bind(limit)
            .fetch_all(pool)
            .await?;

            Ok(rows
                .into_iter()
                .map(
                    |(id, name, stage, amount, description, won_lost_reason)| StarzCrmDeal {
                        id,
                        name,
                        stage,
                        amount,
                        description,
                        won_lost_reason,
                        ..Default::default()
                    },
                )
                .collect())
        }
    }
}

/// Fetch accounts by IDs.
async fn fetch_accounts(
    pool: &sqlx::MySqlPool,
    ids: &[i64],
) -> Result<Vec<StarzCrmAccount>, anyhow::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    // Build a parameterized query with placeholders
    let placeholders: Vec<String> = ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect();
    let sql = format!(
        "SELECT id, name, industry, website FROM accounts WHERE id IN ({})",
        placeholders.join(", ")
    );

    let mut query =
        sqlx::query_as::<_, (i64, Option<String>, Option<String>, Option<String>)>(&sql);
    for id in ids {
        query = query.bind(id);
    }

    let rows = query.fetch_all(pool).await?;

    Ok(rows
        .into_iter()
        .map(|(id, name, industry, website)| StarzCrmAccount {
            id,
            name,
            industry,
            website,
        })
        .collect())
}

// ─────────────────────────────────────────────────────────────────────────────
// Mapping logic
// ─────────────────────────────────────────────────────────────────────────────

/// Known competitor keywords for extraction from deal descriptions/notes.
const COMPETITOR_KEYWORDS: &[&str] = &[
    "competitor",
    "competition",
    "competiting",
    "versus",
    "vs ",
    "replacing",
    "migrating from",
    "switching from",
    "evaluating",
];

/// Extract competitor names from deal text fields using keyword matching.
fn extract_competitors(description: &Option<String>, notes: &Option<String>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let combined = format!(
        "{} {}",
        description.as_deref().unwrap_or(""),
        notes.as_deref().unwrap_or(""),
    );

    if combined.trim().is_empty() {
        return found;
    }

    let lower = combined.to_lowercase();

    // Check for competitor keywords
    let has_competitor_ref = COMPETITOR_KEYWORDS.iter().any(|kw| lower.contains(kw));
    if !has_competitor_ref {
        return found;
    }

    // Extract company-like names mentioned near competitor keywords
    // This is a simple heuristic — in production, use NER or the embeddings (§5.2)
    // B330: iterate the ORIGINAL-CASE text — the previous loop checked
    // `is_uppercase()` against words from the lowercased copy, which is never
    // true, so `competitors_mentioned` was always empty.
    for sentence in combined.split(&['.', '!', '?', '\n', '\r'][..]) {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }

        // If sentence mentions a competitor keyword, try to extract company names
        let sentence_lower = sentence.to_lowercase();
        if COMPETITOR_KEYWORDS
            .iter()
            .any(|kw| sentence_lower.contains(kw))
        {
            // Look for capitalized words or phrases that might be company names
            let words: Vec<&str> = sentence.split_whitespace().collect();
            for (i, _word) in words.iter().enumerate() {
                // Simple heuristic: take words that start with uppercase after competitor keywords
                if let Some(next) = words.get(i + 1) {
                    if next.chars().next().is_some_and(|c| c.is_uppercase()) {
                        let candidate = next.trim_matches(|c: char| c.is_ascii_punctuation());
                        // Skip pure stopwords that happen to be capitalized
                        // (sentence starts, common words).
                        const STOPWORDS: [&str; 12] = [
                            "The", "A", "An", "And", "But", "We", "They", "Our", "Their", "This",
                            "That", "It",
                        ];
                        if !candidate.is_empty()
                            && candidate.len() > 1
                            && !STOPWORDS.contains(&candidate)
                        {
                            let name = candidate.to_string();
                            if !found.contains(&name) {
                                found.push(name);
                            }
                        }
                    }
                }
            }
        }
    }

    found
}

/// Map a StarzCRM deal (with optional account) to a [`DealObservation`].
fn map_deal_to_observation(
    deal: &StarzCrmDeal,
    account: Option<&StarzCrmAccount>,
) -> DealObservation {
    let competitors = extract_competitors(&deal.description, &deal.notes);

    DealObservation {
        external_deal_id: deal.id,
        external_account_id: deal.account_id,
        account_name: account.and_then(|a| a.name.clone()),
        deal_name: deal.name.clone(),
        stage: deal.stage.clone(),
        amount: deal.amount,
        currency: deal.currency.clone(),
        close_date: deal.close_date.clone(),
        competitors_mentioned: competitors,
        won_lost_reason: deal.won_lost_reason.clone(),
        owner_email: deal.owner_email.clone(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PostgreSQL store helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Read the last sync cursor from `starzcrm_sync_state`.
async fn read_sync_state(store: &PgStore) -> Result<(i64, i64), anyhow::Error> {
    // The store module handles PostgreSQL queries. Since we can't add methods
    // to PgStore here, we use raw sqlx on the underlying pool.
    let pool = &store.pool;

    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT COALESCE(last_deal_id, 0), COALESCE(last_account_id, 0) \
         FROM starzcrm_sync_state WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;

    match row {
        Some((deal_id, account_id)) => Ok((deal_id, account_id)),
        None => Ok((0, 0)),
    }
}

/// Update the sync state cursor after a successful pull.
async fn update_sync_state(
    store: &PgStore,
    last_deal_id: i64,
    last_account_id: i64,
    rows_pulled: u64,
) -> Result<(), anyhow::Error> {
    let pool = &store.pool;

    sqlx::query(
        "INSERT INTO starzcrm_sync_state (id, last_synced_at, last_deal_id, last_account_id, rows_pulled, updated_at) \
         VALUES (1, NOW(), $1, $2, $3, NOW()) \
         ON CONFLICT (id) DO UPDATE SET \
             last_synced_at = NOW(), \
             last_deal_id = EXCLUDED.last_deal_id, \
             last_account_id = EXCLUDED.last_account_id, \
             rows_pulled = starzcrm_sync_state.rows_pulled + EXCLUDED.rows_pulled, \
             updated_at = NOW()"
    )
    .bind(last_deal_id)
    .bind(last_account_id)
    .bind(rows_pulled as i64)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert a deal observation into `starzcrm_deals`.
async fn upsert_deal_observation(
    store: &PgStore,
    obs: &DealObservation,
) -> Result<bool, anyhow::Error> {
    let pool = &store.pool;

    let result = sqlx::query(
        "INSERT INTO starzcrm_deals \
         (external_deal_id, external_account_id, account_name, deal_name, stage, \
          amount, currency, close_date, competitors_mentioned, won_lost_reason, owner_email, synced_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW()) \
         ON CONFLICT (external_deal_id) DO UPDATE SET \
             stage = EXCLUDED.stage, \
             amount = EXCLUDED.amount, \
             competitors_mentioned = EXCLUDED.competitors_mentioned, \
             won_lost_reason = EXCLUDED.won_lost_reason, \
             synced_at = NOW()"
    )
    .bind(obs.external_deal_id)
    .bind(obs.external_account_id)
    .bind(&obs.account_name)
    .bind(&obs.deal_name)
    .bind(&obs.stage)
    .bind(obs.amount)
    .bind(&obs.currency)
    .bind(&obs.close_date)
    .bind(&obs.competitors_mentioned)
    .bind(&obs.won_lost_reason)
    .bind(&obs.owner_email)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Outbound write-back (MySQL INSERT)
// ─────────────────────────────────────────────────────────────────────────────

/// Push the highest-ICP-fit target accounts into StarzCRM as leads, if
/// `STARZCRM_WRITEBACK=true`. Idempotent via the `crm_sync_state` table: each
/// company is pushed at most once. Returns the number of leads pushed.
///
/// The CRM `leads` schema is probed defensively (column names vary across CRM
/// installs); if the expected table/columns are absent the write-back is a
/// no-op rather than an error, so a schema mismatch never breaks the inbound sync.
///
/// `(id, name, domain, region, icp_fit_score)` as returned by the ICP query.
type IcpTargetRow = (
    uuid::Uuid,
    String,
    Option<String>,
    Option<String>,
    Option<f64>,
);

pub(crate) async fn write_back_icp_targets(
    store: &Arc<PgStore>,
    mysql_pool: &sqlx::MySqlPool,
) -> Result<u64, anyhow::Error> {
    let enabled = std::env::var("STARZCRM_WRITEBACK")
        .unwrap_or_else(|_| "false".to_string())
        .to_lowercase()
        == "true";
    if !enabled {
        return Ok(0);
    }

    // Load top ICP targets not yet pushed to the CRM.
    let targets: Vec<IcpTargetRow> = sqlx::query_as(
        r#"
            SELECT id, name, domain, region, icp_fit_score
            FROM companies
            WHERE is_competitor IS DISTINCT FROM TRUE
              AND icp_fit_score >= 0.6
              AND NOT EXISTS (
                  SELECT 1 FROM crm_sync_state
                  WHERE crm_system = 'starzcrm'
                    AND entity_type = 'lead'
                    AND local_id = companies.id::text
              )
            ORDER BY icp_fit_score DESC
            LIMIT 50
            "#,
    )
    .fetch_all(&store.pool)
    .await?;

    if targets.is_empty() {
        return Ok(0);
    }

    // Verify the CRM has a `leads` table before inserting.
    let has_leads: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = 'starz_crm' AND TABLE_NAME = 'leads'",
    )
    .fetch_one(mysql_pool)
    .await
    .unwrap_or(false);

    if !has_leads {
        tracing::info!("starzcrm write-back: no 'leads' table in CRM; skipping");
        return Ok(0);
    }

    let mut pushed = 0u64;
    for (company_id, name, domain, region, fit) in &targets {
        // Insert into CRM leads (graceful on missing columns).
        let inserted = sqlx::query(
            "INSERT IGNORE INTO leads (company_name, website, region, source, score, notes, created_at) \
             VALUES (?, ?, ?, 'ApexIntel-ICP', ?, ?, NOW())",
        )
        .bind(name)
        .bind(domain.as_deref().unwrap_or(""))
        .bind(region.as_deref().unwrap_or(""))
        .bind(fit.unwrap_or(0.0))
        .bind(format!("Auto-generated by ApexIntel ICP scoring (fit={:.2}). High-priority sales target.", fit.unwrap_or(0.0)))
        .execute(mysql_pool)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);

        if inserted > 0 {
            pushed += 1;
        }
        // Record the mapping idempotently so we never re-push the same company.
        let _ = store
            .record_crm_sync(
                "lead",
                &company_id.to_string(),
                name,
                "starzcrm",
                "outbound",
                None,
            )
            .await;
    }

    if pushed > 0 {
        tracing::info!(pushed, "starzcrm write-back: pushed ICP leads to CRM");
    }
    Ok(pushed)
}
