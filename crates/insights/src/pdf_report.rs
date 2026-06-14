//! PDF report generator for ApexIntel intelligence reports.
//!
//! Produces structured report data that can be rendered to HTML or directly
//! to PDF. Supports aggregation from multiple insights, dossier data, and
//! custom report compositions.
//!
//! # Sensei-Rams compliance
//! - Clean, functional, no decorative elements
//! - Structured sections with clear hierarchy
//! - Monospace font for technical content
//! - Header with report title, date, and classification label
//! - Footer with page numbers

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::dossier::{CompanyDossier, PoiDossier};
use crate::InsightSeverity;

// ────────────────────────────────────────────
// Report types
// ────────────────────────────────────────────

/// The type of intelligence report being generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportType {
    /// Aggregated summary of multiple insights.
    InsightSummary,
    /// Comprehensive dossier on a single entity.
    EntityDossier,
    /// Multi-entity competitive analysis.
    CompetitiveAnalysis,
    /// Periodic intelligence digest.
    WeeklyDigest,
}

impl ReportType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InsightSummary => "Insight Summary",
            Self::EntityDossier => "Entity Dossier",
            Self::CompetitiveAnalysis => "Competitive Analysis",
            Self::WeeklyDigest => "Weekly Intelligence Digest",
        }
    }

    pub fn classification(&self) -> &'static str {
        match self {
            Self::InsightSummary => "CONFIDENTIAL",
            Self::EntityDossier => "CONFIDENTIAL",
            Self::CompetitiveAnalysis => "SENSITIVE",
            Self::WeeklyDigest => "CONFIDENTIAL",
        }
    }
}

// ────────────────────────────────────────────
// Report metadata
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportMetadata {
    /// Unique report identifier.
    pub report_id: String,
    /// Version of the report schema.
    pub schema_version: String,
    /// Source system identifier.
    pub source: String,
    /// Entity ID this report is about (if single-entity).
    pub entity_id: Option<String>,
    /// Entity name (if single-entity).
    pub entity_name: Option<String>,
    /// Custom tags for categorization.
    pub tags: Vec<String>,
    /// How many sources were consulted.
    pub source_count: usize,
    /// Aggregate confidence across all evidence.
    pub aggregate_confidence: f64,
}

impl Default for ReportMetadata {
    fn default() -> Self {
        Self {
            report_id: Uuid::new_v4().to_string(),
            schema_version: env!("CARGO_PKG_VERSION").to_string(),
            source: "ApexIntel".to_string(),
            entity_id: None,
            entity_name: None,
            tags: Vec::new(),
            source_count: 0,
            aggregate_confidence: 0.0,
        }
    }
}

// ────────────────────────────────────────────
// Evidence and sources
// ────────────────────────────────────────────

/// A piece of evidence supporting a report section.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    /// Human-readable label for this evidence.
    pub label: String,
    /// The evidence value or content.
    pub value: String,
    /// Confidence score (0.0 – 1.0).
    pub confidence: f64,
}

impl EvidenceItem {
    pub fn new(label: &str, value: &str) -> Self {
        Self {
            label: label.to_string(),
            value: value.to_string(),
            confidence: 0.5,
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }
}

/// A source reference for evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRef {
    /// Source title or page name.
    pub title: String,
    /// Source URL.
    pub url: String,
    /// Source domain (extracted for grouping).
    pub domain: String,
    /// When the source was observed.
    pub observed_at: Option<DateTime<Utc>>,
}

// ────────────────────────────────────────────
// Report section
// ────────────────────────────────────────────

/// A section within a PDF report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSection {
    /// Section heading.
    pub heading: String,
    /// Optional severity badge for the section.
    pub severity: Option<InsightSeverity>,
    /// Section body text (Markdown-like formatting).
    pub body: String,
    /// Supporting evidence items.
    pub evidence_items: Vec<EvidenceItem>,
    /// Source references.
    pub sources: Vec<SourceRef>,
}

impl ReportSection {
    pub fn new(heading: &str) -> Self {
        Self {
            heading: heading.to_string(),
            severity: None,
            body: String::new(),
            evidence_items: Vec::new(),
            sources: Vec::new(),
        }
    }

    pub fn with_body(mut self, body: &str) -> Self {
        self.body = body.to_string();
        self
    }

    pub fn with_severity(mut self, severity: InsightSeverity) -> Self {
        self.severity = Some(severity);
        self
    }

    pub fn add_evidence(&mut self, item: EvidenceItem) {
        self.evidence_items.push(item);
    }

    pub fn add_source(&mut self, source: SourceRef) {
        self.sources.push(source);
    }
}

// ────────────────────────────────────────────
// Page size configuration
// ────────────────────────────────────────────

/// Supported PDF page sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSize {
    A4,
    Letter,
}

impl PageSize {
    pub fn dimensions_mm(&self) -> (f64, f64) {
        match self {
            Self::A4 => (210.0, 297.0),
            Self::Letter => (215.9, 279.4),
        }
    }
}

// ────────────────────────────────────────────
// PDF Report
// ────────────────────────────────────────────

/// A complete PDF report combining multiple insights and evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfReport {
    pub title: String,
    pub subtitle: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub report_type: ReportType,
    pub sections: Vec<ReportSection>,
    pub metadata: ReportMetadata,
    pub page_size: PageSize,
}

impl PdfReport {
    /// Create a new empty report with the given title and type.
    pub fn new(title: &str, report_type: ReportType) -> Self {
        Self {
            title: title.to_string(),
            subtitle: None,
            generated_at: Utc::now(),
            report_type,
            sections: Vec::new(),
            metadata: ReportMetadata::default(),
            page_size: PageSize::A4,
        }
    }

    /// Set the report subtitle.
    pub fn with_subtitle(mut self, subtitle: &str) -> Self {
        self.subtitle = Some(subtitle.to_string());
        self
    }

    /// Set the page size.
    pub fn with_page_size(mut self, size: PageSize) -> Self {
        self.page_size = size;
        self
    }

    /// Add a section to the report.
    pub fn add_section(&mut self, section: ReportSection) {
        self.sections.push(section);
    }

    /// Build a `PdfReport` from a slice of insight data.
    ///
    /// Aggregates multiple insights into a structured report with sections
    /// grouped by insight type/severity, collecting evidence URLs and sources.
    pub fn from_insights(title: &str, insights: &[InsightReportRow]) -> Self {
        let mut report = Self::new(title, ReportType::InsightSummary);
        let mut all_sources = Vec::new();
        let mut total_confidence = 0.0;

        // Group insights by severity for ordered presentation
        let mut by_severity: HashMap<u8, Vec<&InsightReportRow>> = HashMap::new();
        for insight in insights {
            let severity = insight.severity.as_str();
            let priority = match severity {
                "critical" => 5,
                "high" => 4,
                "medium" => 3,
                "low" => 2,
                _ => 1,
            };
            by_severity.entry(priority).or_default().push(insight);
        }

        // Process in order of severity (highest first)
        let mut severity_keys: Vec<u8> = by_severity.keys().copied().collect();
        severity_keys.sort_by(|a, b| b.cmp(a));

        for &sev_key in &severity_keys {
            let group = &by_severity[&sev_key];
            for insight in group {
                let _severity_label = insight.severity.as_str();
                let mut section = ReportSection::new(&insight.title)
                    .with_body(&insight.summary)
                    .with_severity(insight.severity);

                // Add evidence items from the insight's metadata
                for ev in &insight.evidence {
                    section.add_evidence(EvidenceItem::new(&ev.label, &ev.value)
                        .with_confidence(ev.confidence));
                }

                // Add source references
                for src in &insight.sources {
                    let domain = src.url.split('/').nth(2)
                        .unwrap_or(&src.url)
                        .to_string();
                    let domain_clone = domain.clone();
                    section.add_source(SourceRef {
                        title: src.title.clone(),
                        url: src.url.clone(),
                        domain,
                        observed_at: insight.generated_at,
                    });
                    all_sources.push(domain_clone);
                }

                total_confidence += insight.confidence;
                report.add_section(section);
            }
        }

        // Deduplicate sources and count
        all_sources.sort();
        all_sources.dedup();
        let source_count = all_sources.len();
        let aggregate_confidence = if insights.is_empty() {
            0.0
        } else {
            total_confidence / insights.len() as f64
        };

        report.metadata.source_count = source_count;
        report.metadata.aggregate_confidence = aggregate_confidence;
        report.metadata.tags = insights.iter()
            .flat_map(|i| i.tags.iter().cloned())
            .collect();

        report
    }

    /// Build a `PdfReport` from a company dossier.
    pub fn from_company_dossier(title: &str, dossier: &CompanyDossier) -> Self {
        let mut report = Self::new(title, ReportType::EntityDossier);
        report.metadata.entity_id = Some(dossier.company_id.to_string());
        report.metadata.entity_name = Some(dossier.company_name.clone());

        // Profile section
        let profile = &dossier.profile_section;
        let mut profile_section = ReportSection::new("Company Profile");
        profile_section.body = format!(
            "Type: {}\nCountry: {}\nRegion: {}\nIndustry: {}\nEmployees: {}\nRevenue: {}",
            profile.company_type,
            profile.country,
            profile.region,
            profile.industry_tags.join(", "),
            profile.employee_estimate.map_or("Unknown".to_string(), |e| e.to_string()),
            profile.revenue_estimate_usd.map_or("Unknown".to_string(), |r| format!("${}", r)),
        );
        report.add_section(profile_section);

        // Capability assessment
        let cap = &dossier.capability_assessment;
        let mut cap_section = ReportSection::new("Capability Assessment");
        cap_section.body = format!(
            "Total capabilities: {}\nGrade A: {} | Grade B: {} | Grade C: {} | Grade D: {}\nCoverage score: {:.2}",
            cap.total_capabilities,
            cap.grade_a_count, cap.grade_b_count, cap.grade_c_count, cap.grade_d_count,
            cap.coverage_score,
        );
        for tc in &cap.top_capabilities {
            cap_section.add_evidence(EvidenceItem::new(
                &tc.capability,
                &format!("Grade: {} | Evidence: {}", tc.proof_grade, tc.evidence_count),
            ));
        }
        report.add_section(cap_section);

        // Certification analysis
        let cert = &dossier.certification_analysis;
        let mut cert_section = ReportSection::new("Certification Analysis");
        cert_section.body = format!(
            "Total: {} | Active: {} | Expired: {} | Pending: {}\nHealth score: {:.2}",
            cert.total, cert.active, cert.expired, cert.pending, cert.cert_health_score,
        );
        if !cert.gaps.is_empty() {
            cert_section.body.push_str("\nGaps:\n");
            for gap in &cert.gaps {
                cert_section.body.push_str(&format!("- {}\n", gap));
            }
        }
        report.add_section(cert_section);

        // Risk assessment
        let risk = &dossier.risk_assessment;
        let mut risk_section = ReportSection::new("Risk Assessment")
            .with_severity(if risk.overall_risk >= 0.7 {
                crate::InsightSeverity::Critical
            } else if risk.overall_risk >= 0.4 {
                crate::InsightSeverity::High
            } else {
                crate::InsightSeverity::Medium
            });
        risk_section.body = format!(
            "Overall risk: {:.2} ({})\nThreat score: {:.2}",
            risk.overall_risk, risk.risk_label, risk.threat_score,
        );
        for factor in &risk.factors {
            risk_section.add_evidence(EvidenceItem::new(
                &factor.factor,
                &format!("[{}] {}", factor.severity, factor.description),
            ));
        }
        report.add_section(risk_section);

        // Opportunity analysis
        let opp = &dossier.opportunity_analysis;
        let mut opp_section = ReportSection::new("Opportunity Analysis");
        opp_section.body = format!(
            "Strategic relevance: {:.2}\nOverlap: {:.2}\nRecommendation: {}",
            opp.strategic_relevance, opp.overlap_score, opp.approach_recommendation,
        );
        for op in &opp.opportunities {
            opp_section.add_evidence(EvidenceItem::new("Opportunity", op));
        }
        report.add_section(opp_section);

        // Competitive position
        let comp = &dossier.competitive_position;
        let mut comp_section = ReportSection::new("Competitive Position");
        comp_section.body = format!("Position: {}", comp.position_label);
        for s in &comp.strengths {
            comp_section.add_evidence(EvidenceItem::new("Strength", s));
        }
        for w in &comp.weaknesses {
            comp_section.add_evidence(EvidenceItem::new("Weakness", w));
        }
        report.add_section(comp_section);

        report
    }

    /// Build a `PdfReport` from a POI dossier.
    pub fn from_poi_dossier(title: &str, dossier: &PoiDossier) -> Self {
        let mut report = Self::new(title, ReportType::EntityDossier);
        report.metadata.entity_id = Some(dossier.person_id.to_string());
        report.metadata.entity_name = Some(dossier.person_name.clone());

        // Professional profile
        let prof = &dossier.professional_profile;
        let mut prof_section = ReportSection::new("Professional Profile");
        prof_section.body = format!(
            "Name: {}\nRole: {}\nRole family: {}\nOrganization: {}\nCountry: {}\nRegion: {}",
            prof.name,
            prof.current_role.as_deref().unwrap_or("Unknown"),
            prof.role_family,
            prof.organization.as_deref().unwrap_or("Unknown"),
            prof.country.as_deref().unwrap_or("Unknown"),
            prof.region.as_deref().unwrap_or("Unknown"),
        );
        if let Some(bio) = &prof.bio_summary {
            prof_section.body.push_str(&format!("\nBio: {}", bio));
        }
        if let Some(ds) = &prof.decision_style {
            prof_section.add_evidence(EvidenceItem::new("Decision Style", ds));
        }
        report.add_section(prof_section);

        // Priority analysis
        let pa = &dossier.priority_analysis;
        let mut pa_section = ReportSection::new("Priority Vector Analysis");
        pa_section.body = format!(
            "Dominant: {}\nCost: {:.2} | Quality: {:.2} | Speed: {:.2}\nResilience: {:.2} | Compliance: {:.2} | Security: {:.2}\nInterpretation: {}",
            pa.dominant_priority,
            pa.cost, pa.quality, pa.speed,
            pa.resilience, pa.compliance, pa.security,
            pa.interpretation,
        );
        report.add_section(pa_section);

        // Influence assessment
        let inf = &dossier.influence_assessment;
        let mut inf_section = ReportSection::new("Influence Assessment");
        inf_section.body = format!(
            "Score: {:.2} ({})\nPain index: {:.2}\nRole drift: {:.2}\nChange risk: {:.2}",
            inf.influence_score, inf.influence_label,
            inf.pain_index, inf.role_drift, inf.change_risk,
        );
        for topic in &inf.trigger_topics {
            inf_section.add_evidence(EvidenceItem::new("Trigger Topic", topic));
        }
        report.add_section(inf_section);

        // Artifact summary
        let art = &dossier.artifact_summary;
        let mut art_section = ReportSection::new("Artifact Summary");
        art_section.body = format!("Total artifacts: {}", art.total_artifacts);
        for (atype, count) in &art.by_type {
            art_section.add_evidence(EvidenceItem::new(atype, &count.to_string()));
        }
        for hl in &art.recent_highlights {
            art_section.add_source(SourceRef {
                title: hl.title.clone().unwrap_or_default(),
                url: hl.url.clone(),
                domain: hl.url.split('/').nth(2).unwrap_or(&hl.url).to_string(),
                observed_at: Some(hl.date),
            });
        }
        report.add_section(art_section);

        // Approach guidance
        let ag = &dossier.approach_guidance;
        let mut ag_section = ReportSection::new("Approach Guidance");
        ag_section.body = format!(
            "Channel: {}\nTiming: {}\nPriority: {}",
            ag.best_channel, ag.timing_recommendation, ag.engagement_priority,
        );
        for tp in &ag.talking_points {
            ag_section.add_evidence(EvidenceItem::new("Talking Point", tp));
        }
        for av in &ag.avoid_topics {
            ag_section.add_evidence(EvidenceItem::new("Avoid Topic", av));
        }
        report.add_section(ag_section);

        report
    }

    /// Render the report to a self-contained HTML string.
    ///
    /// The HTML includes embedded CSS and is suitable for:
    /// - Preview in browser
    /// - Conversion to PDF via printpdf
    /// - Email rendering
    pub fn to_html(&self) -> String {
        let mut html = String::with_capacity(4096);

        // CSS - inline, self-contained
        html.push_str(
            "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<style>\n\
  @page { margin: 20mm 15mm; }\n\
  * { margin: 0; padding: 0; box-sizing: border-box; }\n\
  body {\n\
    font-family: \"SF Mono\", \"JetBrains Mono\", \"Fira Code\", \"Cascadia Code\", \"Consolas\", monospace;\n\
    font-size: 9pt; line-height: 1.5; color: #1a1a1a; background: #fff;\n\
    max-width: 180mm; margin: 0 auto; padding: 20mm 15mm;\n\
  }\n\
  .report-header { border-bottom: 2px solid #1a1a1a; padding-bottom: 8mm; margin-bottom: 8mm; }\n\
  .report-header h1 { font-size: 16pt; font-weight: 700; letter-spacing: -0.02em; }\n\
  .report-header .subtitle { font-size: 10pt; color: #555; margin-top: 2mm; }\n\
  .report-header .meta { font-size: 7pt; text-transform: uppercase; letter-spacing: 0.1em; color: #888; margin-top: 3mm; display: flex; justify-content: space-between; }\n\
  .report-header .classification { font-size: 7pt; font-weight: 700; letter-spacing: 0.15em; color: #c0392b; margin-top: 2mm; }\n\
  .section { margin-bottom: 6mm; page-break-inside: avoid; }\n\
  .section h2 { font-size: 11pt; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em; border-bottom: 1px solid #ccc; padding-bottom: 1mm; margin-bottom: 3mm; }\n\
  .section .severity-badge { display: inline-block; font-size: 6pt; font-weight: 700; text-transform: uppercase; letter-spacing: 0.1em; padding: 1mm 2mm; border: 1px solid #1a1a1a; margin-left: 2mm; }\n\
  .section .body { white-space: pre-wrap; font-size: 8.5pt; margin-bottom: 3mm; }\n\
  .evidence-table { width: 100%; border-collapse: collapse; margin: 2mm 0; font-size: 8pt; }\n\
  .evidence-table th { text-align: left; font-weight: 700; text-transform: uppercase; font-size: 6.5pt; letter-spacing: 0.08em; border-bottom: 1px solid #ccc; padding: 1mm 0; }\n\
  .evidence-table td { padding: 0.8mm 0; border-bottom: 1px solid #eee; vertical-align: top; }\n\
  .evidence-table .conf { text-align: right; font-variant-numeric: tabular-nums; }\n\
  .sources { margin-top: 3mm; }\n\
  .sources h3 { font-size: 7pt; font-weight: 700; text-transform: uppercase; letter-spacing: 0.1em; margin-bottom: 1mm; }\n\
  .sources ul { list-style: none; padding: 0; }\n\
  .sources li { font-size: 7pt; padding: 0.3mm 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }\n\
  .sources li a { color: #2c6b9e; text-decoration: none; }\n\
  .report-footer { margin-top: 15mm; padding-top: 3mm; border-top: 1px solid #ccc; font-size: 7pt; color: #888; display: flex; justify-content: space-between; }\n\
  .toc { margin-bottom: 8mm; }\n\
  .toc h2 { font-size: 11pt; font-weight: 700; text-transform: uppercase; margin-bottom: 3mm; }\n\
  .toc ul { list-style: none; padding: 0; }\n\
  .toc li { font-size: 8.5pt; padding: 0.5mm 0; border-bottom: 1px dotted #ddd; }\n\
  @media print { body { padding: 0; max-width: none; } .section { page-break-inside: avoid; } }\n\
</style>\n\
</head>\n\
<body>\n"
        );

        // Header
        html.push_str(r#"<div class="report-header">"#);
        html.push_str(&format!("<h1>{}</h1>", Self::html_escape(&self.title)));
        if let Some(sub) = &self.subtitle {
            html.push_str(&format!(
                r#"<div class="subtitle">{}</div>"#,
                Self::html_escape(sub)
            ));
        }
        html.push_str(&format!(
            r#"<div class="meta">
  <span>ApexIntel Intelligence Report</span>
  <span>{}</span>
</div>"#,
            self.generated_at.format("%Y-%m-%d %H:%M UTC"),
        ));
        html.push_str(&format!(
            r#"<div class="classification">{} · {}</div>"#,
            self.report_type.classification(),
            self.report_type.as_str(),
        ));
        html.push_str("</div>");

        // Table of contents (for multi-section reports)
        if self.sections.len() > 1 {
            html.push_str(r#"<div class="toc"><h2>Contents</h2><ul>"#);
            for (i, section) in self.sections.iter().enumerate() {
                html.push_str(&format!(
                    r#"<li>{}. {}<span class="page"></span></li>"#,
                    i + 1,
                    Self::html_escape(&section.heading),
                ));
            }
            html.push_str("</ul></div>");
        }

        // Sections
        for (i, section) in self.sections.iter().enumerate() {
            html.push_str(&format!(
                r#"<div class="section"><h2>{}. {}"#,
                i + 1,
                Self::html_escape(&section.heading),
            ));
            if let Some(sev) = &section.severity {
                html.push_str(&format!(
                    r#"<span class="severity-badge">{}</span>"#,
                    sev.as_str(),
                ));
            }
            html.push_str("</h2>");

            if !section.body.is_empty() {
                html.push_str(&format!(
                    r#"<div class="body">{}</div>"#,
                    Self::html_escape(&section.body),
                ));
            }

            // Evidence table
            if !section.evidence_items.is_empty() {
                html.push_str(
                    r#"<table class="evidence-table"><thead><tr><th>Evidence</th><th>Value</th><th class="conf">Conf</th></tr></thead><tbody>"#,
                );
                for ev in &section.evidence_items {
                    html.push_str(&format!(
                        r#"<tr><td>{}</td><td>{}</td><td class="conf">{:.0}%</td></tr>"#,
                        Self::html_escape(&ev.label),
                        Self::html_escape(&ev.value),
                        ev.confidence * 100.0,
                    ));
                }
                html.push_str("</tbody></table>");
            }

            // Sources
            if !section.sources.is_empty() {
                html.push_str(r#"<div class="sources"><h3>Sources</h3><ul>"#);
                for src in &section.sources {
                    html.push_str(&format!(
                        r#"<li><a href="{}">{}</a> · {}</li>"#,
                        Self::html_escape(&src.url),
                        Self::html_escape(&src.title),
                        Self::html_escape(&src.domain),
                    ));
                }
                html.push_str("</ul></div>");
            }

            html.push_str("</div>");
        }

        // Footer
        let classification = self.report_type.classification();
        html.push_str(&format!(
            r#"<div class="report-footer">
  <span>ApexIntel Intelligence Report</span>
  <span>Page 1 / 1</span>
  <span>{}</span>
</div>"#,
            classification,
        ));

        html.push_str("</body></html>");
        html
    }

    /// Escape HTML special characters.
    fn html_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}

// ────────────────────────────────────────────
// Input types for building reports from insights
// ────────────────────────────────────────────

/// A lightweight insight record used to build reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightReportRow {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub severity: InsightSeverity,
    pub confidence: f64,
    pub insight_type: String,
    pub region: Option<String>,
    pub evidence: Vec<EvidenceItem>,
    pub sources: Vec<ReportSourceRef>,
    pub tags: Vec<String>,
    pub generated_at: Option<DateTime<Utc>>,
}

/// A source reference for report-building.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSourceRef {
    pub title: String,
    pub url: String,
}

// ────────────────────────────────────────────
// PDF export configuration (shared between crates)
// ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PdfExportConfig {
    pub output_dir: std::path::PathBuf,
    pub max_pages: usize,
    pub page_size: PageSize,
}

impl Default for PdfExportConfig {
    fn default() -> Self {
        Self {
            output_dir: std::path::PathBuf::from("/tmp/apex_pdf_output"),
            max_pages: 50,
            page_size: PageSize::A4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_insight_row(id: &str, title: &str, severity: &str, confidence: f64) -> InsightReportRow {
        let sev = match severity {
            "critical" => InsightSeverity::Critical,
            "high" => InsightSeverity::High,
            "medium" => InsightSeverity::Medium,
            "low" => InsightSeverity::Low,
            _ => InsightSeverity::Info,
        };
        InsightReportRow {
            id: id.to_string(),
            title: title.to_string(),
            summary: format!("Summary for {}", title),
            severity: sev,
            confidence,
            insight_type: "supply_chain".to_string(),
            region: Some("EU".to_string()),
            evidence: vec![
                EvidenceItem::new("source_a", "Observed increase in procurement activity")
                    .with_confidence(0.85),
                EvidenceItem::new("source_b", "New supplier relationship established")
                    .with_confidence(0.72),
            ],
            sources: vec![
                ReportSourceRef {
                    title: "Industry_Report_Q1".to_string(),
                    url: "https://example.com/report".to_string(),
                },
            ],
            tags: vec!["supply_chain".to_string(), "procurement".to_string()],
            generated_at: Some(Utc::now()),
        }
    }

    #[test]
    fn test_report_new() {
        let report = PdfReport::new("Test Report", ReportType::InsightSummary);
        assert_eq!(report.title, "Test Report");
        assert_eq!(report.report_type, ReportType::InsightSummary);
        assert!(report.sections.is_empty());
    }

    #[test]
    fn test_report_from_insights() {
        let insights = vec![
            sample_insight_row("1", "Critical Alert", "critical", 0.95),
            sample_insight_row("2", "Medium Intel", "medium", 0.65),
            sample_insight_row("3", "Info Note", "info", 0.30),
        ];
        let report = PdfReport::from_insights("Intel Summary", &insights);

        assert_eq!(report.title, "Intel Summary");
        assert_eq!(report.sections.len(), 3);
        assert_eq!(report.metadata.source_count, 1);
        assert!(report.metadata.aggregate_confidence > 0.0);
        assert_eq!(report.report_type, ReportType::InsightSummary);
    }

    #[test]
    fn test_report_from_insights_empty() {
        let report = PdfReport::from_insights("Empty Report", &[]);
        assert!(report.sections.is_empty());
        assert_eq!(report.metadata.aggregate_confidence, 0.0);
        assert_eq!(report.metadata.source_count, 0);
    }

    #[test]
    fn test_report_add_section() {
        let mut report = PdfReport::new("Section Test", ReportType::CompetitiveAnalysis);
        let section = ReportSection::new("Findings")
            .with_body("Key findings here")
            .with_severity(InsightSeverity::High);
        report.add_section(section);
        assert_eq!(report.sections.len(), 1);
        assert_eq!(report.sections[0].heading, "Findings");
        assert_eq!(report.sections[0].body, "Key findings here");
    }

    #[test]
    fn test_section_evidence_and_sources() {
        let mut section = ReportSection::new("Test");
        section.add_evidence(EvidenceItem::new("Item 1", "Value 1"));
        section.add_source(SourceRef {
            title: "Source".to_string(),
            url: "https://example.com".to_string(),
            domain: "example.com".to_string(),
            observed_at: None,
        });
        assert_eq!(section.evidence_items.len(), 1);
        assert_eq!(section.sources.len(), 1);
    }

    #[test]
    fn test_to_html_basic() {
        let mut report = PdfReport::new("HTML Test", ReportType::InsightSummary);
        let section = ReportSection::new("Test Section")
            .with_body("This is a test body")
            .with_severity(InsightSeverity::High);
        report.add_section(section);

        let html = report.to_html();
        assert!(html.contains("HTML Test"));
        assert!(html.contains("Test Section"));
        assert!(html.contains("This is a test body"));
        assert!(html.contains("ApexIntel Intelligence Report"));
        assert!(html.contains("CONFIDENTIAL"));
        assert!(html.contains("</html>"));
    }

    #[test]
    fn test_to_html_with_evidence() {
        let mut report = PdfReport::new("Evidence Test", ReportType::InsightSummary);
        let mut section = ReportSection::new("Section with Evidence")
            .with_body("Body text");
        section.add_evidence(EvidenceItem::new("Detection", "Signal detected")
            .with_confidence(0.92));
        section.add_source(SourceRef {
            title: "Example Source".to_string(),
            url: "https://example.com/src".to_string(),
            domain: "example.com".to_string(),
            observed_at: None,
        });
        report.add_section(section);

        let html = report.to_html();
        assert!(html.contains("Detection"));
        assert!(html.contains("Signal detected"));
        assert!(html.contains("92%"));
        assert!(html.contains("example.com"));
    }

    #[test]
    fn test_to_html_html_escaping() {
        let mut report = PdfReport::new("Escaping <test>", ReportType::InsightSummary);
        let section = ReportSection::new("Section & Special")
            .with_body("Body with <script>alert('xss')</script>");
        report.add_section(section);

        let html = report.to_html();
        assert!(html.contains("&lt;test&gt;"), "title should be HTML-escaped");
        assert!(html.contains("&amp;"));
        assert!(html.contains("&lt;script&gt;"), "body script tag should be HTML-escaped");
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn test_page_size_dimensions() {
        assert_eq!(PageSize::A4.dimensions_mm(), (210.0, 297.0));
        assert_eq!(PageSize::Letter.dimensions_mm(), (215.9, 279.4));
    }

    #[test]
    fn test_report_type_display() {
        assert_eq!(ReportType::InsightSummary.as_str(), "Insight Summary");
        assert_eq!(ReportType::EntityDossier.as_str(), "Entity Dossier");
        assert_eq!(ReportType::CompetitiveAnalysis.as_str(), "Competitive Analysis");
        assert_eq!(ReportType::WeeklyDigest.as_str(), "Weekly Intelligence Digest");
    }

    #[test]
    fn test_report_type_classification() {
        assert_eq!(ReportType::InsightSummary.classification(), "CONFIDENTIAL");
        assert_eq!(ReportType::CompetitiveAnalysis.classification(), "SENSITIVE");
    }

    #[test]
    fn test_evidence_item_confidence_clamping() {
        let item = EvidenceItem::new("Test", "Value").with_confidence(1.5);
        assert!((item.confidence - 1.0).abs() < f64::EPSILON);

        let item = EvidenceItem::new("Test", "Value").with_confidence(-0.5);
        assert!((item.confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_report_metadata_defaults() {
        let meta = ReportMetadata::default();
        assert_eq!(meta.source, "ApexIntel");
        assert!(!meta.report_id.is_empty());
    }

    #[test]
    fn test_pdf_export_config_default() {
        let config = PdfExportConfig::default();
        assert_eq!(config.max_pages, 50);
        assert_eq!(config.page_size, PageSize::A4);
    }

    #[test]
    fn test_from_insights_aggregates_tags() {
        let mut i1 = sample_insight_row("1", "A", "high", 0.8);
        i1.tags = vec!["supply_chain".to_string()];
        let mut i2 = sample_insight_row("2", "B", "medium", 0.6);
        i2.tags = vec!["logistics".to_string()];
        let report = PdfReport::from_insights("Test", &[i1, i2]);
        assert!(report.metadata.tags.contains(&"supply_chain".to_string()));
        assert!(report.metadata.tags.contains(&"logistics".to_string()));
    }
}
