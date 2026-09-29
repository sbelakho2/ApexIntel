use crate::*;

pub(crate) fn warning_row_to_response(row: WarningRow) -> WarningResponse {
    let ts = row.ts_utc;
    WarningResponse {
        id: row.id.to_string(),
        title: row.title,
        description: row.description.unwrap_or_default(),
        severity: row.severity,
        warning_type: row.warning_type,
        region: row.region.unwrap_or_default(),
        source_urls: row.source_urls.unwrap_or_default(),
        entity_ids: row
            .entity_ids
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        recipe_code: row.recipe_code,
        confidence: clamp_ratio(row.confidence.unwrap_or(0.0)),
        calibrated_probability: None,
        bayesian_interpretation: None,
        confidence_interval: None,
        evidence_quality_label: None,
        information_gain_bits: None,
        acknowledged: row.acknowledged,
        acknowledged_by: row.acknowledged_by,
        acknowledged_at: row.acknowledged_at,
        acknowledged_note: row.acknowledged_note,
        review_outcome: None,
        reviewed_by: None,
        reviewed_at: None,
        deleted_at: row.deleted_at,
        ts_utc: ts,
        created_at: row.created_at.unwrap_or(ts),
        updated_at: row.updated_at.unwrap_or(ts),
    }
}

pub(crate) fn insight_row_to_response(row: InsightRow) -> InsightResponse {
    InsightResponse {
        id: row.id.to_string(),
        title: row.title,
        summary: row.summary,
        insight_type: row.insight_type.unwrap_or_else(|| "general".to_string()),
        region: row.region.unwrap_or_default(),
        confidence: clamp_ratio(row.confidence.unwrap_or(0.0)),
        evidence_urls: row.evidence_urls.unwrap_or_default(),
        entity_ids: row
            .entity_ids
            .unwrap_or_default()
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        tags: row.tags.unwrap_or_default(),
        information_gain_bits: None,
        information_gain_sparkline: vec![],
        diversity_score: None,
        diversity_label: None,
        causal_flag: None,
        created_at: row.created_at,
        updated_at: row.updated_at,
        bookmarked: None,
        quality_score: None,
    }
}

pub(crate) fn company_row_to_item(row: CompanyRow) -> CompanyListItem {
    let is_competitor = row
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("is_competitor"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    CompanyListItem {
        id: row.id.to_string(),
        name: row.name,
        domain: row.domain,
        region: row.region.unwrap_or_default(),
        country: row.country_code.unwrap_or_default(),
        entity_type: row.company_type.unwrap_or_else(|| "unknown".to_string()),
        is_competitor,
        threat_score: row.threat_score.map(clamp_ratio),
        capabilities: row.industry_tags.unwrap_or_default(),
        community_badges: vec![],
        source_entropy: None,
        updated_at: row.updated_at,
    }
}

pub(crate) fn person_row_to_item(row: PersonListRow) -> PersonListItem {
    // One canonical view (person_intelligence): priority from the stored
    // vector, influence measured separately. The previous mapping labelled the
    // priority score as influence and vice versa.
    let view = apex_api::person_intelligence::PersonIntelligenceView::from_measurements(
        row.priority_vector.as_ref(),
        row.influence,
        row.engagement_status.as_deref(),
        None,
        &[],
    );

    PersonListItem {
        id: row.id.to_string(),
        name: row.name,
        role: row.role,
        role_family: row.role_family.clone(),
        organization: row.organization,
        region: row.region,
        country: row.country,
        priority_score: view.priority_score,
        pain_index: row.pain_index,
        change_risk: row.change_risk,
        role_drift_score: row.role_drift_score,
        influence_score: view.influence_score,
        priority: view.priority_band,
        influence_tier: view.influence_tier_label,
        engagement_status: view.engagement_status,
        tags: vec![row.role_family],
        last_signal: row.updated_at.format("%Y-%m-%d").to_string(),
        updated_at: row.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_row_mapping_preserves_empty_arrays() {
        let now = Utc::now();
        let row = WarningRow {
            id: Uuid::new_v4(),
            recipe_code: None,
            warning_type: "demand_spike".to_string(),
            title: "Test warning".to_string(),
            description: None,
            severity: "high".to_string(),
            region: None,
            source_urls: Some(vec![]),
            entity_ids: Some(vec![]),
            confidence: None,
            impact: None,
            actions: None,
            ts_utc: now,
            acknowledged: false,
            acknowledged_by: None,
            acknowledged_at: None,
            acknowledged_note: None,
            review_outcome: None,
            deleted_at: None,
            created_at: Some(now),
            updated_at: Some(now),
        };

        let mapped = warning_row_to_response(row);
        assert!(mapped.source_urls.is_empty());
        assert!(mapped.entity_ids.is_empty());
    }

    #[test]
    fn insight_row_mapping_preserves_empty_arrays() {
        let now = Utc::now();
        let row = InsightRow {
            id: Uuid::new_v4(),
            title: "Test insight".to_string(),
            summary: "Summary".to_string(),
            insight_type: Some("general".to_string()),
            region: Some("TN".to_string()),
            confidence: Some(0.61),
            evidence_urls: Some(vec![]),
            entity_ids: Some(vec![]),
            tags: Some(vec![]),
            metadata: None,
            created_at: Some(now),
            updated_at: Some(now),
        };

        let mapped = insight_row_to_response(row);
        assert!(mapped.evidence_urls.is_empty());
        assert!(mapped.entity_ids.is_empty());
        assert!(mapped.tags.is_empty());
    }

    fn mapped_row_fixture() -> PersonListRow {
        PersonListRow {
            id: Uuid::new_v4(),
            name: "Fixture".to_string(),
            role: "CTO".to_string(),
            role_family: "Executive".to_string(),
            organization: "Fixture Ltd".to_string(),
            region: "EU".to_string(),
            country: "FI".to_string(),
            priority_vector: None,
            influence: None,
            pain_index: None,
            change_risk: None,
            role_drift_score: None,
            engagement_status: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn person_row_mapping_preserves_expected_priority_fields() {
        let row = PersonListRow {
            id: Uuid::new_v4(),
            name: "Jordan Smith".to_string(),
            role: "CEO".to_string(),
            role_family: "Executive".to_string(),
            organization: "Acme EMS".to_string(),
            region: "US".to_string(),
            country: "US".to_string(),
            priority_vector: Some(serde_json::json!({
                "decision_power": 0.81,
                "domain_relevance": 0.81,
                "network_centrality": 0.81,
                "engagement_potential": 0.81,
                "intelligence_value": 0.81,
            })),
            // Measured influence is a separate quantity from priority.
            influence: Some(0.72),
            pain_index: Some(0.0),
            change_risk: Some(0.0),
            role_drift_score: Some(0.0),
            engagement_status: Some("engaged".to_string()),
            updated_at: Utc::now(),
        };

        let mapped = person_row_to_item(row);
        assert_eq!(mapped.priority.as_deref(), Some("A"));
        assert_eq!(mapped.influence_tier, "high");
        assert_eq!(mapped.influence_score, Some(72));
        assert_eq!(mapped.engagement_status, "engaged");

        // Unmeasured priority and influence are absent, not zero.
        let bare = PersonListRow {
            priority_vector: None,
            influence: None,
            engagement_status: None,
            ..mapped_row_fixture()
        };
        let bare = person_row_to_item(bare);
        assert_eq!(bare.priority_score, None);
        assert_eq!(bare.priority, None);
        assert_eq!(bare.influence_score, None);
        assert_eq!(bare.influence_tier, "not measured");
        assert_eq!(bare.engagement_status, "not measured");
    }
}
