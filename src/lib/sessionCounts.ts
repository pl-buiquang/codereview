import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import type { ClaudeSession } from "./types";

export function isWaiting(s: Pick<ClaudeSession, "live" | "status" | "waiting_for">): boolean {
  return s.live && (s.status === "waiting" || s.waiting_for != null);
}

/** Live sessions asking for input, for the sidebar badge. Shares its cache entry with the Board's live-session
 *  query; polls slowly because listing spawns `claude agents`, `ps` and `tmux`. */
export function useWaitingSessions(): ClaudeSession[] {
  const query = useQuery({
    queryKey: ["claude-sessions", 0],
    queryFn: () => api.listClaudeSessions(0, ""),
    refetchInterval: 10_000,
    refetchOnWindowFocus: true,
  });
  return (query.data?.sessions ?? []).filter(isWaiting);
}
