import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const createReview = vi.fn();
const listBranches = vi.fn();
const removeWorktree = vi.fn();
const pruneWorktrees = vi.fn();
const openInVscode = vi.fn();
vi.mock("../../lib/api", () => ({
  api: {
    createReview: (args: unknown) => createReview(args),
    listBranches: (id: number) => listBranches(id),
    removeWorktree: (id: number, path: string) => removeWorktree(id, path),
    pruneWorktrees: (id: number) => pruneWorktrees(id),
    openInVscode: (path: string) => openInVscode(path),
  },
}));

const confirmDialog = vi.fn();
vi.mock("../../lib/confirm", () => ({
  confirmDialog: (opts: unknown) => confirmDialog(opts),
}));

import { RepoSidebar } from "./RepoSidebar";
import { ReviewVsDialog } from "./ReviewVsDialog";
import { useSettingsStore } from "../../lib/settings";
import type { RefInfo, Repository, WorktreeInfo } from "../../lib/types";

const repo: Repository = {
  id: 7,
  local_path: "/home/me/widget",
  remote_owner: "acme",
  remote_name: "widget",
  default_branch: "main",
  added_at: "2026-01-01",
};

const refs: RefInfo[] = [
  { name: "main", kind: "local", sha: "a".repeat(40), is_head: true },
  { name: "feature/login", kind: "local", sha: "b".repeat(40), is_head: false },
  { name: "fix/crash", kind: "local", sha: "c".repeat(40), is_head: false },
  { name: "origin/HEAD", kind: "remote", sha: "a".repeat(40), is_head: false },
  { name: "origin/main", kind: "remote", sha: "a".repeat(40), is_head: false },
  { name: "origin/feature/login", kind: "remote", sha: "b".repeat(40), is_head: false },
  { name: "v1.0.0", kind: "tag", sha: "d".repeat(40), is_head: false },
];

const worktrees: WorktreeInfo[] = [
  {
    path: "/home/me/widget",
    display_path: "~/widget",
    head_sha: "a".repeat(40),
    branch: "main",
    is_main: true,
    is_prunable: false,
    prunable_reason: null,
    source: "manual",
  },
  {
    path: "/home/me/.claude/worktrees/widget/wt1",
    display_path: "~/.claude/worktrees/widget/wt1",
    head_sha: "b".repeat(40),
    branch: "feature/login",
    is_main: false,
    is_prunable: true,
    prunable_reason: "gone",
    source: "claude",
  },
];

function setup(overrides: Partial<Parameters<typeof RepoSidebar>[0]> = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const props = {
    repo,
    refs,
    worktrees,
    contextPath: null,
    onJumpToRef: vi.fn(),
    onSelectWorktree: vi.fn(),
    openReview: vi.fn(),
    ...overrides,
  };
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  render(<RepoSidebar {...props} />, { wrapper });
  return { props, invalidate, user: userEvent.setup() };
}

function section(id: string): HTMLElement {
  return document.querySelector(`[data-section="${id}"]`) as HTMLElement;
}

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState({ defaultThreeDot: true });
  createReview.mockResolvedValue({ id: 99 });
  listBranches.mockResolvedValue([
    { name: "main", is_remote: false, sha: "a" },
    { name: "develop", is_remote: false, sha: "e" },
    { name: "feature/login", is_remote: false, sha: "b" },
  ]);
  removeWorktree.mockResolvedValue(undefined);
  pruneWorktrees.mockResolvedValue(undefined);
  openInVscode.mockResolvedValue(undefined);
  confirmDialog.mockResolvedValue(true);
});

describe("RepoSidebar sections", () => {
  it("shows a count per section, excluding remote HEAD aliases", () => {
    setup();
    expect(screen.getByTestId("count-local")).toHaveTextContent("3");
    expect(screen.getByTestId("count-remote")).toHaveTextContent("2");
    expect(screen.getByTestId("count-worktrees")).toHaveTextContent("2");
    expect(screen.getByTestId("count-tags")).toHaveTextContent("1");
    expect(screen.queryByText("origin/HEAD")).not.toBeInTheDocument();
  });

  it("filters rows within a section and shows shown/total", async () => {
    const { user } = setup();
    await user.type(screen.getByLabelText("Filter local"), "LOGIN");
    const local = section("local");
    expect(within(local).getByText("feature/login")).toBeInTheDocument();
    expect(within(local).queryByText("fix/crash")).not.toBeInTheDocument();
    expect(screen.getByTestId("count-local")).toHaveTextContent("1/3");
    expect(within(section("remote")).getByText("origin/feature/login")).toBeInTheDocument();

    await user.clear(screen.getByLabelText("Filter local"));
    await user.type(screen.getByLabelText("Filter local"), "zzz");
    expect(within(local).getByText("No matches")).toBeInTheDocument();
  });

  it("collapses and expands a section", async () => {
    const { user } = setup();
    const toggle = within(section("tags")).getByRole("button", { name: /tags/i });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("v1.0.0")).toBeInTheDocument();

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("v1.0.0")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Filter tags")).not.toBeInTheDocument();

    await user.click(toggle);
    expect(screen.getByText("v1.0.0")).toBeInTheDocument();
  });

  it("clicking a ref row jumps to it", async () => {
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: /Jump to origin\/main/ }));
    expect(props.onJumpToRef).toHaveBeenCalledWith(refs[4]);
    await user.click(screen.getByRole("button", { name: /Jump to v1.0.0/ }));
    expect(props.onJumpToRef).toHaveBeenCalledWith(refs[6]);
  });
});

describe("RepoSidebar worktrees", () => {
  it("clicking a worktree row selects it as the context", async () => {
    const { user, props } = setup();
    await user.click(screen.getByText("~/.claude/worktrees/widget/wt1"));
    expect(props.onSelectWorktree).toHaveBeenCalledWith("/home/me/.claude/worktrees/widget/wt1");
  });

  it("highlights the main worktree when context is null, else the matching one", () => {
    setup({ contextPath: "/home/me/.claude/worktrees/widget/wt1" });
    const rows = within(section("worktrees")).getAllByRole("button", { pressed: true });
    expect(rows).toHaveLength(1);
    expect(rows[0]).toHaveTextContent("~/.claude/worktrees/widget/wt1");
  });

  it("removes a worktree after confirming and invalidates the worktree list", async () => {
    const onWorktreeRemoved = vi.fn();
    const { user, invalidate, props } = setup({ onWorktreeRemoved });
    await user.click(screen.getByRole("button", { name: /Remove worktree ~\/.claude/ }));
    await waitFor(() =>
      expect(removeWorktree).toHaveBeenCalledWith(7, "/home/me/.claude/worktrees/widget/wt1"),
    );
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ["worktrees", 7] }),
    );
    expect(onWorktreeRemoved).toHaveBeenCalledWith("/home/me/.claude/worktrees/widget/wt1");
    expect(props.onSelectWorktree).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Remove worktree ~\/widget$/ })).toBeNull();
  });

  it("does not remove when the confirm is cancelled", async () => {
    confirmDialog.mockResolvedValue(false);
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: /Remove worktree ~\/.claude/ }));
    await waitFor(() => expect(confirmDialog).toHaveBeenCalled());
    expect(removeWorktree).not.toHaveBeenCalled();
  });

  it("prunes stale worktrees and opens in VS Code without selecting", async () => {
    const { user, invalidate, props } = setup();
    await user.click(screen.getByRole("button", { name: "Prune stale" }));
    await waitFor(() => expect(pruneWorktrees).toHaveBeenCalledWith(7));
    await waitFor(() =>
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ["worktrees", 7] }),
    );

    await user.click(screen.getByRole("button", { name: "Open ~/widget in VSCode" }));
    expect(openInVscode).toHaveBeenCalledWith("/home/me/widget");
    expect(props.onSelectWorktree).not.toHaveBeenCalled();
  });
});

describe("Branch actions", () => {
  it("'Review vs default' creates a three-dot review and opens it", async () => {
    const { user, props, invalidate } = setup();
    await user.click(screen.getByRole("button", { name: "Actions for feature/login" }));
    const menu = screen.getByRole("menu", { name: "Actions for feature/login" });
    await user.click(within(menu).getByRole("menuitem", { name: /Review vs main/ }));

    await waitFor(() =>
      expect(createReview).toHaveBeenCalledWith({
        repoId: 7,
        baseRef: "main",
        headRef: "feature/login",
        threeDot: true,
      }),
    );
    await waitFor(() => expect(props.openReview).toHaveBeenCalledWith(99));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["reviews", 7] });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("opens the menu on right-click and disables 'Review vs default' on the default branch", async () => {
    const { user } = setup();
    const row = screen.getByRole("button", { name: /Jump to main/ });
    await user.pointer({ keys: "[MouseRight]", target: row });
    const menu = screen.getByRole("menu", { name: "Actions for main" });
    expect(within(menu).getByRole("menuitem", { name: /Review vs main/ })).toBeDisabled();
  });

  it("has no actions button for tags", () => {
    setup();
    expect(screen.queryByRole("button", { name: "Actions for v1.0.0" })).not.toBeInTheDocument();
  });

  it("'Review vs…' opens the dialog and creates a review with the chosen base", async () => {
    useSettingsStore.setState({ defaultThreeDot: false });
    const { user, props } = setup();
    await user.click(screen.getByRole("button", { name: "Actions for feature/login" }));
    await user.click(screen.getByRole("menuitem", { name: "Review vs…" }));

    const dialog = screen.getByRole("dialog", { name: "Review vs" });
    const select = within(dialog).getByLabelText("Base branch");
    await waitFor(() => expect(select).toHaveValue("main"));
    expect(within(select).queryByRole("option", { name: "feature/login" })).toBeNull();
    expect(within(dialog).getByRole("checkbox")).not.toBeChecked();

    await user.selectOptions(select, "develop");
    await user.click(within(dialog).getByRole("button", { name: "Start review" }));
    await waitFor(() =>
      expect(createReview).toHaveBeenCalledWith({
        repoId: 7,
        baseRef: "develop",
        headRef: "feature/login",
        threeDot: false,
      }),
    );
    await waitFor(() => expect(props.openReview).toHaveBeenCalledWith(99));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

describe("ReviewVsDialog", () => {
  it("defaults merge-base to the setting and cancels without creating", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const onClose = vi.fn();
    render(
      <QueryClientProvider client={client}>
        <ReviewVsDialog repo={repo} head="feature/login" onClose={onClose} />
      </QueryClientProvider>,
    );
    const user = userEvent.setup();
    expect(screen.getByRole("checkbox")).toBeChecked();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onClose).toHaveBeenCalled();
    expect(createReview).not.toHaveBeenCalled();
  });
});
