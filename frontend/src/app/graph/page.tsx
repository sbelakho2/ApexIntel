"use client";

import { useQuery } from "@tanstack/react-query";
import {
  PageHeader, SurfaceCard, StatCard, EmptyState, DataTable,
  PAGE_ICONS, Zap, Boxes, Users, Eye, Radio
} from "@/components/ui";
import { fetchGraph } from "@/lib/api";

export default function GraphPage() {
  const { data: graphData, isLoading } = useQuery({
    queryKey: ["graph"],
    queryFn: ({ signal }) => fetchGraph({ signal }),
  });

  const edgeTypeRows = (graphData?.edge_type_counts ?? []).map((et) => [
    et.edge_type,
    et.count.toString(),
  ]);

  const edgeRows = (graphData?.edges ?? []).slice(0, 50).map((e) => [
    e.source.slice(0, 8) + "…",
    e.target.slice(0, 8) + "…",
    e.edge_type,
    e.weight.toFixed(2),
  ]);

  return (
    <div>
      <PageHeader
        title="Graph Explorer"
        subtitle="Entity relationship graph with edges connecting companies, persons, and signals."
        icon={PAGE_ICONS.graph}
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5 mb-6">
        <StatCard label="Companies" value={(graphData?.companies_total ?? 0).toString()} icon={<Boxes className="h-3.5 w-3.5" />} accent="#4A90E2" />
        <StatCard label="POIs" value={(graphData?.persons_total ?? 0).toString()} icon={<Users className="h-3.5 w-3.5" />} accent="#2D8C3C" />
        <StatCard label="Warnings" value={(graphData?.warnings_total ?? 0).toString()} icon={<Zap className="h-3.5 w-3.5" />} accent="#D62D2D" />
        <StatCard label="Insights" value={(graphData?.insights_total ?? 0).toString()} icon={<Eye className="h-3.5 w-3.5" />} accent="#FFBE00" />
        <StatCard label="Edges" value={(graphData?.edges_total ?? 0).toString()} icon={<Radio className="h-3.5 w-3.5" />} accent="#9333EA" />
      </div>

      <div className="grid gap-4 lg:grid-cols-3 mb-6">
        <SurfaceCard title="Edge Types" icon={<Radio className="h-3.5 w-3.5" />}>
          {isLoading ? (
            <p className="text-xs text-muted-foreground animate-pulse">Loading graph data…</p>
          ) : edgeTypeRows.length === 0 ? (
            <EmptyState
              title="No edges yet"
              description="Graph edges will appear once entity relationships are discovered."
            />
          ) : (
            <DataTable headers={["Type", "Count"]} rows={edgeTypeRows} />
          )}
        </SurfaceCard>

        <div className="lg:col-span-2">
          <SurfaceCard title="Recent Edges" icon={<Zap className="h-3.5 w-3.5" />}>
            {isLoading ? (
              <p className="text-xs text-muted-foreground animate-pulse">Loading edges…</p>
            ) : edgeRows.length === 0 ? (
              <EmptyState
                title="No edges to display"
                description="Edges connect entities in the knowledge graph."
              />
            ) : (
              <DataTable headers={["Source", "Target", "Type", "Weight"]} rows={edgeRows} />
            )}
          </SurfaceCard>
        </div>
      </div>
    </div>
  );
}
