//! Person handler — GET /persons/:id (detail)
//!
//! Covers: person detail page with bio, affiliations, role history,
//! priority vector, peer summary, and engagement timeline.

use std::sync::Arc;

use askama::Template;
use axum::{extract::Path, extract::Query, http::HeaderMap, response::IntoResponse, Extension};
use uuid::Uuid;

use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use apex_store::postgres::{PersonListFilters, PersonOrderBy, PgStore, WarningListFilters};

fn normalize_percent(value: f64) -> f64 {
    let normalized = if value <= 1.0 { value * 100.0 } else { value };
    normalized.clamp(0.0, 100.0).round()
}

fn normalize_ratio(value: f64) -> f64 {
    let normalized = if value <= 1.0 { value } else { value / 100.0 };
    normalized.clamp(0.0, 1.0)
}

fn priority_metric(vector: &Option<serde_json::Value>, key: &str) -> Option<f64> {
    vector
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_f64())
        .map(normalize_ratio)
}

fn average_priority_metrics(vector: &Option<serde_json::Value>, keys: &[&str]) -> Option<f64> {
    let values: Vec<f64> = keys
        .iter()
        .filter_map(|key| priority_metric(vector, key))
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

fn role_exposure_risk(role_family: Option<&str>, current_role: Option<&str>) -> f64 {
    let role = current_role.unwrap_or_default().to_lowercase();
    let family = role_family.unwrap_or_default().to_lowercase();

    if role.contains("ceo")
        || role.contains("chief")
        || role.contains("president")
        || role.contains("chair")
        || family.contains("executive")
        || family.contains("board")
    {
        0.60
    } else if role.contains("minister")
        || role.contains("general")
        || role.contains("director")
        || family.contains("government")
        || family.contains("military")
        || family.contains("security")
    {
        0.45
    } else if family.contains("procurement")
        || family.contains("operations")
        || family.contains("engineering")
        || family.contains("legal")
        || family.contains("finance")
    {
        0.30
    } else {
        0.18
    }
}

#[allow(clippy::too_many_arguments)]
fn derive_profile_priority_vector(
    person: &apex_store::postgres::PersonRow,
    artifact_count: usize,
    role_history_count: usize,
    affiliation_count: usize,
    peer_count: usize,
    warning_count: i64,
    insight_count: i64,
    recent_change_count: usize,
) -> PriorityVector {
    let influence_ratio = priority_metric(&person.priority_vector, "influence")
        .or(person.influence_score.map(normalize_ratio))
        .unwrap_or(0.0);

    let live_connectivity = 0.45 * (peer_count as f64 / 8.0).min(1.0)
        + 0.20 * (affiliation_count as f64 / 4.0).min(1.0)
        + 0.15 * (role_history_count as f64 / 6.0).min(1.0)
        + 0.10 * if person.primary_org_id.is_some() { 1.0 } else { 0.0 }
        + 0.10 * if person.public_email.as_ref().is_some_and(|value| !value.trim().is_empty()) {
            1.0
        } else {
            0.0
        };
    let connectivity_ratio = priority_metric(&person.priority_vector, "connectivity")
        .or(average_priority_metrics(
            &person.priority_vector,
            &["network_centrality", "domain_relevance"],
        ))
        .unwrap_or(live_connectivity);

    let live_activity = 0.35 * (artifact_count as f64 / 12.0).min(1.0)
        + 0.25 * (recent_change_count as f64 / 8.0).min(1.0)
        + 0.20 * (warning_count as f64 / 6.0).min(1.0)
        + 0.10 * (insight_count as f64 / 6.0).min(1.0)
        + 0.10 * {
            let freshness_days = person
                .updated_at
                .or(person.created_at)
                .map(|ts| chrono::Utc::now().signed_duration_since(ts).num_days())
                .unwrap_or(365);
            if freshness_days <= 7 {
                1.0
            } else if freshness_days <= 30 {
                0.65
            } else if freshness_days <= 90 {
                0.35
            } else {
                0.12
            }
        };
    let activity_ratio = priority_metric(&person.priority_vector, "activity")
        .or(priority_metric(&person.priority_vector, "engagement_potential"))
        .unwrap_or(live_activity);

    let metadata_change_risk = person
        .metadata
        .as_ref()
        .and_then(|value| value.get("change_risk"))
        .and_then(|value| value.as_f64())
        .map(normalize_ratio)
        .unwrap_or(0.0);
    let metadata_role_drift = person
        .metadata
        .as_ref()
        .and_then(|value| value.get("role_drift_score"))
        .and_then(|value| value.as_f64())
        .map(normalize_ratio)
        .unwrap_or(0.0);
    let metadata_pain = person
        .metadata
        .as_ref()
        .and_then(|value| value.get("pain_index"))
        .and_then(|value| value.as_f64())
        .map(normalize_ratio)
        .unwrap_or(0.0);
    let stored_risk = average_priority_metrics(
        &person.priority_vector,
        &["risk", "security", "compliance", "resilience"],
    )
    .unwrap_or(0.0);
    let baseline_risk = 0.45 * influence_ratio
        + 0.35 * role_exposure_risk(person.role_family.as_deref(), person.current_role.as_deref())
        + 0.10 * if artifact_count > 0 { 1.0 } else { 0.0 }
        + 0.10 * if person.primary_org_id.is_some() { 1.0 } else { 0.0 };
    let live_risk = 0.40 * (warning_count as f64 / 6.0).min(1.0)
        + 0.20 * (recent_change_count as f64 / 8.0).min(1.0)
        + 0.15 * metadata_change_risk
        + 0.15 * metadata_role_drift
        + 0.10 * metadata_pain;
    let risk_ratio = priority_metric(&person.priority_vector, "risk")
        .unwrap_or((stored_risk * 0.25 + live_risk * 0.35 + baseline_risk * 0.40).clamp(0.0, 1.0));

    let overall_ratio = priority_metric(&person.priority_vector, "overall").unwrap_or(
        (0.35 * influence_ratio + 0.20 * connectivity_ratio + 0.20 * activity_ratio + 0.25 * risk_ratio)
            .clamp(0.0, 1.0),
    );

    PriorityVector {
        influence: normalize_percent(influence_ratio),
        connectivity: normalize_percent(connectivity_ratio),
        activity: normalize_percent(activity_ratio),
        risk: normalize_percent(risk_ratio),
        overall: normalize_percent(overall_ratio),
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct PersonListQuery {
    pub region: Option<String>,
    pub priority: Option<String>,
    pub q: Option<String>,
}

const PERSON_REGION_FILTERS: [&str; 6] = ["Tunisia", "Morocco", "Israel", "EU", "China", "Global"];

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct PersonAffiliation {
    pub company_id: String,
    pub company_name: String,
    pub role: String,
    pub since: String,
    pub until: Option<String>,
    pub is_current: bool,
}

#[derive(Clone, Debug)]
pub struct PersonRoleHistory {
    pub company_name: String,
    pub role: String,
    pub start_date: String,
    pub end_date: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PersonPeer {
    pub id: String,
    pub name: String,
    pub role: String,
    pub shared_company: String,
}

#[derive(Clone, Debug)]
pub struct PersonEvent {
    pub kind: String,
    pub description: String,
    pub date: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct PriorityVector {
    pub influence: f64,
    pub connectivity: f64,
    pub activity: f64,
    pub risk: f64,
    pub overall: f64,
}

#[derive(Clone, Debug)]
pub struct PersonListCard {
    pub id: String,
    pub name: String,
    pub role: String,
    pub organization: String,
    pub region: String,
    pub priority: String,
    pub influence_score: i64,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct InfluenceGroup {
    pub range: String,
    pub count: i64,
    pub pct: i64,
}

#[derive(Clone, Debug)]
pub struct RegionFilterChip {
    pub label: String,
    pub href: String,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct PriorityFilterChip {
    pub label: String,
    pub href: String,
    pub active: bool,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/person_detail.html")]
pub struct PersonDetailPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub briefing_mode: bool,

    pub id: String,
    pub name: String,
    pub title: String,
    pub bio: String,
    pub region: String,
    pub priority_tier: String,
    pub priority_vector: PriorityVector,
    pub affiliations: Vec<PersonAffiliation>,
    pub role_history: Vec<PersonRoleHistory>,
    pub peers: Vec<PersonPeer>,
    pub recent_events: Vec<PersonEvent>,
    pub warning_count_person: i64,
    pub insight_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct PersonDetailQuery {
    pub briefing: Option<bool>,
}

#[derive(Template)]
#[template(path = "pages/persons.html")]
pub struct PersonsPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub persons: Vec<PersonListCard>,
    pub total_persons: i64,
    pub priority_a: i64,
    pub priority_b: i64,
    pub avg_influence: i64,
    pub influence_groups: Vec<InfluenceGroup>,
    pub region_filters: Vec<RegionFilterChip>,
    pub priority_filters: Vec<PriorityFilterChip>,
    pub active_filters: i64,
    pub reset_href: String,
    pub search_query: String,
}

fn url_encode_component(input: &str) -> String {
    // Minimal URL encoding for query values used by region and search params.
    input
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('+', "%2B")
}

fn build_persons_href(region: Option<&str>, priority: Option<&str>, q: Option<&str>) -> String {
    let mut params: Vec<String> = Vec::new();
    if let Some(region) = region {
        let region = region.trim();
        if !region.is_empty() {
            params.push(format!("region={}", url_encode_component(region)));
        }
    }
    if let Some(priority) = priority {
        let priority = priority.trim();
        if !priority.is_empty() {
            params.push(format!("priority={}", url_encode_component(priority)));
        }
    }
    if let Some(q) = q {
        let q = q.trim();
        if !q.is_empty() {
            params.push(format!("q={}", url_encode_component(q)));
        }
    }

    if params.is_empty() {
        "/persons".to_string()
    } else {
        format!("/persons?{}", params.join("&"))
    }
}

// ─── Handler ────────────────────────────────────────────────────────────────

pub async fn list_persons(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Query(query): Query<PersonListQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/persons", unack);

    let all_rows = store
        .list_persons(
            &PersonListFilters::default(),
            Some(PersonOrderBy::UpdatedAt),
            true,
            500,
            0,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to list persons for web page: {e}");
            vec![]
        });

    let selected_region = query.region.clone().unwrap_or_default();
    let selected_priority = query.priority.clone().unwrap_or_default();
    let selected_q = query.q.clone().unwrap_or_default();
    let selected_region_lc = selected_region.to_lowercase();
    let selected_q_lc = selected_q.to_lowercase();

    let filtered_rows = all_rows
        .iter()
        .filter(|row| {
            if !selected_region_lc.is_empty() {
                if selected_region_lc == "global" {
                    // Global means show all regions.
                } else if row.region.trim().to_lowercase() != selected_region_lc {
                    return false;
                }
            }

            let score = (row.priority_score * 100.0).round() as i64;
            if !match selected_priority.as_str() {
                "A" => score >= 80,
                "B" => (50..80).contains(&score),
                "C" => score < 50,
                _ => true,
            } {
                return false;
            }

            if !selected_q_lc.is_empty() {
                let haystack = format!(
                    "{} {} {} {}",
                    row.name, row.role, row.organization, row.role_family
                )
                .to_lowercase();
                if !haystack.contains(&selected_q_lc) {
                    return false;
                }
            }

            true
        })
        .cloned()
        .collect::<Vec<_>>();

    let total_persons = filtered_rows.len() as i64;

    let mut priority_a = 0_i64;
    let mut priority_b = 0_i64;
    let mut influence_sum = 0_i64;
    let mut groups = [0_i64; 5];

    for row in &filtered_rows {
        let score = (row.priority_score * 100.0).round() as i64;
        influence_sum += score;

        if score >= 80 {
            priority_a += 1;
        } else if score >= 50 {
            priority_b += 1;
        }

        match score {
            x if x < 20 => groups[0] += 1,
            x if x < 40 => groups[1] += 1,
            x if x < 60 => groups[2] += 1,
            x if x < 80 => groups[3] += 1,
            _ => groups[4] += 1,
        }
    }

    let avg_influence = if total_persons > 0 {
        influence_sum / total_persons
    } else {
        0
    };

    let max_group = groups.iter().copied().max().unwrap_or(0).max(1);
    let influence_groups = vec![
        InfluenceGroup {
            range: "0-20".into(),
            count: groups[0],
            pct: ((groups[0] * 100) / max_group).max(6),
        },
        InfluenceGroup {
            range: "20-40".into(),
            count: groups[1],
            pct: ((groups[1] * 100) / max_group).max(6),
        },
        InfluenceGroup {
            range: "40-60".into(),
            count: groups[2],
            pct: ((groups[2] * 100) / max_group).max(6),
        },
        InfluenceGroup {
            range: "60-80".into(),
            count: groups[3],
            pct: ((groups[3] * 100) / max_group).max(6),
        },
        InfluenceGroup {
            range: "80-100".into(),
            count: groups[4],
            pct: ((groups[4] * 100) / max_group).max(6),
        },
    ];

    let persons = filtered_rows
        .iter()
        .map(|row| {
            let score = (row.priority_score * 100.0).round() as i64;
            let priority = if score >= 80 {
                "A"
            } else if score >= 50 {
                "B"
            } else {
                "C"
            };
            let mut tags = Vec::new();
            if !row.role_family.trim().is_empty() {
                tags.push(row.role_family.clone());
            }
            if !row.country.trim().is_empty() {
                tags.push(row.country.clone());
            }
            PersonListCard {
                id: row.id.to_string(),
                name: row.name.clone(),
                role: row.role.clone(),
                organization: row.organization.clone(),
                region: row.region.clone(),
                priority: priority.to_string(),
                influence_score: score,
                tags,
            }
        })
        .collect::<Vec<_>>();

    let region_filters = PERSON_REGION_FILTERS
        .iter()
        .map(|region| {
            let active = selected_region.eq_ignore_ascii_case(region);
            let href = if active {
                build_persons_href(None, Some(&selected_priority), Some(&selected_q))
            } else {
                build_persons_href(Some(region), Some(&selected_priority), Some(&selected_q))
            };
            RegionFilterChip {
                href,
                active,
                label: (*region).to_string(),
            }
        })
        .collect::<Vec<_>>();

    let priority_filters = ["A", "B", "C"]
        .iter()
        .map(|priority| {
            let active = selected_priority == *priority;
            let href = if active {
                build_persons_href(Some(&selected_region), None, Some(&selected_q))
            } else {
                build_persons_href(Some(&selected_region), Some(priority), Some(&selected_q))
            };
            PriorityFilterChip {
                href,
                active,
                label: (*priority).to_string(),
            }
        })
        .collect::<Vec<_>>();

    let active_filters = i64::from(!selected_region.is_empty())
        + i64::from(!selected_priority.is_empty())
        + i64::from(!selected_q.is_empty());

    let tpl = PersonsPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        persons,
        total_persons,
        priority_a,
        priority_b,
        avg_influence,
        influence_groups,
        region_filters,
        priority_filters,
        active_filters,
        reset_href: "/persons".into(),
        search_query: selected_q,
    };

    let _ = is_htmx_request(&headers);
    super::render_template(&tpl)
}

/// GET /persons/:id — person of interest detail page.
pub async fn get_person(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
    Path(id): Path<String>,
    Query(query): Query<PersonDetailQuery>,
) -> impl IntoResponse {
    let unack = store
        .count_warnings(&WarningListFilters {
            acknowledged: Some(false),
            ..Default::default()
        })
        .await
        .unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/persons", unack);

    let uuid = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return super::errors::not_found_with_context(
                &ctx.username,
                "/persons",
                ctx.warning_count,
            );
        }
    };

    let person = match store.get_person(uuid).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return super::errors::not_found_with_context(
                &ctx.username,
                "/persons",
                ctx.warning_count,
            );
        }
        Err(e) => {
            tracing::error!("Failed to fetch person {id}: {e}");
            return super::errors::not_found_with_context(
                &ctx.username,
                "/persons",
                ctx.warning_count,
            );
        }
    };

    let organization_name = match person.primary_org_id {
        Some(org_id) => store
            .get_company(org_id)
            .await
            .ok()
            .flatten()
            .map(|company| company.name)
            .unwrap_or_default(),
        None => String::new(),
    };

    // Fetch role history
    let rh_rows = store
        .get_role_history_for_person(uuid)
        .await
        .unwrap_or_default();
    let mut role_history: Vec<PersonRoleHistory> = rh_rows
        .iter()
        .map(|rh| PersonRoleHistory {
            company_name: rh.org_name.clone(),
            role: rh.title.clone(),
            start_date: rh
                .start_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            end_date: rh.end_date.map(|d| d.format("%Y-%m-%d").to_string()),
        })
        .collect();

    // Build affiliations from role history (current = no end_date)
    let mut affiliations: Vec<PersonAffiliation> = rh_rows
        .iter()
        .map(|rh| PersonAffiliation {
            company_id: rh.org_id.map(|u| u.to_string()).unwrap_or_default(),
            company_name: rh.org_name.clone(),
            role: rh.title.clone(),
            since: rh
                .start_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            until: rh.end_date.map(|d| d.format("%Y-%m-%d").to_string()),
            is_current: rh.end_date.is_none(),
        })
        .collect();

    if role_history.is_empty() && !organization_name.is_empty() {
        role_history.push(PersonRoleHistory {
            company_name: organization_name.clone(),
            role: person.current_role.clone().unwrap_or_else(|| {
                person
                    .role_family
                    .clone()
                    .unwrap_or_else(|| "Tracked contact".to_string())
            }),
            start_date: person
                .created_at
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            end_date: None,
        });
    }

    if affiliations.is_empty() && !organization_name.is_empty() {
        affiliations.push(PersonAffiliation {
            company_id: person
                .primary_org_id
                .map(|u| u.to_string())
                .unwrap_or_default(),
            company_name: organization_name.clone(),
            role: person.current_role.clone().unwrap_or_else(|| {
                person
                    .role_family
                    .clone()
                    .unwrap_or_else(|| "Tracked contact".to_string())
            }),
            since: person
                .created_at
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            until: None,
            is_current: true,
        });
    }

    // Fetch peers
    let rf = person.role_family.as_deref().unwrap_or("Unknown");
    let region = person.region.as_deref().unwrap_or("");
    let peer_rows = store
        .get_person_peers(uuid, rf, region, 10)
        .await
        .unwrap_or_default();
    let peers: Vec<PersonPeer> = peer_rows
        .iter()
        .map(|p| PersonPeer {
            id: p.id.to_string(),
            name: p.name.clone(),
            role: p.role.clone(),
            shared_company: p.organization.clone(),
        })
        .collect();

    let warning_count_person = store
        .get_warnings_by_entity_ids(&[uuid], 200)
        .await
        .map(|rows| rows.len() as i64)
        .unwrap_or(0);
    let insight_count = store
        .get_insights_by_entity_ids(&[uuid], 200)
        .await
        .map(|rows| rows.len() as i64)
        .unwrap_or(0);

    let artifact_count = store
        .get_artifacts_for_person(uuid, 200)
        .await
        .map(|rows| rows.len())
        .unwrap_or(0);

    let person_changes = store
        .get_person_changes(uuid, 10)
        .await
        .unwrap_or_default();
    let recent_change_count = person_changes.len();

    let pv = derive_profile_priority_vector(
        &person,
        artifact_count,
        role_history.len(),
        affiliations.len(),
        peers.len(),
        warning_count_person,
        insight_count,
        recent_change_count,
    );

    // Determine priority tier (values are 0-100 now)
    let tier = match pv.overall {
        x if x >= 80.0 => "critical",
        x if x >= 60.0 => "high",
        x if x >= 40.0 => "medium",
        _ => "low",
    };

    let mut recent_events = person_changes
        .into_iter()
        .map(|change| PersonEvent {
            kind: change.change_type,
            description: change
                .description
                .or(change.new_value)
                .or(change.old_value)
                .unwrap_or_else(|| "Person profile updated".to_string()),
            date: change
                .detected_at
                .or(change.created_at)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "—".to_string()),
            source: change.source_url.unwrap_or_else(|| "system".to_string()),
        })
        .collect::<Vec<_>>();

    if recent_events.is_empty() {
        let description = if !organization_name.is_empty() {
            format!(
                "Tracking {} at {} while profile enrichment continues.",
                person
                    .current_role
                    .clone()
                    .unwrap_or_else(|| "current role".to_string()),
                organization_name
            )
        } else {
            "Person profile is being tracked and awaits additional source-backed updates."
                .to_string()
        };
        recent_events.push(PersonEvent {
            kind: "profile_tracking".to_string(),
            description,
            date: person
                .updated_at
                .or(person.created_at)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "—".to_string()),
            source: "system".to_string(),
        });
    }

    let bio = person.public_bio.clone().filter(|value| !value.trim().is_empty()).unwrap_or_else(|| {
        let role = person
            .current_role
            .clone()
            .or_else(|| person.role_family.clone())
            .unwrap_or_else(|| "tracked contact".to_string());
        let org_fragment = if organization_name.is_empty() {
            "".to_string()
        } else {
            format!(" at {}", organization_name)
        };
        let region_fragment = person
            .region
            .clone()
            .filter(|value| !value.trim().is_empty())
            .map(|region| format!(" in {}", region))
            .unwrap_or_default();
        format!(
            "{} is currently tracked as {}{}{}. Source-backed enrichment is still in progress, so this profile may not yet include full role history, relationships, or activity context.",
            person.name, role, org_fragment, region_fragment
        )
    });

    let tpl = PersonDetailPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        briefing_mode: query.briefing.unwrap_or(false),
        id: person.id.to_string(),
        name: person.name.clone(),
        title: person.current_role.clone().unwrap_or_default(),
        bio,
        region: person.region.clone().unwrap_or_default(),
        priority_tier: tier.to_string(),
        priority_vector: pv,
        affiliations,
        role_history,
        peers,
        recent_events,
        warning_count_person,
        insight_count,
        created_at: person
            .created_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
        updated_at: person
            .updated_at
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default(),
    };

    let _ = is_htmx_request(&headers);
    super::render_template(&tpl)
}
