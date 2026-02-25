"use client";

import { useState, useMemo } from "react";
import { BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid, Legend } from "recharts";
import {
  PageHeader, SurfaceCard, CompetitorCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, Target, TrendingUp
} from "@/components/ui";
import { COMPETITORS } from "@/lib/mock-data";

const THREAT_FILTERS = ["high", "medium", "low"] as const;
const OVERLAP_FILTERS = ["50%+", "30-50%", "<30%"] as const;

function threatTier(score: number): "high" | "medium" | "low" {
  if (score >= 70) return "high";
  if (score >= 40) return "medium";
  return "low";
}

function overlapTier(pct: number): "50%+" | "30-50%" | "<30%" {
  if (pct >= 50) return "50%+";
  if (pct >= 30) return "30-50%";
  return "<30%";
}

export default function CompetitorsPage() {
  const [threatFilter, setThreatFilter] = useState<string | null>(null);
  const [overlapFilter, setOverlapFilter] = useState<string | null>(null);

  const filteredCompetitors = useMemo(() => {
    return COMPETITORS.filter((c) => {
      if (threatFilter && threatTier(c.threat_score) !== threatFilter) return false;
      if (overlapFilter && overlapTier(c.overlap_pct) !== overlapFilter) return false;
      return true;
    });
  }, [threatFilter, overlapFilter]);

  const avgThreat = COMPETITORS.length > 0
    ? Math.round(COMPETITORS.reduce((s, c) => s + c.threat_score, 0) / COMPETITORS.length)
    : 0;

  const avgOverlap = COMPETITORS.length > 0
    ? Math.round(COMPETITORS.reduce((s, c) => s + c.overlap_pct, 0) / COMPETITORS.length)
    : 0;

  const highThreat = COMPETITORS.filter((c) => c.threat_score >= 70).length;
  const activeFilterCount = Number(Boolean(threatFilter)) + Number(Boolean(overlapFilter));

  /* Chart data: threat vs overlap scatter-style bar */
  const chartData = COMPETITORS.map((c) => ({
    name: c.name,
    threat: c.threat_score,
    overlap: c.overlap_pct,
  }));

  return (
    <div>
      <PageHeader
        title="Competitor Dashboard"
        subtitle="Threat and overlap tracking with competitor change monitoring."
        icon={PAGE_ICONS.competitors}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Competitors"
          value={COMPETITORS.length.toString()}
          icon={<Target className="h-3.5 w-3.5" />}
          accent="#4A90E2"
        />
        <StatCard
          label="High Threat"
          value={highThreat.toString()}
          delta="≥70"
          direction="flat"
          icon={<Target className="h-3.5 w-3.5" />}
          accent="#D62D2D"
        />
        <StatCard
          label="Avg Threat"
          value={avgThreat.toString()}
          icon={<Target className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Avg Overlap"
          value={`${avgOverlap}%`}
          icon={<TrendingUp className="h-3.5 w-3.5" />}
          accent="#2D8C3C"
        />
      </div>

      {/* ─── Chart ───────────────────────────────────────────── */}
      <SurfaceCard title="Threat vs Overlap Comparison" icon={<Target className="h-3.5 w-3.5" />}>
        <div className="h-48 mt-2">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={chartData} margin={{ top: 8, right: 16, left: 0, bottom: 0 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#ccc" />
              <XAxis dataKey="name" tick={{ fontSize: 9 }} label={{ value: "Competitor", position: "insideBottom", offset: -4, style: { fontSize: 10, fill: "#888" } }} />
              <YAxis tick={{ fontSize: 10 }} domain={[0, 100]} label={{ value: "Score (%)", angle: -90, position: "insideLeft", style: { fontSize: 10, fill: "#888" } }} />
              <Tooltip
                formatter={(value: number, name: string) => [
                  `${value.toLocaleString("en-US")}%`,
                  name === "threat" ? "Threat score" : "Overlap",
                ]}
                contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
              />
              <Legend verticalAlign="top" height={24} iconSize={8} wrapperStyle={{ fontSize: 10 }} />
              <Bar dataKey="threat" name="Threat Score" fill="#D62D2D" radius={[2, 2, 0, 0]} />
              <Bar dataKey="overlap" name="Overlap %" fill="#4A90E2" radius={[2, 2, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </SurfaceCard>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <FilterBar>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine competitor comparisons:</span>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Threat:</span>
        {THREAT_FILTERS.map((t) => (
          <FilterChip
            key={t}
            label={t}
            active={threatFilter === t}
            onClick={() => setThreatFilter(threatFilter === t ? null : t)}
          />
        ))}
        <span className="mx-3 h-4 w-px bg-border" />
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Overlap:</span>
        {OVERLAP_FILTERS.map((o) => (
          <FilterChip
            key={o}
            label={o}
            active={overlapFilter === o}
            onClick={() => setOverlapFilter(overlapFilter === o ? null : o)}
          />
        ))}
        <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
        <button
          type="button"
          onClick={() => {
            setThreatFilter(null);
            setOverlapFilter(null);
          }}
          className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
        >
          Reset Filters
        </button>
      </FilterBar>

      {/* ─── Competitors Grid ───────────────────────────────── */}
      <div className="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {filteredCompetitors.length === 0 ? (
          <div className="col-span-full">
            <EmptyState
              title="No competitors match current filters"
              description="Try broadening threat/overlap criteria or use Reset Filters to compare all competitors."
            />
          </div>
        ) : (
          filteredCompetitors.map((c) => (
            <CompetitorCard
              key={c.id}
              name={c.name}
              region={c.region}
              tier={c.tier}
              threatScore={c.threat_score}
              overlapPct={c.overlap_pct}
              recentChanges={c.recent_changes}
              metrics={c.metrics}
            />
          ))
        )}
      </div>
    </div>
  );
}
