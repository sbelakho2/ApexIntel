"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, StatCard,
  Database, Globe, Clock, Radio, Check
} from "@/components/ui";
import { fetchHealth, fetchEndpoints } from "@/lib/api";

export default function SettingsPage() {
  const { data: health } = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => fetchHealth({ signal }),
  });

  const { data: endpoints } = useQuery({
    queryKey: ["endpoints"],
    queryFn: ({ signal }) => fetchEndpoints({ signal }),
  });

  const [apiKeyVisible, setApiKeyVisible] = useState(false);
  const apiKey = process.env.NEXT_PUBLIC_APEX_API_KEY || "(not configured)";

  return (
    <div>
      <PageHeader
        title="Settings"
        subtitle="System configuration, API status, and environment information."
        icon={<Database className="h-4 w-4" />}
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard label="API Status" value={health?.status ?? "–"} icon={<Database className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Endpoints" value={(endpoints?.length ?? "–").toString()} icon={<Globe className="h-3.5 w-3.5" />} accent="#2D8C3C" />
        <StatCard label="Uptime" value={health?.uptime_secs ? `${Math.floor(health.uptime_secs / 3600)}h` : "–"} icon={<Clock className="h-3.5 w-3.5" />} accent="#FFBE00" />
        <StatCard label="Version" value={health?.version ?? "–"} icon={<Radio className="h-3.5 w-3.5" />} accent="#D62D2D" />
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <SurfaceCard title="API Configuration" icon={<Database className="h-3.5 w-3.5" />}>
          <div className="divide-y divide-border">
            <div className="flex items-center justify-between py-3">
              <span className="text-sm font-bold">API Key</span>
              <div className="flex items-center gap-2">
                <code className="text-xs font-mono bg-secondary px-2 py-1 rounded">
                  {apiKeyVisible ? apiKey : "•".repeat(Math.min(apiKey.length, 24))}
                </code>
                <button type="button" onClick={() => setApiKeyVisible(!apiKeyVisible)}
                  className="text-[10px] font-bold uppercase text-muted-foreground hover:text-foreground">
                  {apiKeyVisible ? "Hide" : "Show"}
                </button>
              </div>
            </div>
            <div className="flex items-center justify-between py-3">
              <span className="text-sm font-bold">Backend URL</span>
              <code className="text-xs font-mono bg-secondary px-2 py-1 rounded">via Next.js rewrite proxy</code>
            </div>
          </div>
        </SurfaceCard>

        <SurfaceCard title="Health Checks" icon={<Check className="h-3.5 w-3.5" />}>
          {health?.checks && health.checks.length > 0 ? (
            <div className="divide-y divide-border">
              {health.checks.map((check) => (
                <div key={check.name} className="flex items-center justify-between py-3">
                  <span className="text-sm font-bold">{check.name}</span>
                  <span className={`text-xs font-bold uppercase ${
                    check.status === "Healthy" ? "text-green-600" :
                    check.status === "Degraded" ? "text-yellow-600" : "text-red-500"
                  }`}>{check.status}</span>
                </div>
              ))}
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">No health check data available.</p>
          )}
        </SurfaceCard>

        <div className="lg:col-span-2">
          <SurfaceCard title="Registered API Endpoints" icon={<Globe className="h-3.5 w-3.5" />}>
            {endpoints && endpoints.length > 0 ? (
              <div className="overflow-x-auto">
                <table className="data-table">
                  <thead>
                    <tr>
                      <th>Method</th>
                      <th>Path</th>
                      <th>Description</th>
                      <th>Auth</th>
                    </tr>
                  </thead>
                  <tbody>
                    {endpoints.map((ep) => (
                      <tr key={`${ep.method}-${ep.path}`}>
                        <td><span className="font-mono text-xs font-bold">{ep.method}</span></td>
                        <td><span className="font-mono text-xs">{ep.path}</span></td>
                        <td className="text-xs text-muted-foreground">{ep.description}</td>
                        <td>{ep.auth_required ? <span className="text-xs font-bold text-yellow-600">Yes</span> : <span className="text-xs text-muted-foreground">No</span>}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-xs text-muted-foreground">No endpoint data available.</p>
            )}
          </SurfaceCard>
        </div>
      </div>
    </div>
  );
}
