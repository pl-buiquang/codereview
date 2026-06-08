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
  changeKeyOf,
  countChanges,
  fileDisplayPath,
  hunkContextSnippet,
  indexFile,
  tokenizeFile,
} from "../lib/diff";
import { FileJumpList } from "./FileJumpList";
import { FileViewPane } from "./FileViewPane";
import { OpenPrButton } from "./OpenPrButton";
import { githubPrUrl } from "../lib/githubUrl";
import { useDebouncedCallback } from "../lib/useDebouncedCallback";
import { useSettingsStore } from "../lib/settings";
import { useUIStore } from "../store";
import type { Comment, ReviewDetail, ReviewEvent, Side } from "../lib/types";

type SaveState = "idle" | "saving" | "saved";

const EXPAND_CHUNK = 20;

const VERDICTS: { value: ReviewEvent; label: string }[] = [
  { value: "comment", label: "Comment" },
  { value: "approve", label: "Approve" },
  { value: "request_changes", label: "Request changes" },
];

export function ReviewView({ reviewId }: { reviewId: number }) {
  const queryClient = useQueryClient();
  const closeReview = useUIStore((s) => s.closeReview);
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
    queryKey: ["review-diff", reviewId, detailQuery.data?.target.id],
    enabled: detailQuery.data != null,
    queryFn: () => api.reviewDiff(reviewId),
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
        onBack={closeReview}
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
  onBack,
  onSaving,
  onSaved,
}: {
  detail: ReviewDetail;
  saveState: SaveState;
  readOnly: boolean;
  viewType: ViewType;
  setViewType: (v: ViewType) => void;
  onBack: () => void;
  onSaving: () => void;
  onSaved: () => void;
}) {
  const queryClient = useQueryClient();
  const closeReview = useUIStore((s) => s.closeReview);
  const { review, target } = detail;
  const [body, setBody] = useState(review.body);
  const [event, setEvent] = useState<ReviewEvent | "">(review.event ?? "");
  const [showExport, setShowExport] = useState(false);

  const save = useDebouncedCallback((nextBody: string, nextEvent: string) => {
    onSaving();
    api
      .updateReview(review.id, nextBody, nextEvent)
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
    mutationFn: () => api.publishReview(review.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["review", review.id] });
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      toast.success("Review published to GitHub.");
    },
    onError: (e) => toast.error(`Publish failed:\n${String(e)}`),
  });

  const isPr = target.kind === "github_pr";
  const published = review.status === "published";
  const prUrl =
    isPr && detail.remote_owner && detail.remote_name && target.github_pr_number != null
      ? githubPrUrl(detail.remote_owner, detail.remote_name, target.github_pr_number)
      : null;

  return (
    <header className="review-header">
      <div className="review-header-top">
        <button onClick={onBack}>← Back</button>
        <h2 className="review-title">{target.title}</h2>
        <span className={`status-badge ${review.status}`}>{review.status}</span>
        <span className="save-state">
          {saveState === "saving" ? "Saving…" : "Saved"}
        </span>
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
        <button onClick={() => setShowExport(true)}>Export</button>
        {isPr && (
          <button
            className="btn-primary"
            disabled={published || publishReview.isPending}
            title={
              published
                ? "Already published — cannot publish again"
                : "Post this review to the GitHub PR"
            }
            onClick={async () => {
              if (
                await confirmDialog({
                  title: "Publish review",
                  message: "Publish this review to the GitHub PR? This cannot be undone.",
                  confirmLabel: "Publish",
                  danger: true,
                })
              )
                publishReview.mutate();
            }}
          >
            {publishReview.isPending ? "Publishing…" : published ? "Published" : "Publish"}
          </button>
        )}
        <button
          className="btn-danger"
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

      <div className="review-summary">
        <textarea
          placeholder="Review summary…"
          value={body}
          disabled={readOnly}
          onChange={(e) => {
            setBody(e.target.value);
            save(e.target.value, event);
          }}
        />
        <div className="verdict">
          <span className="muted">Verdict:</span>
          {VERDICTS.map((v) => (
            <label key={v.value} className="verdict-option">
              <input
                type="radio"
                name="verdict"
                disabled={readOnly}
                checked={event === v.value}
                onChange={() => {
                  setEvent(v.value);
                  save(body, v.value);
                }}
              />
              {v.label}
            </label>
          ))}
        </div>
      </div>
    </header>
  );
}

function ReviewDiff({
  diffText,
  viewType,
  detail,
  readOnly,
  onOpenFilePane,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  diffText: string;
  viewType: ViewType;
  detail: ReviewDetail;
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
  // side; GitHub PR targets are rejected server-side (v1 — see file_source).
  const canExpand =
    detail.target.kind === "local" &&
    file.type !== "add" &&
    file.type !== "delete" &&
    !file.isBinary;

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

  // Comments attached to the whole file rather than a specific line.
  const fileComments = useMemo(
    () => detail.comments.filter((c) => c.file_path === path && c.subject_type === "file"),
    [detail.comments, path],
  );

  // Group this file's line comments by the change key they anchor to.
  const { commentsByKey, orphans } = useMemo(() => {
    const byKey = new Map<string, Comment[]>();
    const orphan: Comment[] = [];
    for (const c of detail.comments) {
      if (c.file_path !== path || c.subject_type === "file") continue;
      if (c.origin === "file_view") continue; // shown in the full-file pane, not the diff
      const key = keyByAnchor.get(`${c.side}:${c.line}`);
      if (!key) {
        orphan.push(c);
        continue;
      }
      const arr = byKey.get(key) ?? [];
      arr.push(c);
      byKey.set(key, arr);
    }
    return { commentsByKey: byKey, orphans: orphan };
  }, [detail.comments, path, keyByAnchor]);

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
  if (selection) keys.add(selection.focusKey);
  for (const key of keys) {
    const composerOpen = !readOnly && selection?.focusKey === key;
    const rangeLabel =
      composerOpen && range && range.lo !== range.hi
        ? `Lines ${range.lo}–${range.hi} (${selection!.side})`
        : undefined;
    widgets[key] = (
      <LineWidget
        comments={commentsByKey.get(key) ?? []}
        headSha={detail.target.head_sha}
        composerOpen={!!composerOpen}
        rangeLabel={rangeLabel}
        readOnly={readOnly}
        onCloseComposer={() => setSelection(null)}
        onAdd={submitSelectionComment}
        onSaving={onSaving}
        onSaved={onSaved}
        onCommentsChanged={onCommentsChanged}
      />
    );
  }

  const { add, del } = useMemo(() => countChanges(file), [file]);

  return (
    <div className="diff-file" id={`file-${index}`}>
      <div className="diff-file-header">
        <span className="file-path">{path}</span>
        <span className="diff-stats">
          <span className="add">+{add}</span>
          <span className="del">−{del}</span>
          {!readOnly && (
            <button
              className="file-comment-btn"
              title="Add a comment on the whole file"
              onClick={() => setFileComposerOpen(true)}
            >
              💬 Comment on file
            </button>
          )}
          <button
            className="view-file-btn"
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
            View file
          </button>
          <button
            className="open-file-btn"
            title={
              isDeleted
                ? "File was deleted; no working copy to open"
                : "Open working copy in default app"
            }
            disabled={isDeleted}
            onClick={openInDefaultApp}
          >
            Open
          </button>
          <label className="viewed-toggle" title="Collapse this file">
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
          {fileComments.map((c) => (
            <CommentItem
              key={c.id}
              comment={c}
              headSha={detail.target.head_sha}
              readOnly={readOnly}
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
          headSha={detail.target.head_sha}
          readOnly={readOnly}
          canExpand={canExpand}
          expanding={expanding}
          expandError={expandError}
          onExpandBetween={expandBetween}
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
  headSha,
  readOnly,
  canExpand,
  expanding,
  expandError,
  onExpandBetween,
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
  orphans: Comment[];
  headSha: string | null;
  readOnly: boolean;
  canExpand: boolean;
  expanding: boolean;
  expandError: string | null;
  onExpandBetween: (prev: HunkData, next: HunkData, n: number) => void;
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
          {orphans.map((c) => (
            <CommentItem
              key={c.id}
              comment={c}
              headSha={headSha}
              readOnly={readOnly}
              onSaving={onSaving}
              onSaved={onSaved}
              onCommentsChanged={onCommentsChanged}
            />
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
              // v1: only the gaps BETWEEN hunks get an expander; leading and
              // trailing collapsed blocks (expand-to-top/bottom) are v1.1.
              const prev = i === 0 ? null : renderedHunks[i - 1];
              const collapsed = getCollapsedLinesCountBetween(prev, hunk);
              const rows: React.ReactElement[] = [];
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
  onExpandChunk,
  onExpandAll,
}: {
  count: number;
  busy: boolean;
  onExpandChunk: () => void;
  onExpandAll: () => void;
}) {
  return (
    <div className="expand-control">
      {busy ? (
        <span className="muted">Expanding…</span>
      ) : (
        <>
          <span className="expand-label">⋯ {count} hidden lines</span>
          {count > EXPAND_CHUNK && (
            <button className="expand-btn" onClick={onExpandChunk} disabled={busy}>
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
  comments,
  headSha,
  composerOpen,
  rangeLabel,
  readOnly,
  showOrigin,
  onCloseComposer,
  onAdd,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  comments: Comment[];
  headSha: string | null;
  composerOpen: boolean;
  rangeLabel?: string;
  readOnly: boolean;
  showOrigin?: boolean;
  onCloseComposer: () => void;
  onAdd: (text: string) => Promise<void>;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  return (
    <div className="line-widget">
      {comments.map((c) => (
        <CommentItem
          key={c.id}
          comment={c}
          headSha={headSha}
          readOnly={readOnly}
          showOrigin={showOrigin}
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

export function CommentItem({
  comment,
  headSha,
  readOnly,
  showOrigin,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  comment: Comment;
  headSha: string | null;
  readOnly: boolean;
  showOrigin?: boolean;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const [body, setBody] = useState(comment.body);
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
      <textarea
        value={body}
        disabled={readOnly}
        onChange={(e) => {
          setBody(e.target.value);
          save(e.target.value);
        }}
      />
      {!readOnly && (
        <button
          className="btn-icon"
          title="Delete comment"
          onClick={async () => {
            if (
              await confirmDialog({
                title: "Delete comment",
                message: "Delete this comment?",
                confirmLabel: "Delete",
                danger: true,
              })
            ) {
              await api.deleteComment(comment.id);
              onCommentsChanged();
            }
          }}
        >
          🗑
        </button>
      )}
    </div>
  );
}

export function Composer({
  onSubmit,
  onCancel,
  rangeLabel,
}: {
  onSubmit: (text: string) => Promise<void>;
  onCancel: () => void;
  rangeLabel?: string;
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
        <button onClick={onCancel}>Cancel</button>
        <button
          className="btn-primary"
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
          Add comment
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
