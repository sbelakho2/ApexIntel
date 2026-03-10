use super::super::*;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct SecuritySubQuery {
    limit: Option<i64>,
}

fn normalize_security_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).min(200)
}

pub(crate) async fn get_dns_posture(
    State(state): State<AppState>,
    Query(params): Query<SecuritySubQuery>,
) -> (StatusCode, Json<ApiResponse<DnsPostureOverview>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = normalize_security_limit(params.limit);
    match state.store.get_dns_posture_entries(limit).await {
        Ok(rows) => {
            let items: Vec<DnsPostureItem> = rows
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
                    DnsPostureItem {
                        domain: row
                            .value
                            .get("domain")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        company_id: row.entity_id.map(|id| id.to_string()).unwrap_or_default(),
                        company_name: row
                            .value
                            .get("company_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        has_spf,
                        has_dkim,
                        has_dmarc,
                        dmarc_policy: row
                            .value
                            .get("dmarc_policy")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        posture_score: dns_score(has_spf, has_dkim, has_dmarc) * 100.0,
                        last_checked: row.ts_utc.to_rfc3339(),
                    }
                })
                .collect();
            let overall_score = if items.is_empty() {
                0.0
            } else {
                items.iter().map(|item| item.posture_score).sum::<f64>() / items.len() as f64
            };
            let domains_checked = items.len();
            let overview = DnsPostureOverview {
                items,
                overall_score,
                domains_checked,
            };
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_dns_posture", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    overview,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "dns posture failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load DNS posture",
                ))),
            )
        }
    }
}

pub(crate) async fn get_lookalike_domains(
    State(state): State<AppState>,
    Query(params): Query<SecuritySubQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<LookalikeDomainItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = normalize_security_limit(params.limit);
    match state.store.get_lookalike_domains(limit).await {
        Ok(rows) => {
            let items: Vec<LookalikeDomainItem> = rows
                .iter()
                .map(|row| LookalikeDomainItem {
                    id: row.id.to_string(),
                    original_domain: row
                        .value
                        .get("original_domain")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    lookalike_domain: row
                        .value
                        .get("domain")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    distance: row
                        .value
                        .get("distance")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(1),
                    threat_type: row
                        .value
                        .get("threat_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("typosquat")
                        .to_string(),
                    detected_at: row.ts_utc.to_rfc3339(),
                    active: row
                        .value
                        .get("active")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    registrar: row
                        .value
                        .get("registrar")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    registration_date: row
                        .value
                        .get("registration_date")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
                .collect();
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_lookalike_domains", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    items,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "lookalike domains failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load lookalike domains",
                ))),
            )
        }
    }
}

pub(crate) async fn get_kev_relevance(
    State(state): State<AppState>,
    Query(params): Query<SecuritySubQuery>,
) -> (StatusCode, Json<ApiResponse<Vec<KevItem>>>) {
    let start = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let limit = normalize_security_limit(params.limit);
    match state.store.get_kev_relevance(limit).await {
        Ok(rows) => {
            let items: Vec<KevItem> = rows
                .iter()
                .map(|row| KevItem {
                    cve_id: row
                        .value
                        .get("cve_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    vendor: row
                        .value
                        .get("vendor")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    product: row
                        .value
                        .get("product")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    vulnerability_name: row
                        .value
                        .get("vulnerability_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    date_added: row
                        .value
                        .get("date_added")
                        .and_then(|v| v.as_str())
                        .unwrap_or("1970-01-01")
                        .to_string(),
                    due_date: row
                        .value
                        .get("due_date")
                        .and_then(|v| v.as_str())
                        .unwrap_or("1970-01-01")
                        .to_string(),
                    relevance_score: row
                        .value
                        .get("relevance_score")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.5),
                    affected_companies: row
                        .value
                        .get("affected_companies")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|s| s.as_str().map(|ss| ss.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    notes: row
                        .value
                        .get("notes")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                })
                .collect();
            let duration_ms = start.elapsed().as_millis() as u64;
            log_latency("get_kev_relevance", duration_ms);
            (
                StatusCode::OK,
                Json(success_with_meta(
                    items,
                    ResponseMeta::now()
                        .with_request_id(request_id)
                        .with_duration(duration_ms),
                )),
            )
        }
        Err(err) => {
            tracing::error!(request_id = %request_id, "kev relevance failed: {err:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(error_response(ApiError::internal(
                    "Failed to load KEV relevance",
                ))),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_security_limit;

    #[test]
    fn test_normalize_security_limit_defaults_to_fifty() {
        assert_eq!(normalize_security_limit(None), 50);
    }

    #[test]
    fn test_normalize_security_limit_caps_at_two_hundred() {
        assert_eq!(normalize_security_limit(Some(500)), 200);
    }
}
