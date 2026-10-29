import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const prMeta = vi.fn();
const openUrl = vi.fn();
const setPrLabel = vi.fn();
const setPrReviewer = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    prMeta: (owner: string, name: string, number: number) => prMeta(owner, name, number),
    openUrl: (url: string) => openUrl(url),
    setPrLabel: (...args: unknown[]) => setPrLabel(...args),
    setPrReviewer: (...args: unknown[]) => setPrReviewer(...args),
  },
}));

import { PrMetaPanel } from "./PrMetaPanel";
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

function renderPanel(active = true) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<PrMetaPanel owner="acme" name="widget" number={42} active={active} />, {
    wrapper,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
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

  it("shows the base with its branch, commit and parent PR links only when set", async () => {
    const user = userEvent.setup();
    prMeta.mockResolvedValue(
      meta({
        base: {
          refName: "feat/parent",
          oid: "abcdef1234567890",
          pr: { number: 41, title: "Parent", url: "https://github.com/acme/widget/pull/41", state: "OPEN" },
        },
      }),
    );
    renderPanel();

    expect(await screen.findByText("Base")).toBeInTheDocument();
    await user.click(screen.getByText("abcdef1"));
    expect(openUrl).toHaveBeenCalledWith("https://github.com/acme/widget/tree/abcdef1234567890");
    await user.click(screen.getByText("#41 Parent"));
    expect(openUrl).toHaveBeenCalledWith("https://github.com/acme/widget/pull/41");
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
