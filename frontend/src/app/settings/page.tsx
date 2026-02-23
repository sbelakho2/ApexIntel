import { EmptyState, PageHeader, SurfaceCard } from "@/components/ui";

export default function SettingsPage() {
  return (
    <div>
      <PageHeader
        title="Settings"
        subtitle="API keys, region coverage, watchlist, and scheduler controls."
      />
      <div className="grid gap-4 lg:grid-cols-2">
        <SurfaceCard title="API Keys">
          <EmptyState
            title="Key inventory"
            description="API key issuance, rotation, and role assignment are managed through controlled operational workflows."
          />
        </SurfaceCard>
        <SurfaceCard title="Operational Controls">
          <EmptyState
            title="Scheduler controls"
            description="Runtime scheduling and operational controls are enforced through worker orchestration policies."
          />
        </SurfaceCard>
      </div>
    </div>
  );
}
