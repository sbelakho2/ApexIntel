"use client";

import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, EmptyState,
  PAGE_ICONS, AlertTriangle, Eye
} from "@/components/ui";
import { fetchInsights, fetchWarnings } from "@/lib/api";

export default function MemosPage() {
  const { data: insightsData } = useQuery({
    queryKey: ["insights", "memos"],
    queryFn: ({ signal }) => fetchInsights({ signal, per_page: 10 }),
  });
  const { data: warningsData } = useQuery({
    queryKey: ["warnings", "memos"],
    queryFn: ({ signal }) => fetchWarnings({ signal, per_page: 10 }),
  });

  const insights = insightsData?.items ?? [];
  const warnings = warningsData?.items ?? [];

  return (
    <div>
      <PageHeader
        title="Strategy Memos"
        subtitle="Weekly strategy memos for executive review."
        icon={PAGE_ICONS.memos}
      />

      <div className="grid gap-4 lg:grid-cols-2">
        <SurfaceCard title="Latest Insights" icon={<Eye className="h-3.5 w-3.5" />}>
          {insights.length === 0 ? (
            <EmptyState
              title="No insights available"
              description="Insights will appear here as analysis recipes generate new signals."
            />
          ) : (
            <ol className="space-y-3">
              {insights.slice(0, 5).map((i) => (
                <li key={i.id} className="rounded-sm border border-border bg-secondary/30 px-3 py-2">
                  <p className="text-xs font-bold uppercase tracking-[0.08em] text-muted-foreground">{i.insight_type.replace(/_/g, " ")}</p>
                  <p className="text-sm font-bold text-foreground mt-1">{i.title}</p>
                  <p className="text-xs text-muted-foreground mt-1 line-clamp-2">{i.summary}</p>
                </li>
              ))}
            </ol>
          )}
        </SurfaceCard>

        <SurfaceCard title="Critical Warnings" icon={<AlertTriangle className="h-3.5 w-3.5" />}>
          {warnings.length === 0 ? (
            <EmptyState
              title="No warnings yet"
              description="Warnings will appear here once monitoring detects priority events."
            />
          ) : (
            <ol className="space-y-3">
              {warnings.slice(0, 5).map((w) => (
                <li key={w.id} className="rounded-sm border border-border bg-secondary/30 px-3 py-2">
                  <p className="text-xs font-bold uppercase tracking-[0.08em] text-muted-foreground">{w.warning_type.replace(/_/g, " ")}</p>
                  <p className="text-sm font-bold text-foreground mt-1">{w.title}</p>
                  <p className="text-xs text-muted-foreground mt-1 line-clamp-2">{w.description}</p>
                </li>
              ))}
            </ol>
          )}
        </SurfaceCard>
      </div>
    </div>
  );
}
