import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { buildDigest } from "./boardDigest";
import { useSettingsStore } from "./settings";

/** Ids of the cards that need my action (the uncapped "Do next" ranking) in the enabled kinds, snoozed or not.
 *  These are the cards the sidebar badge can count, so the only ones worth a Snooze button. */
export function useBoardBadgeCardIds(): Set<string> {
  const board = useQuery({ queryKey: ["board"], queryFn: api.boardList });
  const kinds = useSettingsStore((s) => s.boardBadgeKinds);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  return useMemo(
    () =>
      new Set(
        buildDigest(board.data ?? [], new Date(now))
          .actionable.filter((a) => kinds[a.kind])
          .map((a) => a.cardId),
      ),
    [board.data, kinds, now],
  );
}

/** Cards in the badge set that aren't snoozed, for the sidebar badge. Re-evaluated every minute so a snooze
 *  lapses on its own. */
export function useBoardActionCount(): number {
  const ids = useBoardBadgeCardIds();
  const snoozes = useSettingsStore((s) => s.boardSnoozes);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(id);
  }, []);
  return useMemo(() => [...ids].filter((id) => !((snoozes[id] ?? 0) > now)).length, [ids, snoozes, now]);
}
