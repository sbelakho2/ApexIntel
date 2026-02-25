"use client";

import { useState, useMemo } from "react";
import Link from "next/link";
import { BarChart, Bar, XAxis, YAxis, Tooltip, ResponsiveContainer, CartesianGrid } from "recharts";
import {
  PageHeader, SurfaceCard, PersonCard, FilterBar, FilterChip, StatCard, EmptyState,
  PAGE_ICONS, Users
} from "@/components/ui";
import { PERSONS } from "@/lib/mock-data";

const REGION_FILTERS = ["Tunisia", "Morocco", "Israel", "EU", "China", "Global"] as const;
const PRIORITY_FILTERS = ["A", "B", "C"] as const;

export default function PersonsPage() {
  const [regionFilter, setRegionFilter] = useState<string | null>(null);
  const [priorityFilter, setPriorityFilter] = useState<string | null>(null);

  const filteredPersons = useMemo(() => {
    return PERSONS.filter((p) => {
      if (regionFilter && !p.region.toLowerCase().includes(regionFilter.toLowerCase())) return false;
      if (priorityFilter && p.priority !== priorityFilter) return false;
      return true;
    });
  }, [regionFilter, priorityFilter]);

  const avgInfluence = PERSONS.length > 0
    ? Math.round(PERSONS.reduce((sum, p) => sum + p.influence_score, 0) / PERSONS.length)
    : 0;

  const priorityCounts = {
    A: PERSONS.filter((p) => p.priority === "A").length,
    B: PERSONS.filter((p) => p.priority === "B").length,
    C: PERSONS.filter((p) => p.priority === "C").length,
  };
  const activeFilterCount = Number(Boolean(regionFilter)) + Number(Boolean(priorityFilter));

  /* Use influence distribution for bar chart */
  const influenceGroups = [
    { range: "0-20", count: PERSONS.filter((p) => p.influence_score < 20).length },
    { range: "20-40", count: PERSONS.filter((p) => p.influence_score >= 20 && p.influence_score < 40).length },
    { range: "40-60", count: PERSONS.filter((p) => p.influence_score >= 40 && p.influence_score < 60).length },
    { range: "60-80", count: PERSONS.filter((p) => p.influence_score >= 60 && p.influence_score < 80).length },
    { range: "80-100", count: PERSONS.filter((p) => p.influence_score >= 80).length },
  ];

  return (
    <div>
      <PageHeader
        title="POI Browser"
        subtitle="Searchable person-of-interest profiles filtered by region and role."
        icon={PAGE_ICONS.persons}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Persons Tracked"
          value={PERSONS.length.toString()}
          icon={<Users className="h-3.5 w-3.5" />}
          accent="#4A90E2"
        />
        <StatCard
          label="Priority A"
          value={priorityCounts.A.toString()}
          icon={<Users className="h-3.5 w-3.5" />}
          accent="#D62D2D"
        />
        <StatCard
          label="Priority B"
          value={priorityCounts.B.toString()}
          icon={<Users className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
        <StatCard
          label="Avg Influence"
          value={avgInfluence.toString()}
          icon={<Users className="h-3.5 w-3.5" />}
          accent="#2D8C3C"
        />
      </div>

      {/* ─── Chart ───────────────────────────────────────────── */}
      <SurfaceCard title="Influence Score Distribution" icon={<Users className="h-3.5 w-3.5" />}>
        <div className="h-40 mt-2">
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={influenceGroups} margin={{ top: 8, right: 16, left: 0, bottom: 0 }}>
              <CartesianGrid strokeDasharray="3 3" stroke="#ccc" />
              <XAxis dataKey="range" tick={{ fontSize: 10 }} label={{ value: "Influence band", position: "insideBottom", offset: -4, style: { fontSize: 10, fill: "#888" } }} />
              <YAxis tick={{ fontSize: 10 }} allowDecimals={false} label={{ value: "POI count", angle: -90, position: "insideLeft", style: { fontSize: 10, fill: "#888" } }} />
              <Tooltip
                formatter={(value: number) => [`${value.toLocaleString("en-US")} people`, "Count"]}
                contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }}
              />
              <Bar dataKey="count" fill="#4A90E2" radius={[2, 2, 0, 0]} />
            </BarChart>
          </ResponsiveContainer>
        </div>
      </SurfaceCard>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <FilterBar>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine POI list:</span>
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
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Priority:</span>
        {PRIORITY_FILTERS.map((p) => (
          <FilterChip
            key={p}
            label={p}
            active={priorityFilter === p}
            onClick={() => setPriorityFilter(priorityFilter === p ? null : p)}
          />
        ))}
        <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
        <button
          type="button"
          onClick={() => {
            setRegionFilter(null);
            setPriorityFilter(null);
          }}
          className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
        >
          Reset Filters
        </button>
      </FilterBar>

      {/* ─── Persons Grid ────────────────────────────────────── */}
      <div className="mt-4 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {filteredPersons.length === 0 ? (
          <div className="col-span-full">
            <EmptyState
              title="No persons match current filters"
              description="Try broadening region/priority filters or reset filters to review all tracked POIs."
            />
          </div>
        ) : (
          filteredPersons.map((p) => (
            <Link key={p.id} href={`/persons/${p.id}`}>
              <PersonCard
                name={p.name}
                role={p.role}
                organization={p.organization}
                region={p.region}
                priority={p.priority}
                influenceScore={p.influence_score}
                tags={p.tags}
              />
            </Link>
          ))
        )}
      </div>
    </div>
  );
}
