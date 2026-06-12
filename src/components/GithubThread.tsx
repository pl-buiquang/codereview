import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { timeAgo } from "../lib/timeAgo";
import { Markdown } from "./Markdown";
import { Composer } from "./ReviewView";
import type { PrThread, PrThreadComment, PrThreadCtx } from "../lib/types";

/** Display of an existing GitHub PR review thread. Never persisted or exported —
 *  it's fetched ephemerally and shown "from GitHub" so it can't be confused with
 *  a local draft. When `ctx` is supplied the thread can be replied to and
 *  resolved/unresolved, acting on GitHub state directly (no local storage). */
export function GithubThread({
  thread,
  ctx,
}: {
  thread: PrThread;
  ctx?: PrThreadCtx | null;
}) {
  const collapsible = thread.isResolved && thread.isCollapsed;
  const [expanded, setExpanded] = useState(!collapsible);
  const [replying, setReplying] = useState(false);
  const queryClient = useQueryClient();

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: ["pr-threads", ctx!.owner, ctx!.name, ctx!.number],
    });

  const setResolved = useMutation({
    mutationFn: (resolved: boolean) => api.setPrThreadResolved(thread.id, resolved),
    onSuccess: invalidate,
    onError: (e) => toast.error(`Thread update failed:\n${String(e)}`),
  });

  const rootId = thread.comments[0]?.databaseId ?? null;

  const reply = useMutation({
    mutationFn: (body: string) =>
      api.replyToThread(ctx!.owner, ctx!.name, ctx!.number, rootId!, body),
    onSuccess: () => {
      setReplying(false);
      invalidate();
    },
    onError: (e) => toast.error(`Reply failed:\n${String(e)}`),
  });

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
        {ctx && (
          <button
            className="github-thread-action"
            disabled={setResolved.isPending}
            onClick={() => setResolved.mutate(!thread.isResolved)}
          >
            {thread.isResolved ? "Unresolve" : "Resolve"}
          </button>
        )}
      </div>
      {expanded &&
        thread.comments.map((c) => <ThreadComment key={c.id} comment={c} />)}
      {expanded && ctx && rootId != null && (
        <div className="github-thread-reply">
          {!replying ? (
            <button
              className="github-thread-action"
              onClick={() => setReplying(true)}
            >
              Reply…
            </button>
          ) : (
            <Composer
              submitLabel="Reply"
              onSubmit={async (text) => {
                await reply.mutateAsync(text);
              }}
              onCancel={() => setReplying(false)}
            />
          )}
        </div>
      )}
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
