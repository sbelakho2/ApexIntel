use leptos::*;

#[component]
pub fn ActivityPage() -> impl IntoView {
    let (events, set_events) = create_signal::<Vec<crate::api::ActivityEvent>>(vec![]);
    let (loading, set_loading) = create_signal(true);
    let (error, set_error) = create_signal::<Option<String>>(None);
    let (filter, set_filter) = create_signal("all".to_string());

    let fetch_events = move || {
        set_loading.set(true);
        set_error.set(None);
        spawn_local(async move {
            match crate::api::fetch_activity_feed(50, 0).await {
                Ok(data) => {
                    set_events.set(data);
                    set_loading.set(false);
                }
                Err(e) => {
                    set_error.set(Some(format!("Failed to load activity: {}", e)));
                    set_loading.set(false);
                }
            }
        });
    };

    create_effect(move |_| {
        fetch_events();
    });

    let filtered_events = move || {
        let f = filter.get();
        events
            .get()
            .into_iter()
            .filter(move |e| f == "all" || e.event_type.to_lowercase() == f)
            .collect::<Vec<_>>()
    };

    let event_icon = |event_type: &str| -> &'static str {
        match event_type {
            "crawl" | "crawl_complete" | "crawl_started" => "🕷️",
            "insight" | "insight_generated" | "insight_updated" => "💡",
            "warning" | "warning_created" | "warning_resolved" => "⚠️",
            "poi" | "poi_updated" | "poi_detected" | "poi_change" => "👤",
            "company" | "company_updated" | "company_added" => "🏢",
            "recipe" | "recipe_complete" | "recipe_started" => "📋",
            "triage" | "triage_complete" | "triage_assigned" => "🔍",
            "threat" | "threat_detected" | "threat_resolved" => "🛡️",
            "system" | "system_event" => "⚙️",
            "user" | "user_action" | "user_login" => "👋",
            "comment" | "annotation" => "💬",
            "share" | "shared" => "📤",
            "assign" | "assigned" => "👥",
            "memo" | "memo_created" => "📝",
            "dossier" | "dossier_updated" => "📁",
            _ => "📌",
        }
    };

    let time_ago = |ts: &str| -> String {
        match chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.f") {
            Ok(dt) => {
                let now = chrono::Utc::now().naive_utc();
                let duration = now.signed_duration_since(dt);
                if duration.num_seconds() < 60 {
                    "just now".to_string()
                } else if duration.num_minutes() < 60 {
                    format!("{}m ago", duration.num_minutes())
                } else if duration.num_hours() < 24 {
                    format!("{}h ago", duration.num_hours())
                } else if duration.num_days() < 7 {
                    format!("{}d ago", duration.num_days())
                } else if duration.num_days() < 30 {
                    format!("{}w ago", duration.num_days() / 7)
                } else {
                    let dt_utc =
                        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc);
                    dt_utc.format("%b %d, %Y").to_string()
                }
            }
            Err(_) => ts.chars().take(10).collect::<String>(),
        }
    };

    let filter_labels: Vec<(&str, &str)> = vec![
        ("all", "All Activity"),
        ("insight", "Insights"),
        ("warning", "Warnings"),
        ("crawl", "Crawls"),
        ("poi", "POIs"),
        ("threat", "Threats"),
        ("system", "System"),
        ("user", "User"),
    ];

    view! {
        <div class="page activity-page">
            <div class="page-header">
                <div class="page-header-content">
                    <div>
                        <h1 class="page-title">Activity Feed</h1>
                        <p class="page-subtitle">Real-time intelligence gathering activity, system events, and user actions</p>
                    </div>
                    <button class="btn btn-secondary btn-sm" on:click=move |_| fetch_events() aria-label="Refresh activity feed">
                        <svg class="icon-sm" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
                            <polyline points="23 4 23 10 17 10"/>
                            <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
                        </svg>
                        Refresh
                    </button>
                </div>
            </div>

            <div class="filter-bar" role="toolbar" aria-label="Activity filters">
                {filter_labels.iter().map(|(type_val, label)| {
                    let type_val = type_val.to_string();
                    let label = label.to_string();
                    let f = filter;
                    let type_val_clone = type_val.clone();
                    view! {
                        <button
                            class=move || {
                                let active = if f.get() == type_val_clone { "filter-chip-active" } else { "" };
                                format!("filter-chip {}", active)
                            }
                            on:click={
                                let tv = type_val.clone();
                                move |_| set_filter.set(tv.clone())
                            }
                            aria-pressed={
                                let tv = type_val.clone();
                                move || (filter.get() == tv).to_string()
                            }
                        >
                            {label.clone()}
                        </button>
                    }
                }).collect_view()}
            </div>

            {move || match (loading.get(), error.get()) {
                (true, _) => view! {
                    <div class="loading-state">
                        <div class="spinner" aria-label="Loading activity feed"></div>
                        <p>Loading activity...</p>
                    </div>
                }.into_view(),
                (_, Some(err)) => view! {
                    <div class="error-state" role="alert">
                        <svg class="error-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
                            <circle cx="12" cy="12" r="10"/>
                            <line x1="12" y1="8" x2="12" y2="12"/>
                            <line x1="12" y1="16" x2="12.01" y2="16"/>
                        </svg>
                        <p>{err}</p>
                        <button class="btn btn-secondary" on:click=move |_| fetch_events()>Retry</button>
                    </div>
                }.into_view(),
                (false, None) if filtered_events().is_empty() => view! {
                    <div class="empty-state">
                        <svg class="empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
                            <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
                        </svg>
                        <h3>No Activity Yet</h3>
                        <p>Activity will appear here as the system crawls sources, generates insights, and processes intelligence.</p>
                    </div>
                }.into_view(),
                (false, None) => view! {
                    <div class="activity-timeline" role="feed" aria-label="Activity feed">
                        {filtered_events().into_iter().map(|event| view! {
                            <article class="activity-event" role="article" aria-label=format!("{}: {}", event.event_type, event.title)>
                                <div class="activity-event-icon" aria-hidden="true">
                                    {event_icon(&event.event_type)}
                                </div>
                                <div class="activity-event-content">
                                    <div class="activity-event-header">
                                        <span class=format!("badge badge-event-{}", event.event_type.to_lowercase())>
                                            {event.event_type.clone()}
                                        </span>
                                        <span class="activity-event-severity" aria-label=format!("Severity: {}", event.severity)>
                                            {if !event.severity.is_empty() {
                                                view! { <span class=format!("dot dot-{}", event.severity.to_lowercase())></span> }
                                            } else {
                                                view! { <span></span> }
                                            }}
                                        </span>
                                    </div>
                                    <h4 class="activity-event-title">{event.title.clone()}</h4>
                                    {event.description.clone().map(|desc| view! {
                                        <p class="activity-event-description">{desc}</p>
                                    })}
                                    <div class="activity-event-meta">
                                        <span class="activity-event-time">{time_ago(&event.timestamp)}</span>
                                        {event.entity_name.clone().map(|name| view! {
                                            <span class="activity-event-entity">{name}</span>
                                        })}
                                        {event.source.clone().map(|src| view! {
                                            <span class="activity-event-source">via {src}</span>
                                        })}
                                    </div>
                                </div>
                            </article>
                        }).collect_view()}
                    </div>
                }.into_view(),
            }}
        </div>
    }
}
