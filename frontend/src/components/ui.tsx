"use client";

import React from "react";
import {
  Activity, AlertTriangle, Boxes, ChevronDown, ChevronUp, Database,
  FileText, Globe, Lock, Minus, Search, Shield, Sparkles, TrendingUp,
  Users, Zap, Radio, Eye, Server, Clock, BarChart3, Target, Check, X
} from "lucide-react";
import type { Severity } from "@/lib/mock-data";

class CardErrorBoundary extends React.Component<{ children: React.ReactNode }, { hasError: boolean }> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError() {
    return { hasError: true };
  }

  override componentDidCatch(error: unknown) {
    console.error("[ApexIntel] card render failure", error);
  }

  override render() {
    if (this.state.hasError) {
      return (
        <div className="rounded-sm border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          This panel failed to render. Please refresh or navigate back to Overview.
        </div>
      );
    }
    return this.props.children;
  }
}

/* ──────────────────────────────────────────────────────────────────
   SHARED PRIMITIVES — Sensei-Rams 3.0 Compliant
   ────────────────────────────────────────────────────────────────── */

// ─── Page Header ────────────────────────────────────────────────

export function PageHeader({
  title,
  subtitle,
  actionHref,
  actionLabel,
  icon,
}: {
  title: string;
  subtitle?: string;
  actionHref?: string;
  actionLabel?: string;
  icon?: React.ReactNode;
}) {
  return (
    <div className="mb-6 flex items-start justify-between gap-4 border-b border-border pb-4">
      <div className="flex items-start gap-3">
        <div className="mt-1 rounded-sm border border-border bg-secondary p-2 text-foreground">
          {icon ?? <Sparkles className="h-4 w-4" />}
        </div>
        <div>
          <h1>{title}</h1>
          {subtitle ? (
            <p className="mt-1 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground">{subtitle}</p>
          ) : null}
        </div>
      </div>
      {actionHref && actionLabel ? (
        <a
          href={actionHref}
          className="rounded-sm border border-border bg-primary px-3 py-2 text-xs font-black uppercase tracking-[0.14em] text-primary-foreground"
        >
          {actionLabel}
        </a>
      ) : null}
    </div>
  );
}

// ─── Stat Card (KPI) ────────────────────────────────────────────

export function StatCard({
  label,
  value,
  delta,
  direction,
  icon,
  accent,
}: {
  label: string;
  value: string;
  delta?: string;
  direction?: "up" | "down" | "flat";
  icon?: React.ReactNode;
  accent?: string;
}) {
  const displayValue = /^-?\d+(?:\.\d+)?$/.test(value)
    ? new Intl.NumberFormat("en-US").format(Number(value))
    : value;

  return (
    <div className="apex-card p-4 relative overflow-hidden">
      <div
        className="absolute top-0 left-0 w-full h-1"
        style={{ backgroundColor: accent ?? "var(--rams-orange)" }}
      />
      <div className="mb-3 flex items-center justify-between pt-1">
        <div className="rounded-sm border border-border bg-secondary p-1.5 text-muted-foreground">
          {icon ?? <Activity className="h-3.5 w-3.5" />}
        </div>
        {delta && (
          <span
            className={`flex items-center gap-0.5 text-[11px] font-bold tabular-nums ${
              direction === "up"
                ? "text-green-600"
                : direction === "down"
                ? "text-red-500"
                : "text-muted-foreground"
            }`}
          >
            {direction === "up" ? (
              <ChevronUp className="h-3 w-3" />
            ) : direction === "down" ? (
              <ChevronDown className="h-3 w-3" />
            ) : (
              <Minus className="h-3 w-3" />
            )}
            {delta}
          </span>
        )}
      </div>
      <p className="text-[10px] font-black uppercase tracking-[0.14em] text-muted-foreground">
        {label}
      </p>
      <p className="mt-1.5 apex-metric-number text-2xl">{displayValue}</p>
    </div>
  );
}

// ─── Surface Card ───────────────────────────────────────────────

export function SurfaceCard({
  title,
  children,
  icon,
  actions,
}: {
  title: string;
  children: React.ReactNode;
  icon?: React.ReactNode;
  actions?: React.ReactNode;
}) {
  return (
    <section className="apex-card p-4">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <div className="rounded-sm border border-border bg-secondary p-1.5 text-muted-foreground">
            {icon ?? <Database className="h-3.5 w-3.5" />}
          </div>
          <h2 className="text-sm font-black uppercase tracking-[0.14em]">{title}</h2>
        </div>
        {actions}
      </div>
      <div className="mt-3 h-px w-full bg-border" />
      <div className="mt-3">
        <CardErrorBoundary>{children}</CardErrorBoundary>
      </div>
    </section>
  );
}

// ─── Empty State ────────────────────────────────────────────────

export function EmptyState({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="apex-card p-8 text-center">
      <div className="mx-auto mb-4 flex h-10 w-10 items-center justify-center rounded-sm border border-border bg-secondary text-muted-foreground">
        <Boxes className="h-5 w-5" />
      </div>
      <p className="text-lg font-black uppercase tracking-[0.08em]">{title}</p>
      <p className="mx-auto mt-2 max-w-xl text-sm text-muted-foreground">{description}</p>
    </div>
  );
}

// ─── Data Table ─────────────────────────────────────────────────

export function DataTable({
  headers,
  rows,
  onRowClick,
}: {
  headers: string[];
  rows: (string | React.ReactNode)[][];
  onRowClick?: (idx: number) => void;
}) {
  return (
    <div className="overflow-x-auto rounded-sm border border-border bg-card">
      <table className="data-table">
        <thead>
          <tr>
            {headers.map((h) => (
              <th key={h}>{h}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.length === 0 ? (
            <tr>
              <td colSpan={headers.length} className="px-4 py-8 text-center text-muted-foreground">
                <div className="flex items-center justify-center gap-2 text-xs font-semibold uppercase tracking-[0.08em]">
                  <FileText className="h-4 w-4" />
                  No records match the current view.
                </div>
              </td>
            </tr>
          ) : (
            rows.map((r, i) => (
              <tr
                key={i}
                className={onRowClick ? "cursor-pointer" : ""}
                onClick={() => onRowClick?.(i)}
                tabIndex={onRowClick ? 0 : undefined}
                role={onRowClick ? "button" : undefined}
                onKeyDown={onRowClick ? (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onRowClick(i); } } : undefined}
              >
                {r.map((c, j) => (
                  <td key={j}>{c}</td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}

// ─── Severity Badge ─────────────────────────────────────────────

const SEVERITY_STYLES: Record<Severity, string> = {
  critical: "bg-red-500/10 text-red-600 border-red-500/20",
  high: "bg-orange-500/10 text-orange-600 border-orange-500/20",
  medium: "bg-yellow-500/10 text-yellow-700 border-yellow-500/20",
  low: "bg-blue-400/10 text-blue-600 border-blue-400/20",
};

export function SeverityBadge({ severity }: { severity: Severity }) {
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-sm border px-2 py-0.5 text-[10px] font-black uppercase tracking-[0.12em] ${SEVERITY_STYLES[severity]}`}
    >
      <span
        className={`h-1.5 w-1.5 rounded-full ${
          severity === "critical"
            ? "bg-red-500 animate-pulse"
            : severity === "high"
            ? "bg-orange-500"
            : severity === "medium"
            ? "bg-yellow-500"
            : "bg-blue-400"
        }`}
      />
      {severity}
    </span>
  );
}

// ─── Status Badge ───────────────────────────────────────────────

export function StatusBadge({
  status,
  variant,
}: {
  status: string;
  variant?: "success" | "warning" | "error" | "info" | "muted";
}) {
  const v = variant ?? "muted";
  const styles: Record<string, string> = {
    success: "bg-green-500/10 text-green-700 border-green-500/20",
    warning: "bg-yellow-500/10 text-yellow-700 border-yellow-500/20",
    error: "bg-red-500/10 text-red-600 border-red-500/20",
    info: "bg-blue-500/10 text-blue-600 border-blue-500/20",
    muted: "bg-muted/50 text-muted-foreground border-border",
  };
  return (
    <span className={`inline-flex items-center rounded-sm border px-2 py-0.5 text-[10px] font-black uppercase tracking-[0.12em] ${styles[v]}`}>
      {status}
    </span>
  );
}

// ─── Progress Bar ───────────────────────────────────────────────

export function ProgressBar({
  value,
  max = 100,
  color,
  label,
  showValue = true,
}: {
  value: number;
  max?: number;
  color?: string;
  label?: string;
  showValue?: boolean;
}) {
  const pct = Math.min(100, Math.max(0, (value / max) * 100));
  return (
    <div className="w-full">
      {(label || showValue) && (
        <div className="flex items-center justify-between mb-1">
          {label && <span className="text-[10px] font-bold uppercase tracking-[0.1em] text-muted-foreground">{label}</span>}
          {showValue && <span className="text-[11px] font-bold tabular-nums text-foreground">{Math.round(pct)}%</span>}
        </div>
      )}
      <div
        className="intel-score-bar"
        role="progressbar"
        aria-valuenow={Math.round(pct)}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-label={label ?? undefined}
      >
        <div
          className="intel-score-fill"
          style={{
            width: `${pct}%`,
            backgroundColor: color ?? "var(--rams-orange)",
          }}
        />
      </div>
    </div>
  );
}

// ─── Mini Spark Line (pure CSS) ─────────────────────────────────

export function SparkBar({ values, color }: { values: number[]; color?: string }) {
  const max = Math.max(...values, 1);
  return (
    <div className="flex items-end gap-px h-8">
      {values.map((v, i) => (
        <div
          key={i}
          className="flex-1 rounded-t-sm min-w-[3px]"
          style={{
            height: `${(v / max) * 100}%`,
            backgroundColor: color ?? "var(--rams-orange)",
            opacity: 0.3 + (v / max) * 0.7,
          }}
        />
      ))}
    </div>
  );
}

// ─── Posture Check Row ──────────────────────────────────────────

export function PostureCheck({
  check,
  status,
  value,
}: {
  check: string;
  status: "pass" | "warn" | "fail";
  value: string;
}) {
  const styles = {
    pass: { bg: "bg-green-500/10", dot: "bg-green-500", text: "text-green-700", label: "PASS" },
    warn: { bg: "bg-yellow-500/10", dot: "bg-yellow-500", text: "text-yellow-700", label: "WARN" },
    fail: { bg: "bg-red-500/10", dot: "bg-red-500 animate-pulse", text: "text-red-600", label: "FAIL" },
  };
  const s = styles[status];
  return (
    <div className={`flex items-center justify-between rounded-sm border border-border px-3 py-2.5 ${s.bg}`}>
      <div className="flex items-center gap-3">
        <span className={`h-2 w-2 rounded-full ${s.dot}`} />
        <span className="text-sm font-bold">{check}</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-xs text-muted-foreground font-mono max-w-[120px] sm:max-w-[200px] md:max-w-[280px] truncate">{value}</span>
        <span className={`text-[10px] font-black uppercase tracking-[0.12em] ${s.text}`}>{s.label}</span>
      </div>
    </div>
  );
}

// ─── Priority Badge ─────────────────────────────────────────────

export function PriorityBadge({ priority }: { priority: "A" | "B" | "C" | "P1" | "P2" | "P3" }) {
  const styles: Record<string, string> = {
    A: "bg-red-500 text-black",
    B: "bg-yellow-500 text-black",
    C: "bg-blue-400 text-black",
    P1: "bg-red-500 text-black",
    P2: "bg-yellow-500 text-black",
    P3: "bg-blue-400 text-black",
  };
  return (
    <span className={`inline-flex h-5 w-5 items-center justify-center rounded-sm text-[10px] font-black ${styles[priority]}`}>
      {priority}
    </span>
  );
}

// ─── Region Badge ───────────────────────────────────────────────

export function RegionBadge({ region }: { region: string }) {
  return (
    <span className="inline-flex items-center gap-1 rounded-sm border border-border bg-secondary px-2 py-0.5 text-[10px] font-bold uppercase tracking-[0.08em] text-muted-foreground">
      <Globe className="h-2.5 w-2.5" />
      {region}
    </span>
  );
}

// ─── Tag Pill ───────────────────────────────────────────────────

export function TagPill({ tag }: { tag: string }) {
  return (
    <span className="inline-flex items-center rounded-sm bg-muted/60 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-[0.1em] text-muted-foreground">
      {tag}
    </span>
  );
}

// ─── Influence Score Ring ───────────────────────────────────────

export function ScoreRing({ score, size = 36 }: { score: number; size?: number }) {
  const r = (size - 6) / 2;
  const circ = 2 * Math.PI * r;
  const offset = circ - (score / 100) * circ;
  const color = score >= 85 ? "#D62D2D" : score >= 65 ? "#FFBE00" : "#4A90E2";
  return (
    <svg width={size} height={size} className="block" role="img" aria-label={`Score: ${score} out of 100`}>
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--rams-line)" strokeWidth="3" />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke={color}
        strokeWidth="3"
        strokeDasharray={circ}
        strokeDashoffset={offset}
        strokeLinecap="round"
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
      <text x="50%" y="50%" textAnchor="middle" dominantBaseline="central" className="text-[10px] font-black fill-foreground">
        {score}
      </text>
    </svg>
  );
}

// ─── Filter Bar ─────────────────────────────────────────────────

export function FilterBar({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-sm border border-border bg-card px-3 py-2 mb-4">
      <Search className="h-3.5 w-3.5 text-muted-foreground" />
      {children}
    </div>
  );
}

export function FilterChip({
  label,
  active,
  onClick,
}: {
  label: string;
  active?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-sm border px-2.5 py-2 text-[10px] font-black uppercase tracking-[0.1em] transition-none min-h-[44px] min-w-[44px] ${
        active
          ? "border-primary bg-primary/10 text-foreground"
          : "border-border bg-secondary text-muted-foreground hover:text-foreground"
      }`}
    >
      {label}
    </button>
  );
}

// ─── Section Divider ────────────────────────────────────────────

export function SectionDivider({ label }: { label: string }) {
  return (
    <div className="flex items-center gap-3 my-6">
      <div className="h-px flex-1 bg-border" />
      <span className="text-[10px] font-black uppercase tracking-[0.16em] text-muted-foreground">{label}</span>
      <div className="h-px flex-1 bg-border" />
    </div>
  );
}

// ─── Warning Card ───────────────────────────────────────────────

export function WarningCard({
  severity,
  title,
  region,
  type,
  created,
  status,
}: {
  severity: Severity;
  title: string;
  region: string;
  type: string;
  created: string;
  status: string;
}) {
  const severityBorder = {
    critical: "border-l-red-500",
    high: "border-l-orange-500",
    medium: "border-l-yellow-500",
    low: "border-l-blue-400",
  };
  return (
    <div className={`apex-card p-4 border-l-4 ${severityBorder[severity]}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-2">
            <SeverityBadge severity={severity} />
            <RegionBadge region={region} />
            <StatusBadge status={status} variant={status === "active" ? "error" : status === "acknowledged" ? "warning" : "success"} />
          </div>
          <p className="text-sm font-bold text-foreground line-clamp-2">{title}</p>
          <div className="flex items-center gap-3 mt-2 text-[10px] text-muted-foreground">
            <span className="uppercase tracking-wide">{type.replace(/_/g, " ")}</span>
            <span>•</span>
            <span className="tabular-nums">{new Date(created).toISOString().slice(0, 16).replace("T", " ")} UTC</span>
          </div>
        </div>
        <AlertTriangle className={`h-5 w-5 flex-shrink-0 ${severity === "critical" ? "text-red-500" : severity === "high" ? "text-orange-500" : severity === "medium" ? "text-yellow-500" : "text-blue-500"}`} />
      </div>
    </div>
  );
}

// ─── Insight Card ───────────────────────────────────────────────

export function InsightCard({
  title,
  type,
  region,
  confidence,
  impact,
  sources,
  summary,
}: {
  title: string;
  type: string;
  region: string;
  confidence: number;
  impact: "high" | "medium" | "low";
  sources: number;
  summary: string;
}) {
  const impactColor = impact === "high" ? "#D62D2D" : impact === "medium" ? "#FFBE00" : "#4A90E2";
  return (
    <div className="apex-card p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-2">
            <span className="inline-flex items-center rounded-sm bg-primary/10 border border-primary/20 px-2 py-0.5 text-[10px] font-black uppercase tracking-[0.1em] text-primary">
              {type.replace(/_/g, " ")}
            </span>
            <RegionBadge region={region} />
          </div>
          <p className="text-sm font-bold text-foreground mb-2">{title}</p>
          <p className="text-xs text-muted-foreground line-clamp-2 mb-3">{summary}</p>
          <div className="flex items-center gap-4">
            <div className="flex items-center gap-1.5">
              <span className="text-[10px] font-bold uppercase text-muted-foreground">Confidence</span>
              <ProgressBar value={confidence * 100} showValue={false} color="var(--rams-orange)" />
              <span className="text-[11px] font-bold tabular-nums">{Math.round(confidence * 100)}%</span>
            </div>
            <div className="flex items-center gap-1.5">
              <span className="text-[10px] font-bold uppercase text-muted-foreground">Impact</span>
              <span className="h-2 w-2 rounded-full" style={{ backgroundColor: impactColor }} />
              <span className="text-[10px] font-bold uppercase">{impact}</span>
            </div>
            <div className="flex items-center gap-1.5">
              <Database className="h-3 w-3 text-muted-foreground" />
              <span className="text-[10px] font-bold">{sources} sources</span>
            </div>
          </div>
        </div>
        <Eye className="h-5 w-5 text-muted-foreground flex-shrink-0" />
      </div>
    </div>
  );
}

// ─── Company Card ───────────────────────────────────────────────

export function CompanyCard({
  name,
  domain,
  region,
  type,
  tier,
  riskScore,
  capabilities,
}: {
  name: string;
  domain: string;
  region: string;
  type: string;
  tier: number;
  riskScore: number;
  capabilities: string[];
}) {
  const riskColor = riskScore >= 70 ? "#D62D2D" : riskScore >= 40 ? "#FFBE00" : "#2D8C3C";
  return (
    <div className="apex-card p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <p className="text-sm font-black uppercase">{name}</p>
            <span className="text-[10px] font-bold text-muted-foreground">T{tier}</span>
          </div>
          <p className="text-xs text-muted-foreground mb-2 max-w-[220px] truncate" title={domain}>{domain}</p>
          <div className="flex items-center gap-2 mb-3">
            <RegionBadge region={region} />
            <span className="inline-flex items-center rounded-sm bg-secondary border border-border px-2 py-0.5 text-[10px] font-bold uppercase tracking-[0.08em] text-muted-foreground">
              {type}
            </span>
          </div>
          <div className="flex flex-wrap gap-1">
            {capabilities.slice(0, 4).map((c) => (
              <TagPill key={c} tag={c} />
            ))}
            {capabilities.length > 4 && <TagPill tag={`+${capabilities.length - 4}`} />}
          </div>
        </div>
        <div className="flex flex-col items-center gap-1">
          <ScoreRing score={riskScore} />
          <span className="text-[9px] font-bold uppercase text-muted-foreground">Risk</span>
        </div>
      </div>
    </div>
  );
}

// ─── Person Card ────────────────────────────────────────────────

export function PersonCard({
  name,
  role,
  organization,
  region,
  priority,
  influenceScore,
  tags,
}: {
  name: string;
  role: string;
  organization: string;
  region: string;
  priority: "A" | "B" | "C";
  influenceScore: number;
  tags: string[];
}) {
  return (
    <div className="apex-card p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <PriorityBadge priority={priority} />
            <p className="text-sm font-black">{name}</p>
          </div>
          <p className="text-xs font-semibold text-foreground">{role}</p>
          <p className="text-xs text-muted-foreground mb-2">{organization}</p>
          <div className="flex items-center gap-2 mb-3">
            <RegionBadge region={region} />
          </div>
          <div className="flex flex-wrap gap-1">
            {tags.slice(0, 4).map((t) => (
              <TagPill key={t} tag={t} />
            ))}
          </div>
        </div>
        <div className="flex flex-col items-center gap-1">
          <ScoreRing score={influenceScore} />
          <span className="text-[9px] font-bold uppercase text-muted-foreground">Influence</span>
        </div>
      </div>
    </div>
  );
}

// ─── Competitor Card ────────────────────────────────────────────

export function CompetitorCard({
  name,
  region,
  tier,
  threatScore,
  overlapPct,
  recentChanges,
  metrics,
}: {
  name: string;
  region: string;
  tier: number;
  threatScore: number;
  overlapPct: number;
  recentChanges: string[];
  metrics: { capability: number; pricing: number; quality: number; delivery: number; geography: number };
}) {
  return (
    <div className="apex-card p-4">
      <div className="flex items-start justify-between gap-4 mb-4">
        <div>
          <div className="flex items-center gap-2 mb-1">
            <p className="text-sm font-black uppercase">{name}</p>
            <span className="text-[10px] font-bold text-muted-foreground">T{tier}</span>
          </div>
          <RegionBadge region={region} />
        </div>
        <div className="flex items-center gap-3">
          <div className="text-center">
            <ScoreRing score={threatScore} />
            <span className="text-[9px] font-bold uppercase text-muted-foreground">Threat</span>
          </div>
          <div className="text-center">
            <div className="text-lg font-black tabular-nums text-foreground">{overlapPct}%</div>
            <span className="text-[9px] font-bold uppercase text-muted-foreground">Overlap</span>
          </div>
        </div>
      </div>
      <div className="grid grid-cols-5 gap-2 mb-4">
        {Object.entries(metrics).map(([k, v]) => (
          <div key={k}>
            <ProgressBar value={v} showValue={false} color={v >= 80 ? "#D62D2D" : v >= 60 ? "#FFBE00" : "#4A90E2"} />
            <span className="text-[9px] font-bold uppercase text-muted-foreground">{k.slice(0, 3)}</span>
          </div>
        ))}
      </div>
      <div className="space-y-1">
        {recentChanges.slice(0, 2).map((c, i) => (
          <div key={i} className="flex items-center gap-2 text-xs text-muted-foreground">
            <span className="h-1 w-1 rounded-full bg-primary" />
            <span className="line-clamp-1">{c}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── Recipe Card ────────────────────────────────────────────────

export function RecipeCard({
  name,
  description,
  status,
  precision,
  recall,
  fpr,
  alerts7d,
}: {
  name: string;
  description: string;
  status: "production" | "staging" | "deprecated";
  precision: number;
  recall: number;
  fpr: number;
  alerts7d: number;
}) {
  const statusVariant = status === "production" ? "success" : status === "staging" ? "warning" : "muted";
  return (
    <div className="apex-card p-4">
      <div className="flex items-start justify-between gap-3 mb-3">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <Zap className="h-4 w-4 text-primary" />
            <p className="text-sm font-black uppercase">{name}</p>
          </div>
          <p className="text-xs text-muted-foreground line-clamp-2">{description}</p>
        </div>
        <StatusBadge status={status} variant={statusVariant} />
      </div>
      <div className="grid grid-cols-4 gap-3">
        <div>
          <ProgressBar value={precision * 100} showValue={false} color="#2D8C3C" />
          <div className="flex items-center justify-between mt-1">
            <span className="text-[9px] font-bold uppercase text-muted-foreground">Prec</span>
            <span className="text-[10px] font-bold tabular-nums">{Math.round(precision * 100)}%</span>
          </div>
        </div>
        <div>
          <ProgressBar value={recall * 100} showValue={false} color="#4A90E2" />
          <div className="flex items-center justify-between mt-1">
            <span className="text-[9px] font-bold uppercase text-muted-foreground">Recall</span>
            <span className="text-[10px] font-bold tabular-nums">{Math.round(recall * 100)}%</span>
          </div>
        </div>
        <div>
          <ProgressBar value={fpr * 100} showValue={false} color="#D62D2D" />
          <div className="flex items-center justify-between mt-1">
            <abbr tabIndex={0} title="False Positive Rate" className="text-[9px] font-bold uppercase text-muted-foreground no-underline">
              FPR
            </abbr>
            <span className="text-[10px] font-bold tabular-nums">{Math.round(fpr * 100)}%</span>
          </div>
        </div>
        <div className="text-center">
          <div className="text-lg font-black tabular-nums text-foreground">{alerts7d}</div>
          <span className="text-[9px] font-bold uppercase text-muted-foreground">7d Alerts</span>
        </div>
      </div>
    </div>
  );
}

// ─── Icon map for page types ────────────────────────────────────

export const PAGE_ICONS = {
  overview: <BarChart3 className="h-4 w-4" />,
  warnings: <AlertTriangle className="h-4 w-4" />,
  insights: <Eye className="h-4 w-4" />,
  memos: <FileText className="h-4 w-4" />,
  companies: <Boxes className="h-4 w-4" />,
  persons: <Users className="h-4 w-4" />,
  competitors: <Target className="h-4 w-4" />,
  security: <Shield className="h-4 w-4" />,
  graph: <Radio className="h-4 w-4" />,
  recipes: <Zap className="h-4 w-4" />,
  settings: <Server className="h-4 w-4" />,
};

// Re-export icons used by pages
export {
  Activity, AlertTriangle, Boxes, Database, FileText, Globe, Lock, Search,
  Shield, Sparkles, TrendingUp, Users, Zap, Radio, Eye, Server, Clock,
  BarChart3, Target, ChevronUp, ChevronDown, Minus, Check, X
};
