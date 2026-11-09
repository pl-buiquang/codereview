import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { execFileSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { hostFilterMatches, mergeRemoteSessions, remoteKey, sshAttach } from "./remoteSessions";
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

describe("sshAttach with hostile remote values", () => {
  const ok = { host_ssh_target: "koza", harness: "claude", session_id: "abc", cwd: "/home/x", job_id: null, tmux: null };

  it("refuses control characters in any field, and leading dashes", () => {
    const bad = ["a\nb", "a\rb", "a\u001b[200~b", "a\u0000b", "a\u007fb"];
    for (const v of bad) {
      for (const key of ["host_ssh_target", "session_id", "cwd", "job_id"] as const) {
        const r = sshAttach({ ...ok, [key]: v });
        expect(r.command, `${key}=${JSON.stringify(v)}`).toBeNull();
      }
      for (const key of ["session", "window", "pane"] as const) {
        const tmux = { session: "s", window: "@1", pane: "%1", label: null, [key]: v };
        expect(sshAttach({ ...ok, tmux }).command).toBeNull();
      }
    }
    expect(sshAttach({ ...ok, host_ssh_target: "-oProxyCommand=x" }).command).toBeNull();
    expect(sshAttach({ ...ok, session_id: "--dangerous" }).command).toBeNull();
    expect(sshAttach({ ...ok, harness: "claude", job_id: "-x" }).command).toBeNull();
    expect(sshAttach({ ...ok, tmux: { session: "-t", window: "@1", pane: "%1", label: null } }).command).toBeNull();
    expect(sshAttach({ ...ok, tmux: { session: "s", window: "@1; id", pane: "%1", label: null } }).command).toBeNull();
    const r = sshAttach({ ...ok, session_id: "a\nb" });
    expect(r.command === null && r.reason).toMatch(/unsupported characters/);
  });

  // The command is run through a stub `ssh` that hands the remote command to `sh -c`, with stub
  // harness binaries that only record their arguments: nothing but the intended program may run.
  describe("round trip through sh", () => {
    let dir: string;
    let env: Record<string, string>;
    beforeAll(() => {
      dir = mkdtempSync(join(tmpdir(), "ssh-rt-"));
      const bin = join(dir, "bin");
      mkdirSync(bin);
      const stub = (name: string, body: string) => {
        writeFileSync(join(bin, name), `#!/bin/sh\n${body}\n`);
        chmodSync(join(bin, name), 0o755);
      };
      stub("ssh", 'shift; [ "$1" = "--" ] && shift; shift; exec sh -c "$*"');
      for (const n of ["claude", "codex", "opencode", "tmux"]) {
        stub(n, `{ printf '%s' "${n}"; for a in "$@"; do printf '|%s' "$a"; done; echo; } >> "$LOG"`);
      }
      // a login shell rebuilds PATH from the profile files
      writeFileSync(join(dir, ".profile"), `export PATH="${bin}:$PATH"\n`);
      env = { PATH: `${bin}:/usr/bin:/bin`, SHELL: "/bin/sh", LOG: join(dir, "log"), HOME: dir };
    });
    afterAll(() => rmSync(dir, { recursive: true, force: true }));

    const run = (command: string, cwd: string) => {
      writeFileSync(join(dir, "log"), "");
      try {
        execFileSync("sh", ["-c", command], { cwd, env, stdio: "pipe" });
      } catch (e) {
        throw new Error(`${command}\n${String((e as { stderr?: unknown }).stderr)}`);
      }
      return readFileSync(join(dir, "log"), "utf8").trim().split("\n");
    };
    const pwned = () => readdirSync(dir).filter((f: string) => f.startsWith("PWN"));
    const nasty = [`a'b`, `a"b`, "a;touch PWN1;", "$(touch PWN2)", "`touch PWN3`", "a b", "a|b", "a&&touch PWN4", "a\\b", "a*?[b]"];

    it("resume: the harness gets exactly the hostile id as one argument, in the hostile cwd", () => {
      for (const [i, v] of nasty.entries()) {
        const cwd = join(dir, `d${i} ${v.replace(/\//g, "_")}`);
        mkdirSync(cwd);
        for (const harness of ["claude", "codex", "opencode"]) {
          const r = sshAttach({ ...ok, harness, session_id: v, cwd });
          expect(r.command).not.toBeNull();
          const lines = run(r.command!, dir);
          expect(lines).toHaveLength(1);
          const expected = { claude: ["claude", "--resume", v], codex: ["codex", "resume", v], opencode: ["opencode", "--session", v] }[harness]!;
          expect(lines[0]).toBe(expected.join("|"));
        }
      }
      expect(pwned()).toEqual([]);
    });

    it("attach and tmux: hostile job and session names stay single arguments", () => {
      for (const v of nasty) {
        const job = sshAttach({ ...ok, job_id: v });
        expect(run(job.command!, dir)).toEqual([`claude|attach|${v}`]);
        const tm = sshAttach({ ...ok, tmux: { session: v, window: "@3", pane: "%4", label: null } });
        expect(run(tm.command!, dir)).toEqual([`tmux|attach|-t|=${v}|;|select-window|-t|@3|;|select-pane|-t|%4`]);
      }
      expect(pwned()).toEqual([]);
    });
  });
});
