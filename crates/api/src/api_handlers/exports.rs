use super::super::*;

fn csv_escape(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub(crate) async fn export_companies_csv(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> axum::response::Response {
    let filters = CompanyListFilters::default();
    let rows = state
        .store
        .list_companies(&filters, None, true, 10_000, 0)
        .await
        .unwrap_or_default();
    let row_count = rows.len() as i64;

    let mut csv = String::from(
        "id,name,domain,region,country,entity_type,is_competitor,threat_score,capabilities,updated_at\n",
    );
    for row in rows {
        let item = company_row_to_item(row);
        csv.push_str(&format!(
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

    let _ = state
        .store
        .record_export_history(
            &auth_ctx.user_id,
            "companies",
            "csv",
            &serde_json::json!({}),
            row_count,
            Some("companies.csv"),
        )
        .await;
    let _ = state
        .store
        .record_audit_event(
            &auth_ctx.user_id,
            "companies_exported",
            &serde_json::json!({
                "format": "csv",
                "row_count": row_count,
                "download_name": "companies.csv"
            }),
        )
        .await;

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"companies.csv\"",
        )
        .body(axum::body::Body::from(csv))
        .unwrap()
}

pub(crate) async fn export_persons_csv(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> axum::response::Response {
    let filters = PersonListFilters::default();
    let rows = state
        .store
        .list_persons(&filters, None, true, 10_000, 0)
        .await
        .unwrap_or_default();
    let row_count = rows.len() as i64;

    let mut csv = String::from(
        "id,name,role,organization,region,priority_score,engagement_status,updated_at\n",
    );
    for row in rows {
        let item = person_row_to_item(row);
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_escape(&item.id),
            csv_escape(&item.name),
            csv_escape(&item.role),
            csv_escape(&item.organization),
            csv_escape(&item.region),
            format!("{:.4}", item.priority_score),
            csv_escape(&item.engagement_status),
            csv_escape(&item.updated_at.to_rfc3339()),
        ));
    }

    let _ = state
        .store
        .record_export_history(
            &auth_ctx.user_id,
            "persons",
            "csv",
            &serde_json::json!({}),
            row_count,
            Some("persons.csv"),
        )
        .await;
    let _ = state
        .store
        .record_audit_event(
            &auth_ctx.user_id,
            "persons_exported",
            &serde_json::json!({
                "format": "csv",
                "row_count": row_count,
                "download_name": "persons.csv"
            }),
        )
        .await;

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"persons.csv\"",
        )
        .body(axum::body::Body::from(csv))
        .unwrap()
}

pub(crate) async fn export_insights_csv(
    State(state): State<AppState>,
    Extension(auth_ctx): Extension<ApiAuthContext>,
) -> axum::response::Response {
    let filters = InsightListFilters {
        exclude_internal: true,
        ..Default::default()
    };
    let rows = state
        .store
        .list_insights(&filters, 10_000, 0)
        .await
        .unwrap_or_default();
    let row_count = rows.len() as i64;

    let mut csv = String::from("id,title,insight_type,summary,region,confidence,created_at\n");
    for row in rows {
        csv.push_str(&format!(
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

    let _ = state
        .store
        .record_export_history(
            &auth_ctx.user_id,
            "insights",
            "csv",
            &serde_json::json!({"exclude_internal": true}),
            row_count,
            Some("insights.csv"),
        )
        .await;
    let _ = state
        .store
        .record_audit_event(
            &auth_ctx.user_id,
            "insights_exported",
            &serde_json::json!({
                "format": "csv",
                "row_count": row_count,
                "download_name": "insights.csv",
                "exclude_internal": true
            }),
        )
        .await;

    axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"insights.csv\"",
        )
        .body(axum::body::Body::from(csv))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::csv_escape;

    #[test]
    fn test_csv_escape_quotes_and_wraps_fields() {
        assert_eq!(csv_escape("alpha,beta"), "\"alpha,beta\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn test_csv_escape_preserves_empty_string() {
        assert_eq!(csv_escape(""), "\"\"");
    }
}
