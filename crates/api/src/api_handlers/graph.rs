#![allow(clippy::disallowed_methods)]

use std::collections::{HashMap, HashSet, VecDeque};

use axum::extract::Query;

use crate::*;
use crate::routes::graph::{
    parse_edge_types, GraphEdge as RouteGraphEdge, GraphNode as RouteGraphNode,
    NeighborhoodQuery, NeighborhoodResponse, PathQuery, PathResponse, PathStep,
};

#[derive(sqlx::FromRow)]
struct GraphLabelRow {
    id: Uuid,
    label: String,
    node_type: String,
}

fn parse_graph_uuid(id: &str, field_name: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id).map_err(|_| ApiError::bad_request(format!("Invalid {} UUID", field_name)))
}

pub(crate) async fn get_graph_neighborhood(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<NeighborhoodQuery>,
) -> (StatusCode, Json<ApiResponse<NeighborhoodResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let uid = match parse_graph_uuid(&id, "entity") {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let query = query.sanitize();
    let max_nodes = query.max_nodes.unwrap_or(50) as usize;
    let max_depth = query.depth.unwrap_or(1);
    let min_weight = query.min_weight;
    let allowed_edge_types: HashSet<String> = query
        .edge_types
        .as_deref()
        .map(parse_edge_types)
        .unwrap_or_default()
        .into_iter()
        .collect();

    let mut seen_nodes: HashSet<Uuid> = HashSet::from([uid]);
    let mut node_depths: HashMap<Uuid, u32> = HashMap::from([(uid, 0)]);
    let mut frontier = vec![uid];
    let mut seen_edges: HashMap<Uuid, EdgeRow> = HashMap::new();
    let mut depth_reached = 0;
    let per_node_limit = ((max_nodes as u32).saturating_mul(6)).clamp(20, 240);

    for depth in 1..=max_depth {
        if frontier.is_empty() || seen_nodes.len() >= max_nodes {
            break;
        }

        let mut next_frontier = Vec::new();
        let mut next_seen = HashSet::new();
        for current in frontier {
            let edges = match state.store.get_neighborhood(current, per_node_limit).await {
                Ok(edges) => edges,
                Err(err) => {
                    tracing::error!(request_id = %request_id, node = %current, "graph neighborhood failed: {err:#}");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(error_response(ApiError::internal(
                            "Failed to load graph neighborhood",
                        ))),
                    );
                }
            };

            for edge in edges.into_iter().filter(|edge| edge_matches_filters(edge, &allowed_edge_types, min_weight)) {
                seen_edges.entry(edge.id).or_insert_with(|| edge.clone());
                for node_id in [edge.source_id, edge.target_id] {
                    if seen_nodes.len() >= max_nodes || seen_nodes.contains(&node_id) {
                        continue;
                    }
                    seen_nodes.insert(node_id);
                    node_depths.insert(node_id, depth);
                    if next_seen.insert(node_id) {
                        next_frontier.push(node_id);
                    }
                }
            }
        }

        if !next_frontier.is_empty() {
            depth_reached = depth;
        }
        frontier = next_frontier;
    }

    let labels = match resolve_graph_labels(&state.store, &seen_nodes).await {
        Ok(labels) => labels,
        Err(err) => {
            tracing::error!(request_id = %request_id, "graph neighborhood label resolution failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to resolve graph neighborhood labels",
                ))),
            );
        }
    };

    let mut nodes: Vec<RouteGraphNode> = seen_nodes
        .iter()
        .map(|node_id| {
            let (label, node_type) = labels
                .get(node_id)
                .cloned()
                .unwrap_or_else(|| (short_graph_id(node_id), "unknown".to_string()));
            RouteGraphNode {
                id: node_id.to_string(),
                label,
                node_type,
                properties: serde_json::json!({
                    "is_center": *node_id == uid,
                }),
                depth: *node_depths.get(node_id).unwrap_or(&0),
            }
        })
        .collect();
    nodes.sort_by(|left, right| left.depth.cmp(&right.depth).then_with(|| left.label.cmp(&right.label)));

    let mut edges: Vec<RouteGraphEdge> = seen_edges
        .into_values()
        .filter(|edge| seen_nodes.contains(&edge.source_id) && seen_nodes.contains(&edge.target_id))
        .map(edge_row_to_route_edge)
        .collect();
    edges.sort_by(|left, right| right.weight.partial_cmp(&left.weight).unwrap_or(std::cmp::Ordering::Equal));

    let response = NeighborhoodResponse {
        center_id: uid.to_string(),
        total_neighbors: seen_nodes.len().saturating_sub(1) as u32,
        depth_reached,
        nodes,
        edges,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_graph_neighborhood", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            response,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn get_graph_path(
    State(state): State<AppState>,
    Path((from, to)): Path<(String, String)>,
    Query(query): Query<PathQuery>,
) -> (StatusCode, Json<ApiResponse<PathResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let from_id = match parse_graph_uuid(&from, "from") {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let to_id = match parse_graph_uuid(&to, "to") {
        Ok(uid) => uid,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let query = query.sanitize();
    let max_hops = query.max_hops.unwrap_or(5);
    let allowed_edge_types: HashSet<String> = query
        .edge_types
        .as_deref()
        .map(parse_edge_types)
        .unwrap_or_default()
        .into_iter()
        .collect();

    if from_id == to_id {
        let labels = match resolve_graph_labels(&state.store, &HashSet::from([from_id])).await {
            Ok(labels) => labels,
            Err(err) => {
                tracing::error!(request_id = %request_id, "graph path label resolution failed: {err:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Failed to resolve graph path labels",
                    ))),
                );
            }
        };
        let label = labels
            .get(&from_id)
            .map(|(label, _)| label.clone())
            .unwrap_or_else(|| short_graph_id(&from_id));
        let response = PathResponse {
            from_id: from_id.to_string(),
            to_id: to_id.to_string(),
            total_hops: 0,
            total_weight: 0.0,
            found: true,
            path: vec![PathStep {
                node_id: from_id.to_string(),
                node_label: label,
                edge_type: None,
                edge_weight: None,
            }],
        };
        let duration_ms = start.elapsed().as_millis() as u64;
        log_latency("get_graph_path", duration_ms);
        return (
            StatusCode::OK,
            Json(success_with_meta(
                response,
                ResponseMeta::now()
                    .with_request_id(request_id)
                    .with_duration(duration_ms),
            )),
        );
    }

    let mut visited: HashSet<Uuid> = HashSet::from([from_id]);
    let mut queue: VecDeque<(Uuid, u32)> = VecDeque::from([(from_id, 0)]);
    let mut parents: HashMap<Uuid, (Uuid, String, f64)> = HashMap::new();
    let mut found = false;
    let per_node_limit = 160_u32;

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_hops {
            continue;
        }

        let edges = match state.store.get_neighborhood(current, per_node_limit).await {
            Ok(edges) => edges,
            Err(err) => {
                tracing::error!(request_id = %request_id, node = %current, "graph path failed: {err:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal(
                        "Failed to load graph path",
                    ))),
                );
            }
        };

        for edge in edges.into_iter().filter(|edge| edge_matches_filters(edge, &allowed_edge_types, None)) {
            let next_id = if edge.source_id == current {
                edge.target_id
            } else if edge.target_id == current {
                edge.source_id
            } else {
                continue;
            };

            if !visited.insert(next_id) {
                continue;
            }
            parents.insert(
                next_id,
                (current, edge.edge_type.clone(), edge.weight.unwrap_or(1.0)),
            );
            if next_id == to_id {
                found = true;
                break;
            }
            queue.push_back((next_id, depth + 1));
        }

        if found {
            break;
        }
    }

    let mut node_sequence = Vec::new();
    let mut total_weight = 0.0;
    if found {
        let mut cursor = to_id;
        node_sequence.push(cursor);
        while let Some((prev, _, weight)) = parents.get(&cursor) {
            total_weight += *weight;
            cursor = *prev;
            node_sequence.push(cursor);
            if cursor == from_id {
                break;
            }
        }
        node_sequence.reverse();
    }

    let label_ids: HashSet<Uuid> = if found {
        node_sequence.iter().copied().collect()
    } else {
        HashSet::from([from_id, to_id])
    };
    let labels = match resolve_graph_labels(&state.store, &label_ids).await {
        Ok(labels) => labels,
        Err(err) => {
            tracing::error!(request_id = %request_id, "graph path label resolution failed: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to resolve graph path labels",
                ))),
            );
        }
    };

    let path = if found {
        node_sequence
            .iter()
            .enumerate()
            .map(|(index, node_id)| {
                let node_label = labels
                    .get(node_id)
                    .map(|(label, _)| label.clone())
                    .unwrap_or_else(|| short_graph_id(node_id));
                let edge_meta = if index == 0 {
                    None
                } else {
                    parents.get(node_id).map(|(_, edge_type, weight)| (edge_type.clone(), *weight))
                };
                PathStep {
                    node_id: node_id.to_string(),
                    node_label,
                    edge_type: edge_meta.as_ref().map(|(edge_type, _)| edge_type.clone()),
                    edge_weight: edge_meta.as_ref().map(|(_, weight)| *weight),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let response = PathResponse {
        from_id: from_id.to_string(),
        to_id: to_id.to_string(),
        total_hops: node_sequence.len().saturating_sub(1) as u32,
        total_weight,
        found,
        path,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_graph_path", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            response,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

fn edge_matches_filters(
    edge: &EdgeRow,
    allowed_edge_types: &HashSet<String>,
    min_weight: Option<f64>,
) -> bool {
    if let Some(min_weight) = min_weight {
        if edge.weight.unwrap_or(1.0) < min_weight {
            return false;
        }
    }

    allowed_edge_types.is_empty()
        || allowed_edge_types.contains(&edge.edge_type.to_ascii_lowercase())
}

fn edge_row_to_route_edge(edge: EdgeRow) -> RouteGraphEdge {
    RouteGraphEdge {
        source: edge.source_id.to_string(),
        target: edge.target_id.to_string(),
        edge_type: edge.edge_type,
        weight: edge.weight.unwrap_or(1.0),
        label: None,
    }
}

async fn resolve_graph_labels(
    store: &PgStore,
    node_ids: &HashSet<Uuid>,
) -> anyhow::Result<HashMap<Uuid, (String, String)>> {
    if node_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let ids: Vec<Uuid> = node_ids.iter().copied().collect();
    let rows = sqlx::query_as::<_, GraphLabelRow>(
        r#"SELECT id, name AS label, 'company' AS node_type FROM companies WHERE id = ANY($1)
           UNION ALL
           SELECT id, name AS label, 'person' AS node_type FROM persons WHERE id = ANY($1)"#,
    )
    .bind(&ids)
    .fetch_all(&store.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.id, (row.label, row.node_type)))
        .collect())
}

fn short_graph_id(id: &Uuid) -> String {
    let text = id.to_string();
    format!("{}…", &text[..8])
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{edge_matches_filters, parse_graph_uuid};
    use apex_store::postgres::EdgeRow;
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn test_parse_graph_uuid_accepts_valid_uuid() {
        let id = Uuid::new_v4().to_string();

        let parsed = parse_graph_uuid(&id, "entity").expect("valid uuid should parse");

        assert_eq!(parsed.to_string(), id);
    }

    #[test]
    fn test_parse_graph_uuid_uses_field_name_in_error() {
        let err = parse_graph_uuid("bad-id", "from").expect_err("invalid uuid should fail");

        assert_eq!(err.http_status(), 400);
        assert_eq!(err.message, "Invalid from UUID");
    }

    #[test]
    fn test_edge_matches_filters_respects_edge_type_and_weight() {
        let edge = EdgeRow {
            id: Uuid::new_v4(),
            source_id: Uuid::new_v4(),
            source_type: "company".to_string(),
            target_id: Uuid::new_v4(),
            target_type: "person".to_string(),
            edge_type: "supplier_of".to_string(),
            weight: Some(0.82),
            confidence: Some(0.9),
            evidence_ids: None,
            metadata: None,
            first_seen: Some(Utc::now()),
            last_seen: Some(Utc::now()),
        };

        assert!(edge_matches_filters(&edge, &HashSet::new(), None));
        assert!(edge_matches_filters(
            &edge,
            &HashSet::from(["supplier_of".to_string()]),
            Some(0.6),
        ));
        assert!(!edge_matches_filters(
            &edge,
            &HashSet::from(["competes_with".to_string()]),
            None,
        ));
        assert!(!edge_matches_filters(&edge, &HashSet::new(), Some(0.9)));
    }
}
