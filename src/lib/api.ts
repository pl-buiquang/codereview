import { invoke } from "./tracedInvoke";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { LogLevel } from "./log";
import type {
  ChatWorktree,
  WorktreeGcReport,
  BoardCard,
  BoardOverview,
  PromptInfo,
  BoardMeta,
  BoardMetaKind,
  Branch,
  SessionsSnapshot,
  ChatMessage,
  ChatTurnResult,
  ClaudeSession,
  Comment,
  CommitDetail,
  DeleteGoneOutcome,
  FocusLevel,
  FocusOverride,
  FocusView,
  FetchResult,
  FreshnessResult,
  GraphCommit,
  GraphScope,
  InboxItem,
  InboxMeta,
  PrMeta,
  PrSummary,
  PrThread,
  PublishDraftRepliesResult,
  RefInfo,
  ReanchorResult,
  RepoSyncState,
  Repository,
  Review,
  ReviewDetail,
  ReviewEvent,
  ReviewSummary,
  Side,
  SyncMode,
  SyncReport,
  SyncScope,
  SyncStatus,
  AutoSync,
  ToolEnv,
  UpdateReviewResult,
  WipKind,
  WipStatus,
  WorktreeInfo,
} from "./types";
import type { TerminalHost, TmuxTarget } from "./settings";

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
  listTargetReviews: (targetId: number) =>
    invoke<ReviewSummary[]>("list_target_reviews", { targetId }),
  newReviewOnTarget: (targetId: number) =>
    invoke<Review>("new_review_on_target", { targetId }),
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

  // Focus
  getFocus: (reviewId: number) => invoke<FocusView | null>("get_focus", { reviewId }),
  setFocusLevel: (reviewId: number, level: FocusLevel) =>
    invoke<void>("set_focus_level", { reviewId, level }),
  setHunkOverride: (reviewId: number, hunkId: string, value: FocusOverride | null) =>
    invoke<void>("set_hunk_override", { reviewId, hunkId, value }),
  setFileOverride: (reviewId: number, filePath: string, value: FocusOverride | null) =>
    invoke<void>("set_file_override", { reviewId, filePath, value }),
  clearFocus: (reviewId: number) => invoke<void>("clear_focus", { reviewId }),
  generateFocus: (reviewId: number, model?: string) =>
    invoke<void>("generate_focus", { reviewId, model: model || null }),

  // Chat
  crInstallNote: () => invoke<string | null>("cr_install_note"),
  appVersion: () => invoke<string>("app_version"),

  // Logs
  logPath: () => invoke<string>("log_path"),
  setLogLevel: (level: LogLevel) => invoke<void>("set_log_level", { level }),
  readAppLog: (maxLines: number) => invoke<string>("read_app_log", { maxLines }),

  chatSend: (reviewId: number, text: string, model?: string) =>
    invoke<ChatTurnResult>("chat_send", { reviewId, text, model: model || null }),
  chatMessages: (reviewId: number) =>
    invoke<ChatMessage[]>("chat_messages", { reviewId }),
  chatClear: (reviewId: number) =>
    invoke<void>("chat_clear", { reviewId }),
  chatWorktree: (reviewId: number) =>
    invoke<ChatWorktree | null>("chat_worktree", { reviewId }),
  ensureChatWorktree: (reviewId: number) =>
    invoke<string>("ensure_chat_worktree", { reviewId }),
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
  findClosedPrReviews: () => invoke<number[]>("find_closed_pr_reviews"),
  deleteReviews: (reviewIds: number[]) =>
    invoke<number>("delete_reviews", { reviewIds }),
  refreshReview: (reviewId: number) =>
    invoke<FreshnessResult>("refresh_review", { reviewId }),
  reanchorComments: (reviewId: number) =>
    invoke<ReanchorResult>("reanchor_comments", { reviewId }),
  updateReviewPin: (reviewId: number) =>
    invoke<UpdateReviewResult>("update_review_pin", { reviewId }),

  // GitHub
  ghAuthStatus: () => invoke<boolean>("gh_auth_status"),
  checkEnvironment: () => invoke<ToolEnv>("check_environment"),
  listPrs: (repoId: number) => invoke<PrSummary[]>("list_prs", { repoId }),
  prMeta: (owner: string, name: string, number: number) =>
    invoke<PrMeta>("pr_meta", { owner, name, number }),
  /** The PR header as last mirrored (instant, possibly stale). */
  mirrorPrMeta: (owner: string, name: string, number: number) =>
    invoke<PrMeta | null>("mirror_pr_meta", { owner, name, number }),
  setPrDraft: (owner: string, name: string, number: number, draft: boolean) =>
    invoke<void>("set_pr_draft", { owner, name, number, draft }),
  updatePrBranch: (owner: string, name: string, number: number, rebase: boolean) =>
    invoke<void>("update_pr_branch", { owner, name, number, rebase }),
  setPrAutoMerge: (owner: string, name: string, number: number, enable: boolean) =>
    invoke<void>("set_pr_auto_merge", { owner, name, number, enable }),
  prReviewThreads: (owner: string, name: string, number: number) =>
    invoke<PrThread[]>("pr_review_threads", { owner, name, number }),
  /** A PR's review threads as last mirrored (instant, possibly stale). */
  mirrorPrThreads: (owner: string, name: string, number: number) =>
    invoke<PrThread[] | null>("mirror_pr_threads", { owner, name, number }),
  replyToThread: (owner: string, name: string, number: number, commentId: number, body: string) =>
    invoke<number>("reply_to_thread", { owner, name, number, commentId, body }),
  setPrThreadResolved: (threadId: string, resolved: boolean) =>
    invoke<boolean>("set_pr_thread_resolved", { threadId, resolved }),
  addDraftReply: (args: {
    reviewId: number;
    githubThreadRootId: number;
    body: string;
    filePath: string;
    side: Side;
    line: number;
    startLine?: number | null;
  }) => invoke<Comment>("add_draft_reply", args),
  updateDraftReply: (commentId: number, body: string) =>
    invoke<void>("update_draft_reply", { commentId, body }),
  deleteDraftReply: (commentId: number) =>
    invoke<void>("delete_draft_reply", { commentId }),
  publishDraftReply: (commentId: number) =>
    invoke<void>("publish_draft_reply", { commentId }),
  publishDraftReplies: (reviewId: number) =>
    invoke<PublishDraftRepliesResult>("publish_draft_replies", { reviewId }),
  createReviewForPr: (owner: string, name: string, prNumber: number, basePaths: string[] = []) =>
    invoke<Review>("create_review_for_pr", { owner, name, prNumber, basePaths }),
  publishReview: (reviewId: number) => invoke<Review>("publish_review", { reviewId }),
  publishReviewPending: (reviewId: number) =>
    invoke<Review>("publish_review_pending", { reviewId }),
  submitPendingReview: (reviewId: number) =>
    invoke<Review>("submit_pending_review", { reviewId }),
  discardPendingReview: (reviewId: number) =>
    invoke<Review>("discard_pending_review", { reviewId }),

  // Sync engine
  syncHint: (scope: SyncScope, active: boolean, visibleIds: string[] = []) =>
    invoke<void>("sync_hint", { scope, active, visibleIds }),
  syncNow: (scope: SyncScope, mode: SyncMode, visibleIds: string[] = []) =>
    invoke<SyncReport>("sync_now", { scope, mode, visibleIds }),
  syncWindowState: (visible: boolean, focused: boolean) =>
    invoke<void>("sync_window_state", { visible, focused }),
  syncConfigure: (autoSync: AutoSync, discoveryIntervalMin: number) =>
    invoke<void>("sync_configure", { config: { autoSync, discoveryIntervalMin } }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  resetMirror: () => invoke<void>("reset_mirror"),

  // GitHub inbox
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
  setRepositoryFriendlyName: (repoId: number, friendlyName: string | null) =>
    invoke<Repository>("set_repository_friendly_name", { repoId, friendlyName }),

  // Kanban board
  boardLastRefreshAt: () => invoke<string | null>("board_last_refresh_at"),
  boardList: () => invoke<BoardCard[]>("board_list"),
  boardOverviewGet: () => invoke<BoardOverview | null>("board_overview_get"),
  boardOverviewGenerate: (model: string) =>
    invoke<BoardOverview>("board_overview_generate", { model: model || null }),

  // AI prompts
  promptsList: () => invoke<PromptInfo[]>("prompts_list"),
  promptSet: (key: string, body: string) => invoke<void>("prompt_set", { key, body }),
  promptReset: (key: string) => invoke<void>("prompt_reset", { key }),
  boardTrackedIds: () => invoke<string[]>("board_tracked_ids"),
  boardTrack: (reference: string) => invoke<string>("board_track", { reference }),
  boardTrackPr: (reference: string) => invoke<string>("board_track_pr", { reference }),
  boardDiscard: (cardId: string) => invoke<void>("board_discard", { cardId }),
  boardPrSetAutoMerge: (cardId: string, enable: boolean) =>
    invoke<void>("board_pr_set_auto_merge", { cardId, enable }),
  boardPrSetDraft: (cardId: string, draft: boolean) => invoke<void>("board_pr_set_draft", { cardId, draft }),
  boardPrUpdateBranch: (cardId: string, rebase: boolean) =>
    invoke<void>("board_pr_update_branch", { cardId, rebase }),
  boardLinkPr: (issueId: string, prReference: string) =>
    invoke<void>("board_link_pr", { issueId, prReference }),
  boardUnlink: (issueId: string, prId: string) => invoke<void>("board_unlink", { issueId, prId }),
  boardArchiveDone: (olderThanDays: number) =>
    invoke<number>("board_archive_done", { olderThanDays }),
  boardOpenReview: (cardId: string, basePaths: string[] = []) =>
    invoke<Review>("board_open_review", { cardId, basePaths }),
  boardOpenVscode: (cardId: string, basePaths: string[] = []) =>
    invoke<void>("board_open_vscode", { cardId, basePaths }),
  boardOpenShell: (
    cardId: string,
    host: TerminalHost,
    tmuxTarget: TmuxTarget,
    tmuxSession: string,
    basePaths: string[] = [],
  ) => invoke<void>("board_open_shell", { cardId, basePaths, host, tmuxTarget, tmuxSession }),
  boardOpenAgent: (
    cardId: string,
    host: TerminalHost,
    tmuxTarget: TmuxTarget,
    tmuxSession: string,
    basePaths: string[] = [],
  ) => invoke<void>("board_open_agent", { cardId, basePaths, host, tmuxTarget, tmuxSession }),
  reviewAgentSessions: (reviewId: number) => invoke<ClaudeSession[]>("review_agent_sessions", { reviewId }),
  reviewOpenAgent: (reviewId: number, host: TerminalHost, tmuxTarget: TmuxTarget, tmuxSession: string) =>
    invoke<void>("review_open_agent", { reviewId, host, tmuxTarget, tmuxSession }),
  listClaudeSessions: (historyDays: number, summaryDir: string) =>
    invoke<SessionsSnapshot>("list_claude_sessions", { historyDays, summaryDir }),
  focusClaudeSession: (sessionId: string) => invoke<void>("focus_claude_session", { sessionId }),
  resumeClaudeSession: (
    sessionId: string,
    cwd: string,
    repo: string | null,
    host: TerminalHost,
    tmuxTarget: TmuxTarget,
    tmuxSession: string,
  ) => invoke<void>("resume_claude_session", { sessionId, cwd, repo, host, tmuxTarget, tmuxSession }),
  stopClaudeSession: (sessionId: string) => invoke<void>("stop_claude_session", { sessionId }),
  boardMetaAdd: (args: {
    cardId: string;
    kind: BoardMetaKind;
    value: string;
    tag?: string | null;
    label?: string | null;
    cwd?: string | null;
  }) => invoke<BoardMeta>("board_meta_add", { ...args, tag: args.tag ?? null, label: args.label ?? null, cwd: args.cwd ?? null }),
  boardMetaUpdate: (id: number, args: { value?: string | null; tag?: string | null; label?: string | null }) =>
    invoke<void>("board_meta_update", {
      id,
      value: args.value ?? null,
      tag: args.tag ?? null,
      label: args.label ?? null,
    }),
  boardMetaDelete: (id: number) => invoke<void>("board_meta_delete", { id }),
  boardMetaTags: () => invoke<string[]>("board_meta_tags"),
  boardCardForPr: (owner: string, name: string, number: number) =>
    invoke<BoardCard | null>("board_card_for_pr", { owner, name, number }),

  // Worktrees
  listWorktrees: (repoId: number) =>
    invoke<WorktreeInfo[]>("list_worktrees", { repoId }),
  worktreeGcRun: (remove: boolean, sync: boolean) =>
    invoke<WorktreeGcReport>("worktree_gc_run", { remove, sync }),
  worktreeGcLast: () => invoke<WorktreeGcReport | null>("worktree_gc_last"),
  worktreeGcReload: () => invoke<WorktreeGcReport | null>("worktree_gc_reload"),
  worktreeSync: (repoId: number, worktreePath: string) =>
    invoke<string>("worktree_sync", { repoId, worktreePath }),
  worktreeOpenTerminal: (
    repoId: number,
    worktreePath: string,
    host: TerminalHost,
    tmuxTarget: TmuxTarget,
    tmuxSession: string,
  ) => invoke<void>("worktree_open_terminal", { repoId, worktreePath, host, tmuxTarget, tmuxSession }),
  removeWorktree: (repoId: number, worktreePath: string) =>
    invoke<void>("remove_worktree", { repoId, worktreePath }),
  pruneWorktrees: (repoId: number) =>
    invoke<string>("prune_worktrees", { repoId }),
  openInVscode: (path: string, file?: string) =>
    invoke<void>("open_in_vscode", { path, file: file ?? null }),

  // Git graph
  graphLog: (args: {
    repoId: number;
    scope: GraphScope;
    worktreePath?: string | null;
    skip: number;
    limit: number;
  }) =>
    invoke<GraphCommit[]>("graph_log", {
      ...args,
      worktreePath: args.worktreePath ?? null,
    }),
  listRefs: (repoId: number, worktreePath?: string | null) =>
    invoke<RefInfo[]>("list_refs", { repoId, worktreePath: worktreePath ?? null }),
  listGoneBranches: (repoId: number) => invoke<string[]>("list_gone_branches", { repoId }),
  fetchRepo: (repoId: number, minIntervalSecs?: number) =>
    invoke<FetchResult>("fetch_repo", { repoId, minIntervalSecs: minIntervalSecs ?? null }),
  repoSyncState: (repoId: number) => invoke<RepoSyncState>("repo_sync_state", { repoId }),
  watchAcquire: (repoId: number) => invoke<boolean>("watch_acquire", { repoId }),
  watchRelease: (repoId: number) => invoke<void>("watch_release", { repoId }),
  deleteGoneBranches: (repoId: number, names: string[]) =>
    invoke<DeleteGoneOutcome>("delete_gone_branches", { repoId, names }),
  commitAvatar: (repoId: number, email: string, sha: string) =>
    invoke<string | null>("commit_avatar", { repoId, email, sha }),
  commitDetail: (repoId: number, sha: string) =>
    invoke<CommitDetail>("commit_detail", { repoId, sha }),
  commitFileDiff: (args: {
    repoId: number;
    sha: string;
    path: string;
    oldPath?: string | null;
  }) =>
    invoke<string>("commit_file_diff", { ...args, oldPath: args.oldPath ?? null }),
  worktreeStatus: (repoId: number, worktreePath?: string | null) =>
    invoke<WipStatus>("worktree_status", { repoId, worktreePath: worktreePath ?? null }),
  worktreeFileDiff: (args: {
    repoId: number;
    worktreePath?: string | null;
    path: string;
    kind: WipKind;
  }) =>
    invoke<string>("worktree_file_diff", {
      ...args,
      worktreePath: args.worktreePath ?? null,
    }),

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
  setCommentTag: (commentId: number, tag: "ai" | "local" | null) =>
    invoke<void>("set_comment_tag", { commentId, tag }),
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

  // Settings backup
  writeSettingsFile: (destPath: string, contents: string) =>
    invoke<void>("write_settings_file", { destPath, contents }),
  readSettingsFile: (srcPath: string) =>
    invoke<string>("read_settings_file", { srcPath }),
  readSettingsMirror: () => invoke<string | null>("read_settings_mirror"),
  writeSettingsMirror: (contents: string) => invoke<void>("write_settings_mirror", { contents }),
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

/** Open a native file picker (any file); returns the chosen path or null. */
export async function pickFile(): Promise<string | null> {
  const selected = await open({ multiple: false });
  return typeof selected === "string" ? selected : null;
}
