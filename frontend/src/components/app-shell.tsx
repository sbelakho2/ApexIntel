"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const navItems = [
  { href: "/", label: "Overview" },
  { href: "/warnings", label: "Warnings" },
  { href: "/insights", label: "Insights" },
  { href: "/memos", label: "Memos" },
  { href: "/companies", label: "Companies" },
  { href: "/persons", label: "Persons" },
  { href: "/competitors", label: "Competitors" },
  { href: "/security", label: "Security" },
  { href: "/graph", label: "Graph" },
  { href: "/recipes", label: "Recipes" },
  { href: "/settings", label: "Settings" },
] as const;

export function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();

  return (
    <div className="min-h-screen bg-background">
      <div className="flex">
        <aside className="hidden lg:flex lg:w-64 lg:flex-col lg:border-r lg:border-border lg:bg-card">
          <div className="px-4 py-5 border-b border-border">
            <p className="text-lg font-semibold">ApexIntel</p>
            <p className="text-sm text-muted-foreground">OSINT Intelligence Platform</p>
          </div>
          <nav className="flex-1 p-3 space-y-1">
            {navItems.map((item) => {
              const active = pathname === item.href || pathname.startsWith(`${item.href}/`);
              return (
                <Link
                  key={item.href}
                  href={item.href}
                  className={`sidebar-item ${active ? "active" : ""}`}
                >
                  {item.label}
                </Link>
              );
            })}
          </nav>
        </aside>

        <main className="flex-1 min-w-0">
          <header className="sticky top-0 z-20 border-b border-border bg-background/95 backdrop-blur">
            <div className="mx-auto max-w-7xl px-4 py-3 sm:px-6 lg:px-8">
              <p className="text-sm text-muted-foreground">Operational Intelligence for EMS, Supply Chain, and Security</p>
            </div>
          </header>
          <div className="mx-auto max-w-7xl px-4 py-6 sm:px-6 lg:px-8">{children}</div>
        </main>
      </div>
    </div>
  );
}
