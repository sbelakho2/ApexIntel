"use client";

import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, WarningCard, StatCard, EmptyState,
  PAGE_ICONS, Shield, AlertTriangle
} from "@/components/ui";
import { fetchWarnings } from "@/lib/api";

export default function SecurityPage() {
  const { data, isLoading } = useQuery({
    queryKey: ["warnings", "security"],
    queryFn: ({ signal }) => fetchWarnings({ signal, warning_types: "security", per_page: 100 }),
  });

  const warnings = data?.items ?? [];
  const active = warnings.filter((w) => !w.acknowledged);

  return (
    <div>
      <PageHeader
        title="Security Center"
        subtitle="Security warnings for DNS posture, lookalikes, and KEV relevance."
        icon={PAGE_ICONS.security}
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 mb-6">
        <StatCard label="Security Warnings" value={(data?.total ?? 0).toString()} icon={<Shield className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="Active" value={active.length.toString()} icon={<AlertTriangle className="h-3.5 w-3.5" />} accent="#D62D2D" />
        <StatCard label="Acknowledged" value={warnings.filter((w) => w.acknowledged).length.toString()} icon={<Shield className="h-3.5 w-3.5" />} accent="#2D8C3C" />
      </div>

      <SurfaceCard title="Security Findings" icon={<Shield className="h-3.5 w-3.5" />}>
        {isLoading ? (
          <p className="text-xs text-muted-foreground animate-pulse">Loading security warnings…</p>
        ) : warnings.length === 0 ? (
          <EmptyState
            title="No security warnings yet"
            description="Security warnings will appear here when DNS posture, lookalike domain, or KEV checks identify issues."
          />
        ) : (
          <div className="space-y-3">
            {warnings.map((w) => (
              <WarningCard
                key={w.id}
                severity={w.severity as "critical" | "high" | "medium" | "low"}
                title={w.title}
                region={w.region}
                type={w.warning_type}
                created={w.created_at}
                status={w.acknowledged ? "acknowledged" : "active"}
              />
            ))}
          </div>
        )}
      </SurfaceCard>
    </div>
  );
}
