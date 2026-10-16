import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { WorktreeGcReport, WorktreeRow } from "../lib/types";

const worktreeGcLast = vi.fn();
const worktreeGcRun = vi.fn();
const worktreeSync = vi.fn();
const removeWorktree = vi.fn();
const worktreeOpenTerminal = vi.fn();
const worktreeGcReload = vi.fn();
const listRepositories = vi.fn();
const watchAcquire = vi.fn();
const watchRelease = vi.fn();
const gitChangedHandlers: (() => void)[] = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_event: string, cb: () => void) => {
    gitChangedHandlers.push(cb);
    return Promise.resolve(() => {});
  },
}));
vi.mock("../lib/api", () => ({
  api: {
    worktreeGcLast: () => worktreeGcLast(),
    worktreeGcReload: () => worktreeGcReload(),
    listRepositories: () => listRepositories(),
    watchAcquire: (id: number) => watchAcquire(id),
    watchRelease: (id: number) => watchRelease(id),
    worktreeGcRun: (...a: unknown[]) => worktreeGcRun(...a),
    worktreeSync: (...a: unknown[]) => worktreeSync(...a),
    removeWorktree: (...a: unknown[]) => removeWorktree(...a),
    worktreeOpenTerminal: (...a: unknown[]) => worktreeOpenTerminal(...a),
    openInVscode: vi.fn(),
  },
}));
vi.mock("../lib/confirm", () => ({ confirmDialog: () => Promise.resolve(true) }));

import { WorktreesView } from "./WorktreesView";

const row = (over: Partial<WorktreeRow>): WorktreeRow => ({
  path: "/wt/a",
  display_path: "~/wt/a",
  branch: "pbq/a",
  head_sha: "abcdef1234",
  source: "claude",
  is_main: false,
  pr: null,
  upstream: { kind: "tracking", ahead: 0, behind: 0, head_was_pushed: false },
  last_commit_at: null,
  verdict: { kind: "keep", reason: "PR #1 open" },
  ...over,
});

const report: WorktreeGcReport = {
  ran_at: new Date().toISOString(),
  actions: [],
  repos: [
    {
      repo_id: 3,
      path: "/repo",
      slug: "org/app",
      error: null,
      worktrees: [
        row({ path: "/repo", display_path: "~/repo", is_main: true }),
        row({
          path: "/wt/merged",
          display_path: "~/wt/merged",
          pr: { number: 9, state: "MERGED", headRefOid: "x", headRefName: "pbq/a" },
          verdict: { kind: "remove", reason: "PR #9 merged", delete_branch: true },
        }),
        row({
          path: "/wt/behind",
          display_path: "~/wt/behind",
          upstream: { kind: "tracking", ahead: 0, behind: 3, head_was_pushed: false },
          verdict: { kind: "fast_forward", reason: "3 behind origin/pbq/a" },
        }),
        row({ path: "/wt/dirty", display_path: "~/wt/dirty", verdict: { kind: "flag", reason: "uncommitted or untracked changes" } }),
      ],
    },
  ],
};

function renderView() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <WorktreesView />
    </QueryClientProvider>,
  );
}

describe("WorktreesView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    worktreeGcLast.mockResolvedValue(report);
    worktreeGcRun.mockResolvedValue(report);
    worktreeSync.mockResolvedValue("fast-forwarded");
    worktreeGcReload.mockResolvedValue(null);
    listRepositories.mockResolvedValue([]);
    watchAcquire.mockResolvedValue(true);
    gitChangedHandlers.length = 0;
  });

  it("watches every clone and reloads changed repos on open and on git events", async () => {
    worktreeGcLast.mockResolvedValue(null);
    listRepositories.mockResolvedValue([
      { id: 3, local_path: "/repo" },
      { id: 4, local_path: null },
    ]);
    worktreeGcReload.mockResolvedValue(report);
    const view = renderView();
    expect(await screen.findByText("~/wt/merged")).toBeInTheDocument();
    expect(watchAcquire).toHaveBeenCalledTimes(1);
    expect(watchAcquire).toHaveBeenCalledWith(3);
    expect(worktreeGcReload).toHaveBeenCalledTimes(1);
    expect(worktreeGcRun).not.toHaveBeenCalled();

    gitChangedHandlers.forEach((h) => h());
    gitChangedHandlers.forEach((h) => h());
    await waitFor(() => expect(worktreeGcReload).toHaveBeenCalledTimes(2), { timeout: 3000 });

    view.unmount();
    expect(watchRelease).toHaveBeenCalledWith(3);
  });

  it("lists linked worktrees with verdicts, hiding the main checkout", async () => {
    renderView();
    expect(await screen.findByText("~/wt/merged")).toBeInTheDocument();
    expect(screen.getByText("safe to remove")).toBeInTheDocument();
    expect(screen.getByText("needs a look")).toBeInTheDocument();
    expect(screen.getByText("#9 merged")).toBeInTheDocument();
    expect(screen.getByText("↓3")).toBeInTheDocument();
    expect(screen.queryByText("~/repo")).not.toBeInTheDocument();
    expect(screen.getByText(/3 worktrees · 1 need a look · 2 actionable/)).toBeInTheDocument();
  });

  it("does not scan on its own; Scan runs a dry run", async () => {
    worktreeGcLast.mockResolvedValue(null);
    renderView();
    expect(await screen.findByText(/press Scan/)).toBeInTheDocument();
    expect(worktreeGcRun).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: /Scan/ }));
    await waitFor(() => expect(worktreeGcRun).toHaveBeenCalledWith(false, false));
    expect(await screen.findByText("~/wt/behind")).toBeInTheDocument();
  });

  it("drops a removed worktree without rescanning", async () => {
    removeWorktree.mockResolvedValue(undefined);
    renderView();
    await screen.findByText("~/wt/dirty");
    const row = screen.getByText("~/wt/dirty").closest(".worktrees-row") as HTMLElement;
    await userEvent.click(row.querySelector('button[title="Remove worktree"]') as HTMLElement);
    await waitFor(() => expect(screen.queryByText("~/wt/dirty")).not.toBeInTheDocument());
    expect(removeWorktree).toHaveBeenCalledWith(3, "/wt/dirty");
    expect(worktreeGcRun).not.toHaveBeenCalled();
  });

  it("runs a forced cleanup that applies removals and syncs", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("button", { name: /Clean up/ }));
    await waitFor(() => expect(worktreeGcRun).toHaveBeenCalledWith(true, true));
  });

  it("syncs a behind worktree", async () => {
    renderView();
    await userEvent.click(await screen.findByRole("button", { name: "Sync" }));
    await waitFor(() => expect(worktreeSync).toHaveBeenCalledWith(3, "/wt/behind"));
  });

  it("opens a terminal in the worktree with the configured host", async () => {
    worktreeOpenTerminal.mockResolvedValue(undefined);
    renderView();
    await screen.findByText("~/wt/dirty");
    const row = screen.getByText("~/wt/dirty").closest(".worktrees-row") as HTMLElement;
    await userEvent.click(row.querySelector('button[title="Open a terminal here"]') as HTMLElement);
    await waitFor(() =>
      expect(worktreeOpenTerminal).toHaveBeenCalledWith(3, "/wt/dirty", "iterm2", "current", "codereview"),
    );
  });
});
