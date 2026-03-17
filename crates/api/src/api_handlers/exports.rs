#![allow(clippy::disallowed_methods)]

use crate::*;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ExportQuery {
    pub window: Option<u32>,
    pub cursor: Option<u32>,
}

fn csv_escape(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn normalize_export_window(
    config: &ApiRuntimeConfig,
    query: &ExportQuery,
) -> std::result::Result<(u32, u32), ApiError> {
    let window = query.window.unwrap_or(config.export.default_window);
    if window == 0 {
        return Err(ApiError::bad_request("export window must be at least 1 row"));
    }
    if window > config.export.max_window {
        return Err(ApiError::bad_request(format!(
            "export window {} exceeds maximum {}",
            window, config.export.max_window
        )));
    }

    Ok((window, query.cursor.unwrap_or(0)))
}

fn export_error_response(err: ApiError) -> axum::response::Response {
    let status = StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    (status, Json(error_response::<serde_json::Value>(err))).into_response()
}

fn csv_stream_response(
    body: axum::body::Body,
    download_name: &str,
    window: u32,
    cursor: u32,
    chunk_size: u32,
) -> axum::response::Response {
    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", download_name),
        )
        .header("x-export-window", window.to_string())
        .header("x-export-cursor", cursor.to_string())
        .header("x-export-chunk-size", chunk_size.to_string())
        .body(body)
        .unwrap_or_else(|err| {
            tracing::error!(%err, "failed to build csv stream response");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
        })
}

type CsvByteStream = std::pin::Pin<
    Box<
        dyn futures_core::Stream<Item = std::result::Result<axum::body::Bytes, std::io::Error>>
            + Send,
    >,
>;

pub(crate) async fn export_companies_csv(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> axum::response::Response {
    let (window, cursor) = match normalize_export_window(&state.config, &query) {
        Ok(bounds) => bounds,
        Err(err) => return export_error_response(err),
    };
    let chunk_size = state.config.export.chunk_size.min(window).max(1);
    let store = state.store.clone();
    let user_id = auth_ctx.user_id.clone();
    let filters_json = serde_json::json!({"cursor": cursor, "window": window});

    let stream: CsvByteStream = Box::pin(async_stream::try_stream! {
        yield axum::body::Bytes::from_static(b"id,name,domain,region,country,entity_type,is_competitor,threat_score,capabilities,updated_at\n");

        let filters = CompanyListFilters::default();
        let mut emitted: i64 = 0;
        let mut offset = cursor as i64;
        let target = window as i64;

        while emitted < target {
            let batch_limit = (target - emitted).min(chunk_size as i64);
            let rows = store
                .list_companies(&filters, None, true, batch_limit, offset)
                .await
                .map_err(std::io::Error::other)?;
            if rows.is_empty() {
                break;
            }

            let fetched = rows.len() as i64;
            let mut chunk = String::new();
            for row in rows {
                let item = company_row_to_item(row);
                chunk.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{}\n",
                    csv_escape(&item.id),
                    csv_escape(&item.name),
                    csv_escape(item.domain.as_deref().unwrap_or("")),
                    csv_escape(&item.region),
                    csv_escape(&item.country),
                    csv_escape(&item.entity_type),
                    item.is_competitor,
                    item.threat_score
                        .map(|score| format!("{score:.4}"))
                        .unwrap_or_default(),
                    csv_escape(&item.capabilities.join("|")),
                    csv_escape(&item.updated_at.to_rfc3339()),
                ));
            }

            emitted += fetched;
            offset += fetched;
            yield axum::body::Bytes::from(chunk);

            if fetched < batch_limit {
                break;
            }
        }

        let _ = store
            .record_export_history(&user_id, "companies", "csv", &filters_json, emitted, Some("companies.csv"))
            .await;
        let _ = store
            .record_audit_event(
                &user_id,
                "companies_exported",
                &serde_json::json!({
                    "format": "csv",
                    "row_count": emitted,
                    "download_name": "companies.csv",
                    "cursor": cursor,
                    "window": window,
                }),
            )
            .await;
            });

    csv_stream_response(
        axum::body::Body::from_stream(stream),
        "companies.csv",
        window,
        cursor,
        chunk_size,
    )
}

pub(crate) async fn export_persons_csv(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> axum::response::Response {
    let (window, cursor) = match normalize_export_window(&state.config, &query) {
        Ok(bounds) => bounds,
        Err(err) => return export_error_response(err),
    };
    let chunk_size = state.config.export.chunk_size.min(window).max(1);
    let store = state.store.clone();
    let user_id = auth_ctx.user_id.clone();
    let filters_json = serde_json::json!({"cursor": cursor, "window": window});

    let stream: CsvByteStream = Box::pin(async_stream::try_stream! {
        yield axum::body::Bytes::from_static(b"id,name,role,organization,region,priority_score,engagement_status,updated_at\n");

        let filters = PersonListFilters::default();
        let mut emitted: i64 = 0;
        let mut offset = cursor as i64;
        let target = window as i64;

        while emitted < target {
            let batch_limit = (target - emitted).min(chunk_size as i64);
            let rows = store
                .list_persons(&filters, None, true, batch_limit, offset)
                .await
                .map_err(std::io::Error::other)?;
            if rows.is_empty() {
                break;
            }

            let fetched = rows.len() as i64;
            let mut chunk = String::new();
            for row in rows {
                let item = person_row_to_item(row);
                chunk.push_str(&format!(
                    "{},{},{},{},{},{:.4},{},{}\n",
                    csv_escape(&item.id),
                    csv_escape(&item.name),
                    csv_escape(&item.role),
                    csv_escape(&item.organization),
                    csv_escape(&item.region),
                    item.priority_score,
                    csv_escape(&item.engagement_status),
                    csv_escape(&item.updated_at.to_rfc3339()),
                ));
            }

            emitted += fetched;
            offset += fetched;
            yield axum::body::Bytes::from(chunk);

            if fetched < batch_limit {
                break;
            }
        }

        let _ = store
            .record_export_history(&user_id, "persons", "csv", &filters_json, emitted, Some("persons.csv"))
            .await;
        let _ = store
            .record_audit_event(
                &user_id,
                "persons_exported",
                &serde_json::json!({
                    "format": "csv",
                    "row_count": emitted,
                    "download_name": "persons.csv",
                    "cursor": cursor,
                    "window": window,
                }),
            )
            .await;
            });

    csv_stream_response(
        axum::body::Body::from_stream(stream),
        "persons.csv",
        window,
        cursor,
        chunk_size,
    )
}

pub(crate) async fn export_insights_csv(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> axum::response::Response {
    let (window, cursor) = match normalize_export_window(&state.config, &query) {
        Ok(bounds) => bounds,
        Err(err) => return export_error_response(err),
    };
    let chunk_size = state.config.export.chunk_size.min(window).max(1);
    let store = state.store.clone();
    let user_id = auth_ctx.user_id.clone();
    let filters_json = serde_json::json!({"exclude_internal": true, "cursor": cursor, "window": window});

    let stream: CsvByteStream = Box::pin(async_stream::try_stream! {
        yield axum::body::Bytes::from_static(b"id,title,insight_type,summary,region,confidence,created_at\n");

        let filters = InsightListFilters {
            exclude_internal: true,
            ..Default::default()
        };
        let mut emitted: i64 = 0;
        let mut offset = cursor as i64;
        let target = window as i64;

        while emitted < target {
            let batch_limit = (target - emitted).min(chunk_size as i64);
            let rows = store
                .list_insights(&filters, batch_limit, offset)
                .await
                .map_err(std::io::Error::other)?;
            if rows.is_empty() {
                break;
            }

            let fetched = rows.len() as i64;
            let mut chunk = String::new();
            for row in rows {
                chunk.push_str(&format!(
                    "{},{},{},{},{},{},{}\n",
                    csv_escape(&row.id.to_string()),
                    csv_escape(&row.title),
                    csv_escape(row.insight_type.as_deref().unwrap_or("")),
                    csv_escape(&row.summary),
                    csv_escape(row.region.as_deref().unwrap_or("")),
                    row.confidence
                        .map(|confidence| format!("{confidence:.4}"))
                        .unwrap_or_default(),
                    csv_escape(&row.created_at.unwrap_or_else(Utc::now).to_rfc3339()),
                ));
            }

            emitted += fetched;
            offset += fetched;
            yield axum::body::Bytes::from(chunk);

            if fetched < batch_limit {
                break;
            }
        }

        let _ = store
            .record_export_history(&user_id, "insights", "csv", &filters_json, emitted, Some("insights.csv"))
            .await;
        let _ = store
            .record_audit_event(
                &user_id,
                "insights_exported",
                &serde_json::json!({
                    "format": "csv",
                    "row_count": emitted,
                    "download_name": "insights.csv",
                    "exclude_internal": true,
                    "cursor": cursor,
                    "window": window,
                }),
            )
            .await;
            });

    csv_stream_response(
        axum::body::Body::from_stream(stream),
        "insights.csv",
        window,
        cursor,
        chunk_size,
    )
}

#[cfg(test)]
mod tests {
    use super::{csv_escape, normalize_export_window, ExportQuery};
    use apex_api::config::ApiRuntimeConfig;

    #[test]
    fn test_csv_escape_quotes_and_wraps_fields() {
        assert_eq!(csv_escape("alpha,beta"), "\"alpha,beta\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn test_csv_escape_preserves_empty_string() {
        assert_eq!(csv_escape(""), "\"\"");
    }

    #[test]
    fn test_export_window_rejects_unbounded_or_oversized_requests() {
        std::env::remove_var("PORT");
        std::env::set_var("DATABASE_URL", "postgres://postgres:postgres@localhost/apex");
        let config = ApiRuntimeConfig::from_env().expect("config");

        let zero = normalize_export_window(
            &config,
            &ExportQuery {
                window: Some(0),
                cursor: Some(0),
            },
        )
        .expect_err("zero window should fail");
        assert!(zero.message.contains("at least 1 row"));

        let oversized = normalize_export_window(
            &config,
            &ExportQuery {
                window: Some(config.export.max_window + 1),
                cursor: Some(0),
            },
        )
        .expect_err("oversized window should fail");
        assert!(oversized.message.contains("exceeds maximum"));
    }
}
