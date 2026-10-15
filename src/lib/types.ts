export interface Repository {
  id: number;
  local_path: string | null;
  remote_owner: string | null;
  remote_name: string | null;
  default_branch: string | null;
  added_at: string;
  friendly_name: string | null;
}

export interface Branch {
  name: string;
  is_remote: boolean;
  sha: string;
}

export type ReviewEvent = "comment" | "approve" | "request_changes";
export type Side = "LEFT" | "RIGHT";

export interface Target {
  id: number;
  repo_id: number;
  kind: "github_pr" | "local";
  github_pr_number: number | null;
  title: string;
  base_ref: string;
  head_ref: string;
  base_sha: string | null;
  head_sha: string | null;
  three_dot: boolean;
  created_at: string;
}

export interface Review {
  id: number;
  target_id: number;
  body: string;
  event: ReviewEvent | null;
  status: "draft" | "published_pending" | "published";
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
  /** `ai`/`local`/none. Tagged comments are never published and don't pin a
   *  focus-hidden hunk/file visible. Editing an `ai` comment clears the tag. */
  tag: "ai" | "local" | null;
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
  mergeable: string | null; // MERGEABLE | CONFLICTING | UNKNOWN
  reviewDecision: string | null; // APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED | null
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

export interface RefreshResult {
  viewerLogin: string;
  itemCount: number;
  closedCount: number;
  durationMs: number;
}

export interface FreshnessResult {
  headMoved: boolean;
  previousHeadSha: string | null;
  currentHeadSha: string | null;
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
  // Computed/enriched by board_list, flattened alongside the row above.
  lane: BoardLane | null;
  meta: BoardMeta[];
  inherited_meta: BoardMeta[];
  linked_issue_ids: string[];
  linked_pr_ids: string[];
  existing_review_id: number | null;
}

export interface BoardRefreshResult {
  cardCount: number;
  autoTracked: number;
  durationMs: number;
}
