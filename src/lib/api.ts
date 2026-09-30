import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  Branch,
  ChatMessage,
  ChatTurnResult,
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
  WorktreeInfo,
} from "./types";

export const api = {
  listRepositories: () => invoke<Repository[]>("list_repositories"),
  addRepository: (path: string) => invoke<Repository>("add_repository", { path }),
  removeRepository: (id: number) => invoke<void>("remove_repository", { id }),
  listBranches: (repoId: number) =>
    invoke<Branch[]>("list_branches", { repoId }),
  diffRefs: (repoId: number, base: string, head: string, threeDot: boolean) =>
    invoke<string>("diff_refs", { repoId, base, head, threeDot }),

  // Reviews
  createReview: (args: {
    repoId: number;
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
  setSidebarCollapsed: (reviewId: number, collapsed: boolean) =>
    invoke<void>("set_sidebar_collapsed", { reviewId, collapsed }),
  setChatCollapsed: (reviewId: number, collapsed: boolean) =>
    invoke<void>("set_chat_collapsed", { reviewId, collapsed }),
  setPanelWidths: (reviewId: number, leftPanelWidth: number, rightPanelWidth: number) =>
    invoke<void>("set_panel_widths", { reviewId, leftPanelWidth, rightPanelWidth }),

  // Chat
  crInstallNote: () => invoke<string | null>("cr_install_note"),
  appVersion: () => invoke<string>("app_version"),

  chatSend: (reviewId: number, text: string, model?: string, mode?: string) =>
    invoke<ChatTurnResult>("chat_send", { reviewId, text, model: model || null, mode: mode || null }),
  chatMessages: (reviewId: number) =>
    invoke<ChatMessage[]>("chat_messages", { reviewId }),
  chatClear: (reviewId: number) =>
    invoke<void>("chat_clear", { reviewId }),
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
  listPrs: (repoId: number) => invoke<PrSummary[]>("list_prs", { repoId }),
  prMeta: (owner: string, name: string, number: number) =>
    invoke<PrMeta>("pr_meta", { owner, name, number }),
  prReviewThreads: (owner: string, name: string, number: number) =>
    invoke<PrThread[]>("pr_review_threads", { owner, name, number }),
  replyToThread: (owner: string, name: string, number: number, commentId: number, body: string) =>
    invoke<number>("reply_to_thread", { owner, name, number, commentId, body }),
  setPrThreadResolved: (threadId: string, resolved: boolean) =>
    invoke<boolean>("set_pr_thread_resolved", { threadId, resolved }),
  createReviewForPr: (owner: string, name: string, prNumber: number, basePaths: string[] = []) =>
    invoke<Review>("create_review_for_pr", { owner, name, prNumber, basePaths }),
  publishReview: (reviewId: number) => invoke<Review>("publish_review", { reviewId }),
  publishReviewPending: (reviewId: number) =>
    invoke<Review>("publish_review_pending", { reviewId }),
  submitPendingReview: (reviewId: number) =>
    invoke<Review>("submit_pending_review", { reviewId }),
  discardPendingReview: (reviewId: number) =>
    invoke<Review>("discard_pending_review", { reviewId }),

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
  openPrReview: (itemId: string, owner: string, name: string, number: number, basePaths: string[] = []) =>
    invoke<Review>("open_pr_review", { itemId, owner, name, number, basePaths }),
  linkLocalPath: (repoId: number, path: string) =>
    invoke<Repository>("link_local_path", { repoId, path }),
  autoLinkRepos: (basePaths: string[]) =>
    invoke<number>("auto_link_repos", { basePaths }),

  // Worktrees
  listWorktrees: (repoId: number) =>
    invoke<WorktreeInfo[]>("list_worktrees", { repoId }),
  removeWorktree: (repoId: number, worktreePath: string) =>
    invoke<void>("remove_worktree", { repoId, worktreePath }),
  pruneWorktrees: (repoId: number) =>
    invoke<string>("prune_worktrees", { repoId }),
  openInVscode: (path: string) =>
    invoke<void>("open_in_vscode", { path }),

  // Images
  fetchGithubImage: (url: string) =>
    invoke<string>("fetch_github_image", { url }),

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
  exportVscodeReview: (reviewId: number) =>
    invoke<string>("export_vscode_review", { reviewId }),

  // Import
  importReview: (srcPath: string) =>
    invoke<Review>("import_review", { srcPath }),
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

/** Open a native file picker for JSON files; returns the chosen path or null. */
export async function pickJsonFile(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    filters: [{ name: "JSON", extensions: ["json"] }],
  });
  return typeof selected === "string" ? selected : null;
}
