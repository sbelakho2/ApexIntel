import Link from "next/link";
import { DataTable, EmptyState, PageHeader } from "@/components/ui";

export default function CompaniesPage() {
  return (
    <div>
      <PageHeader
        title="Company Browser"
        subtitle="Searchable company profiles with dossier links."
      />
      <EmptyState
        title="No companies in current result set"
        description="No company entities matched the current filters and search scope."
      />
      <div className="mt-6 rounded-lg border border-border p-4 text-sm text-muted-foreground">
        Dossier route format: <Link className="text-primary" href="/companies/example-id">/companies/[id]</Link>
      </div>
      <div className="mt-6">
        <DataTable headers={["Name", "Region", "Type", "Threat", "Updated"]} rows={[]} />
      </div>
    </div>
  );
}
