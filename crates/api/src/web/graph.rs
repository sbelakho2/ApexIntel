//! Graph handler — GET /graph
//!
//! Covers: interactive entity relationship graph with nodes (companies,
//! persons) and edges (affiliations, supply-chain links, etc.).

use std::sync::Arc;
use std::collections::{HashMap, HashSet};

use askama::Template;
use axum::{
    http::HeaderMap,
    response::IntoResponse,
    Extension,
};

use apex_store::postgres::{
    CompanyListFilters,
    InsightListFilters,
    PersonListFilters,
    PgStore,
    WarningListFilters,
};
use super::{is_htmx_request, PageContext};
use crate::middleware::session::WebSession;
use uuid::Uuid;

// ─── Template data ──────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub node_type: String,  // "company" | "person" | "site"
    pub risk_score: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct GraphRenderNode {
    pub id: String,
    pub label: String,
    pub node_type: String,
    pub color: String,
    pub size: i64,
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Debug)]
pub struct GraphRenderEdge {
    pub source: String,
    pub target: String,
    pub x1: i64,
    pub y1: i64,
    pub x2: i64,
    pub y2: i64,
    pub edge_type: String,
}

#[derive(Clone, Debug)]
pub struct EdgeTypeCount {
    pub edge_type: String,
    pub count: i64,
}

#[derive(Clone, Debug)]
pub struct GraphEdgeRow {
    pub source_short: String,
    pub target_short: String,
    pub edge_type: String,
    pub weight: String,
}

// ─── Template ───────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "pages/graph.html")]
pub struct GraphPage {
    pub current_path: String,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,

    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub companies_total: i64,
    pub persons_total: i64,
    pub regions_total: i64,
    pub countries_total: i64,
    pub certs_total: i64,
    pub tenders_total: i64,
    pub edge_type_counts: Vec<EdgeTypeCount>,
    pub edge_rows: Vec<GraphEdgeRow>,
    pub render_nodes: Vec<GraphRenderNode>,
    pub render_edges: Vec<GraphRenderEdge>,
    /// JSON-serialized graph data for the client-side renderer.
    pub graph_json: String,
}

// ─── Handler ────────────────────────────────────────────────────────────────

/// GET /graph — entity relationship graph page.
pub async fn graph_page(
    headers: HeaderMap,
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    let unack = store.count_warnings(&WarningListFilters { acknowledged: Some(false), ..Default::default() }).await.unwrap_or(0);
    let ctx = PageContext::from_session(&session, "/graph", unack);

    let edge_rows = store.list_all_edges(500).await.unwrap_or_else(|e| {
        tracing::error!("Failed to list graph edges: {e}");
        vec![]
    });
    let _edges_total = store.count_edges().await.unwrap_or(edge_rows.len() as i64);

    let companies_total = store.count_companies(&CompanyListFilters::default()).await.unwrap_or(0);
    let persons_total = store.count_persons(&PersonListFilters::default()).await.unwrap_or(0);
    let _warnings_total = store.count_warnings(&WarningListFilters::default()).await.unwrap_or(0);
    let _insights_total = store.count_insights(&InsightListFilters::default()).await.unwrap_or(0);

    let node_ids: Vec<Uuid> = {
        let mut seen = HashSet::new();
        for row in &edge_rows {
            seen.insert(row.source_id);
            seen.insert(row.target_id);
        }
        seen.into_iter().collect()
    };

    let node_labels_by_id: HashMap<String, String> = if node_ids.is_empty() {
        HashMap::new()
    } else {
        #[derive(sqlx::FromRow)]
        struct NodeNameRow {
            id: Uuid,
            label: String,
        }

        sqlx::query_as::<_, NodeNameRow>(
            r#"SELECT id, name AS label FROM companies WHERE id = ANY($1)
               UNION ALL
               SELECT id, name AS label FROM persons WHERE id = ANY($1)"#,
        )
        .bind(&node_ids)
        .fetch_all(&store.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| (row.id.to_string(), row.label))
        .collect()
    };

    #[derive(sqlx::FromRow)]
    struct GraphCompanyRow {
        id: Uuid,
        name: String,
        region: Option<String>,
        country_code: Option<String>,
        domain: Option<String>,
    }

    let mut company_ids_from_edges: HashSet<Uuid> = HashSet::new();
    for row in &edge_rows {
        if normalize_node_type(&row.source_type) == "company" {
            company_ids_from_edges.insert(row.source_id);
        }
        if normalize_node_type(&row.target_type) == "company" {
            company_ids_from_edges.insert(row.target_id);
        }
    }

    let mut company_rows: Vec<GraphCompanyRow> = if company_ids_from_edges.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, GraphCompanyRow>(
            r#"SELECT id, name, region, country_code, domain
               FROM companies
               WHERE id = ANY($1)"#,
        )
        .bind(company_ids_from_edges.into_iter().collect::<Vec<_>>())
        .fetch_all(&store.pool)
        .await
        .unwrap_or_default()
    };

    if company_rows.is_empty() {
        company_rows = store
            .list_companies(&CompanyListFilters::default(), None, true, 1200, 0)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|company| GraphCompanyRow {
                id: company.id,
                name: company.name,
                region: company.region,
                country_code: company.country_code,
                domain: company.domain,
            })
            .collect();
    }
    let cert_rows = store
        .list_certifications(None, 400, 0)
        .await
        .unwrap_or_default();
    let warning_rows = store
        .list_warnings(&WarningListFilters::default(), None, true, 400, 0)
        .await
        .unwrap_or_default();

    #[derive(sqlx::FromRow)]
    struct SiteCountryRow {
        company_id: Uuid,
        country_code: Option<String>,
    }

    let company_ids: Vec<Uuid> = company_rows.iter().map(|company| company.id).collect();
    let site_country_rows: Vec<SiteCountryRow> = if company_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as::<_, SiteCountryRow>(
            r#"SELECT company_id, country_code
               FROM sites
               WHERE company_id = ANY($1)
                 AND country_code IS NOT NULL"#,
        )
        .bind(&company_ids)
        .fetch_all(&store.pool)
        .await
        .unwrap_or_default()
    };

    // Build nodes from edges
    let mut node_map: HashMap<String, GraphNode> = HashMap::new();
    for er in &edge_rows {
        let src_id = er.source_id.to_string();
        let src_label = node_labels_by_id
            .get(&src_id)
            .cloned()
            .unwrap_or_else(|| er.source_type.clone());
        node_map.entry(src_id.clone()).or_insert_with(|| GraphNode {
            id: src_id,
            label: src_label,
            node_type: er.source_type.clone(),
            risk_score: None,
        });
        let tgt_id = er.target_id.to_string();
        let tgt_label = node_labels_by_id
            .get(&tgt_id)
            .cloned()
            .unwrap_or_else(|| er.target_type.clone());
        node_map.entry(tgt_id.clone()).or_insert_with(|| GraphNode {
            id: tgt_id,
            label: tgt_label,
            node_type: er.target_type.clone(),
            risk_score: None,
        });
    }

    let mut synthetic_edges: Vec<GraphEdge> = Vec::new();
    let mut synthetic_edge_seen: HashSet<(String, String)> = HashSet::new();
    for company in &company_rows {
        let company_id = company.id.to_string();
        node_map.entry(company_id.clone()).or_insert_with(|| GraphNode {
            id: company_id.clone(),
            label: company.name.clone(),
            node_type: "company".to_string(),
            risk_score: None,
        });

        if let Some(region_name) = company
            .region
            .clone()
            .filter(|value| !value.trim().is_empty())
        {
            let normalized_region = region_name.trim().to_string();
            let region_id = format!(
                "region:{}",
                normalized_region
                    .to_ascii_lowercase()
                    .replace(' ', "_")
            );

            node_map.entry(region_id.clone()).or_insert_with(|| GraphNode {
                id: region_id.clone(),
                label: normalized_region,
                node_type: "region".to_string(),
                risk_score: None,
            });

            if synthetic_edge_seen.insert((company_id.clone(), region_id.clone())) {
                synthetic_edges.push(GraphEdge {
                    source: company_id.clone(),
                    target: region_id,
                    edge_type: "operates_in".to_string(),
                    weight: 1.0,
                });
            }
        }

        if let Some(domain_name) = company
            .domain
            .clone()
            .filter(|value| !value.trim().is_empty())
        {
            let normalized_domain = domain_name.trim().to_string();
            let domain_id = format!(
                "domain:{}",
                normalized_domain
                    .to_ascii_lowercase()
                    .replace(' ', "_")
            );

            node_map.entry(domain_id.clone()).or_insert_with(|| GraphNode {
                id: domain_id.clone(),
                label: normalized_domain,
                node_type: "domain".to_string(),
                risk_score: None,
            });

            if synthetic_edge_seen.insert((company_id.clone(), domain_id.clone())) {
                synthetic_edges.push(GraphEdge {
                    source: company_id.clone(),
                    target: domain_id,
                    edge_type: "has_domain".to_string(),
                    weight: 1.0,
                });
            }
        }

        let country_label = company
            .country_code
            .clone()
            .filter(|value| !value.trim().is_empty());

        if let Some(country_name) = country_label {
            let normalized_country = country_name.trim().to_string();
            let country_id = format!(
                "country:{}",
                normalized_country
                    .to_ascii_lowercase()
                    .replace(' ', "_")
            );

            node_map.entry(country_id.clone()).or_insert_with(|| GraphNode {
                id: country_id.clone(),
                label: normalized_country,
                node_type: "country".to_string(),
                risk_score: None,
            });

            if synthetic_edge_seen.insert((company_id.clone(), country_id.clone())) {
                synthetic_edges.push(GraphEdge {
                    source: company_id.clone(),
                    target: country_id,
                    edge_type: "located_in".to_string(),
                    weight: 1.0,
                });
            }
        }
    }

    for site in site_country_rows {
        let Some(country_code) = site
            .country_code
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let company_id = site.company_id.to_string();
        if !node_map.contains_key(&company_id) {
            continue;
        }

        let normalized_country = country_code.to_string();
        let country_id = format!(
            "country:{}",
            normalized_country
                .to_ascii_lowercase()
                .replace(' ', "_")
        );

        node_map.entry(country_id.clone()).or_insert_with(|| GraphNode {
            id: country_id.clone(),
            label: normalized_country,
            node_type: "country".to_string(),
            risk_score: None,
        });

        if synthetic_edge_seen.insert((company_id.clone(), country_id.clone())) {
            synthetic_edges.push(GraphEdge {
                source: company_id,
                target: country_id,
                edge_type: "located_in".to_string(),
                weight: 1.0,
            });
        }
    }

    for cert in cert_rows {
        let Some(company_id) = cert.company_id.map(|id| id.to_string()) else {
            continue;
        };
        if !node_map.contains_key(&company_id) {
            continue;
        }

        let cert_label = cert.standard.trim().to_string();
        if cert_label.is_empty() {
            continue;
        }

        let cert_id = format!(
            "cert:{}",
            cert_label
                .to_ascii_lowercase()
                .replace(' ', "_")
        );

        node_map.entry(cert_id.clone()).or_insert_with(|| GraphNode {
            id: cert_id.clone(),
            label: cert_label,
            node_type: "cert".to_string(),
            risk_score: None,
        });

        if synthetic_edge_seen.insert((company_id.clone(), cert_id.clone())) {
            synthetic_edges.push(GraphEdge {
                source: company_id,
                target: cert_id,
                edge_type: "certified_for".to_string(),
                weight: 1.0,
            });
        }
    }

    for warning in warning_rows {
        let warning_type_low = warning.warning_type.to_ascii_lowercase();
        let recipe_low = warning
            .recipe_code
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_tender_like = warning_type_low.contains("tender")
            || warning_type_low.contains("procurement")
            || warning_type_low.contains("contract")
            || warning_type_low.contains("rfp")
            || recipe_low.contains("tender")
            || recipe_low.contains("procurement")
            || recipe_low.contains("contract")
            || recipe_low.contains("rfp");
        if !is_tender_like {
            continue;
        }

        let tender_label = if warning.title.trim().is_empty() {
            "Tender Signal".to_string()
        } else {
            warning.title.trim().to_string()
        };
        let tender_id = format!("tender:{}", warning.id);

        node_map.entry(tender_id.clone()).or_insert_with(|| GraphNode {
            id: tender_id.clone(),
            label: tender_label,
            node_type: "tender".to_string(),
            risk_score: None,
        });

        let anchor_id = warning
            .entity_ids
            .as_ref()
            .and_then(|ids| {
                ids.iter()
                    .map(|id| id.to_string())
                    .find(|id| node_map.contains_key(id))
            })
            .or_else(|| {
                warning.region.as_ref().and_then(|region| {
                    let region_id = format!(
                        "region:{}",
                        region
                            .trim()
                            .to_ascii_lowercase()
                            .replace(' ', "_")
                    );
                    if node_map.contains_key(&region_id) {
                        Some(region_id)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| company_rows.first().map(|company| company.id.to_string()));

        if let Some(anchor_id) = anchor_id {
            if synthetic_edge_seen.insert((anchor_id.clone(), tender_id.clone())) {
                synthetic_edges.push(GraphEdge {
                    source: anchor_id,
                    target: tender_id,
                    edge_type: "tender_signal".to_string(),
                    weight: 1.0,
                });
            }
        }
    }

    let nodes: Vec<GraphNode> = node_map.into_values().collect();
    let mut edges: Vec<GraphEdge> = edge_rows.iter().map(|er| GraphEdge {
        source: er.source_id.to_string(),
        target: er.target_id.to_string(),
        edge_type: er.edge_type.clone(),
        weight: er.weight.unwrap_or(1.0),
    }).collect();
    edges.extend(synthetic_edges);

    // Count edge types
    let mut type_counts: HashMap<String, i64> = HashMap::new();
    for e in &edges {
        *type_counts.entry(e.edge_type.clone()).or_insert(0) += 1;
    }
    let edge_type_counts: Vec<EdgeTypeCount> = type_counts.into_iter()
        .map(|(edge_type, count)| EdgeTypeCount { edge_type, count })
        .collect();

    let mut regions_total = 0_i64;
    let mut countries_total = 0_i64;
    let mut certs_total = 0_i64;
    let mut tenders_total = 0_i64;
    let mut companies_total_graph = 0_i64;
    let mut persons_total_graph = 0_i64;
    for n in &nodes {
        let t = normalize_node_type(&n.node_type);
        if t == "company" {
            companies_total_graph += 1;
        }
        if t == "person" {
            persons_total_graph += 1;
        }
        if t == "region" {
            regions_total += 1;
        }
        if t == "country" {
            countries_total += 1;
        }
        if t == "cert" {
            certs_total += 1;
        }
        if t == "tender" {
            tenders_total += 1;
        }
    }

    if regions_total == 0 {
        let mut region_set = std::collections::BTreeSet::new();
        for c in &company_rows {
            if let Some(region) = &c.region {
                if !region.trim().is_empty() {
                    region_set.insert(region.clone());
                }
            }
        }
        regions_total = region_set.len() as i64;
    }

    let selected_node_ids = select_nodes_balanced(&nodes, &edges, 140, 28);
    let mut node_slice = nodes
        .iter()
        .filter(|node| selected_node_ids.contains(&node.id))
        .cloned()
        .collect::<Vec<_>>();
    node_slice.sort_by(|left, right| left.id.cmp(&right.id));

    let mut degree_map: HashMap<String, i64> = HashMap::new();
    for edge in &edges {
        *degree_map.entry(edge.source.clone()).or_insert(0) += 1;
        *degree_map.entry(edge.target.clone()).or_insert(0) += 1;
    }

    let mut by_type_count: HashMap<String, usize> = HashMap::new();
    for node in &node_slice {
        let t = normalize_node_type(&node.node_type);
        *by_type_count.entry(t).or_insert(0) += 1;
    }
    let mut by_type_index: HashMap<String, usize> = HashMap::new();

    let mut pos_map: HashMap<String, (i64, i64)> = HashMap::new();
    let mut render_nodes = Vec::new();
    for node in &node_slice {
        let node_type = normalize_node_type(&node.node_type);
        let idx = *by_type_index.entry(node_type.clone()).or_insert(0);
        *by_type_index.entry(node_type.clone()).or_insert(0) += 1;
        let total = *by_type_count.get(&node_type).unwrap_or(&1);

        let (cx, cy) = match node_type.as_str() {
            "company" => (400.0_f64, 290.0_f64),
            "person" => (620.0_f64, 290.0_f64),
            "region" => (260.0_f64, 130.0_f64),
            "country" => (520.0_f64, 130.0_f64),
            "cert" => (220.0_f64, 260.0_f64),
            "tender" => (520.0_f64, 450.0_f64),
            _ => (130.0_f64, 360.0_f64),
        };

        let angle = if total <= 1 {
            0.0
        } else {
            (idx as f64 / total as f64) * std::f64::consts::PI * 2.0
        };
        let ring = 26.0 + ((idx / 6) as f64 * 18.0);
        let x = (cx + angle.cos() * ring).round() as i64;
        let y = (cy + angle.sin() * ring).round() as i64;
        pos_map.insert(node.id.clone(), (x, y));

        let degree = *degree_map.get(&node.id).unwrap_or(&1);
        let size = (12_i64 + (degree * 2)).clamp(12, 28);
        let color = match node_type.as_str() {
            "company" => "#4A90E2",
            "person" => "#2D8C3C",
            "region" => "#FFBE00",
            "country" => "#FFBE00",
            "cert" => "#4A90E2",
            "tender" => "#D62D2D",
            _ => "#D62D2D",
        };

        let label = if is_generic_graph_label(&node.label) {
            short_id(&node.id)
        } else {
            short_label(&node.label, 14)
        };

        render_nodes.push(GraphRenderNode {
            id: node.id.clone(),
            label,
            node_type,
            color: color.to_string(),
            size,
            x,
            y,
        });
    }

    let mut render_edges = Vec::new();
    for edge in edges
        .iter()
        .filter(|edge| {
            selected_node_ids.contains(&edge.source) && selected_node_ids.contains(&edge.target)
        })
        .take(320)
    {
        if let (Some((x1, y1)), Some((x2, y2))) = (pos_map.get(&edge.source), pos_map.get(&edge.target)) {
            render_edges.push(GraphRenderEdge {
                source: edge.source.clone(),
                target: edge.target.clone(),
                x1: *x1,
                y1: *y1,
                x2: *x2,
                y2: *y2,
                edge_type: edge.edge_type.clone(),
            });
        }
    }

    let edge_rows: Vec<GraphEdgeRow> = edges
        .iter()
        .take(50)
        .map(|e| GraphEdgeRow {
            source_short: short_id(&e.source),
            target_short: short_id(&e.target),
            edge_type: e.edge_type.clone(),
            weight: format!("{:.2}", e.weight),
        })
        .collect();

    // Serialize for client
    let graph_json = serde_json::json!({
        "nodes": render_nodes.iter().map(|n| serde_json::json!({
            "id": n.id,
            "label": n.label,
            "type": n.node_type,
            "node_type": n.node_type,
            "size": n.size,
            "x": n.x,
            "y": n.y,
            "color": n.color,
        })).collect::<Vec<_>>(),
        "edges": render_edges.iter().map(|e| serde_json::json!({
            "source": e.source,
            "target": e.target,
            "type": e.edge_type,
            "edge_type": e.edge_type,
            "weight": 1.0,
        })).collect::<Vec<_>>(),
    }).to_string();

    let tpl = GraphPage {
        current_path: ctx.current_path,
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        companies_total: if companies_total_graph > 0 { companies_total_graph } else { companies_total },
        persons_total: if persons_total_graph > 0 { persons_total_graph } else { persons_total },
        regions_total,
        countries_total,
        certs_total,
        tenders_total,
        edge_type_counts,
        edge_rows,
        render_nodes,
        render_edges,
        graph_json,
        nodes,
        edges,
    };

    let _ = is_htmx_request(&headers);
    tpl.into_response()
}

fn short_id(value: &str) -> String {
    if value.chars().count() <= 8 {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(8).collect::<String>())
    }
}

fn normalize_node_type(value: &str) -> String {
    let low = value.to_lowercase();
    if low.contains("company") || low == "org" || low == "organization" {
        "company".to_string()
    } else if low.contains("person") || low.contains("poi") {
        "person".to_string()
    } else if low.contains("country") || low.contains("countries") || low.contains("nation") || low.contains("state") {
        "country".to_string()
    } else if low.contains("region") || low.contains("regions") || low.contains("geo") || low.contains("location") || low.contains("site") {
        "region".to_string()
    } else if low.contains("cert") || low.contains("certificate") || low.contains("compliance") {
        "cert".to_string()
    } else if low.contains("tender") {
        "tender".to_string()
    } else {
        "domain".to_string()
    }
}

fn short_label(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        format!("{}…", trimmed.chars().take(max_chars.saturating_sub(1)).collect::<String>())
    }
}

fn is_generic_graph_label(value: &str) -> bool {
    let low = value.trim().to_lowercase();
    low.is_empty()
        || low == "company"
        || low == "person"
        || low == "region"
        || low == "country"
        || low == "cert"
        || low == "certificate"
        || low == "tender"
        || low == "domain"
        || low == "organization"
        || low == "org"
}

fn select_nodes_balanced(nodes: &[GraphNode], edges: &[GraphEdge], max_total: usize, per_type: usize) -> HashSet<String> {
    let mut buckets: HashMap<String, Vec<&GraphNode>> = HashMap::new();
    for node in nodes {
        buckets
            .entry(normalize_node_type(&node.node_type))
            .or_default()
            .push(node);
    }

    for bucket in buckets.values_mut() {
        bucket.sort_by(|left, right| left.label.cmp(&right.label).then(left.id.cmp(&right.id)));
    }

    let mut selected: HashSet<String> = HashSet::new();
    let mut bucket_keys = buckets.keys().cloned().collect::<Vec<_>>();
    bucket_keys.sort();

    for key in &bucket_keys {
        if let Some(bucket) = buckets.get(key) {
            for node in bucket.iter().take(per_type) {
                if selected.len() >= max_total {
                    return selected;
                }
                selected.insert(node.id.clone());
            }
        }
    }

    if selected.is_empty() {
        for node in nodes.iter().take(max_total) {
            selected.insert(node.id.clone());
        }
    }

    for edge in edges {
        if selected.len() >= max_total {
            break;
        }
        if selected.contains(&edge.source) {
            selected.insert(edge.target.clone());
        } else if selected.contains(&edge.target) {
            selected.insert(edge.source.clone());
        }
    }

    selected
}
