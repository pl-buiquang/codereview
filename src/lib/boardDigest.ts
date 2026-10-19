import type { BoardCard } from "./types";
import { mergeFacets } from "../components/BoardCard";

export const MAX_DO_NEXT = 7;
export const REVIEW_WAITING_DAYS = 2;
export const STALE_PR_DAYS = 7;
export const STALE_DRAFT_DAYS = 14;
export const STALE_ISSUE_DAYS = 21;

export interface DigestItem {
  cardId: string;
  ref: string;
  title: string;
  reason: string;
}

export interface ProjectCounts {
  repo: string;
  backlog: number;
  in_progress: number;
  submitted: number;
  reviews: number;
}

export interface Digest {
  open: number;
  doNext: DigestItem[];
  blocked: DigestItem[];
  waiting: DigestItem[];
  stale: DigestItem[];
  byProject: ProjectCounts[];
  hygiene: DigestItem[];
}

const DAY_MS = 86_400_000;

export function isOpenCard(c: BoardCard): boolean {
  return !c.archived_at && !c.discarded_at && (c.state == null || c.state === "open");
}

function daysSince(ts: string, now: Date): number {
  const t = Date.parse(ts);
  return Number.isNaN(t) ? 0 : Math.max(0, Math.floor((now.getTime() - t) / DAY_MS));
}

const shortRepo = (repo: string) => repo.split("/").pop() ?? repo;

function item(c: BoardCard, reason: string): DigestItem {
  return { cardId: c.id, ref: `${shortRepo(c.repo)}#${c.number}`, title: c.title, reason };
}

const ciFailing = (c: BoardCard) => c.ci_status === "FAILURE" || c.ci_status === "ERROR";

export function buildDigest(cards: BoardCard[], now: Date): Digest {
  const open = cards.filter(isOpenCard);
  const byId = new Map(cards.map((c) => [c.id, c]));

  const ranked: { rank: number; age: number; item: DigestItem }[] = [];
  const waiting: DigestItem[] = [];
  const blocked: DigestItem[] = [];
  const stale: DigestItem[] = [];
  const hygiene: DigestItem[] = [];
  const projects = new Map<string, ProjectCounts>();

  for (const c of open) {
    const age = daysSince(c.updated_at, now);
    const counts = projects.get(c.repo) ?? {
      repo: c.repo,
      backlog: 0,
      in_progress: 0,
      submitted: 0,
      reviews: 0,
    };
    if (c.lane && c.lane !== "done") counts[c.lane] += 1;
    projects.set(c.repo, counts);

    if (c.type === "issue") {
      if (age >= STALE_ISSUE_DAYS) stale.push(item(c, `no update ${age}d: nudge or close`));
      const prs = c.linked_pr_ids.map((id) => byId.get(id)).filter((p): p is BoardCard => !!p);
      if (prs.length > 0 && prs.every((p) => !isOpenCard(p))) {
        hygiene.push(item(c, "linked PR is no longer open: close or archive"));
      }
      continue;
    }

    const facets = mergeFacets(c);
    const conflicts = facets.includes("conflicts");
    const behind = facets.includes("behind");
    const autoMerge = facets.includes("auto_merge");
    const threads = c.pending_threads ?? 0;
    const approved = c.review_decision === "APPROVED";
    const changes = c.review_decision === "CHANGES_REQUESTED";
    const mineActive = c.is_mine && !c.is_draft;

    if (c.is_mine && c.is_draft) {
      if (age >= STALE_DRAFT_DAYS) stale.push(item(c, `draft idle ${age}d: finish or close`));
      continue;
    }

    if (age >= STALE_PR_DAYS) {
      stale.push(item(c, `no update ${age}d: ${c.is_mine ? "nudge reviewers or rebase" : "ping author or close"}`));
    }
    if (ciFailing(c)) blocked.push(item(c, "CI failing"));
    if (conflicts) blocked.push(item(c, "merge conflicts"));
    if (changes) blocked.push(item(c, "changes requested"));

    if (!c.is_mine) {
      if (c.lane === "reviews" && !approved) {
        if (age >= REVIEW_WAITING_DAYS) blocked.push(item(c, `review request waiting ${age}d`));
        ranked.push({ rank: 1, age, item: item(c, `Review (waiting ${age}d)`) });
      }
      continue;
    }

    if (!c.linked_issue_ids.length) hygiene.push(item(c, "PR has no linked issue"));

    if (approved && !autoMerge) {
      ranked.push({ rank: 2, age, item: item(c, behind ? "Update branch, then merge" : "Merge") });
    } else if (conflicts) {
      ranked.push({ rank: 3, age, item: item(c, "Resolve conflicts") });
    } else if (ciFailing(c)) {
      ranked.push({ rank: 4, age, item: item(c, "Fix CI") });
    } else if (changes || threads > 0) {
      ranked.push({
        rank: 5,
        age,
        item: item(c, threads > 0 ? `Address ${threads} unresolved thread${threads === 1 ? "" : "s"}` : "Address requested changes"),
      });
    } else if (behind) {
      ranked.push({ rank: 6, age, item: item(c, "Update branch") });
    } else if (mineActive) {
      const why = autoMerge
        ? "auto-merge on"
        : c.ci_status === "PENDING"
          ? "CI running"
          : "awaiting review";
      waiting.push(item(c, why));
    }
  }

  ranked.sort((a, b) => a.rank - b.rank || b.age - a.age);
  const doNext = ranked.slice(0, MAX_DO_NEXT).map((r) => r.item);

  return {
    open: open.length,
    doNext,
    blocked,
    waiting,
    stale,
    byProject: [...projects.values()].sort((a, b) => a.repo.localeCompare(b.repo)),
    hygiene,
  };
}

export function digestHeadline(d: Digest): string {
  return [
    `${d.open} open`,
    `${d.doNext.length} to do next`,
    `${d.blocked.length} blocked`,
    `${d.stale.length} stale`,
  ].join(" · ");
}
