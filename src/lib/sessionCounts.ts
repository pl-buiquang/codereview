import { useQuery } from "@tanstack/react-query";
import { api } from "./api";
import type { AgentSession } from "./types";

export function isWaiting(s: Pick<AgentSession, "live" | "status" | "waiting_for">): boolean {
  return s.live && (s.status === "waiting" || s.waiting_for != null);
}

/** Live sessions (no history). Shares its cache entry with the Board's live-session query; polls slowly
 *  because listing spawns `claude agents`, `ps` and `tmux`. */
export function useLiveSessions(): AgentSession[] {
  const query = useQuery({
    queryKey: ["claude-sessions", 0],
    queryFn: () => api.listClaudeSessions(0, ""),
    refetchInterval: 10_000,
    refetchOnWindowFocus: true,
  });
  return (query.data?.sessions ?? []).filter((s) => s.live);
}

/** Live sessions asking for input, for the sidebar badge. */
export function useWaitingSessions(): AgentSession[] {
  return useLiveSessions().filter(isWaiting);
}
