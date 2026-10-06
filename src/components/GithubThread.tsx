import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { confirmDialog } from "../lib/confirm";
import { toast } from "../lib/toast";
import { timeAgo } from "../lib/timeAgo";
import { useDebouncedCallback } from "../lib/useDebouncedCallback";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";
import { Composer } from "./ReviewView";
import type { Comment, PrThread, PrThreadComment, PrThreadCtx, Side } from "../lib/types";

/** Display of an existing GitHub PR review thread. Never persisted or exported —
 *  it's fetched ephemerally and shown "from GitHub" so it can't be confused with
 *  a local draft. When `ctx` is supplied the thread can be replied to and
 *  resolved/unresolved, acting on GitHub state directly (no local storage).
 *  When `reviewId` is also supplied and the thread is anchored to the current
 *  diff, replying offers a choice: "Reply now" posts to GitHub immediately,
 *  "Add draft reply" saves a local draft (shown via `draftReplies`) that can be
 *  edited, deleted, or published (individually or all at once) later. */
export function GithubThread({
  thread,
  ctx,
  reviewId,
  draftReplies,
  readOnly,
  onCommentsChanged,
}: {
  thread: PrThread;
  ctx?: PrThreadCtx | null;
  reviewId?: number;
  draftReplies?: Comment[];
  readOnly?: boolean;
  onCommentsChanged?: () => void;
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
  const canDraft =
    reviewId != null && thread.path != null && thread.line != null && thread.diffSide != null;

  const reply = useMutation({
    mutationFn: (body: string) =>
      api.replyToThread(ctx!.owner, ctx!.name, ctx!.number, rootId!, body),
    onSuccess: () => {
      setReplying(false);
      invalidate();
    },
    onError: (e) => toast.error(`Reply failed:\n${String(e)}`),
  });

  const draftReply = useMutation({
    mutationFn: (body: string) =>
      api.addDraftReply({
        reviewId: reviewId!,
        githubThreadRootId: rootId!,
        body,
        filePath: thread.path!,
        side: thread.diffSide as Side,
        line: thread.line!,
        startLine: thread.startLine,
      }),
    onSuccess: () => {
      setReplying(false);
      onCommentsChanged?.();
    },
    onError: (e) => toast.error(`Draft reply failed:\n${String(e)}`),
  });

  return (
    <div className="github-thread" data-thread-id={thread.id}>
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
      {expanded && draftReplies && draftReplies.length > 0 && (
        <div className="github-thread-drafts">
          {draftReplies.map((d) => (
            <DraftReplyItem
              key={d.id}
              comment={d}
              ctx={ctx}
              readOnly={readOnly}
              onCommentsChanged={onCommentsChanged}
            />
          ))}
        </div>
      )}
      {expanded && ctx && rootId != null && !readOnly && (
        <div className="github-thread-reply">
          {!replying ? (
            <button
              className="github-thread-action"
              onClick={() => setReplying(true)}
            >
              Reply…
            </button>
          ) : canDraft ? (
            <Composer
              submitLabel="Add draft reply"
              onSubmit={async (text) => {
                await draftReply.mutateAsync(text);
              }}
              secondaryLabel="Reply now"
              onSecondarySubmit={async (text) => {
                await reply.mutateAsync(text);
              }}
              onCancel={() => setReplying(false)}
            />
          ) : (
            <Composer
              submitLabel="Reply now"
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

function DraftReplyItem({
  comment,
  ctx,
  readOnly,
  onCommentsChanged,
}: {
  comment: Comment;
  ctx?: PrThreadCtx | null;
  readOnly?: boolean;
  onCommentsChanged?: () => void;
}) {
  const [body, setBody] = useState(comment.body);
  const queryClient = useQueryClient();
  const save = useDebouncedCallback((text: string) => {
    api.updateDraftReply(comment.id, text).catch((e) => toast.error(String(e)));
  }, 400);

  const publish = useMutation({
    mutationFn: async () => {
      if (body !== comment.body) await api.updateDraftReply(comment.id, body);
      await api.publishDraftReply(comment.id);
    },
    onSuccess: () => {
      onCommentsChanged?.();
      if (ctx) {
        queryClient.invalidateQueries({
          queryKey: ["pr-threads", ctx.owner, ctx.name, ctx.number],
        });
      }
    },
    onError: (e) => toast.error(`Publish draft reply failed:\n${String(e)}`),
  });

  return (
    <div className="github-thread-comment">
      <div className="github-thread-comment-head">
        <span className="draft-badge" title="Not posted to GitHub yet">
          Draft
        </span>
        {!readOnly && (
          <>
            <button
              className="btn btn-primary github-draft-publish"
              disabled={publish.isPending || body.trim() === ""}
              title="Post this reply to GitHub"
              onClick={() => publish.mutate()}
            >
              {publish.isPending ? "Publishing…" : "Publish"}
            </button>
            <button
              className="btn-icon github-draft-delete"
              title="Delete draft reply"
              onClick={async () => {
                if (
                  await confirmDialog({
                    title: "Delete draft reply",
                    message: "Delete this draft reply?",
                    confirmLabel: "Delete",
                    danger: true,
                  })
                ) {
                  await api.deleteDraftReply(comment.id);
                  onCommentsChanged?.();
                }
              }}
            >
              <Icon name="x" size={12} />
            </button>
          </>
        )}
      </div>
      {readOnly ? (
        <Markdown source={comment.body} />
      ) : (
        <textarea
          className="github-draft-textarea"
          value={body}
          onChange={(e) => {
            setBody(e.target.value);
            save(e.target.value);
          }}
        />
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
