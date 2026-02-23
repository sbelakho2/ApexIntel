import { EmptyState, PageHeader, StatCard, SurfaceCard } from "@/components/ui";

export default function OverviewPage() {
  return (
    <div>
      <PageHeader
        title="Overview"
        subtitle="Top-level KPI view for warnings, insights, POI coverage, and recipe health."
      />

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <StatCard label="Active Warnings" value="0" />
        <StatCard label="Insights (7d)" value="0" />
        <StatCard label="POI Coverage" value="0%" />
        <StatCard label="Recipe Health" value="0%" />
      </div>

      <div className="mt-6 grid gap-4 lg:grid-cols-2">
        <SurfaceCard title="Warning Pipeline">
          <EmptyState
            title="No warning events in current interval"
            description="No warning activity is currently detected for the operational interval being displayed."
          />
        </SurfaceCard>
        <SurfaceCard title="Intelligence Activity">
          <EmptyState
            title="No intelligence events in current interval"
            description="No ranked intelligence records are currently emitted for the active dashboard scope."
          />
        </SurfaceCard>
      </div>
    </div>
  );
}
