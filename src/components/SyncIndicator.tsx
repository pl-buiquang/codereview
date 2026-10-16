import { useEffect, useRef, useState } from "react";
import { useSyncNow, useSyncStatus } from "../lib/sync";
import { timeAgo } from "../lib/timeAgo";
import { Icon } from "./icons";

function latest(...ts: (string | null)[]): string | null {
  const sorted = ts.filter((t): t is string => t != null).sort();
  return sorted[sorted.length - 1] ?? null;
}

/** HomeNav status of the GitHub sync engine, with Sync now / Full Sync. */
export function SyncIndicator({ collapsed }: { collapsed: boolean }) {
  const status = useSyncStatus();
  const syncAll = useSyncNow("all");
  const [open, setOpen] = useState(false);
  const [, setTick] = useState(0);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  if (!status) return null;
  const running = status.running.length > 0 || syncAll.isPending;
  const last = latest(status.lastInboxAt, status.lastBoardAt);
  const label = running
    ? status.full
      ? "Full Sync…"
      : "Syncing…"
    : status.lastError
      ? "Sync failed"
      : last
        ? `Synced ${timeAgo(last)}`
        : "Not synced yet";
  const row = (name: string, at: string | null) => (
    <div className="cr-sync-row">
      <span>{name}</span>
      <span className="muted">{at ? timeAgo(at) : "never"}</span>
    </div>
  );

  return (
    <div className="cr-sync" ref={ref}>
      <button
        className={`cr-nav-item cr-sync-btn${status.lastError && !running ? " error" : ""}`}
        onClick={() => setOpen((v) => !v)}
        title={collapsed ? label : "GitHub sync status"}
      >
        {running ? <span className="spinner" /> : <Icon name="refresh" size={15} />}
        {!collapsed && <span className="cr-sync-label">{label}</span>}
      </button>
      {open && (
        <div className="cr-sync-pop card" role="dialog" aria-label="Sync status">
          {row("Looked for new items", status.lastDiscoveryAt)}
          {row("Inbox", status.lastInboxAt)}
          {row("Board", status.lastBoardAt)}
          {status.lastError && (
            <div className="cr-sync-error" title={status.lastErrorAt ?? undefined}>
              {status.lastError}
            </div>
          )}
          <div className="cr-sync-actions">
            <button
              className="btn btn-sm btn-primary"
              disabled={running}
              title="Look for new items, then refetch what changed"
              onClick={() => syncAll.mutate({ mode: "forced" })}
            >
              Sync now
            </button>
            <button
              className="btn btn-sm"
              disabled={running}
              title="Refetch everything from GitHub and fetch every local clone"
              onClick={() => syncAll.mutate({ mode: "full" })}
            >
              Full Sync
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
