"use client";

import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, InsightCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, Eye, TrendingUp
} from "@/components/ui";
import { fetchInsights } from "@/lib/api";

const TYPE_FILTERS = ["demand_signal", "supply_risk", "competitive_intel", "security_posture", "macro_shift", "poi_movement"] as const;

export default function InsightsPage() {
  const [typeFilter, setTypeFilter] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["insights"],
    queryFn: ({ signal }) => fetchInsights({ signal, per_page: 100 }),
  });

  const insights = data?.items ?? [];

  const filteredInsights = useMemo(() => {
    if (!typeFilter) return insights;
    return insights.filter((i) => i.insight_type === typeFilter);
  }, [insights, typeFilter]);

  const avgConfidence = insights.length > 0
    ? Math.round((insights.reduce((sum, i) => sum + i.confidence, 0) / insights.length) * 100)
    : 0;

  return (
    <div>
      <PageHeader
        title="Insight Feed"
        subtitle="Ranked insights with evidence links and context."
        icon={PAGE_ICONS.insights}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 mb-6">
        <StatCard label="Total Insights" value={(data?.total ?? 0).toString()} icon={<Eye className="h-3.5 w-3.5" />} accent="#FFBE00" />
        <StatCard label="Avg Confidence" value={`${avgConfidence}%`} icon={<TrendingUp className="h-3.5 w-3.5" />} accent="#2D8C3C" />
        <StatCard label="Types" value={new Set(insights.map((i) => i.insight_type)).size.toString()} icon={<Eye className="h-3.5 w-3.5" />} accent="#4A90E2" />
      </div>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <div className="mb-4">
        <FilterBar>
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Type:</span>
          {TYPE_FILTERS.map((t) => (
            <FilterChip key={t} label={t.replace(/_/g, " ")} active={typeFilter === t} onClick={() => setTypeFilter(typeFilter === t ? null : t)} />
          ))}
          <button type="button" onClick={() => setTypeFilter(null)}
            className="ml-2 rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground">
            Reset
          </button>
        </FilterBar>
      </div>

      {/* ─── Insights List ───────────────────────────────────── */}
      <div className="space-y-3">
        {isLoading ? (
          <SurfaceCard title="Loading"><p className="text-xs text-muted-foreground animate-pulse">Loading insights from backend…</p></SurfaceCard>
        ) : filteredInsights.length === 0 ? (
          <EmptyState
            title="No insights available"
            description={insights.length === 0 ? "No insights have been generated yet. Insights populate as crawlers and analysis recipes run." : "Try adjusting type filter or reset."}
          />
        ) : (
          filteredInsights.map((insight) => (
            <InsightCard
              key={insight.id}
              title={insight.title}
              type={insight.insight_type}
              region={insight.region}
              confidence={insight.confidence}
              impact="medium"
              sources={insight.evidence_urls}
              summary={insight.summary}
            />
          ))
        )}
      </div>
    </div>
  );
}
