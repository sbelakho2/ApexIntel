use apex_shared::{BayesianInterpretation, CalibrationCurve, ConfidenceInterval};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApiErrorBody {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApiEnvelope<T> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<ApiErrorBody>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PagedResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub per_page: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DashboardSeverity {
    pub severity: String,
    pub count: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DashboardStats {
    pub total_companies: u64,
    pub total_persons: u64,
    pub total_warnings: u64,
    pub unacknowledged_warnings: u64,
    pub total_insights: u64,
    pub active_recipes: u64,
    pub new_insights_24h: u64,
    pub new_warnings_24h: u64,
    pub threat_distribution: Vec<DashboardSeverity>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WarningRecord {
    pub id: String,
    pub title: String,
    pub description: String,
    pub severity: String,
    pub warning_type: String,
    pub region: String,
    pub confidence: f64,
    pub calibrated_probability: Option<f64>,
    pub bayesian_interpretation: Option<BayesianInterpretation>,
    pub confidence_interval: Option<ConfidenceInterval>,
    pub evidence_quality_label: Option<String>,
    pub information_gain_bits: Option<f64>,
    pub acknowledged: bool,
    pub ts_utc: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct InsightRecord {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub insight_type: String,
    pub region: String,
    pub confidence: f64,
    pub tags: Vec<String>,
    pub information_gain_bits: Option<f64>,
    pub information_gain_sparkline: Vec<f64>,
    pub diversity_score: Option<f64>,
    pub diversity_label: Option<String>,
    pub causal_flag: Option<String>,
    pub created_at: String,
    pub bookmarked: Option<bool>,
    pub quality_score: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CompanyListItem {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub region: String,
    pub country: String,
    pub entity_type: String,
    pub is_competitor: bool,
    pub threat_score: Option<f64>,
    pub capabilities: Vec<String>,
    pub community_badges: Vec<String>,
    pub source_entropy: Option<f64>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CompanySite {
    pub name: String,
    pub location: String,
    pub site_type: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CompanyEvent {
    pub event_type: String,
    pub description: String,
    pub date: String,
    pub source_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CompanyKeyPerson {
    pub person_id: String,
    pub name: String,
    pub role: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct CompanyDetail {
    pub id: String,
    pub name: String,
    pub legal_name: Option<String>,
    pub region: String,
    pub country: String,
    pub city: Option<String>,
    pub website: Option<String>,
    pub entity_type: String,
    pub is_competitor: bool,
    pub threat_score: Option<f64>,
    pub overlap_score: Option<f64>,
    pub capabilities: Vec<String>,
    pub certifications: Vec<String>,
    pub sites: Vec<CompanySite>,
    pub key_persons: Vec<CompanyKeyPerson>,
    pub recent_events: Vec<CompanyEvent>,
    pub community_badges: Vec<String>,
    pub source_entropy: Option<f64>,
    pub source_quality_label: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PersonListItem {
    pub id: String,
    pub name: String,
    pub role: String,
    pub organization: String,
    pub region: String,
    pub priority_score: f64,
    pub influence_score: i64,
    pub priority: String,
    #[serde(default)]
    pub pain_index: f64,
    #[serde(default)]
    pub change_risk: f64,
    #[serde(default)]
    pub role_drift_score: f64,
    pub tags: Vec<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PersonEvent {
    pub event_type: String,
    pub description: String,
    pub date: String,
    pub source_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PersonPeer {
    pub id: String,
    pub name: String,
    pub role: String,
    pub organization: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PersonDetail {
    pub id: String,
    pub name: String,
    pub role: String,
    pub organization: String,
    pub region: String,
    pub bio: Option<String>,
    pub priority_score: f64,
    pub influence_score: i64,
    pub priority: String,
    #[serde(default)]
    pub pain_index: f64,
    #[serde(default)]
    pub change_risk: f64,
    #[serde(default)]
    pub role_drift_score: f64,
    pub tags: Vec<String>,
    pub timeline: Vec<PersonEvent>,
    pub peers: Vec<PersonPeer>,
    pub warning_count: i64,
    pub insight_count: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GraphNodeLabel {
    pub id: String,
    pub label: String,
    pub node_type: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GraphEdgeRecord {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub weight: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EdgeTypeCount {
    pub edge_type: String,
    pub count: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GraphOverview {
    pub companies_total: u64,
    pub persons_total: u64,
    pub warnings_total: u64,
    pub insights_total: u64,
    pub edges_total: u64,
    pub nodes: Vec<GraphNodeLabel>,
    pub edges: Vec<GraphEdgeRecord>,
    pub edge_type_counts: Vec<EdgeTypeCount>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub entity_type: String,
    pub title: String,
    pub snippet: String,
    pub region: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub total_hits: u64,
    pub results: Vec<SearchHit>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SecuritySummary {
    pub dns_posture_score: f64,
    pub lookalike_domains_detected: u64,
    pub kev_matches: u64,
    pub last_scan_at: Option<String>,
    pub domains_monitored: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WeeklyMemoSection {
    pub title: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WeeklyMemoActionItem {
    pub owner: String,
    pub action: String,
    pub priority: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WeeklyMemoKeyMetrics {
    pub warning_count: Option<u64>,
    pub insight_count: Option<u64>,
    pub company_count: Option<u64>,
    pub poi_count: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WeeklyMemo {
    pub id: String,
    pub title: String,
    pub week_start: String,
    pub week_end: String,
    pub executive_summary: String,
    pub sections: Vec<WeeklyMemoSection>,
    pub key_metrics: WeeklyMemoKeyMetrics,
    pub action_items: Vec<WeeklyMemoActionItem>,
    pub generated_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RecipeRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub region: Option<String>,
    pub precision: f64,
    pub recall: f64,
    pub false_positive_rate: f64,
    pub fired_count: u32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct NotificationPrefs {
    pub email_enabled: bool,
    pub slack_enabled: bool,
    pub browser_push: bool,
    pub min_severity: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UserPreferences {
    pub user_id: String,
    pub theme: String,
    pub locale: String,
    pub default_region: Option<String>,
    pub dashboard_layout: String,
    pub notifications: NotificationPrefs,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PreferencesResponse {
    pub preferences: UserPreferences,
}

#[cfg(target_arch = "wasm32")]
async fn get_api<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;

    let response = Request::get(path)
        .credentials(RequestCredentials::SameOrigin)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    let envelope: ApiEnvelope<T> = serde_json::from_str(&body).map_err(|err| err.to_string())?;

    if status >= 400 || !envelope.success {
        return Err(envelope
            .error
            .map(|error| error.message)
            .unwrap_or_else(|| format!("Request failed with status {status}")));
    }

    envelope
        .data
        .ok_or_else(|| "Response body missing data payload".to_string())
}

#[cfg(target_arch = "wasm32")]
async fn send_api<T: DeserializeOwned, B: Serialize>(
    method: &str,
    path: &str,
    body: Option<&B>,
) -> Result<T, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;

    let request = match method {
        "POST" => Request::post(path),
        "DELETE" => Request::delete(path),
        other => return Err(format!("Unsupported method {other}")),
    }
    .credentials(RequestCredentials::SameOrigin);

    let request = if let Some(payload) = body {
        request.json(payload).map_err(|err| err.to_string())?
    } else {
        request.build().map_err(|err| err.to_string())?
    };

    let response = request.send().await.map_err(|err| err.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    let envelope: ApiEnvelope<T> = serde_json::from_str(&body).map_err(|err| err.to_string())?;

    if status >= 400 || !envelope.success {
        return Err(envelope
            .error
            .map(|error| error.message)
            .unwrap_or_else(|| format!("Request failed with status {status}")));
    }

    envelope
        .data
        .ok_or_else(|| "Response body missing data payload".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn get_api<T: DeserializeOwned>(_path: &str) -> Result<T, String> {
    Err("WASM data fetching is only available in the browser runtime".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
async fn send_api<T: DeserializeOwned, B: Serialize>(
    _method: &str,
    _path: &str,
    _body: Option<&B>,
) -> Result<T, String> {
    Err("WASM mutations are only available in the browser runtime".to_string())
}

fn query_string(params: &[(&str, Option<String>)]) -> String {
    let values = params
        .iter()
        .filter_map(|(key, value)| {
            value
                .as_ref()
                .map(|value| format!("{}={}", key, urlencoding::encode(value)))
        })
        .collect::<Vec<_>>();

    if values.is_empty() {
        String::new()
    } else {
        format!("?{}", values.join("&"))
    }
}

pub async fn fetch_dashboard() -> Result<DashboardStats, String> {
    get_api("/api/dashboard").await
}

pub async fn fetch_warnings(
    page: u32,
    severity: Option<String>,
) -> Result<PagedResponse<WarningRecord>, String> {
    get_api(&format!(
        "/api/warnings{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("8".to_string())),
            ("severities", severity),
        ])
    ))
    .await
}

pub async fn fetch_insights(
    page: u32,
    bookmarked: bool,
) -> Result<PagedResponse<InsightRecord>, String> {
    get_api(&format!(
        "/api/insights{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("8".to_string())),
            ("bookmarked", bookmarked.then(|| "true".to_string()),),
        ])
    ))
    .await
}

pub async fn set_insight_bookmark(insight_id: &str, bookmarked: bool) -> Result<(), String> {
    let path = format!("/api/insights/{insight_id}/bookmark");
    let _: Value = if bookmarked {
        send_api("POST", &path, Option::<&Value>::None).await?
    } else {
        send_api("DELETE", &path, Option::<&Value>::None).await?
    };
    Ok(())
}

pub async fn submit_insight_feedback(
    insight_id: &str,
    feedback_type: &str,
    notes: Option<String>,
) -> Result<(), String> {
    let path = format!("/api/insights/{insight_id}/feedback");
    let mut payload = serde_json::Map::new();
    payload.insert("feedback_type".to_string(), feedback_type.into());
    payload.insert(
        "notes".to_string(),
        notes.map(Value::String).unwrap_or(Value::Null),
    );
    let payload = Value::Object(payload);
    let _: Value = send_api("POST", &path, Some(&payload)).await?;
    Ok(())
}

pub async fn fetch_companies(
    page: u32,
    competitor_only: bool,
) -> Result<PagedResponse<CompanyListItem>, String> {
    get_api(&format!(
        "/api/companies{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("8".to_string())),
            ("is_competitor", competitor_only.then(|| "true".to_string()),),
        ])
    ))
    .await
}

pub async fn fetch_company_detail(company_id: &str) -> Result<CompanyDetail, String> {
    get_api(&format!("/api/companies/{company_id}")).await
}

pub async fn fetch_persons(
    page: u32,
    priority: Option<String>,
) -> Result<PagedResponse<PersonListItem>, String> {
    get_api(&format!(
        "/api/persons{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("8".to_string())),
            ("priority", priority),
        ])
    ))
    .await
}

pub async fn fetch_person_detail(person_id: &str) -> Result<PersonDetail, String> {
    get_api(&format!("/api/persons/{person_id}")).await
}

pub async fn fetch_graph() -> Result<GraphOverview, String> {
    get_api("/api/graph").await
}

pub async fn fetch_search(page: u32, query: &str) -> Result<SearchResponse, String> {
    get_api(&format!(
        "/api/search{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("10".to_string())),
            ("q", Some(query.to_string())),
        ])
    ))
    .await
}

pub async fn fetch_memos(page: u32) -> Result<PagedResponse<WeeklyMemo>, String> {
    get_api(&format!(
        "/api/memos{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("6".to_string()))
        ])
    ))
    .await
}

pub async fn fetch_weekly_memo() -> Result<WeeklyMemo, String> {
    get_api("/api/insights/weekly-memo").await
}

pub async fn fetch_security_summary() -> Result<SecuritySummary, String> {
    get_api("/api/security").await
}

pub async fn fetch_calibration_curve() -> Result<CalibrationCurve, String> {
    get_api("/api/admin/calibration").await
}

pub async fn fetch_competitors(page: u32) -> Result<PagedResponse<CompanyListItem>, String> {
    get_api(&format!(
        "/api/competitors{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("8".to_string()))
        ])
    ))
    .await
}

pub async fn fetch_competitor_changes(page: u32) -> Result<PagedResponse<Value>, String> {
    get_api(&format!(
        "/api/competitors/changes{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("8".to_string()))
        ])
    ))
    .await
}

pub async fn fetch_recipes(
    page: u32,
    status: Option<String>,
) -> Result<PagedResponse<RecipeRecord>, String> {
    get_api(&format!(
        "/api/recipes{}",
        query_string(&[
            ("page", Some(page.to_string())),
            ("per_page", Some("8".to_string())),
            ("status", status),
        ])
    ))
    .await
}

pub async fn fetch_preferences() -> Result<PreferencesResponse, String> {
    get_api("/api/preferences").await
}

pub async fn fetch_admin_value(path: &str) -> Result<Value, String> {
    get_api(path).await
}

#[cfg(target_arch = "wasm32")]
pub async fn submit_login(username: &str, password: &str) -> Result<String, String> {
    use gloo_net::http::Request;
    use web_sys::RequestCredentials;

    let body = format!(
        "username={}&password={}",
        urlencoding::encode(username),
        urlencoding::encode(password)
    );

    let response = Request::post("/login")
        .credentials(RequestCredentials::Include)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .map_err(|err| err.to_string())?
        .send()
        .await
        .map_err(|err| err.to_string())?;

    if response.redirected() {
        return Ok(response.url());
    }

    let body = response.text().await.map_err(|err| err.to_string())?;
    if body.contains("Invalid credentials") {
        Err("Invalid credentials".to_string())
    } else if body.contains("Server misconfiguration") {
        Err("Server misconfiguration — contact administrator".to_string())
    } else if response.ok() {
        Ok("/".to_string())
    } else {
        Err(format!("Login failed with status {}", response.status()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn submit_login(_username: &str, _password: &str) -> Result<String, String> {
    Err("Login submission is only available in the browser runtime".to_string())
}
