//! Battlecard response type and the helpers shared by the web UI and the
//! JSON API (status vocabulary, title validation, Markdown export,
//! comparison selection).

use serde::Serialize;
use uuid::Uuid;

// ─── Response Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct BattlecardResponse {
    pub id: Uuid,
    pub our_company_id: Uuid,
    pub competitor_id: Uuid,
    pub title: String,
    pub status: String,
    pub sections: std::collections::HashMap<String, Option<serde_json::Value>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub regenerated_at: Option<chrono::DateTime<chrono::Utc>>,
}

// ─── Shared helpers (web UI + JSON API) ────────────────────────────────────

/// Battlecard JSONB sections in display order with their human labels.
pub const SECTIONS: &[(&str, &str)] = &[
    ("positioning", "Positioning"),
    ("pricing", "Pricing"),
    ("feature_matrix", "Feature Matrix"),
    ("strengths", "Strengths"),
    ("weaknesses", "Weaknesses"),
    ("objection_handlers", "Objection Handlers"),
    ("kill_shots", "Kill Shots"),
    ("recent_news", "Recent News"),
    ("win_loss", "Win / Loss"),
];

/// Minimum and maximum number of battlecards on the comparison page.
pub const COMPARE_MIN: usize = 2;
pub const COMPARE_MAX: usize = 4;

/// Normalise a user-supplied status to the canonical stored value.
pub fn normalize_status(raw: &str) -> Option<&'static str> {
    let wanted = raw.trim().to_ascii_lowercase();
    apex_store::postgres::BATTLECARD_STATUSES
        .iter()
        .copied()
        .find(|status| *status == wanted)
}

/// Validate and trim a battlecard title.
pub fn validate_title(raw: &str) -> Result<String, &'static str> {
    let title = raw.trim();
    if title.is_empty() {
        return Err("title is required");
    }
    if title.chars().count() > apex_store::postgres::BATTLECARD_TITLE_MAX_CHARS {
        return Err("title must be at most 255 characters");
    }
    Ok(title.to_string())
}

/// Borrow a battlecard section by name.
pub fn section_value<'a>(
    row: &'a apex_store::postgres::BattlecardRow,
    name: &str,
) -> Option<&'a serde_json::Value> {
    match name {
        "positioning" => row.positioning.as_ref(),
        "pricing" => row.pricing.as_ref(),
        "feature_matrix" => row.feature_matrix.as_ref(),
        "strengths" => row.strengths.as_ref(),
        "weaknesses" => row.weaknesses.as_ref(),
        "objection_handlers" => row.objection_handlers.as_ref(),
        "kill_shots" => row.kill_shots.as_ref(),
        "recent_news" => row.recent_news.as_ref(),
        "win_loss" => row.win_loss.as_ref(),
        _ => None,
    }
    .filter(|value| has_content(value))
}

/// Whether a JSON section carries any displayable content.
pub fn has_content(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(s) => !s.trim().is_empty(),
        serde_json::Value::Array(items) => items.iter().any(has_content),
        serde_json::Value::Object(map) => map.values().any(has_content),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

fn humanize_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for (i, word) in key.split('_').filter(|w| !w.is_empty()).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if i == 0 {
                out.extend(first.to_uppercase());
            } else {
                out.push(first);
            }
            out.push_str(chars.as_str());
        }
    }
    out
}

fn scalar_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.trim().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(if *b { "yes" } else { "no" }.to_string()),
        _ => None,
    }
}

/// One-line plain-text rendering of a JSON section (used for compare cells
/// and editor previews).
pub fn section_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(section_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("; "),
        serde_json::Value::Object(map) => map
            .iter()
            .filter(|(_, v)| has_content(v))
            .map(|(k, v)| format!("{}: {}", humanize_key(k), section_text(v)))
            .collect::<Vec<_>>()
            .join(", "),
        other => scalar_text(other).unwrap_or_default(),
    }
}

/// Plain-text lines for a section: one per array item or object field
/// (used by the detail page).
pub fn section_lines(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter(|v| has_content(v))
            .map(section_text)
            .collect(),
        serde_json::Value::Object(map) => map
            .iter()
            .filter(|(_, v)| has_content(v))
            .map(|(k, v)| format!("{}: {}", humanize_key(k), section_text(v)))
            .collect(),
        other => scalar_text(other)
            .filter(|text| !text.is_empty())
            .into_iter()
            .collect(),
    }
}

/// Truncate `text` to at most `max_chars` characters on a char boundary.
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn push_markdown(out: &mut String, value: &serde_json::Value, depth: usize) {
    let indent = "  ".repeat(depth);
    match value {
        serde_json::Value::Array(items) => {
            for item in items.iter().filter(|v| has_content(v)) {
                match item {
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        out.push_str(&format!("{indent}- {}\n", section_text(item)));
                    }
                    other => {
                        out.push_str(&format!(
                            "{indent}- {}\n",
                            scalar_text(other).unwrap_or_default()
                        ));
                    }
                }
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map.iter().filter(|(_, v)| has_content(v)) {
                match scalar_text(item) {
                    Some(text) => {
                        out.push_str(&format!("{indent}- **{}:** {text}\n", humanize_key(key)));
                    }
                    None => {
                        out.push_str(&format!("{indent}- **{}:**\n", humanize_key(key)));
                        push_markdown(out, item, depth + 1);
                    }
                }
            }
        }
        other => {
            if let Some(text) = scalar_text(other) {
                out.push_str(&format!("{indent}{text}\n"));
            }
        }
    }
}

/// Render a battlecard as a Markdown document.
pub fn render_markdown(
    row: &apex_store::postgres::BattlecardRow,
    competitor_name: Option<&str>,
    our_company_name: Option<&str>,
) -> String {
    let mut out = format!("# {}\n\n", row.title.trim());
    out.push_str(&format!(
        "- **Competitor:** {}\n",
        competitor_name.unwrap_or("Unknown company")
    ));
    out.push_str(&format!(
        "- **Our company:** {}\n",
        our_company_name.unwrap_or("Unknown company")
    ));
    out.push_str(&format!("- **Status:** {}\n", row.status));
    out.push_str(&format!(
        "- **Updated:** {}\n",
        row.updated_at.format("%Y-%m-%d %H:%M UTC")
    ));
    if let Some(regenerated) = row.regenerated_at {
        out.push_str(&format!(
            "- **Regenerated:** {}\n",
            regenerated.format("%Y-%m-%d %H:%M UTC")
        ));
    }
    let mut any = false;
    for (name, label) in SECTIONS {
        if let Some(value) = section_value(row, name) {
            any = true;
            out.push_str(&format!("\n## {label}\n\n"));
            push_markdown(&mut out, value, 0);
        }
    }
    if !any {
        out.push_str("\n_No sections have been generated yet._\n");
    }
    out
}

/// Render a side-by-side comparison of several battlecards as a Markdown
/// table. `cards` pairs each row with its display name.
pub fn render_comparison_markdown(
    cards: &[(&apex_store::postgres::BattlecardRow, String)],
) -> String {
    let escape = |s: &str| s.replace('|', "\\|").replace(['\n', '\r'], " ");
    let mut out = String::from("# Battlecard comparison\n\n| Section |");
    for (_, name) in cards {
        out.push_str(&format!(" {} |", escape(name)));
    }
    out.push_str("\n|---|");
    for _ in cards {
        out.push_str("---|");
    }
    out.push('\n');
    for (name, label) in SECTIONS {
        out.push_str(&format!("| {label} |"));
        for (row, _) in cards {
            let cell = section_value(row, name)
                .map(|v| escape(&section_text(v)))
                .unwrap_or_else(|| "—".to_string());
            out.push_str(&format!(" {cell} |"));
        }
        out.push('\n');
    }
    out
}

/// Parse the comparison selection from a raw query string. Accepts both
/// `ids=a,b` and repeated `ids=a&ids=b`; duplicates are dropped in order.
pub fn parse_compare_ids(raw_query: Option<&str>) -> Result<Vec<Uuid>, &'static str> {
    let mut ids: Vec<Uuid> = Vec::new();
    for (key, value) in url::form_urlencoded::parse(raw_query.unwrap_or("").as_bytes()) {
        if key != "ids" {
            continue;
        }
        for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let id = Uuid::parse_str(part).map_err(|_| "invalid battlecard id")?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.len() < COMPARE_MIN {
        return Err("select at least two battlecards to compare");
    }
    if ids.len() > COMPARE_MAX {
        return Err("select at most four battlecards to compare");
    }
    Ok(ids)
}

/// Resolve display names for the given company ids (missing ids are absent).
pub async fn company_names(
    store: &apex_store::postgres::PgStore,
    ids: &[Uuid],
) -> anyhow::Result<std::collections::HashMap<Uuid, String>> {
    let mut unique: Vec<Uuid> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    Ok(store
        .get_company_names_by_ids(&unique)
        .await?
        .into_iter()
        .map(|(id, name, _region, _company_type)| (id, name))
        .collect())
}

/// Build a safe `Content-Disposition` filename stem from a title.
pub fn export_filename(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = truncate_chars(&slug, 80)
        .trim_end_matches(['…', '-'])
        .to_string();
    if slug.is_empty() {
        "battlecard".to_string()
    } else {
        format!("battlecard-{slug}")
    }
}

impl From<apex_store::postgres::BattlecardRow> for BattlecardResponse {
    fn from(row: apex_store::postgres::BattlecardRow) -> Self {
        let mut sections = std::collections::HashMap::new();
        sections.insert("positioning".to_string(), row.positioning);
        sections.insert("pricing".to_string(), row.pricing);
        sections.insert("feature_matrix".to_string(), row.feature_matrix);
        sections.insert("strengths".to_string(), row.strengths);
        sections.insert("weaknesses".to_string(), row.weaknesses);
        sections.insert("objection_handlers".to_string(), row.objection_handlers);
        sections.insert("kill_shots".to_string(), row.kill_shots);
        sections.insert("recent_news".to_string(), row.recent_news);
        sections.insert("win_loss".to_string(), row.win_loss);

        Self {
            id: row.id,
            our_company_id: row.our_company_id,
            competitor_id: row.competitor_id,
            title: row.title,
            status: row.status,
            sections,
            created_at: row.created_at,
            updated_at: row.updated_at,
            regenerated_at: row.regenerated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_store::postgres::BattlecardRow;
    use serde_json::json;

    fn row() -> BattlecardRow {
        let now = chrono::Utc::now();
        BattlecardRow {
            id: Uuid::nil(),
            our_company_id: Uuid::nil(),
            competitor_id: Uuid::nil(),
            title: "Us vs Them".to_string(),
            status: "draft".to_string(),
            positioning: Some(json!({"summary": "We win on price", "segment": ""})),
            pricing: None,
            feature_matrix: None,
            strengths: Some(json!(["Fast delivery", {"point": "Big | team"}])),
            weaknesses: Some(json!([])),
            objection_handlers: None,
            kill_shots: None,
            recent_news: None,
            win_loss: Some(json!({"win_rate": 0.5, "notes": {"top": "price"}})),
            created_at: now,
            updated_at: now,
            updated_by: None,
            regenerated_at: None,
        }
    }

    #[test]
    fn normalize_status_accepts_only_canonical_values() {
        assert_eq!(normalize_status(" Published "), Some("published"));
        assert_eq!(normalize_status("draft"), Some("draft"));
        assert_eq!(normalize_status("active"), None);
        assert_eq!(normalize_status(""), None);
    }

    #[test]
    fn validate_title_trims_and_bounds() {
        assert_eq!(validate_title("  A  ").as_deref(), Ok("A"));
        assert!(validate_title("   ").is_err());
        assert!(validate_title(&"x".repeat(256)).is_err());
        assert!(validate_title(&"é".repeat(255)).is_ok());
    }

    #[test]
    fn empty_sections_are_not_content() {
        let r = row();
        assert!(section_value(&r, "weaknesses").is_none());
        assert!(section_value(&r, "pricing").is_none());
        assert!(section_value(&r, "strengths").is_some());
        assert!(section_value(&r, "bogus").is_none());
    }

    #[test]
    fn markdown_renders_sections_not_debug_output() {
        let md = render_markdown(&row(), Some("Them Inc"), Some("Us Ltd"));
        assert!(md.starts_with("# Us vs Them\n"));
        assert!(md.contains("- **Competitor:** Them Inc"));
        assert!(md.contains("## Positioning\n\n- **Summary:** We win on price\n"));
        assert!(md.contains("## Strengths\n\n- Fast delivery\n- Point: Big | team\n"));
        assert!(md.contains("- **Notes:**\n  - **Top:** price\n"));
        assert!(!md.contains("Some("), "debug formatting leaked: {md}");
        assert!(!md.contains("## Weaknesses"));
    }

    #[test]
    fn section_lines_flatten_one_line_per_entry() {
        let r = row();
        assert_eq!(
            section_lines(r.strengths.as_ref().unwrap()),
            vec!["Fast delivery".to_string(), "Point: Big | team".to_string()]
        );
        assert_eq!(
            section_lines(r.positioning.as_ref().unwrap()),
            vec!["Summary: We win on price".to_string()]
        );
        assert_eq!(
            section_lines(&json!("  plain  ")),
            vec!["plain".to_string()]
        );
        assert!(section_lines(&json!(null)).is_empty());
        assert!(section_lines(&json!([" ", null])).is_empty());
    }

    #[test]
    fn markdown_without_sections_says_so() {
        let mut r = row();
        r.positioning = None;
        r.strengths = None;
        r.win_loss = None;
        let md = render_markdown(&r, None, None);
        assert!(md.contains("No sections have been generated yet"));
        assert!(md.contains("Unknown company"));
    }

    #[test]
    fn comparison_markdown_escapes_pipes() {
        let a = row();
        let b = row();
        let md = render_comparison_markdown(&[(&a, "A|1".to_string()), (&b, "B".to_string())]);
        assert!(md.contains("| Section | A\\|1 | B |"));
        assert!(md.contains("Big \\| team"));
        assert!(md.contains("| Pricing | — | — |"));
    }

    #[test]
    fn parse_compare_ids_accepts_both_encodings() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert_eq!(
            parse_compare_ids(Some(&format!("ids={a},{b}"))).unwrap(),
            vec![a, b]
        );
        assert_eq!(
            parse_compare_ids(Some(&format!("ids={a}&ids={b}&ids={a}"))).unwrap(),
            vec![a, b]
        );
        assert!(parse_compare_ids(Some(&format!("ids={a}"))).is_err());
        assert!(parse_compare_ids(Some("ids=nope,also-nope")).is_err());
        assert!(parse_compare_ids(None).is_err());
        let many: Vec<String> = (0..5).map(|_| Uuid::new_v4().to_string()).collect();
        assert!(parse_compare_ids(Some(&format!("ids={}", many.join(",")))).is_err());
    }

    #[test]
    fn export_filename_is_header_safe() {
        assert_eq!(export_filename("Us vs Them!"), "battlecard-us-vs-them");
        assert_eq!(export_filename("\"\r\n;"), "battlecard");
        assert!(export_filename(&"a".repeat(300)).len() <= "battlecard-".len() + 80);
    }
}
