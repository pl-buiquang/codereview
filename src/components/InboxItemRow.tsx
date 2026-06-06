import { timeAgo } from "../lib/timeAgo";
import type { InboxItem } from "../lib/types";
import { CiBadge, ReasonBadge, StatusPill, TypeBadge } from "./InboxBadges";

interface TopFile {
  path: string;
  additions: number;
  deletions: number;
  changes: number;
}

function parseTopFiles(json: string | null): TopFile[] {
  if (!json) return [];
  try {
    return JSON.parse(json) as TopFile[];
  } catch {
    return [];
  }
}

export type RowVariant = "inbox" | "visited" | "archive" | "closed";

export function InboxItemRow({
  item,
  variant,
  busy,
  onEngage,
  onUnengage,
  onUntrack,
  onRetrack,
  onOpenReview,
}: {
  item: InboxItem;
  variant: RowVariant;
  busy: boolean;
  onEngage: () => void;
  onUnengage: () => void;
  onUntrack: () => void;
  onRetrack: () => void;
  onOpenReview: () => void;
}) {
  const topFiles = parseTopFiles(item.top_files_json);

  return (
    <div className="inbox-row">
      <div className="inbox-row-body">
        {item.author_avatar ? (
          <img className="inbox-avatar" src={item.author_avatar} alt={item.author_login ?? ""} />
        ) : (
          <div className="inbox-avatar inbox-avatar-empty" />
        )}

        <div className="inbox-row-main">
          <div className="inbox-row-meta">
            <TypeBadge type={item.type} />
            <span className="inbox-repo">{item.repo}</span>
            <span className="inbox-number">#{item.number}</span>
            <StatusPill state={item.state} isDraft={item.is_draft} />
            {item.type === "pr" && <CiBadge state={item.ci_state} />}
            <span className="inbox-updated" title={item.updated_at}>
              {timeAgo(item.updated_at)}
            </span>
          </div>

          <a className="inbox-title" href={item.url} target="_blank" rel="noreferrer">
            {item.title}
          </a>

          <div className="inbox-reasons">
            {item.reasons.map((r, i) => (
              <ReasonBadge key={i} reason={r.reason} detail={r.detail} />
            ))}
            <span className="inbox-author">by {item.author_login ?? "unknown"}</span>
          </div>

          {(item.latest_comment || item.body) && (
            <p className="inbox-snippet">
              {item.latest_actor && item.latest_comment && (
                <span className="inbox-snippet-actor">{item.latest_actor}: </span>
              )}
              {item.latest_comment ?? item.body}
            </p>
          )}

          {item.type === "pr" && (
            <div className="inbox-stats">
              <span>
                <span className="mono">{item.files_changed ?? 0}</span> files
              </span>
              <span>
                <span className="add">+{item.additions ?? 0}</span>{" "}
                <span className="del">−{item.deletions ?? 0}</span>
              </span>
              {item.review_decision && (
                <span className="inbox-decision">
                  {item.review_decision.toLowerCase().replace(/_/g, " ")}
                </span>
              )}
              {topFiles.length > 0 && (
                <span className="inbox-topfiles" title={topFiles.map((f) => f.path).join("\n")}>
                  {topFiles.length} top file{topFiles.length > 1 ? "s" : ""}
                </span>
              )}
            </div>
          )}
        </div>

        <div className="inbox-row-actions">
          {item.type === "pr" && (
            <button className="btn-primary btn-xs" disabled={busy} onClick={onOpenReview}>
              Open as review
            </button>
          )}
          {(variant === "inbox" || variant === "visited") && (
            <>
              {variant === "inbox" ? (
                <button
                  className="btn-xs"
                  disabled={busy}
                  onClick={onEngage}
                  title="Done — moves to Visited; resurfaces on new activity"
                >
                  ✓ Done
                </button>
              ) : (
                <button
                  className="btn-xs"
                  disabled={busy}
                  onClick={onUnengage}
                  title="Bring back to its inbox bucket"
                >
                  ↩ Bring back
                </button>
              )}
              <button
                className="btn-xs"
                disabled={busy}
                onClick={onUntrack}
                title="Untrack — dismiss (find it again in Archive)"
              >
                ✕ Untrack
              </button>
            </>
          )}
          {variant === "archive" && (
            <button className="btn-xs" disabled={busy} onClick={onRetrack} title="Re-track">
              ↺ Re-track
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
