"use client";

import { useState, useMemo } from "react";
import { BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer } from "recharts";
import {
  PageHeader, SurfaceCard, InsightCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, Eye, TrendingUp
} from "@/components/ui";
import { INSIGHTS, INSIGHT_TREND } from "@/lib/mock-data";

const TYPE_FILTERS = ["demand_signal", "supply_risk", "competitive_intel", "security_posture", "macro_shift", "poi_movement"] as const;
const IMPACT_FILTERS = ["high", "medium", "low"] as const;

export default function InsightsPage() {
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [impactFilter, setImpactFilter] = useState<string | null>(null);

  const filteredInsights = useMemo(() => {
    return INSIGHTS.filter((i) => {
      if (typeFilter && i.type !== typeFilter) return false;
      if (impactFilter && i.impact !== impactFilter) return false;
      return true;
    });
  }, [typeFilter, impactFilter]);

  const counts = {
    high: INSIGHTS.filter((i) => i.impact === "high").length,
    medium: INSIGHTS.filter((i) => i.impact === "medium").length,
    low: INSIGHTS.filter((i) => i.impact === "low").length,
    total: INSIGHTS.length,
  };

  const avgConfidence = INSIGHTS.length > 0
    ? Math.round((INSIGHTS.reduce((sum, i) => sum + i.confidence, 0) / INSIGHTS.length) * 100)
    : 0;

  const activeFilterCount = Number(Boolean(typeFilter)) + Number(Boolean(impactFilter));

  return (
    <div>
      <PageHeader
        title="Insight Feed"
        subtitle="Ranked insights with evidence links and context."
        icon={PAGE_ICONS.insights}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Total Insights"
          value={counts.total.toString()}
          delta="Last 7 days"
          direction="flat"
          icon={<Eye className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="High Impact"
          value={counts.high.toString()}
          icon={<TrendingUp className="h-3.5 w-3.5" />}
          accent="#D62D2D"
        />
        <StatCard
          label="Medium Impact"
          value={counts.medium.toString()}
          icon={<TrendingUp className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Avg Confidence"
          value={`${avgConfidence}%`}
          icon={<Eye className="h-3.5 w-3.5" />}
          accent="#2D8C3C"
        />
      </div>

      {/* ─── Trend Chart ─────────────────────────────────────── */}
      <SurfaceCard title="Insight Generation (30d)" icon={<TrendingUp className="h-3.5 w-3.5" />}>
        <div className="h-40">
          {INSIGHT_TREND.length === 0 ? (
            <EmptyState title="No insight trend data" description="No chart data is currently available for this period." />
          ) : (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={INSIGHT_TREND} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
              <XAxis dataKey="date" tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} label={{ value: "Date", position: "insideBottom", offset: -4, style: { fontSize: 10, fill: "#888" } }} />
              <YAxis tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} width={24} label={{ value: "Insights", angle: -90, position: "insideLeft", style: { fontSize: 10, fill: "#888" } }} />
              <Tooltip
                formatter={(value: number) => [`${value.toLocaleString("en-US")} insights`, "Count"]}
                contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
              />
              <Bar dataKey="demand_signal" stackId="a" fill="#FF8C00" />
              <Bar dataKey="competitive_intel" stackId="a" fill="#D62D2D" />
              <Bar dataKey="supply_risk" stackId="a" fill="#FFBE00" />
              <Bar dataKey="security_posture" stackId="a" fill="#4A90E2" />
              <Bar dataKey="macro_shift" stackId="a" fill="#2D8C3C" radius={[2, 2, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
          )}
        </div>
        <details className="mt-3 rounded-sm border border-border bg-secondary/30 px-3 py-2">
          <summary className="cursor-pointer text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground">
            Chart data table
          </summary>
          <div className="mt-2 overflow-x-auto">
            <table className="data-table">
              <thead>
                <tr>
                  <th>Date</th>
                  <th>Demand</th>
                  <th>Competitive</th>
                  <th>Supply</th>
                  <th>Security</th>
                  <th>Macro</th>
                </tr>
              </thead>
              <tbody>
                {INSIGHT_TREND.slice(-10).map((row) => (
                  <tr key={row.date}>
                    <td>{row.date}</td>
                    <td>{row.demand_signal}</td>
                    <td>{row.competitive_intel}</td>
                    <td>{row.supply_risk}</td>
                    <td>{row.security_posture}</td>
                    <td>{row.macro_shift}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>
      </SurfaceCard>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <div className="mt-6">
        <FilterBar>
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine insight feed:</span>
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Type:</span>
          {TYPE_FILTERS.map((t) => (
            <FilterChip
              key={t}
              label={t.replace(/_/g, " ")}
              active={typeFilter === t}
              onClick={() => setTypeFilter(typeFilter === t ? null : t)}
            />
          ))}
          <span className="mx-3 h-4 w-px bg-border" />
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Impact:</span>
          {IMPACT_FILTERS.map((im) => (
            <FilterChip
              key={im}
              label={im}
              active={impactFilter === im}
              onClick={() => setImpactFilter(impactFilter === im ? null : im)}
            />
          ))}
          <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
          <button
            type="button"
            onClick={() => {
              setTypeFilter(null);
              setImpactFilter(null);
            }}
            className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
          >
            Reset Filters
          </button>
        </FilterBar>
      </div>

      {/* ─── Insights List ───────────────────────────────────── */}
      <div className="mt-4 space-y-3">
        {filteredInsights.length === 0 ? (
          <EmptyState
            title="No insights match the current filters"
            description="Try broadening type/impact filters or use Reset Filters to review all available insights."
          />
        ) : (
          filteredInsights.map((insight) => (
            <InsightCard
              key={insight.id}
              title={insight.title}
              type={insight.type}
              region={insight.region}
              confidence={insight.confidence}
              impact={insight.impact}
              sources={insight.sources}
              summary={insight.summary}
            />
          ))
        )}
      </div>
    </div>
  );
}
