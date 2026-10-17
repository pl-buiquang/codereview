import { QueryCache, QueryClient } from "@tanstack/react-query";
import { brief, log, logEnabled } from "./log";

/** Queries that poll for the log viewer itself; tracing them would feed the log with its own reads. */
const UNTRACED_KEYS = new Set(["app-log", "log-path"]);

const queryCache = new QueryCache({
  onError: (error, query) => {
    log.warn("query", `${brief(query.queryKey)} failed: ${brief(error)}`);
  },
});

export const queryClient = new QueryClient({
  queryCache,
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});

// Debug trace of every fetch the cache starts/finishes: a refetch loop shows
// up as the same key repeating.
const fetchStart = new Map<string, number>();
queryCache.subscribe((event) => {
  if (!logEnabled("debug")) return;
  const key = brief(event.query.queryKey);
  if (UNTRACED_KEYS.has(String(event.query.queryKey[0]))) return;
  if (event.type === "updated") {
    const { action } = event;
    if (action.type === "fetch") {
      fetchStart.set(key, performance.now());
      log.debug("query", `fetch ${key} (observers ${event.query.getObserversCount()})`);
    } else if (action.type === "success" || action.type === "error") {
      const t0 = fetchStart.get(key);
      fetchStart.delete(key);
      const ms = t0 === undefined ? "?" : Math.round(performance.now() - t0);
      log.debug("query", `${action.type} ${key} in ${ms}ms`);
    } else if (action.type === "invalidate") {
      log.debug("query", `invalidate ${key}`);
    }
  } else if (event.type === "added") {
    log.debug("query", `added ${key}`);
  } else if (event.type === "removed") {
    log.debug("query", `removed ${key}`);
  }
});
