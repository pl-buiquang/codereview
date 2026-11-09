import type { AgentSession, HostSessions, HostSessionsStatus, RemoteSession } from "./types";
import { attachCommand, resumeCommand, shellQuote } from "./sessionCommands";

export const HOST_STATUS_LABEL: Record<HostSessionsStatus, string> = {
  ok: "connected",
  connecting: "connecting…",
  unreachable: "unreachable",
  protocol_mismatch: "protocol mismatch",
  disabled: "disabled",
  error: "error",
};

/** `""` every host, `"local"` this machine only, `"host:<id>"` one remote host. */
export type HostFilter = string;

export function hostFilterMatches(filter: HostFilter, hostId: number | null): boolean {
  if (filter === "") return true;
  if (filter === "local") return hostId == null;
  return filter === `host:${hostId}`;
}

/** A session's identity in the UI: `(host, harness, session id)`; a local session has no host. */
export function remoteKey(s: Pick<RemoteSession, "host_id" | "harness" | "session_id">): string {
  return `${s.host_id}:${s.harness}:${s.session_id}`;
}

export function isRemote(s: AgentSession | RemoteSession): s is RemoteSession {
  return "host_id" in s && (s as RemoteSession).host_id != null;
}

/**
 * The command to paste in a terminal to reach a remote session, or `null` when none is known.
 * Runs through the remote's login shell, because harness binaries are often on its PATH only.
 * With a tmux pane known the command attaches to it; a background Claude job gets `claude attach`;
 * anything else resumes the session in its folder (best effort).
 */
export function sshAttachCommand(s: Pick<RemoteSession, "host_ssh_target" | "harness" | "session_id" | "cwd" | "job_id" | "tmux">): string | null {
  let inner: string | null;
  if (s.tmux) {
    const { session, window, pane } = s.tmux;
    // `\;` is tmux's command separator as the remote shell must hand it over.
    inner = String.raw`tmux attach -t ${shellQuote(session)} \; select-window -t ${shellQuote(window)} \; select-pane -t ${shellQuote(pane)}`;
  } else if (s.harness === "claude" && s.job_id) {
    inner = attachCommand(s);
  } else {
    inner = resumeCommand(s);
  }
  if (inner === null) return null;
  return `ssh -t ${shellQuote(s.host_ssh_target)} ${shellQuote(`$SHELL -lc ${shellQuote(inner)}`)}`;
}

export interface MergedSessions {
  /** Local sessions first (their order is kept), then each host's by host name, newest activity first. */
  sessions: Array<AgentSession | RemoteSession>;
  /** Hosts that gave no sessions because they could not be asked, by host name. */
  down: HostSessions[];
}

/** Merge the local list with the per-host answers. A host that is down never fails the list. */
export function mergeRemoteSessions(local: AgentSession[], hosts: HostSessions[]): MergedSessions {
  const byName = [...hosts].sort((a, b) => a.host_name.localeCompare(b.host_name) || a.host_id - b.host_id);
  const remote = byName.flatMap((h) => (h.status === "ok" ? [...h.sessions].sort((a, b) => b.updated_at - a.updated_at) : []));
  return { sessions: [...local, ...remote], down: byName.filter((h) => h.status !== "ok") };
}
