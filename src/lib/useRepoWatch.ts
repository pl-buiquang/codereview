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

/** Holds watchers for every repo in `repoIds` while mounted. Returns true once
 *  every acquire has settled (some repos may still be unwatchable). */
export function useRepoWatches(repoIds: number[] | null): boolean {
  const [settled, setSettled] = useState(false);
  const key = repoIds ? [...repoIds].sort((a, b) => a - b).join(",") : null;

  useEffect(() => {
    if (key == null) return;
    const ids = key ? key.split(",").map(Number) : [];
    let disposed = false;
    const acquired: number[] = [];
    void Promise.all(
      ids.map((id) =>
        api
          .watchAcquire(id)
          .then((ok) => {
            if (!ok) return;
            if (disposed) void api.watchRelease(id);
            else acquired.push(id);
          })
          .catch(() => {}),
      ),
    ).then(() => {
      if (!disposed) setSettled(true);
    });
    return () => {
      disposed = true;
      for (const id of acquired) void api.watchRelease(id);
      setSettled(false);
    };
  }, [key]);

  return settled;
}
