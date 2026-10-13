import type { ItemReasonKind } from "../lib/types";
import { Icon } from "./icons";

const REASON_LABEL: Record<ItemReasonKind, string> = {
  assigned: "assigned",
  mention: "mention",
  direct_review: "review",
  team_review: "team review",
  author: "authored",
  comment: "comment",
};

export function ReasonBadge({ reason, detail }: { reason: ItemReasonKind; detail?: string }) {
  const label = REASON_LABEL[reason] ?? reason;
  const team = reason === "team_review" && detail ? ` · ${detail.split("/")[1] ?? detail}` : "";
  return (
    <span className="chip" title={detail || undefined}>
      {label}
      {team}
    </span>
  );
}

export function TypeBadge({ type }: { type: "issue" | "pr" }) {
  if (type === "pr") return <span className="badge badge-pr">PR</span>;
  return <span className="badge">issue</span>;
}

export function StatusPill({ state, isDraft }: { state: string | null; isDraft: boolean }) {
  if (isDraft) return <span className="badge badge-draft">draft</span>;
  if (!state) return null;
  const kind =
    state === "open" ? "badge-open" : state === "merged" ? "badge-pr" : "badge-review";
  return <span className={`badge ${kind}`}>{state}</span>;
}

export function CiBadge({ state }: { state: string | null }) {
  if (!state) return null;
  const kind = state === "success" ? "ok" : state === "failure" || state === "error" ? "bad" : "";
  const icon = kind === "ok" ? "check" : kind === "bad" ? "x" : "dot";
  return (
    <span className={`chip ${kind}`.trim()} title={`CI: ${state}`}>
      <Icon name={icon} size={11} /> ci
    </span>
  );
}

/** Mergeability (conflicts), distinct from CI: a PR can pass every check and
 *  still conflict with its base, or vice versa. `null`/`UNKNOWN` hides the
 *  badge rather than showing a stale "checking…" state. */
export function MergeableBadge({ mergeable }: { mergeable: string | null }) {
  if (!mergeable || mergeable === "UNKNOWN") return null;
  const ok = mergeable === "MERGEABLE";
  return (
    <span className={`chip ${ok ? "ok" : "bad"}`} title={ok ? "No merge conflicts" : "Merge conflicts"}>
      <Icon name={ok ? "check" : "x"} size={11} /> merge
    </span>
  );
}
