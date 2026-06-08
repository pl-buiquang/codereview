import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";
import { Markdown } from "./Markdown";
import { CiBadge, StatusPill } from "./InboxBadges";
import type { PrCheck, PrLabel, PrReviewer } from "../lib/types";

export function PrMetaPanel({
  owner,
  name,
  number,
}: {
  owner: string;
  name: string;
  number: number;
}) {
  const q = useQuery({
    queryKey: ["pr-meta", owner, name, number],
    queryFn: () => api.prMeta(owner, name, number),
  });

  if (q.isLoading) return <div className="pr-meta-panel muted">Loading PR details…</div>;
  if (q.isError || !q.data)
    return (
      <div className="pr-meta-panel muted pr-meta-error">
        Could not load PR details: {String(q.error)}
      </div>
    );

  const meta = q.data;

  return (
    <div className="pr-meta-panel">
      <div className="pr-meta-row">
        <StatusPill state={meta.state.toLowerCase()} isDraft={meta.isDraft} />
        <CiBadge state={meta.ciState ? meta.ciState.toLowerCase() : null} />
        <Mergeability mergeable={meta.mergeable} />
        <span className="pr-meta-counts">
          <span className="add">+{meta.additions}</span>{" "}
          <span className="del">−{meta.deletions}</span> ·{" "}
          {meta.changedFiles} file{meta.changedFiles === 1 ? "" : "s"}
        </span>
      </div>

      <ReviewSummaryRow decision={meta.reviewDecision} reviews={meta.reviews} />

      {meta.labels.length > 0 && (
        <div className="pr-meta-labels">
          {meta.labels.map((l) => (
            <LabelChip key={l.name} label={l} />
          ))}
        </div>
      )}

      {meta.checks.length > 0 && <ChecksList checks={meta.checks} />}

      {meta.body.trim() && <Description body={meta.body} />}
    </div>
  );
}

const MERGEABLE: Record<string, { label: string; kind: string }> = {
  MERGEABLE: { label: "Mergeable", kind: "ok" },
  CONFLICTING: { label: "Conflicts", kind: "warn" },
};

function Mergeability({ mergeable }: { mergeable: string | null }) {
  const m = (mergeable && MERGEABLE[mergeable]) || { label: "Checking…", kind: "neutral" };
  return <span className={`pr-meta-mergeable mergeable-${m.kind}`}>{m.label}</span>;
}

const DECISION_LABEL: Record<string, { label: string; kind: string }> = {
  APPROVED: { label: "Approved", kind: "ok" },
  CHANGES_REQUESTED: { label: "Changes requested", kind: "bad" },
  REVIEW_REQUIRED: { label: "Review required", kind: "pending" },
};

function ReviewSummaryRow({
  decision,
  reviews,
}: {
  decision: string | null;
  reviews: PrReviewer[];
}) {
  if (!decision && reviews.length === 0) return null;
  const d = decision ? DECISION_LABEL[decision] : null;
  return (
    <div className="pr-meta-reviews">
      {d && <span className={`pr-meta-decision decision-${d.kind}`}>{d.label}</span>}
      {reviews.map((r, i) => (
        <Reviewer key={`${r.author?.login ?? "?"}-${i}`} reviewer={r} />
      ))}
    </div>
  );
}

const REVIEW_KIND: Record<string, string> = {
  APPROVED: "ok",
  CHANGES_REQUESTED: "bad",
  DISMISSED: "neutral",
  COMMENTED: "neutral",
  PENDING: "pending",
};

function Reviewer({ reviewer }: { reviewer: PrReviewer }) {
  const login = reviewer.author?.login ?? "unknown";
  const kind = REVIEW_KIND[reviewer.state] ?? "neutral";
  const title = `${login} · ${reviewer.state.toLowerCase().replace(/_/g, " ")}`;
  return (
    <span className={`pr-reviewer reviewer-${kind}`} title={title}>
      {reviewer.author?.avatarUrl ? (
        <img className="pr-reviewer-avatar" src={reviewer.author.avatarUrl} alt={login} />
      ) : (
        <span className="pr-reviewer-avatar pr-reviewer-avatar-empty" />
      )}
      <span className="pr-reviewer-login">{login}</span>
    </span>
  );
}

function LabelChip({ label }: { label: PrLabel }) {
  const bg = `#${label.color}`;
  return (
    <span
      className="pr-label-chip"
      style={{ background: bg, color: readableText(label.color) }}
    >
      {label.name}
    </span>
  );
}

function ChecksList({ checks }: { checks: PrCheck[] }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="pr-meta-checks">
      <button className="pr-meta-toggle" onClick={() => setOpen((v) => !v)}>
        {open ? "▾" : "▸"} {checks.length} check{checks.length === 1 ? "" : "s"}
      </button>
      {open && (
        <ul className="pr-check-list">
          {checks.map((c, i) => (
            <li key={`${c.name}-${i}`} className={`pr-check check-${checkKind(c.state)}`}>
              <span className="pr-check-mark">{checkMark(c.state)}</span>
              {c.url ? (
                <a
                  href={c.url}
                  onClick={(e) => {
                    e.preventDefault();
                    if (c.url) api.openUrl(c.url);
                  }}
                >
                  {c.name}
                </a>
              ) : (
                <span>{c.name}</span>
              )}
              {c.state && <span className="pr-check-state">{c.state.toLowerCase()}</span>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Description({ body }: { body: string }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="pr-meta-description">
      <div className={`pr-meta-desc-body ${expanded ? "scroll" : "clamped"}`}>
        <Markdown source={body} />
      </div>
      <button className="pr-meta-toggle" onClick={() => setExpanded((v) => !v)}>
        {expanded ? "Show less" : "Show more"}
      </button>
    </div>
  );
}

const PASSED = new Set(["success", "completed", "neutral", "skipped"]);
const FAILED = new Set(["failure", "error", "cancelled", "timed_out", "action_required"]);

function checkKind(state: string | null): string {
  if (!state) return "pending";
  const s = state.toLowerCase();
  if (PASSED.has(s)) return "ok";
  if (FAILED.has(s)) return "bad";
  return "pending";
}

function checkMark(state: string | null): string {
  const k = checkKind(state);
  return k === "ok" ? "✓" : k === "bad" ? "✕" : "•";
}

/** Pick black/white text for a hex background via relative luminance. */
function readableText(hex: string): string {
  const n = parseInt(hex, 16);
  if (Number.isNaN(n) || hex.length !== 6) return "#000";
  const r = (n >> 16) & 0xff;
  const g = (n >> 8) & 0xff;
  const b = n & 0xff;
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  return luminance > 0.6 ? "#000" : "#fff";
}
