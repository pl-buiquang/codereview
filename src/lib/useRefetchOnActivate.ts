import { useEffect, useRef } from "react";

/** Refetches when `active` turns true: tabs stay mounted while hidden, so
 *  coming back to one doesn't remount (and so doesn't refetch) its queries. */
export function useRefetchOnActivate(active: boolean, refetch: () => unknown) {
  const wasActive = useRef(active);
  useEffect(() => {
    if (active && !wasActive.current) void refetch();
    wasActive.current = active;
  }, [active, refetch]);
}
