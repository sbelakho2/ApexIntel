"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useState, useCallback, useEffect, useRef, useMemo } from "react";

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

function NavLinks({ pathname, onNavigate }: { pathname: string; onNavigate?: () => void }) {
  return (
    <>
      {navItems.map((item) => {
        const active = item.href === "/"
          ? pathname === "/"
          : pathname === item.href || pathname.startsWith(`${item.href}/`);
        return (
          <Link
            key={item.href}
            href={item.href}
            onClick={onNavigate}
            aria-current={active ? "page" : undefined}
            className={`sidebar-item ${active ? "active" : ""}`}
          >
            <span className={`h-1.5 w-1.5 rounded-full ${active ? "bg-primary" : "bg-border"}`} aria-hidden="true" />
            {item.label}
          </Link>
        );
      })}
    </>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const [mobileOpen, setMobileOpen] = useState(false);
  const [routeLoading, setRouteLoading] = useState(false);
  const [isOffline, setIsOffline] = useState(false);
  const mobileCloseRef = useRef<HTMLButtonElement>(null);

  const breadcrumbs = useMemo(() => {
    const parts = pathname.split("/").filter(Boolean);
    return parts.map((part, index) => {
      const href = `/${parts.slice(0, index + 1).join("/")}`;
      const label = decodeURIComponent(part).replace(/[-_]/g, " ");
      return {
        href,
        label: label.length === 0 ? label : label[0].toUpperCase() + label.slice(1),
      };
    });
  }, [pathname]);

  const closeMobile = useCallback(() => setMobileOpen(false), []);

  // Close mobile nav on route change
  useEffect(() => {
    setMobileOpen(false);
  }, [pathname]);

  useEffect(() => {
    setRouteLoading(true);
    const timeout = window.setTimeout(() => setRouteLoading(false), 280);
    return () => window.clearTimeout(timeout);
  }, [pathname]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (!event.altKey || event.metaKey || event.ctrlKey || event.shiftKey) {
        return;
      }
      const shortcutMap: Record<string, string> = {
        "1": "/",
        "2": "/warnings",
        "3": "/insights",
        "4": "/memos",
        "5": "/companies",
        "6": "/persons",
        "7": "/security",
        "8": "/graph",
        "9": "/recipes",
        "0": "/settings",
      };
      const target = shortcutMap[event.key];
      if (!target) {
        return;
      }
      event.preventDefault();
      window.location.assign(target);
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  useEffect(() => {
    const syncOnlineState = () => {
      setIsOffline(!navigator.onLine);
    };
    syncOnlineState();
    window.addEventListener("online", syncOnlineState);
    window.addEventListener("offline", syncOnlineState);
    return () => {
      window.removeEventListener("online", syncOnlineState);
      window.removeEventListener("offline", syncOnlineState);
    };
  }, []);

  // Prevent body scroll when mobile nav is open & move focus into drawer
  useEffect(() => {
    if (mobileOpen) {
      document.body.style.overflow = "hidden";
      // Move focus into the drawer close button
      requestAnimationFrame(() => mobileCloseRef.current?.focus());

      const handleEscape = (e: KeyboardEvent) => {
        if (e.key === "Escape") setMobileOpen(false);
      };
      document.addEventListener("keydown", handleEscape);
      return () => {
        document.body.style.overflow = "";
        document.removeEventListener("keydown", handleEscape);
      };
    }
  }, [mobileOpen]);

  return (
    <div className="min-h-screen bg-background">
      <div className={`route-progress ${routeLoading ? "is-active" : ""}`} aria-hidden="true" />
      <p className="sr-only" aria-live="polite">
        {routeLoading ? "Loading page content" : "Page content loaded"}
      </p>
      <div className="pointer-events-none fixed inset-0 z-40 hidden border-[6px] border-background lg:block" aria-hidden="true" />
      <div className="flex">
        {/* Desktop sidebar */}
        <aside className="hidden lg:flex lg:w-64 lg:flex-col lg:border-r lg:border-border lg:bg-card" aria-label="Main navigation">
          <div className="px-4 py-5 border-b border-border">
            <p className="text-lg font-black uppercase tracking-[0.08em]">ApexIntel</p>
            <p className="text-[11px] font-bold uppercase tracking-[0.14em] text-muted-foreground">OSINT Intelligence Platform</p>
          </div>
          <nav className="flex-1 p-3 space-y-1">
            <NavLinks pathname={pathname} />
          </nav>
        </aside>

        {/* Mobile overlay */}
        {mobileOpen && (
          <div
            className="fixed inset-0 z-50 bg-black/40 lg:hidden"
            onClick={closeMobile}
            aria-hidden="true"
          />
        )}

        {/* Mobile sidebar drawer */}
        <aside
          className={`fixed inset-y-0 left-0 z-50 w-64 flex-col border-r border-border bg-card transition-transform duration-200 lg:hidden ${
            mobileOpen ? "flex translate-x-0" : "flex -translate-x-full"
          }`}
          aria-label="Mobile navigation"
        >
          <div className="flex items-center justify-between px-4 py-5 border-b border-border">
            <div>
              <p className="text-lg font-black uppercase tracking-[0.08em]">ApexIntel</p>
              <p className="text-[11px] font-bold uppercase tracking-[0.14em] text-muted-foreground">OSINT Intelligence Platform</p>
            </div>
            <button
              ref={mobileCloseRef}
              onClick={closeMobile}
              className="rounded-sm p-1 text-muted-foreground hover:text-foreground"
              aria-label="Close navigation"
            >
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          </div>
          <nav className="flex-1 p-3 space-y-1 overflow-y-auto">
            <NavLinks pathname={pathname} onNavigate={closeMobile} />
          </nav>
        </aside>

        <main id="main-content" className="flex-1 min-w-0 pb-8 md:pb-10" tabIndex={-1}>
          {isOffline && (
            <div className="border-b border-border bg-destructive/10 px-4 py-2 text-[11px] font-bold uppercase tracking-[0.1em] text-destructive sm:px-6 lg:px-8" role="status" aria-live="polite">
              Offline: showing cached or local data only
            </div>
          )}
          <header className="sticky top-0 z-20 border-b border-border bg-background/95">
            <div className="mx-auto max-w-7xl px-4 py-3 sm:px-6 lg:px-8">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-3">
                  <button
                    onClick={() => setMobileOpen(true)}
                    className="rounded-sm p-1 text-muted-foreground hover:text-foreground lg:hidden"
                    aria-label="Open navigation"
                  >
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="18" x2="21" y2="18"/></svg>
                  </button>
                  <p className="text-[11px] font-black uppercase tracking-[0.14em] text-muted-foreground">Operational Intelligence for EMS, Supply Chain, and Security</p>
                  <p className="hidden text-[10px] font-bold uppercase tracking-[0.1em] text-muted-foreground md:block">Data freshness: near real-time</p>
                </div>
                <span className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.12em] text-muted-foreground">Live</span>
              </div>
              {breadcrumbs.length > 0 && (
                <nav aria-label="Breadcrumb" className="mt-2 overflow-x-auto">
                  <ol className="flex min-w-max items-center gap-1 text-[10px] font-bold uppercase tracking-[0.1em] text-muted-foreground">
                    <li>
                      <Link href="/" className="hover:text-foreground">
                        Overview
                      </Link>
                    </li>
                    {breadcrumbs.map((crumb) => (
                      <li key={crumb.href} className="flex items-center gap-1">
                        <span aria-hidden="true">/</span>
                        <a
                          href={crumb.href}
                          aria-current={crumb.href === pathname ? "page" : undefined}
                          className={crumb.href === pathname ? "text-foreground" : "hover:text-foreground"}
                        >
                          {crumb.label}
                        </a>
                      </li>
                    ))}
                  </ol>
                </nav>
              )}
            </div>
          </header>
          <div className="mx-auto max-w-7xl space-y-6 px-4 py-6 sm:px-6 lg:px-8">{children}</div>
        </main>
      </div>
    </div>
  );
}
