import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ClaudeSession } from "../lib/types";

const listClaudeSessions = vi.fn();
const boardCardSessions = vi.fn();
const focusClaudeSession = vi.fn();
const resumeClaudeSession = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    listClaudeSessions: (d: number) => listClaudeSessions(d),
    boardCardSessions: () => boardCardSessions(),
    focusClaudeSession: (id: string) => focusClaudeSession(id),
    resumeClaudeSession: (...args: unknown[]) => resumeClaudeSession(...args),
  },
}));

import { SessionsView, resumeCommand, tildePath } from "./SessionsView";
import { useUIStore } from "../store";

const session = (over: Partial<ClaudeSession> = {}): ClaudeSession => ({
  session_id: "s1",
  pid: 1,
  live: true,
  status: "idle",
  kind: "interactive",
  attached: false,
  job_id: null,
  waiting_for: null,
  agent_view: null,
  name: "live one",
  cwd: "/Users/me/projects/app",
  git_branch: "feat/x",
  repo: "org/app",
  tmux: { session: "0", window: "@1", pane: "%1", label: "0:2" },
  tty: null,
  updated_at: Date.now(),
  focus: "ready",
  ...over,
});

function renderView() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <SessionsView />
    </QueryClientProvider>,
  );
}

describe("SessionsView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    focusClaudeSession.mockResolvedValue(undefined);
    resumeClaudeSession.mockResolvedValue(undefined);
    boardCardSessions.mockResolvedValue([{ card_id: "c1", repo: "org/app", number: 42, session_id: "s1" }]);
    listClaudeSessions.mockResolvedValue([
      session(),
      session({ session_id: "s2", name: "detached one", focus: "detached" }),
      session({ session_id: "bg123456789", name: "bg one", kind: "bg", job_id: "job12345", tmux: null, focus: "background" }),
      session({ session_id: "w1", name: "waiting one", waiting_for: "input needed" }),
      session({
        session_id: "av1",
        name: "via view",
        kind: "bg",
        job_id: "view1234",
        tmux: null,
        focus: "background",
        agent_view: { pid: 9, cwd: "/Users/me/projects", filter_cwd: null, tmux: { session: "0", window: "@1", pane: "%0", label: "0:1" }, tty: "/dev/ttys003", focus: "ready" },
      }),
      session({ session_id: "s3", name: "old one", live: false, pid: null, tmux: null, focus: "not_live" }),
    ]);
  });

  it("lists live and history sessions, focuses ready ones, disables detached", async () => {
    renderView();
    await screen.findByText("live one");
    expect(screen.getByText("old one")).toBeInTheDocument();
    expect(screen.getAllByText("~/projects/app", { exact: false })).toHaveLength(6);

    const focusButtons = screen.getAllByRole("button", { name: /Focus/ });
    expect(focusButtons).toHaveLength(3);
    expect(focusButtons[1]).toBeDisabled();
    await userEvent.click(focusButtons[0]);
    await waitFor(() => expect(focusClaudeSession).toHaveBeenCalledWith("s1"));
    expect(screen.getByRole("button", { name: "Copy resume command" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /Copy attach/ })[0]).toHaveAttribute(
      "title",
      expect.stringContaining("claude attach job12345"),
    );
    expect(screen.getByText("input needed")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /Copy attach/ })).toHaveLength(2);
    await userEvent.click(screen.getByRole("button", { name: /Agent view/ }));
    await waitFor(() => expect(focusClaudeSession).toHaveBeenCalledWith("av1"));
    expect(screen.getAllByRole("button", { name: "Stop" })[0]).toHaveAttribute("title", "claude stop job12345");
  });

  it("resumes a history session in a new terminal", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("button", { name: /Resume/ }));
    await waitFor(() =>
      expect(resumeClaudeSession).toHaveBeenCalledWith("s3", "/Users/me/projects/app", "org/app", "iterm2", "current", "codereview"),
    );
  });

  it("card chip jumps to the board card", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("button", { name: "app#42" }));
    const s = useUIStore.getState();
    expect(s.homeSection).toBe("board");
    expect(s.boardHighlightCardId).toBe("c1");
  });
});

describe("helpers", () => {
  it("abbreviates home dirs", () => {
    expect(tildePath("/Users/me/projects")).toBe("~/projects");
    expect(tildePath("/home/me")).toBe("~");
    expect(tildePath("/opt/x")).toBe("/opt/x");
  });

  it("quotes cwd in resume command", () => {
    expect(resumeCommand({ cwd: "/a/b", session_id: "id" })).toBe("cd /a/b; claude --resume id");
    expect(resumeCommand({ cwd: "/a b/it's", session_id: "id" })).toBe(`cd '/a b/it'\\''s'; claude --resume id`);
  });
});
