import { EmptyState, PageHeader, SurfaceCard } from "@/components/ui";

export default function PersonDossierPage({ params }: { params: { id: string } }) {
  return (
    <div>
      <PageHeader
        title={`POI Dossier: ${params.id}`}
        subtitle="Full POI profile, priority vector, and engagement guide."
      />
      <SurfaceCard title="Person Profile">
        <EmptyState
          title="POI dossier currently unavailable"
          description="No dossier package is currently published for this person identifier."
        />
      </SurfaceCard>
    </div>
  );
}
