import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Diff,
  Hunk,
  parseDiff,
  type ChangeData,
  type ViewType,
} from "react-diff-view";
import { api } from "../lib/api";
import { isImagePath } from "../lib/imageDiff";
import { ImageDiffLink } from "./ImageDiffLink";
import { headFileSourceQuery, reviewDiffQuery } from "../lib/reviewQueries";
import {
  buildFullFileFile,
  changeKeyOf,
  changedRightLines,
  fileDisplayPath,
  indexFile,
  languageForPath,
  tokenizeFile,
} from "../lib/diff";
import { useSyntaxReady } from "../lib/highlight";
import { DiffSearchBar, LineWidget, ThreadItem, flashRow } from "./ReviewView";
import { filterDiffMatches, wrapIndex } from "../lib/diffSearch";
import { useDefinitionLinks, type DefinitionLookup } from "../lib/useDefinitionLinks";
import { groupThreads, isDraftReply, type CommentThread } from "../lib/threads";
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
 * on their lines, and diff-changed lines get a discreet gutter bar. Cmd/Ctrl-hover/click
 * go-to-definition and Cmd+F search both work on this file (not the diff).
 */
export function FileViewPane({
  reviewId,
  detail,
  filePath,
  readOnly,
  focus,
  bodyRef,
  active,
  pin,
  diffPaths,
  definitionOpen,
  onLookup,
  onClose,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  reviewId: number;
  detail: ReviewDetail;
  filePath: string;
  readOnly: boolean;
  /** Line to scroll to and flash once the file is shown (a new object re-triggers it). */
  focus?: { line: number } | null;
  /** The scrollable body; owned by the parent so Back/Forward can save and restore its scroll. */
  bodyRef: RefObject<HTMLDivElement | null>;
  /** False while the review's tab is hidden: its subtree stays mounted. */
  active: boolean;
  /** `base:head` commit ids, the definition cache key's pin. */
  pin: string;
  diffPaths: string[];
  /** The definition popover is open (it owns the next Escape). */
  definitionOpen: boolean;
  onLookup: (lookup: DefinitionLookup) => void;
  onClose: () => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchIdx, setSearchIdx] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        setSearchOpen(true);
        searchInputRef.current?.select();
        return;
      }
      // the search input and the definition popover handle their own Escape
      if (e.key !== "Escape" || e.defaultPrevented || definitionOpen) return;
      if (searchOpen) setSearchOpen(false);
      else onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, definitionOpen, searchOpen, onClose]);

  useDefinitionLinks({
    rootRef: bodyRef,
    from: "file",
    enabled: active,
    reviewId,
    pin,
    diffPaths,
    onLookup,
  });

  const sourceQuery = useQuery(headFileSourceQuery(reviewId, filePath, detail.review.head_sha));

  // The diff is already cached from the main view. We only need it to know
  // which head lines are added/modified.
  const diffQuery = useQuery(reviewDiffQuery(reviewId, detail));

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
  const syntaxReady = useSyntaxReady(languageForPath(filePath));
  const tokens = useMemo(
    () => (synthetic ? tokenizeFile(synthetic) : undefined),
    [synthetic, syntaxReady],
  );
  const { metaByKey, keyByAnchor } = useMemo(
    () => (synthetic ? indexFile(synthetic) : { metaByKey: new Map(), keyByAnchor: new Map() }),
    [synthetic],
  );

  useEffect(() => {
    if (!focus || !synthetic) return;
    const key = keyByAnchor.get(`RIGHT:${focus.line}`);
    if (!key) return;
    // after the Diff has rendered the rows
    const raf = requestAnimationFrame(() => {
      const cell = bodyRef.current?.querySelector<HTMLElement>(`[data-change-key="${CSS.escape(key)}"]`);
      const row = cell?.closest<HTMLElement>("tr") ?? cell;
      if (row) flashRow(row);
    });
    return () => cancelAnimationFrame(raf);
  }, [focus, synthetic, keyByAnchor]);

  const matches = useMemo(
    () => (searchOpen ? filterDiffMatches([metaByKey], searchQuery) : []),
    [searchOpen, metaByKey, searchQuery],
  );
  const matchKeys = useMemo(() => new Set(matches.map((m) => m.changeKey)), [matches]);
  const curMatch = matches.length ? matches[wrapIndex(searchIdx, matches.length)] : null;
  useEffect(() => {
    if (!curMatch) return;
    const raf = requestAnimationFrame(() => {
      bodyRef.current?.querySelector(".search-current")?.scrollIntoView({ block: "center" });
    });
    return () => cancelAnimationFrame(raf);
  }, [curMatch, bodyRef]);

  // This file's RIGHT-side line comments (both diff- and file-view-origin),
  // grouped by the change key they anchor to. LEFT/deletion comments don't map
  // to head lines, so they stay in the diff view only.
  const { commentsByKey, orphans } = useMemo(() => {
    const byKey = new Map<string, CommentThread[]>();
    const orphan: CommentThread[] = [];
    const rightComments = detail.comments.filter(
      (c) =>
        c.file_path === filePath &&
        c.subject_type !== "file" &&
        c.side === "RIGHT" &&
        !isDraftReply(c),
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
      anchoredHeadSha: detail.review.head_sha,
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
        headSha={detail.review.head_sha}
        baseSha={detail.review.base_sha}
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
    const extra: string[] = [];
    if (changes.some((c) => c != null && c.type === "normal" && changedLines.has(c.newLineNumber)))
      extra.push("changed-line");
    if (matchKeys.size && changes.some((c) => c != null && matchKeys.has(changeKeyOf(c))))
      extra.push("search-match");
    if (curMatch && changes.some((c) => c != null && changeKeyOf(c) === curMatch.changeKey))
      extra.push("search-current");
    return [base, ...extra].filter(Boolean).join(" ") || undefined;
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
      {searchOpen && (
        <DiffSearchBar
          inputRef={searchInputRef}
          placeholder="Find in file…"
          query={searchQuery}
          count={matches.length}
          index={matches.length ? wrapIndex(searchIdx, matches.length) : 0}
          onChange={(v) => {
            setSearchQuery(v);
            setSearchIdx(0);
          }}
          onNext={() => setSearchIdx((i) => i + 1)}
          onPrev={() => setSearchIdx((i) => i - 1)}
          onClose={() => setSearchOpen(false)}
        />
      )}
      <div className="file-view-pane-body" ref={bodyRef} data-file-path={filePath}>
        {sourceQuery.isLoading && <p className="muted">Loading file…</p>}
        {sourceQuery.isError && (
          <p className="error">Could not load file: {String(sourceQuery.error)}</p>
        )}
        {diffFile?.isBinary && (
          <p className="muted binary-note">
            Binary file not shown. {isImagePath(filePath) && <ImageDiffLink reviewId={reviewId} path={filePath} />}
          </p>
        )}
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
                    headSha={detail.review.head_sha}
                    baseSha={detail.review.base_sha}
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
