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

async function handleJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    throw new Error(text || `Request failed with status ${response.status}`);
  }
  return response.json();
}

export async function fetchHealth({ signal }: { signal?: AbortSignal } = {}): Promise<HealthResponse> {
  const res = await fetch("/api/health", { cache: "no-store", signal });
  return handleJson<HealthResponse>(res);
}

export async function fetchEndpoints({ signal }: { signal?: AbortSignal } = {}): Promise<EndpointDef[]> {
  const res = await fetch("/api/endpoints", { cache: "no-store", signal });
  return handleJson<EndpointDef[]>(res);
}
