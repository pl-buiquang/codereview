import type { AgentSession } from "./types";

export function shellQuote(s: string): string {
  return /^[\w./~@%+=:-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}

export function attachCommand(s: Pick<AgentSession, "session_id" | "job_id">): string {
  return `claude attach ${s.job_id ?? s.session_id.slice(0, 8)}`;
}

/** The shell command resuming a session, or `null` when its harness has no known resume command. */
export function resumeCommand(s: Pick<AgentSession, "cwd" | "session_id"> & { harness?: string | null }): string | null {
  let resume: string;
  switch (s.harness) {
    case "claude":
      resume = `claude --resume ${shellQuote(s.session_id)}`;
      break;
    case "opencode":
      resume = `opencode --session ${shellQuote(s.session_id)}`;
      break;
    case "codex":
      resume = `codex resume ${shellQuote(s.session_id)}`;
      break;
    default:
      return null;
  }
  return `cd ${shellQuote(s.cwd)}; ${resume}`;
}
