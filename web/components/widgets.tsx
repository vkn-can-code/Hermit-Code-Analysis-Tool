"use client";

import { useMemo } from "react";
import { Background, Controls, MarkerType, ReactFlow, type Edge, type Node } from "@xyflow/react";
import { local, tierColor, type Report, type Tier } from "./api";

/** Semi-circular 0–100 risk gauge. */
export function RiskGauge({ score, tier }: { score: number; tier: Tier }) {
  const r = 70;
  const c = Math.PI * r;
  const filled = (Math.min(100, Math.max(0, score)) / 100) * c;
  const color = tierColor[tier];
  return (
    <svg viewBox="0 0 180 110" className="w-full max-w-[220px]" role="img" aria-label={`Risk ${score} of 100, ${tier}`}>
      <path d="M20 95 A70 70 0 0 1 160 95" fill="none" stroke="var(--color-line)" strokeWidth="14" strokeLinecap="round" />
      <path
        d="M20 95 A70 70 0 0 1 160 95"
        fill="none"
        stroke={color}
        strokeWidth="14"
        strokeLinecap="round"
        strokeDasharray={`${filled} ${c}`}
      />
      <text x="90" y="82" textAnchor="middle" fontSize="34" fontWeight="800" fill="var(--color-text)">{score}</text>
      <text x="90" y="102" textAnchor="middle" fontSize="11" fontWeight="700" letterSpacing="2" fill={color}>
        {tier.toUpperCase()} RISK
      </text>
    </svg>
  );
}

/** Horizontal bars for each explainable risk factor. */
export function FactorBars({ factors }: { factors: Report["risk"]["factors"] }) {
  return (
    <div className="space-y-3">
      {factors.map((f) => (
        <div key={f.name}>
          <div className="flex justify-between text-[13px]">
            <span className="font-semibold">{f.name.replace("_", " ")}</span>
            <span className="mono text-muted">{f.points.toFixed(1)} / {f.max}</span>
          </div>
          <div className="mt-1 h-2 rounded bg-line overflow-hidden">
            <div
              className="h-full rounded"
              style={{
                width: `${(f.points / f.max) * 100}%`,
                background: f.points / f.max > 0.66 ? "var(--color-danger)" : f.points / f.max > 0.33 ? "var(--color-amber)" : "var(--color-cyan)",
              }}
            />
          </div>
          <div className="mt-1 text-[12px] text-muted">{f.detail}</div>
        </div>
      ))}
    </div>
  );
}

export const STAGES = [
  { key: "fetch", label: "Ingest" },
  { key: "diff", label: "Diff" },
  { key: "ast+security", label: "AST + Security" },
  { key: "blast_radius", label: "Blast radius" },
  { key: "tests", label: "Tests" },
  { key: "risk", label: "Risk" },
  { key: "ai", label: "AI review" },
];

/** The seven pipeline stages with the current one highlighted. */
export function Pipeline({ stage, status }: { stage: string; status: string }) {
  const idx = STAGES.findIndex((s) => s.key === stage);
  const done = status === "done";
  return (
    <ol className="grid grid-cols-7 gap-1.5">
      {STAGES.map((s, i) => {
        const state = done || i < idx ? "done" : i === idx ? "active" : "todo";
        const color = state === "done" ? "var(--color-emerald)" : state === "active" ? "var(--color-cyan)" : "var(--color-line)";
        return (
          <li key={s.key} className="text-center">
            <div className="h-1.5 rounded" style={{ background: color }} />
            <div className={`mt-1.5 text-[11px] ${state === "todo" ? "text-muted" : "text-text"} ${state === "active" ? "pulse" : ""}`}>
              {s.label}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

/** Call-graph view: changed symbols (left) → callers by depth (right). */
export function BlastGraph({ report }: { report: Report }) {
  const { nodes, edges } = useMemo(() => {
    const nodes: Node[] = [];
    const edges: Edge[] = [];
    const changed = report.symbols.filter((s) => !s.symbol.is_test && s.symbol.kind !== "impl");
    changed.forEach((s, i) => {
      nodes.push({
        id: s.symbol.qualified,
        position: { x: 0, y: i * 80 },
        data: { label: `${s.symbol.name}()` },
        style: { background: "#0e2a33", color: "#e0fbff", border: "1.5px solid var(--color-cyan)", borderRadius: 10, fontSize: 12, width: 190 },
      });
    });
    const byDepth = new Map<number, Report["impacted"]>();
    report.impacted.forEach((i) => byDepth.set(i.depth, [...(byDepth.get(i.depth) ?? []), i]));
    for (const [depth, items] of byDepth) {
      items.forEach((it, j) => {
        nodes.push({
          id: it.qualified,
          position: { x: depth * 260, y: j * 80 },
          data: { label: `${local(it.qualified)}()  · ${it.file}` },
          style: {
            background: it.sensitive ? "#2a1418" : "var(--color-card)",
            color: "var(--color-text)",
            border: `1px solid ${it.sensitive ? "var(--color-danger)" : "var(--color-line)"}`,
            borderRadius: 10,
            fontSize: 11,
            width: 220,
          },
        });
      });
    }
    // Edge from the node one level closer (or the root) to each impacted symbol.
    const ids = new Set(nodes.map((n) => n.id));
    report.impacted.forEach((it) => {
      const parent =
        report.impacted.find((p) => p.depth === it.depth - 1 && p.via === it.via)?.qualified ?? it.via;
      if (ids.has(parent)) {
        edges.push({
          id: `${parent}->${it.qualified}`,
          source: parent,
          target: it.qualified,
          animated: it.depth === 1,
          markerEnd: { type: MarkerType.ArrowClosed },
          style: { stroke: it.sensitive ? "var(--color-danger)" : "var(--color-muted)" },
        });
      }
    });
    report.new_dependencies.forEach((d) => {
      const id = `dep:${d.to}`;
      nodes.push({
        id,
        position: { x: -260, y: (changed.length + 0.2) * 80 },
        data: { label: `new dep → ${local(d.to)}()` },
        style: { background: "#2a2210", color: "var(--color-text)", border: "1px dashed var(--color-amber)", borderRadius: 10, fontSize: 11, width: 200 },
      });
      if (ids.has(d.from)) {
        edges.push({ id: `${d.from}->${id}`, source: d.from, target: id, style: { stroke: "var(--color-amber)", strokeDasharray: "4 3" } });
      }
    });
    return { nodes, edges };
  }, [report]);

  if (nodes.length === 0) return <p className="text-muted text-sm">No callable symbols changed.</p>;
  return (
    <div className="h-[340px] rounded-lg border border-line bg-card-2">
      <ReactFlow nodes={nodes} edges={edges} fitView nodesDraggable proOptions={{ hideAttribution: true }}>
        <Background gap={20} color="#1b2438" />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}

export function SeverityPill({ severity }: { severity: string }) {
  const color: Record<string, string> = {
    critical: "var(--color-danger)",
    high: "#f87171",
    medium: "var(--color-amber)",
    low: "#facc15",
    info: "var(--color-muted)",
  };
  return (
    <span
      className="inline-block rounded px-1.5 py-0.5 text-[11px] font-bold uppercase"
      style={{ color: color[severity], border: `1px solid ${color[severity]}` }}
    >
      {severity}
    </span>
  );
}

export function TierBadge({ tier, score }: { tier: Tier | null; score: number | null }) {
  if (tier === null || score === null) return <span className="text-muted">—</span>;
  return (
    <span className="mono font-bold" style={{ color: tierColor[tier] }}>
      {score} {tier.toUpperCase()}
    </span>
  );
}
