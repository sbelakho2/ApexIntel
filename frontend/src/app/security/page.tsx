"use client";

import { useState, useMemo } from "react";
import { PieChart, Pie, Cell, ResponsiveContainer, Tooltip } from "recharts";
import {
  PageHeader, SurfaceCard, StatCard, FilterBar, FilterChip, PostureCheck, SeverityBadge, EmptyState,
  PAGE_ICONS, Shield, Lock, AlertTriangle
} from "@/components/ui";
import { SECURITY_FINDINGS, DNS_POSTURE } from "@/lib/mock-data";

const SIGNAL_FILTERS = ["dns_posture", "lookalike", "kev", "breach", "phishing"] as const;
const RISK_FILTERS = ["critical", "high", "medium", "low"] as const;

export default function SecurityPage() {
  const [signalFilter, setSignalFilter] = useState<string | null>(null);
  const [riskFilter, setRiskFilter] = useState<string | null>(null);

  const filteredFindings = useMemo(() => {
    return SECURITY_FINDINGS.filter((f) => {
      if (signalFilter && f.type !== signalFilter) return false;
      if (riskFilter && f.risk !== riskFilter) return false;
      return true;
    });
  }, [signalFilter, riskFilter]);

  const criticalCount = SECURITY_FINDINGS.filter((f) => f.risk === "critical").length;
  const highCount = SECURITY_FINDINGS.filter((f) => f.risk === "high").length;
  const mediumCount = SECURITY_FINDINGS.filter((f) => f.risk === "medium").length;
  const lowCount = SECURITY_FINDINGS.filter((f) => f.risk === "low").length;

  const passCount = DNS_POSTURE.filter((p) => p.status === "pass").length;
  const warnCount = DNS_POSTURE.filter((p) => p.status === "warn").length;
  const failCount = DNS_POSTURE.filter((p) => p.status === "fail").length;

  const riskPie = [
    { name: "Critical", value: criticalCount, fill: "#D62D2D" },
    { name: "High", value: highCount, fill: "#FF8C00" },
    { name: "Medium", value: mediumCount, fill: "#FFBE00" },
    { name: "Low", value: lowCount, fill: "#4A90E2" },
  ].filter((d) => d.value > 0);
  const activeFilterCount = Number(Boolean(signalFilter)) + Number(Boolean(riskFilter));

  return (
    <div>
      <PageHeader
        title="Security Center"
        subtitle="DNS posture, lookalike domains, and KEV relevance monitoring."
        icon={PAGE_ICONS.security}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4 mb-6">
        <StatCard
          label="Total Findings"
          value={SECURITY_FINDINGS.length.toString()}
          icon={<Shield className="h-3.5 w-3.5" />}
          accent="#4A90E2"
        />
        <StatCard
          label="Critical / High"
          value={`${criticalCount}/${highCount}`}
          icon={<AlertTriangle className="h-3.5 w-3.5" />}
          accent="#D62D2D"
        />
        <StatCard
          label="DNS Posture Pass"
          value={`${passCount}/${DNS_POSTURE.length}`}
          icon={<Lock className="h-3.5 w-3.5" />}
          accent="#2D8C3C"
        />
        <StatCard
          label="Posture Warnings"
          value={(warnCount + failCount).toString()}
          icon={<Shield className="h-3.5 w-3.5" />}
          accent="#FFBE00"
        />
      </div>

      {/* ─── Charts & Posture ────────────────────────────────── */}
      <div className="grid gap-4 lg:grid-cols-3 mb-6">
        <SurfaceCard title="Risk Distribution" icon={<Shield className="h-3.5 w-3.5" />}>
          <div className="h-40">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie
                  data={riskPie}
                  dataKey="value"
                  nameKey="name"
                  cx="50%"
                  cy="50%"
                  innerRadius={40}
                  outerRadius={60}
                  paddingAngle={2}
                >
                  {riskPie.map((entry, idx) => (
                    <Cell key={idx} fill={entry.fill} />
                  ))}
                </Pie>
                <Tooltip contentStyle={{ background: "rgb(var(--card))", color: "rgb(var(--card-foreground))", border: "1px solid rgb(var(--border))", borderRadius: 2, fontSize: 11 }} />
              </PieChart>
            </ResponsiveContainer>
          </div>
          <div className="flex flex-wrap justify-center gap-2 mt-1">
            {riskPie.map((r) => (
              <span key={r.name} className="flex items-center gap-1 text-[10px] font-bold uppercase">
                <span className="h-2.5 w-2.5 rounded-sm" style={{ backgroundColor: r.fill }} />
                {r.name}
              </span>
            ))}
          </div>
        </SurfaceCard>

        <div className="lg:col-span-2">
          <SurfaceCard title="DNS Posture Checks" icon={<Lock className="h-3.5 w-3.5" />}>
            <div className="divide-y divide-border max-h-64 overflow-y-auto">
              {DNS_POSTURE.map((p) => (
                <PostureCheck key={p.check} check={p.check} status={p.status} value={p.value} />
              ))}
            </div>
          </SurfaceCard>
        </div>
      </div>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <FilterBar>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine security findings:</span>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Signal:</span>
        {SIGNAL_FILTERS.map((s) => (
          <FilterChip
            key={s}
            label={s.replace(/_/g, " ")}
            active={signalFilter === s}
            onClick={() => setSignalFilter(signalFilter === s ? null : s)}
          />
        ))}
        <span className="mx-3 h-4 w-px bg-border" />
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Risk:</span>
        {RISK_FILTERS.map((r) => (
          <FilterChip
            key={r}
            label={r}
            active={riskFilter === r}
            onClick={() => setRiskFilter(riskFilter === r ? null : r)}
          />
        ))}
        <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
        <button
          type="button"
          onClick={() => {
            setSignalFilter(null);
            setRiskFilter(null);
          }}
          className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
        >
          Reset Filters
        </button>
      </FilterBar>

      {/* ─── Findings List ───────────────────────────────────── */}
      <div className="mt-4 space-y-3">
        {filteredFindings.length === 0 ? (
          <EmptyState
            title="No findings match current filters"
            description="Try broadening signal/risk criteria or reset filters to inspect the full finding set."
          />
        ) : (
          filteredFindings.map((f) => (
            <div
              key={f.id}
              className="apex-card p-4 border-l-4"
              style={{ borderLeftColor: f.risk === "critical" ? "#D62D2D" : f.risk === "high" ? "#FF8C00" : f.risk === "medium" ? "#FFBE00" : "#4A90E2" }}
            >
              <div className="flex items-center justify-between mb-2">
                <span className="font-bold text-sm">{f.domain}</span>
                <SeverityBadge severity={f.risk} />
              </div>
              <p className="text-xs text-muted-foreground mb-2">{f.details}</p>
              <div className="flex items-center gap-3 text-[10px] text-muted-foreground uppercase">
                <span className="font-bold">{f.signal}</span>
                <span>{f.type.replace(/_/g, "-")}</span>
                <span>{f.detected}</span>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
