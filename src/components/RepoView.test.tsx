import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Mock the API layer used by RepoView (PR list, graph tab + the surrounding queries).
const listReviews = vi.fn();
const listBranches = vi.fn();
const ghAuthStatus = vi.fn();
const listPrs = vi.fn();
const createReviewForPr = vi.fn();
const graphLog = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    listReviews: (id: number | null) => listReviews(id),
    listBranches: (id: number) => listBranches(id),
    graphLog: (args: unknown) => graphLog(args),
    listRefs: () => Promise.resolve([]),
    worktreeStatus: () =>
      Promise.resolve({ head_sha: "a".repeat(40), staged: [], unstaged: [], untracked: [] }),
    listWorktrees: () => Promise.resolve([]),
    ghAuthStatus: () => ghAuthStatus(),
    listPrs: (id: number) => listPrs(id),
    createReviewForPr: (o: string, n: string, num: number, paths: string[]) => createReviewForPr(o, n, num, paths),
  },
}));

import { RepoView } from "./RepoView";
import { useSettingsStore } from "../lib/settings";
import type { PrSummary, Repository } from "../lib/types";

const repo: Repository = {
  id: 1,
  local_path: "/home/me/projects/widget",
  remote_owner: "acme",
  remote_name: "widget",
  default_branch: "main",
  added_at: "2026-01-01",
};

const pr: PrSummary = {
  number: 42,
  title: "Fix anchor drift",
  author: { login: "alice" },
  headRefName: "fix/drift",
  baseRefName: "main",
  createdAt: "2026-06-01T00:00:00Z",
  url: "https://github.com/acme/widget/pull/42",
};

function renderRepo(r: Repository = repo) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<RepoView repo={r} />, { wrapper });
}

async function openPrTab(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "GitHub PRs" }));
}

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState({ prListPollMs: 0 });
  listReviews.mockResolvedValue([]);
  listBranches.mockResolvedValue([]);
  ghAuthStatus.mockResolvedValue(true);
  listPrs.mockResolvedValue([pr]);
  graphLog.mockResolvedValue([]);
});

describe("RepoView tabs", () => {
  it("shows the Graph, GitHub PRs and Reviews tabs, defaulting to Graph", async () => {
    renderRepo();
    const tabs = screen.getAllByRole("button", { name: /^(Graph|GitHub PRs|Reviews|Virtual PR|Worktrees)$/ });
    expect(tabs.map((t) => t.textContent)).toEqual(["Graph", "GitHub PRs", "Reviews"]);
    expect(screen.getByRole("button", { name: "Graph" })).toHaveClass("active");
    await waitFor(() => expect(graphLog).toHaveBeenCalled());
    expect(screen.queryByText("No reviews yet.")).not.toBeInTheDocument();
  });

  it("shows the reviews list only on the Reviews tab", async () => {
    const user = userEvent.setup();
    renderRepo();
    await openPrTab(user);
    expect(screen.queryByText("No reviews yet.")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reviews" }));
    await waitFor(() => expect(screen.getByText("No reviews yet.")).toBeInTheDocument());
  });

  it("disables the Graph tab without a local clone and defaults to GitHub PRs", async () => {
    renderRepo({ ...repo, local_path: null });
    const graphTab = screen.getByRole("button", { name: "Graph" });
    expect(graphTab).toBeDisabled();
    expect(graphTab).toHaveAttribute("title", "Graph requires a local clone");
    expect(screen.getByRole("button", { name: "GitHub PRs" })).toHaveClass("active");
    await waitFor(() => expect(screen.getByText(/#42 Fix anchor drift/)).toBeInTheDocument());
    expect(graphLog).not.toHaveBeenCalled();
  });
});

describe("RepoView PR list", () => {
  it("manual refresh refetches the PR list", async () => {
    const user = userEvent.setup();
    renderRepo();
    await openPrTab(user);

    await waitFor(() => expect(screen.getByText(/#42 Fix anchor drift/)).toBeInTheDocument());
    const before = listPrs.mock.calls.length;

    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(listPrs).toHaveBeenCalledTimes(before + 1));
  });

  it("staleness label appears after first load", async () => {
    const user = userEvent.setup();
    renderRepo();
    await openPrTab(user);

    await waitFor(() => expect(screen.getByText(/#42 Fix anchor drift/)).toBeInTheDocument());
    expect(screen.getByText(/updated just now/)).toBeInTheDocument();
  });

  it("interval select writes the setting", async () => {
    const user = userEvent.setup();
    renderRepo();
    await openPrTab(user);

    await waitFor(() => expect(screen.getByText(/#42 Fix anchor drift/)).toBeInTheDocument());
    await user.selectOptions(screen.getByRole("combobox"), "30s");
    expect(useSettingsStore.getState().prListPollMs).toBe(30000);
  });

  it("toolbar still renders when the list is empty", async () => {
    const user = userEvent.setup();
    listPrs.mockResolvedValue([]);
    renderRepo();
    await openPrTab(user);

    await waitFor(() =>
      expect(screen.getByText("No open pull requests.")).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
  });
});
