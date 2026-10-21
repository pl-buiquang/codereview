import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import type { Comment, FocusEntry, FocusView, ReviewDetail } from "../lib/types";

const handlers: Record<string, (...args: unknown[]) => unknown> = {};
const ARRAY_RESULTS = new Set(["chatMessages", "listRepositories", "listReviews"]);
vi.mock("../lib/api", () => ({
  api: new Proxy(
    {},
    {
      get: (_t, key: string) =>
        handlers[key] ?? (() => Promise.resolve(ARRAY_RESULTS.has(key) ? [] : null)),
    },
  ),
  pickSavePath: () => Promise.resolve(null),
}));

import { ReviewView, newCommitsLabel } from "./ReviewView";
import { useUIStore } from "../store";
import { useSettingsStore } from "../lib/settings";

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

// src/a.ts has two hunks: a visible critical one, and a trivial one that focus
// hides but stays inside the file (collapsed bar). Cargo.lock has a single
// trivial hunk, so hiding it hides the whole file (removed from the diff).
const DIFF = `diff --git a/src/a.ts b/src/a.ts
index 1111111..2222222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -1,3 +1,3 @@
 const a = 1;
-const b = 2;
+const b = 3;
 const c = 4;
@@ -10,2 +10,2 @@
-const d = 5;
+const d = 6;
 const e = 7;
diff --git a/Cargo.lock b/Cargo.lock
index 3333333..4444444 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,1 +1,1 @@
-lockline_old
+lockline_new
`;

function comment(over: Partial<Comment> & { id: number }): Comment {
  return {
    review_id: 7,
    file_path: "src/a.ts",
    subject_type: "line",
    origin: "diff",
    side: "RIGHT",
    line: 1,
    start_line: null,
    diff_hunk: null,
    body: `body ${over.id}`,
    parent_id: null,
    anchored_head_sha: "head",
    anchored_base_sha: null,
    github_comment_id: null,
    resolved_at: null,
    github_thread_root_id: null,
    tag: null,
    created_at: "now",
    updated_at: "now",
    ...over,
  };
}

function detail(comments: Comment[] = []): ReviewDetail {
  return {
    review: {
      id: 7,
      target_id: 1,
      body: "",
      event: null,
      status: "draft",
      base_sha: "base",
      head_sha: "head",
      published_at: null,
      github_review_id: null,
      last_exported_at: null,
      created_at: "now",
      updated_at: "now",
      sidebar_collapsed: true,
      chat_collapsed: true,
      pr_details_collapsed: false,
      left_panel_width: 248,
      right_panel_width: 360,
    },
    target: {
      id: 1,
      repo_id: 1,
      kind: "local",
      github_pr_number: null,
      title: "feature",
      base_ref: "main",
      head_ref: "feature",
      three_dot: true,
      created_at: "now",
    },
    local_path: "/repo",
    remote_owner: null,
    remote_name: null,
    comments,
    viewed_files: [],
  } as unknown as ReviewDetail;
}

function entry(over: Partial<FocusEntry>): FocusEntry {
  return {
    hunk_id: "a1",
    file_path: "src/a.ts",
    old_start: 1,
    new_start: 1,
    header: "@@ -1,3 +1,3 @@",
    additions: 1,
    deletions: 1,
    tier: "critical",
    source: "ai",
    reason: null,
    override: null,
    is_new: false,
    ...over,
  };
}

function focusView(): FocusView {
  return {
    focus: {
      id: 1,
      review_id: 7,
      head_sha: "head",
      base_sha: "base",
      level: "focus",
      summary: "Bumps b.",
      model: null,
      status: "ready",
      error: null,
      planned_at: "now",
      created_at: "now",
    },
    stale: false,
    hunks: [
      entry({}),
      entry({
        hunk_id: "a2",
        old_start: 10,
        new_start: 10,
        header: "@@ -10,2 +10,2 @@",
        tier: "trivial",
        source: "rule",
        reason: "import reorder",
      }),
      entry({
        hunk_id: "l1",
        file_path: "Cargo.lock",
        header: "@@ -1,1 +1,1 @@",
        tier: "trivial",
        source: "rule",
        reason: "lockfile",
      }),
    ],
    file_overrides: [],
  };
}

function renderReview() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<ReviewView reviewId={7} />, { wrapper });
}

beforeEach(() => {
  useSettingsStore.setState({ aiEnabled: true });
  for (const k of Object.keys(handlers)) delete handlers[k];
  handlers.getReview = vi.fn(() => Promise.resolve(detail()));
  handlers.reviewDiff = vi.fn(() => Promise.resolve(DIFF));
  handlers.getFocus = vi.fn(() => Promise.resolve(focusView()));
  handlers.setFocusLevel = vi.fn(() => Promise.resolve());
  handlers.setHunkOverride = vi.fn(() => Promise.resolve());
  handlers.setFileOverride = vi.fn(() => Promise.resolve());
  handlers.generateFocus = vi.fn(() => Promise.resolve());
  handlers.checkEnvironment = vi.fn(() =>
    Promise.resolve({ git: "git", gh: "gh", gh_authed: true, claude: "/bin/claude" }),
  );
});

describe("ReviewView focus overlay", () => {
  it("collapses a trivial hunk into a bar inside its file, hides a fully-trivial file entirely", async () => {
    renderReview();
    expect(await screen.findByText("Bumps b.")).toBeInTheDocument();
    expect(screen.getByText("critical")).toBeInTheDocument();
    // The in-file hunk stays inside src/a.ts, collapsed.
    expect(await screen.findByText(/1 hidden hunk — import reorder/)).toBeInTheDocument();
    // Cargo.lock is fully hidden: removed from the diff, not just collapsed.
    expect(screen.queryByText("lockline_new")).not.toBeInTheDocument();
    expect(screen.queryByText(/1 hidden hunk — lockfile/)).not.toBeInTheDocument();
    // ... but still shows up in the digest, grouped by reason.
    expect(screen.getByText("lockfile")).toBeInTheDocument();
    expect(screen.getByText("Cargo.lock")).toBeInTheDocument();
  });

  it("reveals an in-file hidden hunk temporarily via show, without an API call", async () => {
    const user = userEvent.setup();
    renderReview();
    const bar = (await screen.findByText(/1 hidden hunk — import reorder/)).closest(
      ".focus-bar",
    ) as HTMLElement;
    await user.click(within(bar).getByRole("button", { name: "show" }));
    await waitFor(() =>
      expect(screen.queryByText(/1 hidden hunk — import reorder/)).not.toBeInTheDocument(),
    );
    expect(handlers.setHunkOverride).not.toHaveBeenCalled();
  });

  it("keep shown persists an in-file hidden hunk as a hunk override", async () => {
    const user = userEvent.setup();
    renderReview();
    const bar = (await screen.findByText(/1 hidden hunk — import reorder/)).closest(
      ".focus-bar",
    ) as HTMLElement;
    await user.click(within(bar).getByRole("button", { name: "keep shown" }));
    expect(handlers.setHunkOverride).toHaveBeenCalledWith(7, "a2", "show");
  });

  it("reveals a fully-hidden file from the digest", async () => {
    const user = userEvent.setup();
    renderReview();
    const group = (await screen.findByText("lockfile")).closest("li") as HTMLElement;
    await user.click(within(group).getByRole("button", { name: "show" }));
    await waitFor(() => expect(screen.getByText("lockline_new")).toBeInTheDocument());
  });

  it("never hides a hunk that has an untagged comment", async () => {
    handlers.getReview = vi.fn(() =>
      Promise.resolve(
        detail([comment({ id: 1, file_path: "Cargo.lock", line: 1, body: "why?" })]),
      ),
    );
    renderReview();
    expect(await screen.findByText("lockline_new")).toBeInTheDocument();
    expect(screen.queryByText(/hidden hunk — lockfile/)).not.toBeInTheDocument();
  });

  it("shows the AI pill on an ai-tagged comment, and doesn't let it pin a hidden hunk visible", async () => {
    handlers.getReview = vi.fn(() =>
      Promise.resolve(
        detail([
          comment({ id: 1, line: 1, body: "visible-hunk note", tag: "ai" }),
          comment({ id: 2, line: 10, body: "hidden-hunk note", tag: "ai" }),
        ]),
      ),
    );
    renderReview();
    expect(await screen.findByText("visible-hunk note")).toBeInTheDocument();
    expect(screen.getAllByText("AI").length).toBeGreaterThan(0);
    // The second comment is anchored inside the trivial hunk; being `ai`-tagged,
    // it must not force that hunk visible.
    expect(screen.getByText(/1 hidden hunk — import reorder/)).toBeInTheDocument();
    expect(screen.queryByText("hidden-hunk note")).not.toBeInTheDocument();
  });

  it("starts a generation from the header", async () => {
    const user = userEvent.setup();
    handlers.getFocus = vi.fn(() => Promise.resolve(null));
    renderReview();
    await user.click(await screen.findByRole("button", { name: /Generate focus/ }));
    const pop = screen.getByRole("dialog", { name: "Generate focus" });
    await user.click(within(pop).getByRole("button", { name: "Generate" }));
    expect(handlers.generateFocus).toHaveBeenCalledWith(7, undefined);
  });

  it("moves Regenerate into the review actions menu once a focus exists", async () => {
    const user = userEvent.setup();
    renderReview();
    await user.click(await screen.findByRole("button", { name: "Review actions" }));
    await user.click(await screen.findByRole("menuitem", { name: "Regenerate AI focus" }));
    expect(handlers.generateFocus).toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: /Regenerate focus/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Review actions" }));
    expect(await screen.findByRole("menuitem", { name: "Re-anchor comments" })).toBeDisabled();
  });

  it("disables Generate without the claude CLI", async () => {
    handlers.getFocus = vi.fn(() => Promise.resolve(null));
    handlers.checkEnvironment = vi.fn(() =>
      Promise.resolve({ git: "git", gh: "gh", gh_authed: true, claude: null }),
    );
    renderReview();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Generate focus/ })).toBeDisabled(),
    );
  });
});

describe("ReviewView pin", () => {
  it("shows new commits on a local draft and Update review moves the pin", async () => {
    const user = userEvent.setup();
    useUIStore.setState({ activeTabId: "review-7" });
    handlers.refreshReview = vi.fn(() =>
      Promise.resolve({ moved: true, pinnedHeadSha: "head", liveHeadSha: "head2", liveBaseSha: "base", newCommits: 2 }),
    );
    handlers.updateReviewPin = vi.fn(() =>
      Promise.resolve({ moved: true, reanchor: { reanchored: 1, lost: 0, skippedNoChange: 0 } }),
    );
    renderReview();

    expect(await screen.findByText("2 new commits")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Update review" }));
    await waitFor(() => expect(handlers.updateReviewPin).toHaveBeenCalledWith(7));
  });

  it("labels what moved", () => {
    const live = { moved: true, pinnedHeadSha: "h", liveHeadSha: "h2", liveBaseSha: "b", newCommits: 1 };
    expect(newCommitsLabel(live, "h")).toBe("1 new commit");
    expect(newCommitsLabel({ ...live, liveHeadSha: "h", newCommits: 0 }, "h")).toBe("base moved");
    expect(newCommitsLabel(undefined, "h")).toBe("new commits");
  });
});
