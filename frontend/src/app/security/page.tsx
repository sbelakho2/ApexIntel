import { DataTable, EmptyState, PageHeader } from "@/components/ui";

export default function SecurityPage() {
  return (
    <div>
      <PageHeader
        title="Security Center"
        subtitle="DNS posture, lookalike domains, and KEV relevance monitoring."
      />
      <EmptyState
        title="No security findings in current window"
        description="No DNS posture, lookalike, or KEV relevance findings are currently returned for the active view."
      />
      <div className="mt-6">
        <DataTable
          headers={["Domain", "Risk", "Signal", "Region", "Detected"]}
          rows={[]}
        />
      </div>
    </div>
  );
}
