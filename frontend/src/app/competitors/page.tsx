"use client";

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, StatCard, EmptyState,
  PAGE_ICONS, Target, Boxes
} from "@/components/ui";
import { fetchCompanies } from "@/lib/api";

export default function CompetitorsPage() {
  const { data, isLoading } = useQuery({
    queryKey: ["companies", "competitors"],
    queryFn: ({ signal }) => fetchCompanies({ signal, is_competitor: true, per_page: 100 }),
  });

  const competitors = data?.items ?? [];

  const avgThreat = competitors.length > 0
    ? Math.round(competitors.filter((c) => c.threat_score !== null).reduce((s, c) => s + (c.threat_score ?? 0), 0) / Math.max(1, competitors.filter((c) => c.threat_score !== null).length))
    : 0;

  return (
    <div>
      <PageHeader
        title="Competitor Dashboard"
        subtitle="Threat and overlap tracking with competitor change monitoring."
        icon={PAGE_ICONS.competitors}
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 mb-6">
        <StatCard label="Competitors" value={(data?.total ?? 0).toString()} icon={<Target className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Avg Threat" value={avgThreat > 0 ? avgThreat.toString() : "–"} icon={<Target className="h-3.5 w-3.5" />} accent="#D62D2D" />
        <StatCard label="Regions" value={new Set(competitors.map((c) => c.region)).size.toString()} icon={<Boxes className="h-3.5 w-3.5" />} accent="#2D8C3C" />
      </div>

      <SurfaceCard title="Competitors" icon={<Target className="h-3.5 w-3.5" />}>
        {isLoading ? (
          <p className="text-xs text-muted-foreground animate-pulse">Loading competitors…</p>
        ) : competitors.length === 0 ? (
          <EmptyState
            title="No competitors identified yet"
            description="Competitors are companies flagged as is_competitor in the entity pipeline. Once the system identifies competitors from crawled data, they will appear here with threat scores and capability overlap analysis."
          />
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {competitors.map((c) => (
              <div key={c.id} className="apex-card p-4">
                <p className="font-bold text-sm">{c.name}</p>
                <p className="text-[10px] text-muted-foreground uppercase mt-1">{c.region} · {c.entity_type}</p>
                {c.threat_score !== null && (
                  <p className="text-xs mt-2">Threat Score: <span className="font-bold tabular-nums">{c.threat_score}</span></p>
                )}
                {c.capabilities.length > 0 && (
                  <div className="flex flex-wrap gap-1 mt-2">
                    {c.capabilities.map((cap) => (
                      <span key={cap} className="rounded-sm border border-border bg-secondary px-1.5 py-0.5 text-[9px] font-bold uppercase">{cap}</span>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </SurfaceCard>
    </div>
  );
}
