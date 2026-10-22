import { useRef, useState } from "react";
import type { ItemReasonKind } from "../lib/types";
import { CiChecksPopover, type PrRef } from "./CiChecksPopover";
import { Icon } from "./icons";

const REASON_LABEL: Record<ItemReasonKind, string> = {
  assigned: "assigned",
  mention: "mention",
  direct_review: "review",
  team_review: "team review",
  author: "authored",
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

export function BotBadge() {
  return (
    <span className="chip" title="Authored by a bot">
      <Icon name="bot" size={11} /> bot
    </span>
  );
}

export function StaleBadge() {
  return (
    <span className="chip warn" title="No longer matches any GitHub search (e.g. review request removed)">
      stale
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

/** The CI chip. With `pr` it becomes a button opening the checks popover. */
export function CiBadge({ state, pr }: { state: string | null; pr?: PrRef }) {
  const btn = useRef<HTMLButtonElement>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  if (!state) return null;
  const kind = state === "success" ? "ok" : state === "failure" || state === "error" ? "bad" : "";
  const icon = kind === "ok" ? "check" : kind === "bad" ? "x" : "dot";
  const chip = (
    <span className={`chip ${kind}`.trim()} title={`CI: ${state}`}>
      <Icon name={icon} size={11} /> ci
    </span>
  );
  if (!pr) return chip;
  return (
    <>
      <button
        ref={btn}
        className="chip-button"
        title={`CI: ${state} — show checks`}
        onClick={(e) => {
          e.stopPropagation();
          setAnchor(anchor ? null : (btn.current?.getBoundingClientRect() ?? null));
        }}
      >
        {chip}
      </button>
      {anchor && <CiChecksPopover pr={pr} anchor={anchor} onClose={() => setAnchor(null)} />}
    </>
  );
}

/** Mergeability (conflicts), distinct from CI: a PR can pass every check and
 *  still conflict with its base. Only ever flags the bad case — a mergeable
 *  PR gets no badge, same as a PR with no checks gets no CI badge. */
export function MergeableBadge({ mergeable }: { mergeable: string | null }) {
  if (mergeable !== "CONFLICTING") return null;
  return (
    <span className="chip bad" title="Merge conflicts">
      <Icon name="x" size={11} /> merge
    </span>
  );
}

/** Flags a head ref that's behind its base (`mergeStateStatus = BEHIND`):
 *  GitHub's "Update branch", or "Rebase stack" for a native stacked PR. */
export function MergeStateBadge({ state, stacked }: { state: string | null; stacked: boolean }) {
  if (state !== "BEHIND") return null;
  return (
    <span
      className="chip warn"
      title={stacked ? "Out of date with its base — rebase the stack" : "Out of date with its base — update the branch"}
    >
      <Icon name="branch" size={11} /> {stacked ? "rebase stack" : "update branch"}
    </span>
  );
}

/** Marks a PR that belongs to a native GitHub stack. Clickable when `onClick` is given. */
export function StackBadge({
  number,
  position,
  onClick,
}: {
  number: number | null;
  position: number | null;
  onClick?: () => void;
}) {
  if (number == null) return null;
  const label = `stack #${number}${position != null ? ` · ${position}` : ""}`;
  const title = `Part of GitHub stack #${number}${position != null ? ` (position ${position})` : ""}`;
  const content = (
    <>
      <Icon name="branch" size={11} /> {label}
    </>
  );
  if (!onClick) {
    return (
      <span className="chip" title={title}>
        {content}
      </span>
    );
  }
  return (
    <button type="button" className="chip" title={`${title} — filter to this stack`} onClick={onClick}>
      {content}
    </button>
  );
}

/** Only shown when GitHub auto-merge is enabled on the PR. */
export function AutoMergeBadge({ method, enabledBy }: { method: string | null | undefined; enabledBy?: string | null }) {
  if (!method) return null;
  const how = method.toLowerCase();
  return (
    <span
      className="chip accent"
      title={`Auto-merge enabled (${how})${enabledBy ? ` by ${enabledBy}` : ""} — merges once requirements pass`}
    >
      <Icon name="check" size={11} /> auto-merge
    </span>
  );
}

export function ReviewDecisionBadge({ decision }: { decision: string | null | undefined }) {
  if (decision === "APPROVED")
    return (
      <span className="chip ok" title="Approved">
        <Icon name="check" size={11} /> approved
      </span>
    );
  if (decision === "CHANGES_REQUESTED")
    return (
      <span className="chip bad" title="Changes requested">
        <Icon name="x" size={11} /> changes requested
      </span>
    );
  if (decision === "REVIEW_REQUIRED")
    return (
      <span className="chip" title="Review required">
        <Icon name="eye" size={11} /> review required
      </span>
    );
  return null;
}

export function CommentsBadge({ count }: { count: number | null }) {
  if (!count) return null;
  return (
    <span className="chip accent" title={`${count} unresolved thread(s)`}>
      <Icon name="comment" size={11} /> {count}
    </span>
  );
}
