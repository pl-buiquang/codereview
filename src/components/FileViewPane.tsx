import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Diff,
  Hunk,
  parseDiff,
  type ChangeData,
  type ViewType,
} from "react-diff-view";
import { api } from "../lib/api";
import {
  buildFullFileFile,
  changeKeyOf,
  changedRightLines,
  fileDisplayPath,
  indexFile,
  tokenizeFile,
} from "../lib/diff";
import { LineWidget, ThreadItem } from "./ReviewView";
import { groupThreads, type CommentThread } from "../lib/threads";
import type { ReviewDetail } from "../lib/types";

interface Selection {
  anchorLine: number;
  focusLine: number;
  focusKey: string;
}

/**
 * Right-hand slide-out pane rendering the full head-side file. Lines outside the
 * diff hunks are commentable here; those comments are stored as `origin =
 * 'file_view'` and fold into the review body on publish/export rather than
 * posting inline. Existing RIGHT-side diff comments are surfaced (and editable)
 * on their lines, and diff-changed lines get a discreet gutter bar.
 */
export function FileViewPane({
  reviewId,
  detail,
  filePath,
  readOnly,
  onClose,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  reviewId: number;
  detail: ReviewDetail;
  filePath: string;
  readOnly: boolean;
  onClose: () => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const sourceQuery = useQuery({
    queryKey: ["file-source", reviewId, filePath, "RIGHT"],
    queryFn: () => api.fileSource(reviewId, filePath, "RIGHT"),
  });

  // The diff is already cached from the main view; re-running the query just
  // returns it. We only need it to know which head lines are added/modified.
  const diffQuery = useQuery({
    queryKey: ["review-diff", reviewId, detail.target.id, detail.target.head_sha],
    queryFn: () => api.reviewDiff(reviewId),
  });

  const diffFile = useMemo(() => {
    if (diffQuery.data == null) return null;
    return parseDiff(diffQuery.data).find((f) => fileDisplayPath(f) === filePath) ?? null;
  }, [diffQuery.data, filePath]);
  const changedLines = useMemo(
    () => (diffFile ? changedRightLines(diffFile) : new Set<number>()),
    [diffFile],
  );

  const synthetic = useMemo(
    () => (sourceQuery.data != null ? buildFullFileFile(filePath, sourceQuery.data) : null),
    [sourceQuery.data, filePath],
  );
  const tokens = useMemo(() => (synthetic ? tokenizeFile(synthetic) : undefined), [synthetic]);
  const { metaByKey, keyByAnchor } = useMemo(
    () => (synthetic ? indexFile(synthetic) : { metaByKey: new Map(), keyByAnchor: new Map() }),
    [synthetic],
  );

  // This file's RIGHT-side line comments (both diff- and file-view-origin),
  // grouped by the change key they anchor to. LEFT/deletion comments don't map
  // to head lines, so they stay in the diff view only.
  const { commentsByKey, orphans } = useMemo(() => {
    const byKey = new Map<string, CommentThread[]>();
    const orphan: CommentThread[] = [];
    const rightComments = detail.comments.filter(
      (c) => c.file_path === filePath && c.subject_type !== "file" && c.side === "RIGHT",
    );
    for (const thread of groupThreads(rightComments)) {
      const key = keyByAnchor.get(`RIGHT:${thread.root.line}`);
      if (!key) {
        orphan.push(thread);
        continue;
      }
      const arr = byKey.get(key) ?? [];
      arr.push(thread);
      byKey.set(key, arr);
    }
    return { commentsByKey: byKey, orphans: orphan };
  }, [detail.comments, filePath, keyByAnchor]);

  const [selection, setSelection] = useState<Selection | null>(null);
  const range = selection
    ? {
        lo: Math.min(selection.anchorLine, selection.focusLine),
        hi: Math.max(selection.anchorLine, selection.focusLine),
      }
    : null;

  const onLineClick = (args: { change: ChangeData | null }, event: React.MouseEvent) => {
    if (readOnly || !args.change) return;
    const key = changeKeyOf(args.change);
    const meta = metaByKey.get(key);
    if (!meta) return;
    setSelection((prev) => {
      if (event?.shiftKey && prev) {
        return { ...prev, focusLine: meta.line, focusKey: key };
      }
      return { anchorLine: meta.line, focusLine: meta.line, focusKey: key };
    });
  };

  const selectedChanges = useMemo(() => {
    if (!selection || !range) return [];
    const keys: string[] = [];
    for (const [k, m] of metaByKey) {
      if (m.line >= range.lo && m.line <= range.hi) keys.push(k);
    }
    return keys;
  }, [selection, range, metaByKey]);

  const submitComment = async (text: string) => {
    if (!selection || !range) return;
    onSaving();
    await api.addFileViewComment({
      reviewId,
      filePath,
      line: range.hi,
      startLine: range.lo === range.hi ? null : range.lo,
      body: text,
      anchoredHeadSha: detail.target.head_sha,
    });
    setSelection(null);
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
        ? `Lines ${range.lo}–${range.hi}`
        : undefined;
    widgets[key] = (
      <LineWidget
        threads={commentsByKey.get(key) ?? []}
        headSha={detail.target.head_sha}
        composerOpen={!!composerOpen}
        rangeLabel={rangeLabel}
        readOnly={readOnly}
        showOrigin
        canReply={!readOnly}
        onCloseComposer={() => setSelection(null)}
        onAdd={submitComment}
        onSaving={onSaving}
        onSaved={onSaved}
        onCommentsChanged={onCommentsChanged}
      />
    );
  }

  const generateLineClassName = ({
    changes,
    defaultGenerate,
  }: {
    changes: ChangeData[];
    defaultGenerate: () => string;
  }): string | undefined => {
    const base = defaultGenerate();
    const changed = changes.some(
      (c) => c.type === "normal" && changedLines.has(c.newLineNumber),
    );
    if (!changed) return base;
    return base ? `${base} changed-line` : "changed-line";
  };

  return (
    <aside className="file-view-pane">
      <div className="file-view-pane-header">
        <span className="file-path">{filePath}</span>
        {!readOnly && (
          <span className="muted file-view-hint">
            Click a line to comment · folds into the review summary
          </span>
        )}
        <button className="btn-icon" title="Close (Esc)" onClick={onClose}>
          ✕
        </button>
      </div>
      <div className="file-view-pane-body">
        {sourceQuery.isLoading && <p className="muted">Loading file…</p>}
        {sourceQuery.isError && (
          <p className="error">Could not load file: {String(sourceQuery.error)}</p>
        )}
        {diffFile?.isBinary && <p className="muted binary-note">Binary file not shown.</p>}
        {synthetic && !diffFile?.isBinary && (
          <>
            {orphans.length > 0 && (
              <div className="orphan-comments">
                <p className="muted">
                  Comments not matching the current file (head may have changed):
                </p>
                {orphans.map((t) => (
                  <ThreadItem
                    key={t.root.id}
                    thread={t}
                    headSha={detail.target.head_sha}
                    readOnly={readOnly}
                    showOrigin
                    canReply={false}
                    onSaving={onSaving}
                    onSaved={onSaved}
                    onCommentsChanged={onCommentsChanged}
                  />
                ))}
              </div>
            )}
            <Diff
              viewType={"unified" as ViewType}
              diffType="modify"
              hunks={synthetic.hunks}
              tokens={tokens}
              widgets={widgets}
              selectedChanges={selectedChanges}
              generateLineClassName={generateLineClassName}
              gutterEvents={{ onClick: onLineClick }}
              codeEvents={{ onClick: onLineClick }}
            >
              {(hunks) => hunks.map((h) => <Hunk key={h.content} hunk={h} />)}
            </Diff>
          </>
        )}
      </div>
    </aside>
  );
}
