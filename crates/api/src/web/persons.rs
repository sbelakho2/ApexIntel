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
                "B" => score >= 50 && score < 80,
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
            if row.artifact_count > 0 {
                tags.push(format!("{} artifacts", row.artifact_count));
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

    if is_htmx_request(&headers) {
        tpl.into_response()
    } else {
        tpl.into_response()
    }
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

    // Parse priority vector from JSON (convert 0-1 range to 0-100 for display)
    let pv = person
        .priority_vector
        .as_ref()
        .and_then(|v| {
            Some(PriorityVector {
                influence: normalize_percent(v.get("influence")?.as_f64()?),
                connectivity: normalize_percent(v.get("connectivity")?.as_f64()?),
                activity: normalize_percent(v.get("activity")?.as_f64()?),
                risk: normalize_percent(v.get("risk")?.as_f64()?),
                overall: normalize_percent(v.get("overall")?.as_f64()?),
            })
        })
        .unwrap_or(PriorityVector {
            influence: normalize_percent(person.influence_score.unwrap_or(0.0)),
            connectivity: 0.0,
            activity: 0.0,
            risk: 0.0,
            overall: normalize_percent(person.influence_score.unwrap_or(0.0)),
        });

    // Determine priority tier (values are 0-100 now)
    let tier = match pv.overall {
        x if x >= 80.0 => "critical",
        x if x >= 60.0 => "high",
        x if x >= 40.0 => "medium",
        _ => "low",
    };

    // Fetch role history
    let rh_rows = store
        .get_role_history_for_person(uuid)
        .await
        .unwrap_or_default();
    let role_history: Vec<PersonRoleHistory> = rh_rows
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
    let affiliations: Vec<PersonAffiliation> = rh_rows
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

    let recent_events = store
        .get_person_changes(uuid, 10)
        .await
        .unwrap_or_default()
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

    let tpl = PersonDetailPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        briefing_mode: query.briefing.unwrap_or(false),
        id: person.id.to_string(),
        name: person.name.clone(),
        title: person.current_role.clone().unwrap_or_default(),
        bio: person.public_bio.clone().unwrap_or_default(),
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

    if is_htmx_request(&headers) {
        tpl.into_response()
    } else {
        tpl.into_response()
    }
}
