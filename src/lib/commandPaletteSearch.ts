import type { BoardCard, Repository, ReviewSummary } from "./types";

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

export interface PaletteBoardResult {
  kind: "board-card";
  cardId: string;
  title: string;
  repoLabel: string;
  number: number;
  type: "pr" | "issue";
}

export type PaletteResult = PaletteBoardResult | PaletteReviewResult | PaletteCreateResult;

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
const GITHUB_PR_RE = /github\.com\/([^/\s]+)\/([^/\s]+)\/pull\/(\d+)/;

const GITHUB_ITEM_RE = /github\.com\/([^/\s]+)\/([^/\s]+)\/(?:pull|issues)\/(\d+)/i;
const REPO_REF_RE = /^([\w.-]+)\/([\w.-]+)#(\d+)$/;

/** Visible (tracked, not archived/discarded) card for `owner/name` + number. */
export function findBoardCard(cards: BoardCard[], repo: string, number: number): BoardCard | undefined {
  const r = repo.toLowerCase();
  return cards.find(
    (c) => c.number === number && c.repo.toLowerCase() === r && !c.archived_at && !c.discarded_at,
  );
}

/** Board card for a pasted PR/issue URL or `owner/repo#N`. */
export function matchBoardCard(query: string, cards: BoardCard[]): PaletteBoardResult | null {
  const q = query.trim();
  const m = GITHUB_ITEM_RE.exec(q) ?? REPO_REF_RE.exec(q);
  if (!m) return null;
  const card = findBoardCard(cards, `${m[1]}/${m[2]}`, parseInt(m[3], 10));
  if (!card) return null;
  return {
    kind: "board-card",
    cardId: card.id,
    title: card.title,
    repoLabel: card.repo,
    number: card.number,
    type: card.type,
  };
}

function parseGithubPrUrl(q: string): { owner: string; name: string; prNumber: number } | null {
  const m = GITHUB_PR_RE.exec(q);
  if (!m) return null;
  return { owner: m[1], name: m[2], prNumber: parseInt(m[3], 10) };
}

export function searchPalette(
  query: string,
  reviews: ReviewSummary[],
  repos: Repository[],
  boardCards: BoardCard[] = [],
): PaletteResult[] {
  const q = query.trim().toLowerCase();
  const boardHit = matchBoardCard(query, boardCards);
  const boardResults: PaletteBoardResult[] = boardHit ? [boardHit] : [];

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
  const pastedPr = parseGithubPrUrl(query.trim());

  const matched = reviews
    .filter((s) => {
      if (pastedPr) {
        return (
          s.target.github_pr_number === pastedPr.prNumber &&
          s.repo_label.toLowerCase() === `${pastedPr.owner}/${pastedPr.name}`.toLowerCase()
        );
      }
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

  // GitHub PR URL pasted → always offer to open it (if not already in results)
  if (pastedPr) {
    const { owner, name, prNumber } = pastedPr;
    const label = `${owner}/${name}`;
    const alreadyInReviews = reviewResults.some(
      (r) => r.repoLabel.toLowerCase() === label.toLowerCase() && r.prNumber === prNumber,
    );
    const alreadyInCreates = createResults.some(
      (r) => r.owner === owner && r.name === name && r.prNumber === prNumber,
    );
    if (!alreadyInReviews && !alreadyInCreates) {
      createResults.unshift({ kind: "create-pr", owner, name, prNumber, repoLabel: label });
    }
  }

  return [...boardResults, ...reviewResults, ...createResults];
}
