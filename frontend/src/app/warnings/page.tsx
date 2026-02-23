import { DataTable, EmptyState, PageHeader } from "@/components/ui";

export default function WarningsPage() {
  return (
    <div>
      <PageHeader
        title="Warning Center"
        subtitle="Real-time warnings with severity, type, and region filters."
      />
      <EmptyState
        title="Warning stream currently clear"
        description="No active warning records are returned for the selected operational window and filters."
      />
      <div className="mt-6">
        <DataTable
          headers={["Severity", "Type", "Region", "Title", "Created", "Status"]}
          rows={[]}
        />
      </div>
    </div>
  );
}
