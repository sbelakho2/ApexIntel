"use client";

import { useState, useMemo } from "react";
import Link from "next/link";
import { PieChart, Pie, Cell, ResponsiveContainer, Tooltip } from "recharts";
import {
  PageHeader, SurfaceCard, CompanyCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, Boxes, Globe
} from "@/components/ui";
import { COMPANIES, COMPANY_REGION_DIST } from "@/lib/mock-data";

const REGION_FILTERS = ["Tunisia", "Morocco", "Israel", "EU", "China", "Global"] as const;
const TIER_FILTERS = [1, 2, 3, 4, 5] as const;

export default function CompaniesPage() {
  const [regionFilter, setRegionFilter] = useState<string | null>(null);
  const [tierFilter, setTierFilter] = useState<number | null>(null);

  const filteredCompanies = useMemo(() => {
    return COMPANIES.filter((c) => {
      if (regionFilter && !c.region.toLowerCase().includes(regionFilter.toLowerCase())) return false;
      if (tierFilter && c.tier !== tierFilter) return false;
      return true;
    });
  }, [regionFilter, tierFilter]);

  const avgRisk = COMPANIES.length > 0
    ? Math.round(COMPANIES.reduce((sum, c) => sum + c.risk_score, 0) / COMPANIES.length)
    : 0;

  const highRiskCount = COMPANIES.filter((c) => c.risk_score >= 70).length;
  const activeFilterCount = Number(Boolean(regionFilter)) + Number(Boolean(tierFilter));

  return (
    <div>
      <PageHeader
        title="Company Browser"
        subtitle="Searchable company profiles with dossier links."
        icon={PAGE_ICONS.companies}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Companies Tracked"
          value={COMPANIES.length.toString()}
          icon={<Boxes className="h-3.5 w-3.5" />}
          accent="#4A90E2"
        />
        <StatCard
          label="High Risk"
          value={highRiskCount.toString()}
          delta={`≥70 score`}
          direction="flat"
          icon={<Boxes className="h-3.5 w-3.5" />}
          accent="#D62D2D"
        />
        <StatCard
          label="Avg Risk Score"
          value={avgRisk.toString()}
          icon={<Boxes className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Regions"
          value={COMPANY_REGION_DIST.length.toString()}
          icon={<Globe className="h-3.5 w-3.5" />}
          accent="#2D8C3C"
        />
      </div>

      {/* ─── Region Distribution Chart ───────────────────────── */}
      <div className="grid gap-4 lg:grid-cols-3 mb-6">
        <div className="lg:col-span-2">
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
                    innerRadius={50}
                    outerRadius={80}
                    paddingAngle={2}
                  >
                    {COMPANY_REGION_DIST.map((entry, index) => (
                      <Cell key={`cell-${index}`} fill={entry.fill} />
                    ))}
                  </Pie>
                  <Tooltip contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }} />
                </PieChart>
              </ResponsiveContainer>
            </div>
            <div className="flex flex-wrap justify-center gap-3 mt-2">
              {COMPANY_REGION_DIST.map((r) => (
                <span key={r.name} className="flex items-center gap-1 text-[10px] font-bold uppercase">
                  <span className="h-2.5 w-2.5 rounded-sm" style={{ backgroundColor: r.fill }} />
                  {r.name} ({r.value})
                </span>
              ))}
            </div>
          </SurfaceCard>
        </div>
        <SurfaceCard title="Quick Links" icon={<Boxes className="h-3.5 w-3.5" />}>
          <div className="space-y-2">
            <p className="text-xs text-muted-foreground mb-3">Access individual company dossiers:</p>
            {COMPANIES.slice(0, 5).map((c) => (
              <Link
                key={c.id}
                href={`/companies/${c.id}`}
                className="flex items-center justify-between rounded-sm border border-border bg-secondary/30 px-3 py-2 text-sm hover:bg-secondary"
              >
                <span className="font-bold">{c.name}</span>
                <span className="text-[10px] text-muted-foreground">T{c.tier}</span>
              </Link>
            ))}
          </div>
        </SurfaceCard>
      </div>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <FilterBar>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine company list:</span>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Region:</span>
        {REGION_FILTERS.map((r) => (
          <FilterChip
            key={r}
            label={r}
            active={regionFilter === r}
            onClick={() => setRegionFilter(regionFilter === r ? null : r)}
          />
        ))}
        <span className="mx-3 h-4 w-px bg-border" />
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Tier:</span>
        {TIER_FILTERS.map((t) => (
          <FilterChip
            key={t}
            label={`T${t}`}
            active={tierFilter === t}
            onClick={() => setTierFilter(tierFilter === t ? null : t)}
          />
        ))}
        <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
        <button
          type="button"
          onClick={() => {
            setRegionFilter(null);
            setTierFilter(null);
          }}
          className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
        >
          Reset Filters
        </button>
      </FilterBar>

      {/* ─── Companies Grid ──────────────────────────────────── */}
      <div className="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {filteredCompanies.length === 0 ? (
          <div className="col-span-full">
            <EmptyState
              title="No companies match current filters"
              description="Try broadening region/tier criteria or use Reset Filters to restore the default company list."
            />
          </div>
        ) : (
          filteredCompanies.map((c) => (
            <Link key={c.id} href={`/companies/${c.id}`}>
              <CompanyCard
                name={c.name}
                domain={c.domain}
                region={c.region}
                type={c.type}
                tier={c.tier}
                riskScore={c.risk_score}
                capabilities={c.capabilities}
              />
            </Link>
          ))
        )}
      </div>
    </div>
  );
}
