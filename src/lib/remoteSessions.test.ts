import { describe, it, expect } from "vitest";
import { hostFilterMatches, mergeRemoteSessions, remoteKey } from "./remoteSessions";
import type { AgentSession, HostSessions, RemoteSession } from "./types";

const base = (id: string, updated_at: number): AgentSession => ({
  harness: "claude", session_id: id, pid: null, live: true, status: "idle", kind: null, attached: false, job_id: null,
  waiting_for: null, agent_view: null, name: id, cwd: "/x", git_branch: null, repo: null, tmux: null, tty: null,
  updated_at, focus: "no_terminal", summaries: [], original_name: null,
});
const remote = (id: string, updated_at: number, host_id: number, host_name: string): RemoteSession => ({
  ...base(id, updated_at), host_id, host_name, host_ssh_target: host_name,
});
const host = (host_id: number, host_name: string, sessions: RemoteSession[], status: HostSessions["status"] = "ok"): HostSessions => ({
  host_id, host_name, status, error: status === "ok" ? null : "down", sessions,
});

describe("mergeRemoteSessions", () => {
  it("keeps the local order, then hosts by name with newest activity first, and lists down hosts apart", () => {
    const local = [base("l2", 1), base("l1", 9)];
    const out = mergeRemoteSessions(local, [
      host(3, "nija", [], "unreachable"),
      host(2, "koza", [remote("k-old", 1, 2, "koza"), remote("k-new", 8, 2, "koza")]),
      host(1, "alpha", [remote("a1", 5, 1, "alpha")]),
    ]);
    expect(out.sessions.map((s) => s.session_id)).toEqual(["l2", "l1", "a1", "k-new", "k-old"]);
    expect(out.down.map((h) => h.host_name)).toEqual(["nija"]);
  });

  it("ignores sessions of a host that is not ok and works with no host", () => {
    const out = mergeRemoteSessions([base("l", 1)], [host(1, "x", [remote("r", 1, 1, "x")], "protocol_mismatch")]);
    expect(out.sessions.map((s) => s.session_id)).toEqual(["l"]);
    expect(mergeRemoteSessions([], []).sessions).toEqual([]);
  });
});

describe("identity and host filter", () => {
  it("keys remote sessions by host, harness and id", () => {
    expect(remoteKey(remote("s", 1, 4, "k"))).toBe("4:claude:s");
    expect(remoteKey(remote("s", 1, 5, "k"))).not.toBe(remoteKey(remote("s", 1, 4, "k")));
  });
  it("matches local, one host or all", () => {
    expect(hostFilterMatches("", null)).toBe(true);
    expect(hostFilterMatches("local", null)).toBe(true);
    expect(hostFilterMatches("local", 2)).toBe(false);
    expect(hostFilterMatches("host:2", 2)).toBe(true);
    expect(hostFilterMatches("host:2", null)).toBe(false);
  });
});
