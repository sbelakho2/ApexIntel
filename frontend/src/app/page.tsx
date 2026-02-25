"use client";

import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { BarChart, Bar, LineChart, Line, XAxis, YAxis, Tooltip, ResponsiveContainer, PieChart, Pie, Cell, ReferenceLine } from "recharts";
import { fetchEndpoints, fetchHealth, HealthResponse } from "@/lib/api";
import {
  PageHeader, StatCard, SurfaceCard, WarningCard, SparkBar, ProgressBar,
  PAGE_ICONS, AlertTriangle, Zap, Users, Shield, Eye, Boxes, BarChart3,
  Clock, Target, Globe
} from "@/components/ui";
import {
  DASHBOARD_KPIS, WARNINGS, WARNING_TREND, INSIGHTS, CRAWL_ACTIVITY,
  COMPANY_REGION_DIST, RECIPES
} from "@/lib/mock-data";

function statusLabel(status?: HealthResponse["status"]): string {
  if (!status) return "–";
  return status.toLowerCase();
}

export default function OverviewPage() {
  const { data: health, isLoading: loadingHealth, isError: healthError, refetch: refetchHealth } = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => fetchHealth({ signal }),
  });

  const { data: endpoints, isError: endpointsError, refetch: refetchEndpoints } = useQuery({
    queryKey: ["endpoints"],
    queryFn: ({ signal }) => fetchEndpoints({ signal }),
  });

  const activeWarnings = WARNINGS.filter((w) => w.status === "active");
  const criticalCount = activeWarnings.filter((w) => w.severity === "critical").length;
  const highCount = activeWarnings.filter((w) => w.severity === "high").length;
  const productionRecipes = RECIPES.filter((r) => r.status === "production");
  const avgPrecision = productionRecipes.length > 0
    ? productionRecipes.reduce((sum, r) => sum + r.precision, 0) / productionRecipes.length
    : 0;

  // Sparkline data for warnings
  const warningSparkValues = WARNING_TREND.slice(-14).map((d) => d.critical + d.high + d.medium + d.low);
  // Stable 14-point insight sparkline derived from mock INSIGHTS data to avoid SSR/hydration mismatch
  const insightSparkValues = useMemo(
    () => INSIGHTS.slice(0, 14).map((_, i) => ((i * 7 + 3) % 5) + 1),
    []
  );

  const cappedWarningTrend = useMemo(() => {
    const maxPoints = 24;
    if (WARNING_TREND.length <= maxPoints) {
      return WARNING_TREND;
    }
    const step = Math.ceil(WARNING_TREND.length / maxPoints);
    return WARNING_TREND.filter((_, index) => index % step === 0).slice(-maxPoints);
  }, []);

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
          label="Insights (7d)"
          value={DASHBOARD_KPIS.insights_generated_7d.toString()}
          delta="+2 vs prior"
          direction="up"
          icon={<Eye className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Companies Tracked"
          value={DASHBOARD_KPIS.companies_tracked.toString()}
          delta="12 active"
          direction="flat"
          icon={<Boxes className="h-3.5 w-3.5" />}
          accent="#4A90E2"
        />
        <StatCard
          label="POIs Profiled"
          value={DASHBOARD_KPIS.pois_profiled.toString()}
          delta="+1 new"
          direction="up"
          icon={<Users className="h-3.5 w-3.5" />}
          accent="#2D8C3C"
        />
      </div>

      {/* ─── Secondary KPI Row ───────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Recipes (Prod)"
          value={productionRecipes.length.toString()}
          delta={`${RECIPES.filter((r) => r.status === "staging").length} staging`}
          direction="flat"
          icon={<Zap className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Avg Precision"
          value={`${Math.round(avgPrecision * 100)}%`}
          delta="+2%"
          direction="up"
          icon={<Target className="h-3.5 w-3.5" />}
          accent="#2D8C3C"
        />
        <StatCard
          label="Crawl Success"
          value={`${DASHBOARD_KPIS.crawl_success_rate}%`}
          delta="-0.8%"
          direction="down"
          icon={<Globe className="h-3.5 w-3.5" />}
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
            Some live API checks failed. Retry to refresh service health and endpoint status.
          </p>
          <button
            type="button"
            onClick={() => {
              void refetchHealth();
              void refetchEndpoints();
            }}
            className="mt-2 rounded-sm border border-border bg-secondary px-3 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-foreground hover:bg-muted"
          >
            Retry checks
          </button>
        </div>
      )}

      {health && (health.status === "Degraded" || health.uptime_secs === 0) && (
        <div className="mb-6 rounded-sm border border-warning/40 bg-warning/10 px-4 py-3">
          <p className="text-xs font-bold uppercase tracking-[0.1em] text-foreground">
            Data may be stale: upstream health is degraded or running in fallback mode.
          </p>
        </div>
      )}

      {/* ─── Warning Trend + Crawl Activity Charts ───────────── */}
      <div className="grid gap-4 lg:grid-cols-2 mb-6">
        <SurfaceCard title="Warning Trend (30d)" icon={<AlertTriangle className="h-3.5 w-3.5" />}>
          <div className="h-48">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={cappedWarningTrend} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                <XAxis dataKey="date" tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} label={{ value: "Date", position: "insideBottom", offset: -4, style: { fontSize: 10, fill: "#888" } }} />
                <YAxis tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} width={24} label={{ value: "Warnings", angle: -90, position: "insideLeft", style: { fontSize: 10, fill: "#888" } }} />
                <Tooltip
                  formatter={(value: number) => [`${value.toLocaleString("en-US")} events`, "Count"]}
                  contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
                  labelStyle={{ fontWeight: "bold" }}
                />
                <Bar dataKey="critical" stackId="a" fill="#D62D2D" radius={[0, 0, 0, 0]} />
                <Bar dataKey="high" stackId="a" fill="#FF8C00" radius={[0, 0, 0, 0]} />
                <Bar dataKey="medium" stackId="a" fill="#FFBE00" radius={[0, 0, 0, 0]} />
                <Bar dataKey="low" stackId="a" fill="#4A90E2" radius={[2, 2, 0, 0]} />
                <ReferenceLine y={10} stroke="#6B7280" strokeDasharray="4 4" label={{ value: "Alert threshold", fill: "#6B7280", fontSize: 10 }} />
              </BarChart>
            </ResponsiveContainer>
          </div>
          <div className="flex items-center justify-center gap-4 mt-2 text-[10px] font-bold uppercase">
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-red-500" /> Critical</span>
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm" style={{ background: "#FF8C00" }} /> High</span>
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-yellow-500" /> Medium</span>
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-blue-500" /> Low</span>
          </div>
        </SurfaceCard>

        <SurfaceCard title="Crawl Activity (24h)" icon={<Clock className="h-3.5 w-3.5" />}>
          <div className="h-48">
            <ResponsiveContainer width="100%" height="100%">
              <LineChart data={CRAWL_ACTIVITY} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                <XAxis dataKey="hour" tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} interval={3} label={{ value: "Hour (UTC)", position: "insideBottom", offset: -4, style: { fontSize: 10, fill: "#888" } }} />
                <YAxis tick={{ fontSize: 9, fill: "#999" }} tickLine={false} axisLine={false} width={24} label={{ value: "Crawl jobs", angle: -90, position: "insideLeft", style: { fontSize: 10, fill: "#888" } }} />
                <Tooltip
                  formatter={(value: number) => [`${value.toLocaleString("en-US")} jobs`, "Volume"]}
                  contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
                />
                <Line type="monotone" dataKey="success" stroke="#2D8C3C" strokeWidth={2} dot={false} />
                <Line type="monotone" dataKey="errors" stroke="#D62D2D" strokeWidth={1.5} dot={false} />
              </LineChart>
            </ResponsiveContainer>
          </div>
          <div className="flex items-center justify-center gap-4 mt-2 text-[10px] font-bold uppercase">
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-green-600" /> Success</span>
            <span className="flex items-center gap-1"><span className="h-2 w-2 rounded-sm bg-red-500" /> Errors</span>
          </div>
        </SurfaceCard>
      </div>

      {/* ─── Warnings + Region Distribution ──────────────────── */}
      <div className="grid gap-4 lg:grid-cols-3 mb-6">
        <div className="lg:col-span-2">
          <SurfaceCard title="Recent Warnings" icon={<AlertTriangle className="h-3.5 w-3.5" />}>
            <div className="space-y-3 max-h-[400px] overflow-y-auto">
              {activeWarnings.slice(0, 5).map((w) => (
                <WarningCard
                  key={w.id}
                  severity={w.severity}
                  title={w.title}
                  region={w.region}
                  type={w.type}
                  created={w.created}
                  status={w.status}
                />
              ))}
            </div>
            <a
              href="/warnings"
              className="mt-4 flex items-center justify-center gap-2 rounded-sm border border-border bg-secondary px-3 py-2 text-[10px] font-black uppercase tracking-[0.12em] text-muted-foreground hover:text-foreground"
            >
              View All Warnings
            </a>
          </SurfaceCard>
        </div>

        <SurfaceCard title="Coverage by Region" icon={<Globe className="h-3.5 w-3.5" />}>
          <div className="h-48">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={COMPANY_REGION_DIST}
                  dataKey="value"
                  nameKey="name"
                  cx="50%"
                  cy="50%"
                  innerRadius={40}
                  outerRadius={70}
                  paddingAngle={2}
                >
                  {COMPANY_REGION_DIST.map((entry, index) => (
                    <Cell key={`cell-${index}`} fill={entry.fill} />
                  ))}
                </Pie>
                <Tooltip
                  contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
                />
              </PieChart>
            </ResponsiveContainer>
          </div>
          <div className="flex flex-wrap justify-center gap-2 mt-2">
            {COMPANY_REGION_DIST.map((r) => (
              <span key={r.name} className="flex items-center gap-1 text-[9px] font-bold uppercase">
                <span className="h-2 w-2 rounded-sm" style={{ backgroundColor: r.fill }} />
                {r.name}
              </span>
            ))}
          </div>
        </SurfaceCard>
      </div>

      {/* ─── Quick Stats Grid ────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <SurfaceCard title="Warnings Spark" icon={<AlertTriangle className="h-3.5 w-3.5" />}>
          <SparkBar values={warningSparkValues} color="#D62D2D" />
          <p className="text-[10px] text-muted-foreground mt-2 text-center">Last 14 days</p>
        </SurfaceCard>
        <SurfaceCard title="Insights Spark" icon={<Eye className="h-3.5 w-3.5" />}>
          <SparkBar values={insightSparkValues} color="#FFBE00" />
          <p className="text-[10px] text-muted-foreground mt-2 text-center">Last 14 days</p>
        </SurfaceCard>
        <SurfaceCard title="Security Posture" icon={<Shield className="h-3.5 w-3.5" />}>
          <ProgressBar value={72} label="DNS Health" color="#2D8C3C" />
          <ProgressBar value={45} label="Threat Exposure" color="#D62D2D" />
        </SurfaceCard>
        <SurfaceCard title="Sources Monitored" icon={<Boxes className="h-3.5 w-3.5" />}>
          <div className="text-center">
            <p className="text-3xl font-black tabular-nums text-foreground">{DASHBOARD_KPIS.sources_monitored}</p>
            <p className="text-[10px] font-bold uppercase text-muted-foreground mt-1">Active Sources</p>
          </div>
          <div className="flex justify-center gap-4 mt-3">
            <div className="text-center">
              <p className="text-lg font-bold tabular-nums">{DASHBOARD_KPIS.regions_covered}</p>
              <p className="text-[9px] uppercase text-muted-foreground">Regions</p>
            </div>
            <div className="text-center">
              <p className="text-lg font-bold tabular-nums">{endpoints?.length ?? "–"}</p>
              <p className="text-[9px] uppercase text-muted-foreground">Endpoints</p>
            </div>
          </div>
        </SurfaceCard>
      </div>
    </div>
  );
}
