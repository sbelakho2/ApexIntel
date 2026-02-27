"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import {
  DataTable,
  EmptyState,
  PageHeader,
  ProgressBar,
  SurfaceCard,
  StatCard,
  StatusBadge,
  PAGE_ICONS,
  TrendingUp,
  Users,
  Globe,
  Boxes,
} from "@/components/ui";
import { CopyIdButton } from "@/components/copy-id-button";
import { fetchPerson } from "@/lib/api";

function formatDate(value?: string) {
  if (!value) return "–";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "–" : date.toLocaleDateString();
}

export default function PersonDossierClient({ id }: { id: string }) {
  const { data, isLoading, error } = useQuery({
    queryKey: ["person", id],
    queryFn: ({ signal }) => fetchPerson({ id, signal }),
  });

  const priorityPct = data ? Math.round(data.priority_score * 100) : null;

  return (
    <div>
      <PageHeader
        title={`POI Profile: ${data?.name ?? id}`}
        subtitle="Full POI profile, priority vector, and engagement guide."
        icon={PAGE_ICONS.persons}
      />
      <div className="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-bold uppercase tracking-[0.1em]">ID:</span>
        <span className="max-w-[320px] truncate font-mono" title={id}>{id}</span>
        <CopyIdButton idValue={id} />
      </div>

      {isLoading ? (
        <SurfaceCard title="POI Profile">
          <p className="text-xs text-muted-foreground animate-pulse">Loading POI dossier...</p>
        </SurfaceCard>
      ) : error || !data ? (
        <EmptyState
          title="POI dossier currently unavailable"
          description="No dossier package is currently published for this person identifier."
        />
      ) : (
        <>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
            <StatCard label="Priority" value={priorityPct !== null ? `${priorityPct}%` : "–"} icon={<TrendingUp className="h-3.5 w-3.5" />} accent="#D62D2D" />
            <StatCard label="Region" value={data.region || "–"} icon={<Globe className="h-3.5 w-3.5" />} accent="#4A90E2" />
            <StatCard label="Engagement" value={data.engagement_status || "–"} icon={<Users className="h-3.5 w-3.5" />} accent="#FFBE00" />
            <StatCard label="Affiliations" value={data.affiliations.length.toString()} icon={<Boxes className="h-3.5 w-3.5" />} accent="#2D8C3C" />
          </div>

          <SurfaceCard title="POI Profile" icon={<Users className="h-3.5 w-3.5" />}>
            <div className="grid gap-3 text-sm">
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Role</span>
                <span className="font-semibold">{data.role || "–"}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Organization</span>
                <span className="font-semibold">{data.organization || "–"}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Country</span>
                <span className="font-semibold">{data.country || "–"}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Engagement</span>
                <StatusBadge status={data.engagement_status || "untracked"} variant="info" />
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Updated</span>
                <span className="font-semibold">{formatDate(data.updated_at)}</span>
              </div>
            </div>
          </SurfaceCard>

          <div className="grid gap-4 lg:grid-cols-2 mt-6">
            <SurfaceCard title="Priority Vector" icon={<TrendingUp className="h-3.5 w-3.5" />}>
              <div className="space-y-4">
                <ProgressBar value={data.priority_vector.decision_power * 100} label="Decision Power" />
                <ProgressBar value={data.priority_vector.domain_relevance * 100} label="Domain Relevance" color="#4A90E2" />
                <ProgressBar value={data.priority_vector.network_centrality * 100} label="Network Centrality" color="#2D8C3C" />
                <ProgressBar value={data.priority_vector.engagement_potential * 100} label="Engagement Potential" color="#FFBE00" />
                <ProgressBar value={data.priority_vector.intelligence_value * 100} label="Intelligence Value" color="#D62D2D" />
              </div>
            </SurfaceCard>

            <SurfaceCard title="Contact & Tags" icon={<Boxes className="h-3.5 w-3.5" />}>
              <div className="grid gap-2 text-sm">
                <div className="flex items-center justify-between">
                  <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Email</span>
                  <span className="font-semibold">{data.email || "–"}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Phone</span>
                  <span className="font-semibold">{data.phone || "–"}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">LinkedIn</span>
                  <span className="font-semibold">{data.linkedin || "–"}</span>
                </div>
              </div>
              <div className="mt-4 flex flex-wrap gap-2">
                {data.tags.length === 0 ? (
                  <span className="text-xs text-muted-foreground">No tags assigned.</span>
                ) : (
                  data.tags.map((tag) => (
                    <span key={tag} className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-bold uppercase tracking-[0.1em]">
                      {tag}
                    </span>
                  ))
                )}
              </div>
            </SurfaceCard>
          </div>

          <div className="grid gap-4 lg:grid-cols-2 mt-6">
            <SurfaceCard title="Affiliations" icon={<Users className="h-3.5 w-3.5" />}>
              {data.affiliations.length === 0 ? (
                <EmptyState title="No affiliations" description="No affiliations are currently associated with this POI." />
              ) : (
                <DataTable
                  headers={["Organization", "Role", "Current"]}
                  rows={data.affiliations.map((aff) => [
                    aff.organization,
                    aff.role,
                    aff.current ? "Yes" : "No",
                  ])}
                />
              )}
            </SurfaceCard>

            <SurfaceCard title="Timeline" icon={<Boxes className="h-3.5 w-3.5" />}>
              {data.timeline.length === 0 ? (
                <EmptyState title="No timeline events" description="No recent events are recorded for this person." />
              ) : (
                <DataTable
                  headers={["Date", "Type", "Details"]}
                  rows={data.timeline.map((event) => [
                    formatDate(event.date),
                    event.event_type,
                    event.source_url ? (
                      <Link key={event.source_url} href={{ pathname: event.source_url }} className="text-xs font-bold uppercase tracking-[0.1em] text-primary">
                        Source
                      </Link>
                    ) : (
                      event.description
                    ),
                  ])}
                />
              )}
            </SurfaceCard>
          </div>
        </>
      )}
    </div>
  );
}
