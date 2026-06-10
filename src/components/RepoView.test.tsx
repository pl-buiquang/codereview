import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Mock the API layer used by RepoView (PR list + the surrounding queries).
const listReviews = vi.fn();
const listBranches = vi.fn();
const ghAuthStatus = vi.fn();
const listPrs = vi.fn();
const createReviewForPr = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    listReviews: (id: number | null) => listReviews(id),
    listBranches: (p: string) => listBranches(p),
    ghAuthStatus: () => ghAuthStatus(),
    listPrs: (p: string) => listPrs(p),
    createReviewForPr: (o: string, n: string, num: number) => createReviewForPr(o, n, num),
  },
}));

import { RepoView } from "./RepoView";
import { useSettingsStore } from "../lib/settings";
import type { PrSummary, Repository } from "../lib/types";

const repo: Repository = {
  id: 1,
  path: "/home/me/projects/widget",
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

function renderRepo() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<RepoView repo={repo} />, { wrapper });
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
});

describe("RepoView PR list", () => {
  it("manual refresh refetches the PR list", async () => {
    const user = userEvent.setup();
    renderRepo();
    await openPrTab(user);

    await waitFor(() => expect(screen.getByText(/#42 Fix anchor drift/)).toBeInTheDocument());
    expect(listPrs).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: /Refresh/ }));
    await waitFor(() => expect(listPrs).toHaveBeenCalledTimes(2));
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
    expect(screen.getByRole("button", { name: /Refresh/ })).toBeInTheDocument();
  });
});
