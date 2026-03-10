use super::super::*;

fn build_enhanced_search_query(raw_query: &str) -> String {
    let sanitized = apex_api::routes::semantic_search::sanitize_query(raw_query);
    let terms = apex_api::routes::semantic_search::extract_terms(&sanitized);
    if terms.is_empty() {
        return sanitized;
    }

    let exact_phrase = format!("\"{}\"^4", sanitized);
    let title_terms = terms
        .iter()
        .map(|term| format!("title:{term}^3 tags:{term}^2 body:{term}"))
        .collect::<Vec<_>>()
        .join(" ");
    let fuzzy_terms = terms
        .iter()
        .filter(|term| term.len() >= 4)
        .map(|term| format!("title:{term}~1^1.5 body:{term}~1 tags:{term}"))
        .collect::<Vec<_>>()
        .join(" ");

    if fuzzy_terms.is_empty() {
        format!("({exact_phrase}) OR ({title_terms})")
    } else {
        format!("({exact_phrase}) OR ({title_terms}) OR ({fuzzy_terms})")
    }
}

fn result_matches_filters(
    result: &apex_store::tantivy_index::SearchResult,
    entity_types: &[String],
    regions: &[String],
    from_ts: Option<i64>,
    to_ts: Option<i64>,
) -> bool {
    let entity_match = entity_types.is_empty()
        || entity_types
            .iter()
            .any(|value| value == &result.entity_type);
    let region_match = regions.is_empty() || regions.iter().any(|value| value == &result.region);
    let from_match = from_ts.is_none_or(|from_ts| result.timestamp >= from_ts);
    let to_match = to_ts.is_none_or(|to_ts| result.timestamp <= to_ts);
    entity_match && region_match && from_match && to_match
}

fn append_or_filter(mut base_query: String, field: &str, values: &[String]) -> String {
    let clause = values
        .iter()
        .filter(|value| {
            value
                .chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-')
        })
        .map(|value| format!("{}:{}", field, value))
        .collect::<Vec<_>>()
        .join(" OR ");
    if !clause.is_empty() {
        base_query = format!("({}) AND ({})", base_query, clause);
    }
    base_query
}

pub(crate) async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> (StatusCode, Json<ApiResponse<SearchResponse>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };
    let limit = per_page.clamp(1, 50) as usize;
    let query = match validate_search_query(&params.q) {
        Ok(value) => value,
        Err(msg) => {
            let err = ApiError::bad_request(msg);
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let offset = ((page - 1) as usize).saturating_mul(limit);

    let mut full_query = build_enhanced_search_query(&query);
    if params.entity_types.is_some() {
        let types = match parse_csv_lower_strict(&params.entity_types, 32, "entity_types") {
            Ok(values) => values,
            Err(api_err) => {
                return (
                    StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                    Json(error_response(api_err)),
                );
            }
        };
        if !types.is_empty() {
            full_query = append_or_filter(full_query, "entity_type", &types);
        }
    }
    if params.regions.is_some() {
        let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
            Ok(values) => values,
            Err(api_err) => {
                return (
                    StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                    Json(error_response(api_err)),
                );
            }
        };
        if let Err(api_err) = validate_region_codes(&regions) {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
        if !regions.is_empty() {
            full_query = append_or_filter(full_query, "region", &regions);
        }
    }

    if params.date_from.is_some() || params.date_to.is_some() {
        let from_dt = match parse_date_start(&params.date_from) {
            Ok(value) => value,
            Err(msg) => {
                let api_err = ApiError::bad_request(msg);
                return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
            }
        };
        let to_dt = match parse_date_end(&params.date_to) {
            Ok(value) => value,
            Err(msg) => {
                let api_err = ApiError::bad_request(msg);
                return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
            }
        };
        if let Err(msg) = validate_date_range(&from_dt, &to_dt) {
            let api_err = ApiError::bad_request(msg);
            return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
        }
        let from = from_dt.map(|dt| dt.timestamp()).unwrap_or(i64::MIN / 2);
        let to = to_dt.map(|dt| dt.timestamp()).unwrap_or(i64::MAX / 2);
        full_query = format!("({}) AND timestamp:[{} TO {}]", full_query, from, to);
    }

    let (results, total_hits) =
        match state
            .search_index
            .search_with_total(&full_query, limit, offset)
        {
            Ok(value) => value,
            Err(err) => {
                let err_msg = err.to_string();
                let is_parse_err = err_msg.contains("invalid query")
                    || err_msg.contains("Syntax Error")
                    || err_msg.contains("expected");
                if is_parse_err {
                    let api_err = ApiError::bad_request("Invalid search query syntax");
                    return (StatusCode::BAD_REQUEST, Json(error_response(api_err)));
                }
                tracing::error!(request_id = %request_id, "search failed: {err:#}");
                let api_err = ApiError::internal("Search service error");
                return (
                    StatusCode::from_u16(api_err.http_status())
                        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                    Json(error_response(api_err)),
                );
            }
        };

    let tokens = tokenize_query(&query);
    let mut hits: Vec<SearchHit> = results
        .into_iter()
        .map(|result| SearchHit {
            id: result.id,
            entity_type: result.entity_type,
            title: result.title,
            snippet: highlight_snippet(&result.snippet, &tokens, 240),
            score: result.score as f64,
            region: if result.region.is_empty() {
                None
            } else {
                Some(result.region)
            },
            url: if result.url.is_empty() {
                None
            } else {
                Some(result.url)
            },
            updated_at: Utc
                .timestamp_opt(result.timestamp, 0)
                .single()
                .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap()),
        })
        .collect();

    sort_by_score(&mut hits);
    let facets = build_facets(&hits);

    let response = SearchResponse {
        query,
        total_hits,
        results: hits,
        facets,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("search", duration_ms);

    (StatusCode::OK, Json(success_with_meta(response, meta)))
}

pub(crate) async fn semantic_search(
    State(state): State<AppState>,
    Query(params): Query<apex_api::routes::semantic_search::SemanticSearchQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<apex_api::routes::semantic_search::SemanticSearchResult>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let sanitized_query = apex_api::routes::semantic_search::sanitize_query(&params.q);
    if sanitized_query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_response(ApiError::bad_request("q is required"))),
        );
    }

    let enhanced_query = build_enhanced_search_query(&sanitized_query);
    let fetch_limit = (params.limit().saturating_add(params.offset()))
        .saturating_mul(4)
        .clamp(20, 400);
    let (raw_results, _) =
        match state
            .search_index
            .search_with_total(&enhanced_query, fetch_limit, 0)
        {
            Ok(value) => value,
            Err(err) => {
                tracing::error!(request_id = %request_id, "semantic_search failed: {err:#}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(error_response(ApiError::internal("Semantic search failed"))),
                );
            }
        };

    let entity_types = params
        .entity_types
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    let regions = params
        .regions
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .collect::<Vec<_>>();
    let from_ts = params
        .from_date
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|dt| dt.and_utc().timestamp());
    let to_ts = params
        .to_date
        .and_then(|date| date.and_hms_opt(23, 59, 59))
        .map(|dt| dt.and_utc().timestamp());
    let terms = apex_api::routes::semantic_search::extract_terms(&sanitized_query);
    let term_refs = terms.iter().map(String::as_str).collect::<Vec<_>>();

    let filtered = raw_results
        .into_iter()
        .filter(|result| result_matches_filters(result, &entity_types, &regions, from_ts, to_ts))
        .filter(|result| {
            params
                .min_score
                .is_none_or(|min_score| result.score as f64 >= min_score)
        })
        .collect::<Vec<_>>();

    let total_hits = filtered.len();
    let facets = apex_api::routes::semantic_search::SearchFacets {
        entity_types: {
            let mut counts = HashMap::<String, usize>::new();
            for result in &filtered {
                *counts.entry(result.entity_type.clone()).or_default() += 1;
            }
            let mut values = counts
                .into_iter()
                .map(
                    |(value, count)| apex_api::routes::semantic_search::FacetCount { value, count },
                )
                .collect::<Vec<_>>();
            values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
            values
        },
        regions: {
            let mut counts = HashMap::<String, usize>::new();
            for result in &filtered {
                *counts.entry(result.region.clone()).or_default() += 1;
            }
            let mut values = counts
                .into_iter()
                .map(
                    |(value, count)| apex_api::routes::semantic_search::FacetCount { value, count },
                )
                .collect::<Vec<_>>();
            values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
            values
        },
    };

    let paged = filtered
        .into_iter()
        .skip(params.offset())
        .take(params.limit())
        .collect::<Vec<_>>();
    let results = paged
        .into_iter()
        .map(|result| apex_api::routes::semantic_search::SearchHit {
            entity_id: result.entity_id,
            entity_type: result.entity_type,
            title: result.title,
            snippet: apex_api::routes::semantic_search::generate_snippet(
                &result.snippet,
                &term_refs,
                120,
            ),
            score: result.score as f64,
            region: if result.region.is_empty() {
                None
            } else {
                Some(result.region.clone())
            },
            created_at: Utc.timestamp_opt(result.timestamp, 0).single(),
            highlights: term_refs
                .iter()
                .filter(|term| {
                    result
                        .snippet
                        .to_ascii_lowercase()
                        .contains(&term.to_ascii_lowercase())
                })
                .map(
                    |term| apex_api::routes::semantic_search::HighlightFragment {
                        field: "snippet".to_string(),
                        text: (*term).to_string(),
                    },
                )
                .collect(),
        })
        .collect::<Vec<_>>();

    let response = apex_api::routes::semantic_search::SemanticSearchResult {
        total_hits,
        results,
        facets,
        query_time_ms: start.elapsed().as_millis() as u64,
    };

    (StatusCode::OK, Json(success(response)))
}

#[cfg(test)]
mod semantic_search_tests {
    use super::build_enhanced_search_query;

    #[test]
    fn enhanced_search_query_includes_phrase_and_fuzzy_clauses() {
        let query = build_enhanced_search_query("pcb assembly");

        assert!(query.contains("\"pcb assembly\"^4"));
        assert!(query.contains("title:pcb^3"));
        assert!(query.contains("assembly~1"));
    }
}

pub(crate) async fn list_graph(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<GraphOverviewWithEdges>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let companies_total = match state
        .store
        .count_companies(&CompanyListFilters::default())
        .await
    {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count companies failed: {err:#}");
            let api_err = ApiError::internal("Failed to count companies");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let persons_total = match state
        .store
        .count_persons(&PersonListFilters::default())
        .await
    {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count persons failed: {err:#}");
            let api_err = ApiError::internal("Failed to count persons");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let warnings_total = match state
        .store
        .count_warnings(&WarningListFilters::default())
        .await
    {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count warnings failed: {err:#}");
            let api_err = ApiError::internal("Failed to count warnings");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let insights_total = match state
        .store
        .count_insights(&InsightListFilters::default())
        .await
    {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::error!(request_id = %request_id, "count insights failed: {err:#}");
            let api_err = ApiError::internal("Failed to count insights");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let edges_total = match state.store.count_edges().await {
        Ok(value) => value.max(0) as u64,
        Err(err) => {
            tracing::warn!(request_id = %request_id, "count edges failed: {err:#}");
            0
        }
    };

    let edge_rows = match state.store.list_all_edges(200).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(request_id = %request_id, "list edges failed: {err:#}");
            Vec::new()
        }
    };

    let edges: Vec<GraphEdge> = edge_rows.iter().map(edge_row_to_graph_edge).collect();
    let edge_type_counts = compute_edge_type_counts(&edge_rows);

    let node_ids: Vec<Uuid> = {
        let mut seen = std::collections::HashSet::new();
        for row in &edge_rows {
            seen.insert(row.source_id);
            seen.insert(row.target_id);
        }
        seen.into_iter().collect()
    };

    let node_labels: Vec<GraphNodeLabel> = if node_ids.is_empty() {
        Vec::new()
    } else {
        #[derive(sqlx::FromRow)]
        struct NodeNameRow {
            id: Uuid,
            label: String,
            node_type: String,
        }
        sqlx::query_as::<_, NodeNameRow>(
            r#"SELECT id, name AS label, 'company' AS node_type FROM companies WHERE id = ANY($1)
               UNION ALL
               SELECT id, name AS label, 'person' AS node_type FROM persons WHERE id = ANY($1)"#,
        )
        .bind(&node_ids)
        .fetch_all(&state.store.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| GraphNodeLabel {
            id: row.id.to_string(),
            label: row.label,
            node_type: row.node_type,
        })
        .collect()
    };

    let payload = GraphOverviewWithEdges {
        companies_total,
        persons_total,
        warnings_total,
        insights_total,
        edges_total,
        nodes: node_labels,
        edges,
        edge_type_counts,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_graph", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

pub(crate) async fn list_recipes(
    State(state): State<AppState>,
    Query(params): Query<ListRecipesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<RecipeListItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _offset) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => {
            return (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(err)),
            );
        }
    };

    let recipe_stats = match state.store.get_recipe_stats().await {
        Ok(stats) => {
            tracing::info!(request_id = %request_id, count = stats.len(), "recipe stats loaded");
            stats
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "get recipe stats failed: {err:#}");
            let api_err = ApiError::internal("Failed to load recipe statistics");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let mut items: Vec<RecipeListItem> = recipe_stats
        .into_iter()
        .map(|stat| recipe_stat_to_list_item(&stat))
        .collect();

    if let Some(status_raw) = params.status.as_deref() {
        let status = match parse_recipe_status(status_raw) {
            Some(value) => value,
            None => {
                let err = ApiError::validation(
                    "status",
                    "expected one of: staging|production|deprecated",
                );
                return (
                    StatusCode::from_u16(err.http_status())
                        .unwrap_or(StatusCode::UNPROCESSABLE_ENTITY),
                    Json(error_response(err)),
                );
            }
        };
        items.retain(|item| item.status == status);
    }

    if let Some(search) = params.search.as_deref() {
        let search = search.trim().to_lowercase();
        if !search.is_empty() {
            items.retain(|item| {
                item.id.to_lowercase().contains(&search)
                    || item.name.to_lowercase().contains(&search)
                    || item.description.to_lowercase().contains(&search)
            });
        }
    }

    if let Some(min_precision) = params.min_precision {
        let min_precision = clamp_ratio(min_precision);
        items.retain(|item| item.precision >= min_precision);
    }

    if let Some(region) = params.region.as_deref() {
        let region = region.trim().to_lowercase();
        items.retain(|item| {
            item.region
                .as_deref()
                .map(|value| value.eq_ignore_ascii_case(&region))
                .unwrap_or(false)
        });
    }

    let sort_field = params.sort_by.clone().unwrap_or(RecipeSortField::CreatedAt);
    let sort_desc = !matches!(sort_field, RecipeSortField::Name);
    sort_recipes(&mut items, &sort_field, sort_desc);

    let total = items.len() as u64;
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as usize).saturating_mul(per_page as usize);
    let paged_items = items
        .into_iter()
        .skip(offset)
        .take(per_page as usize)
        .collect();

    let payload = PagedResponse {
        items: paged_items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_recipes", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

pub(crate) async fn list_security(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<SecuritySummary>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let dns_rows = state
        .store
        .get_dns_posture_entries(500)
        .await
        .unwrap_or_default();
    let lookalike_rows = state
        .store
        .get_lookalike_domains(5000)
        .await
        .unwrap_or_default();
    let kev_rows = state.store.get_kev_relevance(200).await.unwrap_or_default();

    let domains_monitored = dns_rows.len() as u64;
    let dns_scores: Vec<f64> = dns_rows
        .iter()
        .map(|row| {
            let has_spf = row
                .value
                .get("has_spf")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let has_dkim = row
                .value
                .get("has_dkim")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let has_dmarc = row
                .value
                .get("has_dmarc")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            dns_score(has_spf, has_dkim, has_dmarc) * 100.0
        })
        .collect();
    let dns_posture_score = if dns_scores.is_empty() {
        0.0
    } else {
        dns_scores.iter().sum::<f64>() / dns_scores.len() as f64
    };

    let lookalike_domains_detected = lookalike_rows.len() as u64;
    let kev_matches = kev_rows.len() as u64;

    let last_scan_at = dns_rows
        .iter()
        .chain(lookalike_rows.iter())
        .chain(kev_rows.iter())
        .map(|row| row.ts_utc)
        .max()
        .map(|ts| ts.to_rfc3339());

    let payload = SecuritySummary {
        dns_posture_score,
        lookalike_domains_detected,
        kev_matches,
        domains_monitored,
        last_scan_at,
    };

    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_security", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

#[cfg(test)]
mod tests {
    use super::append_or_filter;

    #[test]
    fn test_append_or_filter_wraps_existing_query() {
        let query = append_or_filter(
            "chips".to_string(),
            "entity_type",
            &["company".to_string(), "person".to_string()],
        );

        assert_eq!(
            query,
            "(chips) AND (entity_type:company OR entity_type:person)"
        );
    }

    #[test]
    fn test_append_or_filter_ignores_unsafe_tokens() {
        let query = append_or_filter(
            "chips".to_string(),
            "region",
            &["US".to_string(), "EU OR *".to_string()],
        );

        assert_eq!(query, "(chips) AND (region:US)");
    }
}
