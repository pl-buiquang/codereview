import { timeAgo } from "../lib/timeAgo";
import type { InboxItem } from "../lib/types";
import { CiBadge, ReasonBadge, StatusPill, TypeBadge } from "./InboxBadges";
import { RepoName } from "./RepoName";
import { Icon } from "./icons";

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
    <div className="card pr-card">
      <span className="avatar">
        {item.author_avatar ? (
          <img src={item.author_avatar} alt={item.author_login ?? ""} />
        ) : (
          <Icon name="person" size={16} />
        )}
      </span>

      <div className="pr-main">
        <div className="pr-meta">
          <TypeBadge type={item.type} />
          <RepoName className="repo mono" name={item.repo} />
          <span className="mono">#{item.number}</span>
          <StatusPill state={item.state} isDraft={item.is_draft} />
          {item.type === "pr" && <CiBadge state={item.ci_state} />}
        </div>

        <a className="pr-title" href={item.url} target="_blank" rel="noreferrer">
          {item.title}
        </a>

        <div className="pr-checks">
          {item.reasons.map((r, i) => (
            <ReasonBadge key={i} reason={r.reason} detail={r.detail} />
          ))}
          <span className="faint">by {item.author_login ?? "unknown"}</span>
        </div>

        {(item.latest_comment || item.body) && (
          <p className="pr-snippet">
            {item.latest_actor && item.latest_comment && (
              <span className="mono">{item.latest_actor}: </span>
            )}
            {item.latest_comment ?? item.body}
          </p>
        )}

        {item.type === "pr" && (
          <div className="pr-foot">
            <span>
              <span className="mono">{item.files_changed ?? 0}</span> files
            </span>
            <span>
              <span className="delta-add">+{item.additions ?? 0}</span>{" "}
              <span className="delta-del">−{item.deletions ?? 0}</span>
            </span>
            {item.review_decision && (
              <span>{item.review_decision.toLowerCase().replace(/_/g, " ")}</span>
            )}
            {topFiles.length > 0 && (
              <a href={item.url} target="_blank" rel="noreferrer" title={topFiles.map((f) => f.path).join("\n")}>
                {topFiles.length} top file{topFiles.length > 1 ? "s" : ""}
              </a>
            )}
          </div>
        )}
      </div>

      <div className="pr-side">
        <span className="pr-when" title={item.updated_at}>
          {timeAgo(item.updated_at)}
        </span>
        <div className="pr-actions">
          {item.type === "pr" && (
            <button className="btn btn-sm btn-primary" disabled={busy} onClick={onOpenReview}>
              Open as review
            </button>
          )}
          {(variant === "inbox" || variant === "visited") && (
            <>
              {variant === "inbox" ? (
                <button
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={onEngage}
                  title="Done — moves to Visited; resurfaces on new activity"
                >
                  Done
                </button>
              ) : (
                <button
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={onUnengage}
                  title="Bring back to its inbox bucket"
                >
                  Bring back
                </button>
              )}
              <button
                className="btn btn-sm btn-ghost"
                disabled={busy}
                onClick={onUntrack}
                title="Untrack — dismiss (find it again in Archive)"
              >
                Untrack
              </button>
            </>
          )}
          {variant === "archive" && (
            <button className="btn btn-sm" disabled={busy} onClick={onRetrack} title="Re-track">
              Re-track
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
