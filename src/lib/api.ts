import { invoke } from "./tracedInvoke";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { LogLevel } from "./log";
import type { CommentTag } from "./commentTag";
import type {
  ReactionContent,
  ReactionGroup,
  StackRebasePreview,
  StackRebaseResult,
  ChatWorktree,
  DefinitionHit,
  WorktreeGcReport,
  BoardCard,
  BoardOverview,
  MonitorLogEntry,
  PromptInfo,
  AiReviewLayer,
  AiRun,
  AiUsageSummary,
  UsageRange,
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
  ConversationFilters,
  ConversationList,
  IgnoreRule,
  InboxMeta,
  PrActor,
  PrLabel,
  PrMeta,
  PrChecks,
  Routine,
  RoutineContextRef,
  RoutineInput,
  RoutineOption,
  RoutineRun,
  RoutineRunEvent,
  RoutineRunFilter,
  RoutineSettings,
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
  SessionSummary,
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
  StashEntry,
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
  setPrDetailsCollapsed: (reviewId: number, collapsed: boolean) =>
    invoke<void>("set_pr_details_collapsed", { reviewId, collapsed }),
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

  // AI review (headless /local-review)
  runAiReview: (reviewId: number, layer: AiReviewLayer, model?: string) =>
    invoke<void>("run_ai_review", { reviewId, layer, model: model || null }),
  aiReviewStatus: (reviewId: number) => invoke<AiReviewLayer | null>("ai_review_status", { reviewId }),

  // Routines
  routineList: () => invoke<Routine[]>("routine_list"),
  routineSave: (routine: RoutineInput) => invoke<Routine>("routine_save", { routine }),
  routineDelete: (id: number) => invoke<void>("routine_delete", { id }),
  routineReset: (id: number) => invoke<Routine>("routine_reset", { id }),
  routineListForContext: (context: RoutineContextRef) =>
    invoke<RoutineOption[]>("routine_list_for_context", { context }),
  routineLaunch: (routineId: number, context: RoutineContextRef, note?: string, model?: string) =>
    invoke<RoutineRun>("routine_launch", { routineId, context, note: note || null, model: model || null }),
  routineRunList: (filter?: RoutineRunFilter) => invoke<RoutineRun[]>("routine_run_list", { filter: filter ?? null }),
  routineRunEvents: (runId: number, afterSeq = 0) => invoke<RoutineRunEvent[]>("routine_run_events", { runId, afterSeq }),
  routineRunStop: (runId: number) => invoke<void>("routine_run_stop", { runId }),
  routineRunRelaunch: (runId: number) => invoke<RoutineRun>("routine_run_relaunch", { runId }),
  routineMaxConcurrency: () => invoke<number>("routine_max_concurrency"),
  routineSetMaxConcurrency: (n: number) => invoke<void>("routine_set_max_concurrency", { n }),
  routineSettings: () => invoke<RoutineSettings>("routine_settings"),
  routineSetSettings: (settings: RoutineSettings) => invoke<void>("routine_set_settings", { settings }),

  // AI usage log
  aiUsageSummary: (range: UsageRange) => invoke<AiUsageSummary>("ai_usage_summary", { range }),
  aiUsageRuns: (range: UsageRange, feature: string | null, limit = 50) =>
    invoke<AiRun[]>("ai_usage_runs", { range, feature, limit }),

  // Chat
  crInstallNote: () => invoke<string | null>("cr_install_note"),
  appVersion: () => invoke<string>("app_version"),

  // Logs
  logPath: () => invoke<string>("log_path"),
  setLogLevel: (level: LogLevel) => invoke<void>("set_log_level", { level }),
  setAiEnabled: (enabled: boolean) => invoke<void>("set_ai_enabled", { enabled }),
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
  /** `hover` lookups are speculative: the backend rejects them with "cancelled" when superseded. */
  findDefinition: (
    reviewId: number,
    symbol: string,
    filePath: string,
    side: Side,
    diffFiles: string[],
    hover: boolean,
  ) =>
    invoke<DefinitionHit[]>("find_definition", { reviewId, symbol, filePath, side, diffFiles, hover }),
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
  bodyImages: (nodeId: string) =>
    invoke<Record<string, string>>("body_images", { nodeId }),
  prChecks: (owner: string, name: string, number: number) =>
    invoke<PrChecks>("pr_checks", { owner, name, number }),
  rerunCiRuns: (owner: string, name: string, runIds: number[], failedOnly: boolean) =>
    invoke<void>("rerun_ci_runs", { owner, name, runIds, failedOnly }),
  rerunCiJob: (owner: string, name: string, jobId: number) =>
    invoke<void>("rerun_ci_job", { owner, name, jobId }),
  /** The PR header as last mirrored (instant, possibly stale). */
  mirrorPrMeta: (owner: string, name: string, number: number) =>
    invoke<PrMeta | null>("mirror_pr_meta", { owner, name, number }),
  setPrDraft: (owner: string, name: string, number: number, draft: boolean) =>
    invoke<void>("set_pr_draft", { owner, name, number, draft }),
  updatePrBranch: (owner: string, name: string, number: number, rebase: boolean) =>
    invoke<void>("update_pr_branch", { owner, name, number, rebase }),
  setPrAutoMerge: (owner: string, name: string, number: number, enable: boolean) =>
    invoke<void>("set_pr_auto_merge", { owner, name, number, enable }),
  approvePr: (owner: string, name: string, number: number) =>
    invoke<void>("approve_pr", { owner, name, number }),
  setPrLabel: (owner: string, name: string, number: number, label: string, on: boolean) =>
    invoke<void>("set_pr_label", { owner, name, number, label, on }),
  setPrReviewer: (owner: string, name: string, number: number, reviewer: string, team: boolean, on: boolean) =>
    invoke<void>("set_pr_reviewer", { owner, name, number, reviewer, team, on }),
  repoLabels: (owner: string, name: string) => invoke<PrLabel[]>("repo_labels", { owner, name }),
  repoAssignableUsers: (owner: string, name: string, query: string) =>
    invoke<PrActor[]>("repo_assignable_users", { owner, name, query }),
  prReviewThreads: (owner: string, name: string, number: number) =>
    invoke<PrThread[]>("pr_review_threads", { owner, name, number }),
  /** A PR's review threads as last mirrored (instant, possibly stale). */
  mirrorPrThreads: (owner: string, name: string, number: number) =>
    invoke<PrThread[] | null>("mirror_pr_threads", { owner, name, number }),
  replyToThread: (owner: string, name: string, number: number, commentId: number, body: string) =>
    invoke<number>("reply_to_thread", { owner, name, number, commentId, body }),
  setPrThreadResolved: (threadId: string, resolved: boolean) =>
    invoke<boolean>("set_pr_thread_resolved", { threadId, resolved }),
  setCommentReaction: (subjectId: string, content: ReactionContent, on: boolean, discussionSubjectId?: string) =>
    invoke<ReactionGroup[]>("set_comment_reaction", {
      subjectId,
      content,
      on,
      discussionSubjectId: discussionSubjectId ?? null,
    }),
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
  /** Resumes the PR's open draft, else starts a review. */
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
  syncConfigure: (autoSync: AutoSync, discoveryIntervalMin: number, boardMonitor: boolean, boardMonitorIntervalMin: number) =>
    invoke<void>("sync_configure", { config: { autoSync, discoveryIntervalMin, boardMonitor, boardMonitorIntervalMin } }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  resetMirror: () => invoke<void>("reset_mirror"),

  // GitHub inbox
  listInbox: () => invoke<InboxItem[]>("list_inbox"),
  /** Archived and tracked items; `cause` is an ArchiveCause or "tracked". */
  listInboxArchive: (search: string | null, cause: string | null, limit: number) =>
    invoke<InboxItem[]>("list_inbox_archive", { search, cause, limit }),
  listConversations: (filters: ConversationFilters, botLogins: string[], rules: IgnoreRule[], limit: number) =>
    invoke<ConversationList>("list_conversations", { filters: { ...filters, botLogins, rules }, limit }),
  setConversationPinned: (id: string, subjectId: string, pinned: boolean) =>
    invoke<void>("set_conversation_pinned", { id, subjectId, pinned }),
  /** Read up to `seenAt` (a conversation's latest comment time); an earlier time marks it unread. */
  setConversationSeen: (id: string, subjectId: string, seenAt: string | null) =>
    invoke<void>("set_conversation_seen", { id, subjectId, seenAt }),
  replyToDiscussion: (subjectId: string, owner: string, name: string, number: number, body: string) =>
    invoke<void>("reply_to_discussion", { subjectId, owner, name, number, body }),
  inboxMeta: () => invoke<InboxMeta>("inbox_meta"),
  /** Mark a New item read: it stops counting in the badges and stays on the New tab. */
  markItemViewed: (id: string) => invoke<void>("mark_item_viewed", { id }),
  markItemUnread: (id: string) => invoke<void>("mark_item_unread", { id }),
  archiveItem: (id: string) => invoke<void>("archive_item", { id }),
  /** Archive for good: a new reason doesn't bring it back to New. */
  forgetItem: (id: string) => invoke<void>("forget_item", { id }),
  restoreItem: (id: string) => invoke<void>("restore_item", { id }),
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
  /** Raw model reply for a custom-theme suggestion (see `lib/themeSuggest.ts`). */
  themeSuggest: (current: unknown, scope: string, inspiration: string, feedback?: string) =>
    invoke<string>("theme_suggest", { current, scope, inspiration, feedback: feedback ?? null }),
  boardTrackedIds: () => invoke<string[]>("board_tracked_ids"),
  boardTrack: (reference: string) => invoke<string>("board_track", { reference }),
  boardTrackPr: (reference: string) => invoke<string>("board_track_pr", { reference }),
  boardDiscard: (cardId: string) => invoke<void>("board_discard", { cardId }),
  boardPrSetAutoMerge: (cardId: string, enable: boolean) =>
    invoke<void>("board_pr_set_auto_merge", { cardId, enable }),
  boardPrSquashMerge: (cardId: string) => invoke<void>("board_pr_squash_merge", { cardId }),
  boardPrSetDraft: (cardId: string, draft: boolean) => invoke<void>("board_pr_set_draft", { cardId, draft }),
  boardPrUpdateBranch: (cardId: string, rebase: boolean) =>
    invoke<void>("board_pr_update_branch", { cardId, rebase }),
  boardStackRebasePreview: (cardId: string) =>
    invoke<StackRebasePreview>("board_stack_rebase_preview", { cardId }),
  boardStackRebase: (cardId: string) => invoke<StackRebaseResult>("board_stack_rebase", { cardId }),
  boardPrRerunCi: (cardId: string, failedOnly: boolean) => invoke<number>("board_pr_rerun_ci", { cardId, failedOnly }),
  boardLinkPr: (issueId: string, prReference: string) =>
    invoke<void>("board_link_pr", { issueId, prReference }),
  boardUnlink: (issueId: string, prId: string) => invoke<void>("board_unlink", { issueId, prId }),
  boardArchiveDone: (olderThanDays: number) =>
    invoke<number>("board_archive_done", { olderThanDays }),
  boardMonitorLog: (limit: number) => invoke<MonitorLogEntry[]>("board_monitor_log", { limit }),
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
  boardOpenIssueAgent: (cardId: string, host: TerminalHost, tmuxTarget: TmuxTarget, tmuxSession: string) =>
    invoke<void>("board_open_issue_agent", { cardId, host, tmuxTarget, tmuxSession }),
  reviewAgentSessions: (reviewId: number) => invoke<ClaudeSession[]>("review_agent_sessions", { reviewId }),
  reviewOpenAgent: (reviewId: number, host: TerminalHost, tmuxTarget: TmuxTarget, tmuxSession: string) =>
    invoke<void>("review_open_agent", { reviewId, host, tmuxTarget, tmuxSession }),
  listClaudeSessions: (historyDays: number, summaryDir: string) =>
    invoke<SessionsSnapshot>("list_claude_sessions", { historyDays, summaryDir }),
  generateLiveSessionSummary: (sessionId: string, fork: boolean) =>
    invoke<SessionSummary>("generate_live_session_summary", { sessionId, fork }),
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
  openInVscode: (path: string, file?: string, line?: number) =>
    invoke<void>("open_in_vscode", { path, file: file ?? null, line: line ?? null }),

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
  checkoutOpenClaude: (
    repoId: number,
    worktreePath: string | null,
    host: TerminalHost,
    tmuxTarget: TmuxTarget,
    tmuxSession: string,
  ) => invoke<void>("checkout_open_claude", { repoId, worktreePath, host, tmuxTarget, tmuxSession }),

  // Graph mutations (spec 24); every call runs in the context checkout (`worktreePath`, null = main).
  gitCheckoutBranch: (repoId: number, worktreePath: string | null, name: string) =>
    invoke<void>("git_checkout_branch", { repoId, worktreePath, name }),
  gitCheckoutRemoteBranch: (repoId: number, worktreePath: string | null, remoteRef: string) =>
    invoke<void>("git_checkout_remote_branch", { repoId, worktreePath, remoteRef }),
  gitCheckoutDetached: (repoId: number, worktreePath: string | null, rev: string) =>
    invoke<void>("git_checkout_detached", { repoId, worktreePath, rev }),
  gitCreateBranch: (repoId: number, worktreePath: string | null, name: string, at: string, switchTo: boolean) =>
    invoke<void>("git_create_branch", { repoId, worktreePath, name, at, switch: switchTo }),
  gitDeleteBranch: (repoId: number, worktreePath: string | null, name: string, force: boolean) =>
    invoke<void>("git_delete_branch", { repoId, worktreePath, name, force }),
  gitRenameBranch: (repoId: number, worktreePath: string | null, old: string, newName: string) =>
    invoke<void>("git_rename_branch", { repoId, worktreePath, old, new: newName }),
  gitCreateTag: (
    repoId: number,
    worktreePath: string | null,
    name: string,
    at: string,
    message: string | null,
    push: boolean,
  ) => invoke<void>("git_create_tag", { repoId, worktreePath, name, at, message, push }),
  gitDeleteTag: (repoId: number, worktreePath: string | null, name: string) =>
    invoke<void>("git_delete_tag", { repoId, worktreePath, name }),
  gitStage: (repoId: number, worktreePath: string | null, paths: string[]) =>
    invoke<void>("git_stage", { repoId, worktreePath, paths }),
  gitUnstage: (repoId: number, worktreePath: string | null, paths: string[]) =>
    invoke<void>("git_unstage", { repoId, worktreePath, paths }),
  gitDiscard: (repoId: number, worktreePath: string | null, paths: string[], kind: WipKind) =>
    invoke<void>("git_discard", { repoId, worktreePath, paths, kind }),
  gitCommit: (repoId: number, worktreePath: string | null, message: string, amend: boolean) =>
    invoke<void>("git_commit", { repoId, worktreePath, message, amend }),
  gitPull: (repoId: number, worktreePath: string | null) =>
    invoke<string>("git_pull", { repoId, worktreePath }),
  gitPush: (repoId: number, worktreePath: string | null, forceWithLease: boolean) =>
    invoke<string>("git_push", { repoId, worktreePath, forceWithLease }),
  gitStashPush: (
    repoId: number,
    worktreePath: string | null,
    message: string | null,
    includeUntracked: boolean,
  ) => invoke<void>("git_stash_push", { repoId, worktreePath, message, includeUntracked }),
  gitStashList: (repoId: number, worktreePath: string | null) =>
    invoke<StashEntry[]>("git_stash_list", { repoId, worktreePath }),
  gitStashApply: (repoId: number, worktreePath: string | null, index: number) =>
    invoke<void>("git_stash_apply", { repoId, worktreePath, index }),
  gitStashPop: (repoId: number, worktreePath: string | null, index: number) =>
    invoke<void>("git_stash_pop", { repoId, worktreePath, index }),
  gitStashDrop: (repoId: number, worktreePath: string | null, index: number) =>
    invoke<void>("git_stash_drop", { repoId, worktreePath, index }),
  gitStashDiff: (repoId: number, worktreePath: string | null, index: number) =>
    invoke<string>("git_stash_diff", { repoId, worktreePath, index }),

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
  setCommentTag: (commentId: number, tag: CommentTag | null) =>
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
