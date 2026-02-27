"use client";

import { useState, useMemo } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { PieChart, Pie, Cell, ResponsiveContainer, Tooltip } from "recharts";
import {
  PageHeader, SurfaceCard, CompanyCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, Boxes, Globe
} from "@/components/ui";
import { fetchCompanies } from "@/lib/api";

const REGION_COLORS: Record<string, string> = {
  TN: "#FFBE00", MA: "#D62D2D", IL: "#4A90E2", EU: "#2D8C3C",
  CN: "#999999", US: "#666666", Global: "#333333",
};

export default function CompaniesPage() {
  const [regionFilter, setRegionFilter] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["companies"],
    queryFn: ({ signal }) => fetchCompanies({ signal, per_page: 200 }),
  });

  const companies = data?.items ?? [];

  const filteredCompanies = useMemo(() => {
    if (!regionFilter) return companies;
    return companies.filter((c) => c.region.toLowerCase().includes(regionFilter.toLowerCase()));
  }, [companies, regionFilter]);

  const regionDist = useMemo(() => {
    const map = new Map<string, number>();
    companies.forEach((c) => {
      const key = c.region || "Other";
      map.set(key, (map.get(key) || 0) + 1);
    });
    return Array.from(map.entries()).map(([name, value]) => ({
      name, value, fill: REGION_COLORS[name] || "#888888",
    }));
  }, [companies]);

  const regionFilters = useMemo(() => {
    const regions = new Set(companies.map((c) => c.region));
    return Array.from(regions).sort();
  }, [companies]);

  const competitorCount = companies.filter((c) => c.is_competitor).length;

  return (
    <div>
      <PageHeader title="Company Browser" subtitle="Searchable company profiles with dossier links." icon={PAGE_ICONS.companies} />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard label="Companies Tracked" value={(data?.total ?? 0).toString()} icon={<Boxes className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Competitors" value={competitorCount.toString()} icon={<Boxes className="h-3.5 w-3.5" />} accent="#D62D2D" />
        <StatCard label="Regions" value={regionDist.length.toString()} icon={<Globe className="h-3.5 w-3.5" />} accent="#2D8C3C" />
        <StatCard label="Avg Threat" value={companies.length > 0 ? Math.round(companies.filter((c) => c.threat_score !== null).reduce((s, c) => s + (c.threat_score ?? 0), 0) / Math.max(1, companies.filter((c) => c.threat_score !== null).length)).toString() : "–"} icon={<Boxes className="h-3.5 w-3.5" />} accent="#FFBE00" />
      </div>

      {regionDist.length > 0 && (
        <div className="grid gap-4 lg:grid-cols-3 mb-6">
          <div className="lg:col-span-2">
            <SurfaceCard title="Coverage by Region" icon={<Globe className="h-3.5 w-3.5" />}>
              <div className="h-48">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie data={regionDist} dataKey="value" nameKey="name" cx="50%" cy="50%" innerRadius={50} outerRadius={80} paddingAngle={2}>
                      {regionDist.map((entry, index) => (<Cell key={`cell-${index}`} fill={entry.fill} />))}
                    </Pie>
                    <Tooltip contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }} />
                  </PieChart>
                </ResponsiveContainer>
              </div>
              <div className="flex flex-wrap justify-center gap-3 mt-2">
                {regionDist.map((r) => (
                  <span key={r.name} className="flex items-center gap-1 text-[10px] font-bold uppercase">
                    <span className="h-2.5 w-2.5 rounded-sm" style={{ backgroundColor: r.fill }} />{r.name} ({r.value})
                  </span>
                ))}
              </div>
            </SurfaceCard>
          </div>
          <SurfaceCard title="Quick Links" icon={<Boxes className="h-3.5 w-3.5" />}>
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground mb-3">Access individual company dossiers:</p>
              {companies.slice(0, 5).map((c) => (
                <Link key={c.id} href={`/companies/${c.id}`} className="flex items-center justify-between rounded-sm border border-border bg-secondary/30 px-3 py-2 text-sm hover:bg-secondary">
                  <span className="font-bold">{c.name}</span>
                  <span className="text-[10px] text-muted-foreground">{c.region}</span>
                </Link>
              ))}
            </div>
          </SurfaceCard>
        </div>
      )}

      {regionFilters.length > 0 && (
        <FilterBar>
          <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Region:</span>
          {regionFilters.map((r) => (
            <FilterChip key={r} label={r} active={regionFilter === r} onClick={() => setRegionFilter(regionFilter === r ? null : r)} />
          ))}
          <button type="button" onClick={() => setRegionFilter(null)}
            className="ml-2 rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground">
            Reset
          </button>
        </FilterBar>
      )}

      <div className="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {isLoading ? (
          <div className="col-span-full"><SurfaceCard title="Loading"><p className="text-xs text-muted-foreground animate-pulse">Loading companies…</p></SurfaceCard></div>
        ) : filteredCompanies.length === 0 ? (
          <div className="col-span-full">
            <EmptyState title="No companies found" description={companies.length === 0 ? "No companies have been ingested yet. Companies populate as crawlers analyze entity data." : "Try broadening your filter."} />
          </div>
        ) : (
          filteredCompanies.map((c) => (
            <Link key={c.id} href={`/companies/${c.id}`}>
              <CompanyCard name={c.name} domain="" region={c.region} type={c.entity_type} tier={1} riskScore={c.threat_score ?? 0} capabilities={c.capabilities} />
            </Link>
          ))
        )}
      </div>
    </div>
  );
}
