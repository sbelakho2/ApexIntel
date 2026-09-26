#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use anyhow::Result;
use apex_api::auth::ApiRole;
use apex_api::destructive_actions::{
    ApiAuthContext, DELETE_ALL_WARNINGS_CONFIRM_HEADER, DELETE_ALL_WARNINGS_CONFIRM_VALUE,
    DELETE_ALL_WARNINGS_REASON_HEADER,
};
use apex_api::phase01::{build_phase01_router, Phase01State, Phase01Store};
use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Request};
use axum::Router;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use apex_api::auth::{self, ApiKey};
use apex_store::postgres::{
    CertificationRow, CompanyRow, InsightListFilters, InsightRow, PersonRow, SiteRow,
    WarningListFilters, WarningOrderBy, WarningRow,
};

pub fn admin_auth_context() -> ApiAuthContext {
    ApiAuthContext {
        key_id: "admin-key".to_string(),
        user_id: "user-admin".into(),
        role: ApiRole::Admin,
    }
}

pub fn readonly_auth_context() -> ApiAuthContext {
    ApiAuthContext {
        key_id: "viewer-key".to_string(),
        user_id: "user-viewer".into(),
        role: ApiRole::Viewer,
    }
}

pub fn delete_all_warnings_headers(confirmation: Option<&str>, reason: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(value) = confirmation {
        headers.insert(
            DELETE_ALL_WARNINGS_CONFIRM_HEADER,
            HeaderValue::from_str(value).expect("valid confirmation header"),
        );
    }
    if let Some(value) = reason {
        headers.insert(
            DELETE_ALL_WARNINGS_REASON_HEADER,
            HeaderValue::from_str(value).expect("valid reason header"),
        );
    }
    headers
}

pub fn valid_delete_all_warnings_headers(reason: &str) -> HeaderMap {
    delete_all_warnings_headers(Some(DELETE_ALL_WARNINGS_CONFIRM_VALUE), Some(reason))
}

#[derive(Default)]
pub struct FakeStore {
    warnings: Mutex<Vec<WarningRow>>,
    warning_count_sequence: Mutex<Vec<i64>>,
    insights: Mutex<Vec<InsightRow>>,
    company: Mutex<Option<CompanyRow>>,
    sites: Mutex<Vec<SiteRow>>,
    certifications: Mutex<Vec<CertificationRow>>,
    persons: Mutex<Vec<PersonRow>>,
    audit_events: Mutex<Vec<(String, String, Value)>>,
    delete_all_calls: Mutex<usize>,
    health_ok: Mutex<bool>,
}

impl FakeStore {
    pub fn seeded() -> Arc<Self> {
        let now = Utc::now();
        let company_id = Uuid::new_v4();
        Arc::new(Self {
            warnings: Mutex::new(vec![WarningRow {
                id: Uuid::new_v4(),
                recipe_code: Some("R001".to_string()),
                warning_type: "capacity_alert".to_string(),
                title: "Capacity pressure rising".to_string(),
                description: Some("Persistent utilization signals".to_string()),
                severity: "high".to_string(),
                region: Some("US".to_string()),
                source_urls: Some(vec!["https://example.test/warning".to_string()]),
                entity_ids: Some(vec![company_id]),
                confidence: Some(0.81),
                impact: Some("Capacity tightness".to_string()),
                actions: Some(vec!["Acknowledge and assign an owner".to_string()]),
                ts_utc: now,
                acknowledged: false,
                acknowledged_by: None,
                acknowledged_at: None,
                acknowledged_note: None,
                review_outcome: None,
                deleted_at: None,
                created_at: Some(now),
                updated_at: Some(now),
            }]),
            warning_count_sequence: Mutex::new(Vec::new()),
            insights: Mutex::new(vec![InsightRow {
                id: Uuid::new_v4(),
                title: "Competitor procurement shift".to_string(),
                summary: "Supplier mix changed over the last quarter".to_string(),
                insight_type: Some("supply_chain".to_string()),
                region: Some("US".to_string()),
                confidence: Some(0.74),
                evidence_urls: Some(vec!["https://example.test/insight".to_string()]),
                entity_ids: Some(vec![company_id]),
                tags: Some(vec!["procurement".to_string()]),
                metadata: None,
                created_at: Some(now),
                updated_at: Some(now),
            }]),
            company: Mutex::new(Some(CompanyRow {
                id: company_id,
                name: "Acme EMS".to_string(),
                legal_name: Some("Acme EMS Holdings".to_string()),
                domain: Some("acme.test".to_string()),
                country_code: Some("US".to_string()),
                region: Some("US".to_string()),
                company_type: Some("ems".to_string()),
                industry_tags: Some(vec!["pcb".to_string(), "assembly".to_string()]),
                employee_estimate: None,
                revenue_estimate_usd: None,
                risk_score: None,
                threat_score: Some(0.63),
                overlap_score: Some(0.45),
                strategic_relevance: None,
                is_competitor: Some(false),
                metadata: Some(serde_json::json!({"is_competitor": true})),
                created_at: Some(now),
                updated_at: Some(now),
            })),
            sites: Mutex::new(vec![SiteRow {
                id: Uuid::new_v4(),
                company_id: Some(company_id),
                name: "Austin Plant".to_string(),
                address: Some("100 Foundry Way".to_string()),
                city: Some("Austin".to_string()),
                country_code: Some("US".to_string()),
                region: Some("TX".to_string()),
                lat: None,
                lon: None,
                site_type: Some("factory".to_string()),
                capabilities: Some(vec!["SMT".to_string()]),
                certifications: None,
                employee_estimate: None,
                free_zone: None,
                metadata: None,
                created_at: Some(now),
                updated_at: Some(now),
            }]),
            certifications: Mutex::new(vec![CertificationRow {
                id: Uuid::new_v4(),
                company_id: Some(company_id),
                site_id: None,
                standard: "ISO 9001".to_string(),
                status: None,
                issuing_body: None,
                valid_from: None,
                valid_until: None,
                scope: None,
                evidence_url: None,
                metadata: None,
                created_at: Some(now),
                updated_at: Some(now),
            }]),
            persons: Mutex::new(vec![PersonRow {
                id: Uuid::new_v4(),
                name: "Jordan Smith".to_string(),
                name_ar: None,
                name_fr: None,
                primary_org_id: Some(company_id),
                current_role: Some("CEO".to_string()),
                role_family: Some("Executive".to_string()),
                region: Some("US".to_string()),
                country_code: Some("US".to_string()),
                public_bio: None,
                public_email: None,
                priority_vector: None,
                influence_score: Some(0.88),
                trigger_topics: None,
                decision_style: None,
                risk_tolerance: None,
                change_appetite: None,
                communication_style: None,
                decision_mode: None,
                preferred_proof_type: None,
                pain_index: None,
                change_risk: None,
                role_drift_score: None,
                metadata: None,
                created_at: Some(now),
                updated_at: Some(now),
            }]),
            audit_events: Mutex::new(Vec::new()),
            delete_all_calls: Mutex::new(0),
            health_ok: Mutex::new(true),
        })
    }

    pub fn delete_all_call_count(&self) -> usize {
        *self.delete_all_calls.lock().expect("delete calls")
    }

    pub fn audit_events(&self) -> Vec<(String, String, Value)> {
        self.audit_events.lock().expect("audit events").clone()
    }

    pub fn warnings_snapshot(&self) -> Vec<WarningRow> {
        self.warnings.lock().expect("warnings").clone()
    }

    pub fn set_warning_count_sequence(&self, counts: Vec<i64>) {
        *self
            .warning_count_sequence
            .lock()
            .expect("warning count sequence") = counts;
    }

    pub fn seeded_company_id(&self) -> Uuid {
        self.warnings_snapshot()[0]
            .entity_ids
            .as_ref()
            .and_then(|ids| ids.first().copied())
            .expect("seeded company id")
    }
}

#[async_trait]
impl Phase01Store for FakeStore {
    async fn health_check(&self) -> Result<()> {
        if *self.health_ok.lock().expect("health flag") {
            Ok(())
        } else {
            Err(anyhow::anyhow!("database unavailable"))
        }
    }

    async fn count_warnings(&self, filters: &WarningListFilters) -> Result<i64> {
        if let Some(value) = {
            let mut sequence = self
                .warning_count_sequence
                .lock()
                .expect("warning count sequence");
            if sequence.is_empty() {
                None
            } else {
                Some(sequence.remove(0))
            }
        } {
            return Ok(value);
        }
        Ok(self
            .warnings
            .lock()
            .expect("warnings")
            .iter()
            .filter(|warning| filters.include_deleted || warning.deleted_at.is_none())
            .count() as i64)
    }

    async fn list_warnings(
        &self,
        filters: &WarningListFilters,
        _order_by: Option<WarningOrderBy>,
        _desc: bool,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WarningRow>> {
        Ok(self
            .warnings
            .lock()
            .expect("warnings")
            .iter()
            .filter(|warning| filters.include_deleted || warning.deleted_at.is_none())
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
    }

    async fn delete_all_warnings(&self) -> Result<u64> {
        *self.delete_all_calls.lock().expect("delete calls") += 1;
        let mut warnings = self.warnings.lock().expect("warnings");
        let mut deleted_count = 0u64;
        for warning in warnings
            .iter_mut()
            .filter(|warning| warning.deleted_at.is_none())
        {
            warning.deleted_at = Some(Utc::now());
            deleted_count += 1;
        }
        Ok(deleted_count)
    }

    async fn record_audit_event(
        &self,
        actor: &str,
        event_type: &str,
        detail: &Value,
    ) -> Result<()> {
        self.audit_events.lock().expect("audit events").push((
            actor.to_string(),
            event_type.to_string(),
            detail.clone(),
        ));
        Ok(())
    }

    async fn count_insights(&self, _filters: &InsightListFilters) -> Result<i64> {
        Ok(self.insights.lock().expect("insights").len() as i64)
    }

    async fn list_insights(
        &self,
        _filters: &InsightListFilters,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<InsightRow>> {
        Ok(self
            .insights
            .lock()
            .expect("insights")
            .iter()
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
    }

    async fn get_company(&self, id: Uuid) -> Result<Option<CompanyRow>> {
        Ok(self
            .company
            .lock()
            .expect("company")
            .clone()
            .filter(|company| company.id == id))
    }

    async fn get_sites_for_company(&self, id: Uuid) -> Result<Vec<SiteRow>> {
        Ok(self
            .sites
            .lock()
            .expect("sites")
            .iter()
            .filter(|site| site.company_id == Some(id))
            .cloned()
            .collect())
    }

    async fn get_certifications_for_company(&self, id: Uuid) -> Result<Vec<CertificationRow>> {
        Ok(self
            .certifications
            .lock()
            .expect("certifications")
            .iter()
            .filter(|certification| certification.company_id == Some(id))
            .cloned()
            .collect())
    }

    async fn list_persons_by_org(&self, id: Uuid) -> Result<Vec<PersonRow>> {
        Ok(self
            .persons
            .lock()
            .expect("persons")
            .iter()
            .filter(|person| person.primary_org_id == Some(id))
            .cloned()
            .collect())
    }
}

pub fn build_test_router() -> (Router, Arc<FakeStore>) {
    let store = FakeStore::seeded();
    let router = build_phase01_router(Phase01State {
        store: store.clone(),
        api_keys: Arc::new(test_api_keys()),
        started_at: Utc::now(),
        search_ready: true,
    });
    (router, store)
}

pub fn admin_auth_header() -> HeaderValue {
    HeaderValue::from_static("Bearer admin-secret-key")
}

pub fn viewer_auth_header() -> HeaderValue {
    HeaderValue::from_static("Bearer viewer-secret-key")
}

pub fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("request")
}

fn test_api_keys() -> HashMap<String, ApiKey> {
    let now = Utc::now();
    HashMap::from([
        (
            "admin".to_string(),
            ApiKey {
                key_id: "admin-key".to_string(),
                owner_user_id: "user-admin".into(),
                key_hash: auth::hash_api_key("admin-secret-key"),
                name: "Admin".to_string(),
                role: ApiRole::Admin,
                created_at: now,
                expires_at: None,
                enabled: true,
                rate_limit_per_min: 120,
                allowed_origins: vec![],
            },
        ),
        (
            "viewer".to_string(),
            ApiKey {
                key_id: "viewer-key".to_string(),
                owner_user_id: "user-viewer".into(),
                key_hash: auth::hash_api_key("viewer-secret-key"),
                name: "Viewer".to_string(),
                role: ApiRole::Viewer,
                created_at: now,
                expires_at: None,
                enabled: true,
                rate_limit_per_min: 120,
                allowed_origins: vec![],
            },
        ),
    ])
}
