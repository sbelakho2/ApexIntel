import { EmptyState, PageHeader, SurfaceCard } from "@/components/ui";

export default function GraphPage() {
  return (
    <div>
      <PageHeader
        title="Graph Explorer"
        subtitle="Interactive network neighborhood and path exploration across entities."
      />
      <SurfaceCard title="Graph Canvas">
        <EmptyState
          title="No graph projection for current query"
          description="The current graph request returned no connected neighborhood for the selected entity context."
        />
      </SurfaceCard>
    </div>
  );
}
