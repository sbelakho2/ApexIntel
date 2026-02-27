"use client";

import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, WarningCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, AlertTriangle
} from "@/components/ui";
import { fetchWarnings, acknowledgeWarning } from "@/lib/api";

type Severity = "critical" | "high" | "medium" | "low";
const SEVERITY_FILTERS: Severity[] = ["critical", "high", "medium", "low"];
const STATUS_FILTERS = ["active", "acknowledged"] as const;

export default function WarningsPage() {
  const [severityFilter, setSeverityFilter] = useState<Severity | null>(null);
  const [statusFilter, setStatusFilter] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["warnings"],
    queryFn: ({ signal }) => fetchWarnings({ signal, per_page: 100 }),
  });

  const warnings = data?.items ?? [];

  const filteredWarnings = useMemo(() => {
    return warnings.filter((w) => {
      if (severityFilter && w.severity !== severityFilter) return false;
      if (statusFilter === "active" && w.acknowledged) return false;
      if (statusFilter === "acknowledged" && !w.acknowledged) return false;
      return true;
    });
  }, [warnings, severityFilter, statusFilter]);

  const counts = {
    critical: warnings.filter((w) => w.severity === "critical" && !w.acknowledged).length,
    high: warnings.filter((w) => w.severity === "high" && !w.acknowledged).length,
    medium: warnings.filter((w) => w.severity === "medium" && !w.acknowledged).length,
    low: warnings.filter((w) => w.severity === "low" && !w.acknowledged).length,
  };

  const activeFilterCount = Number(Boolean(severityFilter)) + Number(Boolean(statusFilter));

  return (
    <div>
      <PageHeader
        title="Warning Center"
        subtitle="Real-time warnings with severity, type, and region filters."
        icon={PAGE_ICONS.warnings}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard label="Critical" value={counts.critical.toString()} icon={<AlertTriangle className="h-3.5 w-3.5" />} accent="#D62D2D" />
        <StatCard label="High" value={counts.high.toString()} icon={<AlertTriangle className="h-3.5 w-3.5" />} accent="#FF8C00" />
        <StatCard label="Medium" value={counts.medium.toString()} icon={<AlertTriangle className="h-3.5 w-3.5" />} accent="#FFBE00" />
        <StatCard label="Low" value={counts.low.toString()} icon={<AlertTriangle className="h-3.5 w-3.5" />} accent="#4A90E2" />
      </div>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <div className="mb-4">
        <FilterBar>
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Severity:</span>
          {SEVERITY_FILTERS.map((s) => (
            <FilterChip key={s} label={s} active={severityFilter === s} onClick={() => setSeverityFilter(severityFilter === s ? null : s)} />
          ))}
          <span className="mx-3 h-4 w-px bg-border" />
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Status:</span>
          {STATUS_FILTERS.map((st) => (
            <FilterChip key={st} label={st} active={statusFilter === st} onClick={() => setStatusFilter(statusFilter === st ? null : st)} />
          ))}
          <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
          <button type="button" onClick={() => { setSeverityFilter(null); setStatusFilter(null); }}
            className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground">
            Reset Filters
          </button>
        </FilterBar>
      </div>

      {/* ─── Warnings List ───────────────────────────────────── */}
      <div className="space-y-3">
        {isLoading ? (
          <SurfaceCard title="Loading"><p className="text-xs text-muted-foreground animate-pulse">Loading warnings from backend…</p></SurfaceCard>
        ) : filteredWarnings.length === 0 ? (
          <EmptyState
            title="No warnings match the current filters"
            description={warnings.length === 0 ? "No warnings have been generated yet. The system will populate warnings as crawlers and recipes run." : "Try adjusting filters or use Reset Filters."}
          />
        ) : (
          filteredWarnings.map((w) => (
            <WarningCard
              key={w.id}
              severity={w.severity as "critical"|"high"|"medium"|"low"}
              title={w.title}
              region={w.region}
              type={w.warning_type}
              created={w.created_at}
              status={w.acknowledged ? "acknowledged" : "active"}
            />
          ))
        )}
      </div>
    </div>
  );
}
