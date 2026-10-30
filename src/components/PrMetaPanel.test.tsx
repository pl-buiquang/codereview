import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const prMeta = vi.fn();
const openUrl = vi.fn();
const setPrLabel = vi.fn();
const setPrReviewer = vi.fn();
const listReviews = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    prMeta: (owner: string, name: string, number: number) => prMeta(owner, name, number),
    openUrl: (url: string) => openUrl(url),
    setPrLabel: (...args: unknown[]) => setPrLabel(...args),
    setPrReviewer: (...args: unknown[]) => setPrReviewer(...args),
    listReviews: () => listReviews(),
  },
}));

import { PrMetaPanel } from "./PrMetaPanel";
import { useUIStore } from "../store";
import type { PrMeta } from "../lib/types";

const meta = (over: Partial<PrMeta> = {}): PrMeta => ({
  number: 42,
  title: "Add widgets",
  url: "https://github.com/acme/widget/pull/42",
  body: "",
  bodyImages: {},
  state: "OPEN",
  isDraft: false,
  headRefOid: null,
  mergeable: "MERGEABLE",
  mergeStateStatus: "CLEAN",
  viewerCanUpdateBranch: false,
  reviewDecision: null,
  autoMerge: null,
  additions: 120,
  deletions: 7,
  changedFiles: 5,
  author: { login: "octocat", avatarUrl: null },
  labels: [],
  reviews: [],
  reviewRequests: [],
  ciState: null,
  checks: [],
  base: null,
  ...over,
});

function renderPanel(active = true, repoId: number | null = null) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<PrMetaPanel owner="acme" name="widget" number={42} active={active} repoId={repoId} />, {
    wrapper,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  listReviews.mockResolvedValue([]);
});

describe("PrMetaPanel", () => {
  it("renders counts, labels and the description as Markdown", async () => {
    prMeta.mockResolvedValue(
      meta({
        body: "**Summary** line.",
        labels: [{ name: "bug", color: "d73a4a" }],
      }),
    );
    renderPanel();

    expect(await screen.findByText(/5 files/)).toBeInTheDocument();
    expect(screen.getByText("+120")).toBeInTheDocument();
    expect(screen.getByText("−7")).toBeInTheDocument();
    expect(screen.getByText("bug")).toBeInTheDocument();
    expect(screen.getByText("Summary").tagName).toBe("STRONG");
  });

  const stacked = meta({
    base: {
      refName: "feat/parent",
      oid: "abcdef1234567890",
      pr: { number: 41, title: "Parent", url: "https://github.com/acme/widget/pull/41", state: "OPEN" },
    },
  });

  it("opens the base commit in the local graph and the parent PR on GitHub when it has no review", async () => {
    const user = userEvent.setup();
    const focusGraph = vi.spyOn(useUIStore.getState(), "focusGraph");
    useUIStore.setState({ focusGraph });
    prMeta.mockResolvedValue(stacked);
    renderPanel(true, 7);

    await user.click(await screen.findByText("abcdef1"));
    expect(focusGraph).toHaveBeenCalledWith({ repoId: 7, worktree: null, ref: null, sha: "abcdef1234567890" });
    await user.click(screen.getByText(/#41 Parent/));
    expect(openUrl).toHaveBeenCalledWith("https://github.com/acme/widget/pull/41");
  });

  it("opens the parent PR's review when one exists", async () => {
    const user = userEvent.setup();
    const openReview = vi.fn();
    useUIStore.setState({ openReview });
    listReviews.mockResolvedValue([
      {
        review: { id: 99, status: "draft" },
        target: { kind: "github_pr", github_pr_number: 41 },
        repo_remote: "acme/widget",
      },
    ]);
    prMeta.mockResolvedValue(stacked);
    renderPanel(true, 7);

    const link = await screen.findByText("#41 Parent");
    await waitFor(() => expect(link).toHaveClass("pr-link-review"));
    await user.click(link);
    expect(openReview).toHaveBeenCalledWith(99);
    expect(openUrl).not.toHaveBeenCalled();
  });

  it("does not link the base to GitHub when the repo has no local clone", async () => {
    prMeta.mockResolvedValue(stacked);
    renderPanel(true, null);

    expect(await screen.findByText("abcdef1")).toBeInTheDocument();
    expect(screen.getByText("abcdef1").closest("button")).toBeNull();
  });

  it("hides the base row when the base is the default branch", async () => {
    prMeta.mockResolvedValue(meta());
    renderPanel();

    await screen.findByText(/5 files/);
    expect(screen.queryByText("Base")).not.toBeInTheDocument();
  });

  it("shows a neutral Checking… state for unknown mergeability", async () => {
    prMeta.mockResolvedValue(meta({ mergeable: "UNKNOWN" }));
    renderPanel();

    const el = await screen.findByText("Checking…");
    expect(el).toHaveClass("mergeable-neutral");
  });

  it("renders a muted inline error without throwing", async () => {
    prMeta.mockRejectedValue("boom");
    renderPanel();

    expect(await screen.findByText(/Could not load PR details/)).toBeInTheDocument();
  });

  it("expands the check list and opens check urls via api.openUrl", async () => {
    const user = userEvent.setup();
    prMeta.mockResolvedValue(
      meta({ checks: [{ name: "build", state: "SUCCESS", url: "https://ci/build" }] }),
    );
    renderPanel();

    await user.click(await screen.findByText(/1 check/));
    await user.click(screen.getByRole("link", { name: "build" }));

    await waitFor(() => expect(openUrl).toHaveBeenCalledWith("https://ci/build"));
  });

  it("does not fetch while its review tab is inactive, then fetches once active", async () => {
    prMeta.mockResolvedValue(meta());
    const { rerender } = renderPanel(false);

    expect(screen.getByText(/Loading PR details/)).toBeInTheDocument();
    expect(prMeta).not.toHaveBeenCalled();

    rerender(<PrMetaPanel owner="acme" name="widget" number={42} active />);
    await waitFor(() => expect(prMeta).toHaveBeenCalledTimes(1));
  });

  it("shows the author and pending review requests, and removes a request", async () => {
    prMeta
      .mockResolvedValueOnce(meta({ reviewRequests: [{ login: "core", avatarUrl: null, isTeam: true }] }))
      .mockResolvedValue(meta());
    setPrReviewer.mockResolvedValue(undefined);
    renderPanel();

    expect(await screen.findByText("octocat")).toBeInTheDocument();
    await userEvent.click(screen.getByTitle("Remove review request for @core"));
    expect(setPrReviewer).toHaveBeenCalledWith("acme", "widget", 42, "core", true, false);
    await waitFor(() => expect(screen.queryByText("@core")).not.toBeInTheDocument());
  });

  it("hides reviewer editing on a merged PR but keeps labels editable", async () => {
    prMeta.mockResolvedValue(
      meta({
        state: "MERGED",
        labels: [{ name: "bug", color: "d73a4a" }],
        reviewRequests: [{ login: "rev", avatarUrl: null, isTeam: false }],
      }),
    );
    setPrLabel.mockResolvedValue(undefined);
    renderPanel();

    expect(await screen.findByText("rev")).toBeInTheDocument();
    expect(screen.queryByTitle("Remove review request for rev")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Request reviewers")).not.toBeInTheDocument();
    await userEvent.click(screen.getByTitle("Remove bug"));
    expect(setPrLabel).toHaveBeenCalledWith("acme", "widget", 42, "bug", false);
  });
});
