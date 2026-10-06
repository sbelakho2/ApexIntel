//! Executive Dashboard handler — GET /executive
//!
//! Covers: C-suite-ready single page with top-5 competitors by mention-volume,
//! top-3 risks, trending topics, recent wins/losses, and recommended actions.
//! Reuses existing [`PgStore`] methods for dashboard stats, strategic opportunities,
//! critical threats, and insights.

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::{response::IntoResponse, Extension};

use super::PageContext;
use crate::middleware::session::WebSession;
use apex_core::data_state::{DataState, DegradedNotice};
use apex_store::postgres::{InsightListFilters, PgStore};

// ─── Display types ───────────────────────────────────────────────────────────

/// A single stat card on the executive dashboard.
#[derive(Debug, Clone)]
pub struct ExecutiveStatCard {
    pub label: String,
    pub value: String,
    pub icon: String,
    pub accent_class: String,
    pub delta: Option<String>,
    pub direction: String,
}

/// A competitor ranked by *mention volume* — attention in tracked reporting,
/// not a strategic risk score. The label deliberately says "mention
/// intensity" so the dashboard never presents visibility as risk.
#[derive(Debug, Clone)]
pub struct CompetitorMention {
    pub name: String,
    pub mention_count: i64,
    pub trend_direction: String,
    /// Relative mention-volume band versus the most-mentioned competitor
    /// (`high` / `medium` / `low`).
    pub mention_intensity: String,
    /// Bar width percentage for visualisation (0–100).
    pub bar_pct: i64,
}

/// A top risk item for the executive view.
#[derive(Debug, Clone)]
pub struct TopRisk {
    pub id: String,
    pub title: String,
    pub severity: String,
    pub company: String,
    pub score: f64,
    pub region: String,
    pub score_display: String,
}

/// A trending topic with week-over-week change.
#[derive(Debug, Clone)]
pub struct TrendingTopic {
    pub topic: String,
    pub mention_count: i64,
    pub change_pct: String,
}

/// A recent win or loss from strategic opportunities.
#[derive(Debug, Clone)]
pub struct RecentWinLoss {
    pub title: String,
    pub result_type: String,
    pub company: String,
    pub date: String,
    pub description: String,
}

/// A recommended action derived from high-priority items.
#[derive(Debug, Clone)]
pub struct RecommendedAction {
    pub id: String,
    pub title: String,
    pub priority: String,
    pub owner: String,
    pub due_date: String,
    pub description: String,
}

// ─── Template struct ─────────────────────────────────────────────────────────

/// Executive dashboard page context.
#[derive(Debug, Clone, Template)]
#[template(path = "pages/executive.html")]
pub struct ExecutiveDashboardPage {
    // Base layout fields
    pub current_path: String,
    pub can_admin: bool,
    pub can_write: bool,
    pub username: String,
    pub warning_count: i64,
    pub theme: String,
    pub status_strip: crate::system_status::StatusStrip,

    // Executive dashboard data
    pub stat_cards: Vec<ExecutiveStatCard>,
    pub competitors: Vec<CompetitorMention>,
    pub top_risks: Vec<TopRisk>,
    pub trending_topics: Vec<TrendingTopic>,
    pub recent_wins_losses: Vec<RecentWinLoss>,
    pub recommended_actions: Vec<RecommendedAction>,
    pub degraded_notice: Option<String>,
}

// ─── Handler ─────────────────────────────────────────────────────────────────

/// Case-insensitive whole-word containment. Tracked competitor names must
/// match as whole words: "Apex" must not match "ApexIntel" (audit #156).
fn contains_whole_word(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    let haystack_lower = haystack.to_lowercase();
    let needle_lower = needle.to_lowercase();
    let mut search_from = 0;
    while let Some(relative) = haystack_lower[search_from..].find(&needle_lower) {
        let start = search_from + relative;
        let end = start + needle_lower.len();
        let before_ok = haystack_lower[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack_lower[end..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        search_from = start + needle_lower.chars().next().map_or(1, char::len_utf8);
        if search_from >= haystack_lower.len() {
            break;
        }
    }
    false
}

/// Whether an insight mentions a tracked competitor: a resolved entity id
/// whose name matches, or a whole-word occurrence in its title, summary, or
/// tags.
fn insight_mentions_competitor(
    insight: &apex_store::postgres::InsightRow,
    competitor: &str,
    entity_names: &HashMap<String, String>,
) -> bool {
    let entity_match = insight.entity_ids.as_ref().is_some_and(|ids| {
        ids.iter().any(|id| {
            entity_names
                .get(&id.to_string())
                .is_some_and(|name| name.eq_ignore_ascii_case(competitor))
        })
    });
    entity_match
        || contains_whole_word(&insight.title, competitor)
        || contains_whole_word(&insight.summary, competitor)
        || insight
            .tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|tag| contains_whole_word(tag, competitor)))
}

/// Classify a stored `strategic_opportunities.status` for the wins/losses
/// panel. The table's `chk_opportunity_status` constraint stores a terminal
/// win as `completed` and a loss as `abandoned`; `won`/`lost` are accepted so
/// the panel keeps working if a deployment widens the constraint. Any other
/// status is still an open opportunity, not a result (audit #156).
fn opportunity_result_type(status: &str) -> &'static str {
    let is_win = status == "won" || status == "completed";
    let is_loss = status == "lost" || status == "abandoned";
    if is_win {
        "win"
    } else if is_loss {
        "loss"
    } else {
        "opportunity"
    }
}

/// GET /executive — render the executive dashboard page.
pub async fn executive_dashboard(
    session: Extension<WebSession>,
    Extension(store): Extension<Arc<PgStore>>,
) -> impl IntoResponse {
    // ── Fetch data sources with graceful degradation ──────────────────────

    let mut degraded_notice: Option<String> = None;

    let stats_state = DataState::from_result(
        store.get_dashboard_stats().await,
        "get_dashboard_stats failed (web executive dashboard)",
        |_| false,
    );
    DegradedNotice::capture(&stats_state, &mut degraded_notice);
    let stats_data = stats_state.into_loaded_or_default();

    let opportunities_state = DataState::from_result(
        store.list_strategic_opportunities(false, 20).await,
        "list_strategic_opportunities failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&opportunities_state, &mut degraded_notice);
    let opportunities = opportunities_state.into_items();

    // Terminal results (`completed`/`abandoned`) are excluded from the open
    // pipeline query above, so the wins/losses panel used to be structurally
    // unable to contain a win (audit #156). Fetch them explicitly.
    let closed_opportunities_state = DataState::from_result(
        store.list_strategic_opportunities(true, 500).await,
        "list_strategic_opportunities (closed) failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&closed_opportunities_state, &mut degraded_notice);
    let closed_opportunities: Vec<_> = closed_opportunities_state
        .into_items()
        .into_iter()
        .filter(|o| opportunity_result_type(&o.status) != "opportunity")
        .collect();

    let threats_state = DataState::from_result(
        store.list_critical_threats(20).await,
        "list_critical_threats failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&threats_state, &mut degraded_notice);
    let threats = threats_state.into_items();

    // Fetch recent insights for trending topics & competitor mention volume
    let insight_filters = InsightListFilters {
        exclude_internal: true,
        ..Default::default()
    };
    let recent_insights_state = DataState::from_result(
        store.list_insights(&insight_filters, 100, 0).await,
        "list_insights failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&recent_insights_state, &mut degraded_notice);
    let recent_insights = recent_insights_state.into_items();

    // B312: competitor names come from the tracked competitor set, not a
    // hardcoded demo list that only matched one specific deployment.

    let competitor_names_state = DataState::from_result(
        store.list_competitors(50, 0).await,
        "list_competitors failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&competitor_names_state, &mut degraded_notice);
    let competitor_names: Vec<String> = competitor_names_state
        .into_items()
        .into_iter()
        .map(|c| c.name)
        .collect();

    // #156: resolve referenced entity ids to names once, so competitor
    // attribution and the risks/wins panels show names, not raw UUIDs.
    let mut entity_ids: Vec<uuid::Uuid> = Vec::new();
    entity_ids.extend(
        recent_insights
            .iter()
            .flat_map(|insight| insight.entity_ids.clone().unwrap_or_default()),
    );
    for raw in threats.iter().map(|t| t.entity_id.as_deref()).chain(
        opportunities
            .iter()
            .map(|o| o.entity_id.as_deref())
            .chain(closed_opportunities.iter().map(|o| o.entity_id.as_deref())),
    ) {
        if let Some(id) = raw.and_then(|value| uuid::Uuid::parse_str(value).ok()) {
            entity_ids.push(id);
        }
    }
    entity_ids.sort();
    entity_ids.dedup();

    let company_entity_state = DataState::from_result(
        store.get_company_names_by_ids(&entity_ids).await,
        "get_company_names_by_ids failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&company_entity_state, &mut degraded_notice);
    let mut entity_names: HashMap<String, String> = HashMap::new();
    for (id, name, _, _) in company_entity_state.into_items() {
        entity_names.insert(id.to_string(), name);
    }
    let person_entity_state = DataState::from_result(
        store.get_person_names_by_ids(&entity_ids).await,
        "get_person_names_by_ids failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&person_entity_state, &mut degraded_notice);
    for (id, name, _) in person_entity_state.into_items() {
        entity_names.entry(id.to_string()).or_insert(name);
    }

    let resolve_entity_name = |raw: Option<&str>| -> Option<String> {
        match raw {
            None => None,
            Some(value) => match uuid::Uuid::parse_str(value) {
                Ok(id) => entity_names.get(&id.to_string()).cloned(),
                // Not a UUID: the stored value is already a human label.
                Err(_) => Some(value.to_string()),
            },
        }
    };

    let unack_warnings = stats_data.unacknowledged_warnings as i64;
    let ctx = PageContext::from_session(&session, "/executive", unack_warnings);

    // ── Build stat cards ─────────────────────────────────────────────────

    // #156: headline counts are SQL COUNT(*)/AVG aggregates from the store,
    // not the length of the capped page (totals previously froze at the page
    // limit and "Avg Confidence" ignored every row beyond it).
    let aggregates_state = DataState::from_result(
        store
            .executive_dashboard_aggregates(true, true, 0.7, None)
            .await,
        "executive_dashboard_aggregates failed (web executive dashboard)",
        |(opportunities, threats, _, _, _)| *opportunities == 0 && *threats == 0,
    );
    DegradedNotice::capture(&aggregates_state, &mut degraded_notice);
    let (total_opportunities, active_threats, high_priority_count, avg_confidence, _regions) =
        aggregates_state.into_loaded_or((0_i64, 0_i64, 0_i64, 0.0_f64, Vec::new()));

    let companies_tracked = stats_data.total_companies;

    let stat_cards = vec![
        ExecutiveStatCard {
            label: "Opportunities".into(),
            value: total_opportunities.to_string(),
            icon: "trending-up".into(),
            accent_class: "metric-rail-green".into(),
            delta: None,
            direction: "flat".into(),
        },
        ExecutiveStatCard {
            label: "Active Threats".into(),
            value: active_threats.to_string(),
            icon: "alert-triangle".into(),
            accent_class: "metric-rail-red".into(),
            delta: (high_priority_count > 0)
                .then(|| format!("{high_priority_count} high priority")),
            direction: "flat".into(),
        },
        ExecutiveStatCard {
            label: "Companies Tracked".into(),
            value: companies_tracked.to_string(),
            icon: "boxes".into(),
            accent_class: "metric-rail-steel".into(),
            delta: None,
            direction: "flat".into(),
        },
        ExecutiveStatCard {
            label: "Avg Confidence".into(),
            value: format!("{:.0}%", avg_confidence * 100.0),
            icon: "bar-chart-3".into(),
            accent_class: "metric-rail-orange".into(),
            delta: None,
            direction: "flat".into(),
        },
    ];

    // ── Build competitor mention-volume ──────────────────────────────────

    // #156: mention volume comes from resolved entity references plus
    // whole-word text matches against the tracked competitor set. The old
    // tag-counting loop summed substring matches and counted the same
    // insight twice (tag pass + text pass).
    let mut sorted_competitors: Vec<(String, i64)> = competitor_names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| {
            let count = recent_insights
                .iter()
                .filter(|insight| insight_mentions_competitor(insight, name, &entity_names))
                .count() as i64;
            (name.to_string(), count)
        })
        .filter(|(_, count)| *count > 0)
        .collect();
    sorted_competitors.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted_competitors.truncate(5);

    let max_mention = sorted_competitors
        .first()
        .map(|(_, c)| *c)
        .unwrap_or(1)
        .max(1);
    let competitors: Vec<CompetitorMention> = sorted_competitors
        .into_iter()
        .map(|(name, count)| {
            let raw_pct = count * 100 / max_mention;
            let bar_pct = raw_pct.clamp(5, 100);
            CompetitorMention {
                name,
                mention_count: count,
                // B313: no fabricated trend — direction is unknown without a
                // comparison window; the template renders a neutral state.
                trend_direction: "flat".into(),
                // Relative-to-top volume bands (share of the noisiest
                // competitor), not absolute mention counts.
                mention_intensity: if raw_pct >= 67 {
                    "high".into()
                } else if raw_pct >= 34 {
                    "medium".into()
                } else {
                    "low".into()
                },
                bar_pct,
            }
        })
        .collect();

    // ── Build top-3 risks from critical threats ──────────────────────────

    let mut sorted_threats: Vec<_> = threats.into_iter().collect();
    sorted_threats.sort_by(|a, b| {
        b.impact_score
            .partial_cmp(&a.impact_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted_threats.truncate(3);

    let top_risks: Vec<TopRisk> = sorted_threats
        .into_iter()
        .map(|t| TopRisk {
            id: t.id.to_string(),
            title: t.title.clone(),
            severity: t.severity.clone(),
            company: resolve_entity_name(t.entity_id.as_deref())
                .unwrap_or_else(|| "Unknown".into()),
            score: t.impact_score,
            region: t.region.clone().unwrap_or_default(),
            score_display: format!("{:.1}", t.impact_score),
        })
        .collect();

    // ── Build trending topics ────────────────────────────────────────────

    // #156: topics are aggregated in SQL over the two-week window (tags via
    // `unnest` plus non-LLM insight types), instead of scanning one capped
    // page of insights in Rust and reporting counts frozen at that cap.
    let now = chrono::Utc::now();
    let week_ago = now - chrono::Duration::days(7);
    let two_weeks_ago = now - chrono::Duration::days(14);
    let trend_rows_state = DataState::from_result(
        sqlx::query_as::<_, (String, i64, i64)>(
            r#"
            WITH topic_rows AS (
                SELECT tag AS topic, i.created_at
                FROM insights i
                CROSS JOIN LATERAL unnest(COALESCE(i.tags, ARRAY[]::text[])) AS tag
                WHERE i.created_at >= $1
                  AND COALESCE(i.metadata->>'retracted','false') <> 'true'
                UNION ALL
                SELECT i.insight_type AS topic, i.created_at
                FROM insights i
                WHERE i.created_at >= $1
                  AND i.insight_type IS NOT NULL
                  AND i.insight_type NOT LIKE 'llm_%'
                  AND COALESCE(i.metadata->>'retracted','false') <> 'true'
            ),
            filtered AS (
                SELECT topic, created_at
                FROM topic_rows
                WHERE topic IS NOT NULL
                  AND btrim(topic) <> ''
                  AND NOT EXISTS (
                      SELECT 1 FROM unnest($3::text[]) AS competitor(name)
                      WHERE lower(btrim(competitor.name)) = lower(btrim(topic))
                  )
            )
            SELECT topic,
                   COUNT(*) FILTER (WHERE created_at >= $2)::bigint AS this_week,
                   COUNT(*) FILTER (WHERE created_at < $2)::bigint AS prev_week
            FROM filtered
            GROUP BY topic
            ORDER BY COUNT(*) DESC, topic ASC
            LIMIT 5
            "#,
        )
        .bind(two_weeks_ago)
        .bind(week_ago)
        .bind(competitor_names.clone())
        .fetch_all(&store.pool)
        .await,
        "topic trend query failed (web executive dashboard)",
        Vec::is_empty,
    );
    DegradedNotice::capture(&trend_rows_state, &mut degraded_notice);
    let trending_topics: Vec<TrendingTopic> = trend_rows_state
        .into_items()
        .into_iter()
        .map(|(topic, this_week, prev_week)| {
            let change_pct = if prev_week == 0 {
                if this_week == 0 {
                    "±0%".to_string()
                } else {
                    "new".to_string()
                }
            } else {
                let delta =
                    ((this_week - prev_week) as f64 / prev_week as f64 * 100.0).round() as i64;
                format!("{}{}%", if delta >= 0 { "+" } else { "" }, delta)
            };
            TrendingTopic {
                topic,
                mention_count: this_week,
                change_pct,
            }
        })
        .collect();

    // ── Build recent wins/losses from strategic opportunities ────────────

    let recent_wins_losses: Vec<RecentWinLoss> = {
        let mut terminal = closed_opportunities;
        // Most recently updated result first; the panel is "recent".
        terminal.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
        terminal.truncate(5);
        terminal
            .into_iter()
            .map(|o| RecentWinLoss {
                title: o.title.clone(),
                result_type: opportunity_result_type(&o.status).to_string(),
                company: resolve_entity_name(o.entity_id.as_deref()).unwrap_or_default(),
                date: o.updated_at.format("%Y-%m-%d").to_string(),
                description: o.description.clone().unwrap_or_default(),
            })
            .collect()
    };

    // ── Build recommended actions from high-priority items ───────────────

    // B313: priority_score is validated to [0,1] on write, so the previous
    // `>= 7.0` / `>= 8.0` cutoffs filtered out every opportunity and the
    // recommended-actions module was permanently empty.
    let high_priority_opps: Vec<_> = opportunities
        .iter()
        .filter(|o| o.priority_score >= 0.7)
        .take(3)
        .collect();

    let recommended_actions: Vec<RecommendedAction> = high_priority_opps
        .into_iter()
        .map(|o| RecommendedAction {
            id: o.id.to_string(),
            title: o.title.clone(),
            priority: if o.priority_score >= 0.8 {
                "critical".into()
            } else {
                "high".into()
            },
            owner: o.owner_id.clone().unwrap_or_else(|| "Unassigned".into()),
            due_date: o
                .due_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "No deadline".into()),
            description: o.description.clone().unwrap_or_default(),
        })
        .collect();

    let page = ExecutiveDashboardPage {
        current_path: ctx.current_path,
        can_admin: ctx.can_admin,
        can_write: ctx.can_write,
        status_strip: crate::system_status::StatusStrip::current(),
        username: ctx.username,
        warning_count: ctx.warning_count,
        theme: ctx.theme,
        stat_cards,
        competitors,
        top_risks,
        trending_topics,
        recent_wins_losses,
        recommended_actions,
        degraded_notice,
    };

    super::render_template(&page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_status::StatusStrip;

    /// The stored status vocabulary decides the result: `completed` is the
    /// table's terminal win, `abandoned` its terminal loss, and anything else
    /// is still an open opportunity (audit #156).
    #[test]
    fn stored_terminal_statuses_classify_as_win_or_loss() {
        assert_eq!(opportunity_result_type("completed"), "win");
        assert_eq!(opportunity_result_type("won"), "win");
        assert_eq!(opportunity_result_type("abandoned"), "loss");
        assert_eq!(opportunity_result_type("lost"), "loss");
        assert_eq!(opportunity_result_type("active"), "opportunity");
        assert_eq!(opportunity_result_type("pursued"), "opportunity");
    }

    fn win_loss(title: &str, result_type: &str) -> RecentWinLoss {
        RecentWinLoss {
            title: title.into(),
            result_type: result_type.into(),
            company: String::new(),
            date: "2026-01-02".into(),
            description: String::new(),
        }
    }

    fn page(recent_wins_losses: Vec<RecentWinLoss>) -> ExecutiveDashboardPage {
        ExecutiveDashboardPage {
            current_path: "/executive".into(),
            can_admin: false,
            can_write: false,
            username: "analyst".into(),
            warning_count: 0,
            theme: String::new(),
            status_strip: StatusStrip::unknown(),
            stat_cards: vec![],
            competitors: vec![],
            top_risks: vec![],
            trending_topics: vec![],
            recent_wins_losses,
            recommended_actions: vec![],
            degraded_notice: None,
        }
    }

    /// A stored win and a stored loss both render in the wins/losses panel —
    /// before the fix the panel only ever emitted "opportunity".
    #[test]
    fn wins_losses_panel_renders_a_win_and_a_loss() {
        let html = page(vec![
            win_loss("Renewal signed", "win"),
            win_loss("Pilot cancelled", "loss"),
        ])
        .render()
        .expect("executive dashboard renders");

        assert!(html.contains("Recent Wins &amp; Losses"));
        let win_item = rendered_item(&html, "Renewal signed");
        assert!(win_item.contains("win"), "win badge missing: {win_item}");
        assert!(!win_item.contains("opportunity"));
        let loss_item = rendered_item(&html, "Pilot cancelled");
        assert!(
            loss_item.contains("loss"),
            "loss badge missing: {loss_item}"
        );
        assert!(!loss_item.contains("win"));
    }

    // ── #156: whole-word competitor matching ─────────────────────────────

    #[test]
    fn whole_word_match_does_not_match_substrings() {
        assert!(contains_whole_word("Acme Corp wins a deal", "Acme"));
        assert!(contains_whole_word("Acme-Corp expands", "Acme"));
        assert!(contains_whole_word("the ACME deal", "acme"));
        assert!(contains_whole_word("Acme", "Acme"));
        assert!(!contains_whole_word("ApexIntel filing", "Apex"));
        assert!(!contains_whole_word("MyAcmeCorp", "Acme"));
        assert!(!contains_whole_word("anything", "  "));
    }

    #[test]
    fn competitor_mentions_use_resolved_entity_names() {
        let entity_id = uuid::Uuid::new_v4();
        let mut insight = insight_row();
        insight.entity_ids = Some(vec![entity_id]);
        insight.title = "Quarterly update".to_string();
        insight.summary = "No company named here".to_string();
        insight.tags = None;

        let mut names = HashMap::new();
        names.insert(entity_id.to_string(), "Acme Corp".to_string());

        assert!(insight_mentions_competitor(&insight, "Acme Corp", &names));
        assert!(!insight_mentions_competitor(&insight, "Other Corp", &names));
    }

    #[test]
    fn competitor_mentions_ignore_substring_text_matches() {
        let mut insight = insight_row();
        insight.title = "ApexIntel ships a product".to_string();
        insight.summary = "MyAcmeCorp response".to_string();
        insight.tags = Some(vec!["ApexIntel".to_string()]);
        insight.entity_ids = None;

        let names = HashMap::new();
        assert!(!insight_mentions_competitor(&insight, "Apex", &names));
        assert!(!insight_mentions_competitor(&insight, "Acme", &names));
        assert!(insight_mentions_competitor(&insight, "ApexIntel", &names));
    }

    fn insight_row() -> apex_store::postgres::InsightRow {
        apex_store::postgres::InsightRow {
            id: uuid::Uuid::new_v4(),
            title: String::new(),
            summary: String::new(),
            insight_type: Some("market_shift".to_string()),
            region: None,
            confidence: None,
            evidence_urls: None,
            entity_ids: None,
            tags: None,
            metadata: None,
            created_at: None,
            updated_at: None,
        }
    }

    /// Text of the rendered list item starting at `title` up to the item's
    /// closing `</div>`.
    fn rendered_item<'a>(html: &'a str, title: &str) -> &'a str {
        let after = html
            .split_once(title)
            .unwrap_or_else(|| panic!("item title {title:?} did not render"))
            .1;
        let end = after.find("</div>").unwrap_or(after.len());
        &after[..end]
    }
}
