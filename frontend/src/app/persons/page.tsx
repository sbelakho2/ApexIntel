"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, EmptyState, FilterBar, FilterChip, StatCard, DataTable,
  PAGE_ICONS, Users, Globe, TrendingUp
} from "@/components/ui";
import { fetchPersons } from "@/lib/api";

export default function PersonsPage() {
  const router = useRouter();
  const [regionFilter, setRegionFilter] = useState<string | null>(null);
  const [roleFilter, setRoleFilter] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["persons"],
    queryFn: ({ signal }) => fetchPersons({ signal, per_page: 200, sort_by: "priority" }),
  });

  const persons = data?.items ?? [];

  const filtered = useMemo(() => {
    return persons.filter((p) => {
      if (regionFilter && p.region !== regionFilter) return false;
      if (roleFilter && p.role !== roleFilter) return false;
      return true;
    });
  }, [persons, regionFilter, roleFilter]);

  const regionFilters = useMemo(() => {
    const regions = new Set(persons.map((p) => p.region).filter(Boolean));
    return Array.from(regions).sort();
  }, [persons]);

  const roleFilters = useMemo(() => {
    const roles = new Set(persons.map((p) => p.role).filter(Boolean));
    return Array.from(roles).sort();
  }, [persons]);

  const avgPriority = persons.length > 0
    ? Math.round((persons.reduce((sum, p) => sum + p.priority_score, 0) / persons.length) * 100)
    : 0;

  const rows = filtered.map((p) => [
    p.name,
    p.role || "Unknown",
    p.organization || "Independent",
    p.region || "–",
    `${Math.round(p.priority_score * 100)}%`,
    p.engagement_status || "untracked",
  ]);

  return (
    <div>
      <PageHeader
        title="POI Browser"
        subtitle="Searchable person-of-interest profiles filtered by region and role."
        icon={PAGE_ICONS.persons}
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 mb-6">
        <StatCard label="Total POIs" value={(data?.total ?? 0).toString()} icon={<Users className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Regions" value={regionFilters.length.toString()} icon={<Globe className="h-3.5 w-3.5" />} accent="#2D8C3C" />
        <StatCard label="Avg Priority" value={avgPriority > 0 ? `${avgPriority}%` : "–"} icon={<TrendingUp className="h-3.5 w-3.5" />} accent="#FFBE00" />
      </div>

      {(regionFilters.length > 0 || roleFilters.length > 0) && (
        <div className="mb-4">
          <FilterBar>
            <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Region:</span>
            {regionFilters.map((r) => (
              <FilterChip key={r} label={r} active={regionFilter === r} onClick={() => setRegionFilter(regionFilter === r ? null : r)} />
            ))}
            <span className="mx-3 h-4 w-px bg-border" />
            <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Role:</span>
            {roleFilters.map((r) => (
              <FilterChip key={r} label={r} active={roleFilter === r} onClick={() => setRoleFilter(roleFilter === r ? null : r)} />
            ))}
            <button
              type="button"
              onClick={() => { setRegionFilter(null); setRoleFilter(null); }}
              className="ml-2 rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
            >
              Reset
            </button>
          </FilterBar>
        </div>
      )}

      <SurfaceCard title="Persons of Interest" icon={<Users className="h-3.5 w-3.5" />}>
        {isLoading ? (
          <p className="text-xs text-muted-foreground animate-pulse">Loading POIs…</p>
        ) : filtered.length === 0 ? (
          <EmptyState
            title="No POIs match current filters"
            description={persons.length === 0 ? "No POI profiles are currently available." : "Try adjusting region or role filters."}
          />
        ) : (
          <DataTable
            headers={["Name", "Role", "Organization", "Region", "Priority", "Status"]}
            rows={rows}
            onRowClick={(idx) => {
              const target = filtered[idx];
              if (target) router.push(`/persons/${target.id}`);
            }}
          />
        )}
      </SurfaceCard>
    </div>
  );
}
