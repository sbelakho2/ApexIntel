import { DataTable, EmptyState, PageHeader } from "@/components/ui";

export default function CompetitorsPage() {
  return (
    <div>
      <PageHeader
        title="Competitor Dashboard"
        subtitle="Threat and overlap tracking with competitor change monitoring."
      />
      <EmptyState
        title="No competitor events in current scope"
        description="No competitor records are currently returned for the active timeframe and filters."
      />
      <div className="mt-6">
        <DataTable
          headers={["Competitor", "Threat Score", "Overlap", "Latest Change", "Updated"]}
          rows={[]}
        />
      </div>
    </div>
  );
}
