import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { Comment, ToolEnv } from "../lib/types";

const checkEnvironment = vi.fn<() => Promise<ToolEnv>>();
const setCommentPublish = vi.fn((_id: number, _v: boolean) => Promise.resolve());
vi.mock("../lib/api", () => ({
  api: {
    checkEnvironment: () => checkEnvironment(),
    setCommentPublish: (id: number, v: boolean) => setCommentPublish(id, v),
  },
  pickSavePath: vi.fn(),
}));

import { CommentItem, ThreadActions } from "./ReviewView";

const env = (skill: string | null): ToolEnv => ({
  git: "/usr/bin/git",
  gh: null,
  gh_authed: false,
  claude: "/bin/claude",
  opencode: null,
  codex: null,
  my_voice_skill: skill,
});

const comment = (over: Partial<Comment> = {}): Comment => ({
  id: 7,
  review_id: 1,
  file_path: "a.rs",
  subject_type: "line",
  origin: "diff",
  side: "RIGHT",
  line: 3,
  start_line: null,
  diff_hunk: null,
  body: "**[high · codex · loop]** this never ends",
  parent_id: null,
  anchored_head_sha: null,
  anchored_base_sha: null,
  github_comment_id: null,
  resolved_at: null,
  github_thread_root_id: null,
  tag: "ai:codex",
  publish: null,
  created_at: "now",
  updated_at: "now",
  ...over,
});

function renderItem(c: Comment, onSendToChat?: () => void) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const noop = () => {};
  return render(
    <QueryClientProvider client={qc}>
      <CommentItem
        comment={c}
        headSha={null}
        baseSha={null}
        readOnly={false}
        onSaving={noop}
        onSaved={noop}
        onCommentsChanged={noop}
      />
      <ThreadActions
        root={c}
        readOnly={false}
        onReply={noop}
        onSendToChat={onSendToChat}
        onCommentsChanged={noop}
      />
    </QueryClientProvider>,
  );
}

describe("comment header and thread actions", () => {
  beforeEach(() => {
    checkEnvironment.mockReset();
    setCommentPublish.mockClear();
  });

  it("shows My voice on AI comments only when the skill is installed", async () => {
    checkEnvironment.mockResolvedValue(env("/home/me/.claude/skills/my-voice"));
    renderItem(comment());
    expect(await screen.findByText("My voice")).toBeTruthy();
  });

  it("hides My voice without the skill", async () => {
    checkEnvironment.mockResolvedValue(env(null));
    renderItem(comment());
    await waitFor(() => expect(checkEnvironment).toHaveBeenCalled());
    expect(screen.queryByText("My voice")).toBeNull();
  });

  it("offers no My voice on untagged comments", async () => {
    checkEnvironment.mockResolvedValue(env("/skill"));
    renderItem(comment({ tag: null }));
    await waitFor(() => expect(checkEnvironment).toHaveBeenCalled());
    expect(screen.queryByText("My voice")).toBeNull();
  });

  const toggle = () => screen.getByRole("button", { pressed: undefined, name: /Click to|never published/ });

  it("AI comments start local; clicking publishes them", async () => {
    checkEnvironment.mockResolvedValue(env(null));
    renderItem(comment());
    expect(toggle().getAttribute("aria-pressed")).toBe("false");
    await userEvent.click(toggle());
    expect(setCommentPublish).toHaveBeenCalledWith(7, true);
  });

  it("untagged comments start published; clicking keeps them local", async () => {
    checkEnvironment.mockResolvedValue(env(null));
    renderItem(comment({ tag: null }));
    expect(toggle().getAttribute("aria-pressed")).toBe("true");
    await userEvent.click(toggle());
    expect(setCommentPublish).toHaveBeenCalledWith(7, false);
  });

  it("local-tagged comments can't be published", async () => {
    checkEnvironment.mockResolvedValue(env(null));
    renderItem(comment({ tag: "local" }));
    expect((toggle() as HTMLButtonElement).disabled).toBe(true);
  });

  it("renders Send to chat on local threads when wired", async () => {
    checkEnvironment.mockResolvedValue(env(null));
    renderItem(comment({ tag: null }), vi.fn());
    expect(screen.getByText("Send to chat")).toBeTruthy();
  });
});
