import type { AgentSession, HostSessions, HostSessionsStatus, RemoteSession } from "./types";
import { attachCommand, resumeInvocation, shellQuote } from "./sessionCommands";

export const HOST_STATUS_LABEL: Record<HostSessionsStatus, string> = {
  ok: "connected",
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

const CONTROL = /[\x00-\x1f\x7f]/;

type SshFields = Pick<RemoteSession, "host_ssh_target" | "harness" | "session_id" | "cwd" | "job_id" | "tmux">;

/**
 * The command to paste in a terminal to reach a remote session, or why there is none.
 *
 * Everything the remote reports is untrusted: a control character (a bracketed-paste escape, a newline)
 * would run on paste, so such a session gets no command; so does an id or target starting with `-`.
 * The command runs through the remote's login shell (`$SHELL -lc`, best effort for fish/csh), because
 * harness binaries are often on its PATH only. With a tmux pane known it attaches to it (an exact
 * session match); a background Claude job gets `claude attach`; anything else resumes the session in
 * its folder, which starts a second client on it.
 */
export function sshAttach(s: SshFields): { command: string; kind: "tmux" | "attach" | "resume" } | { command: null; reason: string } {
  const fields = [s.host_ssh_target, s.session_id, s.cwd, s.job_id, s.tmux?.session, s.tmux?.window, s.tmux?.pane];
  if (fields.some((f) => f != null && CONTROL.test(f))) return { command: null, reason: "unsupported characters in the session's details" };
  if (s.host_ssh_target === "" || s.host_ssh_target.startsWith("-")) return { command: null, reason: "unsupported SSH target" };
  if (s.session_id.startsWith("-") || s.job_id?.startsWith("-") || s.tmux?.session.startsWith("-")) {
    return { command: null, reason: "unsupported characters in the session's details" };
  }
  let inner: string | null;
  let kind: "tmux" | "attach" | "resume";
  if (s.tmux) {
    const { session, window, pane } = s.tmux;
    if (!/^@\d+$/.test(window) || !/^%\d+$/.test(pane) || session === "") return { command: null, reason: "unsupported tmux location" };
    kind = "tmux";
    inner = String.raw`tmux attach -t ${shellQuote(`=${session}`)} \; select-window -t ${window} \; select-pane -t ${pane}`;
  } else if (s.harness === "claude" && s.job_id) {
    kind = "attach";
    inner = attachCommand(s);
  } else {
    kind = "resume";
    const resume = resumeInvocation(s.harness, s.session_id);
    inner = resume === null ? null : `cd -- ${shellQuote(s.cwd)} && ${resume}`;
  }
  if (inner === null) return { command: null, reason: `no attach command known for harness ${s.harness}` };
  return { command: `ssh -t -- ${shellQuote(s.host_ssh_target)} ${shellQuote(`$SHELL -lc ${shellQuote(inner)}`)}`, kind };
}

export function sshAttachCommand(s: SshFields): string | null {
  return sshAttach(s).command;
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
