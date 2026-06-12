import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Decoration,
  Diff,
  Hunk,
  expandFromRawCode,
  getCollapsedLinesCountBetween,
  parseDiff,
  type ChangeData,
  type FileData,
  type HunkData,
  type ViewType,
} from "react-diff-view";
import { api, pickSavePath } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import {
  anchorByLine,
  changeKeyOf,
  countChanges,
  fileDisplayPath,
  hunkContextSnippet,
  indexFile,
  leadingExpandRange,
  sourceLineCount,
  tokenizeFile,
  trailingExpandRange,
  trailingGap,
} from "../lib/diff";
import { FileJumpList } from "./FileJumpList";
import { FileViewPane } from "./FileViewPane";
import { GithubThread } from "./GithubThread";
import { Markdown } from "./Markdown";
import { OpenPrButton } from "./OpenPrButton";
import { PublishButton } from "./PublishButton";
import { PrMetaPanel } from "./PrMetaPanel";
import { Icon } from "./icons";
import { githubPrUrl } from "../lib/githubUrl";
import { useDebouncedCallback } from "../lib/useDebouncedCallback";
import { useSettingsStore } from "../lib/settings";
import { useUIStore } from "../store";
import { groupThreads, type CommentThread } from "../lib/threads";
import type { Comment, PrThread, ReviewDetail, ReviewEvent, Side } from "../lib/types";

type SaveState = "idle" | "saving" | "saved";

const EXPAND_CHUNK = 20;

export function ReviewView({ reviewId }: { reviewId: number }) {
  const queryClient = useQueryClient();
  const defaultViewType = useSettingsStore((s) => s.defaultViewType);
  const [viewType, setViewType] = useState<ViewType>(defaultViewType);
  const [saveState, setSaveState] = useState<SaveState>("saved");
  const [filePanePath, setFilePanePath] = useState<string | null>(null);
  const diffAreaRef = useRef<HTMLDivElement>(null);

  const detailQuery = useQuery({
    queryKey: ["review", reviewId],
    queryFn: () => api.getReview(reviewId),
  });

  const diffQuery = useQuery({
    queryKey: [
      "review-diff",
      reviewId,
      detailQuery.data?.target.id,
      detailQuery.data?.target.head_sha,
    ],
    enabled: detailQuery.data != null,
    queryFn: () => api.reviewDiff(reviewId),
  });

  const target = detailQuery.data?.target;
  const owner = detailQuery.data?.remote_owner ?? null;
  const name = detailQuery.data?.remote_name ?? null;
  const prNumber = target?.github_pr_number ?? null;
  // Kept separate from ["review", id] so comment-autosave invalidations don't
  // refetch GitHub threads (and vice-versa). Ephemeral; never persisted.
  const threadsQuery = useQuery({
    queryKey: ["pr-threads", owner, name, prNumber],
    enabled:
      target?.kind === "github_pr" && !!owner && !!name && prNumber != null,
    queryFn: () => api.prReviewThreads(owner!, name!, prNumber!),
  });

  if (detailQuery.isLoading) return <section className="main-panel">Loading review…</section>;
  if (detailQuery.isError || !detailQuery.data)
    return (
      <section className="main-panel">
        <p className="error">Could not load review: {String(detailQuery.error)}</p>
      </section>
    );

  const detail = detailQuery.data;
  const readOnly = detail.review.status === "published";

  return (
    <section className="main-panel review-panel">
      <ReviewHeader
        detail={detail}
        saveState={saveState}
        readOnly={readOnly}
        viewType={viewType}
        setViewType={setViewType}
        onSaving={() => setSaveState("saving")}
        onSaved={() => {
          setSaveState("saved");
          queryClient.invalidateQueries({ queryKey: ["reviews"] });
        }}
      />

      <div className="review-body">
        <FileJumpList reviewId={reviewId} scrollRootRef={diffAreaRef} />
        <div className="diff-area" ref={diffAreaRef}>
          {!readOnly && (
            <p className="hint muted">
              Click a line to comment · shift-click another line on the same side to select a range.
            </p>
          )}
          {diffQuery.isLoading && <p className="muted">Loading diff…</p>}
          {diffQuery.isError && <p className="error">Diff failed: {String(diffQuery.error)}</p>}
          {diffQuery.data != null && (
            <ReviewDiff
              diffText={diffQuery.data}
              viewType={viewType}
              detail={detail}
              threads={threadsQuery.data ?? []}
              readOnly={readOnly}
              onOpenFilePane={setFilePanePath}
              onSaving={() => setSaveState("saving")}
              onSaved={() => {
                setSaveState("saved");
                queryClient.invalidateQueries({ queryKey: ["review", reviewId] });
              }}
              onCommentsChanged={() =>
                queryClient.invalidateQueries({ queryKey: ["review", reviewId] })
              }
            />
          )}
        </div>
      </div>

      {filePanePath && (
        <FileViewPane
          reviewId={reviewId}
          detail={detail}
          filePath={filePanePath}
          readOnly={readOnly}
          onClose={() => setFilePanePath(null)}
          onSaving={() => setSaveState("saving")}
          onSaved={() => {
            setSaveState("saved");
            queryClient.invalidateQueries({ queryKey: ["review", reviewId] });
          }}
          onCommentsChanged={() =>
            queryClient.invalidateQueries({ queryKey: ["review", reviewId] })
          }
        />
      )}
    </section>
  );
}

function ReviewHeader({
  detail,
  saveState,
  readOnly,
  viewType,
  setViewType,
  onSaving,
  onSaved,
}: {
  detail: ReviewDetail;
  saveState: SaveState;
  readOnly: boolean;
  viewType: ViewType;
  setViewType: (v: ViewType) => void;
  onSaving: () => void;
  onSaved: () => void;
}) {
  const queryClient = useQueryClient();
  const closeReview = useUIStore((s) => s.closeReview);
  const { review, target } = detail;
  const [body, setBody] = useState(review.body);
  const [showExport, setShowExport] = useState(false);
  const [collapsed, setCollapsed] = useState(true);

  const save = useDebouncedCallback((nextBody: string) => {
    onSaving();
    api
      .updateReview(review.id, nextBody)
      .then(onSaved)
      .catch((e) => toast.error(String(e)));
  }, 400);

  const deleteReview = useMutation({
    mutationFn: () => api.deleteReview(review.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      closeReview();
    },
  });

  const publishReview = useMutation({
    // Persist the chosen verdict (and any unsaved summary) before publishing —
    // the backend reads `review.event` from the DB to build the GitHub payload.
    mutationFn: async (event: ReviewEvent) => {
      await api.updateReview(review.id, body, event);
      return api.publishReview(review.id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["review", review.id] });
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      toast.success("Review published to GitHub.");
      closeReview();
    },
    onError: (e) => toast.error(`Publish failed:\n${String(e)}`),
  });

  const owner = detail.remote_owner ?? null;
  const name = detail.remote_name ?? null;
  const prNumber = target.github_pr_number ?? null;

  const refreshReview = useMutation({
    mutationFn: () => api.refreshReview(review.id),
    onSuccess: (r) => {
      queryClient.invalidateQueries({ queryKey: ["review", review.id] });
      queryClient.invalidateQueries({ queryKey: ["pr-threads", owner, name, prNumber] });
      if (r.headMoved) toast.success("Head moved — re-anchor to update comments.");
    },
    onError: (e) => toast.error(`Refresh failed:\n${String(e)}`),
  });

  const reanchorComments = useMutation({
    mutationFn: () => api.reanchorComments(review.id),
    onSuccess: (r) => {
      queryClient.invalidateQueries({ queryKey: ["review", review.id] });
      toast.success(`Re-anchored ${r.reanchored}, ${r.lost} could not be moved`);
    },
    onError: (e) => toast.error(`Re-anchor failed:\n${String(e)}`),
  });

  const isPr = target.kind === "github_pr";
  const published = review.status === "published";
  const headMoved = detail.comments.some(
    (c) =>
      c.anchored_head_sha &&
      target.head_sha &&
      c.anchored_head_sha !== target.head_sha,
  );
  const prUrl =
    isPr && detail.remote_owner && detail.remote_name && target.github_pr_number != null
      ? githubPrUrl(detail.remote_owner, detail.remote_name, target.github_pr_number)
      : null;
  const sourceRepo = owner && name ? `${owner}/${name}` : null;

  return (
    <header className="review-header">
      <div className="review-header-top">
        <button
          className="btn btn-ghost"
          title={collapsed ? "Show review details" : "Hide review details"}
          onClick={() => setCollapsed((c) => !c)}
        >
          <Icon name="chev" size={13} /> {collapsed ? "Expand" : "Collapse"}
        </button>
        <div className="review-title-wrap">
          <h2 className="review-title">{target.title}</h2>
          {sourceRepo && (
            <span className="review-repo mono" title="Source repository">
              {sourceRepo}
              {prNumber != null ? ` #${prNumber}` : ""}
            </span>
          )}
        </div>
        <span className={`badge ${review.status === "draft" ? "badge-draft" : "badge-pr"}`}>
          {review.status}
        </span>
        <span className="save-state">{saveState === "saving" ? "Saving…" : "Saved"}</span>
        <div className="view-toggle">
          <button
            className={viewType === "split" ? "active" : ""}
            onClick={() => setViewType("split")}
          >
            Split
          </button>
          <button
            className={viewType === "unified" ? "active" : ""}
            onClick={() => setViewType("unified")}
          >
            Unified
          </button>
        </div>
        {prUrl && <OpenPrButton url={prUrl} />}
        {headMoved && (
          <span className="head-moved">
            <span
              className="head-moved-badge"
              title="The head has moved since some comments were written — they may no longer line up with the code."
            >
              ⚠ head moved
            </span>
            <button
              className="btn btn-sm"
              disabled={readOnly || reanchorComments.isPending}
              title={
                readOnly
                  ? "Published reviews cannot be re-anchored"
                  : "Move comments onto the current head and clear outdated badges"
              }
              onClick={() => reanchorComments.mutate()}
            >
              {reanchorComments.isPending ? "Re-anchoring…" : "Re-anchor comments"}
            </button>
          </span>
        )}
        <button
          className="btn"
          disabled={readOnly || refreshReview.isPending}
          title={
            readOnly
              ? "Published reviews cannot be refreshed"
              : "Re-resolve the head/base and refresh the diff"
          }
          onClick={() => refreshReview.mutate()}
        >
          {refreshReview.isPending ? (
            <>
              <span className="spinner" /> Refreshing…
            </>
          ) : (
            <>
              <Icon name="refresh" size={13} /> Refresh
            </>
          )}
        </button>
        <button className="btn" onClick={() => setShowExport(true)}>
          Export
        </button>
        {isPr && (
          <PublishButton
            published={published}
            pending={publishReview.isPending}
            onPublish={(event) => publishReview.mutate(event)}
          />
        )}
        <button
          className="btn btn-danger"
          onClick={async () => {
            if (
              await confirmDialog({
                title: "Delete review",
                message: "Delete this review and all its comments?",
                confirmLabel: "Delete",
                danger: true,
              })
            )
              deleteReview.mutate();
          }}
        >
          Delete
        </button>
      </div>

      {showExport && (
        <ExportModal
          reviewId={review.id}
          title={target.title}
          onClose={() => setShowExport(false)}
          onExported={() => {
            queryClient.invalidateQueries({ queryKey: ["review", review.id] });
            queryClient.invalidateQueries({ queryKey: ["reviews"] });
          }}
        />
      )}

      {!collapsed && (
        <>
          {isPr && detail.remote_owner && detail.remote_name && target.github_pr_number != null && (
            <PrMetaPanel
              owner={detail.remote_owner}
              name={detail.remote_name}
              number={target.github_pr_number}
            />
          )}

          <div className="review-summary">
            <textarea
              className="textarea"
              placeholder="Review summary…"
              value={body}
              disabled={readOnly}
              onChange={(e) => {
                setBody(e.target.value);
                save(e.target.value);
              }}
            />
          </div>
        </>
      )}
    </header>
  );
}

function ReviewDiff({
  diffText,
  viewType,
  detail,
  threads,
  readOnly,
  onOpenFilePane,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  diffText: string;
  viewType: ViewType;
  detail: ReviewDetail;
  threads: PrThread[];
  readOnly: boolean;
  onOpenFilePane: (path: string) => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const files = useMemo(() => parseDiff(diffText), [diffText]);

  return (
    <div className="diff-files">
      {files.map((file, index) => (
        <FileReview
          key={`${file.oldRevision}-${file.newRevision}-${index}`}
          index={index}
          file={file}
          viewType={viewType}
          detail={detail}
          threads={threads}
          readOnly={readOnly}
          onOpenFilePane={onOpenFilePane}
          onSaving={onSaving}
          onSaved={onSaved}
          onCommentsChanged={onCommentsChanged}
        />
      ))}
    </div>
  );
}

interface Selection {
  side: Side;
  anchorLine: number;
  focusLine: number;
  focusKey: string;
}

function FileReview({
  index,
  file,
  viewType,
  detail,
  threads,
  readOnly,
  onOpenFilePane,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  index: number;
  file: FileData;
  viewType: ViewType;
  detail: ReviewDetail;
  threads: PrThread[];
  readOnly: boolean;
  onOpenFilePane: (path: string) => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const queryClient = useQueryClient();
  const reviewId = detail.review.id;
  const path = fileDisplayPath(file);
  const isDeleted = file.type === "delete";
  const openInDefaultApp = async () => {
    if (isDeleted) return;
    const fullPath = `${detail.repo_path}/${path}`;
    try {
      await api.openInDefaultApp(fullPath);
    } catch (err) {
      toast.error(`Could not open file:\n${String(err)}`);
    }
  };

  // Hunks the user has expanded sit alongside the parsed ones; this state feeds
  // BOTH rendering and anchoring so revealed lines are clickable/commentable.
  // Ephemeral by design — reparsing (a new diff) resets it.
  const [hunks, setHunks] = useState<HunkData[]>(file.hunks);
  useEffect(() => setHunks(file.hunks), [file]);
  const [rawSource, setRawSource] = useState<string | null>(null);
  const [expanding, setExpanding] = useState(false);
  const [expandError, setExpandError] = useState<string | null>(null);

  // react-diff-view's expansion math works in OLD/LEFT line numbers, so the raw
  // source must be the base file. Added/deleted/binary files have no usable base
  // side. For GitHub-PR targets LEFT is the merge-base blob, served via the
  // backend's base_sha (lazily backfilled) + contents-API fallback.
  const canExpand =
    file.type !== "add" && file.type !== "delete" && !file.isBinary;

  const ensureSource = useCallback(async (): Promise<string | null> => {
    if (rawSource != null) return rawSource;
    setExpanding(true);
    try {
      const src = await api.fileSource(detail.review.id, file.oldPath, "LEFT");
      setRawSource(src);
      return src;
    } catch (e) {
      setExpandError(String(e));
      return null;
    } finally {
      setExpanding(false);
    }
  }, [rawSource, detail.review.id, file.oldPath]);

  const expandBetween = useCallback(
    async (prev: HunkData, next: HunkData, n: number) => {
      const src = await ensureSource();
      if (src == null) return;
      const gapStart = prev.oldStart + prev.oldLines;
      const gapEnd = gapStart + getCollapsedLinesCountBetween(prev, next);
      const end = Math.min(gapStart + n, gapEnd);
      setHunks((h) => expandFromRawCode(h, src, gapStart, end));
    },
    [ensureSource],
  );

  // Edge expansion ranges are computed inside the setHunks updater from the
  // updater's own `h` (current first/last hunk): `await ensureSource()` yields,
  // so hunks may have changed (another expander clicked) by the time we run.
  const expandLeading = useCallback(
    async (n: number) => {
      const src = await ensureSource();
      if (src == null) return;
      setHunks((h) => {
        if (h.length === 0) return h;
        const [start, end] = leadingExpandRange(h[0], n);
        return start < end ? expandFromRawCode(h, src, start, end) : h;
      });
    },
    [ensureSource],
  );

  const expandTrailing = useCallback(
    async (n: number) => {
      const src = await ensureSource();
      if (src == null) return;
      setHunks((h) => {
        if (h.length === 0) return h;
        const [start, end] = trailingExpandRange(h[h.length - 1], sourceLineCount(src), n);
        return start < end ? expandFromRawCode(h, src, start, end) : h;
      });
    },
    [ensureSource],
  );

  // null until the base source is fetched — the trailing gap size is unknown.
  const oldLineCount = useMemo(
    () => (rawSource == null ? null : sourceLineCount(rawSource)),
    [rawSource],
  );

  const { metaByKey, keyByAnchor } = useMemo(
    () => indexFile({ ...file, hunks }),
    [file, hunks],
  );
  const tokens = useMemo(() => tokenizeFile({ ...file, hunks }), [file, hunks]);
  const [selection, setSelection] = useState<Selection | null>(null);
  const viewed = detail.viewed_files.includes(path);
  const [fileComposerOpen, setFileComposerOpen] = useState(false);

  const setViewed = useMutation({
    mutationFn: (v: boolean) => api.setFileViewed(reviewId, path, v),
    onMutate: async (v: boolean) => {
      const key = ["review", reviewId];
      await queryClient.cancelQueries({ queryKey: key });
      const prev = queryClient.getQueryData<ReviewDetail>(key);
      queryClient.setQueryData<ReviewDetail>(key, (old) =>
        old
          ? {
              ...old,
              viewed_files: v
                ? [...old.viewed_files, path]
                : old.viewed_files.filter((p) => p !== path),
            }
          : old,
      );
      return { prev };
    },
    onError: (err, _v, ctx) => {
      if (ctx?.prev) queryClient.setQueryData(["review", reviewId], ctx.prev);
      toast.error(String(err));
    },
  });

  // Comments attached to the whole file rather than a specific line, grouped into
  // threads so pre-existing replies to file comments still nest (no reply
  // affordance on these per spec).
  const fileComments = useMemo(
    () =>
      groupThreads(
        detail.comments.filter((c) => c.file_path === path && c.subject_type === "file"),
      ),
    [detail.comments, path],
  );

  // Group this file's line comments into threads, then key each thread by the
  // root's anchor.
  const { commentsByKey, orphans } = useMemo(() => {
    const byKey = new Map<string, CommentThread[]>();
    const orphan: CommentThread[] = [];
    const lineComments = detail.comments.filter(
      (c) =>
        c.file_path === path &&
        c.subject_type !== "file" &&
        c.origin !== "file_view", // shown in the full-file pane, not the diff
    );
    for (const thread of groupThreads(lineComments)) {
      const key = keyByAnchor.get(`${thread.root.side}:${thread.root.line}`);
      if (!key) {
        orphan.push(thread);
        continue;
      }
      const arr = byKey.get(key) ?? [];
      arr.push(thread);
      byKey.set(key, arr);
    }
    return { commentsByKey: byKey, orphans: orphan };
  }, [detail.comments, path, keyByAnchor]);

  // Existing GitHub review threads on this file: anchored ones become inline
  // widgets, the rest (outdated, file-level, or off the current diff) drop into
  // a "from GitHub" orphan block. Read-only — never converted to comment rows.
  const { byKey: threadsByKey, orphans: orphanThreads } = useMemo(() => {
    const fileThreads = threads.filter((t) => t.path === path);
    return anchorByLine(
      fileThreads,
      (t) => ({ side: t.diffSide ?? "", line: t.line }),
      keyByAnchor,
    );
  }, [threads, path, keyByAnchor]);

  // Click a line to start a 1-line selection; shift-click another line on the
  // same side to extend it into a range (the composer sits on the focus line).
  const onLineClick = (
    args: { change: ChangeData | null },
    event: React.MouseEvent,
  ) => {
    if (readOnly || !args.change) return;
    const key = changeKeyOf(args.change);
    const meta = metaByKey.get(key);
    if (!meta) return;
    setSelection((prev) => {
      if (event?.shiftKey && prev && prev.side === meta.side) {
        return { ...prev, focusLine: meta.line, focusKey: key };
      }
      return { side: meta.side, anchorLine: meta.line, focusLine: meta.line, focusKey: key };
    });
  };

  const range = selection
    ? {
        lo: Math.min(selection.anchorLine, selection.focusLine),
        hi: Math.max(selection.anchorLine, selection.focusLine),
      }
    : null;

  // Highlight every change inside the selected range (used by react-diff-view).
  const selectedChanges = useMemo(() => {
    if (!selection || !range) return [];
    const keys: string[] = [];
    for (const [k, m] of metaByKey) {
      if (m.side === selection.side && m.line >= range.lo && m.line <= range.hi) keys.push(k);
    }
    return keys;
  }, [selection, range, metaByKey]);

  const submitSelectionComment = async (text: string) => {
    if (!selection || !range) return;
    const header = metaByKey.get(selection.focusKey)?.hunk ?? "";
    // Store the surrounding hunk lines (not just the commented ones) so the
    // export diff block carries context. Fall back to the bare selected lines if
    // the focus hunk can't be located.
    const focusHunk = hunks.find((h) => h.content === header);
    const diffHunk = focusHunk
      ? hunkContextSnippet(focusHunk, selection.side, range.lo, range.hi)
      : [
          header,
          ...[...metaByKey.values()]
            .filter((m) => m.side === selection.side && m.line >= range.lo && m.line <= range.hi)
            .sort((a, b) => a.line - b.line)
            .map((m) => m.lineText),
        ].join("\n");
    onSaving();
    await api.addComment({
      reviewId: detail.review.id,
      filePath: path,
      side: selection.side,
      line: range.hi,
      startLine: range.lo === range.hi ? null : range.lo,
      diffHunk,
      body: text,
      anchoredHeadSha: detail.target.head_sha,
    });
    setSelection(null);
    onSaved();
    onCommentsChanged();
  };

  const submitFileComment = async (text: string) => {
    onSaving();
    await api.addFileComment({ reviewId: detail.review.id, filePath: path, body: text });
    setFileComposerOpen(false);
    onSaved();
    onCommentsChanged();
  };

  const widgets: Record<string, React.ReactNode> = {};
  const keys = new Set<string>(commentsByKey.keys());
  for (const key of threadsByKey.keys()) keys.add(key);
  if (selection) keys.add(selection.focusKey);
  for (const key of keys) {
    const composerOpen = !readOnly && selection?.focusKey === key;
    const rangeLabel =
      composerOpen && range && range.lo !== range.hi
        ? `Lines ${range.lo}–${range.hi} (${selection!.side})`
        : undefined;
    const keyThreads = threadsByKey.get(key) ?? [];
    widgets[key] = (
      <>
        <LineWidget
          threads={commentsByKey.get(key) ?? []}
          headSha={detail.target.head_sha}
          composerOpen={!!composerOpen}
          rangeLabel={rangeLabel}
          readOnly={readOnly}
          canReply={!readOnly}
          onCloseComposer={() => setSelection(null)}
          onAdd={submitSelectionComment}
          onSaving={onSaving}
          onSaved={onSaved}
          onCommentsChanged={onCommentsChanged}
        />
        {keyThreads.map((t) => (
          <GithubThread key={t.id} thread={t} />
        ))}
      </>
    );
  }

  const { add, del } = useMemo(() => countChanges(file), [file]);

  return (
    <div className="diff-file" id={`file-${index}`}>
      <div className="diff-file-header">
        <span className="file-path mono">{path}</span>
        <span className="diff-stats">
          <span className="delta-add">+{add}</span>
          <span className="delta-del">−{del}</span>
          {!readOnly && (
            <button
              className="btn btn-sm btn-ghost file-comment-btn"
              title="Add a comment on the whole file"
              onClick={() => setFileComposerOpen(true)}
            >
              <Icon name="comment" size={12} /> Comment on file
            </button>
          )}
          <button
            className="btn btn-sm btn-ghost view-file-btn"
            title={
              isDeleted
                ? "File was deleted; no head version to view"
                : file.isBinary
                  ? "Binary file; nothing to view"
                  : "View the full file and comment on any line"
            }
            disabled={isDeleted || file.isBinary}
            onClick={() => onOpenFilePane(path)}
          >
            <Icon name="eye" size={12} /> View file
          </button>
          <button
            className="btn btn-sm btn-ghost open-file-btn"
            title={
              isDeleted
                ? "File was deleted; no working copy to open"
                : "Open working copy in default app"
            }
            disabled={isDeleted}
            onClick={openInDefaultApp}
          >
            <Icon name="ext" size={12} /> Open
          </button>
          <label className="viewed-toggle check" title="Collapse this file">
            <input
              type="checkbox"
              checked={viewed}
              onChange={(e) => setViewed.mutate(e.target.checked)}
            />
            Viewed
          </label>
        </span>
      </div>
      {(fileComments.length > 0 || fileComposerOpen) && (
        <div className="file-comments">
          {fileComments.map((t) => (
            <ThreadItem
              key={t.root.id}
              thread={t}
              headSha={detail.target.head_sha}
              readOnly={readOnly}
              canReply={false}
              onSaving={onSaving}
              onSaved={onSaved}
              onCommentsChanged={onCommentsChanged}
            />
          ))}
          {fileComposerOpen && !readOnly && (
            <Composer
              onSubmit={submitFileComment}
              onCancel={() => setFileComposerOpen(false)}
              rangeLabel={`Whole file · ${path}`}
            />
          )}
        </div>
      )}
      {viewed ? null : (
        <FileBody
          file={file}
          hunks={hunks}
          viewType={viewType}
          tokens={tokens}
          widgets={widgets}
          selectedChanges={selectedChanges}
          orphans={orphans}
          orphanThreads={orphanThreads}
          headSha={detail.target.head_sha}
          readOnly={readOnly}
          canExpand={canExpand}
          expanding={expanding}
          expandError={expandError}
          oldLineCount={oldLineCount}
          onExpandBetween={expandBetween}
          onExpandLeading={expandLeading}
          onExpandTrailing={expandTrailing}
          onLineClick={onLineClick}
          onSaving={onSaving}
          onSaved={onSaved}
          onCommentsChanged={onCommentsChanged}
        />
      )}
    </div>
  );
}

function FileBody({
  file,
  hunks,
  viewType,
  tokens,
  widgets,
  selectedChanges,
  orphans,
  orphanThreads,
  headSha,
  readOnly,
  canExpand,
  expanding,
  expandError,
  oldLineCount,
  onExpandBetween,
  onExpandLeading,
  onExpandTrailing,
  onLineClick,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  file: FileData;
  hunks: HunkData[];
  viewType: ViewType;
  tokens: ReturnType<typeof tokenizeFile>;
  widgets: Record<string, React.ReactNode>;
  selectedChanges: string[];
  orphans: CommentThread[];
  orphanThreads: PrThread[];
  headSha: string | null;
  readOnly: boolean;
  canExpand: boolean;
  expanding: boolean;
  expandError: string | null;
  oldLineCount: number | null; // null until the base source is fetched
  onExpandBetween: (prev: HunkData, next: HunkData, n: number) => void;
  onExpandLeading: (n: number) => void;
  onExpandTrailing: (n: number) => void;
  onLineClick: (args: { change: ChangeData | null }, event: React.MouseEvent) => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  return (
    <>
      {expandError && (
        <p className="muted expand-error-note">
          Context expansion unavailable: {expandError}
        </p>
      )}
      {orphans.length > 0 && (
        <div className="orphan-comments">
          <p className="muted">
            Comments not matching the current diff (head may have changed):
          </p>
          {orphans.map((t) => (
            <ThreadItem
              key={t.root.id}
              thread={t}
              headSha={headSha}
              readOnly={readOnly}
              canReply={false}
              onSaving={onSaving}
              onSaved={onSaved}
              onCommentsChanged={onCommentsChanged}
            />
          ))}
        </div>
      )}
      {orphanThreads.length > 0 && (
        <div className="github-orphan-threads">
          <p className="muted">GitHub threads not on the current diff:</p>
          {orphanThreads.map((t) => (
            <GithubThread key={t.id} thread={t} />
          ))}
        </div>
      )}
      {file.isBinary ? (
        <p className="muted binary-note">Binary file not shown.</p>
      ) : (
        <Diff
          viewType={viewType}
          diffType={file.type}
          hunks={hunks}
          tokens={tokens}
          widgets={widgets}
          selectedChanges={selectedChanges}
          gutterEvents={{ onClick: onLineClick }}
          codeEvents={{ onClick: onLineClick }}
        >
          {(renderedHunks) =>
            renderedHunks.flatMap((hunk, i) => {
              // All three gap kinds expand: leading (above the first hunk),
              // between hunks, and trailing (below the last hunk — its size is
              // unknown until the base source is fetched, so it renders
              // optimistically with no count).
              const prev = i === 0 ? null : renderedHunks[i - 1];
              const collapsed = getCollapsedLinesCountBetween(prev, hunk);
              const rows: React.ReactElement[] = [];
              if (canExpand && prev == null && collapsed > 0) {
                rows.push(
                  <Decoration key="exp-top" className="diff-expander">
                    <ExpandControl
                      count={collapsed}
                      direction="up"
                      busy={expanding}
                      onExpandChunk={() => onExpandLeading(EXPAND_CHUNK)}
                      onExpandAll={() => onExpandLeading(Number.POSITIVE_INFINITY)}
                    />
                  </Decoration>,
                );
              }
              if (canExpand && prev != null && collapsed > 0) {
                rows.push(
                  <Decoration key={`exp-${hunk.content}`} className="diff-expander">
                    <ExpandControl
                      count={collapsed}
                      busy={expanding}
                      onExpandChunk={() => onExpandBetween(prev, hunk, EXPAND_CHUNK)}
                      onExpandAll={() => onExpandBetween(prev, hunk, collapsed)}
                    />
                  </Decoration>,
                );
              }
              rows.push(<Hunk key={hunk.content} hunk={hunk} />);
              if (
                canExpand &&
                i === renderedHunks.length - 1 &&
                (oldLineCount == null || trailingGap(hunk, oldLineCount) > 0)
              ) {
                rows.push(
                  <Decoration key="exp-bottom" className="diff-expander">
                    <ExpandControl
                      count={oldLineCount == null ? null : trailingGap(hunk, oldLineCount)}
                      direction="down"
                      busy={expanding}
                      onExpandChunk={() => onExpandTrailing(EXPAND_CHUNK)}
                      onExpandAll={() => onExpandTrailing(Number.POSITIVE_INFINITY)}
                    />
                  </Decoration>,
                );
              }
              return rows;
            })
          }
        </Diff>
      )}
    </>
  );
}

function ExpandControl({
  count,
  busy,
  direction = "between",
  onExpandChunk,
  onExpandAll,
}: {
  /** null: size unknown (trailing gap before the base source is fetched). */
  count: number | null;
  busy: boolean;
  direction?: "up" | "down" | "between";
  onExpandChunk: () => void;
  onExpandAll: () => void;
}) {
  const arrow = direction === "up" ? "↑ " : direction === "down" ? "↓ " : "";
  return (
    <div className="expand-control">
      {busy ? (
        <span className="muted">Expanding…</span>
      ) : (
        <>
          <span className="expand-label">
            {count != null ? `⋯ ${count} hidden lines` : "⋯ hidden lines"}
          </span>
          {(count == null || count > EXPAND_CHUNK) && (
            <button className="expand-btn" onClick={onExpandChunk} disabled={busy}>
              {arrow}
              {EXPAND_CHUNK} lines
            </button>
          )}
          <button className="expand-btn" onClick={onExpandAll} disabled={busy}>
            all
          </button>
        </>
      )}
    </div>
  );
}

export function LineWidget({
  threads,
  headSha,
  composerOpen,
  rangeLabel,
  readOnly,
  showOrigin,
  canReply = true,
  onCloseComposer,
  onAdd,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  threads: CommentThread[];
  headSha: string | null;
  composerOpen: boolean;
  rangeLabel?: string;
  readOnly: boolean;
  showOrigin?: boolean;
  canReply?: boolean;
  onCloseComposer: () => void;
  onAdd: (text: string) => Promise<void>;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  return (
    <div className="line-widget">
      {threads.map((t) => (
        <ThreadItem
          key={t.root.id}
          thread={t}
          headSha={headSha}
          readOnly={readOnly}
          showOrigin={showOrigin}
          canReply={canReply && !readOnly}
          onSaving={onSaving}
          onSaved={onSaved}
          onCommentsChanged={onCommentsChanged}
        />
      ))}
      {composerOpen && !readOnly && (
        <Composer onSubmit={onAdd} onCancel={onCloseComposer} rangeLabel={rangeLabel} />
      )}
    </div>
  );
}

/**
 * A root comment plus its (one level of) replies, indented, with a Reply
 * affordance on anchored threads. Replies inherit the root's anchor server-side,
 * so the composer only needs the root id.
 */
export function ThreadItem({
  thread,
  headSha,
  readOnly,
  showOrigin,
  canReply,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  thread: CommentThread;
  headSha: string | null;
  readOnly: boolean;
  showOrigin?: boolean;
  canReply?: boolean;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const [replyOpen, setReplyOpen] = useState(false);
  const { root, replies } = thread;

  const submitReply = async (text: string) => {
    onSaving();
    await api.addReply({ reviewId: root.review_id, parentId: root.id, body: text });
    setReplyOpen(false);
    onSaved();
    onCommentsChanged();
  };

  return (
    <div className="comment-thread">
      <CommentItem
        comment={root}
        headSha={headSha}
        readOnly={readOnly}
        showOrigin={showOrigin}
        replyCount={replies.length}
        onReply={canReply ? () => setReplyOpen(true) : undefined}
        onSaving={onSaving}
        onSaved={onSaved}
        onCommentsChanged={onCommentsChanged}
      />
      {replies.length > 0 && (
        <div className="comment-replies">
          {replies.map((r) => (
            <CommentItem
              key={r.id}
              comment={r}
              headSha={headSha}
              readOnly={readOnly}
              showOrigin={showOrigin}
              onSaving={onSaving}
              onSaved={onSaved}
              onCommentsChanged={onCommentsChanged}
            />
          ))}
        </div>
      )}
      {replyOpen && !readOnly && (
        <Composer
          onSubmit={submitReply}
          onCancel={() => setReplyOpen(false)}
          submitLabel="Reply"
        />
      )}
    </div>
  );
}

export function CommentItem({
  comment,
  headSha,
  readOnly,
  showOrigin,
  onReply,
  replyCount,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  comment: Comment;
  headSha: string | null;
  readOnly: boolean;
  showOrigin?: boolean;
  onReply?: () => void;
  replyCount?: number;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const [body, setBody] = useState(comment.body);
  const [tab, setTab] = useState<"write" | "preview">("write");
  const save = useDebouncedCallback((text: string) => {
    onSaving();
    api
      .updateComment(comment.id, text)
      .then(onSaved)
      .catch((e) => toast.error(String(e)));
  }, 400);

  // The comment was anchored to a head that has since moved, so its line may no
  // longer point at the code it was written against. Flag it rather than
  // silently mislanding it (real re-anchoring is future work — see ROADMAP §2).
  const outdated =
    !!comment.anchored_head_sha &&
    !!headSha &&
    comment.anchored_head_sha !== headSha;

  return (
    <div className="comment-item">
      {showOrigin && comment.origin === "file_view" && (
        <span
          className="origin-badge"
          title="Authored in the full-file view — folds into the review summary on publish, not posted as an inline GitHub comment."
        >
          in summary
        </span>
      )}
      {outdated && (
        <span
          className="stale-badge"
          title={`Anchored to ${comment.anchored_head_sha!.slice(
            0,
            7,
          )}, but the head has since moved — this comment may no longer line up with the code.`}
        >
          outdated · {comment.anchored_head_sha!.slice(0, 7)}
        </span>
      )}
      {readOnly ? (
        <Markdown source={comment.body} />
      ) : (
        <div className="comment-edit">
          <div className="comment-edit-tabs">
            <button
              className={tab === "write" ? "active" : ""}
              onClick={() => setTab("write")}
            >
              Write
            </button>
            <button
              className={tab === "preview" ? "active" : ""}
              onClick={() => setTab("preview")}
            >
              Preview
            </button>
          </div>
          {tab === "write" ? (
            <textarea
              value={body}
              onChange={(e) => {
                setBody(e.target.value);
                save(e.target.value);
              }}
            />
          ) : (
            <Markdown source={body} />
          )}
        </div>
      )}
      {!readOnly && onReply && (
        <button className="btn-sm btn-ghost comment-reply-btn" onClick={onReply}>
          Reply
        </button>
      )}
      {!readOnly && (
        <button
          className="btn-icon"
          title="Delete comment"
          onClick={async () => {
            const message =
              replyCount && replyCount > 0
                ? `Delete this comment and its ${replyCount} ${
                    replyCount === 1 ? "reply" : "replies"
                  }?`
                : "Delete this comment?";
            if (
              await confirmDialog({
                title: "Delete comment",
                message,
                confirmLabel: "Delete",
                danger: true,
              })
            ) {
              await api.deleteComment(comment.id);
              onCommentsChanged();
            }
          }}
        >
          <Icon name="x" size={12} />
        </button>
      )}
    </div>
  );
}

export function Composer({
  onSubmit,
  onCancel,
  rangeLabel,
  submitLabel = "Add comment",
}: {
  onSubmit: (text: string) => Promise<void>;
  onCancel: () => void;
  rangeLabel?: string;
  submitLabel?: string;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <div className="composer">
      {rangeLabel && <div className="composer-range">{rangeLabel}</div>}
      <textarea
        autoFocus
        placeholder="Leave a comment…"
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="composer-actions">
        <button className="btn btn-ghost" onClick={onCancel}>
          Cancel
        </button>
        <button
          className="btn btn-primary"
          disabled={busy || text.trim() === ""}
          onClick={async () => {
            setBusy(true);
            try {
              await onSubmit(text.trim());
            } finally {
              setBusy(false);
            }
          }}
        >
          {submitLabel}
        </button>
      </div>
    </div>
  );
}

function safeFileName(title: string): string {
  return title.replace(/[^a-zA-Z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "review";
}

function ExportModal({
  reviewId,
  title,
  onClose,
  onExported,
}: {
  reviewId: number;
  title: string;
  onClose: () => void;
  onExported: () => void;
}) {
  const previewQuery = useQuery({
    queryKey: ["preview", reviewId, "markdown"],
    queryFn: () => api.previewReview(reviewId, "markdown"),
  });

  const doExport = async (format: "markdown" | "json") => {
    const ext = format === "markdown" ? "md" : "json";
    const path = await pickSavePath(`${safeFileName(title)}.${ext}`, ext);
    if (!path) return;
    try {
      await api.exportReview(reviewId, path, format);
      onExported();
      toast.success(`Exported to ${path}`);
      onClose();
    } catch (e) {
      toast.error(String(e));
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>Export review</h3>
          <button className="btn-icon" onClick={onClose}>
            ✕
          </button>
        </div>
        <p className="muted">Markdown preview (AI-readable). Export is repeatable.</p>
        <pre className="export-preview">
          {previewQuery.isLoading ? "Loading…" : previewQuery.data}
        </pre>
        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button onClick={() => doExport("json")}>Save JSON</button>
          <button className="btn-primary" onClick={() => doExport("markdown")}>
            Save Markdown
          </button>
        </div>
      </div>
    </div>
  );
}
