"use client";

import { useState, useMemo } from "react";
import { BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer } from "recharts";
import {
  PageHeader, SurfaceCard, WarningCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, AlertTriangle, Clock
} from "@/components/ui";
import { WARNINGS, WARNING_TREND, type Severity } from "@/lib/mock-data";

const SEVERITY_FILTERS: Severity[] = ["critical", "high", "medium", "low"];
const STATUS_FILTERS = ["active", "acknowledged", "resolved"] as const;

export default function WarningsPage() {
  const [severityFilter, setSeverityFilter] = useState<Severity | null>(null);
  const [statusFilter, setStatusFilter] = useState<string | null>(null);

  const filteredWarnings = useMemo(() => {
    return WARNINGS.filter((w) => {
      if (severityFilter && w.severity !== severityFilter) return false;
      if (statusFilter && w.status !== statusFilter) return false;
      return true;
    });
  }, [severityFilter, statusFilter]);

  const counts = {
    critical: WARNINGS.filter((w) => w.severity === "critical" && w.status !== "resolved").length,
    high: WARNINGS.filter((w) => w.severity === "high" && w.status !== "resolved").length,
    medium: WARNINGS.filter((w) => w.severity === "medium" && w.status !== "resolved").length,
    low: WARNINGS.filter((w) => w.severity === "low" && w.status !== "resolved").length,
  };

  const activeFilterCount = Number(Boolean(severityFilter)) + Number(Boolean(statusFilter));

  const exportWarningTrend = () => {
    const headers = ["date", "critical", "high", "medium", "low"];
    const rows = WARNING_TREND.map((row) => [row.date, row.critical, row.high, row.medium, row.low].join(","));
    const csv = [headers.join(","), ...rows].join("\n");
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "warning-trend.csv";
    link.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div>
      <PageHeader
        title="Warning Center"
        subtitle="Real-time warnings with severity, type, and region filters."
        icon={PAGE_ICONS.warnings}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Critical"
          value={counts.critical.toString()}
          icon={<AlertTriangle className="h-3.5 w-3.5" />}
          accent="#D62D2D"
        />
        <StatCard
          label="High"
          value={counts.high.toString()}
          icon={<AlertTriangle className="h-3.5 w-3.5" />}
          accent="#FF8C00"
        />
        <StatCard
          label="Medium"
          value={counts.medium.toString()}
          icon={<AlertTriangle className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Low"
          value={counts.low.toString()}
          icon={<AlertTriangle className="h-3.5 w-3.5" />}
          accent="#4A90E2"
        />
      </div>

      {/* ─── Trend Chart ─────────────────────────────────────── */}
      <SurfaceCard
        title="Warning Trend (30d)"
        icon={<Clock className="h-3.5 w-3.5" />}
        actions={(
          <button
            type="button"
            onClick={exportWarningTrend}
            className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
          >
            Export CSV
          </button>
        )}
      >
        <div className="h-40">
          {WARNING_TREND.length === 0 ? (
            <EmptyState title="No warning trend data" description="No chart data is currently available for the selected period." />
          ) : (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={WARNING_TREND} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
              <XAxis dataKey="date" tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} label={{ value: "Date", position: "insideBottom", offset: -4, style: { fontSize: 10, fill: "#888" } }} />
              <YAxis tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} width={24} label={{ value: "Warnings", angle: -90, position: "insideLeft", style: { fontSize: 10, fill: "#888" } }} />
              <Tooltip
                formatter={(value: number) => [`${value.toLocaleString("en-US")} events`, "Count"]}
                contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
              />
              <Bar dataKey="critical" stackId="a" fill="#D62D2D" />
              <Bar dataKey="high" stackId="a" fill="#FF8C00" />
              <Bar dataKey="medium" stackId="a" fill="#FFBE00" />
              <Bar dataKey="low" stackId="a" fill="#4A90E2" radius={[2, 2, 0, 0]} />
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
                  <th>Critical</th>
                  <th>High</th>
                  <th>Medium</th>
                  <th>Low</th>
                </tr>
              </thead>
              <tbody>
                {WARNING_TREND.slice(-10).map((row) => (
                  <tr key={row.date}>
                    <td>{row.date}</td>
                    <td>{row.critical}</td>
                    <td>{row.high}</td>
                    <td>{row.medium}</td>
                    <td>{row.low}</td>
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
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine warning scope:</span>
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Severity:</span>
          {SEVERITY_FILTERS.map((s) => (
            <FilterChip
              key={s}
              label={s}
              active={severityFilter === s}
              onClick={() => setSeverityFilter(severityFilter === s ? null : s)}
            />
          ))}
          <span className="mx-3 h-4 w-px bg-border" />
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Status:</span>
          {STATUS_FILTERS.map((st) => (
            <FilterChip
              key={st}
              label={st}
              active={statusFilter === st}
              onClick={() => setStatusFilter(statusFilter === st ? null : st)}
            />
          ))}
          <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
          <button
            type="button"
            onClick={() => {
              setSeverityFilter(null);
              setStatusFilter(null);
            }}
            className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
          >
            Reset Filters
          </button>
        </FilterBar>
      </div>

      {/* ─── Warnings List ───────────────────────────────────── */}
      <div className="mt-4 space-y-3">
        {filteredWarnings.length === 0 ? (
          <EmptyState
            title="No warnings match the current filters"
            description="Try adjusting scope or use Reset Filters to return to the full warning feed."
          />
        ) : (
          filteredWarnings.map((w) => (
            <WarningCard
              key={w.id}
              severity={w.severity}
              title={w.title}
              region={w.region}
              type={w.type}
              created={w.created}
              status={w.status}
            />
          ))
        )}
      </div>
    </div>
  );
}
