import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock the Tauri IPC layer so the wrappers can be asserted without a backend.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const save = vi.fn();
const open = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (...a: unknown[]) => save(...a),
  open: (...a: unknown[]) => open(...a),
}));

import { api, pickSavePath, pickFolder } from "./api";

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  save.mockReset();
  open.mockReset();
});

describe("api command wrappers", () => {
  it("forwards simple repo commands with correct names and args", async () => {
    await api.listRepositories();
    expect(invoke).toHaveBeenCalledWith("list_repositories");

    await api.addRepository("/some/path");
    expect(invoke).toHaveBeenCalledWith("add_repository", { path: "/some/path" });

    await api.removeRepository(7);
    expect(invoke).toHaveBeenCalledWith("remove_repository", { id: 7 });

    await api.listBranches("/repo");
    expect(invoke).toHaveBeenCalledWith("list_branches", { repoPath: "/repo" });

    await api.diffRefs("/repo", "main", "feature", true);
    expect(invoke).toHaveBeenCalledWith("diff_refs", {
      repoPath: "/repo",
      base: "main",
      head: "feature",
      threeDot: true,
    });
  });

  it("passes review args objects straight through", async () => {
    const args = {
      repoId: 1,
      repoPath: "/repo",
      baseRef: "main",
      headRef: "feat",
      threeDot: false,
    };
    await api.createReview(args);
    expect(invoke).toHaveBeenCalledWith("create_review", args);

    await api.listReviews(null);
    expect(invoke).toHaveBeenCalledWith("list_reviews", { repoId: null });

    await api.getReview(3);
    expect(invoke).toHaveBeenCalledWith("get_review", { reviewId: 3 });

    await api.updateReview(3, "body text", "approve");
    expect(invoke).toHaveBeenCalledWith("update_review", {
      reviewId: 3,
      body: "body text",
      event: "approve",
    });
  });

  it("wraps GitHub and comment commands", async () => {
    await api.ghAuthStatus();
    expect(invoke).toHaveBeenCalledWith("gh_auth_status");

    await api.checkEnvironment();
    expect(invoke).toHaveBeenCalledWith("check_environment");

    await api.listPrs("/repo");
    expect(invoke).toHaveBeenCalledWith("list_prs", { repoPath: "/repo" });

    await api.createReviewForPr("acme", "widget", 42);
    expect(invoke).toHaveBeenCalledWith("create_review_for_pr", {
      owner: "acme",
      name: "widget",
      prNumber: 42,
    });

    await api.publishReview(9);
    expect(invoke).toHaveBeenCalledWith("publish_review", { reviewId: 9 });

    const commentArgs = {
      reviewId: 1,
      filePath: "a.ts",
      side: "RIGHT" as const,
      line: 5,
      body: "note",
    };
    await api.addComment(commentArgs);
    expect(invoke).toHaveBeenCalledWith("add_comment", commentArgs);

    const replyArgs = { ...commentArgs, parentId: 7 };
    await api.addComment(replyArgs);
    expect(invoke).toHaveBeenCalledWith("add_comment", replyArgs);

    await api.addReply({ reviewId: 1, parentId: 7, body: "a reply" });
    expect(invoke).toHaveBeenCalledWith("add_comment", {
      reviewId: 1,
      parentId: 7,
      body: "a reply",
      filePath: "",
      side: "RIGHT",
      line: 0,
    });

    await api.updateComment(2, "edited");
    expect(invoke).toHaveBeenCalledWith("update_comment", { commentId: 2, body: "edited" });

    await api.deleteComment(2);
    expect(invoke).toHaveBeenCalledWith("delete_comment", { commentId: 2 });
  });

  it("wraps export commands", async () => {
    await api.previewReview(1, "markdown");
    expect(invoke).toHaveBeenCalledWith("preview_review", { reviewId: 1, format: "markdown" });

    await api.exportReview(1, "/out.md", "markdown");
    expect(invoke).toHaveBeenCalledWith("export_review", {
      reviewId: 1,
      destPath: "/out.md",
      format: "markdown",
    });
  });

  it("returns the value resolved by invoke", async () => {
    invoke.mockResolvedValueOnce([{ id: 1 }]);
    await expect(api.listRepositories()).resolves.toEqual([{ id: 1 }]);
  });
});

describe("native dialog helpers", () => {
  it("pickSavePath returns the chosen path", async () => {
    save.mockResolvedValueOnce("/chosen/file.md");
    await expect(pickSavePath("review.md", "md")).resolves.toBe("/chosen/file.md");
    expect(save).toHaveBeenCalledWith({
      defaultPath: "review.md",
      filters: [{ name: "MD", extensions: ["md"] }],
    });
  });

  it("pickSavePath returns null when cancelled", async () => {
    save.mockResolvedValueOnce(null);
    await expect(pickSavePath("review.md", "md")).resolves.toBeNull();
  });

  it("pickFolder returns a string path or null", async () => {
    open.mockResolvedValueOnce("/picked/dir");
    await expect(pickFolder()).resolves.toBe("/picked/dir");

    open.mockResolvedValueOnce(null);
    await expect(pickFolder()).resolves.toBeNull();
  });
});
