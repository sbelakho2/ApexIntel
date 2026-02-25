//! Company and POI dossier generator.
//!
//! Produces structured intelligence dossiers for:
//! - Companies: capabilities, risk posture, supply chain position, cert gaps
//! - POIs: professional profile, priority vector, influence, approach guidance

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use apex_core::entities::{
    CertStatus, Company, Capability, Certification, Person, PoiArtifact, ProofGrade, Site,
};

// ────────────────────────────────────────────
// Company Dossier
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyDossier {
    pub id: Uuid,
    pub company_id: Uuid,
    pub company_name: String,
    pub generated_at: DateTime<Utc>,
    pub profile_section: CompanyProfile,
    pub capability_assessment: CapabilityAssessment,
    pub certification_analysis: CertificationAnalysis,
    pub risk_assessment: RiskAssessment,
    pub opportunity_analysis: OpportunityAnalysis,
    pub site_summaries: Vec<SiteSummary>,
    pub competitive_position: CompetitivePosition,
    pub full_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyProfile {
    pub name: String,
    pub company_type: String,
    pub country: String,
    pub region: String,
    pub industry_tags: Vec<String>,
    pub employee_estimate: Option<i32>,
    pub revenue_estimate_usd: Option<i64>,
    pub strategic_relevance: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityAssessment {
    pub total_capabilities: usize,
    pub grade_a_count: usize,
    pub grade_b_count: usize,
    pub grade_c_count: usize,
    pub grade_d_count: usize,
    pub top_capabilities: Vec<CapabilitySummary>,
    pub coverage_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySummary {
    pub capability: String,
    pub proof_grade: String,
    pub evidence_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificationAnalysis {
    pub total: usize,
    pub active: usize,
    pub expired: usize,
    pub pending: usize,
    pub certifications: Vec<CertSummary>,
    pub gaps: Vec<String>,
    pub cert_health_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertSummary {
    pub standard: String,
    pub status: String,
    pub issuing_body: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub overall_risk: f64,
    pub risk_label: String,
    pub threat_score: f64,
    pub factors: Vec<RiskFactor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub factor: String,
    pub severity: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityAnalysis {
    pub overlap_score: f64,
    pub strategic_relevance: f64,
    pub opportunities: Vec<String>,
    pub approach_recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSummary {
    pub name: String,
    pub site_type: String,
    pub city: Option<String>,
    pub country: Option<String>,
    pub capabilities: Vec<String>,
    pub certifications: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitivePosition {
    pub position_label: String,
    pub strengths: Vec<String>,
    pub weaknesses: Vec<String>,
}

// ────────────────────────────────────────────
// POI Dossier
// ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoiDossier {
    pub id: Uuid,
    pub person_id: Uuid,
    pub person_name: String,
    pub generated_at: DateTime<Utc>,
    pub professional_profile: ProfessionalProfile,
    pub priority_analysis: PriorityAnalysis,
    pub influence_assessment: InfluenceAssessment,
    pub artifact_summary: ArtifactSummarySection,
    pub approach_guidance: ApproachGuidance,
    pub full_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfessionalProfile {
    pub name: String,
    pub current_role: Option<String>,
    pub role_family: String,
    pub organization: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub bio_summary: Option<String>,
    pub decision_style: Option<String>,
    pub risk_tolerance: Option<String>,
    pub change_appetite: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityAnalysis {
    pub dominant_priority: String,
    pub cost: f64,
    pub quality: f64,
    pub speed: f64,
    pub resilience: f64,
    pub compliance: f64,
    pub security: f64,
    pub interpretation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfluenceAssessment {
    pub influence_score: f64,
    pub influence_label: String,
    pub pain_index: f64,
    pub role_drift: f64,
    pub change_risk: f64,
    pub trigger_topics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSummarySection {
    pub total_artifacts: usize,
    pub by_type: HashMap<String, usize>,
    pub recent_highlights: Vec<ArtifactHighlight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactHighlight {
    pub artifact_type: String,
    pub title: Option<String>,
    pub url: String,
    pub date: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproachGuidance {
    pub recommended_proof_type: Option<String>,
    pub talking_points: Vec<String>,
    pub avoid_topics: Vec<String>,
    pub best_channel: String,
    pub timing_recommendation: String,
    pub engagement_priority: String,
}

// ────────────────────────────────────────────
// Company dossier generation
// ────────────────────────────────────────────

/// Build a capability assessment from a list of capabilities.
pub fn assess_capabilities(capabilities: &[Capability]) -> CapabilityAssessment {
    let mut grade_a = 0usize;
    let mut grade_b = 0usize;
    let mut grade_c = 0usize;
    let mut grade_d = 0usize;

    for cap in capabilities {
        match cap.proof_grade {
            ProofGrade::A => grade_a += 1,
            ProofGrade::B => grade_b += 1,
            ProofGrade::C => grade_c += 1,
            ProofGrade::D => grade_d += 1,
        }
    }

    let total = capabilities.len();
    // Coverage score: weighted by proof grade (A=1.0, B=0.75, C=0.5, D=0.25)
    let weighted_sum = grade_a as f64 * 1.0
        + grade_b as f64 * 0.75
        + grade_c as f64 * 0.5
        + grade_d as f64 * 0.25;
    let coverage = if total > 0 {
        weighted_sum / total as f64
    } else {
        0.0
    };

    let top: Vec<CapabilitySummary> = capabilities
        .iter()
        .take(10) // B172: cap top_capabilities to avoid uncontrolled growth
        .map(|c| CapabilitySummary {
            capability: c.capability.clone(),
            proof_grade: c.proof_grade.as_str().to_string(),
            evidence_count: c.evidence_urls.len(),
        })
        .collect();

    CapabilityAssessment {
        total_capabilities: total,
        grade_a_count: grade_a,
        grade_b_count: grade_b,
        grade_c_count: grade_c,
        grade_d_count: grade_d,
        top_capabilities: top,
        coverage_score: coverage,
    }
}

/// Analyze certification health.
pub fn analyze_certifications(
    certs: &[Certification],
    required_standards: &[&str],
) -> CertificationAnalysis {
    let mut active = 0usize;
    let mut expired = 0usize;
    let mut pending = 0usize;

    for cert in certs {
        // Use is_valid() so date-expired Active certs are not counted as healthy.
        // This keeps the health score consistent with gap detection (which also
        // uses is_valid()).
        if cert.is_valid() {
            active += 1;
        } else {
            match cert.status {
                CertStatus::Expired | CertStatus::Suspended => expired += 1,
                CertStatus::Pending => pending += 1,
                CertStatus::Active => expired += 1, // Active status but date-expired
            }
        }
    }

    let cert_summaries: Vec<CertSummary> = certs
        .iter()
        .map(|c| CertSummary {
            standard: c.standard.clone(),
            status: c.status.as_str().to_string(),
            issuing_body: c.issuing_body.clone(),
            valid_until: c.valid_until.map(|d| d.to_string()),
        })
        .collect();

    // Find gaps: required standards not present (or expired)
    let active_standards: Vec<&str> = certs
        .iter()
        .filter(|c| c.is_valid())
        .map(|c| c.standard.as_str())
        .collect();

    let gaps: Vec<String> = required_standards
        .iter()
        .filter(|&&req| !active_standards.iter().any(|s| s.eq_ignore_ascii_case(req)))
        .map(|s| s.to_string())
        .collect();

    // B173: Health score = active_ratio − (0.1 × gap_count), clamped to [0, 1].
    // active_ratio = active / total; missing required standards penalize health.
    let total = certs.len();
    let base_health = if total > 0 {
        active as f64 / total as f64
    } else {
        0.0
    };
    let gap_penalty = gaps.len() as f64 * 0.1;
    let health = (base_health - gap_penalty).clamp(0.0, 1.0);

    CertificationAnalysis {
        total: certs.len(),
        active,
        expired,
        pending,
        certifications: cert_summaries,
        gaps,
        cert_health_score: health,
    }
}

/// Assess risk from company scores.
pub fn assess_risk(company: &Company) -> RiskAssessment {
    let overall = company.risk_score.clamp(0.0, 1.0);
    let label = if overall >= 0.8 {
        "Critical"
    } else if overall >= 0.6 {
        "High"
    } else if overall >= 0.4 {
        "Medium"
    } else if overall >= 0.2 {
        "Low"
    } else {
        "Minimal"
    };

    let mut factors = Vec::new();
    if company.risk_score > 0.5 {
        factors.push(RiskFactor {
            factor: "Elevated risk score".to_string(),
            severity: "High".to_string(),
            description: format!("Risk score of {:.2} exceeds caution threshold", company.risk_score),
        });
    }
    if company.threat_score > 0.5 {
        factors.push(RiskFactor {
            factor: "Threat detected".to_string(),
            severity: "High".to_string(),
            description: format!("Threat score of {:.2} indicates potential concerns", company.threat_score),
        });
    }

    RiskAssessment {
        overall_risk: overall,
        risk_label: label.to_string(),
        threat_score: company.threat_score.clamp(0.0, 1.0),
        factors,
    }
}

/// Analyze opportunities for engagement.
pub fn analyze_opportunities(company: &Company) -> OpportunityAnalysis {
    let mut opportunities = Vec::new();

    if company.overlap_score > 0.5 {
        opportunities.push("High capability overlap — potential for direct engagement".to_string());
    }
    if company.strategic_relevance > 0.7 {
        opportunities.push("High strategic relevance — priority target for business development".to_string());
    }
    if company.revenue_estimate_usd.unwrap_or(0) > 100_000_000 {
        opportunities.push("Large revenue base indicates significant procurement volume".to_string());
    }
    if company.employee_estimate.unwrap_or(0) > 500 {
        opportunities.push("Scale suggests multiple supply chain needs".to_string());
    }

    let approach = if company.strategic_relevance > 0.7 && company.overlap_score > 0.5 {
        "Proactive engagement with full capabilities deck".to_string()
    } else if company.strategic_relevance > 0.5 {
        "Targeted approach highlighting specific capability matches".to_string()
    } else {
        "Monitor and engage when opportunity signals emerge".to_string()
    };

    OpportunityAnalysis {
        overlap_score: company.overlap_score.clamp(0.0, 1.0),
        strategic_relevance: company.strategic_relevance.clamp(0.0, 1.0),
        opportunities,
        approach_recommendation: approach,
    }
}

/// Build site summaries from sites data.
pub fn summarize_sites(sites: &[Site]) -> Vec<SiteSummary> {
    sites
        .iter()
        .map(|s| SiteSummary {
            name: s.name.clone(),
            site_type: s.site_type.as_str().to_string(),
            city: s.city.clone(),
            country: s.country_code.clone(),
            capabilities: s.capabilities.clone(),
            certifications: s.certifications.clone(),
        })
        .collect()
}

/// Determine competitive position from company type and scores.
pub fn assess_competitive_position(
    company: &Company,
    capabilities: &[Capability],
) -> CompetitivePosition {
    let mut strengths = Vec::new();
    let mut weaknesses = Vec::new();

    let grade_a_count = capabilities
        .iter()
        .filter(|c| c.proof_grade == ProofGrade::A)
        .count();

    if grade_a_count > 3 {
        strengths.push("Strong certified capability base".to_string());
    }
    if company.strategic_relevance > 0.6 {
        strengths.push("High strategic relevance to Starz Electronics".to_string());
    }
    if !company.industry_tags.is_empty() {
        strengths.push(format!(
            "Active in {} industry segments",
            company.industry_tags.len()
        ));
    }

    if grade_a_count < 2 {
        weaknesses.push("Limited certified capabilities".to_string());
    }
    if company.risk_score > 0.5 {
        weaknesses.push("Elevated risk profile".to_string());
    }
    if company.employee_estimate.unwrap_or(0) < 50 {
        weaknesses.push("Small scale may limit capacity".to_string());
    }

    let position = if strengths.len() > weaknesses.len() {
        "Strong — Multiple advantages outweigh concerns"
    } else if strengths.is_empty() {
        "Weak — Limited known strengths"
    } else {
        "Moderate — Balanced profile with areas for improvement"
    };

    CompetitivePosition {
        position_label: position.to_string(),
        strengths,
        weaknesses,
    }
}

/// Render a company dossier as Markdown text.
pub fn render_company_dossier_text(dossier: &CompanyDossier) -> String {
    let mut lines = Vec::new();

    lines.push(format!("# Company Intelligence Dossier: {}", dossier.company_name));
    lines.push(format!("*Generated: {}*", dossier.generated_at.format("%Y-%m-%d %H:%M UTC")));
    lines.push(String::new());

    // Profile
    let p = &dossier.profile_section;
    lines.push("## Company Profile".to_string());
    lines.push(format!("- **Type:** {}", p.company_type));
    lines.push(format!("- **Country:** {}", p.country));
    lines.push(format!("- **Region:** {}", p.region));
    if let Some(emp) = p.employee_estimate {
        lines.push(format!("- **Employees (est.):** {}", emp));
    }
    if let Some(rev) = p.revenue_estimate_usd {
        lines.push(format!("- **Revenue (est.):** ${}", rev));
    }
    lines.push(format!("- **Strategic Relevance:** {:.0}%", p.strategic_relevance * 100.0));
    lines.push(String::new());

    // Capabilities
    let c = &dossier.capability_assessment;
    lines.push("## Capability Assessment".to_string());
    lines.push(format!(
        "{} capabilities identified (A:{}, B:{}, C:{}, D:{}). Coverage score: {:.0}%",
        c.total_capabilities, c.grade_a_count, c.grade_b_count, c.grade_c_count, c.grade_d_count,
        c.coverage_score * 100.0
    ));
    for cap in &c.top_capabilities {
        lines.push(format!("- **{}** [Grade {}] ({} evidence sources)", cap.capability, cap.proof_grade, cap.evidence_count));
    }
    lines.push(String::new());

    // Certifications
    let cert = &dossier.certification_analysis;
    lines.push("## Certification Analysis".to_string());
    lines.push(format!(
        "{} certifications ({} active, {} expired, {} pending). Health: {:.0}%",
        cert.total, cert.active, cert.expired, cert.pending,
        cert.cert_health_score * 100.0
    ));
    if !cert.gaps.is_empty() {
        lines.push(format!("**Gaps:** {}", cert.gaps.join(", ")));
    }
    lines.push(String::new());

    // Risk
    let r = &dossier.risk_assessment;
    lines.push("## Risk Assessment".to_string());
    lines.push(format!("**Overall:** {} ({:.2})", r.risk_label, r.overall_risk));
    for f in &r.factors {
        lines.push(format!("- [{}] {} — {}", f.severity, f.factor, f.description));
    }
    lines.push(String::new());

    // Opportunity
    let o = &dossier.opportunity_analysis;
    lines.push("## Opportunity Analysis".to_string());
    lines.push(format!("**Approach:** {}", o.approach_recommendation));
    for opp in &o.opportunities {
        lines.push(format!("- {}", opp));
    }
    lines.push(String::new());

    // Sites
    if !dossier.site_summaries.is_empty() {
        lines.push("## Site Summaries".to_string());
        for site in &dossier.site_summaries {
            let loc = match (&site.city, &site.country) {
                (Some(c), Some(co)) => format!("{}, {}", c, co),
                (Some(c), None) => c.clone(),
                (None, Some(co)) => co.clone(),
                _ => "Unknown".to_string(),
            };
            lines.push(format!("### {} ({}) — {}", site.name, site.site_type, loc));
            if !site.capabilities.is_empty() {
                lines.push(format!("Capabilities: {}", site.capabilities.join(", ")));
            }
            if !site.certifications.is_empty() {
                lines.push(format!("Certifications: {}", site.certifications.join(", ")));
            }
        }
        lines.push(String::new());
    }

    // Competitive position
    let cp = &dossier.competitive_position;
    lines.push("## Competitive Position".to_string());
    lines.push(format!("**Assessment:** {}", cp.position_label));
    if !cp.strengths.is_empty() {
        lines.push("Strengths:".to_string());
        for s in &cp.strengths {
            lines.push(format!("- {}", s));
        }
    }
    if !cp.weaknesses.is_empty() {
        lines.push("Weaknesses:".to_string());
        for w in &cp.weaknesses {
            lines.push(format!("- {}", w));
        }
    }

    lines.join("\n")
}

/// Generate a complete company dossier.
pub fn generate_company_dossier(
    company: &Company,
    capabilities: &[Capability],
    certifications: &[Certification],
    sites: &[Site],
    required_standards: &[&str],
) -> CompanyDossier {
    let profile = CompanyProfile {
        name: company.name.clone(),
        company_type: company.company_type.as_str().to_string(),
        country: company.country_code.clone().unwrap_or_else(|| "N/A".to_string()),
        region: company.region.clone().unwrap_or_else(|| "N/A".to_string()),
        industry_tags: company.industry_tags.clone(),
        employee_estimate: company.employee_estimate,
        revenue_estimate_usd: company.revenue_estimate_usd,
        strategic_relevance: company.strategic_relevance,
    };

    let cap_assessment = assess_capabilities(capabilities);
    let cert_analysis = analyze_certifications(certifications, required_standards);
    let risk = assess_risk(company);
    let opportunity = analyze_opportunities(company);
    let site_sums = summarize_sites(sites);
    let competitive = assess_competitive_position(company, capabilities);

    let mut dossier = CompanyDossier {
        id: Uuid::new_v4(),
        company_id: company.id,
        company_name: company.name.clone(),
        generated_at: Utc::now(),
        profile_section: profile,
        capability_assessment: cap_assessment,
        certification_analysis: cert_analysis,
        risk_assessment: risk,
        opportunity_analysis: opportunity,
        site_summaries: site_sums,
        competitive_position: competitive,
        full_text: String::new(),
    };

    dossier.full_text = render_company_dossier_text(&dossier);
    dossier
}

// ────────────────────────────────────────────
// POI dossier generation
// ────────────────────────────────────────────

/// Build professional profile from Person entity.
pub fn build_professional_profile(person: &Person, org_name: Option<&str>) -> ProfessionalProfile {
    ProfessionalProfile {
        name: person.name.clone(),
        current_role: person.current_role.clone(),
        role_family: person.role_family.as_str().to_string(),
        organization: org_name.map(|s| s.to_string()),
        country: person.country_code.clone(),
        region: person.region.clone(),
        bio_summary: person.public_bio.clone(),
        decision_style: person.decision_style.clone(),
        risk_tolerance: person.risk_tolerance.clone(),
        change_appetite: person.change_appetite.clone(),
    }
}

/// Analyze priority vector and produce interpretation.
pub fn analyze_priorities(person: &Person) -> PriorityAnalysis {
    let pv = &person.priority_vector;
    let dominant = pv.dominant().to_string();

    let interpretation = match dominant.as_str() {
        "cost" => "Cost-focused decision maker — lead with ROI and TCO analysis".to_string(),
        "quality" => "Quality-driven — emphasize certifications, audit readiness, zero-defect capability".to_string(),
        "speed" => "Speed-oriented — highlight lead times, rapid prototyping, quick-turn capability".to_string(),
        "resilience" => "Resilience-focused — stress dual sourcing, geographic diversity, business continuity".to_string(),
        "compliance" => "Compliance-driven — lead with regulatory alignment, standards, documentation".to_string(),
        "security" => "Security-conscious — emphasize ITAR readiness, data protection, facility security".to_string(),
        _ => "Balanced decision-making — present comprehensive value proposition".to_string(),
    };

    PriorityAnalysis {
        dominant_priority: dominant,
        cost: pv.cost,
        quality: pv.quality,
        speed: pv.speed,
        resilience: pv.resilience,
        compliance: pv.compliance,
        security: pv.security,
        interpretation,
    }
}

/// Assess influence level.
pub fn assess_influence(person: &Person) -> InfluenceAssessment {
    let label = if person.influence_score >= 0.8 {
        "Key Decision Maker"
    } else if person.influence_score >= 0.6 {
        "Strong Influencer"
    } else if person.influence_score >= 0.4 {
        "Moderate Influencer"
    } else if person.influence_score >= 0.2 {
        "Minor Influencer"
    } else {
        "Limited Influence"
    };

    InfluenceAssessment {
        influence_score: person.influence_score,
        influence_label: label.to_string(),
        pain_index: person.pain_index,
        role_drift: person.role_drift_score,
        change_risk: person.change_risk,
        trigger_topics: person.trigger_topics.clone(),
    }
}

/// Summarize POI artifacts.
pub fn summarize_artifacts(artifacts: &[PoiArtifact]) -> ArtifactSummarySection {
    let mut by_type: HashMap<String, usize> = HashMap::new();
    for art in artifacts {
        *by_type.entry(art.artifact_type.as_str().to_string()).or_default() += 1;
    }

    // Highlights: most recent 5
    let mut sorted = artifacts.to_vec();
    sorted.sort_by(|a, b| b.ts_utc.cmp(&a.ts_utc));

    let highlights: Vec<ArtifactHighlight> = sorted
        .iter()
        .take(5)
        .filter(|a| !a.url.is_empty()) // B177: skip artifacts with no URL
        .map(|a| ArtifactHighlight {
            artifact_type: a.artifact_type.as_str().to_string(),
            title: a.title.clone(),
            url: a.url.clone(),
            date: a.ts_utc,
        })
        .collect();

    ArtifactSummarySection {
        total_artifacts: artifacts.len(),
        by_type,
        recent_highlights: highlights,
    }
}

/// Generate approach guidance for a POI.
pub fn generate_approach_guidance(person: &Person) -> ApproachGuidance {
    let proof_type = person.preferred_proof_type.clone();

    let mut talking_points = Vec::new();
    // Based on role family
    match person.role_family {
        apex_core::entities::RoleFamily::Procurement => {
            talking_points.push("Total cost of ownership analysis".to_string());
            talking_points.push("Supplier consolidation benefits".to_string());
            talking_points.push("Payment terms flexibility".to_string());
        }
        apex_core::entities::RoleFamily::Quality => {
            talking_points.push("Zero-defect manufacturing process".to_string());
            talking_points.push("Certification coverage (IATF, ISO, AS9100)".to_string());
            talking_points.push("Audit readiness and traceability systems".to_string());
        }
        apex_core::entities::RoleFamily::Engineering => {
            talking_points.push("DFM collaboration capability".to_string());
            talking_points.push("Rapid prototyping and NPI services".to_string());
            talking_points.push("Technology roadmap alignment".to_string());
        }
        apex_core::entities::RoleFamily::Operations => {
            talking_points.push("Capacity planning and scalability".to_string());
            talking_points.push("Lead time reliability metrics".to_string());
            talking_points.push("Lean manufacturing implementation".to_string());
        }
        apex_core::entities::RoleFamily::Security => {
            talking_points.push("Facility security and access controls".to_string());
            talking_points.push("ITAR/export compliance readiness".to_string());
            talking_points.push("Data protection and cybersecurity measures".to_string());
        }
        apex_core::entities::RoleFamily::Executive => {
            talking_points.push("Strategic partnership value proposition".to_string());
            talking_points.push("Regional diversification benefits".to_string());
            talking_points.push("Innovation and growth roadmap".to_string());
        }
        apex_core::entities::RoleFamily::Finance => {
            talking_points.push("ROI projections and cost reduction potential".to_string());
            talking_points.push("Payment structure and credit terms".to_string());
            talking_points.push("Currency hedging for cross-border transactions".to_string());
        }
        apex_core::entities::RoleFamily::Government => {
            talking_points.push("Investment and job creation potential".to_string());
            talking_points.push("Compliance with local regulations".to_string());
            talking_points.push("Technology transfer and capacity building".to_string());
        }
        _ => {
            talking_points.push("General capabilities overview".to_string());
            talking_points.push("Quality and compliance track record".to_string());
        }
    }

    // Avoid topics based on pain index / risk tolerance
    let mut avoid = Vec::new();
    if person.pain_index > 0.5 {
        avoid.push("Avoid minimizing their current challenges".to_string());
    }
    if person.risk_tolerance.as_deref() == Some("low") {
        avoid.push("Avoid aggressive or disruptive positioning".to_string());
    }

    // Channel recommendation based on change appetite
    let channel = match person.change_appetite.as_deref() {
        Some("early_adopter") => "Direct outreach via email or conference meeting".to_string(),
        Some("pragmatist") => "Referral-based introduction or industry event".to_string(),
        Some("conservative") => "Formal introduction through mutual connection or association".to_string(),
        Some("laggard") => "Warm introduction with strong reference cases".to_string(),
        _ => "Professional email with concise value proposition".to_string(),
    };

    // Timing
    let timing = if person.pain_index > 0.6 {
        "Urgent — pain level suggests immediate receptivity".to_string()
    } else if person.change_risk > 0.5 {
        "Time-sensitive — role change risk means engagement window may close".to_string()
    } else {
        "Standard — schedule based on next procurement cycle or industry event".to_string()
    };

    // Engagement priority based on influence and role
    let engagement_priority = if person.influence_score > 0.7 {
        "P0 — Critical".to_string()
    } else if person.influence_score > 0.5 {
        "P1 — High".to_string()
    } else if person.influence_score > 0.3 {
        "P2 — Strategic".to_string()
    } else {
        "P3 — Monitor".to_string()
    };

    ApproachGuidance {
        recommended_proof_type: proof_type,
        talking_points,
        avoid_topics: avoid,
        best_channel: channel,
        timing_recommendation: timing,
        engagement_priority,
    }
}

/// Render a POI dossier as Markdown text.
pub fn render_poi_dossier_text(dossier: &PoiDossier) -> String {
    let mut lines = Vec::new();

    lines.push(format!("# Stakeholder Dossier: {}", dossier.person_name));
    lines.push(format!("*Generated: {}*", dossier.generated_at.format("%Y-%m-%d %H:%M UTC")));
    lines.push(String::new());

    // Professional profile
    let p = &dossier.professional_profile;
    lines.push("## Professional Profile".to_string());
    if let Some(ref role) = p.current_role {
        lines.push(format!("- **Role:** {}", role));
    }
    lines.push(format!("- **Role Family:** {}", p.role_family));
    if let Some(ref org) = p.organization {
        lines.push(format!("- **Organization:** {}", org));
    }
    if let Some(ref country) = p.country {
        lines.push(format!("- **Country:** {}", country));
    }
    if let Some(ref style) = p.decision_style {
        lines.push(format!("- **Decision Style:** {}", style));
    }
    if let Some(ref risk) = p.risk_tolerance {
        lines.push(format!("- **Risk Tolerance:** {}", risk));
    }
    lines.push(String::new());

    // Priorities
    let pr = &dossier.priority_analysis;
    lines.push("## Priority Analysis".to_string());
    lines.push(format!("**Dominant Priority:** {}", pr.dominant_priority));
    lines.push(format!("Cost: {:.0}% | Quality: {:.0}% | Speed: {:.0}% | Resilience: {:.0}% | Compliance: {:.0}% | Security: {:.0}%",
        pr.cost * 100.0, pr.quality * 100.0, pr.speed * 100.0,
        pr.resilience * 100.0, pr.compliance * 100.0, pr.security * 100.0));
    lines.push(format!("*{}*", pr.interpretation));
    lines.push(String::new());

    // Influence
    let inf = &dossier.influence_assessment;
    lines.push("## Influence Assessment".to_string());
    lines.push(format!("**Level:** {} ({:.0}%)", inf.influence_label, inf.influence_score * 100.0));
    lines.push(format!("Pain Index: {:.2} | Role Drift: {:.2} | Change Risk: {:.2}",
        inf.pain_index, inf.role_drift, inf.change_risk));
    if !inf.trigger_topics.is_empty() {
        lines.push(format!("Trigger Topics: {}", inf.trigger_topics.join(", ")));
    }
    lines.push(String::new());

    // Artifacts
    let art = &dossier.artifact_summary;
    lines.push("## Artifact Summary".to_string());
    lines.push(format!("{} artifacts collected", art.total_artifacts));
    for (atype, count) in &art.by_type {
        lines.push(format!("- {}: {}", atype, count));
    }
    if !art.recent_highlights.is_empty() {
        lines.push("Recent highlights:".to_string());
        for h in &art.recent_highlights {
            let title = h.title.as_deref().unwrap_or("(untitled)");
            lines.push(format!("- [{}] {} — {}", h.artifact_type, title, h.date.format("%Y-%m-%d")));
        }
    }
    lines.push(String::new());

    // Approach guidance
    let g = &dossier.approach_guidance;
    lines.push("## Approach Guidance".to_string());
    lines.push(format!("**Engagement Priority:** {}", g.engagement_priority));
    lines.push(format!("**Best Channel:** {}", g.best_channel));
    lines.push(format!("**Timing:** {}", g.timing_recommendation));
    if let Some(ref proof) = g.recommended_proof_type {
        lines.push(format!("**Proof Type:** {}", proof));
    }
    lines.push("Talking Points:".to_string());
    for tp in &g.talking_points {
        lines.push(format!("- {}", tp));
    }
    if !g.avoid_topics.is_empty() {
        lines.push("Avoid:".to_string());
        for a in &g.avoid_topics {
            lines.push(format!("- {}", a));
        }
    }

    lines.join("\n")
}

/// Generate a complete POI dossier.
pub fn generate_poi_dossier(
    person: &Person,
    org_name: Option<&str>,
    artifacts: &[PoiArtifact],
) -> PoiDossier {
    let profile = build_professional_profile(person, org_name);
    let priorities = analyze_priorities(person);
    let influence = assess_influence(person);
    let artifact_summary = summarize_artifacts(artifacts);
    let guidance = generate_approach_guidance(person);

    let mut dossier = PoiDossier {
        id: Uuid::new_v4(),
        person_id: person.id,
        person_name: person.name.clone(),
        generated_at: Utc::now(),
        professional_profile: profile,
        priority_analysis: priorities,
        influence_assessment: influence,
        artifact_summary,
        approach_guidance: guidance,
        full_text: String::new(),
    };

    dossier.full_text = render_poi_dossier_text(&dossier);
    dossier
}

// ────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use apex_core::entities::*;

    fn sample_company() -> Company {
        let mut c = Company::new("Foxconn Technology", CompanyType::Oem);
        c.country_code = Some("TW".to_string());
        c.region = Some("EA".to_string());
        c.employee_estimate = Some(800_000);
        c.revenue_estimate_usd = Some(200_000_000_000);
        c.risk_score = 0.3;
        c.threat_score = 0.2;
        c.overlap_score = 0.7;
        c.strategic_relevance = 0.85;
        c.industry_tags = vec!["electronics".to_string(), "automotive".to_string(), "industrial".to_string()];
        c
    }

    fn sample_capabilities() -> Vec<Capability> {
        let id = Uuid::new_v4();
        vec![
            {
                let mut c = Capability::new(id, "SMT Assembly", ProofGrade::A);
                c.evidence_urls = vec!["https://example.com/1".to_string(), "https://example.com/2".to_string()];
                c
            },
            Capability::new(id, "PTH Assembly", ProofGrade::A),
            Capability::new(id, "Box Build", ProofGrade::B),
            Capability::new(id, "Cable Harness", ProofGrade::C),
            Capability::new(id, "IC Programming", ProofGrade::D),
        ]
    }

    fn sample_certifications() -> Vec<Certification> {
        let id = Uuid::new_v4();
        let mut certs = Vec::new();

        let mut c1 = Certification::new(id, "ISO_9001");
        c1.status = CertStatus::Active;
        c1.valid_until = Some(chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
        certs.push(c1);

        let mut c2 = Certification::new(id, "ISO_14001");
        c2.status = CertStatus::Active;
        certs.push(c2);

        let mut c3 = Certification::new(id, "IATF_16949");
        c3.status = CertStatus::Expired;
        certs.push(c3);

        certs
    }

    fn sample_sites() -> Vec<Site> {
        let id = Uuid::new_v4();
        let mut s1 = Site::new(id, "Shenzhen Plant", SiteType::Plant);
        s1.city = Some("Shenzhen".to_string());
        s1.country_code = Some("CN".to_string());
        s1.capabilities = vec!["SMT".to_string(), "PTH".to_string()];
        s1.certifications = vec!["ISO_9001".to_string()];

        let mut s2 = Site::new(id, "Taipei HQ", SiteType::Hq);
        s2.city = Some("Taipei".to_string());
        s2.country_code = Some("TW".to_string());

        vec![s1, s2]
    }

    fn sample_person() -> Person {
        let mut p = Person::new("Ahmed Ben Salah", RoleFamily::Procurement);
        p.current_role = Some("VP Procurement".to_string());
        p.country_code = Some("TN".to_string());
        p.region = Some("TN".to_string());
        p.decision_style = Some("CostFirst".to_string());
        p.risk_tolerance = Some("low".to_string());
        p.change_appetite = Some("pragmatist".to_string());
        p.influence_score = 0.75;
        p.pain_index = 0.65;
        p.role_drift_score = 0.2;
        p.change_risk = 0.3;
        p.preferred_proof_type = Some("case_study".to_string());
        p.trigger_topics = vec!["cost reduction".to_string(), "quality".to_string()];
        p.priority_vector = PriorityVector {
            cost: 0.8,
            quality: 0.5,
            speed: 0.3,
            resilience: 0.4,
            compliance: 0.6,
            security: 0.2,
        };
        p
    }

    fn sample_artifacts() -> Vec<PoiArtifact> {
        let person_id = Uuid::new_v4();
        let now = Utc::now();
        vec![
            {
                let mut a = PoiArtifact::new(person_id, ArtifactType::PressQuote, "https://example.com/press/1", now);
                a.title = Some("Industry outlook interview".to_string());
                a
            },
            {
                let mut a = PoiArtifact::new(person_id, ArtifactType::SpeakerBio, "https://example.com/conf/speaker", now);
                a.title = Some("PCIM 2024 keynote".to_string());
                a
            },
            PoiArtifact::new(person_id, ArtifactType::Patent, "https://patents.example.com/123", now),
        ]
    }

    #[test]
    fn test_assess_capabilities() {
        let caps = sample_capabilities();
        let assessment = assess_capabilities(&caps);
        assert_eq!(assessment.total_capabilities, 5);
        assert_eq!(assessment.grade_a_count, 2);
        assert_eq!(assessment.grade_b_count, 1);
        assert_eq!(assessment.grade_c_count, 1);
        assert_eq!(assessment.grade_d_count, 1);
        // Coverage: (2*1.0 + 1*0.75 + 1*0.5 + 1*0.25) / 5 = 3.5 / 5 = 0.7
        assert!((assessment.coverage_score - 0.7).abs() < 0.01);
        assert_eq!(assessment.top_capabilities.len(), 5);
        assert_eq!(assessment.top_capabilities[0].capability, "SMT Assembly");
        assert_eq!(assessment.top_capabilities[0].evidence_count, 2);
    }

    #[test]
    fn test_assess_capabilities_empty() {
        let assessment = assess_capabilities(&[]);
        assert_eq!(assessment.total_capabilities, 0);
        assert_eq!(assessment.coverage_score, 0.0);
    }

    #[test]
    fn test_analyze_certifications() {
        let certs = sample_certifications();
        let required = &["ISO_9001", "ISO_14001", "AS9100"];
        let analysis = analyze_certifications(&certs, required);

        assert_eq!(analysis.total, 3);
        assert_eq!(analysis.active, 2);
        assert_eq!(analysis.expired, 1);
        assert_eq!(analysis.gaps.len(), 1);
        assert_eq!(analysis.gaps[0], "AS9100");
        assert!(analysis.cert_health_score > 0.0);
    }

    #[test]
    fn test_analyze_certifications_all_active() {
        let id = Uuid::new_v4();
        let c1 = Certification::new(id, "ISO_9001");
        let c2 = Certification::new(id, "ISO_14001");
        let analysis = analyze_certifications(&[c1, c2], &["ISO_9001", "ISO_14001"]);
        assert_eq!(analysis.active, 2);
        assert!(analysis.gaps.is_empty());
        assert!((analysis.cert_health_score - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_assess_risk() {
        let company = sample_company();
        let risk = assess_risk(&company);
        assert_eq!(risk.risk_label, "Low");
        assert!((risk.overall_risk - 0.3).abs() < 0.01);
        assert!(risk.factors.is_empty()); // risk_score and threat_score are both <= 0.5
    }

    #[test]
    fn test_assess_risk_high() {
        let mut company = sample_company();
        company.risk_score = 0.8;
        company.threat_score = 0.7;
        let risk = assess_risk(&company);
        assert_eq!(risk.risk_label, "Critical");
        assert_eq!(risk.factors.len(), 2);
    }

    #[test]
    fn test_analyze_opportunities() {
        let company = sample_company();
        let opps = analyze_opportunities(&company);
        assert_eq!(opps.overlap_score, 0.7);
        assert!(opps.opportunities.len() >= 3); // overlap, strategic relevance, revenue, employees
        assert!(opps.approach_recommendation.contains("Proactive"));
    }

    #[test]
    fn test_summarize_sites() {
        let sites = sample_sites();
        let summaries = summarize_sites(&sites);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].name, "Shenzhen Plant");
        assert_eq!(summaries[0].site_type, "plant");
        assert_eq!(summaries[0].capabilities.len(), 2);
        assert_eq!(summaries[1].name, "Taipei HQ");
    }

    #[test]
    fn test_assess_competitive_position() {
        let company = sample_company();
        let caps = sample_capabilities();
        let position = assess_competitive_position(&company, &caps);
        assert!(position.position_label.contains("Strong"));
        assert!(!position.strengths.is_empty());
    }

    #[test]
    fn test_generate_company_dossier() {
        let company = sample_company();
        let caps = sample_capabilities();
        let certs = sample_certifications();
        let sites = sample_sites();
        let required = &["ISO_9001", "AS9100"];

        let dossier = generate_company_dossier(&company, &caps, &certs, &sites, required);

        assert_eq!(dossier.company_name, "Foxconn Technology");
        assert!(!dossier.full_text.is_empty());
        assert!(dossier.full_text.contains("# Company Intelligence Dossier: Foxconn Technology"));
        assert!(dossier.full_text.contains("## Capability Assessment"));
        assert!(dossier.full_text.contains("## Risk Assessment"));
        assert!(dossier.full_text.contains("## Opportunity Analysis"));
        assert_eq!(dossier.capability_assessment.total_capabilities, 5);
        assert_eq!(dossier.certification_analysis.gaps.len(), 1);
    }

    #[test]
    fn test_build_professional_profile() {
        let person = sample_person();
        let profile = build_professional_profile(&person, Some("Foxconn"));
        assert_eq!(profile.name, "Ahmed Ben Salah");
        assert_eq!(profile.role_family, "procurement");
        assert_eq!(profile.organization, Some("Foxconn".to_string()));
        assert_eq!(profile.decision_style, Some("CostFirst".to_string()));
    }

    #[test]
    fn test_analyze_priorities() {
        let person = sample_person();
        let analysis = analyze_priorities(&person);
        assert_eq!(analysis.dominant_priority, "cost");
        assert!((analysis.cost - 0.8).abs() < 0.01);
        assert!(analysis.interpretation.contains("ROI"));
    }

    #[test]
    fn test_assess_influence() {
        let person = sample_person();
        let influence = assess_influence(&person);
        assert_eq!(influence.influence_label, "Strong Influencer");
        assert!((influence.influence_score - 0.75).abs() < 0.01);
        assert!((influence.pain_index - 0.65).abs() < 0.01);
    }

    #[test]
    fn test_assess_influence_low() {
        let mut person = sample_person();
        person.influence_score = 0.1;
        let influence = assess_influence(&person);
        assert_eq!(influence.influence_label, "Limited Influence");
    }

    #[test]
    fn test_summarize_artifacts() {
        let artifacts = sample_artifacts();
        let summary = summarize_artifacts(&artifacts);
        assert_eq!(summary.total_artifacts, 3);
        assert!(summary.by_type.contains_key("press_quote"));
        assert!(summary.by_type.contains_key("speaker_bio"));
        assert!(summary.by_type.contains_key("patent"));
        assert_eq!(summary.recent_highlights.len(), 3);
    }

    #[test]
    fn test_summarize_artifacts_empty() {
        let summary = summarize_artifacts(&[]);
        assert_eq!(summary.total_artifacts, 0);
        assert!(summary.by_type.is_empty());
        assert!(summary.recent_highlights.is_empty());
    }

    #[test]
    fn test_generate_approach_guidance_procurement() {
        let person = sample_person();
        let guidance = generate_approach_guidance(&person);
        assert!(guidance.talking_points.iter().any(|t| t.contains("cost")));
        assert_eq!(guidance.engagement_priority, "P0 — Critical");
        assert!(guidance.timing_recommendation.contains("Urgent"));
        assert!(guidance.best_channel.contains("Referral"));
        assert_eq!(guidance.recommended_proof_type, Some("case_study".to_string()));
        // pain_index > 0.5 and risk_tolerance = "low"
        assert_eq!(guidance.avoid_topics.len(), 2);
    }

    #[test]
    fn test_generate_approach_guidance_engineering() {
        let mut person = sample_person();
        person.role_family = RoleFamily::Engineering;
        person.influence_score = 0.45;
        person.pain_index = 0.2;
        person.risk_tolerance = Some("high".to_string());
        person.change_appetite = Some("early_adopter".to_string());

        let guidance = generate_approach_guidance(&person);
        assert!(guidance.talking_points.iter().any(|t| t.contains("DFM")));
        assert_eq!(guidance.engagement_priority, "P2 — Strategic");
        assert!(guidance.best_channel.contains("Direct"));
        assert!(guidance.avoid_topics.is_empty()); // pain < 0.5 and risk is "high"
    }

    #[test]
    fn test_generate_poi_dossier() {
        let person = sample_person();
        let artifacts = sample_artifacts();
        let dossier = generate_poi_dossier(&person, Some("Foxconn"), &artifacts);

        assert_eq!(dossier.person_name, "Ahmed Ben Salah");
        assert!(!dossier.full_text.is_empty());
        assert!(dossier.full_text.contains("# Stakeholder Dossier: Ahmed Ben Salah"));
        assert!(dossier.full_text.contains("## Professional Profile"));
        assert!(dossier.full_text.contains("## Priority Analysis"));
        assert!(dossier.full_text.contains("## Influence Assessment"));
        assert!(dossier.full_text.contains("## Artifact Summary"));
        assert!(dossier.full_text.contains("## Approach Guidance"));
        assert_eq!(dossier.priority_analysis.dominant_priority, "cost");
        assert_eq!(dossier.influence_assessment.influence_label, "Strong Influencer");
        assert_eq!(dossier.artifact_summary.total_artifacts, 3);
    }

    #[test]
    fn test_render_company_dossier_text() {
        let company = sample_company();
        let caps = sample_capabilities();
        let certs = sample_certifications();
        let sites = sample_sites();
        let dossier = generate_company_dossier(&company, &caps, &certs, &sites, &["ISO_9001"]);
        let text = &dossier.full_text;

        // Check that Markdown headings are present
        let h2_count = text.lines().filter(|l| l.starts_with("## ")).count();
        assert!(h2_count >= 5);

        // Should contain key data
        assert!(text.contains("SMT Assembly"));
        assert!(text.contains("Shenzhen Plant"));
    }

    #[test]
    fn test_render_poi_dossier_text() {
        let person = sample_person();
        let dossier = generate_poi_dossier(&person, Some("Foxconn"), &[]);
        let text = &dossier.full_text;

        let h2_count = text.lines().filter(|l| l.starts_with("## ")).count();
        assert!(h2_count >= 5);

        assert!(text.contains("VP Procurement"));
        assert!(text.contains("procurement"));
        assert!(text.contains("P0 — Critical"));
    }

    // ── B171: missing optional fields use N/A not empty ──

    #[test]
    fn test_company_profile_missing_fields_use_na() {
        let mut company = sample_company();
        company.country_code = None;
        company.region = None;
        let dossier = generate_company_dossier(&company, &[], &[], &[], &[]);
        assert_eq!(dossier.profile_section.country, "N/A");
        assert_eq!(dossier.profile_section.region, "N/A");
    }

    // ── B172: top_capabilities bounded ──

    #[test]
    fn test_top_capabilities_bounded() {
        let id = Uuid::new_v4();
        let caps: Vec<Capability> = (0..20)
            .map(|i| Capability::new(id, &format!("Cap_{}", i), ProofGrade::A))
            .collect();
        let assessment = assess_capabilities(&caps);
        assert!(assessment.top_capabilities.len() <= 10);
    }

    // ── B173: cert_health_score scaling ──

    #[test]
    fn test_cert_health_score_clamped() {
        let id = Uuid::new_v4();
        // All expired with many gaps — health should not go below 0
        let c = Certification::new(id, "FAKE");
        let analysis = analyze_certifications(
            &[c],
            &["ISO_9001", "ISO_14001", "AS9100", "IATF_16949", "ISO_27001",
              "ISO_13485", "AS9100D", "ISO_45001", "ISO_50001", "NADCAP",
              "SO_17025"],
        );
        assert!(analysis.cert_health_score >= 0.0);
        assert!(analysis.cert_health_score <= 1.0);
    }

    // ── B174: certification gaps with mixed statuses ──

    #[test]
    fn test_cert_gaps_mixed_statuses() {
        let id = Uuid::new_v4();
        let mut c1 = Certification::new(id, "ISO_9001");
        c1.status = CertStatus::Active;
        let mut c2 = Certification::new(id, "IATF_16949");
        c2.status = CertStatus::Expired;
        let mut c3 = Certification::new(id, "AS9100");
        c3.status = CertStatus::Pending;
        let analysis = analyze_certifications(
            &[c1, c2, c3],
            &["ISO_9001", "IATF_16949", "AS9100", "ISO_14001"],
        );
        // ISO_9001 active → no gap; IATF expired → gap; AS9100 pending → gap; ISO_14001 missing → gap
        assert!(analysis.gaps.contains(&"IATF_16949".to_string()));
        assert!(analysis.gaps.contains(&"AS9100".to_string()));
        assert!(analysis.gaps.contains(&"ISO_14001".to_string()));
        assert!(!analysis.gaps.contains(&"ISO_9001".to_string()));
    }

    // ── B175: risk_label consistent with numeric ranges ──

    #[test]
    fn test_risk_label_ranges() {
        let mut c = sample_company();
        for (score, expected_label) in [
            (0.0, "Minimal"), (0.19, "Minimal"), (0.2, "Low"), (0.39, "Low"),
            (0.4, "Medium"), (0.59, "Medium"), (0.6, "High"), (0.79, "High"),
            (0.8, "Critical"), (1.0, "Critical"),
        ] {
            c.risk_score = score;
            let risk = assess_risk(&c);
            assert_eq!(risk.risk_label, expected_label,
                "risk_score={} should map to '{}', got '{}'", score, expected_label, risk.risk_label);
        }
    }

    // ── B176: opportunity_analysis with empty inputs ──

    #[test]
    fn test_opportunity_analysis_empty_company() {
        let c = Company::new("Empty", CompanyType::Other("Supplier".to_string()));
        let opps = analyze_opportunities(&c);
        assert!(opps.opportunities.is_empty());
        assert!(opps.approach_recommendation.contains("Monitor"));
    }

    // ── B177: artifact highlights skip empty URLs ──

    #[test]
    fn test_artifact_highlight_skips_empty_url() {
        let person_id = Uuid::new_v4();
        let now = Utc::now();
        let mut a = PoiArtifact::new(person_id, ArtifactType::PressQuote, "", now);
        a.title = Some("No URL artifact".to_string());
        let summary = summarize_artifacts(&[a]);
        assert_eq!(summary.total_artifacts, 1);
        assert!(summary.recent_highlights.is_empty(), "Artifacts with empty URL should be excluded from highlights");
    }

    // ── B178: artifact_summary counts match items ──

    #[test]
    fn test_artifact_summary_counts_match() {
        let artifacts = sample_artifacts();
        let summary = summarize_artifacts(&artifacts);
        let type_total: usize = summary.by_type.values().sum();
        assert_eq!(type_total, summary.total_artifacts);
    }
}
