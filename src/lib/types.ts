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
