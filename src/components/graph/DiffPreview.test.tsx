import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const commitFileDiff = vi.fn();
const worktreeFileDiff = vi.fn();
vi.mock("../../lib/api", () => ({
  api: {
    commitFileDiff: (a: unknown) => commitFileDiff(a),
    worktreeFileDiff: (a: unknown) => worktreeFileDiff(a),
  },
}));

import { DiffPreview, type DiffPreviewTarget } from "./DiffPreview";

const DIFF = `diff --git a/src/a.ts b/src/a.ts
index 1111111..2222222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -1,2 +1,2 @@
 const x = 1;
-const y = 2;
+const y = 3;
`;

const commitTarget: DiffPreviewTarget = {
  kind: "commit",
  repoId: 7,
  sha: "abcdef1234567890",
  file: { path: "src/new.ts", old_path: "src/old.ts", status: "R", additions: 1, deletions: 1 },
};

const wipTarget: DiffPreviewTarget = {
  kind: "wip",
  repoId: 7,
  worktreePath: "/tmp/wt",
  path: "src/a.ts",
  wipKind: "unstaged",
};

function renderPreview(
  target: DiffPreviewTarget,
  over: { index?: number; files?: string[]; onNavigate?: () => void; onClose?: () => void } = {},
) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const onNavigate = over.onNavigate ?? vi.fn();
  const onClose = over.onClose ?? vi.fn();
  render(
    <DiffPreview
      target={target}
      files={over.files ?? ["a", "b", "c"]}
      index={over.index ?? 0}
      onNavigate={onNavigate}
      onClose={onClose}
    />,
    { wrapper },
  );
  return { onNavigate, onClose };
}

describe("DiffPreview", () => {
  beforeEach(() => {
    commitFileDiff.mockReset().mockResolvedValue(DIFF);
    worktreeFileDiff.mockReset().mockResolvedValue(DIFF);
  });

  it("fetches a commit file diff and renders it", async () => {
    renderPreview(commitTarget);
    expect(screen.getByText("src/old.ts → src/new.ts")).toBeInTheDocument();
    await waitFor(() => expect(document.querySelector(".diff-file")).not.toBeNull());
    expect(screen.getByText("+1")).toBeInTheDocument();
    expect(commitFileDiff).toHaveBeenCalledWith({
      repoId: 7,
      sha: "abcdef1234567890",
      path: "src/new.ts",
      oldPath: "src/old.ts",
    });
    expect(worktreeFileDiff).not.toHaveBeenCalled();
  });

  it("fetches a worktree file diff for WIP targets", async () => {
    renderPreview(wipTarget);
    await waitFor(() => expect(worktreeFileDiff).toHaveBeenCalled());
    expect(worktreeFileDiff).toHaveBeenCalledWith({
      repoId: 7,
      worktreePath: "/tmp/wt",
      path: "src/a.ts",
      kind: "unstaged",
    });
    expect(commitFileDiff).not.toHaveBeenCalled();
  });

  it("closes on X and on Escape", async () => {
    const { onClose } = renderPreview(commitTarget);
    await userEvent.click(screen.getByRole("button", { name: "Close preview" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("wraps prev/next around the file list", async () => {
    const first = renderPreview(commitTarget, { index: 0 });
    await userEvent.click(screen.getByRole("button", { name: "Previous file" }));
    expect(first.onNavigate).toHaveBeenLastCalledWith(2);
    await userEvent.click(screen.getByRole("button", { name: "Next file" }));
    expect(first.onNavigate).toHaveBeenLastCalledWith(1);
  });

  it("wraps next from the last file to the first", async () => {
    const { onNavigate } = renderPreview(commitTarget, { index: 2 });
    await userEvent.click(screen.getByRole("button", { name: "Next file" }));
    expect(onNavigate).toHaveBeenCalledWith(0);
  });

  it("cycles files with the arrow keys", async () => {
    const { onNavigate } = renderPreview(commitTarget, { index: 0 });
    await userEvent.keyboard("{ArrowDown}");
    expect(onNavigate).toHaveBeenLastCalledWith(1);
    await userEvent.keyboard("{ArrowUp}");
    expect(onNavigate).toHaveBeenLastCalledWith(2);
  });

  it("ignores arrow keys with modifiers or in fields", async () => {
    const { onNavigate } = renderPreview(commitTarget, { index: 0 });
    await userEvent.keyboard("{Meta>}{ArrowDown}{/Meta}");
    const field = document.createElement("textarea");
    document.body.appendChild(field);
    field.focus();
    await userEvent.keyboard("{ArrowDown}");
    field.remove();
    expect(onNavigate).not.toHaveBeenCalled();
  });

  it("ignores arrow keys with a single file", async () => {
    const { onNavigate } = renderPreview(commitTarget, { index: 0, files: ["a"] });
    await userEvent.keyboard("{ArrowDown}");
    expect(onNavigate).not.toHaveBeenCalled();
  });

  it("ignores keys when keyboard is off", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const onNavigate = vi.fn();
    const onClose = vi.fn();
    render(
      <QueryClientProvider client={client}>
        <DiffPreview target={commitTarget} files={["a", "b"]} index={0} onNavigate={onNavigate} onClose={onClose} keyboard={false} />
      </QueryClientProvider>,
    );
    await userEvent.keyboard("{ArrowDown}{Escape}");
    expect(onNavigate).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("toggles between split and unified", async () => {
    renderPreview(commitTarget);
    const unified = screen.getByRole("button", { name: "Unified" });
    await userEvent.click(unified);
    expect(unified).toHaveClass("active");
    expect(screen.getByRole("button", { name: "Split" })).not.toHaveClass("active");
  });
});
