import type { Metadata } from "next";
import { EmptyState, PageHeader, SurfaceCard } from "@/components/ui";
import { CopyIdButton } from "@/components/copy-id-button";

export function generateMetadata({ params }: { params: { id: string } }): Metadata {
  return {
    title: `POI ${params.id} — ApexIntel`,
    description: `Person-of-interest dossier for ${params.id} — priority vector, engagement guide, and profile.`,
    openGraph: {
      title: `POI ${params.id} — ApexIntel`,
      description: `POI dossier for ${params.id}`,
      type: "website",
    },
  };
}

export default function PersonDossierPage({ params }: { params: { id: string } }) {
  return (
    <div>
      <PageHeader
          title={`POI Profile: ${params.id}`}
        subtitle="Full POI profile, priority vector, and engagement guide."
      />
        <div className="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
          <span className="font-bold uppercase tracking-[0.1em]">ID:</span>
          <span className="max-w-[320px] truncate font-mono" title={params.id}>{params.id}</span>
          <CopyIdButton idValue={params.id} />
        </div>
        <nav aria-label="In-page sections" className="mb-4 flex items-center gap-2 text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">
          <span>Jump to:</span>
          <a href="#poi-profile" className="rounded-sm border border-border bg-secondary px-2 py-1 hover:text-foreground">POI Profile</a>
        </nav>
        <SurfaceCard title="POI Profile">
          <div id="poi-profile" />
        <EmptyState
          title="POI dossier currently unavailable"
          description="No dossier package is currently published for this person identifier."
        />
      </SurfaceCard>
    </div>
  );
}
