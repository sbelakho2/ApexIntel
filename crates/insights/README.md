# apex-insights

Company dossier generation, person profile building, and weekly strategy memo assembly.

## Responsibilities

- **Dossier** (`dossier.rs`): Generate structured intelligence dossiers from domain entities.
  - `generate_company_dossier(company, sites, capabilities, certs)` — full company dossier with capability, risk, opportunity, and competitive assessment sections.
  - `generate_poi_dossier(person, artifacts, org_name)` — full POI dossier with priority analysis, influence assessment, artifact summary, and approach guidance.
  - `render_company_dossier_text(dossier)` / `render_poi_dossier_text(dossier)` — plain-text rendering for the memo and LLM context window.
- **Renderer** (`renderer.rs`): Insight card generation from recipe candidates.
  - `render_insight(candidate)` — generates a finished `InsightCard` from evidence-backed templates.
  - `rank_insights(cards)` — order by priority score (impact × confidence × severity weight).
  - `filter_by_confidence`, `filter_by_category`, `group_by_category`, `group_by_region`.
  - `impact_label(impact)` — human-readable label: `critical / high / medium / low`.
  - `priority_score(impact, confidence, severity)` — composite 0–1 score.
- **Memo** (`memo.rs`): Weekly intelligence brief assembly.
  - `generate_weekly_memo(cards)` — full `WeeklyMemo` from card list.
  - `generate_executive_summary(inputs)` — condensed summary section.
  - `build_regional_sections(cards, max)` — card grouping by region.
  - `count_by_severity(cards)` — (critical, high, medium) counters.
  - `region_label(code)` — human-readable region name from ISO code.

## Key output types

| Type | Description |
|------|-------------|
| `CompanyDossier` | Full company intelligence package |
| `PoiDossier` | Full person intelligence package |
| `InsightCard` | Rendered insight ready for API delivery |
| `WeeklyMemo` | Weekly strategy brief with sections |
