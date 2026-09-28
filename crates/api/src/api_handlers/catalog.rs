use crate::*;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListSitesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
    region: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListCapabilitiesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListCertificationsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListObservationsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    entity_id: Option<String>,
    observation_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListProductFamiliesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    company_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListLogisticsNodesQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    country_code: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListRegulationsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    jurisdiction: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ListPoiArtifactsQuery {
    page: Option<u32>,
    per_page: Option<u32>,
    person_id: Option<String>,
}

fn parse_optional_uuid_filter(raw: Option<&str>) -> Option<Uuid> {
    raw.and_then(|value| Uuid::parse_str(value).ok())
}

pub(crate) async fn list_sites(
    State(state): State<AppState>,
    Query(params): Query<ListSitesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<SiteRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = parse_optional_uuid_filter(params.company_id.as_deref());
    let region = params.region.as_deref();
    let total = match state.store.count_sites(company_id, region).await {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count sites: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to count sites"))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_sites(company_id, region, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list sites: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal("Failed to list sites"))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_sites", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn list_capabilities(
    State(state): State<AppState>,
    Query(params): Query<ListCapabilitiesQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<CapabilityRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = parse_optional_uuid_filter(params.company_id.as_deref());
    let total = match state.store.count_capabilities(company_id).await {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count capabilities: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count capabilities",
                ))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_capabilities(company_id, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list capabilities: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list capabilities",
                ))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_capabilities", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn list_certifications_all(
    State(state): State<AppState>,
    Query(params): Query<ListCertificationsQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<PagedResponse<CertificationRow>>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = parse_optional_uuid_filter(params.company_id.as_deref());
    let total = match state.store.count_certifications(company_id).await {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count certifications: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count certifications",
                ))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_certifications(company_id, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list certifications: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list certifications",
                ))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_certifications", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn list_observations(
    State(state): State<AppState>,
    Query(params): Query<ListObservationsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<ObservationRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let entity_id = parse_optional_uuid_filter(params.entity_id.as_deref());
    let observation_type = params.observation_type.as_deref();
    let total = match state
        .store
        .count_observations(entity_id, observation_type)
        .await
    {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count observations: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count observations",
                ))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_observations(entity_id, observation_type, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list observations: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list observations",
                ))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_observations", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn list_product_families(
    State(state): State<AppState>,
    Query(params): Query<ListProductFamiliesQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<PagedResponse<ProductFamilyRow>>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let company_id = parse_optional_uuid_filter(params.company_id.as_deref());
    let total = match state.store.count_product_families(company_id).await {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count product families: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count product families",
                ))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_product_families(company_id, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list product families: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list product families",
                ))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_product_families", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn list_logistics_nodes(
    State(state): State<AppState>,
    Query(params): Query<ListLogisticsNodesQuery>,
) -> (
    StatusCode,
    Json<ApiResponse<PagedResponse<LogisticsNodeRow>>>,
) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let country_code = params.country_code.as_deref();
    let total = match state.store.count_logistics_nodes(country_code).await {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count logistics nodes: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count logistics nodes",
                ))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_logistics_nodes(country_code, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list logistics nodes: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list logistics nodes",
                ))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_logistics_nodes", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn list_regulations(
    State(state): State<AppState>,
    Query(params): Query<ListRegulationsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<RegulationRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let jurisdiction = params.jurisdiction.as_deref();
    let total = match state.store.count_regulations(jurisdiction).await {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count regulations: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count regulations",
                ))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_regulations(jurisdiction, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list regulations: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list regulations",
                ))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_regulations", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn list_poi_artifacts(
    State(state): State<AppState>,
    Query(params): Query<ListPoiArtifactsQuery>,
) -> (StatusCode, Json<ApiResponse<PagedResponse<ArtifactRow>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let (page, per_page, _) = match validate_pagination(params.page, params.per_page) {
        Ok(pagination) => pagination,
        Err(err) => return (StatusCode::BAD_REQUEST, Json(error_response(err))),
    };
    let person_id = parse_optional_uuid_filter(params.person_id.as_deref());
    let total = match state.store.count_poi_artifacts(person_id).await {
        Ok(total) => total.max(0) as u64,
        Err(err) => {
            tracing::error!("Failed to count POI artifacts: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to count POI artifacts",
                ))),
            );
        }
    };
    let clamped_page = clamp_page(page, per_page, total);
    let offset = ((clamped_page - 1) as i64) * (per_page as i64);
    let items = match state
        .store
        .list_poi_artifacts(person_id, per_page as i64, offset)
        .await
    {
        Ok(items) => items,
        Err(err) => {
            tracing::error!("Failed to list POI artifacts: {err:#}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to list POI artifacts",
                ))),
            );
        }
    };
    let payload = PagedResponse {
        items,
        total,
        page: clamped_page,
        per_page,
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("list_poi_artifacts", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            payload,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

pub(crate) async fn get_dashboard(
    State(state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<DashboardStats>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let stats = match state.store.get_dashboard_stats().await {
        Ok(stats) => stats,
        Err(err) => {
            tracing::error!(request_id = %request_id, "dashboard stats: {err:#}");
            let api_err = ApiError::internal("Failed to load dashboard stats");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(api_err)),
            );
        }
    };
    let duration_ms = start.elapsed().as_millis() as u64;
    log_latency("get_dashboard", duration_ms);
    (
        StatusCode::OK,
        Json(success_with_meta(
            stats,
            ResponseMeta::now()
                .with_request_id(request_id)
                .with_duration(duration_ms),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_optional_uuid_filter;
    use uuid::Uuid;

    #[test]
    fn test_parse_optional_uuid_filter_accepts_valid_uuid() {
        let uuid = Uuid::new_v4();

        assert_eq!(
            parse_optional_uuid_filter(Some(&uuid.to_string())),
            Some(uuid)
        );
    }

    #[test]
    fn test_parse_optional_uuid_filter_ignores_invalid_uuid() {
        assert_eq!(parse_optional_uuid_filter(Some("not-a-uuid")), None);
        assert_eq!(parse_optional_uuid_filter(None), None);
    }
}
