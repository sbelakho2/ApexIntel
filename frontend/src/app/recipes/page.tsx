import { DataTable, EmptyState, PageHeader } from "@/components/ui";

export default function RecipesPage() {
  return (
    <div>
      <PageHeader
        title="Recipe Manager"
        subtitle="Production, staging, and deprecated recipe lifecycle with performance metrics."
      />
      <EmptyState
        title="No recipe metrics in current scope"
        description="No recipe performance records are currently returned for the selected filters."
      />
      <div className="mt-6">
        <DataTable
          headers={["Recipe", "Status", "Precision", "Recall", "FPR", "Alerts"]}
          rows={[]}
        />
      </div>
    </div>
  );
}
