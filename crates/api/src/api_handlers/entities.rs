#![allow(clippy::disallowed_methods)]

use crate::*;

fn resolve_person_priority_bounds(params: &ListPersonsQuery) -> (Option<f64>, Option<f64>) {
    let (tier_min, tier_max): (Option<f64>, Option<f64>) = match params.tier.as_deref() {
        Some("critical") => (Some(0.8), None),
        Some("high") => (Some(0.6), Some(0.8)),
        Some("medium") => (Some(0.4), Some(0.6)),
        Some("low") => (None, Some(0.4)),
        _ => (None, None),
    };
    let (priority_min, priority_max): (Option<f64>, Option<f64>) = match params.priority.as_deref()
    {
        Some("A") | Some("a") => (Some(0.8), None),
        Some("B") | Some("b") => (Some(0.5), Some(0.8)),
        Some("C") | Some("c") => (None, Some(0.5)),
        _ => (None, None),
    };

    (
        tier_min
            .or(priority_min)
            .or_else(|| params.min_priority.map(clamp_ratio)),
        if tier_max.is_some() {
            tier_max
        } else {
            priority_max
        },
    )
}

pub(crate) async fn list_companies(
    State(state): State<AppState>,
    Query(params): Query<ListCompaniesQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<PagedResponse<CompanyListItem>>>,
) {
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

    let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
        Ok(v) => v,
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

    let filters = CompanyListFilters {
        regions,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(v) => v,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status())
                            .unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
        is_competitor: params.is_competitor,
    };

    let sort_field = params.sort_by.clone();
    let order_by = sort_field.clone().map(map_company_sort);
    let desc = params
        .sort_dir
        .clone()
        .map(|d| d == SortDirection::Desc)
        .unwrap_or_else(|| {
            matches!(
                sort_field,
                Some(CompanySortField::ThreatScore | CompanySortField::UpdatedAt)
            )
        });

    let total = match tracing::info_span!("db.count_companies", request_id = %request_id)
        .in_scope(|| state.store.count_companies(&filters))
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

    let clamped_page = clamp_page(page, per_page, total);
    let clamped_offset = ((clamped_page - 1) as i64).saturating_mul(per_page as i64);

    let rows = match tracing::info_span!("db.list_companies", request_id = %request_id)
        .in_scope(|| {
            state
                .store
                .list_companies(&filters, order_by, desc, per_page as i64, clamped_offset)
        })
        .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list companies failed: {err:#}");
            let api_err = ApiError::internal("Failed to list companies");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let items: Vec<CompanyListItem> = rows.into_iter().map(company_row_to_item).collect();
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_companies", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

pub(crate) async fn list_persons(
    State(state): State<AppState>,
    Query(params): Query<ListPersonsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<PersonListItem>>>) {
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

    let regions = match parse_csv_upper_strict(&params.regions, 32, "regions") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    let roles = match parse_csv_lower_strict(&params.roles, 32, "roles") {
        Ok(v) => v,
        Err(api_err) => {
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    let (min_priority, max_priority) = resolve_person_priority_bounds(&params);

    let filters = PersonListFilters {
        regions,
        roles,
        search: match params.search.as_deref() {
            Some(value) => match validate_search_text(value, 500) {
                Ok(v) => v,
                Err(msg) => {
                    let api_err = ApiError::validation("search", msg);
                    return (
                        StatusCode::from_u16(api_err.http_status())
                            .unwrap_or(StatusCode::BAD_REQUEST),
                        Json(error_response(api_err)),
                    );
                }
            },
            None => None,
        },
        min_priority,
        max_priority,
    };

    let sort_field = params.sort_by.clone();
    let order_by = sort_field.clone().map(map_person_sort);
    let desc = params
        .sort_by
        .clone()
        .map(|v| matches!(v, PersonSortField::Priority | PersonSortField::UpdatedAt))
        .unwrap_or(true);

    let total = match tracing::info_span!("db.count_persons", request_id = %request_id)
        .in_scope(|| state.store.count_persons(&filters))
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

    let clamped_page = clamp_page(page, per_page, total);
    let clamped_offset = ((clamped_page - 1) as i64).saturating_mul(per_page as i64);

    let rows = match tracing::info_span!("db.list_persons", request_id = %request_id)
        .in_scope(|| {
            state
                .store
                .list_persons(&filters, order_by, desc, per_page as i64, clamped_offset)
        })
        .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::error!(request_id = %request_id, "list persons failed: {err:#}");
            let api_err = ApiError::internal("Failed to list persons");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let items: Vec<PersonListItem> = rows.into_iter().map(person_row_to_item).collect();
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("list_persons", duration_ms);

    (StatusCode::OK, Json(success_with_meta(payload, meta)))
}

pub(crate) async fn get_company_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<CompanyDetail>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let company_id = match validate_company_id(&id) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::validation("company_id", msg);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    let dossier = match tracing::info_span!("db.get_company_dossier", request_id = %request_id)
        .in_scope(|| state.store.get_company_dossier(company_id))
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) => {
            let api_err = ApiError::not_found("company", &id);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(api_err)),
            );
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "get company failed: {err:#}");
            let api_err = ApiError::internal("Failed to load company");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let persons = match state.store.list_persons_by_org(company_id).await {
        Ok(values) => values,
        Err(err) => {
            tracing::error!(request_id = %request_id, "company detail lookup failed: {err:#}");
            let api_err = ApiError::internal("Failed to load company detail");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let detail = company_row_to_detail(
        dossier.company.clone(),
        dossier.sites.clone(),
        dossier.certifications.clone(),
        persons,
    );
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("get_company_detail", duration_ms);

    (StatusCode::OK, Json(success_with_meta(detail, meta)))
}

pub(crate) async fn get_person_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<ApiResponse<PersonDetail>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();

    let person_id = match validate_person_id(&id) {
        Ok(value) => value,
        Err(msg) => {
            let api_err = ApiError::validation("person_id", msg);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::BAD_REQUEST),
                Json(error_response(api_err)),
            );
        }
    };

    let row = match tracing::info_span!("db.get_person", request_id = %request_id)
        .in_scope(|| state.store.get_person(person_id))
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) => {
            let api_err = ApiError::not_found("person", &id);
            return (
                StatusCode::from_u16(api_err.http_status()).unwrap_or(StatusCode::NOT_FOUND),
                Json(error_response(api_err)),
            );
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "get person failed: {err:#}");
            let api_err = ApiError::internal("Failed to load person");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let org_id = row.primary_org_id;
    let org_name = if let Some(oid) = org_id {
        match state.store.get_company(oid).await {
            Ok(Some(company)) => company.name,
            Ok(None) => "Independent".to_string(),
            Err(err) => {
                tracing::error!(request_id = %request_id, "get person org failed: {err:#}");
                "Independent".to_string()
            }
        }
    } else {
        "Independent".to_string()
    };

    let role_family = row
        .role_family
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let region = row.region.clone().unwrap_or_default();
    let entity_ids = vec![person_id];
    let (artifacts, role_history_rows, peer_rows, related_warnings, related_insights) = match tokio::try_join!(
        state.store.get_artifacts_for_person(person_id, 12),
        state.store.get_role_history_for_person(person_id),
        state
            .store
            .get_person_peers(person_id, &role_family, &region, 6),
        state.store.get_warnings_by_entity_ids(&entity_ids, 200),
        state.store.get_insights_by_entity_ids(&entity_ids, 200),
    ) {
        Ok(values) => values,
        Err(err) => {
            tracing::error!(request_id = %request_id, "get person detail data failed: {err:#}");
            let api_err = ApiError::internal("Failed to load person detail");
            return (
                StatusCode::from_u16(api_err.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(error_response(api_err)),
            );
        }
    };

    let mut detail = person_row_to_detail(row, org_name, artifacts, &state.config.priority_weights);
    detail.role_history = role_history_rows
        .into_iter()
        .map(|entry| routes::persons::RoleHistoryEntry {
            organization: entry.org_name,
            role: entry.title,
            role_family: entry.role_family,
            start_date: entry.start_date.map(|value| value.to_string()),
            end_date: entry.end_date.map(|value| value.to_string()),
            is_current: entry.end_date.is_none(),
            confidence: entry.confidence,
        })
        .collect();
    detail.peers = peer_rows
        .into_iter()
        .map(|peer| {
            let item = person_row_to_item(peer);
            routes::persons::PeerSummary {
                id: item.id,
                name: item.name,
                role: item.role,
                organization: item.organization,
                region: item.region,
                priority_score: item.priority_score,
                influence_tier: item.influence_tier,
            }
        })
        .collect();
    detail.warning_count = related_warnings.len() as i64;
    detail.insight_count = related_insights.len() as i64;
    let duration_ms = start.elapsed().as_millis() as u64;
    let meta = ResponseMeta::now()
        .with_request_id(request_id)
        .with_duration(duration_ms);
    log_latency("get_person_detail", duration_ms);

    (StatusCode::OK, Json(success_with_meta(detail, meta)))
}

#[cfg(test)]
mod tests {
    use super::resolve_person_priority_bounds;
    use crate::ListPersonsQuery;

    #[test]
    fn test_resolve_person_priority_bounds_prefers_tier_over_priority() {
        let params = ListPersonsQuery {
            page: None,
            per_page: None,
            search: None,
            region: None,
            regions: None,
            roles: None,
            priority: Some("A".to_string()),
            tier: Some("medium".to_string()),
            min_priority: Some(0.9),
            sort_by: None,
        };

        let (min_priority, max_priority) = resolve_person_priority_bounds(&params);

        assert_eq!(min_priority, Some(0.4));
        assert_eq!(max_priority, Some(0.6));
    }

    #[test]
    fn test_resolve_person_priority_bounds_falls_back_to_explicit_min_priority() {
        let params = ListPersonsQuery {
            page: None,
            per_page: None,
            search: None,
            region: None,
            regions: None,
            roles: None,
            priority: None,
            tier: None,
            min_priority: Some(1.5),
            sort_by: None,
        };

        let (min_priority, max_priority) = resolve_person_priority_bounds(&params);

        assert_eq!(min_priority, Some(1.0));
        assert_eq!(max_priority, None);
    }
}
