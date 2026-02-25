"use client";

import { useState } from "react";

export function CopyIdButton({ idValue }: { idValue: string }) {
  const [copied, setCopied] = useState(false);

  return (
    <button
      type="button"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(idValue);
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1200);
        } catch {
          setCopied(false);
        }
      }}
      className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
      aria-live="polite"
    >
      {copied ? "Copied" : "Copy ID"}
    </button>
  );
}
