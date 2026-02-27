"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import {
  DataTable,
  EmptyState,
  PageHeader,
  SurfaceCard,
  StatCard,
  StatusBadge,
  PAGE_ICONS,
  Boxes,
  Globe,
  Users,
  TrendingUp,
} from "@/components/ui";
import { CopyIdButton } from "@/components/copy-id-button";
import { fetchCompany } from "@/lib/api";

function formatDate(value?: string) {
  if (!value) return "–";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "–" : date.toLocaleDateString();
}

export default function CompanyDossierClient({ id }: { id: string }) {
  const { data, isLoading, error } = useQuery({
    queryKey: ["company", id],
    queryFn: ({ signal }) => fetchCompany({ id, signal }),
  });

  const threatPct = data?.threat_score !== null && data?.threat_score !== undefined
    ? Math.round(data.threat_score * 100)
    : null;
  const overlapPct = data?.overlap_score !== null && data?.overlap_score !== undefined
    ? Math.round(data.overlap_score * 100)
    : null;

  return (
    <div>
      <PageHeader
        title={`Company Dossier: ${data?.name ?? id}`}
        subtitle="Full company profile, capabilities, sites, and graph relationships."
        icon={PAGE_ICONS.companies}
      />
      <div className="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-bold uppercase tracking-[0.1em]">ID:</span>
        <span className="max-w-[320px] truncate font-mono" title={id}>{id}</span>
        <CopyIdButton idValue={id} />
      </div>

      {isLoading ? (
        <SurfaceCard title="Company Profile">
          <p className="text-xs text-muted-foreground animate-pulse">Loading company dossier...</p>
        </SurfaceCard>
      ) : error || !data ? (
        <EmptyState
          title="Company dossier currently unavailable"
          description="No dossier document is currently published for this company identifier."
        />
      ) : (
        <>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
            <StatCard label="Threat Score" value={threatPct !== null ? `${threatPct}%` : "–"} icon={<TrendingUp className="h-3.5 w-3.5" />} accent="#D62D2D" />
            <StatCard label="Overlap" value={overlapPct !== null ? `${overlapPct}%` : "–"} icon={<TrendingUp className="h-3.5 w-3.5" />} accent="#4A90E2" />
            <StatCard label="Sites" value={data.sites.length.toString()} icon={<Globe className="h-3.5 w-3.5" />} accent="#2D8C3C" />
            <StatCard label="Key Persons" value={data.key_persons.length.toString()} icon={<Users className="h-3.5 w-3.5" />} accent="#FFBE00" />
          </div>

          <SurfaceCard title="Company Profile" icon={<Boxes className="h-3.5 w-3.5" />}>
            <div className="grid gap-3 text-sm">
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Entity Type</span>
                <span className="font-semibold">{data.entity_type}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Region</span>
                <span className="font-semibold">{data.region || "–"}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Country</span>
                <span className="font-semibold">{data.country || "–"}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Website</span>
                <span className="font-semibold">{data.website || "–"}</span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Competitor</span>
                <StatusBadge status={data.is_competitor ? "Yes" : "No"} variant={data.is_competitor ? "warning" : "muted"} />
              </div>
              <div className="flex items-center justify-between">
                <span className="text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">Updated</span>
                <span className="font-semibold">{formatDate(data.updated_at)}</span>
              </div>
            </div>
          </SurfaceCard>

          <div className="grid gap-4 lg:grid-cols-2 mt-6">
            <SurfaceCard title="Capabilities" icon={<Boxes className="h-3.5 w-3.5" />}>
              {data.capabilities.length === 0 ? (
                <EmptyState title="No capabilities" description="No capabilities are recorded for this company yet." />
              ) : (
                <div className="flex flex-wrap gap-2">
                  {data.capabilities.map((cap) => (
                    <span key={cap} className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-bold uppercase tracking-[0.1em]">
                      {cap}
                    </span>
                  ))}
                </div>
              )}
            </SurfaceCard>

            <SurfaceCard title="Certifications" icon={<Boxes className="h-3.5 w-3.5" />}>
              {data.certifications.length === 0 ? (
                <EmptyState title="No certifications" description="No certifications have been linked to this company." />
              ) : (
                <div className="flex flex-wrap gap-2">
                  {data.certifications.map((cert) => (
                    <span key={cert} className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-bold uppercase tracking-[0.1em]">
                      {cert}
                    </span>
                  ))}
                </div>
              )}
            </SurfaceCard>
          </div>

          <div className="grid gap-4 lg:grid-cols-2 mt-6">
            <SurfaceCard title="Sites" icon={<Globe className="h-3.5 w-3.5" />}>
              {data.sites.length === 0 ? (
                <EmptyState title="No sites" description="No sites are currently associated with this company." />
              ) : (
                <DataTable
                  headers={["Site", "Location", "Type"]}
                  rows={data.sites.map((site) => [site.name, site.location, site.site_type])}
                />
              )}
            </SurfaceCard>

            <SurfaceCard title="Key Persons" icon={<Users className="h-3.5 w-3.5" />}>
              {data.key_persons.length === 0 ? (
                <EmptyState title="No key persons" description="No key persons have been linked to this company." />
              ) : (
                <DataTable
                  headers={["Name", "Role", "Profile"]}
                  rows={data.key_persons.map((person) => [
                    person.name,
                    person.role,
                    <Link key={person.person_id} href={`/persons/${person.person_id}`} className="text-xs font-bold uppercase tracking-[0.1em] text-primary">
                      View
                    </Link>,
                  ])}
                />
              )}
            </SurfaceCard>
          </div>

          <SurfaceCard title="Recent Events" icon={<Boxes className="h-3.5 w-3.5" />}>
            {data.recent_events.length === 0 ? (
              <EmptyState title="No recent events" description="No recent events are recorded for this company." />
            ) : (
              <DataTable
                headers={["Date", "Type", "Description"]}
                rows={data.recent_events.map((event) => [
                  formatDate(event.date),
                  event.event_type,
                  event.description,
                ])}
              />
            )}
          </SurfaceCard>
        </>
      )}
    </div>
  );
}
