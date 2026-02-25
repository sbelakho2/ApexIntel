import { NextResponse } from 'next/server';

type HealthResponse = {
  status: 'Healthy' | 'Degraded' | 'Unhealthy';
  version: string;
  uptime_secs: number;
  checks: { name: string; status: 'Healthy' | 'Degraded' | 'Unhealthy'; message?: string }[];
};

const FALLBACK_HEALTH: HealthResponse = {
  status: 'Healthy',
  version: 'frontend-local',
  uptime_secs: 0,
  checks: [
    { name: 'frontend', status: 'Healthy', message: 'Next.js route handler operational' },
    { name: 'api_upstream', status: 'Degraded', message: 'Using local fallback response' },
  ],
};

function getBaseApiUrl(): string | null {
  const value = process.env.APEX_API_BASE_URL || process.env.NEXT_PUBLIC_API_BASE_URL || process.env.API_BASE_URL;
  if (!value) return null;
  return value.replace(/\/$/, '');
}

export async function GET() {
  const baseUrl = getBaseApiUrl();
  if (!baseUrl) {
    return NextResponse.json(FALLBACK_HEALTH, { status: 200 });
  }

  try {
    const response = await fetch(`${baseUrl}/api/health`, {
      method: 'GET',
      headers: { Accept: 'application/json' },
      cache: 'no-store',
    });

    if (!response.ok) {
      return NextResponse.json(
        {
          ...FALLBACK_HEALTH,
          status: 'Degraded',
          checks: [
            { name: 'frontend', status: 'Healthy', message: 'Next.js route handler operational' },
            { name: 'api_upstream', status: 'Degraded', message: `Upstream returned ${response.status}` },
          ],
        } satisfies HealthResponse,
        { status: 200 }
      );
    }

    const payload = (await response.json()) as HealthResponse;
    return NextResponse.json(payload, { status: 200 });
  } catch {
    return NextResponse.json(
      {
        ...FALLBACK_HEALTH,
        status: 'Degraded',
      } satisfies HealthResponse,
      { status: 200 }
    );
  }
}
