// Typed client for the Hermit REST API (same origin as the dashboard).

export type Tier = "low" | "medium" | "high";

export interface AnalysisRow {
  id: number;
  repo: string;
  pr_number: number | null;
  title: string;
  author: string;
  base_ref: string;
  head_ref: string;
  head_sha: string;
  status: "queued" | "running" | "done" | "failed";
  stage: string;
  risk_score: number | null;
  risk_tier: Tier | null;
  error: string | null;
  created_at: string;
  finished_at: string | null;
}

export interface LineRange { start: number; end: number }

export interface Report {
  repo: string;
  base: string;
  head: string;
  files: { path: string; status: string; language: string | null; additions: number; deletions: number; added: LineRange[] }[];
  symbols: {
    symbol: { name: string; qualified: string; kind: string; file: string; range: LineRange; signature: string; complexity: number; is_test: boolean };
    change: "added" | "modified" | "deleted";
    signature_changed: boolean;
    complexity_before: number;
    complexity_after: number;
  }[];
  findings: {
    rule_id: string; title: string; message: string;
    severity: "info" | "low" | "medium" | "high" | "critical";
    cwe: string | null; file: string; line: number; end_line: number; snippet: string;
    state: "new" | "fixed"; engine: string;
  }[];
  impacted: { qualified: string; file: string; line: number; depth: number; via: string; sensitive: boolean }[];
  new_dependencies: { from: string; to: string; file: string; line: number }[];
  tests: { qualified: string; file: string; line: number; distance: number; covers: string[]; command: string }[];
  untested: string[];
  risk: { score: number; tier: Tier; factors: { name: string; points: number; max: number; detail: string }[] };
  ai: null | {
    model: string; summary: string;
    comments: { file: string; line: number; body: string; suggestion: string | null }[];
    proposed_tests: string[]; rejected: number; latency_ms: number;
  };
  timings: Record<string, number>;
}

export interface Health {
  ok: boolean; database: boolean; ollama: boolean; models: string[]; semgrep: boolean;
  github_app: boolean; webhook_secret: boolean; allow_local: boolean; version: string;
}

async function json<T>(res: Response): Promise<T> {
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error((body as { error?: string }).error ?? res.statusText);
  return body as T;
}

export const api = {
  health: () => fetch("/api/health").then(json<Health>),
  list: () => fetch("/api/analyses?limit=100").then(json<AnalysisRow[]>),
  get: (id: number) =>
    fetch(`/api/analyses/${id}`).then(json<{ analysis: AnalysisRow; report: Report | null; markdown: string | null }>),
  analyzeLocal: (repo_path: string, base: string, head: string, title: string) =>
    fetch("/api/analyses", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ repo_path, base, head, title }),
    }).then(json<{ analysis_id: number }>),
};

export interface ProgressEvent { id: number; status: string; stage: string; risk_score?: number | null }

/** Subscribes to live pipeline progress (Server-Sent Events). Returns an unsubscribe fn. */
export function subscribe(onEvent: (e: ProgressEvent) => void): () => void {
  const es = new EventSource("/api/events");
  es.onmessage = (m) => {
    try { onEvent(JSON.parse(m.data)); } catch { /* ignore malformed */ }
  };
  return () => es.close();
}

export const local = (q: string) => q.split("::").pop() ?? q;

export const tierColor: Record<Tier, string> = {
  low: "var(--color-emerald)",
  medium: "var(--color-amber)",
  high: "var(--color-danger)",
};
