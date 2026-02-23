import Link from "next/link";
import { DataTable, EmptyState, PageHeader } from "@/components/ui";

export default function PersonsPage() {
  return (
    <div>
      <PageHeader
        title="POI Browser"
        subtitle="Searchable person-of-interest profiles filtered by region and role."
      />
      <EmptyState
        title="No POIs in current result set"
        description="No people-of-interest entities matched the active filters and search scope."
      />
      <div className="mt-6 rounded-lg border border-border p-4 text-sm text-muted-foreground">
        Dossier route format: <Link className="text-primary" href="/persons/example-id">/persons/[id]</Link>
      </div>
      <div className="mt-6">
        <DataTable headers={["Name", "Role", "Organization", "Region", "Priority"]} rows={[]} />
      </div>
    </div>
  );
}
