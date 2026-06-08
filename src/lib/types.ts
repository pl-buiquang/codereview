export interface Repository {
  id: number;
  path: string;
  remote_owner: string | null;
  remote_name: string | null;
  default_branch: string | null;
  added_at: string;
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
  status: "draft" | "published";
  published_at: string | null;
  github_review_id: number | null;
  last_exported_at: string | null;
  created_at: string;
  updated_at: string;
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
  github_comment_id: number | null;
  created_at: string;
  updated_at: string;
}

export interface ReviewDetail {
  review: Review;
  target: Target;
  repo_path: string;
  remote_owner: string | null;
  remote_name: string | null;
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
