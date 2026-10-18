import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const api = vi.hoisted(() => ({
  gitCheckoutBranch: vi.fn(),
  gitDeleteBranch: vi.fn(),
  gitPush: vi.fn(),
  gitCreateTag: vi.fn(),
  gitCommit: vi.fn(),
  gitStage: vi.fn(),
  gitUnstage: vi.fn(),
  listClaudeSessions: vi.fn(),
  focusClaudeSession: vi.fn(),
}));
vi.mock("../../lib/api", () => ({ api }));

const confirmDialog = vi.fn();
vi.mock("../../lib/confirm", () => ({ confirmDialog: (o: unknown) => confirmDialog(o) }));

const toastError = vi.fn();
vi.mock("../../lib/toast", () => ({ toast: { success: vi.fn(), error: (m: string) => toastError(m) } }));

import { GitOpsProvider, parseRefusal, worktreeFromRefusal } from "./GitOps";
import { BranchActionsMenu } from "./BranchActionsMenu";
import { CommitPanel } from "./CommitPanel";
import type { RefInfo, Repository, WipStatus, WorktreeInfo } from "../../lib/types";

const repo: Repository = {
  id: 7,
  local_path: "/home/me/widget",
  remote_owner: "acme",
  remote_name: "widget",
  default_branch: "main",
  added_at: "2026-01-01",
  friendly_name: null,
  registered_at: "2026-01-01",
};

const local = (name: string, is_head = false): RefInfo => ({
  name,
  kind: "local",
  sha: "a".repeat(40),
  is_head,
  updated_at: 0,
});

const worktree = (path: string, branch: string | null, is_main = false): WorktreeInfo => ({
  path,
  display_path: path,
  head_sha: "a".repeat(40),
  branch,
  is_main,
  is_prunable: false,
  prunable_reason: null,
  is_locked: false,
  source: "manual",
});

const worktrees = [worktree("/home/me/widget", "main", true), worktree("/wt/feat", "feat")];

function renderWithOps(ui: React.ReactNode, onSwitchContext = vi.fn()) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={qc}>
      <GitOpsProvider
        repo={repo}
        ctx={null}
        refs={[local("main", true), local("feat"), local("topic")]}
        worktrees={worktrees}
        onSwitchContext={onSwitchContext}
      >
        {ui}
      </GitOpsProvider>
    </QueryClientProvider>,
  );
  return { onSwitchContext };
}

beforeEach(() => {
  Object.values(api).forEach((f) => f.mockReset());
  confirmDialog.mockReset();
  toastError.mockReset();
  api.listClaudeSessions.mockResolvedValue({ sessions: [], card_sessions: [] });
});

describe("refusal parsing", () => {
  it("extracts kind and message from the tagged git error", () => {
    expect(parseRefusal("git error: [not_fully_merged] error: The branch 'x' is not fully merged.")).toEqual({
      kind: "not_fully_merged",
      message: "error: The branch 'x' is not fully merged.",
    });
    expect(parseRefusal("git error: plain failure")).toBeNull();
  });

  it("reads the holding worktree from git's message", () => {
    expect(worktreeFromRefusal("fatal: 'feat' is already used by worktree at '/wt/feat'")).toBe("/wt/feat");
    expect(worktreeFromRefusal("fatal: 'feat' is already checked out at '/wt/feat'")).toBe("/wt/feat");
    expect(worktreeFromRefusal("nope")).toBeNull();
  });
});

describe("branch menu git actions", () => {
  const open = (ref: RefInfo) =>
    renderWithOps(<BranchActionsMenu repo={repo} branch={ref.name} refInfo={ref} anchor={{ x: 0, y: 0 }} onClose={vi.fn()} />);

  it("disables checkout and delete on the checked-out branch", () => {
    open(local("main", true));
    expect(screen.getByRole("menuitem", { name: "Checkout" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "Delete" })).toBeDisabled();
  });

  it("enables terminal and Claude only for branches checked out in a worktree", () => {
    open(local("topic"));
    expect(screen.getByRole("menuitem", { name: "Open terminal" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "New Claude session" })).toBeDisabled();
  });

  it("offers terminal and Claude for a branch held by a worktree", () => {
    open(local("feat"));
    expect(screen.getByRole("menuitem", { name: "Open terminal" })).toBeEnabled();
    expect(screen.getByRole("menuitem", { name: "New Claude session" })).toBeEnabled();
  });

  it("lists live Claude sessions in that worktree to focus", async () => {
    api.listClaudeSessions.mockResolvedValue({
      sessions: [{ session_id: "abcdef123456", live: true, cwd: "/wt/feat", name: "fixing it" }],
      card_sessions: [],
    });
    api.focusClaudeSession.mockResolvedValue(undefined);
    open(local("feat"));
    await userEvent.click(await screen.findByRole("menuitem", { name: /Focus Claude/ }));
    expect(api.focusClaudeSession).toHaveBeenCalledWith("abcdef123456");
  });

  it("gives tags detached checkout and delete but no review items", () => {
    open({ name: "v1", kind: "tag", sha: "d".repeat(40), is_head: false, updated_at: 0 });
    expect(screen.getByRole("menuitem", { name: "Checkout (detached)" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Delete tag" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: /Review vs/ })).toBeNull();
  });

  it("escalates a not-fully-merged delete to a confirmed force delete", async () => {
    api.gitDeleteBranch
      .mockRejectedValueOnce("git error: [not_fully_merged] error: The branch 'topic' is not fully merged.")
      .mockResolvedValueOnce(undefined);
    confirmDialog.mockResolvedValue(true);
    open(local("topic"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    await waitFor(() => expect(api.gitDeleteBranch).toHaveBeenCalledTimes(2));
    expect(api.gitDeleteBranch).toHaveBeenNthCalledWith(1, 7, null, "topic", false);
    expect(api.gitDeleteBranch).toHaveBeenNthCalledWith(2, 7, null, "topic", true);
    expect(toastError).not.toHaveBeenCalled();
  });

  it("does not force delete when the user declines", async () => {
    api.gitDeleteBranch.mockRejectedValue("git error: [not_fully_merged] not fully merged");
    confirmDialog.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    open(local("topic"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(2));
    expect(api.gitDeleteBranch).toHaveBeenCalledTimes(1);
    expect(toastError).not.toHaveBeenCalled();
  });

  it("offers to switch worktree when git says the branch is checked out elsewhere", async () => {
    api.gitCheckoutBranch.mockRejectedValue(
      "git error: [checked_out_elsewhere] fatal: 'feat' is already used by worktree at '/wt/feat'",
    );
    confirmDialog.mockResolvedValue(true);
    const { onSwitchContext } = open(local("feat"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Checkout" }));
    await waitFor(() => expect(onSwitchContext).toHaveBeenCalledWith("/wt/feat"));
  });

  it("shows other refusals as an error toast", async () => {
    api.gitCheckoutBranch.mockRejectedValue("git error: [dirty_tree] local changes would be overwritten");
    open(local("topic"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Checkout" }));
    await waitFor(() => expect(toastError).toHaveBeenCalledWith(expect.stringContaining("local changes would be overwritten")));
  });

  it("creates an annotated tag and pushes it from the dialog", async () => {
    api.gitCreateTag.mockResolvedValue(undefined);
    open(local("topic"));
    await userEvent.click(screen.getByRole("menuitem", { name: "Create tag here…" }));
    await userEvent.type(screen.getByLabelText("name"), "v2");
    await userEvent.type(screen.getByLabelText("message (annotated tag if set)"), "release");
    await userEvent.click(screen.getByLabelText("Push to origin"));
    await userEvent.click(screen.getByRole("button", { name: "Create tag" }));
    await waitFor(() =>
      expect(api.gitCreateTag).toHaveBeenCalledWith(7, null, "v2", "a".repeat(40), "release", true),
    );
  });
});

describe("WIP commit controls", () => {
  const wip = (over: Partial<WipStatus> = {}): WipStatus => ({
    head_sha: "a".repeat(40),
    staged: [],
    unstaged: [{ path: "a.ts", old_path: null, status: "M", additions: null, deletions: null }],
    untracked: [],
    ...over,
  });
  const panel = (w: WipStatus) => renderWithOps(<CommitPanel wip={w} onOpenWipFile={vi.fn()} />);

  it("renders no controls without the git ops context", () => {
    render(<CommitPanel wip={wip()} onOpenWipFile={vi.fn()} />);
    expect(screen.queryByLabelText("Commit message")).toBeNull();
    expect(screen.queryByRole("button", { name: /Stage a.ts/ })).toBeNull();
  });

  it("keeps Commit disabled until something is staged and a message is typed", async () => {
    panel(wip());
    const commit = screen.getByRole("button", { name: "Commit" });
    await userEvent.type(screen.getByLabelText("Commit message"), "msg");
    expect(commit).toBeDisabled();
  });

  it("commits staged changes and clears the message", async () => {
    api.gitCommit.mockResolvedValue(undefined);
    panel(
      wip({
        staged: [{ path: "s.ts", old_path: null, status: "M", additions: null, deletions: null }],
      }),
    );
    await userEvent.type(screen.getByLabelText("Commit message"), "feat: x");
    await userEvent.click(screen.getByRole("button", { name: "Commit" }));
    await waitFor(() => expect(api.gitCommit).toHaveBeenCalledWith(7, null, "feat: x", false));
    await waitFor(() => expect(screen.getByLabelText("Commit message")).toHaveValue(""));
  });

  it("allows amending with nothing staged and an empty message", async () => {
    api.gitCommit.mockResolvedValue(undefined);
    panel(wip());
    await userEvent.click(screen.getByLabelText("Amend"));
    await userEvent.click(screen.getByRole("button", { name: "Amend commit" }));
    await waitFor(() => expect(api.gitCommit).toHaveBeenCalledWith(7, null, "", true));
  });

  it("stages and unstages single files", async () => {
    api.gitStage.mockResolvedValue(undefined);
    api.gitUnstage.mockResolvedValue(undefined);
    panel(
      wip({
        staged: [{ path: "s.ts", old_path: null, status: "M", additions: null, deletions: null }],
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Stage a.ts" }));
    expect(api.gitStage).toHaveBeenCalledWith(7, null, ["a.ts"]);
    await userEvent.click(screen.getByRole("button", { name: "Unstage s.ts" }));
    expect(api.gitUnstage).toHaveBeenCalledWith(7, null, ["s.ts"]);
  });
});
