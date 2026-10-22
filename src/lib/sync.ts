import { useEffect } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { queryClient } from "./queryClient";
import { useSettingsStore } from "./settings";
import { toast } from "./toast";
import { brief, log } from "./log";
import type { AutoSync, BoardMonitorPayload, MirrorUpdatedPayload, SyncMode, SyncReport, SyncScope, SyncStatus } from "./types";

export const MIRROR_UPDATED_EVENT = "mirror-updated";
export const SYNC_STATUS_EVENT = "sync-status";
export const BOARD_MONITOR_EVENT = "board-monitor";

/** Queries that only read the mirror (SQLite), so invalidating them never
 *  calls GitHub. The Review's live ["pr-meta"] / ["pr-threads"] queries are
 *  fetchers themselves and are deliberately not listed. */
const MIRROR_QUERY_KEYS = [
  ["inbox"],
  ["inbox-archive"],
  ["inbox-meta"],
  ["conversations"],
  ["board"],
  ["board-last-refresh"],
  ["board-tracked"],
  ["board-card-for-pr"],
  ["pr-meta-mirror"],
  ["pr-threads-mirror"],
  ["reviews"],
];

export function invalidateMirrorQueries() {
  for (const queryKey of MIRROR_QUERY_KEYS) queryClient.invalidateQueries({ queryKey });
}

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** React Query refetch options for GitHub reads the frontend still schedules
 *  itself (the repo PR list, the Review's PR facets), following Auto Sync. */
export function autoSyncRefetch(autoSync: AutoSync, periodicMs: number) {
  return {
    refetchOnWindowFocus: autoSync !== "off",
    refetchInterval: autoSync === "periodic" ? periodicMs : (false as const),
  };
}

/** Tells the sync engine a view of `scope` is on screen while `active`, and
 *  which entity ids it shows. Mounting syncs the scope if it's stale. */
export function useSyncScope(scope: SyncScope, active: boolean, visibleIds: string[]) {
  const idsKey = visibleIds.join(",");
  useEffect(() => {
    if (!isTauriRuntime() || !active) return;
    log.debug("sync", `hint ${scope} on (${idsKey ? idsKey.split(",").length : 0} ids)`);
    api.syncHint(scope, true, idsKey ? idsKey.split(",") : []).catch(() => {});
  }, [scope, active, idsKey]);
  useEffect(() => {
    if (!isTauriRuntime() || !active) return;
    return () => {
      log.debug("sync", `hint ${scope} off`);
      api.syncHint(scope, false).catch(() => {});
    };
  }, [scope, active]);
}

function syncSummary(scope: SyncScope, mode: SyncMode, r: SyncReport): string {
  const what =
    scope === "inbox"
      ? `${r.itemCount} items, ${r.closedCount} closed`
      : scope === "board"
        ? `${r.cardCount} cards, ${r.autoTracked} newly tracked`
        : `${r.itemCount} inbox items, ${r.cardCount} board cards`;
  return `${mode === "full" ? "Full Sync" : "Synced"}: ${what} (${r.changed} updated, ${(r.durationMs / 1000).toFixed(1)}s)`;
}

export interface SyncVars {
  mode: SyncMode;
  visibleIds?: string[];
}

/** A Sync click (forced: fresh for everything on screen) or Full Sync. */
export function useSyncNow(scope: SyncScope) {
  return useMutation({
    mutationFn: ({ mode, visibleIds }: SyncVars) => api.syncNow(scope, mode, visibleIds ?? []),
    onSuccess: (r, { mode }) => {
      invalidateMirrorQueries();
      toast.success(syncSummary(scope, mode, r));
    },
    onError: (e) => toast.error(`Sync failed:\n${String(e)}`),
  });
}

export function useSyncStatus(): SyncStatus | undefined {
  return useQuery({
    queryKey: ["sync-status"],
    queryFn: api.syncStatus,
    enabled: isTauriRuntime(),
    staleTime: Infinity,
  }).data;
}

function useTauriListener<T>(event: string, onPayload: (payload: T) => void) {
  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<T>(event, ({ payload }) => {
      log.debug("event", `${event} ${brief(payload)}`);
      onPayload(payload);
    })
      .then((dispose) => {
        if (disposed) dispose();
        else unlisten = dispose;
      })
      .catch((e) => console.warn(`Could not register ${event} listener`, e));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [event]);
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Toast text for a monitor round, or null when it did nothing worth showing. */
export function monitorSummary({ actions, error }: BoardMonitorPayload): { text: string; isError: boolean } | null {
  const ref = (a: { repo: string; number: number }) => `${a.repo}#${a.number}`;
  const of = (kind: string) => actions.filter((a) => a.kind === kind);
  const updated = of("updated");
  const reran = of("reran");
  const gaveUp = of("gave_up");
  const errors = of("error");
  const parts = [
    updated.length > 0 && `updated ${plural(updated.length, "branch", "branches")}`,
    reran.length > 0 && `reran ${plural(reran.length, "CI run", "CI runs")}`,
    gaveUp.length > 0 && `gave up on ${gaveUp.map(ref).join(", ")}`,
    errors.length > 0 && `failed: ${errors.map((a) => `${ref(a)} (${a.detail})`).join(", ")}`,
    error && error,
  ].filter(Boolean);
  if (parts.length === 0) return null;
  return { text: `Monitor: ${parts.join(" · ")}`, isError: errors.length > 0 || error != null };
}

/** App-level wiring: mirror/status events, window visibility + focus, and the
 *  Auto Sync settings, all forwarded to the backend scheduler. */
export function useSyncBridge() {
  useTauriListener<MirrorUpdatedPayload>(MIRROR_UPDATED_EVENT, () => invalidateMirrorQueries());
  useTauriListener<SyncStatus>(SYNC_STATUS_EVENT, (status) => queryClient.setQueryData(["sync-status"], status));

  useTauriListener<BoardMonitorPayload>(BOARD_MONITOR_EVENT, (payload) => {
    queryClient.invalidateQueries({ queryKey: ["board-monitor-log"] });
    const summary = monitorSummary(payload);
    if (!summary) return;
    if (summary.isError) toast.error(summary.text);
    else toast.success(summary.text);
  });

  const autoSync = useSettingsStore((s) => s.autoSync);
  const discoveryIntervalMin = useSettingsStore((s) => s.discoveryIntervalMin);
  const boardMonitor = useSettingsStore((s) => s.boardMonitor);
  const boardMonitorIntervalMin = useSettingsStore((s) => s.boardMonitorIntervalMin);
  useEffect(() => {
    if (!isTauriRuntime()) return;
    log.debug(
      "sync",
      `configure autoSync=${autoSync} discovery=${discoveryIntervalMin}min monitor=${boardMonitor}/${boardMonitorIntervalMin}min`,
    );
    api.syncConfigure(autoSync, discoveryIntervalMin, boardMonitor, boardMonitorIntervalMin).catch(() => {});
  }, [autoSync, discoveryIntervalMin, boardMonitor, boardMonitorIntervalMin]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const report = () => {
      const visible = document.visibilityState === "visible";
      const focused = document.hasFocus();
      log.debug("window", `visible=${visible} focused=${focused}`);
      return api.syncWindowState(visible, focused).catch(() => {});
    };
    report();
    window.addEventListener("focus", report);
    window.addEventListener("blur", report);
    document.addEventListener("visibilitychange", report);
    return () => {
      window.removeEventListener("focus", report);
      window.removeEventListener("blur", report);
      document.removeEventListener("visibilitychange", report);
    };
  }, []);
}
