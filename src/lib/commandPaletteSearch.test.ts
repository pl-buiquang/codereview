import { describe, expect, it } from "vitest";
import type { BoardCard, ReviewSummary } from "./types";
import { searchPalette } from "./commandPaletteSearch";

const review = (id: number, repo: string, pr: number) =>
  ({
    review: { id, updated_at: "2026-10-16T00:00:00Z" },
    target: { title: `PR ${pr}`, kind: "github_pr", github_pr_number: pr },
    repo_id: 1,
    repo_label: repo,
    comment_count: 0,
  }) as unknown as ReviewSummary;

const card = (id: string, repo: string, number: number, type: "pr" | "issue" = "pr") =>
  ({ id, repo, number, type, title: `T${number}`, archived_at: null, discarded_at: null }) as BoardCard;

const reviews = [review(1, "acme/front", 12), review(2, "acme/back", 12)];
const cards = [card("C1", "acme/front", 12), card("C2", "acme/back", 40, "issue")];

describe("searchPalette", () => {
  it("matches the existing review for a pasted PR URL", () => {
    const r = searchPalette("https://github.com/acme/front/pull/12", reviews, []);
    expect(r.filter((x) => x.kind === "review").map((x) => x.kind === "review" && x.reviewId)).toEqual([1]);
    expect(r.some((x) => x.kind === "create-pr")).toBe(false);
  });

  it("offers the board card for a PR or issue URL", () => {
    const pr = searchPalette("https://github.com/acme/front/pull/12", reviews, [], cards);
    expect(pr[0]).toMatchObject({ kind: "board-card", cardId: "C1" });
    const issue = searchPalette("https://github.com/acme/back/issues/40", [], [], cards);
    expect(issue).toEqual([expect.objectContaining({ kind: "board-card", cardId: "C2" })]);
    const ref = searchPalette("acme/back#40", [], [], cards);
    expect(ref[0]).toMatchObject({ kind: "board-card", cardId: "C2" });
  });

  it("offers to open an untracked PR URL", () => {
    const r = searchPalette("https://github.com/acme/other/pull/3", reviews, [], cards);
    expect(r).toEqual([expect.objectContaining({ kind: "create-pr", owner: "acme", name: "other", prNumber: 3 })]);
  });
});
