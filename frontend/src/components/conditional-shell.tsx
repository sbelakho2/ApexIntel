"use client";

import { usePathname } from "next/navigation";
import { AppShell } from "@/components/app-shell";

const SHELL_EXCLUDED_PATHS = ["/login"];

export function ConditionalShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();

  if (SHELL_EXCLUDED_PATHS.some((p) => pathname === p || pathname.startsWith(`${p}/`))) {
    return <>{children}</>;
  }

  return <AppShell>{children}</AppShell>;
}
