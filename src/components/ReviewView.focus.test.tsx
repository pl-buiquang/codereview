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

import { ReviewView } from "./ReviewView";

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const DIFF = `diff --git a/src/a.ts b/src/a.ts
index 1111111..2222222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -1,3 +1,3 @@
 const a = 1;
-const b = 2;
+const b = 3;
 const c = 4;
diff --git a/Cargo.lock b/Cargo.lock
index 3333333..4444444 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1,1 +1,1 @@
-lockline_old
+lockline_new
`;

function detail(comments: Comment[] = []): ReviewDetail {
  return {
    review: {
      id: 7,
      target_id: 1,
      body: "",
      event: null,
      status: "draft",
      published_at: null,
      github_review_id: null,
      last_exported_at: null,
      created_at: "now",
      updated_at: "now",
      sidebar_collapsed: true,
      chat_collapsed: true,
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
      base_sha: "base",
      head_sha: "head",
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
    note: "Is **3** right?",
    note_dismissed: false,
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
        hunk_id: "l1",
        file_path: "Cargo.lock",
        header: "@@ -1,1 +1,1 @@",
        tier: "trivial",
        source: "rule",
        reason: "lockfile",
        note: null,
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
  for (const k of Object.keys(handlers)) delete handlers[k];
  handlers.getReview = vi.fn(() => Promise.resolve(detail()));
  handlers.reviewDiff = vi.fn(() => Promise.resolve(DIFF));
  handlers.getFocus = vi.fn(() => Promise.resolve(focusView()));
  handlers.setFocusLevel = vi.fn(() => Promise.resolve());
  handlers.setHunkOverride = vi.fn(() => Promise.resolve());
  handlers.adoptFocusNote = vi.fn(() => Promise.resolve({}));
});

describe("ReviewView focus overlay", () => {
  it("collapses low-tier hunks into a bar and shows the digest, tier and note", async () => {
    renderReview();
    expect(await screen.findByText(/1 hidden hunk — lockfile/)).toBeInTheDocument();
    expect(screen.queryByText("lockline_new")).not.toBeInTheDocument();
    expect(screen.getByText("Bumps b.")).toBeInTheDocument();
    expect(screen.getByText("critical")).toBeInTheDocument();
    expect(screen.getByText("right?", { exact: false })).toBeInTheDocument();
  });

  it("reveals a hidden hunk temporarily, or permanently via an override", async () => {
    const user = userEvent.setup();
    renderReview();
    const bar = (await screen.findByText(/1 hidden hunk/)).closest(".focus-bar") as HTMLElement;
    await user.click(within(bar).getByRole("button", { name: "always show" }));
    expect(handlers.setHunkOverride).toHaveBeenCalledWith(7, "l1", "show");
    await waitFor(() => expect(screen.getByText("lockline_new")).toBeInTheDocument());
  });

  it("adopts a note and switches levels", async () => {
    const user = userEvent.setup();
    renderReview();
    await user.click(await screen.findByRole("button", { name: "Adopt" }));
    expect(handlers.adoptFocusNote).toHaveBeenCalledWith(7, "a1");

    await user.selectOptions(screen.getByRole("combobox"), "off");
    expect(handlers.setFocusLevel).toHaveBeenCalledWith(7, "off");
    await waitFor(() => expect(screen.getByText("lockline_new")).toBeInTheDocument());
    expect(screen.queryByText(/hidden hunk/)).not.toBeInTheDocument();
  });

  it("never hides a hunk that has a comment", async () => {
    handlers.getReview = vi.fn(() =>
      Promise.resolve(
        detail([
          {
            id: 1,
            review_id: 7,
            file_path: "Cargo.lock",
            subject_type: "line",
            origin: "diff",
            side: "RIGHT",
            line: 1,
            start_line: null,
            diff_hunk: null,
            body: "why?",
            parent_id: null,
            anchored_head_sha: "head",
            anchored_base_sha: null,
            github_thread_root_id: null,
            resolved_at: null,
            created_at: "now",
            updated_at: "now",
          } as unknown as Comment,
        ]),
      ),
    );
    renderReview();
    expect(await screen.findByText("lockline_new")).toBeInTheDocument();
    expect(screen.queryByText(/hidden hunk/)).not.toBeInTheDocument();
  });
});
