import { EmptyState, PageHeader, SurfaceCard } from "@/components/ui";

export default function MemosPage() {
  return (
    <div>
      <PageHeader title="Strategy Memos" subtitle="Weekly strategy memos for executive review." />
      <SurfaceCard title="Latest Weekly Memo">
        <EmptyState
          title="Weekly memo not available"
          description="No published memo is currently available for the latest reporting period."
        />
      </SurfaceCard>
    </div>
  );
}
