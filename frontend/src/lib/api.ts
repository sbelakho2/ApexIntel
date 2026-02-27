// ─── ApexIntel API Client ────────────────────────────────────────
// All authenticated requests go through /api/proxy/* to keep the API key
// server-side. The proxy adds Bearer token auth before forwarding to backend.
// Public endpoints (health, endpoints) hit the backend directly via rewrite.

/**
 * Route prefix for authenticated API calls.
 * Goes through the server-side proxy which adds the API key.
 */
const PROXY_PREFIX = "/api/proxy";

/**
 * Route prefix for public (unauthenticated) API calls.
 * These go directly to the backend via Next.js rewrite.
 */
const PUBLIC_PREFIX = "/api";

async function handleJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    throw new Error(text || `Request failed with status ${response.status}`);
  }
  return response.json();
}

/** Unwrap the standard ApiResponse envelope */
function unwrapResponse<T>(json: { success: boolean; data?: T; error?: { message: string } }): T {
  if (!json.success || !json.data) {
    throw new Error(json.error?.message || "API returned an error");
  }
  return json.data;
}

// ─── Types ──────────────────────────────────────────────────────

export type HealthResponse = {
  status: "Healthy" | "Degraded" | "Unhealthy";
  version: string;
  uptime_secs: number;
  checks: { name: string; status: "Healthy" | "Degraded" | "Unhealthy"; message?: string }[];
};

export type EndpointDef = {
  method: string;
  path: string;
  description: string;
  auth_required: boolean;
  min_role: string;
};

export interface PagedData<T> {
  items: T[];
  total: number;
  page: number;
  per_page: number;
}

export interface WarningItem {
  id: string;
  title: string;
  description: string;
  severity: string;
  warning_type: string;
  region: string;
  source_urls: string[];
  entity_ids: string[];
  recipe_id: string | null;
  confidence: number;
  acknowledged: boolean;
  acknowledged_by: string | null;
  acknowledged_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface InsightItem {
  id: string;
  title: string;
  summary: string;
  insight_type: string;
  region: string;
  confidence: number;
  evidence_urls: string[];
  entity_ids: string[];
  tags: string[];
  created_at: string;
  updated_at: string;
}

export interface CompanyItem {
  id: string;
  name: string;
  region: string;
  country: string;
  entity_type: string;
  is_competitor: boolean;
  threat_score: number | null;
  capabilities: string[];
  updated_at: string;
}

export interface PersonItem {
  id: string;
  name: string;
  role: string;
  organization: string;
  region: string;
  priority_score: number;
  engagement_status: string;
  updated_at: string;
}

export interface CompanySite {
  name: string;
  location: string;
  site_type: string;
}

export interface CompanyKeyPerson {
  person_id: string;
  name: string;
  role: string;
}

export interface CompanyEvent {
  event_type: string;
  description: string;
  date: string;
  source_url: string | null;
}

export interface CompanyDetail {
  id: string;
  name: string;
  legal_name: string | null;
  region: string;
  country: string;
  city: string | null;
  website: string | null;
  entity_type: string;
  is_competitor: boolean;
  threat_score: number | null;
  overlap_score: number | null;
  capabilities: string[];
  certifications: string[];
  sites: CompanySite[];
  key_persons: CompanyKeyPerson[];
  recent_events: CompanyEvent[];
  created_at: string;
  updated_at: string;
}

export interface PriorityVector {
  decision_power: number;
  domain_relevance: number;
  network_centrality: number;
  engagement_potential: number;
  intelligence_value: number;
}

export interface Affiliation {
  organization: string;
  role: string;
  current: boolean;
}

export interface PersonEvent {
  event_type: string;
  description: string;
  date: string;
  source_url: string | null;
}

export interface PersonDetail {
  id: string;
  name: string;
  role: string;
  organization: string;
  region: string;
  country: string;
  email: string | null;
  phone: string | null;
  linkedin: string | null;
  priority_score: number;
  priority_vector: PriorityVector;
  engagement_status: string;
  tags: string[];
  affiliations: Affiliation[];
  timeline: PersonEvent[];
  created_at: string;
  updated_at: string;
}

// ─── Public Endpoints ───────────────────────────────────────────

export async function fetchHealth({ signal }: { signal?: AbortSignal } = {}): Promise<HealthResponse> {
  const res = await fetch(`${PUBLIC_PREFIX}/health`, { cache: "no-store", signal });
  return handleJson<HealthResponse>(res);
}

export async function fetchEndpoints({ signal }: { signal?: AbortSignal } = {}): Promise<EndpointDef[]> {
  const res = await fetch(`${PUBLIC_PREFIX}/endpoints`, { cache: "no-store", signal });
  return handleJson<EndpointDef[]>(res);
}

// ─── Protected Endpoints ────────────────────────────────────────

export async function fetchWarnings({
  signal,
  page = 1,
  per_page = 50,
  severities,
  regions,
  warning_types,
}: {
  signal?: AbortSignal;
  page?: number;
  per_page?: number;
  severities?: string;
  regions?: string;
  warning_types?: string;
} = {}): Promise<PagedData<WarningItem>> {
  const params = new URLSearchParams({ page: String(page), per_page: String(per_page) });
  if (severities) params.set("severities", severities);
  if (regions) params.set("regions", regions);
  if (warning_types) params.set("warning_types", warning_types);
  const res = await fetch(`${PROXY_PREFIX}/warnings?${params}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: PagedData<WarningItem>; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

export async function fetchPersons({
  signal,
  page = 1,
  per_page = 50,
  regions,
  roles,
  search,
  min_priority,
  sort_by,
}: {
  signal?: AbortSignal;
  page?: number;
  per_page?: number;
  regions?: string;
  roles?: string;
  search?: string;
  min_priority?: number;
  sort_by?: string;
} = {}): Promise<PagedData<PersonItem>> {
  const params = new URLSearchParams({ page: String(page), per_page: String(per_page) });
  if (regions) params.set("regions", regions);
  if (roles) params.set("roles", roles);
  if (search) params.set("search", search);
  if (min_priority !== undefined) params.set("min_priority", String(min_priority));
  if (sort_by) params.set("sort_by", sort_by);
  const res = await fetch(`${PROXY_PREFIX}/persons?${params}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: PagedData<PersonItem>; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

export async function acknowledgeWarning(id: string, userId: string, note?: string): Promise<void> {
  const res = await fetch(`${PROXY_PREFIX}/warnings/${id}/acknowledge`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ user_id: userId, note }),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || "Failed to acknowledge warning");
  }
}

export async function fetchInsights({
  signal,
  page = 1,
  per_page = 50,
  regions,
}: {
  signal?: AbortSignal;
  page?: number;
  per_page?: number;
  regions?: string;
} = {}): Promise<PagedData<InsightItem>> {
  const params = new URLSearchParams({ page: String(page), per_page: String(per_page) });
  if (regions) params.set("regions", regions);
  const res = await fetch(`${PROXY_PREFIX}/insights?${params}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: PagedData<InsightItem>; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

export async function fetchCompanies({
  signal,
  page = 1,
  per_page = 50,
  regions,
  is_competitor,
}: {
  signal?: AbortSignal;
  page?: number;
  per_page?: number;
  regions?: string;
  is_competitor?: boolean;
} = {}): Promise<PagedData<CompanyItem>> {
  const params = new URLSearchParams({ page: String(page), per_page: String(per_page) });
  if (regions) params.set("regions", regions);
  if (is_competitor !== undefined) params.set("is_competitor", String(is_competitor));
  const res = await fetch(`${PROXY_PREFIX}/companies?${params}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: PagedData<CompanyItem>; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

export async function fetchSearch({
  signal,
  q,
  page = 1,
  per_page = 20,
}: {
  signal?: AbortSignal;
  q: string;
  page?: number;
  per_page?: number;
}): Promise<PagedData<{ id: string; title: string; snippet: string; entity_type: string; score: number }>> {
  const params = new URLSearchParams({ q, page: String(page), per_page: String(per_page) });
  const res = await fetch(`${PROXY_PREFIX}/search?${params}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: PagedData<{ id: string; title: string; snippet: string; entity_type: string; score: number }>; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

export async function fetchCompany({
  id,
  signal,
}: {
  id: string;
  signal?: AbortSignal;
}): Promise<CompanyDetail> {
  const res = await fetch(`${PROXY_PREFIX}/companies/${id}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: CompanyDetail; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

export async function fetchPerson({
  id,
  signal,
}: {
  id: string;
  signal?: AbortSignal;
}): Promise<PersonDetail> {
  const res = await fetch(`${PROXY_PREFIX}/persons/${id}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: PersonDetail; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

// ─── Graph Types & Fetcher ──────────────────────────────────────

export interface GraphEdge {
  source: string;
  target: string;
  edge_type: string;
  weight: number;
  label: string | null;
}

export interface EdgeTypeCount {
  edge_type: string;
  count: number;
}

export interface GraphOverview {
  companies_total: number;
  persons_total: number;
  warnings_total: number;
  insights_total: number;
  edges_total: number;
  edges: GraphEdge[];
  edge_type_counts: EdgeTypeCount[];
}

export async function fetchGraph({
  signal,
}: {
  signal?: AbortSignal;
} = {}): Promise<GraphOverview> {
  const res = await fetch(`${PROXY_PREFIX}/graph`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: GraphOverview; error?: { message: string } }>(res);
  return unwrapResponse(json);
}

// ─── Recipe Types & Fetcher ─────────────────────────────────────

export interface RecipeItem {
  id: string;
  name: string;
  description: string;
  status: "Staging" | "Production" | "Deprecated";
  region: string | null;
  precision: number;
  recall: number;
  false_positive_rate: number;
  fired_count: number;
  last_fired: string | null;
  created_at: string;
  updated_at: string;
}

export async function fetchRecipes({
  signal,
  page = 1,
  per_page = 50,
}: {
  signal?: AbortSignal;
  page?: number;
  per_page?: number;
} = {}): Promise<PagedData<RecipeItem>> {
  const params = new URLSearchParams({ page: String(page), per_page: String(per_page) });
  const res = await fetch(`${PROXY_PREFIX}/recipes?${params}`, {
    cache: "no-store",
    signal,
  });
  const json = await handleJson<{ success: boolean; data?: PagedData<RecipeItem>; error?: { message: string } }>(res);
  return unwrapResponse(json);
}
