import Link from "next/link";

export default function NotFound() {
  return (
    <div className="mx-auto flex min-h-[60vh] max-w-xl flex-col justify-center">
      <div className="apex-card p-6 text-center">
        <p className="text-6xl font-black tabular-nums text-muted-foreground">404</p>
        <h2 className="mt-2 text-xl font-black uppercase tracking-[0.08em]">
          Page Not Found
        </h2>
        <p className="mx-auto mt-2 max-w-md text-sm text-muted-foreground">
          The requested page does not exist. It may have been moved or the URL may
          be incorrect.
        </p>
        <div className="mt-4 flex items-center justify-center gap-2">
          <Link
            href="/"
            className="rounded-sm border border-border bg-primary px-4 py-2 text-xs font-black uppercase tracking-[0.14em] text-primary-foreground hover:opacity-90"
          >
            Back to Overview
          </Link>
          <Link
            href="/warnings"
            className="rounded-sm border border-border bg-secondary px-4 py-2 text-xs font-black uppercase tracking-[0.14em] text-foreground hover:bg-muted"
          >
            Open Warnings
          </Link>
        </div>
      </div>
    </div>
  );
}
