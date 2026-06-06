import type { ItemReasonKind } from "../lib/types";

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
    <span className={`reason-badge reason-${reason}`} title={detail || undefined}>
      {label}
      {team}
    </span>
  );
}

export function TypeBadge({ type }: { type: "issue" | "pr" }) {
  return <span className={`type-badge type-${type}`}>{type === "pr" ? "PR" : "Issue"}</span>;
}

export function StatusPill({ state, isDraft }: { state: string | null; isDraft: boolean }) {
  if (isDraft) return <span className="status-pill status-draft">draft</span>;
  if (!state) return null;
  return <span className={`status-pill status-${state}`}>{state}</span>;
}

export function CiBadge({ state }: { state: string | null }) {
  if (!state) return null;
  const kind = state === "success" ? "ok" : state === "failure" || state === "error" ? "bad" : "pending";
  const symbol = kind === "ok" ? "✓" : kind === "bad" ? "✕" : "•";
  return (
    <span className={`ci-badge ci-${kind}`} title={`CI: ${state}`}>
      {symbol} ci
    </span>
  );
}
