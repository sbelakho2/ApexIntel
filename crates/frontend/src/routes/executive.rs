//! Executive View Dashboard - Strategic opportunities, threats, market intelligence.
//!
//! Phase 4.3: User Experience Enhancement

use leptos::*;

use crate::{
    api::{self},
    components::{
        cards::{PageHeader, StatCard, SurfaceCard},
        charts::probability_gauge::ProbabilityGauge,
    },
};

// ────────────────────────────────────────────
// API Response Types
// ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StrategicOpportunity {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub opportunity_type: String,
    pub priority_score: f64,
    pub confidence: f64,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub region: Option<String>,
    pub estimated_value: Option<String>,
    pub recommended_actions: Vec<String>,
    pub owner_id: Option<String>,
    pub status: String,
    pub due_date: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CriticalThreat {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub threat_type: String,
    pub severity: String,
    pub impact_score: f64,
    pub confidence: f64,
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub region: Option<String>,
    pub mitigation_steps: Vec<String>,
    pub owner_id: Option<String>,
    pub status: String,
    pub sla_deadline: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarketIntelligenceSummary {
    pub total_opportunities: u64,
    pub total_threats: u64,
    pub high_priority_count: u64,
    pub regions_affected: Vec<String>,
    pub average_confidence: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecommendedAction {
    pub id: String,
    pub title: String,
    pub description: String,
    pub priority: String,
    pub owner: String,
    pub due_date: Option<String>,
    pub related_entity_id: Option<String>,
    pub related_entity_type: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutiveSummary {
    pub top_opportunities: Vec<StrategicOpportunity>,
    pub critical_threats: Vec<CriticalThreat>,
    pub market_intelligence: MarketIntelligenceSummary,
    pub recommended_actions: Vec<RecommendedAction>,
}

// ────────────────────────────────────────────
// API Functions
// ────────────────────────────────────────────

pub async fn fetch_executive_summary() -> Result<ExecutiveSummary, String> {
    api::get_json("/api/executive/summary").await
}

pub async fn fetch_opportunities() -> Result<Vec<StrategicOpportunity>, String> {
    api::get_json("/api/executive/opportunities").await
}

pub async fn fetch_threats() -> Result<Vec<CriticalThreat>, String> {
    api::get_json("/api/executive/threats").await
}

// ────────────────────────────────────────────
// Components
// ────────────────────────────────────────────

#[component]
fn OpportunityCard(opportunity: StrategicOpportunity) -> impl IntoView {
    let priority_class = if opportunity.priority_score > 0.8 {
        "priority-high"
    } else if opportunity.priority_score > 0.5 {
        "priority-medium"
    } else {
        "priority-low"
    };

    view! {
        <article class="opportunity-card">
            <div class="opportunity-header">
                <h3 class="opportunity-title">{opportunity.title}</h3>
                <span class={format!("priority-badge {}", priority_class)}>
                    {format!("{:.0}%", opportunity.priority_score * 100.0)}
                </span>
            </div>
            {opportunity.description.map(|desc| view! {
                <p class="opportunity-description">{desc}</p>
            })}
            <div class="opportunity-meta">
                <span class="meta-item">
                    <span class="meta-label">"Type:"</span>
                    {opportunity.opportunity_type}
                </span>
                {opportunity.region.map(|region| view! {
                    <span class="meta-item">
                        <span class="meta-label">"Region:"</span>
                        {region}
                    </span>
                })}
                {opportunity.estimated_value.map(|value| view! {
                    <span class="meta-item">
                        <span class="meta-label">"Value:"</span>
                        {value}
                    </span>
                })}
            </div>
            <div class="opportunity-footer">
                <div class="confidence-indicator">
                    <span class="meta-label">"Confidence:"</span>
                    <ProbabilityGauge probability=opportunity.confidence.clamp(0.0, 1.0) />
                </div>
                <span class="status-badge">{opportunity.status}</span>
            </div>
        </article>
    }
}

#[component]
fn ThreatCard(threat: CriticalThreat) -> impl IntoView {
    let severity_class = format!("severity-{}", threat.severity.to_lowercase());

    view! {
        <article class="threat-card">
            <div class="threat-header">
                <span class={format!("severity-chip {}", severity_class)}>{threat.severity}</span>
                <h3 class="threat-title">{threat.title}</h3>
            </div>
            {threat.description.map(|desc| view! {
                <p class="threat-description">{desc}</p>
            })}
            <div class="threat-meta">
                <span class="meta-item">
                    <span class="meta-label">"Type:"</span>
                    {threat.threat_type}
                </span>
                {threat.region.map(|region| view! {
                    <span class="meta-item">
                        <span class="meta-label">"Region:"</span>
                        {region}
                    </span>
                })}
                {threat.sla_deadline.map(|deadline| view! {
                    <span class="meta-item">
                        <span class="meta-label">"SLA:"</span>
                        {deadline}
                    </span>
                })}
            </div>
            <div class="mitigation-preview">
                <span class="meta-label">"Mitigation:"</span>
                <ul class="mitigation-list">
                    {threat.mitigation_steps.iter().take(3).map(|step| view! {
                        <li>{step}</li>
                    }).collect_view()}
                </ul>
            </div>
            <div class="threat-footer">
                <div class="confidence-indicator">
                    <span class="meta-label">"Confidence:"</span>
                    <ProbabilityGauge probability=threat.confidence.clamp(0.0, 1.0) />
                </div>
                <span class="status-badge">{threat.status}</span>
            </div>
        </article>
    }
}

#[component]
fn RecommendedActionRow(action: RecommendedAction) -> impl IntoView {
    let priority_class = format!("priority-{}", action.priority.to_lowercase());

    view! {
        <div class="action-row">
            <div class="action-priority">
                <span class={format!("priority-indicator {}", priority_class)}></span>
            </div>
            <div class="action-content">
                <h4 class="action-title">{action.title}</h4>
                <p class="action-description">{action.description}</p>
                <div class="action-meta">
                    <span class="meta-item">
                        <span class="meta-label">"Owner:"</span>
                        {action.owner}
                    </span>
                    {action.due_date.map(|date| view! {
                        <span class="meta-item">
                            <span class="meta-label">"Due:"</span>
                            {date}
                        </span>
                    })}
                </div>
            </div>
            <div class="action-status">
                <span class={format!("priority-badge small {}", priority_class)}>
                    {action.priority}
                </span>
            </div>
        </div>
    }
}

#[component]
fn MarketIntelligencePanel(summary: MarketIntelligenceSummary) -> impl IntoView {
    view! {
        <SurfaceCard title="Market Intelligence" subtitle="Summary of current intelligence landscape">
            <div class="market-grid">
                <div class="market-stat">
                    <span class="market-value">{summary.total_opportunities}</span>
                    <span class="market-label">"Total Opportunities"</span>
                </div>
                <div class="market-stat">
                    <span class="market-value">{summary.total_threats}</span>
                    <span class="market-label">"Active Threats"</span>
                </div>
                <div class="market-stat highlight">
                    <span class="market-value">{summary.high_priority_count}</span>
                    <span class="market-label">"High Priority"</span>
                </div>
                <div class="market-stat">
                    <span class="market-value">{format!("{:.0}%", summary.average_confidence * 100.0)}</span>
                    <span class="market-label">"Avg Confidence"</span>
                </div>
            </div>
            <div class="regions-list">
                <span class="meta-label">"Regions Affected:"</span>
                <div class="region-tags">
                    {summary.regions_affected.iter().map(|region| view! {
                        <span class="region-tag">{region}</span>
                    }).collect_view()}
                </div>
            </div>
        </SurfaceCard>
    }
}

// ────────────────────────────────────────────
// Main Executive Dashboard Page
// ────────────────────────────────────────────

#[component]
pub fn ExecutivePage() -> impl IntoView {
    let summary = create_resource(|| (), |_| async { fetch_executive_summary().await });

    view! {
        <div class="page executive-page">
            <PageHeader
                eyebrow="Strategic Overview"
                title="Executive Dashboard"
                subtitle="Top strategic opportunities, critical threats, and recommended actions."
            />

            <Suspense fallback=move || view! {
                <SurfaceCard title="Loading" subtitle="Fetching executive summary...">
                    <p class="muted-copy">"Loading executive dashboard..."</p>
                </SurfaceCard>
            }>
                {move || summary.get().map(|result| match result {
                    Ok(data) => {
                        let opportunities_count = data.top_opportunities.len();
                        let high_priority_count = data.top_opportunities.iter().filter(|o| o.priority_score > 0.8).count();
                        let threats_count = data.critical_threats.len();
                        let critical_count = data.critical_threats.iter().filter(|t| t.severity == "critical").count();
                        let actions_count = data.recommended_actions.len();
                        let avg_confidence = data.market_intelligence.average_confidence;
                        let opportunities_empty = data.top_opportunities.is_empty();
                        let threats_empty = data.critical_threats.is_empty();
                        let actions_empty = data.recommended_actions.is_empty();

                        view! {
                            <div class="executive-grid">
                                // Market Intelligence Summary
                                <div class="market-intelligence-section">
                                    <MarketIntelligencePanel summary=data.market_intelligence />
                                </div>

                                // Key Metrics
                                <div class="metrics-row">
                                    <StatCard
                                        label="Opportunities"
                                        value=opportunities_count.to_string()
                                        delta=format!("{} high priority", high_priority_count)
                                    >
                                        <div class="stat-icon opportunities-icon">"📈"</div>
                                    </StatCard>
                                    <StatCard
                                        label="Threats"
                                        value=threats_count.to_string()
                                        delta=format!("{} critical", critical_count)
                                    >
                                        <div class="stat-icon threats-icon">"⚠️"</div>
                                    </StatCard>
                                    <StatCard
                                        label="Actions"
                                        value=actions_count.to_string()
                                        delta="Prioritized list".to_string()
                                    >
                                        <div class="stat-icon actions-icon">"🎯"</div>
                                    </StatCard>
                                    <StatCard
                                        label="Confidence"
                                        value=format!("{:.0}%", avg_confidence * 100.0)
                                        delta="Average intelligence confidence".to_string()
                                    >
                                        <ProbabilityGauge probability=avg_confidence />
                                    </StatCard>
                                </div>

                                // Strategic Opportunities
                                <SurfaceCard title="Top 10 Strategic Opportunities" subtitle="High-priority opportunities requiring attention">
                                    <div class="opportunities-grid">
                                        <For each=move || data.top_opportunities.clone() key=|o| o.id.clone() let:opp>
                                            <OpportunityCard opportunity=opp />
                                        </For>
                                        {if opportunities_empty {
                                            view! {
                                                <div class="empty-state">
                                                    <p>"No strategic opportunities at this time."</p>
                                                </div>
                                            }.into_view()
                                        } else { view! {}.into_view() }}
                                    </div>
                                </SurfaceCard>

                                // Critical Threats
                                <SurfaceCard title="Critical Threats" subtitle="Active threats requiring immediate attention">
                                    <div class="threats-grid">
                                        <For each=move || data.critical_threats.clone() key=|t| t.id.clone() let:threat>
                                            <ThreatCard threat=threat />
                                        </For>
                                        {if threats_empty {
                                            view! {
                                                <div class="empty-state">
                                                    <p>"No critical threats detected."</p>
                                                </div>
                                            }.into_view()
                                        } else { view! {}.into_view() }}
                                    </div>
                                </SurfaceCard>

                                // Recommended Actions
                                <SurfaceCard title="Recommended Actions" subtitle="Prioritized actions based on opportunities and threats">
                                    <div class="actions-list">
                                        <For each=move || data.recommended_actions.clone() key=|a| a.id.clone() let:action>
                                            <RecommendedActionRow action=action />
                                        </For>
                                        {if actions_empty {
                                            view! {
                                                <div class="empty-state">
                                                    <p>"No recommended actions at this time."</p>
                                                </div>
                                            }.into_view()
                                        } else { view! {}.into_view() }}
                                    </div>
                                </SurfaceCard>
                            </div>
                        }.into_view()
                    }
                    Err(message) => view! {
                        <SurfaceCard title="Error" subtitle="Failed to load executive summary">
                            <p class="error-copy">{message}</p>
                        </SurfaceCard>
                    }.into_view(),
                })}
            </Suspense>
        </div>
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::api::ApiEnvelope;

    // ── API Response Type Tests ───────────────────────────────────────────────

    #[test]
    fn strategic_opportunity_deserialization() {
        let json = r#"{
            "id": "opp-123",
            "title": "Market Expansion Opportunity",
            "description": "Enter new market in EMEA",
            "opportunity_type": "market_expansion",
            "priority_score": 0.85,
            "confidence": 0.75,
            "entity_id": "company-456",
            "entity_type": "company",
            "region": "EMEA",
            "estimated_value": "$5M",
            "recommended_actions": ["Assess competition", "Hire local team"],
            "owner_id": "analyst-1",
            "status": "active",
            "due_date": "2024-06-30T00:00:00Z"
        }"#;

        let opp: StrategicOpportunity = serde_json::from_str(json).unwrap();
        assert_eq!(opp.id, "opp-123");
        assert_eq!(opp.title, "Market Expansion Opportunity");
        assert_eq!(opp.priority_score, 0.85);
        assert_eq!(opp.region, Some("EMEA".to_string()));
    }

    #[test]
    fn critical_threat_deserialization() {
        let json = r#"{
            "id": "threat-789",
            "title": "Supply Chain Disruption",
            "description": "Risk of supplier failure",
            "threat_type": "operational",
            "severity": "high",
            "impact_score": 0.8,
            "confidence": 0.9,
            "entity_id": "supplier-123",
            "entity_type": "company",
            "region": "NA",
            "mitigation_steps": ["Identify backup suppliers", "Diversify sources"],
            "owner_id": "risk-manager",
            "status": "active",
            "sla_deadline": "2024-03-15T00:00:00Z"
        }"#;

        let threat: CriticalThreat = serde_json::from_str(json).unwrap();
        assert_eq!(threat.id, "threat-789");
        assert_eq!(threat.severity, "high");
        assert_eq!(threat.impact_score, 0.8);
    }

    #[test]
    fn executive_summary_deserialization() {
        let json = r#"{
            "top_opportunities": [
                {
                    "id": "opp-1",
                    "title": "Opp 1",
                    "description": null,
                    "opportunity_type": "market",
                    "priority_score": 0.9,
                    "confidence": 0.8,
                    "entity_id": null,
                    "entity_type": null,
                    "region": null,
                    "estimated_value": null,
                    "recommended_actions": [],
                    "owner_id": null,
                    "status": "active",
                    "due_date": null
                }
            ],
            "critical_threats": [
                {
                    "id": "threat-1",
                    "title": "Threat 1",
                    "description": null,
                    "threat_type": "risk",
                    "severity": "critical",
                    "impact_score": 0.95,
                    "confidence": 0.85,
                    "entity_id": null,
                    "entity_type": null,
                    "region": null,
                    "mitigation_steps": [],
                    "owner_id": null,
                    "status": "active",
                    "sla_deadline": null
                }
            ],
            "market_intelligence": {
                "total_opportunities": 1,
                "total_threats": 1,
                "high_priority_count": 2,
                "regions_affected": ["EMEA", "NA"],
                "average_confidence": 0.825
            },
            "recommended_actions": [
                {
                    "id": "action-1",
                    "title": "Action 1",
                    "description": "Description",
                    "priority": "high",
                    "owner": "analyst",
                    "due_date": "2024-04-01T00:00:00Z",
                    "related_entity_id": null,
                    "related_entity_type": null
                }
            ]
        }"#;

        let summary: ExecutiveSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.top_opportunities.len(), 1);
        assert_eq!(summary.critical_threats.len(), 1);
        assert_eq!(summary.recommended_actions.len(), 1);
        assert_eq!(summary.market_intelligence.total_opportunities, 1);
    }

    #[test]
    fn recommended_action_deserialization() {
        let json = r#"{
            "id": "action-123",
            "title": "Pursue Acquisition",
            "description": "Evaluate target company for acquisition",
            "priority": "critical",
            "owner": "m&a-team",
            "due_date": "2024-05-30T00:00:00Z",
            "related_entity_id": "company-789",
            "related_entity_type": "company"
        }"#;

        let action: RecommendedAction = serde_json::from_str(json).unwrap();
        assert_eq!(action.priority, "critical");
        assert!(action.due_date.is_some());
        assert_eq!(action.related_entity_type, Some("company".to_string()));
    }

    // ── Priority Badge Class Tests ─────────────────────────────────────────────

    #[test]
    fn opportunity_card_priority_class_high() {
        let opp = StrategicOpportunity {
            id: "1".to_string(),
            title: "Test".to_string(),
            description: None,
            opportunity_type: "test".to_string(),
            priority_score: 0.85,
            confidence: 0.5,
            entity_id: None,
            entity_type: None,
            region: None,
            estimated_value: None,
            recommended_actions: vec![],
            owner_id: None,
            status: "active".to_string(),
            due_date: None,
        };

        let priority_class = if opp.priority_score > 0.8 {
            "priority-high"
        } else if opp.priority_score > 0.5 {
            "priority-medium"
        } else {
            "priority-low"
        };

        assert_eq!(priority_class, "priority-high");
    }

    #[test]
    fn opportunity_card_priority_class_medium() {
        let opp = StrategicOpportunity {
            id: "1".to_string(),
            title: "Test".to_string(),
            description: None,
            opportunity_type: "test".to_string(),
            priority_score: 0.6,
            confidence: 0.5,
            entity_id: None,
            entity_type: None,
            region: None,
            estimated_value: None,
            recommended_actions: vec![],
            owner_id: None,
            status: "active".to_string(),
            due_date: None,
        };

        let priority_class = if opp.priority_score > 0.8 {
            "priority-high"
        } else if opp.priority_score > 0.5 {
            "priority-medium"
        } else {
            "priority-low"
        };

        assert_eq!(priority_class, "priority-medium");
    }

    #[test]
    fn opportunity_card_priority_class_low() {
        let opp = StrategicOpportunity {
            id: "1".to_string(),
            title: "Test".to_string(),
            description: None,
            opportunity_type: "test".to_string(),
            priority_score: 0.3,
            confidence: 0.5,
            entity_id: None,
            entity_type: None,
            region: None,
            estimated_value: None,
            recommended_actions: vec![],
            owner_id: None,
            status: "active".to_string(),
            due_date: None,
        };

        let priority_class = if opp.priority_score > 0.8 {
            "priority-high"
        } else if opp.priority_score > 0.5 {
            "priority-medium"
        } else {
            "priority-low"
        };

        assert_eq!(priority_class, "priority-low");
    }

    // ── Threat Severity Class Tests ────────────────────────────────────────────

    #[test]
    fn threat_card_severity_class() {
        let threat = CriticalThreat {
            id: "1".to_string(),
            title: "Test Threat".to_string(),
            description: None,
            threat_type: "test".to_string(),
            severity: "critical".to_string(),
            impact_score: 0.9,
            confidence: 0.5,
            entity_id: None,
            entity_type: None,
            region: None,
            mitigation_steps: vec![],
            owner_id: None,
            status: "active".to_string(),
            sla_deadline: None,
        };

        let severity_class = format!("severity-{}", threat.severity.to_lowercase());
        assert_eq!(severity_class, "severity-critical");
    }

    // ── API Envelope Tests ─────────────────────────────────────────────────────

    #[test]
    fn api_envelope_success() {
        let json = r#"{
            "success": true,
            "data": {
                "top_opportunities": [],
                "critical_threats": [],
                "market_intelligence": {
                    "total_opportunities": 0,
                    "total_threats": 0,
                    "high_priority_count": 0,
                    "regions_affected": [],
                    "average_confidence": 0.0
                },
                "recommended_actions": []
            }
        }"#;

        let envelope: ApiEnvelope<ExecutiveSummary> = serde_json::from_str(json).unwrap();
        assert!(envelope.success);
        assert!(envelope.data.is_some());
        assert!(envelope.error.is_none());
    }

    #[test]
    fn api_envelope_error() {
        let json = r#"{
            "success": false,
            "data": null,
            "error": {
                "message": "Resource not found"
            }
        }"#;

        let envelope: ApiEnvelope<ExecutiveSummary> = serde_json::from_str(json).unwrap();
        assert!(!envelope.success);
        assert!(envelope.data.is_none());
        assert!(envelope.error.is_some());
    }

    // ── Market Intelligence Summary Tests ──────────────────────────────────────

    #[test]
    fn market_intelligence_zero_confidence() {
        let summary = MarketIntelligenceSummary {
            total_opportunities: 0,
            total_threats: 0,
            high_priority_count: 0,
            regions_affected: vec![],
            average_confidence: 0.0,
        };

        assert_eq!(summary.average_confidence, 0.0);
        assert!(summary.regions_affected.is_empty());
    }

    #[test]
    fn market_intelligence_full_confidence() {
        let summary = MarketIntelligenceSummary {
            total_opportunities: 5,
            total_threats: 3,
            high_priority_count: 2,
            regions_affected: vec!["NA".to_string(), "EMEA".to_string(), "APAC".to_string()],
            average_confidence: 1.0,
        };

        assert_eq!(summary.average_confidence, 1.0);
        assert_eq!(summary.regions_affected.len(), 3);
    }

    // ── Confidence Clamping Tests ─────────────────────────────────────────────

    #[test]
    fn confidence_clamping_below_zero() {
        let confidence = -0.5f64;
        let clamped = confidence.clamp(0.0, 1.0);
        assert_eq!(clamped, 0.0);
    }

    #[test]
    fn confidence_clamping_above_one() {
        let confidence = 1.5f64;
        let clamped = confidence.clamp(0.0, 1.0);
        assert_eq!(clamped, 1.0);
    }

    #[test]
    fn confidence_clamping_within_range() {
        let confidence = 0.75f64;
        let clamped = confidence.clamp(0.0, 1.0);
        assert_eq!(clamped, 0.75);
    }
}
