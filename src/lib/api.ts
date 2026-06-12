import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  Branch,
  Comment,
  FreshnessResult,
  InboxItem,
  InboxMeta,
  PrMeta,
  PrSummary,
  PrThread,
  ReanchorResult,
  RefreshResult,
  Repository,
  Review,
  ReviewDetail,
  ReviewEvent,
  ReviewSummary,
  Side,
  ToolEnv,
} from "./types";

export const api = {
  listRepositories: () => invoke<Repository[]>("list_repositories"),
  addRepository: (path: string) => invoke<Repository>("add_repository", { path }),
  removeRepository: (id: number) => invoke<void>("remove_repository", { id }),
  listBranches: (repoPath: string) =>
    invoke<Branch[]>("list_branches", { repoPath }),
  diffRefs: (repoPath: string, base: string, head: string, threeDot: boolean) =>
    invoke<string>("diff_refs", { repoPath, base, head, threeDot }),

  // Reviews
  createReview: (args: {
    repoId: number;
    repoPath: string;
    baseRef: string;
    headRef: string;
    threeDot: boolean;
  }) => invoke<Review>("create_review", args),
  listReviews: (repoId: number | null) =>
    invoke<ReviewSummary[]>("list_reviews", { repoId }),
  getReview: (reviewId: number) =>
    invoke<ReviewDetail>("get_review", { reviewId }),
  setFileViewed: (reviewId: number, filePath: string, viewed: boolean) =>
    invoke<void>("set_file_viewed", { reviewId, filePath, viewed }),
  reviewDiff: (reviewId: number) => invoke<string>("review_diff", { reviewId }),
  fileSource: (reviewId: number, filePath: string, side: Side) =>
    invoke<string>("file_source", { reviewId, filePath, side }),
  openInDefaultApp: (path: string) =>
    invoke<void>("open_in_default_app", { path }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  updateReview: (reviewId: number, body?: string, event?: string) =>
    invoke<void>("update_review", { reviewId, body, event }),
  deleteReview: (reviewId: number) =>
    invoke<void>("delete_review", { reviewId }),
  refreshReview: (reviewId: number) =>
    invoke<FreshnessResult>("refresh_review", { reviewId }),
  reanchorComments: (reviewId: number) =>
    invoke<ReanchorResult>("reanchor_comments", { reviewId }),

  // GitHub
  ghAuthStatus: () => invoke<boolean>("gh_auth_status"),
  checkEnvironment: () => invoke<ToolEnv>("check_environment"),
  listPrs: (repoPath: string) => invoke<PrSummary[]>("list_prs", { repoPath }),
  prMeta: (owner: string, name: string, number: number) =>
    invoke<PrMeta>("pr_meta", { owner, name, number }),
  prReviewThreads: (owner: string, name: string, number: number) =>
    invoke<PrThread[]>("pr_review_threads", { owner, name, number }),
  replyToThread: (owner: string, name: string, number: number, commentId: number, body: string) =>
    invoke<number>("reply_to_thread", { owner, name, number, commentId, body }),
  setPrThreadResolved: (threadId: string, resolved: boolean) =>
    invoke<boolean>("set_pr_thread_resolved", { threadId, resolved }),
  createReviewForPr: (owner: string, name: string, prNumber: number) =>
    invoke<Review>("create_review_for_pr", { owner, name, prNumber }),
  publishReview: (reviewId: number) => invoke<Review>("publish_review", { reviewId }),

  // GitHub inbox
  refreshInbox: () => invoke<RefreshResult>("refresh_inbox"),
  listInbox: () => invoke<InboxItem[]>("list_inbox"),
  listArchive: (search: string | null) =>
    invoke<InboxItem[]>("list_archive", { search }),
  listClosed: () => invoke<InboxItem[]>("list_closed"),
  inboxMeta: () => invoke<InboxMeta>("inbox_meta"),
  engageItem: (id: string) => invoke<void>("engage_item", { id }),
  unengageItem: (id: string) => invoke<void>("unengage_item", { id }),
  untrackItem: (id: string) => invoke<void>("untrack_item", { id }),
  retrackItem: (id: string) => invoke<void>("retrack_item", { id }),
  openPrReview: (itemId: string, owner: string, name: string, number: number) =>
    invoke<Review>("open_pr_review", { itemId, owner, name, number }),

  // Comments
  addComment: (args: {
    reviewId: number;
    filePath: string;
    side: Side;
    line: number;
    startLine?: number | null;
    diffHunk?: string | null;
    body: string;
    anchoredHeadSha?: string | null;
    parentId?: number | null;
  }) => invoke<Comment>("add_comment", args),

  /** Reply to a root comment. Anchor args are placeholders — the backend copies
   *  the parent's anchor columns and ignores these. */
  addReply: (args: { reviewId: number; parentId: number; body: string }) =>
    invoke<Comment>("add_comment", {
      ...args,
      filePath: "",
      side: "RIGHT" as Side,
      line: 0,
    }),
  addFileComment: (args: { reviewId: number; filePath: string; body: string }) =>
    invoke<Comment>("add_file_comment", args),
  addFileViewComment: (args: {
    reviewId: number;
    filePath: string;
    line: number;
    startLine?: number | null;
    body: string;
    anchoredHeadSha?: string | null;
  }) => invoke<Comment>("add_file_view_comment", args),
  updateComment: (commentId: number, body: string) =>
    invoke<void>("update_comment", { commentId, body }),
  deleteComment: (commentId: number) =>
    invoke<void>("delete_comment", { commentId }),
  setCommentResolved: (commentId: number, resolved: boolean) =>
    invoke<void>("set_comment_resolved", { commentId, resolved }),

  // Export
  previewReview: (reviewId: number, format: "markdown" | "json") =>
    invoke<string>("preview_review", { reviewId, format }),
  exportReview: (reviewId: number, destPath: string, format: "markdown" | "json") =>
    invoke<void>("export_review", { reviewId, destPath, format }),
};

/** Native save dialog; returns chosen path or null. */
export async function pickSavePath(
  defaultName: string,
  ext: string,
): Promise<string | null> {
  const selected = await save({
    defaultPath: defaultName,
    filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
  });
  return selected ?? null;
}

export type { ReviewEvent };

/** Open a native folder picker; returns the chosen absolute path or null. */
export async function pickFolder(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}
