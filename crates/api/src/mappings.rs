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
        created_at: row.created_at.unwrap_or_else(Utc::now),
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
        bookmarked: None,
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
        updated_at: row.updated_at.unwrap_or_else(Utc::now),
    }
}

pub(crate) fn person_row_to_item(row: PersonListRow) -> PersonListItem {
    let score = clamp_ratio(row.priority_score);
    let influence_score = (score * 100.0).round() as i64;
    let priority = if influence_score >= 80 {
        "A"
    } else if influence_score >= 50 {
        "B"
    } else {
        "C"
    }
    .to_string();

    PersonListItem {
        id: row.id.to_string(),
        name: row.name,
        role: row.role,
        role_family: row.role_family.clone(),
        organization: row.organization,
        region: row.region,
        country: row.country,
        priority_score: score,
        influence_score,
        priority,
        influence_tier: priority_tier(score).to_string(),
        engagement_status: row.engagement_status,
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
            created_at: Some(now),
            updated_at: Some(now),
        };

        let mapped = insight_row_to_response(row);
        assert!(mapped.evidence_urls.is_empty());
        assert!(mapped.entity_ids.is_empty());
        assert!(mapped.tags.is_empty());
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
            priority_score: 0.81,
            engagement_status: "engaged".to_string(),
            updated_at: Utc::now(),
        };

        let mapped = person_row_to_item(row);
        assert_eq!(mapped.priority, "A");
        assert_eq!(mapped.influence_tier, "tier_1");
        assert_eq!(mapped.influence_score, 81);
    }
}