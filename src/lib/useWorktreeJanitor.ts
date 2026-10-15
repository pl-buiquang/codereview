import { useEffect } from "react";
import { api } from "./api";
import type { QueryClient } from "@tanstack/react-query";
import { queryClient } from "./queryClient";
import { useSettingsStore } from "./settings";
import { toast } from "./toast";
import type { WorktreeGcAction, WorktreeGcReport } from "./types";

export const WORKTREE_GC_KEY = ["worktree-gc"] as const;
const INTERVAL_MS = 5 * 60_000;

export function summarizeActions(actions: WorktreeGcAction[]): string | null {
  const ok = actions.filter((a) => a.ok);
  const failed = actions.length - ok.length;
  const removed = ok.filter((a) => a.action.startsWith("remove") || a.action === "prune").length;
  const synced = ok.filter((a) => a.action === "fast-forward" || a.action === "resync").length;
  const parts = [
    removed && `removed ${removed}`,
    synced && `synced ${synced}`,
    failed && `${failed} failed`,
  ].filter(Boolean);
  return parts.length ? `Worktrees: ${parts.join(", ")}` : null;
}

export function publishReport(client: QueryClient, report: WorktreeGcReport | null, announce: boolean) {
  if (!report) return;
  const prev = client.getQueryData<WorktreeGcReport>(WORKTREE_GC_KEY);
  client.setQueryData(WORKTREE_GC_KEY, report);
  if (!announce || prev?.ran_at === report.ran_at) return;
  const msg = summarizeActions(report.actions);
  if (msg) toast.success(msg);
}

/** Runs the worktree janitor at startup and every 5 minutes (the backend throttles too). */
export function useWorktreeJanitor() {
  const autoRemove = useSettingsStore((s) => s.worktreeAutoRemove);
  const autoSync = useSettingsStore((s) => s.worktreeAutoSync);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window) || (!autoRemove && !autoSync)) return;
    const tick = () =>
      api
        .worktreeGcRun(autoRemove, autoSync, false)
        .then((r) => publishReport(queryClient, r, true))
        .catch((e) => console.warn("worktree janitor failed", e));
    tick();
    const id = setInterval(tick, INTERVAL_MS);
    return () => clearInterval(id);
  }, [autoRemove, autoSync]);
}
