import type { UseMutationResult } from "@tanstack/react-query";
import { useSyncStatus, type SyncVars } from "../lib/sync";
import type { SyncReport, SyncScope } from "../lib/types";
import { Icon } from "./icons";
import { KebabMenu } from "./KebabMenu";

/** The view's Sync button (fresh for everything on screen; ⇧-click = Full
 *  Sync), plus a kebab with Full Sync. Spins while a background sync of the
 *  same scope runs. */
export function SyncButton({
  scope,
  sync,
  visibleIds,
}: {
  scope: SyncScope;
  sync: UseMutationResult<SyncReport, unknown, SyncVars>;
  visibleIds: string[];
}) {
  const status = useSyncStatus();
  const background = !sync.isPending && (status?.running ?? []).some((s) => s === scope || s === "all");
  return (
    <>
      <button
        className="btn btn-primary"
        onClick={(e) => sync.mutate({ mode: e.shiftKey ? "full" : "forced", visibleIds })}
        disabled={sync.isPending}
        title="Sync what's on screen from GitHub (⇧-click: Full Sync)"
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
        )}
      </KebabMenu>
    </>
  );
}
