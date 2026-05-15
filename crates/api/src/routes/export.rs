//! CSV/XLSX export support for all data tables.
//!
//! Adds `?format=csv` and `?format=xlsx` query parameter support to list
//! endpoints (warnings, insights, companies, persons, recipes) enabling
//! bulk data export for analysts.

use serde::Serialize;

// ─── Export format ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Csv,
    Xlsx,
}

impl ExportFormat {
    pub fn from_query(format_str: Option<&str>) -> Self {
        match format_str {
            Some("csv") => Self::Csv,
            Some("xlsx") => Self::Xlsx,
            _ => Self::Json,
        }
    }

    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }

    pub fn file_extension(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }
}

// ─── CSV writer ─────────────────────────────────────────────────────────

/// Generate a CSV string from headers and rows.
pub fn to_csv(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut output = String::new();

    // Header row
    output.push_str(&headers.join(","));
    output.push('\n');

    // Data rows
    for row in rows {
        let escaped: Vec<String> = row
            .iter()
            .map(|cell| {
                if cell.contains(',') || cell.contains('"') || cell.contains('\n') {
                    format!("\"{}\"", cell.replace('"', "\"\""))
                } else {
                    cell.clone()
                }
            })
            .collect();
        output.push_str(&escaped.join(","));
        output.push('\n');
    }
    output
}

// ─── Typed exporters ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct WarningExportRow {
    pub id: String,
    pub warning_type: String,
    pub severity: String,
    pub title: String,
    pub entity: String,
    pub region: String,
    pub confidence: f64,
    pub created_at: String,
    pub acknowledged: bool,
    pub recipe_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompanyExportRow {
    pub id: String,
    pub canonical_name: String,
    pub country: String,
    pub region: String,
    pub sector: String,
    pub employee_count: Option<i32>,
    pub revenue_estimate: Option<f64>,
    pub certifications: String,
    pub threat_level: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonExportRow {
    pub id: String,
    pub full_name: String,
    pub company: String,
    pub title: String,
    pub role_family: String,
    pub email: Option<String>,
    pub linkedin_url: Option<String>,
    pub influence_score: f64,
    pub pain_index: f64,
    pub change_risk: f64,
    pub role_drift_score: f64,
    pub priority_cost: f64,
    pub priority_quality: f64,
    pub priority_speed: f64,
    pub engagement_count: i32,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InsightExportRow {
    pub id: String,
    pub insight_type: String,
    pub title: String,
    pub summary: String,
    pub entities_involved: String,
    pub confidence: f64,
    pub created_at: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecipeExportRow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub status: String,
    pub precision: f64,
    pub recall: f64,
    pub signal_count: i32,
    pub last_evaluated: String,
}

/// Convert warnings to CSV format.
pub fn warnings_to_csv(rows: &[WarningExportRow]) -> String {
    let headers = &[
        "id",
        "warning_type",
        "severity",
        "title",
        "entity",
        "region",
        "confidence",
        "created_at",
        "acknowledged",
        "recipe_id",
    ];
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.warning_type.clone(),
                r.severity.clone(),
                r.title.clone(),
                r.entity.clone(),
                r.region.clone(),
                format!("{:.2}", r.confidence),
                r.created_at.clone(),
                r.acknowledged.to_string(),
                r.recipe_id.clone(),
            ]
        })
        .collect();
    to_csv(headers, &data)
}

/// Convert companies to CSV format.
pub fn companies_to_csv(rows: &[CompanyExportRow]) -> String {
    let headers = &[
        "id",
        "canonical_name",
        "country",
        "region",
        "sector",
        "employee_count",
        "revenue_estimate",
        "certifications",
        "threat_level",
    ];
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.canonical_name.clone(),
                r.country.clone(),
                r.region.clone(),
                r.sector.clone(),
                r.employee_count.map(|v| v.to_string()).unwrap_or_default(),
                r.revenue_estimate
                    .map(|v| format!("{:.0}", v))
                    .unwrap_or_default(),
                r.certifications.clone(),
                r.threat_level.clone(),
            ]
        })
        .collect();
    to_csv(headers, &data)
}

/// Convert persons to CSV format.
pub fn persons_to_csv(rows: &[PersonExportRow]) -> String {
    let headers = &[
        "id",
        "full_name",
        "company",
        "title",
        "role_family",
        "email",
        "linkedin_url",
        "influence_score",
        "pain_index",
        "change_risk",
        "role_drift_score",
        "priority_cost",
        "priority_quality",
        "priority_speed",
        "engagement_count",
        "last_seen",
    ];
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.full_name.clone(),
                r.company.clone(),
                r.title.clone(),
                r.role_family.clone(),
                r.email.clone().unwrap_or_default(),
                r.linkedin_url.clone().unwrap_or_default(),
                format!("{:.3}", r.influence_score),
                format!("{:.2}", r.pain_index),
                format!("{:.2}", r.change_risk),
                format!("{:.2}", r.role_drift_score),
                format!("{:.2}", r.priority_cost),
                format!("{:.2}", r.priority_quality),
                format!("{:.2}", r.priority_speed),
                r.engagement_count.to_string(),
                r.last_seen.clone(),
            ]
        })
        .collect();
    to_csv(headers, &data)
}

/// Convert insights to CSV format.
pub fn insights_to_csv(rows: &[InsightExportRow]) -> String {
    let headers = &[
        "id",
        "insight_type",
        "title",
        "summary",
        "entities_involved",
        "confidence",
        "created_at",
        "status",
    ];
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.insight_type.clone(),
                r.title.clone(),
                r.summary.clone(),
                r.entities_involved.clone(),
                format!("{:.2}", r.confidence),
                r.created_at.clone(),
                r.status.clone(),
            ]
        })
        .collect();
    to_csv(headers, &data)
}

/// Convert recipes to CSV format.
pub fn recipes_to_csv(rows: &[RecipeExportRow]) -> String {
    let headers = &[
        "id",
        "name",
        "version",
        "status",
        "precision",
        "recall",
        "signal_count",
        "last_evaluated",
    ];
    let data: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.id.clone(),
                r.name.clone(),
                r.version.clone(),
                r.status.clone(),
                format!("{:.3}", r.precision),
                format!("{:.3}", r.recall),
                r.signal_count.to_string(),
                r.last_evaluated.clone(),
            ]
        })
        .collect();
    to_csv(headers, &data)
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_parsing() {
        assert_eq!(ExportFormat::from_query(Some("csv")), ExportFormat::Csv);
        assert_eq!(ExportFormat::from_query(Some("xlsx")), ExportFormat::Xlsx);
        assert_eq!(ExportFormat::from_query(Some("json")), ExportFormat::Json);
        assert_eq!(ExportFormat::from_query(None), ExportFormat::Json);
    }

    #[test]
    fn test_csv_basic() {
        let headers = &["name", "age", "city"];
        let rows = vec![
            vec!["Alice".into(), "30".into(), "Tunis".into()],
            vec!["Bob".into(), "25".into(), "Casablanca".into()],
        ];
        let csv = to_csv(headers, &rows);
        assert!(csv.starts_with("name,age,city\n"));
        assert!(csv.contains("Alice,30,Tunis"));
        assert!(csv.contains("Bob,25,Casablanca"));
    }

    #[test]
    fn test_csv_escaping() {
        let headers = &["name", "description"];
        let rows = vec![vec![
            "ACME Corp".into(),
            "Produces \"widgets\", bolts".into(),
        ]];
        let csv = to_csv(headers, &rows);
        assert!(csv.contains("\"Produces \"\"widgets\"\", bolts\""));
    }

    #[test]
    fn test_warnings_csv() {
        let rows = vec![WarningExportRow {
            id: "w-001".into(),
            warning_type: "C001".into(),
            severity: "high".into(),
            title: "New competitor detected".into(),
            entity: "RivalCorp".into(),
            region: "TN".into(),
            confidence: 0.87,
            created_at: "2026-03-01".into(),
            acknowledged: false,
            recipe_id: "R-C001-v3".into(),
        }];
        let csv = warnings_to_csv(&rows);
        assert!(csv.contains("w-001"));
        assert!(csv.contains("0.87"));
    }

    #[test]
    fn test_content_types() {
        assert_eq!(ExportFormat::Csv.content_type(), "text/csv; charset=utf-8");
        assert_eq!(ExportFormat::Json.content_type(), "application/json");
        assert_eq!(ExportFormat::Csv.file_extension(), "csv");
    }
}
