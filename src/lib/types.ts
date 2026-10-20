import type { CommentTag } from "./commentTag";

export interface Repository {
  id: number;
  local_path: string | null;
  remote_owner: string | null;
  remote_name: string | null;
  default_branch: string | null;
  added_at: string;
  friendly_name: string | null;
  /** NULL for a repo the GitHub mirror auto-added (never listed). */
  registered_at: string | null;
}

export interface Branch {
  name: string;
  is_remote: boolean;
  sha: string;
}

export type ReviewEvent = "comment" | "approve" | "request_changes";
export type Side = "LEFT" | "RIGHT";
export type AiReviewLayer = "explain" | "critique" | "codex";
export interface AiReviewUpdated {
  review_id: number;
  layer: AiReviewLayer;
  status: "running" | "done" | "failed";
  error: string | null;
  summary: string | null;
}

export type UsageRange = "today" | "7d" | "30d" | "all";

export interface UsageTotals {
  runs: number;
  errors: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  cost_usd: number;
  duration_ms: number;
}

export interface FeatureUsage extends UsageTotals {
  feature: string;
  variant: string | null;
}

export interface ModelUsageRow extends UsageTotals {
  model: string;
}

export interface AiUsageSummary {
  totals: UsageTotals;
  by_feature: FeatureUsage[];
  by_model: ModelUsageRow[];
}

export interface AiRun {
  id: number;
  started_at: string;
  duration_ms: number;
  feature: string;
  variant: string | null;
  /** Set only while the review still exists. */
  review_id: number | null;
  label: string;
  requested_model: string;
  models: string[];
  status: "ok" | "error";
  error: string | null;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  cost_usd: number;
  repo: string | null;
  pr_number: number | null;
  target_title: string | null;
  base_ref: string | null;
  head_ref: string | null;
}

export interface Target {
  id: number;
  repo_id: number;
  kind: "github_pr" | "local";
  github_pr_number: number | null;
  /** Node id of the mirrored PR, when the mirror has it. */
  gh_pr_id: string | null;
  title: string;
  base_ref: string;
  head_ref: string;
  three_dot: boolean;
  created_at: string;
}

export interface Review {
  id: number;
  target_id: number;
  body: string;
  event: ReviewEvent | null;
  status: "draft" | "published_pending" | "published";
  /** The pin: the merge-base (or two-dot base tip) and head this review was written against. */
  base_sha: string | null;
  head_sha: string | null;
  published_at: string | null;
  github_review_id: number | null;
  last_exported_at: string | null;
  created_at: string;
  updated_at: string;
  sidebar_collapsed: boolean;
  chat_collapsed: boolean;
  left_panel_width: number;
  right_panel_width: number;
}

export interface Comment {
  id: number;
  review_id: number;
  file_path: string;
  subject_type: "line" | "file";
  origin: "diff" | "file_view";
  side: Side;
  line: number;
  start_line: number | null;
  diff_hunk: string | null;
  body: string;
  parent_id: number | null;
  anchored_head_sha: string | null;
  anchored_base_sha: string | null;
  github_comment_id: number | null;
  resolved_at: string | null;
  /** databaseId of the GitHub thread's root comment this draft reply targets. */
  github_thread_root_id: number | null;
  /** `ai`/`ai:*`/`local`/none. Tagged comments are never published and don't pin a
   *  focus-hidden hunk/file visible. Editing an AI comment clears the tag. */
  tag: CommentTag | null;
  created_at: string;
  updated_at: string;
}

export interface PublishDraftRepliesResult {
  published: number;
  failed: number;
  errors: string[];
}

export interface ReviewDetail {
  review: Review;
  target: Target;
  local_path: string | null;
  remote_owner: string | null;
  remote_name: string | null;
  friendly_name: string | null;
  comments: Comment[];
  viewed_files: string[];
}

export interface ReviewSummary {
  review: Review;
  target: Target;
  repo_id: number;
  repo_label: string;
  comment_count: number;
}

export interface ChatMessage {
  id: number;
  chat_id: number;
  role: "user" | "assistant";
  content: string;
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
  created_at: string;
}

export interface ChatTurnResult {
  text: string;
  input_tokens: number;
  output_tokens: number;
  cost_usd: number;
}

export interface PrSummary {
  number: number;
  title: string;
  author: { login: string | null } | null;
  headRefName: string;
  baseRefName: string;
  createdAt: string;
  url: string;
}

export interface ToolEnv {
  git: string | null;
  gh: string | null;
  gh_authed: boolean;
  claude: string | null;
}

// --- GitHub PR metadata (review header; read-only, ephemeral) ---

export interface PrActor {
  login: string | null;
  avatarUrl: string | null;
}

export interface PrLabel {
  name: string;
  color: string; // hex without '#'
}

export interface PrReviewer {
  author: PrActor | null;
  state: string; // APPROVED | CHANGES_REQUESTED | COMMENTED | DISMISSED | PENDING
}

export interface PrCheck {
  name: string;
  state: string | null; // conclusion/status or StatusContext.state
  url: string | null;
}

export interface PrMeta {
  number: number;
  title: string;
  url: string;
  body: string; // Markdown source
  state: string; // OPEN | CLOSED | MERGED
  isDraft: boolean;
  headRefOid: string | null;
  mergeable: string | null; // MERGEABLE | CONFLICTING | UNKNOWN
  mergeStateStatus: string | null; // BEHIND | CLEAN | DIRTY | BLOCKED | ...
  viewerCanUpdateBranch: boolean;
  reviewDecision: string | null; // APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED | null
  autoMerge: { mergeMethod: string | null; enabledBy: PrActor | null } | null;
  additions: number;
  deletions: number;
  changedFiles: number;
  author: PrActor | null;
  labels: PrLabel[];
  reviews: PrReviewer[];
  ciState: string | null;
  checks: PrCheck[];
}

// --- GitHub PR review threads (read-only, ephemeral; not persisted) ---

export interface PrThreadComment {
  id: string;
  databaseId: number | null;
  author: PrActor | null;
  body: string; // Markdown source
  createdAt: string;
  url: string;
  diffHunk: string | null;
  outdated: boolean;
}

/** Which PR a GitHub thread belongs to — needed by thread mutations and for
 *  invalidating the ["pr-threads", owner, name, number] query. */
export interface PrThreadCtx {
  owner: string;
  name: string;
  number: number;
}

export interface PrThread {
  id: string;
  isResolved: boolean;
  isOutdated: boolean;
  isCollapsed: boolean;
  path: string | null;
  line: number | null;
  startLine: number | null;
  originalLine: number | null;
  diffSide: string | null; // LEFT | RIGHT
  startDiffSide: string | null;
  subjectType: string | null; // LINE | FILE
  comments: PrThreadComment[];
}

// --- GitHub inbox ---

export type ItemReasonKind =
  | "assigned"
  | "mention"
  | "direct_review"
  | "team_review"
  | "author"
  | "comment";

export interface ItemReason {
  reason: ItemReasonKind;
  detail: string;
}

/** One inbox item (PR or issue). Mirrors the backend `items` row, flattened with
 *  its `reasons`. */
export interface InboxItem {
  id: string;
  type: "pr" | "issue";
  number: number;
  repo: string; // "owner/name"
  title: string;
  url: string;
  author_login: string | null;
  author_avatar: string | null;
  state: string | null;
  is_draft: boolean;
  body: string | null;
  latest_comment: string | null;
  latest_actor: string | null;
  updated_at: string;
  files_changed: number | null;
  additions: number | null;
  deletions: number | null;
  top_files_json: string | null;
  ci_state: string | null;
  review_decision: string | null;
  untracked_at: string | null;
  closed_at: string | null;
  engaged_at: string | null;
  first_seen_at: string;
  last_refreshed: string;
  reasons: ItemReason[];
}

export type SyncScope = "inbox" | "board" | "all";
/** light: probe + changed only; forced: a Sync click; full: refetch everything. */
export type SyncMode = "light" | "forced" | "full";
export type AutoSync = "off" | "on_focus" | "periodic";

export interface SyncReport {
  discovered: boolean;
  viewerLogin: string;
  itemCount: number;
  cardCount: number;
  closedCount: number;
  autoTracked: number;
  probed: number;
  changed: number;
  threads: number;
  files: number;
  durationMs: number;
}

export interface SyncStatus {
  running: SyncScope[];
  discovering: boolean;
  full: boolean;
  lastDiscoveryAt: string | null;
  lastInboxAt: string | null;
  lastBoardAt: string | null;
  lastError: string | null;
  lastErrorAt: string | null;
}

export interface MonitorAction {
  card_id: string;
  repo: string;
  number: number;
  kind: "updated" | "reran" | "gave_up" | "error";
  detail: string;
}

export interface MonitorLogEntry {
  id: number;
  at: string;
  card_id: string;
  repo: string;
  number: number;
  kind: MonitorAction["kind"];
  detail: string;
}

export interface BoardMonitorPayload {
  actions: MonitorAction[];
  error: string | null;
}

export interface MirrorUpdatedPayload {
  scopes: SyncScope[];
  ids: string[];
}

/** A review's pin compared with its target's live head. */
export interface FreshnessResult {
  moved: boolean;
  pinnedHeadSha: string | null;
  liveHeadSha: string | null;
  liveBaseSha: string | null;
  /** Commits on the live head the pinned head lacks; local targets only. */
  newCommits: number | null;
}

export interface UpdateReviewResult {
  moved: boolean;
  reanchor: ReanchorResult;
}

export interface ReanchorResult {
  reanchored: number;
  lost: number;
  skippedNoChange: number;
}

export interface InboxMeta {
  lastRefreshAt: string | null;
  viewerLogin: string | null;
}

export type WorktreeSource = "claude" | "codex" | "codereview" | "manual";

export interface WorktreeInfo {
  path: string;
  display_path: string;
  head_sha: string;
  branch: string | null;
  is_main: boolean;
  is_prunable: boolean;
  prunable_reason: string | null;
  is_locked: boolean;
  source: WorktreeSource;
}

// Git graph (spec 22)
export type GraphScope = "all" | "local";

export type WipKind = "staged" | "unstaged" | "untracked";

export type RefKind = "local" | "remote" | "tag";

export type ChangeStatus = "A" | "M" | "D" | "R" | "C" | "T";

export interface GraphCommit {
  sha: string;
  parents: string[];
  subject: string;
  body_preview: string;
  author_name: string;
  author_email: string;
  author_time: number;
  committer_time: number;
}

export interface RefInfo {
  name: string;
  kind: RefKind;
  sha: string;
  is_head: boolean;
  /** Unix seconds: tip commit date, or tagger date for annotated tags. */
  updated_at: number;
}

export interface DeleteGoneOutcome {
  deleted: string[];
  failed: { name: string; error: string }[];
}

export interface ChangedFile {
  path: string;
  old_path: string | null;
  status: ChangeStatus;
  additions: number | null;
  deletions: number | null;
}

export interface CommitDetail {
  sha: string;
  parents: string[];
  subject: string;
  body: string;
  author_name: string;
  author_email: string;
  author_time: number;
  committer_name: string;
  committer_email: string;
  committer_time: number;
  files: ChangedFile[];
}

export interface WipStatus {
  head_sha: string;
  staged: ChangedFile[];
  unstaged: ChangedFile[];
  untracked: string[];
}

export interface StashEntry {
  /** Position in `git stash list`, the N of `stash@{N}`. */
  index: number;
  sha: string;
  message: string;
  /** Unix seconds. */
  created_at: number;
}

// --- Focus review (spec 10) ---

export type FocusLevel = "off" | "thorough" | "focus" | "critical";
export type FocusTier = "trivial" | "skim" | "review" | "critical";
export type FocusOverride = "show" | "hide";

export interface Focus {
  id: number;
  review_id: number;
  head_sha: string | null;
  base_sha: string | null;
  level: FocusLevel;
  summary: string | null;
  model: string | null;
  status: "generating" | "ready" | "failed";
  error: string | null;
  planned_at: string | null;
  created_at: string;
}

export interface FocusEntry {
  hunk_id: string;
  file_path: string;
  old_start: number;
  new_start: number;
  /** Empty for a whole-file pseudo-hunk (rename-only, binary, mode change). */
  header: string;
  additions: number;
  deletions: number;
  tier: FocusTier;
  source: "rule" | "ai" | "default" | null;
  reason: string | null;
  override: FocusOverride | null;
  is_new: boolean;
}

export interface FocusView {
  focus: Focus;
  stale: boolean;
  hunks: FocusEntry[];
  file_overrides: { file_path: string; override: FocusOverride }[];
}

// Kanban board (work tracker)

export type BoardLane = "backlog" | "in_progress" | "submitted" | "reviews" | "done";

export type BoardMetaKind = "file" | "link" | "note" | "claude_session";

export interface BoardMeta {
  id: number;
  card_id: string;
  kind: BoardMetaKind;
  value: string;
  tag: string | null;
  label: string | null;
  cwd: string | null;
  created_at: string;
}

/** One board card (an issue or PR). Mirrors the backend `board_card` row. */
export interface BoardCard {
  id: string;
  type: "pr" | "issue";
  repo: string; // "owner/name"
  number: number;
  url: string;
  title: string;
  author_login: string | null;
  state: string | null;
  is_draft: boolean;
  review_decision: string | null;
  has_reviews: boolean;
  head_ref: string | null;
  is_mine: boolean;
  pending_threads: number | null;
  updated_at: string;
  closed_at: string | null;
  tracked_at: string;
  discarded_at: string | null;
  archived_at: string | null;
  last_refreshed: string | null;
  ci_status: string | null; // SUCCESS | FAILURE | PENDING | ERROR | EXPECTED, from the head commit's statusCheckRollup
  mergeable: string | null; // MERGEABLE | CONFLICTING | UNKNOWN
  merge_state_status: string | null; // CLEAN | BEHIND | BLOCKED | DIRTY | UNSTABLE | HAS_HOOKS | UNKNOWN
  stack_number: number | null; // GitHub native stack id, null when not stacked
  stack_position: number | null;
  viewer_can_update_branch: boolean;
  auto_merge: string | null; // SQUASH | MERGE | REBASE when auto-merge is enabled
  // Computed/enriched by board_list, flattened alongside the row above.
  lane: BoardLane | null;
  meta: BoardMeta[];
  inherited_meta: BoardMeta[];
  linked_issue_ids: string[];
  linked_pr_ids: string[];
  existing_review_id: number | null;
}

// Claude Code sessions

export type SessionFocusState = "ready" | "detached" | "background" | "no_terminal" | "not_live";

/** A running `claude agents` view. */
export interface AgentView {
  pid: number;
  cwd: string | null;
  filter_cwd: string | null;
  tmux: { session: string; window: string; pane: string; label: string | null } | null;
  tty: string;
  focus: SessionFocusState;
}

export interface ClaudeSession {
  session_id: string;
  pid: number | null;
  live: boolean;
  status: string | null; // busy | idle
  kind: string | null; // interactive | bg
  attached: boolean; // bg session viewed through `claude attach`
  job_id: string | null; // background job id for `claude attach|stop|logs`
  waiting_for: string | null; // e.g. "input needed", "dialog open"
  agent_view: AgentView | null; // agent view to open an unattached background session from
  name: string | null;
  cwd: string;
  git_branch: string | null;
  repo: string | null; // "owner/name" of the cwd's origin (live only)
  tmux: { session: string; window: string; pane: string; label: string | null } | null;
  tty: string | null;
  updated_at: number; // epoch ms
  focus: SessionFocusState;
  summaries: SessionSummary[]; // newest first; live sessions only carry an app-generated one
  original_name: string | null; // transcript name when a summary title replaced it
}

export interface SessionSummary {
  date: string | null;
  title: string | null;
  body: string;
  generated: boolean; // made by the app's Generate button, not the SessionEnd hook
}

/** A live session matched to a PR card (via claude_session meta or head-branch checkout). */
export interface CardSession {
  card_id: string;
  repo: string;
  number: number;
  session_id: string;
}

export interface SessionsSnapshot {
  sessions: ClaudeSession[];
  card_sessions: CardSession[];
}

export interface ChatWorktree {
  path: string;
  branch: string | null;
  is_main: boolean;
}

export interface WorktreePr {
  number: number;
  state: "OPEN" | "MERGED" | "CLOSED";
  headRefOid: string;
  headRefName: string;
}

export type WorktreeUpstream =
  | { kind: "none" }
  | { kind: "gone" }
  | { kind: "tracking"; ahead: number; behind: number; head_was_pushed: boolean };

export type WorktreeVerdict =
  | { kind: "remove"; reason: string; delete_branch: boolean }
  | { kind: "prune"; reason: string }
  | { kind: "fast_forward"; reason: string }
  | { kind: "resync"; reason: string }
  | { kind: "flag"; reason: string }
  | { kind: "keep"; reason: string };

export interface WorktreeRow {
  path: string;
  display_path: string;
  branch: string | null;
  head_sha: string;
  source: WorktreeSource;
  is_main: boolean;
  pr: WorktreePr | null;
  upstream: WorktreeUpstream;
  last_commit_at: string | null;
  verdict: WorktreeVerdict;
}

export interface WorktreeRepoReport {
  repo_id: number;
  path: string;
  slug: string | null;
  error: string | null;
  worktrees: WorktreeRow[];
}

export interface WorktreeGcAction {
  repo_id: number;
  path: string;
  action: string;
  ok: boolean;
  message: string | null;
}

export interface WorktreeGcReport {
  ran_at: string;
  repos: WorktreeRepoReport[];
  actions: WorktreeGcAction[];
}

export interface BoardOverview {
  markdown: string;
  generated_at: string;
  model: string | null;
  card_count: number;
  card_ids: string[];
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
}

export interface PromptInfo {
  key: string;
  label: string;
  description: string;
  placeholders: string[];
  default: string;
  override: string | null;
}

/** Fetch / local-change bookkeeping for one repository (`repo_sync`). */
export interface RepoSyncState {
  last_fetch_at: string | null;
  last_fetch_error: string | null;
  last_local_change_at: string | null;
}

export interface FetchResult {
  /** false when skipped because the last successful fetch was recent enough. */
  fetched: boolean;
  state: RepoSyncState;
}

export type GitChangeKind = "head" | "refs" | "index" | "fetch";
