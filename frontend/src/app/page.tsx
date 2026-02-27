"use client";

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { PieChart, Pie, Cell, ResponsiveContainer, Tooltip } from "recharts";
import { fetchEndpoints, fetchHealth, fetchWarnings, fetchInsights, fetchCompanies, HealthResponse } from "@/lib/api";
import {
  PageHeader, StatCard, SurfaceCard, WarningCard, SparkBar, ProgressBar, EmptyState,
  PAGE_ICONS, AlertTriangle, Eye, Boxes, BarChart3,
  Shield, Globe
} from "@/components/ui";

function statusLabel(status?: HealthResponse["status"]): string {
  if (!status) return "–";
  return status.toLowerCase();
}

const REGION_COLORS: Record<string, string> = {
  TN: "#FFBE00", MA: "#D62D2D", IL: "#4A90E2", EU: "#2D8C3C",
  CN: "#999999", US: "#666666", Global: "#333333",
};

export default function OverviewPage() {
  const { data: health, isLoading: loadingHealth, isError: healthError, refetch: refetchHealth } = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => fetchHealth({ signal }),
  });

  const { data: endpoints, isError: endpointsError, refetch: refetchEndpoints } = useQuery({
    queryKey: ["endpoints"],
    queryFn: ({ signal }) => fetchEndpoints({ signal }),
  });

  const { data: warningsData } = useQuery({
    queryKey: ["warnings", "overview"],
    queryFn: ({ signal }) => fetchWarnings({ signal, per_page: 50 }),
  });

  const { data: insightsData } = useQuery({
    queryKey: ["insights", "overview"],
    queryFn: ({ signal }) => fetchInsights({ signal, per_page: 50 }),
  });

  const { data: companiesData } = useQuery({
    queryKey: ["companies", "overview"],
    queryFn: ({ signal }) => fetchCompanies({ signal, per_page: 100 }),
  });

  const warnings = warningsData?.items ?? [];
  const insights = insightsData?.items ?? [];
  const companies = companiesData?.items ?? [];

  const activeWarnings = warnings.filter((w) => !w.acknowledged);
  const criticalCount = activeWarnings.filter((w) => w.severity === "critical").length;
  const highCount = activeWarnings.filter((w) => w.severity === "high").length;

  const regionDist = useMemo(() => {
    const map = new Map<string, number>();
    companies.forEach((c) => {
      const key = c.region || "Other";
      map.set(key, (map.get(key) || 0) + 1);
    });
    return Array.from(map.entries()).map(([name, value]) => ({
      name,
      value,
      fill: REGION_COLORS[name] || "#888888",
    }));
  }, [companies]);

  const warningSparkValues = useMemo(() => {
    if (warnings.length === 0) return [];
    const counts = new Array(14).fill(0);
    const now = Date.now();
    warnings.forEach((w) => {
      const dayAgo = Math.floor((now - new Date(w.created_at).getTime()) / 86400000);
      if (dayAgo >= 0 && dayAgo < 14) {
        counts[13 - dayAgo]++;
      }
    });
    return counts;
  }, [warnings]);

  return (
    <div>
      <PageHeader
        title="Overview"
        subtitle="Real-time intelligence dashboard — warnings, insights, coverage, and system health."
        icon={PAGE_ICONS.overview}
      />

      {/* ─── Primary KPI Row ─────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Active Warnings"
          value={activeWarnings.length.toString()}
          delta={`${criticalCount} crit / ${highCount} high`}
          direction={criticalCount > 0 ? "up" : "flat"}
          icon={<AlertTriangle className="h-3.5 w-3.5" />}
          accent="#D62D2D"
        />
        <StatCard
          label="Total Insights"
          value={(insightsData?.total ?? 0).toString()}
          direction="flat"
          icon={<Eye className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Companies Tracked"
          value={(companiesData?.total ?? 0).toString()}
          direction="flat"
          icon={<Boxes className="h-3.5 w-3.5" />}
          accent="#4A90E2"
        />
        <StatCard
          label="API Status"
          value={loadingHealth ? "…" : healthError ? "degraded" : statusLabel(health?.status)}
          delta={endpoints ? `${endpoints.length} endpoints` : endpointsError ? "endpoint fetch failed" : "–"}
          direction="flat"
          icon={<BarChart3 className="h-3.5 w-3.5" />}
          accent="#999999"
        />
      </div>

      {(healthError || endpointsError) && (
        <div className="mb-6 rounded-sm border border-destructive/30 bg-destructive/10 px-4 py-3">
          <p className="text-xs font-bold uppercase tracking-[0.1em] text-destructive">
            Some live API checks failed. Retry to refresh.
          </p>
          <button type="button" onClick={() => { void refetchHealth(); void refetchEndpoints(); }}
            className="mt-2 rounded-sm border border-border bg-secondary px-3 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-foreground hover:bg-muted">
            Retry checks
          </button>
        </div>
      )}

      {/* ─── Warnings + Regions ──────────────────────────────── */}
      <div className="grid gap-4 lg:grid-cols-3 mb-6">
        <div className="lg:col-span-2">
          <SurfaceCard title="Recent Warnings" icon={<AlertTriangle className="h-3.5 w-3.5" />}>
            {activeWarnings.length === 0 ? (
              <EmptyState title="No active warnings" description="The system is operating normally. Warnings will appear here when detected." />
            ) : (
              <div className="space-y-3 max-h-[400px] overflow-y-auto">
                {activeWarnings.slice(0, 5).map((w) => (
                  <WarningCard key={w.id} severity={w.severity as "critical"|"high"|"medium"|"low"} title={w.title} region={w.region} type={w.warning_type} created={w.created_at} status={w.acknowledged ? "acknowledged" : "active"} />
                ))}
              </div>
            )}
            <a href="/warnings" className="mt-4 flex items-center justify-center gap-2 rounded-sm border border-border bg-secondary px-3 py-2 text-[10px] font-black uppercase tracking-[0.12em] text-muted-foreground hover:text-foreground">
              View All Warnings
            </a>
          </SurfaceCard>
        </div>

        <SurfaceCard title="Coverage by Region" icon={<Globe className="h-3.5 w-3.5" />}>
          {regionDist.length === 0 ? (
            <EmptyState title="No companies yet" description="Add companies to see regional distribution." />
          ) : (
            <>
              <div className="h-48">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie data={regionDist} dataKey="value" nameKey="name" cx="50%" cy="50%" innerRadius={40} outerRadius={70} paddingAngle={2}>
                      {regionDist.map((entry, index) => (<Cell key={`cell-${index}`} fill={entry.fill} />))}
                    </Pie>
                    <Tooltip contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }} />
                  </PieChart>
                </ResponsiveContainer>
              </div>
              <div className="flex flex-wrap justify-center gap-2 mt-2">
                {regionDist.map((r) => (
                  <span key={r.name} className="flex items-center gap-1 text-[9px] font-bold uppercase">
                    <span className="h-2 w-2 rounded-sm" style={{ backgroundColor: r.fill }} />{r.name}
                  </span>
                ))}
              </div>
            </>
          )}
        </SurfaceCard>
      </div>

      {/* ─── Quick Stats Grid ────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <SurfaceCard title="Warnings Spark" icon={<AlertTriangle className="h-3.5 w-3.5" />}>
          {warningSparkValues.length > 0 ? (
            <><SparkBar values={warningSparkValues} color="#D62D2D" /><p className="text-[10px] text-muted-foreground mt-2 text-center">Last 14 days</p></>
          ) : (
            <EmptyState title="No warning data" description="Spark data populates as warnings are generated." />
          )}
        </SurfaceCard>
        <SurfaceCard title="Insights Summary" icon={<Eye className="h-3.5 w-3.5" />}>
          <div className="text-center">
            <p className="text-3xl font-black tabular-nums text-foreground">{insightsData?.total ?? 0}</p>
            <p className="text-[10px] font-bold uppercase text-muted-foreground mt-1">Total Insights</p>
          </div>
        </SurfaceCard>
        <SurfaceCard title="Security Posture" icon={<Shield className="h-3.5 w-3.5" />}>
          <ProgressBar value={health?.status === "Healthy" ? 90 : health?.status === "Degraded" ? 50 : 20} label="System Health" color="#2D8C3C" />
        </SurfaceCard>
        <SurfaceCard title="System Info" icon={<Boxes className="h-3.5 w-3.5" />}>
          <div className="text-center">
            <p className="text-3xl font-black tabular-nums text-foreground">{endpoints?.length ?? "–"}</p>
            <p className="text-[10px] font-bold uppercase text-muted-foreground mt-1">API Endpoints</p>
          </div>
          <div className="flex justify-center gap-4 mt-3">
            <div className="text-center">
              <p className="text-lg font-bold tabular-nums">{regionDist.length}</p>
              <p className="text-[9px] uppercase text-muted-foreground">Regions</p>
            </div>
            <div className="text-center">
              <p className="text-lg font-bold tabular-nums">{health?.uptime_secs ? Math.floor(health.uptime_secs / 3600) : "–"}</p>
              <p className="text-[9px] uppercase text-muted-foreground">Uptime (hrs)</p>
            </div>
          </div>
        </SurfaceCard>
      </div>
    </div>
  );
}
