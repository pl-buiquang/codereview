import type { Repository, ReviewSummary } from "./types";

export interface PaletteReviewResult {
  kind: "review";
  reviewId: number;
  repoId: number;
  title: string;
  repoLabel: string;
  prNumber: number | null;
  commentCount: number;
  updatedAt: string;
}

export interface PaletteCreateResult {
  kind: "create-pr";
  owner: string;
  name: string;
  prNumber: number;
  repoLabel: string;
}

export type PaletteResult = PaletteReviewResult | PaletteCreateResult;

const MAX_REVIEWS = 10;
const MAX_CREATES = 3;
const DEFAULT_RECENT = 5;

// "cardiologs-front" → "front"; "front" → "front"
function shortName(remote_name: string): string {
  return remote_name.replace(/^cardiologs-/, "");
}

function buildSearchText(s: ReviewSummary): string {
  const parts = [s.target.title, s.repo_label];
  if (s.target.github_pr_number != null) {
    parts.push(String(s.target.github_pr_number));
  }
  const repoName = s.repo_label.split("/")[1];
  if (repoName) {
    parts.push(repoName);
    parts.push(shortName(repoName));
  }
  return parts.join(" ").toLowerCase();
}

const WORD_NUMBER_RE = /^(\S+)\s+(\d+)$/;

export function searchPalette(
  query: string,
  reviews: ReviewSummary[],
  repos: Repository[],
): PaletteResult[] {
  const q = query.trim().toLowerCase();

  // Empty query → 5 most recent
  if (!q) {
    return reviews.slice(0, DEFAULT_RECENT).map((s) => ({
      kind: "review",
      reviewId: s.review.id,
      repoId: s.repo_id,
      title: s.target.title,
      repoLabel: s.repo_label,
      prNumber: s.target.github_pr_number,
      commentCount: s.comment_count,
      updatedAt: s.review.updated_at,
    }));
  }

  const tokens = q.split(/\s+/);

  const matched = reviews
    .filter((s) => {
      const text = buildSearchText(s);
      return tokens.every((t) => text.includes(t));
    })
    .slice(0, MAX_REVIEWS);

  const reviewResults: PaletteReviewResult[] = matched.map((s) => ({
    kind: "review",
    reviewId: s.review.id,
    repoId: s.repo_id,
    title: s.target.title,
    repoLabel: s.repo_label,
    prNumber: s.target.github_pr_number,
    commentCount: s.comment_count,
    updatedAt: s.review.updated_at,
  }));

  // "word number" → derive create suggestions
  const createResults: PaletteCreateResult[] = [];
  const m = WORD_NUMBER_RE.exec(q);
  if (m) {
    const word = m[1];
    const num = parseInt(m[2], 10);

    // Build a set of (owner/name, number) combos already covered by review results
    const existing = new Set(
      matched
        .filter((s) => s.target.kind === "github_pr" && s.target.github_pr_number != null)
        .map((s) => `${s.repo_label}#${s.target.github_pr_number}`),
    );

    for (const repo of repos) {
      if (!repo.remote_owner || !repo.remote_name) continue;
      const rn = repo.remote_name.toLowerCase();
      if (!rn.includes(word) && !shortName(rn).includes(word)) continue;
      const label = `${repo.remote_owner}/${repo.remote_name}`;
      if (existing.has(`${label}#${num}`)) continue;
      createResults.push({
        kind: "create-pr",
        owner: repo.remote_owner,
        name: repo.remote_name,
        prNumber: num,
        repoLabel: label,
      });
      if (createResults.length >= MAX_CREATES) break;
    }
  }

  return [...reviewResults, ...createResults];
}
