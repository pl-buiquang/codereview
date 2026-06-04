import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  Branch,
  Comment,
  PrSummary,
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
  updateReview: (reviewId: number, body?: string, event?: string) =>
    invoke<void>("update_review", { reviewId, body, event }),
  deleteReview: (reviewId: number) =>
    invoke<void>("delete_review", { reviewId }),

  // GitHub
  ghAuthStatus: () => invoke<boolean>("gh_auth_status"),
  checkEnvironment: () => invoke<ToolEnv>("check_environment"),
  listPrs: (repoPath: string) => invoke<PrSummary[]>("list_prs", { repoPath }),
  createReviewForPr: (repoId: number, repoPath: string, prNumber: number) =>
    invoke<Review>("create_review_for_pr", { repoId, repoPath, prNumber }),
  publishReview: (reviewId: number) => invoke<Review>("publish_review", { reviewId }),

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
  }) => invoke<Comment>("add_comment", args),
  updateComment: (commentId: number, body: string) =>
    invoke<void>("update_comment", { commentId, body }),
  deleteComment: (commentId: number) =>
    invoke<void>("delete_comment", { commentId }),

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
