"use client";

import { useMemo, useState } from "react";
import {
  PageHeader, SurfaceCard, StatCard, FilterBar, FilterChip,
  PAGE_ICONS, Zap, Boxes, Users, Globe, Activity
} from "@/components/ui";
import { GRAPH_NODES, GRAPH_EDGES } from "@/lib/mock-data";

/* Use the actual types from the mock data */
type NodeType = "company" | "person" | "region" | "domain" | "cert" | "tender";

const TYPE_FILTERS: NodeType[] = ["company", "person", "region", "domain", "cert", "tender"];

const TYPE_COLORS: Record<NodeType, string> = {
  company: "#4A90E2",
  person: "#2D8C3C",
  region: "#FFBE00",
  domain: "#D62D2D",
  cert: "#4A90E2",
  tender: "#D62D2D",
};

export default function GraphPage() {
  const [typeFilter, setTypeFilter] = useState<NodeType | null>(null);
  const [hoveredNode, setHoveredNode] = useState<string | null>(null);

  const filteredNodes = useMemo(() => {
    if (!typeFilter) return GRAPH_NODES;
    return GRAPH_NODES.filter((n) => n.type === typeFilter);
  }, [typeFilter]);

  const nodeSet = useMemo(() => new Set(filteredNodes.map((n) => n.id)), [filteredNodes]);

  const filteredEdges = useMemo(() => {
    return GRAPH_EDGES.filter((e) => nodeSet.has(e.source) && nodeSet.has(e.target));
  }, [nodeSet]);

  /* Build a node position lookup from the mock data */
  const nodePositions = useMemo(() => {
    const pos: Record<string, { x: number; y: number }> = {};
    GRAPH_NODES.forEach((n) => {
      pos[n.id] = { x: n.x, y: n.y };
    });
    return pos;
  }, []);

  /* Highlight based on hover */
  const connectedNodes = useMemo(() => {
    if (!hoveredNode) return new Set<string>();
    const connected = new Set<string>();
    GRAPH_EDGES.forEach((e) => {
      if (e.source === hoveredNode) connected.add(e.target);
      if (e.target === hoveredNode) connected.add(e.source);
    });
    return connected;
  }, [hoveredNode]);

  const counts = {
    company: GRAPH_NODES.filter((n) => n.type === "company").length,
    person: GRAPH_NODES.filter((n) => n.type === "person").length,
    region: GRAPH_NODES.filter((n) => n.type === "region").length,
    cert: GRAPH_NODES.filter((n) => n.type === "cert").length,
    tender: GRAPH_NODES.filter((n) => n.type === "tender").length,
  };
  const activeFilterCount = Number(Boolean(typeFilter));

  return (
    <div>
      <PageHeader
        title="Graph Explorer"
        subtitle="Interactive network neighborhood and path exploration across entities."
        icon={PAGE_ICONS.graph}
      />

      {/* ─── KPI Row ─────────────────────────────────────────── */}
      <div className="grid gap-4 sm:grid-cols-3 lg:grid-cols-5 mb-6">
        <StatCard label="Companies" value={counts.company.toString()} accent={TYPE_COLORS.company} icon={<Boxes className="h-3.5 w-3.5" />} />
        <StatCard label="Persons" value={counts.person.toString()} accent={TYPE_COLORS.person} icon={<Users className="h-3.5 w-3.5" />} />
        <StatCard label="Regions" value={counts.region.toString()} accent={TYPE_COLORS.region} icon={<Globe className="h-3.5 w-3.5" />} />
        <StatCard label="Certs" value={counts.cert.toString()} accent={TYPE_COLORS.cert} icon={<Activity className="h-3.5 w-3.5" />} />
        <StatCard label="Tenders" value={counts.tender.toString()} accent={TYPE_COLORS.tender} icon={<Zap className="h-3.5 w-3.5" />} />
      </div>

      {/* ─── Filters ─────────────────────────────────────────── */}
      <FilterBar>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Refine graph entities:</span>
        <span className="text-[10px] font-bold uppercase text-muted-foreground mr-2">Entity Type:</span>
        {TYPE_FILTERS.map((t) => (
          <FilterChip
            key={t}
            label={t}
            active={typeFilter === t}
            onClick={() => setTypeFilter(typeFilter === t ? null : t)}
          />
        ))}
        <span className="mx-2 text-[10px] font-bold uppercase text-muted-foreground">Active: {activeFilterCount}</span>
        <button
          type="button"
          onClick={() => setTypeFilter(null)}
          className="rounded-sm border border-border bg-secondary px-2 py-1 text-[10px] font-black uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
        >
          Reset Filters
        </button>
      </FilterBar>

      {/* ─── Graph Canvas ────────────────────────────────────── */}
      <SurfaceCard title="Graph Canvas" icon={<Zap className="h-3.5 w-3.5" />}>
        <svg width="100%" viewBox="0 0 800 550" className="bg-secondary/30 rounded">
          {/* Edges */}
          {filteredEdges.map((e, idx) => {
            const from = nodePositions[e.source];
            const to = nodePositions[e.target];
            if (!from || !to) return null;
            const isHighlighted = hoveredNode && (e.source === hoveredNode || e.target === hoveredNode);
            return (
              <line
                key={`${e.source}-${e.target}-${e.label}`}
                x1={from.x}
                y1={from.y}
                x2={to.x}
                y2={to.y}
                stroke={isHighlighted ? "#333" : "#ccc"}
                strokeWidth={isHighlighted ? 2 : 1}
                strokeDasharray={e.label === "competes_with" ? "4 2" : undefined}
              />
            );
          })}

          {/* Nodes */}
          {filteredNodes.map((n) => {
            const color = TYPE_COLORS[n.type] || "#888";
            const isHighlighted = hoveredNode === n.id || connectedNodes.has(n.id);
            const opacity = hoveredNode && !isHighlighted && hoveredNode !== n.id ? 0.3 : 1;
            const radius = Math.max(12, n.size * 0.7);
            return (
              <g
                key={n.id}
                transform={`translate(${n.x}, ${n.y})`}
                opacity={opacity}
                onMouseEnter={() => setHoveredNode(n.id)}
                onMouseLeave={() => setHoveredNode(null)}
                onFocus={() => setHoveredNode(n.id)}
                onBlur={() => setHoveredNode(null)}
                onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setHoveredNode((prev) => prev === n.id ? null : n.id); } }}
                tabIndex={0}
                role="button"
                aria-label={`${n.type}: ${n.label}`}
                style={{ cursor: "pointer" }}
              >
                <circle
                  r={radius}
                  fill={color}
                  stroke={isHighlighted ? "#000" : "#fff"}
                  strokeWidth={isHighlighted ? 2 : 1}
                />
                <text
                  y={radius + 12}
                  textAnchor="middle"
                  className="text-[8px] font-bold uppercase fill-foreground"
                >
                  {n.label.length > 14 ? n.label.slice(0, 13) + "…" : n.label}
                </text>
              </g>
            );
          })}
        </svg>

        {/* Legend */}
        <div className="flex flex-wrap justify-center gap-4 mt-3 text-[10px] font-bold uppercase">
          {TYPE_FILTERS.map((t) => (
            <span key={t} className="flex items-center gap-1">
              <span className="h-2.5 w-2.5 rounded-full" style={{ backgroundColor: TYPE_COLORS[t] }} />
              {t}
            </span>
          ))}
        </div>
      </SurfaceCard>

      {/* ─── Hovered Node Info ───────────────────────────────── */}
      {hoveredNode && (
        <div className="mt-4 apex-card p-4">
          <p className="text-xs font-bold uppercase text-muted-foreground mb-1">Selected Node</p>
          <p className="font-bold">{GRAPH_NODES.find((n) => n.id === hoveredNode)?.label}</p>
          <p className="text-xs text-muted-foreground">{GRAPH_NODES.find((n) => n.id === hoveredNode)?.type}</p>
        </div>
      )}
    </div>
  );
}
