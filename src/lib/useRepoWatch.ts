import { useEffect, useState } from "react";
import { api } from "./api";

/** Holds the backend git watcher for `repoId` while mounted (null = none).
 *  Returns whether the repo is watched; unwatched views should reload on
 *  window focus instead. */
export function useRepoWatch(repoId: number | null): boolean {
  const [watching, setWatching] = useState(false);

  useEffect(() => {
    if (repoId == null) return;
    let disposed = false;
    let acquired = false;
    api
      .watchAcquire(repoId)
      .then((ok) => {
        if (disposed) {
          if (ok) void api.watchRelease(repoId);
          return;
        }
        acquired = ok;
        setWatching(ok);
      })
      .catch(() => {});
    return () => {
      disposed = true;
      if (acquired) void api.watchRelease(repoId);
      setWatching(false);
    };
  }, [repoId]);

  return watching;
}
