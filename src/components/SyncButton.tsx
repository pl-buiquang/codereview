import type { UseMutationResult } from "@tanstack/react-query";
import { useSyncStatus, type SyncVars } from "../lib/sync";
import type { SyncReport, SyncScope } from "../lib/types";
import { Icon } from "./icons";
import { KebabMenu } from "./KebabMenu";

/** The view's Sync button, plus a kebab with Full Sync. Spins while a
 *  background sync of the same scope runs.
 *  - `statusOnly` (Board): a click is a light sync (probe every card, refetch
 *    what changed) without discovery, which is slow; "Look for new PRs" in
 *    the kebab runs it.
 *  - otherwise a click is forced: discovery + refetch everything on screen.
 *  ⇧-click is always Full Sync. */
export function SyncButton({
  scope,
  sync,
  visibleIds,
  statusOnly = false,
}: {
  scope: SyncScope;
  sync: UseMutationResult<SyncReport, unknown, SyncVars>;
  visibleIds: string[];
  statusOnly?: boolean;
}) {
  const status = useSyncStatus();
  const background = !sync.isPending && (status?.running ?? []).some((s) => s === scope || s === "all");
  return (
    <>
      <button
        className="btn btn-primary"
        onClick={(e) =>
          sync.mutate({ mode: e.shiftKey ? "full" : statusOnly ? "light" : "forced", visibleIds })
        }
        disabled={sync.isPending}
        title={
          statusOnly
            ? "Check every card's state, CI and reviews on GitHub (⇧-click: Full Sync)"
            : "Sync what's on screen from GitHub (⇧-click: Full Sync)"
        }
      >
        {sync.isPending ? (
          <>
            <span className="spinner" /> Syncing…
          </>
        ) : (
          <>
            {background ? <span className="spinner" /> : <Icon name="refresh" size={13} />} Sync
          </>
        )}
      </button>
      <KebabMenu title="Sync options" disabled={sync.isPending}>
        {(close) => (
          <>
            {statusOnly && (
              <button
                role="menuitem"
                title="Search GitHub for new PRs of yours to track (also runs on its own every few minutes)"
                onClick={() => {
                  close();
                  sync.mutate({ mode: "forced", visibleIds });
                }}
              >
                Look for new PRs
              </button>
            )}
            <button
              role="menuitem"
              title="Refetch everything from GitHub, ignoring what looks fresh"
              onClick={() => {
                close();
                sync.mutate({ mode: "full", visibleIds });
              }}
            >
              Full Sync
            </button>
          </>
        )}
      </KebabMenu>
    </>
  );
}
