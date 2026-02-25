import { SurfaceCard } from "@/components/ui";

export default function Loading() {
  return (
    <div className="space-y-6" aria-live="polite" aria-busy="true" aria-label="Loading dashboard content">
      <div className="apex-card p-6">
        <div className="skeleton h-8 w-48" />
        <div className="mt-3 skeleton h-4 w-72" />
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {Array.from({ length: 4 }).map((_, idx) => (
          <div key={idx} className="apex-card p-4">
            <div className="skeleton h-4 w-24" />
            <div className="mt-3 skeleton h-8 w-20" />
          </div>
        ))}
      </div>

      <SurfaceCard title="Loading">
        <div className="space-y-3">
          {Array.from({ length: 5 }).map((_, idx) => (
            <div key={idx} className="skeleton h-10 w-full" />
          ))}
        </div>
      </SurfaceCard>
    </div>
  );
}
