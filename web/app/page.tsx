"use client";

import { useCallback, useEffect, useState } from "react";
import { api, local, subscribe, type AnalysisRow, type Health, type Report } from "@/components/api";
import { BlastGraph, FactorBars, Pipeline, RiskGauge, SeverityPill, TierBadge } from "@/components/widgets";

/** Single-page dashboard: list on the left, selected analysis on the right (?id=N). */
export default function Dashboard() {
  const [rows, setRows] = useState<AnalysisRow[]>([]);
  const [health, setHealth] = useState<Health | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.list().then(setRows).catch((e) => setError(String(e.message ?? e)));
  }, []);

  useEffect(() => {
    const q = new URLSearchParams(window.location.search).get("id");
    if (q) setSelected(Number(q));
    refresh();
    api.health().then(setHealth).catch(() => setHealth(null));
    return subscribe((ev) => {
      setRows((prev) => {
        const known = prev.some((r) => r.id === ev.id);
        if (!known) {
          refresh();
          return prev;
        }
        return prev.map((r) =>
          r.id === ev.id
            ? { ...r, status: ev.status as AnalysisRow["status"], stage: ev.stage, risk_score: ev.risk_score ?? r.risk_score }
            : r,
        );
      });
      if (ev.status === "done" || ev.status === "failed") refresh();
    });
  }, [refresh]);

  const select = (id: number) => {
    setSelected(id);
    window.history.replaceState(null, "", `?id=${id}`);
  };

  return (
    <div className="mx-auto max-w-[1400px] px-4 py-6 md:px-8">
      <header className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <div className="eyebrow">Intelligent code analysis</div>
          <h1 className="text-2xl font-extrabold tracking-tight">Hermit</h1>
        </div>
        <HealthBar health={health} />
      </header>

      {error && <div className="mt-4 rounded-lg border border-danger p-3 text-sm text-danger">{error}</div>}

      <div className="mt-6 grid gap-6 lg:grid-cols-[380px_1fr]">
        <aside className="space-y-6">
          <AnalyzeForm onQueued={(id) => { refresh(); select(id); }} disabled={health ? !health.allow_local : false} />
          <AnalysisList rows={rows} selected={selected} onSelect={select} />
        </aside>
        <main>{selected ? <Detail id={selected} rows={rows} /> : <Empty />}</main>
      </div>
    </div>
  );
}

function HealthBar({ health }: { health: Health | null }) {
  const item = (label: string, ok: boolean | undefined) => (
    <span className="flex items-center gap-1.5 text-[12px]">
      <span className="inline-block h-2 w-2 rounded-full" style={{ background: ok ? "var(--color-emerald)" : "var(--color-danger)" }} />
      {label}
    </span>
  );
  return (
    <div className="card flex flex-wrap gap-4 px-4 py-2.5">
      {item("Database", health?.database)}
      {item(`Ollama${health?.models?.length ? ` (${health.models.length})` : ""}`, health?.ollama)}
      {item("Semgrep", health?.semgrep)}
      {item("GitHub App", health?.github_app)}
      <span className="text-[12px] text-muted">v{health?.version ?? "?"}</span>
    </div>
  );
}

function AnalyzeForm({ onQueued, disabled }: { onQueued: (id: number) => void; disabled: boolean }) {
  const [path, setPath] = useState("");
  const [base, setBase] = useState("main");
  const [head, setHead] = useState("HEAD");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setErr(null);
    try {
      const r = await api.analyzeLocal(path, base, head, "");
      onQueued(r.analysis_id);
    } catch (x) {
      setErr(String((x as Error).message));
    } finally {
      setBusy(false);
    }
  };

  const input = "w-full rounded-md border border-line bg-card-2 px-3 py-2 text-sm outline-none focus:border-cyan";
  return (
    <form onSubmit={submit} className="card space-y-3 p-4">
      <div className="eyebrow">Analyze a local repository</div>
      <input className={input} placeholder="/path/to/repo" value={path} onChange={(e) => setPath(e.target.value)} required disabled={disabled} />
      <div className="grid grid-cols-2 gap-2">
        <input className={input} placeholder="base" value={base} onChange={(e) => setBase(e.target.value)} disabled={disabled} />
        <input className={input} placeholder="head" value={head} onChange={(e) => setHead(e.target.value)} disabled={disabled} />
      </div>
      <button
        type="submit"
        disabled={busy || disabled}
        className="w-full rounded-md bg-cyan px-3 py-2 text-sm font-bold text-ink transition hover:brightness-110 disabled:opacity-50"
      >
        {busy ? "Queueing…" : "Run Hermit"}
      </button>
      {err && <p className="text-[12px] text-danger">{err}</p>}
      <p className="text-[11px] text-muted">Pull requests from the GitHub App appear here automatically.</p>
    </form>
  );
}

function AnalysisList({ rows, selected, onSelect }: { rows: AnalysisRow[]; selected: number | null; onSelect: (id: number) => void }) {
  return (
    <div className="card overflow-hidden">
      <div className="eyebrow px-4 pt-4">Recent analyses</div>
      {rows.length === 0 && <p className="p-4 text-sm text-muted">No analyses yet.</p>}
      <ul className="mt-2 max-h-[60vh] overflow-y-auto">
        {rows.map((r) => (
          <li key={r.id}>
            <button
              onClick={() => onSelect(r.id)}
              className={`w-full border-t border-line px-4 py-3 text-left transition hover:bg-card-2 ${selected === r.id ? "bg-card-2" : ""}`}
            >
              <div className="flex items-center justify-between gap-2">
                <span className="truncate text-sm font-semibold">{r.pr_number ? `#${r.pr_number} ` : ""}{r.title || `${r.base_ref}..${r.head_ref}`}</span>
                {r.status === "done" ? (
                  <TierBadge tier={r.risk_tier} score={r.risk_score} />
                ) : (
                  <span className={`text-[11px] font-bold uppercase ${r.status === "failed" ? "text-danger" : "pulse text-cyan"}`}>
                    {r.status === "running" ? r.stage : r.status}
                  </span>
                )}
              </div>
              <div className="mt-0.5 truncate text-[11px] text-muted">{repoLabel(r.repo)} · {new Date(r.created_at).toLocaleString()}</div>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function repoLabel(repo: string) {
  return repo.startsWith("local:") ? `local · ${repo.split("/").pop()}` : repo;
}

function Empty() {
  return (
    <div className="card grid h-full min-h-[300px] place-items-center p-10 text-center">
      <div>
        <div className="eyebrow">Ready</div>
        <p className="mt-2 max-w-md text-soft">Select an analysis, run Hermit on a local repository, or open a pull request on a repository with the Hermit GitHub App installed.</p>
      </div>
    </div>
  );
}

function Detail({ id, rows }: { id: number; rows: AnalysisRow[] }) {
  const [data, setData] = useState<{ analysis: AnalysisRow; report: Report | null } | null>(null);
  const [tab, setTab] = useState<"overview" | "findings" | "impact" | "tests" | "ai">("overview");
  const live = rows.find((r) => r.id === id);

  useEffect(() => {
    api.get(id).then(setData).catch(() => setData(null));
  }, [id, live?.status]);

  if (!data) return <div className="card p-6 text-muted">Loading…</div>;
  const a = { ...data.analysis, ...(live ?? {}) };
  const r = data.report;

  return (
    <div className="space-y-5">
      <section className="card p-5">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="eyebrow">{repoLabel(a.repo)}</div>
            <h2 className="mt-1 text-xl font-bold">{a.pr_number ? `#${a.pr_number} · ` : ""}{a.title}</h2>
            <p className="mono mt-1 text-muted">{a.base_ref} … {a.head_ref} {a.head_sha && `(${a.head_sha.slice(0, 8)})`} · by {a.author}</p>
          </div>
          {r && <RiskGauge score={r.risk.score} tier={r.risk.tier} />}
        </div>
        <div className="mt-4">
          <Pipeline stage={a.stage} status={a.status} />
        </div>
        {a.status === "failed" && <p className="mt-3 text-sm text-danger">{a.error}</p>}
      </section>

      {r && (
        <>
          <nav className="flex flex-wrap gap-2">
            {(["overview", "findings", "impact", "tests", "ai"] as const).map((t) => (
              <button
                key={t}
                onClick={() => setTab(t)}
                className={`rounded-full border px-4 py-1.5 text-[13px] font-semibold capitalize ${tab === t ? "border-cyan bg-cyan/10 text-cyan" : "border-line text-soft hover:border-muted"}`}
              >
                {t === "ai" ? "AI review" : t}
                {t === "findings" && r.findings.length > 0 && ` (${r.findings.length})`}
                {t === "tests" && ` (${r.tests.length})`}
              </button>
            ))}
          </nav>
          {tab === "overview" && <Overview r={r} />}
          {tab === "findings" && <Findings r={r} />}
          {tab === "impact" && <Impact r={r} />}
          {tab === "tests" && <Tests r={r} />}
          {tab === "ai" && <Ai r={r} />}
        </>
      )}
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: string | number; tone?: string }) {
  return (
    <div className="card p-4">
      <div className="text-[11px] font-semibold uppercase tracking-wider text-muted">{label}</div>
      <div className="mt-1 text-2xl font-extrabold" style={{ color: tone }}>{value}</div>
    </div>
  );
}

function Overview({ r }: { r: Report }) {
  const adds = r.files.reduce((s, f) => s + f.additions, 0);
  const dels = r.files.reduce((s, f) => s + f.deletions, 0);
  const newF = r.findings.filter((f) => f.state === "new").length;
  const fixed = r.findings.length - newF;
  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <Stat label="Changed lines" value={`+${adds} / -${dels}`} />
        <Stat label="Symbols changed" value={r.symbols.length} />
        <Stat label="Impacted callers" value={r.impacted.length} tone={r.impacted.some((i) => i.sensitive) ? "var(--color-amber)" : undefined} />
        <Stat label="Findings new / fixed" value={`${newF} / ${fixed}`} tone={newF ? "var(--color-danger)" : fixed ? "var(--color-emerald)" : undefined} />
      </div>
      <div className="grid gap-5 xl:grid-cols-2">
        <section className="card p-5">
          <div className="eyebrow mb-4">Risk breakdown</div>
          <FactorBars factors={r.risk.factors} />
        </section>
        <section className="card p-5">
          <div className="eyebrow mb-3">Summary</div>
          <p className="text-[14px] leading-relaxed text-soft">{r.ai?.summary ?? "AI review not available — deterministic analysis only."}</p>
          <div className="eyebrow mb-2 mt-5">Changed symbols</div>
          <table className="data">
            <thead><tr><th>Symbol</th><th>Change</th><th>Complexity</th></tr></thead>
            <tbody>
              {r.symbols.map((s) => (
                <tr key={s.symbol.qualified}>
                  <td><code>{local(s.symbol.qualified)}</code>{s.signature_changed && <span className="ml-2 text-[11px] text-amber">signature changed</span>}</td>
                  <td className="capitalize">{s.change}</td>
                  <td className="mono">{s.complexity_before} → {s.complexity_after}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      </div>
      <Timings t={r.timings} />
    </div>
  );
}

function Timings({ t }: { t: Record<string, number> }) {
  const parts = ["diff_ms", "parse_ms", "security_ms", "graph_ms", "tests_ms", "risk_ms", "ai_ms"];
  const total = Math.max(1, parts.reduce((s, k) => s + (t[k] ?? 0), 0));
  const colors = ["#38bdf8", "#06b6d4", "#ef4444", "#f59e0b", "#10b981", "#8b5cf6", "#a78bfa"];
  return (
    <section className="card p-5">
      <div className="eyebrow mb-3">Pipeline timing · {t.total_ms} ms total</div>
      <div className="flex h-3 overflow-hidden rounded">
        {parts.map((k, i) => (
          <div key={k} title={`${k}: ${t[k]} ms`} style={{ width: `${((t[k] ?? 0) / total) * 100}%`, background: colors[i] }} />
        ))}
      </div>
      <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[12px] text-muted">
        {parts.map((k, i) => (
          <span key={k}><span className="mr-1 inline-block h-2 w-2 rounded-sm" style={{ background: colors[i] }} />{k.replace("_ms", "")} {t[k] ?? 0} ms</span>
        ))}
      </div>
    </section>
  );
}

function Findings({ r }: { r: Report }) {
  if (r.findings.length === 0) return <div className="card p-5 text-soft">No security or static findings on the changed lines.</div>;
  return (
    <section className="card overflow-x-auto p-2">
      <table className="data">
        <thead><tr><th>Severity</th><th>Finding</th><th>Location</th><th>CWE</th><th>Status</th><th>Engine</th></tr></thead>
        <tbody>
          {r.findings.map((f, i) => (
            <tr key={i}>
              <td><SeverityPill severity={f.severity} /></td>
              <td>
                <div className="font-semibold">{f.title}</div>
                <div className="text-[12px] text-muted">{f.message}</div>
                {f.snippet && <pre className="mono mt-1.5 overflow-x-auto rounded bg-card-2 p-2 text-[11px] text-soft">{f.snippet}</pre>}
              </td>
              <td className="mono whitespace-nowrap">{f.file}:{f.line}</td>
              <td className="mono">{f.cwe ?? "—"}</td>
              <td>{f.state === "fixed" ? <span className="font-bold text-emerald">✓ fixed</span> : <span className="font-bold text-danger">new</span>}</td>
              <td className="text-muted">{f.engine}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

function Impact({ r }: { r: Report }) {
  return (
    <div className="space-y-5">
      <section className="card p-5">
        <div className="eyebrow mb-3">Blast radius graph</div>
        <BlastGraph report={r} />
        <p className="mt-2 text-[12px] text-muted">Cyan: changed symbols · grey: callers by depth · red: sensitive code paths · amber dashed: new dependencies.</p>
      </section>
      <section className="card overflow-x-auto p-2">
        <table className="data">
          <thead><tr><th>Caller</th><th>Location</th><th>Depth</th><th>Via</th><th></th></tr></thead>
          <tbody>
            {r.impacted.map((i) => (
              <tr key={i.qualified}>
                <td><code>{local(i.qualified)}</code></td>
                <td className="mono">{i.file}:{i.line}</td>
                <td>{i.depth}</td>
                <td><code>{local(i.via)}</code></td>
                <td>{i.sensitive && <span className="text-[11px] font-bold text-danger">SENSITIVE</span>}</td>
              </tr>
            ))}
            {r.impacted.length === 0 && <tr><td colSpan={5} className="text-muted">No callers outside the change.</td></tr>}
          </tbody>
        </table>
      </section>
    </div>
  );
}

function Tests({ r }: { r: Report }) {
  const all = r.tests.map((t) => t.command).join("\n");
  return (
    <div className="space-y-5">
      <section className="card p-5">
        <div className="flex items-center justify-between">
          <div className="eyebrow">Recommended regression tests</div>
          {all && (
            <button onClick={() => navigator.clipboard.writeText(all)} className="rounded border border-line px-3 py-1 text-[12px] hover:border-cyan">
              Copy commands
            </button>
          )}
        </div>
        {r.tests.length === 0 ? (
          <p className="mt-3 text-soft">No existing test reaches the changed code.</p>
        ) : (
          <table className="data mt-3">
            <thead><tr><th>Test</th><th>Distance</th><th>Covers</th><th>Command</th></tr></thead>
            <tbody>
              {r.tests.map((t) => (
                <tr key={t.qualified}>
                  <td><code>{local(t.qualified)}</code><div className="text-[11px] text-muted">{t.file}:{t.line}</div></td>
                  <td>{t.distance === 99 ? <span className="text-muted">naming</span> : `${t.distance} hop${t.distance === 1 ? "" : "s"}`}</td>
                  <td className="text-[12px]">{t.covers.map(local).join(", ")}</td>
                  <td><code className="text-cyan-light">{t.command}</code></td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      {r.untested.length > 0 && (
        <section className="card border-amber/50 p-5">
          <div className="eyebrow" style={{ color: "var(--color-amber)" }}>Coverage gaps</div>
          <p className="mt-2 text-soft">No test reaches: {r.untested.map((u) => <code key={u} className="mr-2">{local(u)}</code>)}</p>
        </section>
      )}
    </div>
  );
}

function Ai({ r }: { r: Report }) {
  if (!r.ai) return <div className="card p-5 text-soft">AI review was not run (Ollama unavailable, disabled, or timed out). The deterministic report above is complete on its own.</div>;
  return (
    <div className="space-y-5">
      <section className="card p-5">
        <div className="eyebrow">Summary · {r.ai.model} · {(r.ai.latency_ms / 1000).toFixed(1)} s</div>
        <p className="mt-2 leading-relaxed text-soft">{r.ai.summary}</p>
        <p className="mt-3 text-[12px] text-muted">{r.ai.comments.length} comment(s) verified against the diff · {r.ai.rejected} unverifiable comment(s) dropped by the validator.</p>
      </section>
      {r.ai.comments.map((c, i) => (
        <section key={i} className="card p-5">
          <div className="mono text-cyan">{c.file}:{c.line}</div>
          <p className="mt-2 text-soft">{c.body}</p>
          {c.suggestion && <pre className="mono mt-3 overflow-x-auto rounded border border-emerald/40 bg-card-2 p-3 text-[12px] text-emerald">{c.suggestion}</pre>}
        </section>
      ))}
      {r.ai.proposed_tests.length > 0 && (
        <section className="card p-5">
          <div className="eyebrow mb-2">AI-proposed tests</div>
          {r.ai.proposed_tests.map((t, i) => (
            <pre key={i} className="mono mt-2 overflow-x-auto rounded bg-card-2 p-3 text-[12px] text-soft">{t}</pre>
          ))}
        </section>
      )}
    </div>
  );
}
