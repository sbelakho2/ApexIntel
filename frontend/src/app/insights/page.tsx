import { DataTable, EmptyState, PageHeader } from "@/components/ui";

export default function InsightsPage() {
  return (
    <div>
      <PageHeader
        title="Insight Feed"
        subtitle="Ranked insights with evidence links and context."
      />
      <EmptyState
        title="No ranked insights in this interval"
        description="The current filters returned no insight records for the selected time horizon."
      />
      <div className="mt-6">
        <DataTable
          headers={["Title", "Type", "Region", "Confidence", "Created"]}
          rows={[]}
        />
      </div>
    </div>
  );
}
