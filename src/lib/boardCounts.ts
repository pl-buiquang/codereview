import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { buildDigest } from "./boardDigest";
import { useSettingsStore } from "./settings";

/** Cards that need my action (the uncapped "Do next" ranking) in the enabled kinds and not snoozed, for the
 *  sidebar badge. Re-evaluated every minute so a snooze lapses on its own. */
export function useBoardActionCount(): number {
  const board = useQuery({ queryKey: ["board"], queryFn: api.boardList });
  const kinds = useSettingsStore((s) => s.boardBadgeKinds);
  const snoozes = useSettingsStore((s) => s.boardSnoozes);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  return useMemo(
    () =>
      buildDigest(board.data ?? [], new Date(now)).actionable.filter(
        (a) => kinds[a.kind] && !((snoozes[a.cardId] ?? 0) > now),
      ).length,
    [board.data, kinds, snoozes, now],
  );
}
