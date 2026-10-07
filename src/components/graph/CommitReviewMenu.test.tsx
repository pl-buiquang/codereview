import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const createReview = vi.fn();
const createReviewForPr = vi.fn();
vi.mock("../../lib/api", () => ({
  api: {
    createReview: (args: unknown) => createReview(args),
    createReviewForPr: (...args: unknown[]) => createReviewForPr(...args),
    listBranches: () => Promise.resolve([]),
  },
}));

const openReview = vi.fn();
vi.mock("../../store", () => ({
  useUIStore: (sel: (s: { openReview: typeof openReview }) => unknown) => sel({ openReview }),
}));

import { CommitReviewMenu, reviewHead } from "./CommitReviewMenu";
import type { CommitDetail, PrSummary, RefInfo, Repository } from "../../lib/types";

const repo: Repository = {
  id: 3,
  local_path: "/r",
  remote_owner: "acme",
  remote_name: "w",
  default_branch: "main",
  added_at: "",
};
const SHA = "f".repeat(40);
const PARENT = "e".repeat(40);
const detail = { sha: SHA, parents: [PARENT] } as CommitDetail;
const feat: RefInfo = { name: "feat", kind: "local", sha: SHA, is_head: false, updated_at: 0 };
const pr: PrSummary = {
  number: 12,
  title: "t",
  author: null,
  headRefName: "feat",
  baseRefName: "main",
  createdAt: "",
  url: "",
};

function renderMenu(tipRefs: RefInfo[], pullRequests: PrSummary[]) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <CommitReviewMenu repo={repo} detail={detail} tipRefs={tipRefs} pullRequests={pullRequests} />
    </QueryClientProvider>,
  );
  return userEvent.setup();
}

beforeEach(() => {
  vi.clearAllMocks();
  createReview.mockResolvedValue({ id: 5 });
  createReviewForPr.mockResolvedValue({ id: 6 });
});

describe("CommitReviewMenu", () => {
  it("defaults to the linked GitHub PR", async () => {
    const user = renderMenu([feat], [pr]);
    await user.click(screen.getByRole("button", { name: "Review PR #12" }));
    await waitFor(() => expect(createReviewForPr).toHaveBeenCalledWith("acme", "w", 12, expect.any(Array)));
    await waitFor(() => expect(openReview).toHaveBeenCalledWith(6));
  });

  it("defaults to the branch vs the default branch without a PR", async () => {
    const user = renderMenu([feat], []);
    await user.click(screen.getByRole("button", { name: "Review vs main" }));
    await waitFor(() =>
      expect(createReview).toHaveBeenCalledWith({ repoId: 3, baseRef: "main", headRef: "feat", threeDot: true }),
    );
  });

  it("reviews just this commit against its first parent", async () => {
    const user = renderMenu([], []);
    await user.click(screen.getByRole("button", { name: "More review options" }));
    await user.click(screen.getByRole("menuitem", { name: /This commit only/ }));
    await waitFor(() =>
      expect(createReview).toHaveBeenCalledWith({ repoId: 3, baseRef: PARENT, headRef: SHA, threeDot: false }),
    );
  });

  it("prefers a local branch, then a remote one, then the sha as head", () => {
    const remote: RefInfo = { ...feat, name: "origin/feat", kind: "remote" };
    expect(reviewHead(detail, [remote, feat])).toBe("feat");
    expect(reviewHead(detail, [remote])).toBe("origin/feat");
    expect(reviewHead(detail, [])).toBe(SHA);
  });
});
