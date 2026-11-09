import type { AgentSession } from "./types";

export function shellQuote(s: string): string {
  return /^[\w./~@%+=:-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}

export function attachCommand(s: Pick<AgentSession, "session_id" | "job_id">): string {
  return `claude attach ${shellQuote(s.job_id ?? s.session_id.slice(0, 8))}`;
}

/** The harness' own "resume this session" invocation, or `null` when none is known. */
export function resumeInvocation(harness: string | null | undefined, sessionId: string): string | null {
  switch (harness) {
    case "claude":
      return `claude --resume ${shellQuote(sessionId)}`;
    case "opencode":
      return `opencode --session ${shellQuote(sessionId)}`;
    case "codex":
      return `codex resume ${shellQuote(sessionId)}`;
    default:
      return null;
  }
}

/** The shell command resuming a session, or `null` when its harness has no known resume command. */
export function resumeCommand(s: Pick<AgentSession, "cwd" | "session_id"> & { harness?: string | null }): string | null {
  const resume = resumeInvocation(s.harness, s.session_id);
  return resume === null ? null : `cd ${shellQuote(s.cwd)}; ${resume}`;
}
