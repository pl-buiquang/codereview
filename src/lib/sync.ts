import { useEffect } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { queryClient } from "./queryClient";
import { useSettingsStore } from "./settings";
import { toast } from "./toast";
import type { AutoSync, MirrorUpdatedPayload, SyncMode, SyncReport, SyncScope, SyncStatus } from "./types";

export const MIRROR_UPDATED_EVENT = "mirror-updated";
export const SYNC_STATUS_EVENT = "sync-status";

/** Queries that only read the mirror (SQLite), so invalidating them never
 *  calls GitHub. The Review's live ["pr-meta"] / ["pr-threads"] queries are
 *  fetchers themselves and are deliberately not listed. */
const MIRROR_QUERY_KEYS = [
  ["inbox"],
  ["closed"],
  ["archive"],
  ["inbox-meta"],
  ["board"],
  ["board-last-refresh"],
  ["board-tracked"],
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
    api.syncHint(scope, true, idsKey ? idsKey.split(",") : []).catch(() => {});
  }, [scope, active, idsKey]);
  useEffect(() => {
    if (!isTauriRuntime() || !active) return;
    return () => {
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
    listen<T>(event, ({ payload }) => onPayload(payload))
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

/** App-level wiring: mirror/status events, window visibility + focus, and the
 *  Auto Sync settings, all forwarded to the backend scheduler. */
export function useSyncBridge() {
  useTauriListener<MirrorUpdatedPayload>(MIRROR_UPDATED_EVENT, () => invalidateMirrorQueries());
  useTauriListener<SyncStatus>(SYNC_STATUS_EVENT, (status) => queryClient.setQueryData(["sync-status"], status));

  const autoSync = useSettingsStore((s) => s.autoSync);
  const discoveryIntervalMin = useSettingsStore((s) => s.discoveryIntervalMin);
  useEffect(() => {
    if (!isTauriRuntime()) return;
    api.syncConfigure(autoSync, discoveryIntervalMin).catch(() => {});
  }, [autoSync, discoveryIntervalMin]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const report = () =>
      api.syncWindowState(document.visibilityState === "visible", document.hasFocus()).catch(() => {});
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
