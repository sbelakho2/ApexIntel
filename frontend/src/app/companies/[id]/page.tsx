import { EmptyState, PageHeader, SurfaceCard } from "@/components/ui";

export default function CompanyDossierPage({ params }: { params: { id: string } }) {
  return (
    <div>
      <PageHeader
        title={`Company Dossier: ${params.id}`}
        subtitle="Full company profile, capabilities, sites, and graph relationships."
      />
      <SurfaceCard title="Company Profile">
        <EmptyState
          title="Company dossier currently unavailable"
          description="No dossier document is currently published for this company identifier."
        />
      </SurfaceCard>
    </div>
  );
}
