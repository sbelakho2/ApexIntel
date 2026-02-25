import type { Metadata } from "next";
import { EmptyState, PageHeader, SurfaceCard } from "@/components/ui";
import { CopyIdButton } from "@/components/copy-id-button";

export function generateMetadata({ params }: { params: { id: string } }): Metadata {
  return {
    title: `Company ${params.id} — ApexIntel`,
    description: `Company dossier for ${params.id} — profile, capabilities, sites, and graph relationships.`,
    openGraph: {
      title: `Company ${params.id} — ApexIntel`,
      description: `Company dossier for ${params.id}`,
      type: "website",
    },
  };
}

export default function CompanyDossierPage({ params }: { params: { id: string } }) {
  return (
    <div>
      <PageHeader
        title={`Company Dossier: ${params.id}`}
        subtitle="Full company profile, capabilities, sites, and graph relationships."
      />
      <div className="mb-3 flex items-center gap-2 text-xs text-muted-foreground">
        <span className="font-bold uppercase tracking-[0.1em]">ID:</span>
        <span className="max-w-[320px] truncate font-mono" title={params.id}>{params.id}</span>
        <CopyIdButton idValue={params.id} />
      </div>
      <nav aria-label="In-page sections" className="mb-4 flex items-center gap-2 text-xs font-bold uppercase tracking-[0.1em] text-muted-foreground">
        <span>Jump to:</span>
        <a href="#company-profile" className="rounded-sm border border-border bg-secondary px-2 py-1 hover:text-foreground">Company Profile</a>
      </nav>
      <SurfaceCard title="Company Profile">
        <div id="company-profile" />
        <EmptyState
          title="Company dossier currently unavailable"
          description="No dossier document is currently published for this company identifier."
        />
      </SurfaceCard>
    </div>
  );
}
