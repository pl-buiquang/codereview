import { useCallback, useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { BoardCard, RoutineRun, RoutineRunStatus } from "./types";

export const ROUTINE_RUN_UPDATED_EVENT = "routine-run-updated";
export const ROUTINE_RUN_EVENT = "routine-run-event";

export const ACTIVE_STATUSES: RoutineRunStatus[] = ["queued", "running"];

/** Finished runs that want a human: the run said so, failed, or ran out of time. */
export const PROBLEM_STATUSES: RoutineRunStatus[] = ["needs_attention", "error", "timeout"];

export const isActiveRun = (status: RoutineRunStatus) => status === "queued" || status === "running";

export const STATUS_LABEL: Record<RoutineRunStatus, string> = {
  queued: "Queued",
  running: "Running",
  ok: "Done",
  needs_attention: "Needs attention",
  error: "Failed",
  killed: "Stopped",
  timeout: "Timed out",
  interrupted: "Interrupted",
};

/** Canonical dedup keys, mirroring `routines::context::light` in the backend. */
export const prContextKey = (repo: string, number: number) => `pr:${repo.toLowerCase()}#${number}`;
export const issueContextKey = (repo: string, number: number) => `issue:${repo.toLowerCase()}#${number}`;
export const cardContextKey = (card: Pick<BoardCard, "type" | "repo" | "number">) =>
  card.type === "pr" ? prContextKey(card.repo, card.number) : issueContextKey(card.repo, card.number);

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** Keep routine queries fresh from the backend's run events. Mount once, at the app root. */
export function useRoutineEvents() {
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const register = (event: string, fn: (payload: { run_id: number }) => void) =>
      listen<{ run_id: number }>(event, ({ payload }) => fn(payload))
        .then((dispose) => (disposed ? dispose() : unlisteners.push(dispose)))
        .catch((e) => console.warn(`Could not register ${event} listener`, e));
    register(ROUTINE_RUN_UPDATED_EVENT, () => {
      queryClient.invalidateQueries({ queryKey: ["routine-runs"] });
      queryClient.invalidateQueries({ queryKey: ["routine-options"] });
    });
    register(ROUTINE_RUN_EVENT, ({ run_id }) => {
      queryClient.invalidateQueries({ queryKey: ["routine-run-events", run_id] });
    });
    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, [queryClient]);
}

/** Queued / running runs grouped by context key, for the badges on cards and reviews. */
export function useActiveRoutineRuns(): Map<string, RoutineRun[]> {
  const query = useQuery({
    queryKey: ["routine-runs", "active"],
    queryFn: () => api.routineRunList({ statuses: ACTIVE_STATUSES }),
    refetchInterval: 15_000,
  });
  return useMemo(() => {
    const m = new Map<string, RoutineRun[]>();
    for (const run of query.data ?? []) m.set(run.context_key, [...(m.get(run.context_key) ?? []), run]);
    return m;
  }, [query.data]);
}

const SEEN_KEY = "routines_problems_seen_at";

/** When the Routines tab was last looked at; first use starts from now, so old failures don't flood the badge. */
function readSeenAt(): number {
  const stored = Number(localStorage.getItem(SEEN_KEY));
  if (stored > 0) return stored;
  const now = Date.now();
  localStorage.setItem(SEEN_KEY, String(now));
  return now;
}

/** Runs that ended needing attention, failed or timed out since the Routines tab was last open (badge source). */
export function useUnseenRoutineProblems(): { unseen: RoutineRun[]; markSeen: () => void } {
  const [seenAt, setSeenAt] = useState(readSeenAt);
  const query = useQuery({
    queryKey: ["routine-runs", "problems"],
    queryFn: () => api.routineRunList({ statuses: PROBLEM_STATUSES, limit: 50 }),
    refetchInterval: 15_000,
  });
  const unseen = useMemo(
    () => (query.data ?? []).filter((r) => r.ended_at != null && Date.parse(r.ended_at) > seenAt),
    [query.data, seenAt],
  );
  const markSeen = useCallback(() => {
    const now = Date.now();
    localStorage.setItem(SEEN_KEY, String(now));
    setSeenAt(now);
  }, []);
  return { unseen, markSeen };
}

export function formatDuration(startIso: string | null, endIso: string | null): string {
  if (!startIso) return "";
  const start = new Date(startIso).getTime();
  const end = endIso ? new Date(endIso).getTime() : Date.now();
  const secs = Math.max(0, Math.round((end - start) / 1000));
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ${secs % 60}s`;
  return `${Math.floor(mins / 60)}h ${mins % 60}m`;
}

/** One renderable step of a run's stream-json transcript. */
export type TranscriptItem =
  | { kind: "text"; text: string }
  | { kind: "tool"; id: string; name: string; input: string; result?: string; isError?: boolean }
  | { kind: "result"; text: string; isError: boolean; cost: number | null };

const clip = (s: string, n = 4000) => (s.length > n ? `${s.slice(0, n)}…` : s);

function toolResultText(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content))
    return content
      .map((c) => (c && typeof c === "object" && "text" in c ? String((c as { text: unknown }).text) : ""))
      .join("\n");
  return content == null ? "" : JSON.stringify(content);
}

/** Fold raw stream-json lines into assistant text, tool calls (with their results) and the final result. */
export function buildTranscript(lines: string[]): TranscriptItem[] {
  const items: TranscriptItem[] = [];
  const toolById = new Map<string, Extract<TranscriptItem, { kind: "tool" }>>();
  for (const line of lines) {
    let v: Record<string, any>;
    try {
      v = JSON.parse(line);
    } catch {
      continue;
    }
    if (v.type === "assistant") {
      for (const block of v.message?.content ?? []) {
        if (block.type === "text" && block.text?.trim()) items.push({ kind: "text", text: block.text });
        else if (block.type === "tool_use") {
          const tool = { kind: "tool" as const, id: block.id, name: block.name, input: clip(JSON.stringify(block.input ?? {}, null, 2)) };
          toolById.set(block.id, tool);
          items.push(tool);
        }
      }
    } else if (v.type === "user") {
      const content = v.message?.content;
      if (!Array.isArray(content)) continue;
      for (const block of content) {
        if (block.type !== "tool_result") continue;
        const tool = toolById.get(block.tool_use_id);
        if (tool) {
          tool.result = clip(toolResultText(block.content));
          tool.isError = !!block.is_error;
        }
      }
    } else if (v.type === "result") {
      items.push({
        kind: "result",
        text: typeof v.result === "string" ? v.result : "",
        isError: !!v.is_error,
        cost: typeof v.total_cost_usd === "number" ? v.total_cost_usd : null,
      });
    }
  }
  return items;
}
