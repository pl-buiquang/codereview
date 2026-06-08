import { useState } from "react";
import { api } from "../lib/api";
import { timeAgo } from "../lib/timeAgo";
import { Markdown } from "./Markdown";
import type { PrThread, PrThreadComment } from "../lib/types";

/** Read-only display of an existing GitHub PR review thread. Never editable,
 *  never published or exported — it's fetched ephemerally and shown "from
 *  GitHub" so it can't be confused with a local draft. */
export function GithubThread({ thread }: { thread: PrThread }) {
  const collapsible = thread.isResolved && thread.isCollapsed;
  const [expanded, setExpanded] = useState(!collapsible);

  return (
    <div className="github-thread">
      <div className="github-thread-header">
        <span className="github-thread-mark" title="From GitHub (read-only)">
          GitHub
        </span>
        {thread.isResolved && (
          <span className="github-thread-badge badge-resolved">Resolved</span>
        )}
        {thread.isOutdated && (
          <span className="github-thread-badge badge-outdated">Outdated</span>
        )}
        {collapsible && (
          <button
            className="github-thread-toggle"
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded
              ? "Hide"
              : `Resolved · ${thread.comments.length} comment${
                  thread.comments.length === 1 ? "" : "s"
                }`}
          </button>
        )}
      </div>
      {expanded &&
        thread.comments.map((c) => <ThreadComment key={c.id} comment={c} />)}
    </div>
  );
}

function ThreadComment({ comment }: { comment: PrThreadComment }) {
  const login = comment.author?.login ?? "unknown";
  return (
    <div className="github-thread-comment">
      <div className="github-thread-comment-head">
        {comment.author?.avatarUrl ? (
          <img
            className="github-thread-avatar"
            src={comment.author.avatarUrl}
            alt={login}
          />
        ) : (
          <span className="github-thread-avatar github-thread-avatar-empty" />
        )}
        <span className="github-thread-author">{login}</span>
        <span className="github-thread-time">{timeAgo(comment.createdAt)}</span>
        <a
          className="github-thread-link"
          href={comment.url}
          onClick={(e) => {
            e.preventDefault();
            api.openUrl(comment.url);
          }}
        >
          View on GitHub
        </a>
      </div>
      <Markdown source={comment.body} />
    </div>
  );
}
