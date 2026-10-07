import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import type { CommitDetail, GraphCommit, RefInfo, Repository, WipStatus } from "../../lib/types";

const graphLog = vi.fn();
const listRefs = vi.fn();
const worktreeStatus = vi.fn();
const listWorktrees = vi.fn();
const commitDetail = vi.fn();
const commitFileDiff = vi.fn();
vi.mock("../../lib/api", () => ({
  api: {
    graphLog: (a: unknown) => graphLog(a),
    listRefs: (id: number, wt: unknown) => listRefs(id, wt),
    worktreeStatus: (id: number, wt: unknown) => worktreeStatus(id, wt),
    listWorktrees: (id: number) => listWorktrees(id),
    commitDetail: (id: number, sha: string) => commitDetail(id, sha),
    commitFileDiff: (a: unknown) => commitFileDiff(a),
    listBranches: () => Promise.resolve([]),
  },
}));

const toastError = vi.fn();
vi.mock("../../lib/toast", () => ({
  toast: { error: (m: string) => toastError(m), success: vi.fn() },
}));

import { GraphView, GRAPH_PAGE_SIZE } from "./GraphView";

const repo: Repository = {
  id: 3,
  local_path: "/home/me/widget",
  remote_owner: "acme",
  remote_name: "widget",
  default_branch: "main",
  added_at: "2026-01-01",
};

const sha = (i: number) => `c${String(i).padStart(39, "0")}`;

function chain(from: number, count: number): GraphCommit[] {
  return Array.from({ length: count }, (_, k) => {
    const i = from + k;
    return {
      sha: sha(i),
      parents: [sha(i + 1)],
      subject: `subject ${i}`,
      body_preview: "",
      author_name: "dev",
      author_email: "dev@example.com",
      author_time: 1_700_000_000 - i,
      committer_time: 1_700_000_000 - i,
    };
  });
}

const cleanWip: WipStatus = { head_sha: sha(0), staged: [], unstaged: [], untracked: [] };

function detailFor(s: string): CommitDetail {
  return {
    sha: s,
    parents: [],
    subject: `detail of ${s.slice(0, 7)}`,
    body: "",
    author_name: "dev",
    author_email: "dev@example.com",
    author_time: 1_700_000_000,
    committer_name: "dev",
    committer_email: "dev@example.com",
    committer_time: 1_700_000_000,
    files: [{ path: "src/a.ts", old_path: null, status: "M", additions: 1, deletions: 1 }],
  };
}

function renderView() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<GraphView repo={repo} />, { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
  graphLog.mockResolvedValue(chain(0, 3));
  listRefs.mockResolvedValue([]);
  worktreeStatus.mockResolvedValue(cleanWip);
  listWorktrees.mockResolvedValue([]);
  commitDetail.mockImplementation((_id: number, s: string) => Promise.resolve(detailFor(s)));
  commitFileDiff.mockResolvedValue("");
});

describe("GraphView", () => {
  it("selects a commit, opens a file preview over the still-mounted graph, and closes it", async () => {
    const user = userEvent.setup();
    const { container } = renderView();

    await waitFor(() => expect(screen.getByText("subject 1")).toBeInTheDocument());
    await user.click(screen.getByText("subject 1"));
    await waitFor(() => expect(screen.getByText(`detail of ${sha(1).slice(0, 7)}`)).toBeInTheDocument());
    expect(commitDetail).toHaveBeenCalledWith(repo.id, sha(1));

    await user.click(screen.getByRole("button", { name: /src\/a\.ts/ }));
    expect(screen.getByRole("button", { name: "Close preview" })).toBeInTheDocument();
    const graphWrap = container.querySelector(".gv-graph") as HTMLElement;
    expect(graphWrap.style.display).toBe("none");
    expect(screen.getByTestId("commit-graph")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Close preview" }));
    expect(screen.queryByRole("button", { name: "Close preview" })).not.toBeInTheDocument();
    expect(graphWrap.style.display).toBe("");
  });

  it("requests graph pages with the selected scope", async () => {
    const user = userEvent.setup();
    renderView();
    await waitFor(() =>
      expect(graphLog).toHaveBeenCalledWith(
        expect.objectContaining({ scope: "local", skip: 0, limit: GRAPH_PAGE_SIZE, worktreePath: null }),
      ),
    );
    await user.selectOptions(screen.getByRole("combobox", { name: "Graph scope" }), "all");
    await waitFor(() =>
      expect(graphLog).toHaveBeenCalledWith(expect.objectContaining({ scope: "all", skip: 0 })),
    );
  });

  it("toasts immediately for a remote branch outside the All scope", async () => {
    const user = userEvent.setup();
    const remote: RefInfo = { name: "origin/feature", kind: "remote", sha: sha(99), is_head: false, updated_at: 0 };
    listRefs.mockResolvedValue([remote]);
    renderView();

    await waitFor(() => expect(screen.getByText("subject 0")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: /^Remote/, expanded: false }));
    await user.click(await screen.findByRole("button", { name: "Jump to origin/feature" }));
    expect(toastError).toHaveBeenCalledWith("Switch to All to see this branch");
    expect(graphLog).toHaveBeenCalledTimes(1);
  });

  it("pages forward to find a ref tip beyond the first page", async () => {
    const user = userEvent.setup();
    const target = sha(GRAPH_PAGE_SIZE + 5);
    graphLog.mockImplementation(({ skip }: { skip: number }) =>
      Promise.resolve(skip === 0 ? chain(0, GRAPH_PAGE_SIZE) : chain(GRAPH_PAGE_SIZE, 10)),
    );
    listRefs.mockResolvedValue([{ name: "old", kind: "local", sha: target, is_head: false, updated_at: 0 }]);
    renderView();

    await waitFor(() => expect(screen.getByText("subject 0")).toBeInTheDocument());
    await user.click(await screen.findByRole("button", { name: "Jump to old" }));

    await waitFor(() => expect(commitDetail).toHaveBeenCalledWith(repo.id, target));
    expect(graphLog).toHaveBeenCalledWith(expect.objectContaining({ skip: GRAPH_PAGE_SIZE }));
    expect(toastError).not.toHaveBeenCalled();
  });

  it("toasts when a ref tip is not in the exhausted graph", async () => {
    const user = userEvent.setup();
    listRefs.mockResolvedValue([{ name: "v9", kind: "tag", sha: sha(500), is_head: false, updated_at: 0 }]);
    renderView();
    await waitFor(() => expect(screen.getByText("subject 0")).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: /^Tags/, expanded: false }));
    await user.click(await screen.findByRole("button", { name: "Jump to v9" }));
    await waitFor(() => expect(toastError).toHaveBeenCalledWith("Switch to All to see this branch"));
  });

  it("opens branch actions from a graph ref pill right-click", async () => {
    listRefs.mockResolvedValue([{ name: "topic", kind: "local", sha: sha(1), is_head: false, updated_at: 0 }]);
    renderView();
    await waitFor(() => expect(screen.getByText("subject 1")).toBeInTheDocument());
    const pill = await waitFor(() => {
      const el = document.querySelector('[data-ref-kind="local"]');
      expect(el).not.toBeNull();
      return el as HTMLElement;
    });
    fireEvent.contextMenu(pill);
    expect(screen.getByRole("menu", { name: "Actions for topic" })).toBeInTheDocument();
  });

  it("shows the WIP row and lists its files when selected", async () => {
    const user = userEvent.setup();
    worktreeStatus.mockResolvedValue({
      ...cleanWip,
      unstaged: [{ path: "src/b.ts", old_path: null, status: "M", additions: null, deletions: null }],
      untracked: ["notes.txt"],
    });
    renderView();
    const wipRow = await screen.findByTestId("cg-wip-row");
    expect(wipRow).toHaveTextContent("// WIP +2");
    await user.click(wipRow);
    expect(screen.getByText("Uncommitted changes")).toBeInTheDocument();
    expect(screen.getByText("notes.txt")).toBeInTheDocument();
  });
});
