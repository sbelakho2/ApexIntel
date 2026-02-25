"use client";

import { useState, useMemo } from "react";
import { LineChart, Line, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid, Legend } from "recharts";
import {
  PageHeader, SurfaceCard, RecipeCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, Sparkles, TrendingUp
} from "@/components/ui";
import { RECIPES, RECIPE_PERF_TREND } from "@/lib/mock-data";

const STATUS_FILTERS = ["production", "staging", "deprecated"] as const;

export default function RecipesPage() {
  const [statusFilter, setStatusFilter] = useState<string | null>(null);
  const [smoothSeries, setSmoothSeries] = useState(true);
  const [showPrecision, setShowPrecision] = useState(true);
  const [showRecall, setShowRecall] = useState(true);
  const [showFpr, setShowFpr] = useState(true);

  const filteredRecipes = useMemo(() => {
    if (!statusFilter) return RECIPES;
    return RECIPES.filter((r) => r.status === statusFilter);
  }, [statusFilter]);

  const avgPrecision = RECIPES.length > 0
    ? Math.round((RECIPES.reduce((s, r) => s + r.precision, 0) / RECIPES.length) * 100)
    : 0;

  const avgRecall = RECIPES.length > 0
    ? Math.round((RECIPES.reduce((s, r) => s + r.recall, 0) / RECIPES.length) * 100)
    : 0;

  const avgFPR = RECIPES.length > 0
    ? ((RECIPES.reduce((s, r) => s + r.fpr, 0) / RECIPES.length) * 100).toFixed(1)
    : "0";

  const totalAlerts = RECIPES.reduce((s, r) => s + r.alerts_7d, 0);

  const prodCount = RECIPES.filter((r) => r.status === "production").length;
  const activeFilterCount = Number(Boolean(statusFilter));

  return (
    <div>
      <PageHeader
        title="Recipe Manager"
        subtitle="Production, staging, and deprecated recipe lifecycle with performance metrics."
        icon={PAGE_ICONS.recipes}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5 mb-6">
        <StatCard label="Recipes" value={RECIPES.length.toString()} icon={<Sparkles className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Production" value={prodCount.toString()} icon={<Sparkles className="h-3.5 w-3.5" />} accent="#2D8C3C" />
        <StatCard label="Avg Precision" value={`${avgPrecision}%`} icon={<TrendingUp className="h-3.5 w-3.5" />} accent="#FFBE00" />
        <StatCard label="Avg Recall" value={`${avgRecall}%`} icon={<TrendingUp className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Alerts (7d)" value={totalAlerts.toString()} icon={<Sparkles className="h-3.5 w-3.5" />} accent="#D62D2D" />
      </div>

      {/* ─── Performance Trend ───────────────────────────────── */}
      <SurfaceCard
        title="Precision / Recall / FPR Trend (30d)"
        icon={<TrendingUp className="h-3.5 w-3.5" />}
        actions={(
          <button
            type="button"
            onClick={() => setSmoothSeries((prev) => !prev)}
            className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
          >
            {smoothSeries ? "Smoothing On" : "Smoothing Off"}
          </button>
        )}
      >
        <div className="mb-2 flex flex-wrap items-center gap-2 text-[10px] font-bold uppercase tracking-[0.1em] text-muted-foreground">
          <button type="button" onClick={() => setShowPrecision((v) => !v)} className={`rounded-sm border px-2 py-1 ${showPrecision ? "border-border bg-secondary text-foreground" : "border-border bg-card"}`}>Precision</button>
          <button type="button" onClick={() => setShowRecall((v) => !v)} className={`rounded-sm border px-2 py-1 ${showRecall ? "border-border bg-secondary text-foreground" : "border-border bg-card"}`}>Recall</button>
          <button type="button" onClick={() => setShowFpr((v) => !v)} className={`rounded-sm border px-2 py-1 ${showFpr ? "border-border bg-secondary text-foreground" : "border-border bg-card"}`}>FPR</button>
        </div>
        <div className="h-48 mt-2">
          {RECIPE_PERF_TREND.length === 0 ? (
            <EmptyState title="No recipe trend data" description="No chart data is currently available for this period." />
          ) : (
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={RECIPE_PERF_TREND} margin={{ top: 8, right: 16, left: 0, bottom: 0 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#ccc" />
              <XAxis dataKey="month" tick={{ fontSize: 9 }} label={{ value: "Month", position: "insideBottom", offset: -4, style: { fontSize: 10, fill: "#888" } }} />
              <YAxis tick={{ fontSize: 10 }} domain={[0, 1]} label={{ value: "Rate", angle: -90, position: "insideLeft", style: { fontSize: 10, fill: "#888" } }} />
              <Tooltip
                formatter={(value: number, name: string) => [`${(value * 100).toFixed(1)}%`, String(name)]}
                contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
              />
              <Legend verticalAlign="top" align="center" height={24} iconSize={8} wrapperStyle={{ fontSize: 10 }} />
              {showPrecision && <Line type={smoothSeries ? "monotone" : "linear"} dataKey="precision" stroke="#2D8C3C" strokeWidth={2} dot={false} />}
              {showRecall && <Line type={smoothSeries ? "monotone" : "linear"} dataKey="recall" stroke="#4A90E2" strokeWidth={2} dot={false} />}
              {showFpr && <Line type={smoothSeries ? "monotone" : "linear"} dataKey="fpr" stroke="#D62D2D" strokeWidth={2} dot={false} />}
            </LineChart>
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
                  <th>Month</th>
                  <th>Precision</th>
                  <th>Recall</th>
                  <th>FPR</th>
                </tr>
              </thead>
              <tbody>
                {RECIPE_PERF_TREND.map((row) => (
                  <tr key={row.month}>
                    <td>{row.month}</td>
                    <td>{(row.precision * 100).toFixed(1)}%</td>
                    <td>{(row.recall * 100).toFixed(1)}%</td>
                    <td>{(row.fpr * 100).toFixed(1)}%</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>
      </SurfaceCard>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <FilterBar>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine recipe lifecycle:</span>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Status:</span>
        {STATUS_FILTERS.map((s) => (
          <FilterChip
            key={s}
            label={s}
            active={statusFilter === s}
            onClick={() => setStatusFilter(statusFilter === s ? null : s)}
          />
        ))}
        <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
        <button
          type="button"
          onClick={() => setStatusFilter(null)}
          className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
        >
          Reset Filters
        </button>
      </FilterBar>

      {/* ─── Recipes Grid ────────────────────────────────────── */}
      <div className="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {filteredRecipes.length === 0 ? (
          <div className="col-span-full">
            <EmptyState
              title="No recipes match current filters"
              description="Try broadening status selection or reset filters to restore all recipe lifecycle states."
            />
          </div>
        ) : (
          filteredRecipes.map((r) => (
            <RecipeCard
              key={r.id}
              name={r.name}
              description={r.description}
              status={r.status}
              precision={r.precision}
              recall={r.recall}
              fpr={r.fpr}
              alerts7d={r.alerts_7d}
            />
          ))
        )}
      </div>
    </div>
  );
}
