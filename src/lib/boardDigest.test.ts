import { describe, expect, it } from "vitest";
import type { BoardCard } from "./types";
import { buildDigest, digestHeadline } from "./boardDigest";

const NOW = new Date("2026-10-19T12:00:00Z");
const daysAgo = (n: number) => new Date(NOW.getTime() - n * 86_400_000).toISOString();

let seq = 0;
function card(over: Partial<BoardCard>): BoardCard {
  seq += 1;
  return {
    id: `id${seq}`,
    type: "pr",
    repo: "acme/back",
    number: seq,
    url: "",
    title: `t${seq}`,
    author_login: "me",
    state: "open",
    is_draft: false,
    review_decision: null,
    has_reviews: false,
    head_ref: null,
    is_mine: true,
    pending_threads: null,
    updated_at: daysAgo(1),
    closed_at: null,
    tracked_at: daysAgo(30),
    discarded_at: null,
    archived_at: null,
    last_refreshed: null,
    ci_status: "SUCCESS",
    mergeable: "MERGEABLE",
    merge_state_status: "CLEAN",
    stack_number: null,
    stack_position: null,
    viewer_can_update_branch: false,
    auto_merge: null,
    lane: "submitted",
    meta: [],
    inherited_meta: [],
    linked_issue_ids: ["issue"],
    linked_pr_ids: [],
    existing_review_id: null,
    ...over,
  };
}

describe("actionable", () => {
  it("counts every ranked card beyond the doNext cap", () => {
    const cards = Array.from({ length: 9 }, () => card({ review_decision: "APPROVED" }));
    const d = buildDigest(cards, NOW);
    expect(d.doNext).toHaveLength(7);
    expect(d.actionable).toBe(9);
  });
});

const refs = (items: { ref: string }[]) => items.map((i) => i.ref);

describe("buildDigest", () => {
  it("ranks review requests first, then merge, conflicts, CI", () => {
    const ci = card({ ci_status: "FAILURE" });
    const conflict = card({ mergeable: "CONFLICTING" });
    const approved = card({ review_decision: "APPROVED" });
    const review = card({ is_mine: false, lane: "reviews", review_decision: "REVIEW_REQUIRED" });
    const d = buildDigest([ci, conflict, approved, review], NOW);
    expect(d.doNext.map((i) => i.cardId)).toEqual([review.id, approved.id, conflict.id, ci.id]);
    expect(d.doNext.map((i) => i.reason)).toEqual([
      "Review (waiting 1d)",
      "Merge",
      "Resolve conflicts",
      "Fix CI",
    ]);
  });

  it("says update branch first for approved PRs that are behind", () => {
    const c = card({ review_decision: "APPROVED", merge_state_status: "BEHIND" });
    expect(buildDigest([c], NOW).doNext[0].reason).toBe("Update branch, then merge");
  });

  it("does not ask to merge approved PRs with auto-merge on", () => {
    const c = card({ review_decision: "APPROVED", auto_merge: "SQUASH" });
    const d = buildDigest([c], NOW);
    expect(d.doNext).toEqual([]);
    expect(d.waiting[0].reason).toBe("auto-merge on");
  });

  it("keeps own drafts out of Do next and Blocked, stale only at 14d", () => {
    const wip = card({ is_draft: true, ci_status: "FAILURE", mergeable: "CONFLICTING", updated_at: daysAgo(10) });
    const idle = card({ is_draft: true, updated_at: daysAgo(15) });
    const d = buildDigest([wip, idle], NOW);
    expect(d.doNext).toEqual([]);
    expect(d.blocked).toEqual([]);
    expect(d.stale.map((i) => i.cardId)).toEqual([idle.id]);
  });

  it("puts a PR in Do next or Waiting, never both", () => {
    const pending = card({ ci_status: "PENDING" });
    const failing = card({ ci_status: "FAILURE" });
    const d = buildDigest([pending, failing], NOW);
    expect(d.waiting.map((i) => i.cardId)).toEqual([pending.id]);
    expect(d.doNext.map((i) => i.cardId)).toEqual([failing.id]);
  });

  it("flags review requests waiting more than 2d as blocked", () => {
    const old = card({ is_mine: false, lane: "reviews", review_decision: "REVIEW_REQUIRED", updated_at: daysAgo(3) });
    const approvedByOthers = card({ is_mine: false, lane: "reviews", review_decision: "APPROVED" });
    const d = buildDigest([old, approvedByOthers], NOW);
    expect(d.blocked.map((i) => i.cardId)).toEqual([old.id]);
    expect(d.doNext.map((i) => i.cardId)).toEqual([old.id]);
  });

  it("flags stale PRs and issues with different thresholds", () => {
    const pr = card({ updated_at: daysAgo(8) });
    const issue = card({ type: "issue", lane: "backlog", updated_at: daysAgo(10) });
    const oldIssue = card({ type: "issue", lane: "backlog", updated_at: daysAgo(22) });
    const d = buildDigest([pr, issue, oldIssue], NOW);
    expect(d.stale.map((i) => i.cardId)).toEqual([pr.id, oldIssue.id]);
  });

  it("flags issues whose linked PRs are all closed, and PRs without an issue", () => {
    const merged = card({ state: "merged" });
    const issue = card({ type: "issue", lane: null, linked_pr_ids: [merged.id] });
    const lone = card({ linked_issue_ids: [] });
    const d = buildDigest([merged, issue, lone], NOW);
    expect(d.hygiene.map((i) => [i.cardId, i.reason])).toEqual([
      [issue.id, "linked PR is no longer open: close or archive"],
      [lone.id, "PR has no linked issue"],
    ]);
  });

  it("ignores archived, discarded and closed cards", () => {
    const d = buildDigest(
      [card({ archived_at: daysAgo(1) }), card({ discarded_at: daysAgo(1) }), card({ state: "merged" })],
      NOW,
    );
    expect(d.open).toBe(0);
  });

  it("counts lanes per repo", () => {
    const d = buildDigest(
      [
        card({ lane: "submitted" }),
        card({ lane: "in_progress", is_draft: true }),
        card({ repo: "acme/front", type: "issue", lane: "backlog" }),
      ],
      NOW,
    );
    expect(d.byProject).toEqual([
      { repo: "acme/back", backlog: 0, in_progress: 1, submitted: 1, reviews: 0 },
      { repo: "acme/front", backlog: 1, in_progress: 0, submitted: 0, reviews: 0 },
    ]);
    expect(digestHeadline(d)).toBe("3 open · 0 to do next · 0 blocked · 0 stale");
  });

  it("caps Do next at 7", () => {
    const many = Array.from({ length: 9 }, () => card({ ci_status: "FAILURE" }));
    expect(refs(buildDigest(many, NOW).doNext)).toHaveLength(7);
  });
});
