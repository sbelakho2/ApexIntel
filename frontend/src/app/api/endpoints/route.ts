import { NextResponse } from 'next/server';

type EndpointDef = {
  method: string;
  path: string;
  description: string;
  auth_required: boolean;
  min_role: string;
};

const FALLBACK_ENDPOINTS: EndpointDef[] = [
  { method: 'GET', path: '/api/health', description: 'Service health check', auth_required: false, min_role: 'public' },
  { method: 'GET', path: '/api/endpoints', description: 'List available API endpoints', auth_required: false, min_role: 'public' },
  { method: 'GET', path: '/api/warnings', description: 'List warnings with filters', auth_required: true, min_role: 'viewer' },
  { method: 'GET', path: '/api/insights', description: 'List insights with filters', auth_required: true, min_role: 'viewer' },
  { method: 'GET', path: '/api/companies', description: 'List companies with filters', auth_required: true, min_role: 'viewer' },
  { method: 'GET', path: '/api/persons', description: 'List persons / key contacts', auth_required: true, min_role: 'viewer' },
  { method: 'GET', path: '/api/security/dns-posture', description: 'Get DNS posture checks', auth_required: true, min_role: 'viewer' },
  { method: 'GET', path: '/api/graph/neighborhood/:id', description: 'Get entity neighborhood in graph', auth_required: true, min_role: 'viewer' },
];

function getBaseApiUrl(): string | null {
  const value = process.env.APEX_API_BASE_URL || process.env.NEXT_PUBLIC_API_BASE_URL || process.env.API_BASE_URL;
  if (!value) return null;
  return value.replace(/\/$/, '');
}

export async function GET() {
  const baseUrl = getBaseApiUrl();
  if (!baseUrl) {
    return NextResponse.json(FALLBACK_ENDPOINTS, { status: 200 });
  }

  try {
    const response = await fetch(`${baseUrl}/api/endpoints`, {
      method: 'GET',
      headers: { Accept: 'application/json' },
      cache: 'no-store',
    });

    if (!response.ok) {
      return NextResponse.json(FALLBACK_ENDPOINTS, { status: 200 });
    }

    const payload = (await response.json()) as EndpointDef[];
    return NextResponse.json(payload, { status: 200 });
  } catch {
    return NextResponse.json(FALLBACK_ENDPOINTS, { status: 200 });
  }
}
