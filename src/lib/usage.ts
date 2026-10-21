import type { AiRun } from "./types";

export function formatCost(usd: number): string {
  if (usd === 0) return "$0";
  if (usd < 0.01) return "<$0.01";
  return `$${usd.toFixed(2)}`;
}

export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

export function formatDuration(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}

/** Bedrock models are reported as inference-profile ARNs; keep the trailing id. */
export function shortModel(model: string): string {
  return model.startsWith("arn:") ? (model.split("/").pop() ?? model) : model;
}

const FEATURE_LABELS: Record<string, string> = {
  chat: "Chat",
  ai_review: "AI review",
  focus: "Focus",
  board_overview: "Board overview",
  session_summary: "Session summary",
  routine: "Routine",
};

export function featureLabel(feature: string, variant?: string | null): string {
  const base = FEATURE_LABELS[feature] ?? feature;
  return variant ? `${base} · ${variant}` : base;
}

/** What a run was about: `repo#123`, `repo base…head`, or the run's own label. */
export function runSubject(run: Pick<AiRun, "repo" | "pr_number" | "base_ref" | "head_ref" | "label">): string {
  if (run.repo && run.pr_number != null) return `${run.repo}#${run.pr_number}`;
  if (run.repo && run.base_ref && run.head_ref) return `${run.repo} ${run.base_ref}…${run.head_ref}`;
  return run.label || "—";
}
