"use client";

import { useEffect } from "react";
import Link from "next/link";
import { AlertTriangle } from "@/components/ui";

export default function GlobalError({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    // Log to error reporting service in production
    console.error("[ApexIntel] Unhandled error:", error);
  }, [error]);

  return (
    <div className="mx-auto flex min-h-[60vh] max-w-xl flex-col justify-center">
      <div className="apex-card p-6 text-center">
        <div className="mx-auto w-fit rounded-sm border border-border bg-secondary p-3 text-destructive">
          <AlertTriangle className="h-8 w-8" />
        </div>
        <h2 className="mt-4 text-xl font-black uppercase tracking-[0.08em]">
          Something went wrong
        </h2>
        <p className="mx-auto mt-2 max-w-md text-sm text-muted-foreground">
          An unexpected error occurred while rendering this page. If the problem
          persists, return to overview or contact the engineering team.
        </p>
        {error.digest && (
          <p className="mt-2 font-mono text-xs text-muted-foreground">
            Error ID: {error.digest}
          </p>
        )}
        <div className="mt-4 flex items-center justify-center gap-2">
          <button
            onClick={reset}
            className="rounded-sm border border-border bg-primary px-4 py-2 text-xs font-black uppercase tracking-[0.14em] text-primary-foreground hover:opacity-90"
          >
            Try Again
          </button>
          <Link
            href="/"
            className="rounded-sm border border-border bg-secondary px-4 py-2 text-xs font-black uppercase tracking-[0.14em] text-foreground hover:bg-muted"
          >
            Go to Overview
          </Link>
        </div>
      </div>
    </div>
  );
}
