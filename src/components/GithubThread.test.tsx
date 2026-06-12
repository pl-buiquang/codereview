import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const setPrThreadResolved = vi.fn();
const replyToThread = vi.fn();
const openUrl = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    setPrThreadResolved: (threadId: string, resolved: boolean) =>
      setPrThreadResolved(threadId, resolved),
    replyToThread: (
      owner: string,
      name: string,
      number: number,
      commentId: number,
      body: string,
    ) => replyToThread(owner, name, number, commentId, body),
    openUrl: (url: string) => openUrl(url),
  },
}));

import { GithubThread } from "./GithubThread";
import type { PrThread, PrThreadComment, PrThreadCtx } from "../lib/types";

const ctx: PrThreadCtx = { owner: "acme", name: "widget", number: 42 };

const comment = (over: Partial<PrThreadComment> = {}): PrThreadComment => ({
  id: "C1",
  databaseId: 1001,
  author: { login: "rev1", avatarUrl: null },
  body: "Looks good.",
  createdAt: "2024-03-01T00:00:00Z",
  url: "https://github.com/acme/widget/pull/42#discussion_r1001",
  diffHunk: null,
  outdated: false,
  ...over,
});

const thread = (over: Partial<PrThread> = {}): PrThread => ({
  id: "T1",
  isResolved: false,
  isOutdated: false,
  isCollapsed: false,
  path: "src/a.rs",
  line: 12,
  startLine: null,
  originalLine: 12,
  diffSide: "RIGHT",
  startDiffSide: null,
  subjectType: "LINE",
  comments: [comment()],
  ...over,
});

function renderThread(props: Parameters<typeof GithubThread>[0]) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<GithubThread {...props} />, { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
  setPrThreadResolved.mockResolvedValue(true);
  replyToThread.mockResolvedValue(2002);
});

describe("GithubThread", () => {
  it("resolves a thread and flips the label when already resolved", async () => {
    setPrThreadResolved.mockResolvedValue(true);
    renderThread({ thread: thread({ isResolved: false }), ctx });

    await userEvent.click(screen.getByRole("button", { name: "Resolve" }));
    expect(setPrThreadResolved).toHaveBeenCalledWith("T1", true);

    // A resolved thread shows the inverse action.
    renderThread({ thread: thread({ isResolved: true }), ctx });
    expect(screen.getByRole("button", { name: "Unresolve" })).toBeInTheDocument();
  });

  it("replies via the composer and closes it on success", async () => {
    renderThread({ thread: thread(), ctx });

    await userEvent.click(screen.getByRole("button", { name: "Reply…" }));
    const textarea = screen.getByPlaceholderText("Leave a comment…");
    await userEvent.type(textarea, "from the app");
    await userEvent.click(screen.getByRole("button", { name: "Reply" }));

    expect(replyToThread).toHaveBeenCalledWith("acme", "widget", 42, 1001, "from the app");
    // Composer closes -> the collapsed affordance returns.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Reply…" })).toBeInTheDocument(),
    );
  });

  it("renders read-only when no ctx is provided", () => {
    renderThread({ thread: thread() });
    expect(screen.queryByRole("button", { name: "Resolve" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reply…" })).not.toBeInTheDocument();
  });

  it("hides the reply affordance when the root databaseId is null", () => {
    renderThread({ thread: thread({ comments: [comment({ databaseId: null })] }), ctx });
    expect(screen.getByRole("button", { name: "Resolve" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reply…" })).not.toBeInTheDocument();
  });
});
