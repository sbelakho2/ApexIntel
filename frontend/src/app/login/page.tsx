"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";

export default function LoginPage() {
  const router = useRouter();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);

    try {
      const res = await fetch("/api/auth/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ username, password }),
      });

      if (!res.ok) {
        const data = await res.json().catch(() => ({ error: "Login failed" }));
        setError(data.error || "Invalid credentials");
        setLoading(false);
        return;
      }

      router.push("/");
      router.refresh();
    } catch {
      setError("Network error. Please try again.");
      setLoading(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-background relative overflow-hidden">
      {/* Subtle background grid */}
      <div
        className="absolute inset-0 opacity-[0.03]"
        style={{
          backgroundImage:
            "linear-gradient(rgba(0,0,0,0.3) 1px, transparent 1px), linear-gradient(90deg, rgba(0,0,0,0.3) 1px, transparent 1px)",
          backgroundSize: "48px 48px",
        }}
        aria-hidden="true"
      />

      {/* Decorative corner marks */}
      <div className="absolute top-6 left-6 w-8 h-8 border-l-2 border-t-2 border-border opacity-30" aria-hidden="true" />
      <div className="absolute top-6 right-6 w-8 h-8 border-r-2 border-t-2 border-border opacity-30" aria-hidden="true" />
      <div className="absolute bottom-6 left-6 w-8 h-8 border-l-2 border-b-2 border-border opacity-30" aria-hidden="true" />
      <div className="absolute bottom-6 right-6 w-8 h-8 border-r-2 border-b-2 border-border opacity-30" aria-hidden="true" />

      <div className="relative z-10 w-full max-w-md px-4">
        {/* Brand */}
        <div className="text-center mb-10">
          <div className="inline-flex items-center justify-center w-16 h-16 rounded-sm border-2 border-border bg-card mb-6">
            <svg width="32" height="32" viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg">
              <path d="M16 2L4 8V24L16 30L28 24V8L16 2Z" stroke="currentColor" strokeWidth="1.5" fill="none" />
              <path d="M16 2V30" stroke="currentColor" strokeWidth="1" strokeDasharray="2 2" opacity="0.4" />
              <path d="M4 8L28 24" stroke="currentColor" strokeWidth="1" strokeDasharray="2 2" opacity="0.4" />
              <path d="M28 8L4 24" stroke="currentColor" strokeWidth="1" strokeDasharray="2 2" opacity="0.4" />
              <circle cx="16" cy="16" r="4" fill="var(--rams-orange, #FFBE00)" />
            </svg>
          </div>
          <h1 className="text-2xl font-black uppercase tracking-[0.08em] text-foreground">
            ApexIntel
          </h1>
          <p className="text-[11px] font-bold uppercase tracking-[0.14em] text-muted-foreground mt-1">
            Competitive Intelligence
          </p>
        </div>

        {/* Login Card */}
        <div className="rounded-sm border border-border bg-card p-8 shadow-sm">
          <div className="mb-6">
            <h2 className="text-sm font-black uppercase tracking-[0.1em] text-foreground">
              Authenticate
            </h2>
            <p className="text-[10px] font-bold uppercase tracking-[0.1em] text-muted-foreground mt-1">
              Sign in to continue
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-5">
            <div>
              <label
                htmlFor="username"
                className="block text-[10px] font-black uppercase tracking-[0.12em] text-muted-foreground mb-2"
              >
                Operator ID
              </label>
              <input
                id="username"
                type="text"
                autoComplete="username"
                autoFocus
                required
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                className="w-full h-11 rounded-sm border border-border bg-background px-3 text-sm font-mono placeholder:text-muted-foreground/50 focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors"
                placeholder="Enter operator ID"
              />
            </div>

            <div>
              <label
                htmlFor="password"
                className="block text-[10px] font-black uppercase tracking-[0.12em] text-muted-foreground mb-2"
              >
                Access Key
              </label>
              <input
                id="password"
                type="password"
                autoComplete="current-password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full h-11 rounded-sm border border-border bg-background px-3 text-sm font-mono placeholder:text-muted-foreground/50 focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors"
                placeholder="Enter access key"
              />
            </div>

            {error && (
              <div
                className="rounded-sm border border-destructive/30 bg-destructive/10 px-3 py-2.5 text-[11px] font-bold uppercase tracking-[0.08em] text-destructive"
                role="alert"
              >
                <span className="inline-block w-1.5 h-1.5 rounded-full bg-destructive mr-2 align-middle" />
                {error}
              </div>
            )}

            <button
              type="submit"
              disabled={loading || !username || !password}
              className="w-full h-11 rounded-sm bg-foreground text-background text-[11px] font-black uppercase tracking-[0.14em] hover:opacity-90 focus:outline-none focus:ring-2 focus:ring-primary/50 transition-all disabled:opacity-40 disabled:cursor-not-allowed flex items-center justify-center gap-2"
            >
              {loading ? (
                <>
                  <svg className="h-3.5 w-3.5 animate-spin" viewBox="0 0 24 24" fill="none">
                    <circle cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeDasharray="32" strokeDashoffset="12" />
                  </svg>
                  Authenticating...
                </>
              ) : (
                "Access Platform"
              )}
            </button>
          </form>
        </div>

        {/* Footer */}
        <div className="mt-6 text-center">
          <p className="text-[9px] font-bold uppercase tracking-[0.14em] text-muted-foreground/60">
            Secure access only
          </p>
          <div className="mt-3 flex items-center justify-center gap-3 text-[9px] text-muted-foreground/40">
            <span className="h-px w-8 bg-border" />
            <span className="font-bold uppercase tracking-[0.2em]">ApexIntel v2.0</span>
            <span className="h-px w-8 bg-border" />
          </div>
        </div>
      </div>
    </div>
  );
}
