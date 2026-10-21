import { timeAgo } from "../lib/timeAgo";
import type { InboxItem } from "../lib/types";
import { BotBadge, CiBadge, ReasonBadge, StaleBadge, StatusPill, TypeBadge } from "./InboxBadges";
import { RepoName } from "./RepoName";
import { Icon } from "./icons";

/** `new`: an untriaged item. `archive`: a dismissed, closed, discarded or tracked one. */
export type RowVariant = "new" | "archive";

const CAUSE_LABEL: Record<string, string> = {
  dismissed: "dismissed",
  closed: "closed on GitHub",
  discarded: "discarded from board",
};

export function InboxItemRow({
  item,
  variant,
  busy,
  isBot,
  existingReviewId,
  onArchive,
  onRestore,
  onOpenReview,
  onOpenExisting,
  boardTracked,
  onBoardTrack,
}: {
  item: InboxItem;
  variant: RowVariant;
  busy: boolean;
  /** Author is a bot, by GitHub's account type or the Settings list. */
  isBot: boolean;
  existingReviewId?: number;
  onArchive: () => void;
  onRestore: () => void;
  /** Opens the PR as a review; this also tracks it on the board. */
  onOpenReview: () => void;
  onOpenExisting?: () => void;
  /** Whether this item already has a card on the kanban board. */
  boardTracked?: boolean;
  /** Shown as a "Track" button when the item isn't on the board yet. */
  onBoardTrack?: () => void;
}) {
  const tracked = boardTracked || item.inbox_state === "tracked";

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
          {isBot && <BotBadge />}
          {item.stale && <StaleBadge />}
          {variant === "archive" &&
            (item.inbox_state === "tracked" ? (
              <span className="chip accent" title="On the board">
                tracked
              </span>
            ) : (
              <span className="chip" title={`Moved ${timeAgo(item.state_at)}`}>
                {CAUSE_LABEL[item.archive_cause ?? "dismissed"] ?? "archived"}
              </span>
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
          </div>
        )}
      </div>

      <div className="pr-side">
        <span className="pr-when" title={item.updated_at}>
          {timeAgo(item.updated_at)}
        </span>
        <div className="pr-actions">
          {item.type === "pr" &&
            (existingReviewId != null ? (
              <>
                <button
                  className="btn btn-sm btn-primary"
                  disabled={busy}
                  onClick={onOpenExisting}
                  title="Open your existing review of this PR"
                >
                  Resume review
                </button>
                <button
                  className="btn btn-sm"
                  disabled={busy}
                  onClick={onOpenReview}
                  title="Start a new review of this PR (tracks it on the board)"
                >
                  New review
                </button>
              </>
            ) : (
              <button
                className="btn btn-sm btn-primary"
                disabled={busy}
                onClick={onOpenReview}
                title="Review this PR; it is tracked on the board and leaves the inbox"
              >
                Open as review
              </button>
            ))}
          {!tracked && onBoardTrack && (
            <button
              className="btn btn-sm"
              disabled={busy}
              onClick={onBoardTrack}
              title="Track on the board — leaves the inbox"
            >
              Track
            </button>
          )}
          {variant === "new" ? (
            <button
              className="btn btn-sm btn-ghost"
              disabled={busy}
              onClick={onArchive}
              title="Archive — hide from New; comes back only if GitHub gives it a new reason"
            >
              Archive
            </button>
          ) : (
            item.inbox_state === "archived" && (
              <button className="btn btn-sm btn-ghost" disabled={busy} onClick={onRestore} title="Move back to New">
                Restore
              </button>
            )
          )}
        </div>
      </div>
    </div>
  );
}
