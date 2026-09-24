//! System event auto-logger for the activity feed.
//!
//! Records automated system events (insight generation, POI discovery, crawl completion,
//! company detection, threat detection, psych profile updates) into the `activity_feed`
//! table so the frontend activity feed shows real, meaningful events instead of being empty.
//!
//! All logging methods are fire-and-forget: they log errors via `tracing` but never
//! propagate failures to the caller, ensuring pipeline operations cannot be disrupted
//! by a failed activity log write.

use chrono::Utc;
use serde_json::{json, Value};
use sqlx::PgPool;
use tracing::warn;
use uuid::Uuid;

/// A lightweight activity logger that writes system-generated events to the
/// `activity_feed` table.  Designed to be cheap to construct and safe to clone
/// (it holds only an `Arc<PgPool>` internally via the pool's own sharing).
#[derive(Clone)]
pub struct ActivityLogger {
    pool: PgPool,
}

/// Payload for one activity-feed event.
///
/// Bundled into a struct (instead of ten positional parameters) so append-only
/// call sites cannot silently swap two same-typed string arguments.
pub struct ActivityEvent<'a> {
    pub actor_id: &'a str,
    pub actor_name: &'a str,
    pub action_type: &'a str,
    pub entity_type: Option<&'a str>,
    pub entity_id: Option<&'a str>,
    pub entity_name: Option<&'a str>,
    pub details: &'a Value,
    pub workspace_id: Option<Uuid>,
    pub team_id: Option<&'a str>,
    pub visibility: &'a str,
}

impl ActivityLogger {
    /// Create a new activity logger backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ── helpers ────────────────────────────────────────────────────────────

    /// Build the `details` JSON payload with a standard timestamp.
    fn make_details(fields: &[(&str, Value)]) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("logged_at".to_string(), json!(Utc::now().to_rfc3339()));
        for (k, v) in fields {
            map.insert(k.to_string(), v.clone());
        }
        Value::Object(map)
    }

    /// Insert a row and swallow any errors, logging them at `warn` level.
    pub async fn insert(&self, event: ActivityEvent<'_>) {
        let ActivityEvent {
            actor_id,
            actor_name,
            action_type,
            entity_type,
            entity_id,
            entity_name,
            details,
            workspace_id,
            team_id,
            visibility,
        } = event;
        let result = sqlx::query(
            r#"INSERT INTO activity_feed
                 (actor_id, actor_name, action_type, entity_type, entity_id, entity_name,
                  details, workspace_id, team_id, visibility, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())"#,
        )
        .bind(actor_id)
        .bind(actor_name)
        .bind(action_type)
        .bind(entity_type)
        .bind(entity_id)
        .bind(entity_name)
        .bind(details)
        .bind(workspace_id)
        .bind(team_id)
        .bind(visibility)
        .execute(&self.pool)
        .await;

        if let Err(e) = result {
            warn!(
                action_type = %action_type,
                error = %e,
                "activity_logger: failed to insert activity event"
            );
        }
    }

    // ── public logging methods ─────────────────────────────────────────────

    /// Log that a new insight was generated.
    pub async fn log_insight_generated(
        &self,
        entity_name: &str,
        insight_title: &str,
        confidence: f64,
        category: &str,
        entity_id: Option<&str>,
    ) {
        let details = Self::make_details(&[
            ("title", json!(insight_title)),
            ("confidence", json!(confidence)),
            ("category", json!(category)),
        ]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "ApexIntel Engine",
            action_type: "insight_generated",
            entity_type: Some("company"),
            entity_id,
            entity_name: Some(entity_name),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a new Person of Interest was discovered or enriched.
    pub async fn log_poi_discovered(
        &self,
        person_name: &str,
        company_name: &str,
        role_title: &str,
        role_family: &str,
        person_id: Option<&str>,
    ) {
        let details = Self::make_details(&[
            ("company", json!(company_name)),
            ("role", json!(role_title)),
            ("role_family", json!(role_family)),
        ]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "POI Discovery",
            action_type: "poi_discovered",
            entity_type: Some("person"),
            entity_id: person_id,
            entity_name: Some(person_name),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a crawl job completed.
    pub async fn log_crawl_completed(
        &self,
        source_domain: &str,
        urls_crawled: u32,
        new_observations: u32,
        duration_secs: f64,
    ) {
        let details = Self::make_details(&[
            ("urls_crawled", json!(urls_crawled)),
            ("new_observations", json!(new_observations)),
            ("duration_secs", json!(duration_secs)),
        ]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "Crawl Worker",
            action_type: "crawl_completed",
            entity_type: Some("source"),
            entity_id: None,
            entity_name: Some(source_domain),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a new company entity was detected or enriched.
    pub async fn log_company_detected(
        &self,
        company_name: &str,
        region: Option<&str>,
        signal_type: &str,
        company_id: Option<&str>,
    ) {
        let mut fields: Vec<(&str, Value)> = vec![("signal_type", json!(signal_type))];
        if let Some(r) = region {
            fields.push(("region", json!(r)));
        }
        let details = Self::make_details(&fields);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "Company Discovery",
            action_type: "company_detected",
            entity_type: Some("company"),
            entity_id: company_id,
            entity_name: Some(company_name),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a threat was detected for an entity.
    pub async fn log_threat_detected(
        &self,
        threat_type: &str,
        entity_name: &str,
        severity: &str,
        entity_id: Option<&str>,
        entity_type: Option<&str>,
    ) {
        let details = Self::make_details(&[
            ("threat_type", json!(threat_type)),
            ("severity", json!(severity)),
        ]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "Threat Intel",
            action_type: "threat_detected",
            entity_type,
            entity_id,
            entity_name: Some(entity_name),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a psychological profile was updated for a person.
    pub async fn log_psych_profile_updated(
        &self,
        person_name: &str,
        profile_quality: f64,
        person_id: Option<&str>,
    ) {
        let details = Self::make_details(&[("profile_quality", json!(profile_quality))]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "Psych Profiler",
            action_type: "psych_profile_updated",
            entity_type: Some("person"),
            entity_id: person_id,
            entity_name: Some(person_name),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a competitive intelligence battlecard was generated.
    pub async fn log_battlecard_generated(
        &self,
        entity_name: &str,
        competitor_name: &str,
        entity_id: Option<&str>,
    ) {
        let details = Self::make_details(&[("competitor", json!(competitor_name))]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "Battlecard Generator",
            action_type: "battlecard_generated",
            entity_type: Some("company"),
            entity_id,
            entity_name: Some(entity_name),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a weekly memo was generated.
    pub async fn log_memo_generated(&self, memo_title: &str, entity_count: u32) {
        let details = Self::make_details(&[
            ("title", json!(memo_title)),
            ("entity_count", json!(entity_count)),
        ]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "Memo Generator",
            action_type: "memo_generated",
            entity_type: None,
            entity_id: None,
            entity_name: Some("Weekly Memo"),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }

    /// Log that a recipe was promoted from seed to active.
    pub async fn log_recipe_promoted(&self, recipe_code: &str, category: &str) {
        let details = Self::make_details(&[
            ("recipe_code", json!(recipe_code)),
            ("category", json!(category)),
        ]);
        self.insert(ActivityEvent {
            actor_id: "system",
            actor_name: "Recipe Engine",
            action_type: "recipe_promoted",
            entity_type: Some("recipe"),
            entity_id: None,
            entity_name: Some(recipe_code),
            details: &details,
            workspace_id: None,
            team_id: None,
            visibility: "team",
        })
        .await;
    }
}

impl std::fmt::Debug for ActivityLogger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivityLogger").finish_non_exhaustive()
    }
}
