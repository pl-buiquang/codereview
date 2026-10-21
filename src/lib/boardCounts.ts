import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import { buildDigest } from "./boardDigest";

/** Cards that need my action (the uncapped "Do next" ranking), for the sidebar badge. */
export function useBoardActionCount(): number {
  const board = useQuery({ queryKey: ["board"], queryFn: api.boardList });
  return useMemo(() => buildDigest(board.data ?? [], new Date()).actionable, [board.data]);
}
