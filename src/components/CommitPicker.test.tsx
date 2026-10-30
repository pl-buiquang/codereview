import { describe, it, expect, vi, beforeAll, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const reviewCommits = vi.fn();
const createReview = vi.fn();
const diffRefs = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    reviewCommits: (...a: unknown[]) => reviewCommits(...a),
    createReview: (...a: unknown[]) => createReview(...a),
    diffRefs: (...a: unknown[]) => diffRefs(...a),
  },
}));

import { CommitPicker } from "./CommitPicker";
import type { GraphCommit } from "../lib/types";

const commit = (sha: string, subject: string, parent?: string) =>
  ({ sha, subject, parents: parent ? [parent] : [], author_name: "Ann", author_time: 1_700_000_000 }) as GraphCommit;
// Newest first.
const COMMITS = [
  commit("ccc3333", "third", "bbb2222"),
  commit("bbb2222", "second", "aaa1111"),
  commit("aaa1111", "first", "base000"),
];

function setup(hasClone = true) {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <CommitPicker reviewId={5} repoId={9} headSha="ccc3333" hasClone={hasClone} />
    </QueryClientProvider>,
  );
}

beforeAll(() => {
  Element.prototype.scrollIntoView = vi.fn();
});
beforeEach(() => {
  reviewCommits.mockReset().mockResolvedValue(COMMITS);
  createReview.mockReset().mockResolvedValue({ id: 77 });
  diffRefs.mockReset().mockResolvedValue("");
});

describe("CommitPicker", () => {
  it("is disabled without a local clone and does not query", () => {
    setup(false);
    expect(screen.getByRole("button", { name: /Commits/ })).toBeDisabled();
    expect(reviewCommits).not.toHaveBeenCalled();
  });

  it("loads lazily and creates a review from a shift-click range", async () => {
    setup();
    // One instance, so the held Shift key carries over to the click.
    const user = userEvent.setup();
    expect(reviewCommits).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /Commits/ }));
    await user.click(await screen.findByRole("button", { name: /third/ }));
    await user.keyboard("{Shift>}");
    await user.click(screen.getByRole("button", { name: /second/ }));
    await user.keyboard("{/Shift}");
    expect(screen.getByText("2 selected")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Create review" }));
    expect(createReview).toHaveBeenCalledWith({ repoId: 9, baseRef: "aaa1111", headRef: "ccc3333", threeDot: false });
  });

  it("a plain click selects a single commit", async () => {
    setup();
    await userEvent.click(screen.getByRole("button", { name: /Commits/ }));
    await userEvent.click(await screen.findByRole("button", { name: /first/ }));
    await userEvent.click(screen.getByRole("button", { name: /second/ }));
    expect(screen.getByText("1 selected")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Create review" }));
    expect(createReview).toHaveBeenCalledWith({ repoId: 9, baseRef: "aaa1111", headRef: "bbb2222", threeDot: false });
  });

  it("quick view previews the range diff", async () => {
    setup();
    await userEvent.click(screen.getByRole("button", { name: /Commits/ }));
    await userEvent.click(await screen.findByRole("button", { name: /second/ }));
    await userEvent.click(screen.getByRole("button", { name: "Quick view" }));
    await screen.findByLabelText("Quick view");
    expect(diffRefs).toHaveBeenCalledWith(9, "aaa1111", "bbb2222", false);
  });
});
