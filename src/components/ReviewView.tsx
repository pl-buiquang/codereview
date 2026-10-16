import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MutableRefObject,
} from "react";
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
import { reviewDiffQuery } from "../lib/reviewQueries";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import {
  anchorByLine,
  changeKeyOf,
  countChanges,
  fileDisplayPath,
  hunkContextSnippet,
  indexFile,
  isLargeDiff,
  leadingExpandRange,
  rightLinesText,
  sourceLineCount,
  suggestionFence,
  tokenizeFile,
  trailingExpandRange,
  trailingGap,
} from "../lib/diff";
import { filterDiffMatches, wrapIndex, type DiffMatch } from "../lib/diffSearch";
import { categorize, compileCategoryRules, type FileCategory } from "../lib/fileCategory";
import { FileJumpList } from "./FileJumpList";
import {
  FocusDigest,
  FocusGenerate,
  FocusHiddenBar,
  FocusHunkMeta,
  FocusLevelSelect,
  useFocusActions,
  useGenerateFocus,
  type FocusActions,
  type GenerateFocus,
} from "./Focus";
import {
  buildFocusModel,
  originalsIn,
  type Anchor,
  type FileFocus,
  type HunkFocus,
} from "../lib/focus";
import { FileViewPane } from "./FileViewPane";
import { GithubThread } from "./GithubThread";
import { ShortcutHelp } from "./ShortcutHelp";
import { Markdown } from "./Markdown";
import { ChatPanel } from "./ChatPanel";
import { PublishButton } from "./PublishButton";
import { PrMetaPanel, usePrMeta } from "./PrMetaPanel";
import { ReviewSummaryRow } from "./ReviewSummaryRow";
import { ReviewSwitcher } from "./ReviewSwitcher";
import { AutoMergeBadge, CiBadge, ReviewDecisionBadge, StatusPill } from "./InboxBadges";
import { Icon } from "./icons";
import { AgentSessionItems, KebabMenu } from "./KebabMenu";
import { githubPrUrl } from "../lib/githubUrl";
import { useDebouncedCallback } from "../lib/useDebouncedCallback";
import { useSettingsStore } from "../lib/settings";
import { useUIStore } from "../store";
import {
  isEditableTarget,
  moveCursorKey,
  pickThread,
  type FileKbHandle,
  type JumpListHandle,
} from "../lib/keyboard";
import { groupThreads, isDraftReply, type CommentThread } from "../lib/threads";
import { anchorPin, isCommentOutdated } from "../lib/staleness";
import { summaryLine } from "../lib/text";
import type {
  Comment,
  FocusView,
  PrMeta,
  PrThread,
  PrThreadCtx,
  ReviewDetail,
  ReviewEvent,
  Side,
} from "../lib/types";

type SaveState = "idle" | "saving" | "saved";

const EXPAND_CHUNK = 20;

export function ReviewView({ reviewId }: { reviewId: number }) {
  const queryClient = useQueryClient();
  const defaultViewType = useSettingsStore((s) => s.defaultViewType);
  const [viewType, setViewType] = useState<ViewType>(defaultViewType);
  const [saveState, setSaveState] = useState<SaveState>("saved");
  const [filePanePath, setFilePanePath] = useState<string | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(true);
  const [chatCollapsed, setChatCollapsed] = useState(true);
  const [leftWidth, setLeftWidth] = useState(248);
  const [rightWidth, setRightWidth] = useState(360);
  const [pendingChatMessage, setPendingChatMessage] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<"all" | FileCategory>("all");
  const diffAreaRef = useRef<HTMLDivElement>(null);
  const summaryBodyRef = useRef<string | null>(null);

  // Keyboard navigation. The single window listener lives here (see the effect
  // below); FileJumpList and each FileReview publish imperative handles into
  // these refs so the dispatcher can drive them without re-rendering this heavy
  // diff subtree on every scrollspy tick.
  const isActiveTab = useUIStore(
    (s) => s.activeTabId === `review-${reviewId}`,
  );
  const [showHelp, setShowHelp] = useState(false);
  const jumpListRef = useRef<JumpListHandle | null>(null);
  const fileKbRef = useRef(new Map<number, FileKbHandle>());
  const cursorFileRef = useRef<number | null>(null);

  // Find-in-diff (Cmd/Ctrl+F): scans every file's diff lines, highlights matches,
  // and jumps between them (Enter / Shift+Enter), expanding a collapsed ("viewed")
  // file when a match lands inside it.
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchIdx, setSearchIdx] = useState(0);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Hunk ids shown temporarily by "show" / search; never persisted.
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(new Set());
  const reveal = useCallback(
    (ids: string[]) => setRevealed((prev) => new Set([...prev, ...ids])),
    [],
  );
  const unreveal = useCallback(
    (ids: string[]) =>
      setRevealed((prev) => {
        const next = new Set(prev);
        for (const id of ids) next.delete(id);
        return next;
      }),
    [],
  );
  const [summaryAppend, setSummaryAppend] = useState<{ text: string; nonce: number } | null>(
    null,
  );
  const focusActions = useFocusActions(reviewId);
  const generateFocus = useGenerateFocus(reviewId);
  const chatModel = useSettingsStore((s) => s.chatModel);

  const detailQuery = useQuery({
    queryKey: ["review", reviewId],
    queryFn: () => api.getReview(reviewId),
  });

  const diffQuery = useQuery(reviewDiffQuery(reviewId, detailQuery.data?.target));

  useEffect(() => {
    if (detailQuery.data) {
      setSidebarCollapsed(detailQuery.data.review.sidebar_collapsed);
      setChatCollapsed(detailQuery.data.review.chat_collapsed);
      setLeftWidth(detailQuery.data.review.left_panel_width);
      setRightWidth(detailQuery.data.review.right_panel_width);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    detailQuery.data?.review.sidebar_collapsed,
    detailQuery.data?.review.chat_collapsed,
    detailQuery.data?.review.left_panel_width,
    detailQuery.data?.review.right_panel_width,
  ]);

  const toggleSidebar = useCallback(() => {
    setSidebarCollapsed((prev) => {
      const next = !prev;
      api.setSidebarCollapsed(reviewId, next);
      return next;
    });
  }, [reviewId]);

  const toggleChat = useCallback(() => {
    setChatCollapsed((prev) => {
      const next = !prev;
      api.setChatCollapsed(reviewId, next);
      return next;
    });
  }, [reviewId]);

  const startResize = useCallback(
    (side: "left" | "right") => (e: React.MouseEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startLeft = leftWidth;
      const startRight = rightWidth;
      let finalLeft = startLeft;
      let finalRight = startRight;

      const onMove = (ev: MouseEvent) => {
        const delta = ev.clientX - startX;
        if (side === "left") {
          finalLeft = Math.max(160, Math.min(500, startLeft + delta));
          setLeftWidth(finalLeft);
        } else {
          finalRight = Math.max(200, Math.min(600, startRight - delta));
          setRightWidth(finalRight);
        }
      };

      const onUp = () => {
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        api.setPanelWidths(reviewId, finalLeft, finalRight);
      };

      document.addEventListener("mousemove", onMove);
      document.addEventListener("mouseup", onUp);
    },
    [reviewId, leftWidth, rightWidth],
  );

  const sendToChat = useCallback((formattedMsg: string) => {
    setPendingChatMessage(formattedMsg);
    setChatCollapsed((prev) => {
      if (prev) {
        api.setChatCollapsed(reviewId, false);
        return false;
      }
      return prev;
    });
  }, [reviewId]);

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

  // n/p: stateless, DOM-query based. Collect rendered thread blocks inside the
  // scroll container (skipping a .line-widget that holds only an open composer),
  // then scroll to the first below / last above the current offset.
  const npThreadNav = useCallback((dir: 1 | -1) => {
    const root = diffAreaRef.current;
    if (!root) return;
    const els = [
      ...root.querySelectorAll<HTMLElement>(
        ".line-widget, .github-thread, .file-comments, .orphan-comments",
      ),
    ].filter(
      (el) =>
        el.classList.contains("github-thread") ||
        el.querySelector(".comment-item") != null,
    );
    if (els.length === 0) return;
    const rootTop = root.getBoundingClientRect().top;
    const tops = els.map(
      (el) => el.getBoundingClientRect().top - rootTop + root.scrollTop,
    );
    const idx = pickThread(tops, root.scrollTop, dir);
    if (idx == null) return;
    const el = els[idx];
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    el.classList.add("kb-flash");
    setTimeout(() => el.classList.remove("kb-flash"), 900);
  }, []);

  // Parsed diff lifted here so search can scan every file (ReviewDiff renders it).
  const files = useMemo(
    () => (diffQuery.data != null ? parseDiff(diffQuery.data) : []),
    [diffQuery.data],
  );

  const fileTestPatterns = useSettingsStore((s) => s.fileTestPatterns);
  const fileConfigPatterns = useSettingsStore((s) => s.fileConfigPatterns);
  const categoryRules = useMemo(
    () => compileCategoryRules(fileTestPatterns, fileConfigPatterns),
    [fileTestPatterns, fileConfigPatterns],
  );

  const focusQuery = useQuery({
    queryKey: ["focus", reviewId, detailQuery.data?.target.head_sha],
    enabled: detailQuery.data != null,
    queryFn: () => api.getFocus(reviewId),
  });
  const focusView = focusQuery.data ?? null;

  const focusAnchors = useMemo<Anchor[]>(() => {
    const out: Anchor[] = [];
    for (const c of detailQuery.data?.comments ?? []) {
      if (c.origin === "file_view" || isDraftReply(c) || c.parent_id != null || c.tag === "ai")
        continue;
      if (c.subject_type === "file") {
        out.push({ path: c.file_path, side: "FILE", line: -1 });
        continue;
      }
      out.push({ path: c.file_path, side: c.side, line: c.line });
      if (c.start_line != null) out.push({ path: c.file_path, side: c.side, line: c.start_line });
    }
    for (const t of threadsQuery.data ?? []) {
      if (!t.path) continue;
      out.push({ path: t.path, side: t.line == null ? "FILE" : (t.diffSide ?? "RIGHT"), line: t.line ?? -1 });
    }
    return out;
  }, [detailQuery.data?.comments, threadsQuery.data]);

  const focusModel = useMemo(
    () =>
      buildFocusModel(
        files.map((f) => ({ path: fileDisplayPath(f), hunks: f.hunks })),
        focusView,
        focusAnchors,
        revealed,
      ),
    [files, focusView, focusAnchors, revealed],
  );

  // Files focus hides entirely are removed from both panes, same as the type
  // filter — not just greyed out.
  const focusHiddenPaths = useMemo(() => {
    const set = new Set<string>();
    for (const f of focusModel ?? []) if (f.fullyHidden) set.add(f.path);
    return set;
  }, [focusModel]);

  const visibleFileIdx = useMemo(() => {
    if (typeFilter === "all" && focusHiddenPaths.size === 0) return null;
    const set = new Set<number>();
    files.forEach((f, i) => {
      const path = fileDisplayPath(f);
      if (focusHiddenPaths.has(path)) return;
      if (typeFilter !== "all" && categorize(path, categoryRules) !== typeFilter) return;
      set.add(i);
    });
    return set;
  }, [files, categoryRules, typeFilter, focusHiddenPaths]);

  const jumpToFile = useCallback((fileIndex: number) => {
    // Deferred a frame: revealing a focus-hidden file re-renders it into the
    // DOM, which hasn't happened yet when onReveal/onJump fire together.
    requestAnimationFrame(() => {
      diffAreaRef.current
        ?.querySelector(`#file-${fileIndex}`)
        ?.scrollIntoView({ block: "start" });
    });
  }, []);

  // Per-file change index, built once the find bar opens (covers every file,
  // including collapsed ones); search then re-filters it per keystroke.
  const perFileMeta = useMemo(
    () => (searchOpen ? files.map((f) => indexFile(f).metaByKey) : []),
    [searchOpen, files],
  );
  const largeDiffMaxLines = useSettingsStore((s) => s.largeDiffMaxLines);
  const largeFileIdx = useMemo(
    () => new Set(files.flatMap((f, i) => (isLargeDiff(f, largeDiffMaxLines) ? [i] : []))),
    [files, largeDiffMaxLines],
  );
  const matches = useMemo<DiffMatch[]>(() => {
    if (!searchOpen) return [];
    return filterDiffMatches(perFileMeta, searchQuery).filter(
      (m) =>
        (visibleFileIdx == null || visibleFileIdx.has(m.fileIndex)) && !largeFileIdx.has(m.fileIndex),
    );
  }, [searchOpen, perFileMeta, searchQuery, visibleFileIdx, largeFileIdx]);
  const curMatch = matches.length ? matches[wrapIndex(searchIdx, matches.length)] : null;
  const matchKeysByFile = useMemo(() => {
    const m = new Map<number, Set<string>>();
    for (const mt of matches) {
      let set = m.get(mt.fileIndex);
      if (!set) {
        set = new Set();
        m.set(mt.fileIndex, set);
      }
      set.add(mt.changeKey);
    }
    return m;
  }, [matches]);

  // Reveal a collapsed file when search navigates into it (optimistic un-view).
  const expandForSearch = useMutation({
    mutationFn: (path: string) => api.setFileViewed(reviewId, path, false),
    onMutate: async (path: string) => {
      const key = ["review", reviewId];
      await queryClient.cancelQueries({ queryKey: key });
      const prev = queryClient.getQueryData<ReviewDetail>(key);
      queryClient.setQueryData<ReviewDetail>(key, (old) =>
        old ? { ...old, viewed_files: old.viewed_files.filter((p) => p !== path) } : old,
      );
      return { prev };
    },
    onError: (_e, _v, ctx) => {
      if (ctx?.prev) queryClient.setQueryData(["review", reviewId], ctx.prev);
    },
  });

  // Scroll the current match into view; if its file is collapsed, expand it first
  // (the un-view changes viewedSig, re-running this effect to do the scroll).
  const viewedSig = (detailQuery.data?.viewed_files ?? []).join("\n");
  useEffect(() => {
    if (!searchOpen || !curMatch) return;
    const file = files[curMatch.fileIndex];
    if (!file) return;
    const path = fileDisplayPath(file);
    if ((detailQuery.data?.viewed_files ?? []).includes(path)) {
      expandForSearch.mutate(path);
      return;
    }
    const raf = requestAnimationFrame(() => {
      diffAreaRef.current
        ?.querySelector(".search-current")
        ?.scrollIntoView({ block: "center" });
    });
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchOpen, searchIdx, matches, viewedSig, files, revealed]);

  // A match inside a focus-hidden hunk reveals it, like un-viewing a file.
  useEffect(() => {
    if (!searchOpen || !curMatch || !focusModel) return;
    const meta = perFileMeta[curMatch.fileIndex]?.get(curMatch.changeKey);
    const file = files[curMatch.fileIndex];
    if (!meta || !file) return;
    const i = file.hunks.findIndex((h) => h.content === meta.hunk);
    const hf = focusModel[curMatch.fileIndex]?.hunks[i];
    if (hf?.hidden && hf.entry) reveal([hf.entry.hunk_id]);
  }, [searchOpen, curMatch, focusModel, perFileMeta, files, reveal]);

  useEffect(() => {
    if (!isActiveTab) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        setSearchOpen(true);
        searchInputRef.current?.select();
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (e.key === "Escape") {
        if (searchOpen) {
          setSearchOpen(false);
          return;
        }
        if (showHelp) {
          setShowHelp(false);
          return;
        }
        if (filePanePath) return; // FileViewPane owns its own Escape
        const i = cursorFileRef.current;
        if (i != null) fileKbRef.current.get(i)?.clearCursor();
        return;
      }
      if (isEditableTarget(e.target)) return;
      if (filePanePath || showHelp) {
        // While help is open, only ? toggles it back off.
        if (e.key === "?" && !filePanePath) setShowHelp(false);
        return;
      }
      const jl = jumpListRef.current;
      switch (e.key) {
        case "]":
        case "[": {
          if (!jl || jl.fileCount === 0) return;
          const delta = e.key === "]" ? 1 : -1;
          const visible =
            visibleFileIdx == null
              ? Array.from({ length: jl.fileCount }, (_, i) => i)
              : [...visibleFileIdx].sort((a, b) => a - b);
          if (visible.length === 0) return;
          const pos = visible.indexOf(jl.activeIndex);
          const nextPos = Math.min(
            visible.length - 1,
            Math.max(0, (pos === -1 ? 0 : pos) + delta),
          );
          const next = visible[nextPos];
          if (next !== jl.activeIndex) jl.jumpTo(next);
          break;
        }
        case "n":
          npThreadNav(1);
          break;
        case "p":
          npThreadNav(-1);
          break;
        case "j":
        case "k": {
          const i = jumpListRef.current?.activeIndex ?? 0;
          const prev = cursorFileRef.current;
          if (prev != null && prev !== i) {
            fileKbRef.current.get(prev)?.clearCursor();
          }
          fileKbRef.current.get(i)?.moveCursor(e.key === "j" ? 1 : -1);
          cursorFileRef.current = i;
          break;
        }
        case "c": {
          const i = cursorFileRef.current;
          if (i != null) fileKbRef.current.get(i)?.openComposer();
          break;
        }
        case "b":
          toggleSidebar();
          break;
        case "'":
          toggleChat();
          break;
        case "?":
          setShowHelp(true);
          break;
        default:
          return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    isActiveTab,
    showHelp,
    filePanePath,
    npThreadNav,
    searchOpen,
    toggleSidebar,
    toggleChat,
    visibleFileIdx,
  ]);

  if (detailQuery.isLoading) return <section className="main-panel">Loading review…</section>;
  if (detailQuery.isError || !detailQuery.data)
    return (
      <section className="main-panel">
        <p className="error">Could not load review: {String(detailQuery.error)}</p>
      </section>
    );

  const detail = detailQuery.data;
  const readOnly = detail.review.status !== "draft";

  return (
    <section className="main-panel review-panel">
      <ReviewHeader
        detail={detail}
        saveState={saveState}
        readOnly={readOnly}
        viewType={viewType}
        setViewType={setViewType}
        focusView={focusView}
        focusActions={focusActions}
        generateFocus={generateFocus}
        bodyRef={summaryBodyRef}
      />

      <div
        className="review-body"
        style={{
          "--left-panel-width": `${leftWidth}px`,
          "--right-panel-width": `${rightWidth}px`,
        } as CSSProperties}
      >
        <FileJumpList
          reviewId={reviewId}
          scrollRootRef={diffAreaRef}
          controlRef={jumpListRef}
          paneCollapsed={sidebarCollapsed}
          onToggle={toggleSidebar}
          threads={threadsQuery.data ?? []}
          hiddenPaths={focusHiddenPaths}
          typeFilter={typeFilter}
          onTypeFilterChange={setTypeFilter}
          prContext={target?.kind === "github_pr" && owner && name && prNumber != null ? { owner, name, number: prNumber } : null}
        />
        {!sidebarCollapsed && <div className="resize-handle" onMouseDown={startResize("left")} />}
        <div className="diff-area" ref={diffAreaRef}>
          {searchOpen && (
            <DiffSearchBar
              inputRef={searchInputRef}
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
          <ReviewSummaryRow
              key={detail.review.id}
              reviewId={detail.review.id}
              initialBody={detail.review.body}
              readOnly={readOnly}
              bodyRef={summaryBodyRef}
              summaryAppend={summaryAppend}
              onSaving={() => setSaveState("saving")}
              onSaved={() => {
                setSaveState("saved");
                queryClient.invalidateQueries({ queryKey: ["reviews"] });
              }}
            />
          {!readOnly && (
            <p className="hint muted">
              Click a line to comment · shift-click another line on the same side to select a range.
              · press ? for shortcuts
            </p>
          )}
          {focusView &&
            (focusView.focus.planned_at == null ||
              focusView.focus.level !== "off" ||
              focusView.focus.status !== "ready") && (
              <FocusDigest
                view={focusView}
                model={focusModel}
                headSha={detail.target.head_sha}
                readOnly={readOnly}
                actions={focusActions}
                onReveal={reveal}
                onJump={jumpToFile}
                onCopyToSummary={(text) => setSummaryAppend({ text, nonce: Date.now() })}
                onRegenerate={
                  focusView.focus.status === "generating"
                    ? undefined
                    : () => generateFocus.mutate({ model: chatModel || undefined })
                }
              />
            )}
          {diffQuery.isLoading && <p className="muted">Loading diff…</p>}
          {diffQuery.isError && <p className="error">Diff failed: {String(diffQuery.error)}</p>}
          {diffQuery.data != null && visibleFileIdx != null && visibleFileIdx.size === 0 && (
            <p className="muted">No files match the filter.</p>
          )}
          {diffQuery.data != null && (
            <ReviewDiff
              files={files}
              visibleFileIdx={visibleFileIdx}
              viewType={viewType}
              detail={detail}
              threads={threadsQuery.data ?? []}
              readOnly={readOnly}
              kbHandles={fileKbRef}
              searchKeysByFile={matchKeysByFile}
              searchCurrent={curMatch}
              focusModel={focusModel}
              focusActions={focusActions}
              onReveal={reveal}
              onUnreveal={unreveal}
              onOpenFilePane={setFilePanePath}
              onSaving={() => setSaveState("saving")}
              onSaved={() => {
                setSaveState("saved");
                queryClient.invalidateQueries({ queryKey: ["review", reviewId] });
              }}
              onCommentsChanged={() =>
                queryClient.invalidateQueries({ queryKey: ["review", reviewId] })
              }
              onSendToChat={sendToChat}
            />
          )}
        </div>
        {!chatCollapsed && <div className="resize-handle" onMouseDown={startResize("right")} />}
        <ChatPanel
          reviewId={reviewId}
          paneCollapsed={chatCollapsed}
          onToggle={toggleChat}
          pendingMessage={pendingChatMessage}
          onPendingConsumed={() => setPendingChatMessage(null)}
        />
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

      {showHelp && <ShortcutHelp onClose={() => setShowHelp(false)} />}
    </section>
  );
}

function DiffSearchBar({
  inputRef,
  query,
  count,
  index,
  onChange,
  onNext,
  onPrev,
  onClose,
}: {
  inputRef: React.RefObject<HTMLInputElement | null>;
  query: string;
  count: number;
  index: number;
  onChange: (v: string) => void;
  onNext: () => void;
  onPrev: () => void;
  onClose: () => void;
}) {
  return (
    <div className="diff-search-bar">
      <input
        ref={inputRef}
        autoFocus
        className="input diff-search-input"
        placeholder="Find in diff…"
        value={query}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            if (e.shiftKey) onPrev();
            else onNext();
          } else if (e.key === "Escape") {
            e.preventDefault();
            onClose();
          }
        }}
      />
      <span className="diff-search-count">
        {count ? `${index + 1} / ${count}` : query ? "0 / 0" : ""}
      </span>
      <button className="btn-icon" title="Previous match (Shift+Enter)" disabled={!count} onClick={onPrev}>
        ↑
      </button>
      <button className="btn-icon" title="Next match (Enter)" disabled={!count} onClick={onNext}>
        ↓
      </button>
      <button className="btn-icon" title="Close (Esc)" onClick={onClose}>
        ✕
      </button>
    </div>
  );
}

function PrNumberLink({ prNumber, prUrl }: { prNumber: number; prUrl: string }) {
  const [menuOpen, setMenuOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  return (
    <span ref={ref} className="pr-number-link-wrap">
      {" "}
      <button
        className="review-repo-link review-pr-link"
        title="Open PR in browser"
        onClick={() => api.openUrl(prUrl)}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenuOpen(true);
        }}
      >
        #{prNumber}
      </button>
      {menuOpen && (
        <div className="btn-split-menu card" role="menu">
          <button
            role="menuitem"
            onClick={() => {
              setMenuOpen(false);
              navigator.clipboard.writeText(prUrl).then(
                () => toast.success("PR URL copied to clipboard"),
                (e) => toast.error(`Could not copy URL: ${String(e)}`),
              );
            }}
          >
            Copy URL
          </button>
        </div>
      )}
    </span>
  );
}

function ReviewHeader({
  detail,
  saveState,
  readOnly,
  viewType,
  setViewType,
  focusView,
  focusActions,
  generateFocus,
  bodyRef,
}: {
  detail: ReviewDetail;
  saveState: SaveState;
  readOnly: boolean;
  viewType: ViewType;
  setViewType: (v: ViewType) => void;
  focusView: FocusView | null;
  focusActions: FocusActions;
  generateFocus: GenerateFocus;
  bodyRef: MutableRefObject<string | null>;
}) {
  const queryClient = useQueryClient();
  const closeReview = useUIStore((s) => s.closeReview);
  const openRepo = useUIStore((s) => s.openRepo);
  const { review, target } = detail;
  const isActiveTab = useUIStore((s) => s.activeTabId === `review-${review.id}`);
  const [showExport, setShowExport] = useState(false);
  const [collapsed, setCollapsed] = useState(true);
  const [openingWorktree, setOpeningWorktree] = useState(false);
  const openChatWorktreeInVscode = async () => {
    if (openingWorktree) return;
    setOpeningWorktree(true);
    try {
      const worktreePath = await api.ensureChatWorktree(review.id);
      await api.openInVscode(worktreePath);
      queryClient.invalidateQueries({ queryKey: ["chat-branch", review.id] });
      queryClient.invalidateQueries({ queryKey: ["worktrees", target.repo_id] });
    } catch (err) {
      toast.error(`Could not open VSCode: ${String(err)}`);
    } finally {
      setOpeningWorktree(false);
    }
  };
  const envQuery = useQuery({
    queryKey: ["environment"],
    queryFn: api.checkEnvironment,
    staleTime: Infinity,
  });
  const claudeAvailable = envQuery.data ? envQuery.data.claude != null : true;

  const deleteReview = useMutation({
    mutationFn: () => api.deleteReview(review.id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      queryClient.invalidateQueries({ queryKey: ["target-reviews", review.target_id] });
      closeReview();
    },
  });

  const publishReview = useMutation({
    // Persist the chosen verdict (and any unsaved summary) before publishing —
    // the backend reads `review.event` from the DB to build the GitHub payload.
    mutationFn: async (event: ReviewEvent) => {
      await api.updateReview(review.id, bodyRef.current ?? review.body, event);
      return api.publishReview(review.id);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["review", review.id] });
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      queryClient.invalidateQueries({ queryKey: ["target-reviews", review.target_id] });
      toast.success("Review published to GitHub.");
      closeReview();
    },
    onError: (e) => toast.error(`Publish failed:\n${String(e)}`),
  });

  const invalidateReview = () => {
    queryClient.invalidateQueries({ queryKey: ["review", review.id] });
    queryClient.invalidateQueries({ queryKey: ["reviews"] });
    queryClient.invalidateQueries({ queryKey: ["target-reviews", review.target_id] });
  };

  const publishPending = useMutation({
    // Persist the verdict + summary first, same as a normal publish — the pending
    // payload is built from the stored review, just with the event omitted.
    mutationFn: async (event: ReviewEvent) => {
      await api.updateReview(review.id, bodyRef.current ?? review.body, event);
      return api.publishReviewPending(review.id);
    },
    onSuccess: () => {
      invalidateReview();
      toast.success("Draft review staged on GitHub as pending.");
    },
    onError: (e) => toast.error(`Publish as draft failed:\n${String(e)}`),
  });

  const submitPending = useMutation({
    mutationFn: () => api.submitPendingReview(review.id),
    onSuccess: () => {
      invalidateReview();
      toast.success("Pending review submitted.");
      closeReview();
    },
    onError: (e) => toast.error(`Submit failed:\n${String(e)}`),
  });

  const discardPending = useMutation({
    mutationFn: () => api.discardPendingReview(review.id),
    onSuccess: () => {
      invalidateReview();
      toast.success("Pending review discarded — draft unlocked.");
    },
    onError: (e) => toast.error(`Discard failed:\n${String(e)}`),
  });

  const owner = detail.remote_owner ?? null;
  const name = detail.remote_name ?? null;
  const prNumber = target.github_pr_number ?? null;
  const prMetaQuery = usePrMeta(owner, name, prNumber, isActiveTab);

  const refreshReview = useMutation({
    mutationFn: () => api.refreshReview(review.id),
    onSuccess: (r) => {
      queryClient.invalidateQueries({ queryKey: ["review", review.id] });
      queryClient.invalidateQueries({ queryKey: ["pr-threads", owner, name, prNumber] });
      queryClient.invalidateQueries({ queryKey: ["pr-meta", owner, name, prNumber] });
      if (r.headMoved) toast.success("Head moved — re-anchor to update comments.");
    },
    onError: (e) => toast.error(`Refresh failed:\n${String(e)}`),
  });

  const togglePrDraft = useMutation({
    mutationFn: (nextDraft: boolean) => {
      if (owner == null || name == null || prNumber == null) {
        throw new Error("missing PR identity");
      }
      return api.setPrDraft(owner, name, prNumber, nextDraft);
    },
    onMutate: async (nextDraft) => {
      const key = ["pr-meta", owner, name, prNumber];
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<PrMeta>(key);
      if (previous) {
        queryClient.setQueryData<PrMeta>(key, { ...previous, isDraft: nextDraft });
      }
      return { previous };
    },
    onError: (e, _nextDraft, context) => {
      const key = ["pr-meta", owner, name, prNumber];
      if (context?.previous) queryClient.setQueryData<PrMeta>(key, context.previous);
      toast.error(`Could not update PR draft status:\n${String(e)}`);
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: ["pr-meta", owner, name, prNumber] });
    },
  });

  const updateBranch = useMutation({
    mutationFn: (rebase: boolean) => {
      if (owner == null || name == null || prNumber == null) {
        throw new Error("missing PR identity");
      }
      return api.updatePrBranch(owner, name, prNumber, rebase);
    },
    onSuccess: (_, rebase) => {
      toast.success(rebase ? "Branch rebased" : "Branch updated");
      queryClient.invalidateQueries({ queryKey: ["pr-meta", owner, name, prNumber] });
      queryClient.invalidateQueries({ queryKey: ["board"] });
      refreshReview.mutate();
    },
    onError: (e) => toast.error(`Update branch failed:\n${String(e)}`),
  });

  const setAutoMerge = useMutation({
    mutationFn: (enable: boolean) => {
      if (owner == null || name == null || prNumber == null) {
        throw new Error("missing PR identity");
      }
      return api.setPrAutoMerge(owner, name, prNumber, enable);
    },
    onSuccess: (_, enable) => {
      toast.success(enable ? "Auto-merge enabled" : "Auto-merge disabled");
      queryClient.invalidateQueries({ queryKey: ["pr-meta", owner, name, prNumber] });
      queryClient.invalidateQueries({ queryKey: ["board"] });
    },
    onError: (e) => toast.error(`Auto-merge failed:\n${String(e)}`),
  });

  const noCloneReason = detail.local_path ? null : "Link a local clone first";
  const defaultTerminal = useSettingsStore((s) => s.defaultTerminal);
  const tmuxTarget = useSettingsStore((s) => s.tmuxTarget);
  const tmuxSessionName = useSettingsStore((s) => s.tmuxSessionName);
  const agentSessionsQuery = useQuery({
    queryKey: ["review-agent-sessions", review.id],
    queryFn: () => api.reviewAgentSessions(review.id),
    enabled: detail.local_path != null,
    refetchInterval: 15_000,
  });
  const invalidateWorktree = () => {
    queryClient.invalidateQueries({ queryKey: ["chat-branch", review.id] });
    queryClient.invalidateQueries({ queryKey: ["worktrees", target.repo_id] });
  };
  const openShell = useMutation({
    mutationFn: async () => {
      const worktreePath = await api.ensureChatWorktree(review.id);
      await api.worktreeOpenTerminal(target.repo_id, worktreePath, defaultTerminal, tmuxTarget, tmuxSessionName);
    },
    onSuccess: invalidateWorktree,
    onError: (e) => toast.error(`Could not open terminal:\n${String(e)}`),
  });
  const openAgent = useMutation({
    mutationFn: () => api.reviewOpenAgent(review.id, defaultTerminal, tmuxTarget, tmuxSessionName),
    onSuccess: () => {
      invalidateWorktree();
      queryClient.invalidateQueries({ queryKey: ["review-agent-sessions", review.id] });
      queryClient.invalidateQueries({ queryKey: ["claude-sessions"] });
    },
    onError: (e) => toast.error(`Could not open agent session:\n${String(e)}`),
  });
  const focusSession = useMutation({
    mutationFn: api.focusClaudeSession,
    onError: (e) => toast.error(`Could not focus agent session:\n${String(e)}`),
  });

  const reanchorComments = useMutation({
    mutationFn: () => api.reanchorComments(review.id),
    onSuccess: (r) => {
      queryClient.invalidateQueries({ queryKey: ["review", review.id] });
      toast.success(`Re-anchored ${r.reanchored}, ${r.lost} could not be moved`);
    },
    onError: (e) => toast.error(`Re-anchor failed:\n${String(e)}`),
  });

  const draftReplyCount = detail.comments.filter(
    (c) => c.github_thread_root_id != null,
  ).length;

  const publishDraftReplies = useMutation({
    mutationFn: () => api.publishDraftReplies(review.id),
    onSuccess: (result) => {
      invalidateReview();
      queryClient.invalidateQueries({ queryKey: ["pr-threads", owner, name, prNumber] });
      if (result.failed > 0) {
        toast.error(
          `Published ${result.published}, ${result.failed} failed:\n${result.errors.join("\n")}`,
        );
      } else {
        toast.success(
          `Published ${result.published} draft repl${result.published === 1 ? "y" : "ies"}.`,
        );
      }
    },
    onError: (e) => toast.error(`Publish draft replies failed:\n${String(e)}`),
  });

  const isPr = target.kind === "github_pr";
  const hasPrMeta = isPr && !!detail.remote_owner && !!detail.remote_name && target.github_pr_number != null;
  const published = review.status === "published";
  const pending = review.status === "published_pending";
  const isDraft = review.status === "draft";
  const verdictLabel =
    review.event === "approve"
      ? "Approve"
      : review.event === "request_changes"
        ? "Request changes"
        : "Comment";
  const anchorsStale = detail.comments.some(
    (c) => !isDraftReply(c) && isCommentOutdated(c, target.base_sha, target.head_sha),
  );
  const prUrl =
    isPr && detail.remote_owner && detail.remote_name && target.github_pr_number != null
      ? githubPrUrl(detail.remote_owner, detail.remote_name, target.github_pr_number)
      : null;
  const sourceRepo = owner && name ? `${owner}/${name}` : null;
  const displayRepo = detail.friendly_name?.trim() || sourceRepo;

  return (
    <header className="review-header">
      <div className="review-header-top">
        {hasPrMeta && (
          <button
            className="btn btn-ghost"
            title={collapsed ? "Show PR details" : "Hide PR details"}
            onClick={() => setCollapsed((c) => !c)}
          >
            <Icon name="chev" size={13} /> {collapsed ? "Expand" : "Collapse"}
          </button>
        )}
        <div className="review-title-wrap">
          <h2 className="review-title">{target.title}</h2>
          {(sourceRepo || detail.local_path) && (
            <span className="review-repo mono">
              {sourceRepo && (
                <button
                  className="review-repo-link"
                  title={`Open ${sourceRepo}`}
                  onClick={() => openRepo(detail.target.repo_id)}
                >
                  {displayRepo}
                </button>
              )}
              {prNumber != null && prUrl ? (
                <PrNumberLink prNumber={prNumber} prUrl={prUrl} />
              ) : prNumber != null ? (
                <span> #{prNumber}</span>
              ) : null}
            </span>
          )}
        </div>
        <ReviewSwitcher targetId={target.id} currentReviewId={review.id} />
        {isPr && (
          <span className="review-pr-chips">
            <StatusPill
              state={prMetaQuery.data ? prMetaQuery.data.state.toLowerCase() : null}
              isDraft={prMetaQuery.data?.isDraft ?? false}
            />
            <CiBadge state={prMetaQuery.data?.ciState ? prMetaQuery.data.ciState.toLowerCase() : null} />
            <ReviewDecisionBadge decision={prMetaQuery.data?.reviewDecision} />
            <AutoMergeBadge
              method={prMetaQuery.data?.autoMerge?.mergeMethod ?? (prMetaQuery.data?.autoMerge ? "MERGE" : null)}
              enabledBy={prMetaQuery.data?.autoMerge?.enabledBy?.login}
            />
          </span>
        )}
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
        {focusView?.focus.planned_at != null && (
          <FocusLevelSelect level={focusView.focus.level} onChange={focusActions.setLevel} />
        )}
        <FocusGenerate
          key={focusView?.focus.planned_at ?? "none"}
          view={focusView}
          claudeAvailable={claudeAvailable}
          generate={generateFocus}
        />
        {anchorsStale && (
          <span className="head-moved">
            <span
              className="head-moved-badge"
              title="The base or head has moved since some comments were written — they may no longer line up with the code."
            >
              ⚠ target moved
            </span>
            <button
              className="btn btn-sm"
              disabled={readOnly || reanchorComments.isPending}
              title={
                readOnly
                  ? "Published reviews cannot be re-anchored"
                  : "Move comments onto the current base/head and clear outdated badges"
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
        {isPr && draftReplyCount > 0 && (
          <button
            className="btn btn-primary"
            disabled={publishDraftReplies.isPending}
            title="Post all draft replies to their GitHub threads"
            onClick={() => publishDraftReplies.mutate()}
          >
            {publishDraftReplies.isPending
              ? "Publishing…"
              : `Publish ${draftReplyCount} draft repl${draftReplyCount === 1 ? "y" : "ies"}`}
          </button>
        )}
        {isPr && (isDraft || published) && (
          <PublishButton
            published={published}
            pending={publishReview.isPending}
            draftPending={publishPending.isPending}
            verdictLabel={verdictLabel}
            onPublish={(event) => publishReview.mutate(event)}
            onPublishDraft={(event) => publishPending.mutate(event)}
          />
        )}
        {isPr && pending && (
          <>
            <button
              className="btn btn-primary"
              disabled={submitPending.isPending}
              title="Submit the pending review to the PR with the stored verdict"
              onClick={async () => {
                if (
                  await confirmDialog({
                    title: "Submit review",
                    message: `Submit the pending review to the PR with verdict “${verdictLabel}”? This cannot be undone.`,
                    confirmLabel: "Submit review",
                    danger: true,
                  })
                )
                  submitPending.mutate();
              }}
            >
              {submitPending.isPending ? (
                <>
                  <span className="spinner" /> Submitting…
                </>
              ) : (
                "Submit review"
              )}
            </button>
            <button
              className="btn"
              disabled={discardPending.isPending}
              title="Delete the pending review from GitHub; your local draft is kept"
              onClick={async () => {
                if (
                  await confirmDialog({
                    title: "Discard pending review",
                    message:
                      "Delete the pending review from GitHub? Your local draft is kept and unlocked.",
                    confirmLabel: "Discard pending review",
                    danger: true,
                  })
                )
                  discardPending.mutate();
              }}
            >
              {discardPending.isPending ? "Discarding…" : "Discard pending review"}
            </button>
          </>
        )}
        <KebabMenu title="Review actions">
          {(close) => {
            const run = (fn: () => void) => () => {
              close();
              fn();
            };
            const prOpen = prMetaQuery.data?.state === "OPEN";
            const updateDisabledReason = !prMetaQuery.data
              ? "Loading PR status…"
              : !prOpen
                ? "PR is not open"
                : prMetaQuery.data.mergeStateStatus !== "BEHIND"
                  ? "Branch is up to date"
                  : !prMetaQuery.data.viewerCanUpdateBranch
                    ? "You can't update this branch"
                    : null;
            const autoMergeOn = prMetaQuery.data?.autoMerge != null;
            const autoMergeDisabledReason = !prMetaQuery.data
              ? "Loading PR status…"
              : !prOpen
                ? "PR is not open"
                : prMetaQuery.data.isDraft && !autoMergeOn
                  ? "Draft PRs can't be merged"
                  : null;
            const toggleAutoMerge = async () => {
              close();
              if (autoMergeOn) {
                setAutoMerge.mutate(false);
                return;
              }
              if (
                await confirmDialog({
                  title: "Enable auto-merge",
                  message: `Squash-merge #${prNumber} "${target.title}" and delete its branch once all requirements pass?`,
                  confirmLabel: "Enable auto-merge",
                })
              )
                setAutoMerge.mutate(true);
            };
            return (
              <>
                {isPr && (
                  <>
                    <button
                      role="menuitem"
                      disabled={autoMergeDisabledReason != null || setAutoMerge.isPending}
                      title={autoMergeDisabledReason ?? undefined}
                      onClick={toggleAutoMerge}
                    >
                      {autoMergeOn ? "Disable auto-merge" : "Enable auto-merge (squash)"}
                    </button>
                    <button
                      role="menuitem"
                      disabled={!prOpen || togglePrDraft.isPending}
                      title={prOpen ? undefined : "PR is not open"}
                      onClick={run(() => togglePrDraft.mutate(!prMetaQuery.data!.isDraft))}
                    >
                      {prMetaQuery.data?.isDraft ? "Mark ready for review" : "Convert to draft"}
                    </button>
                    <button
                      role="menuitem"
                      disabled={updateDisabledReason != null || updateBranch.isPending}
                      title={updateDisabledReason ?? undefined}
                      onClick={run(() => updateBranch.mutate(false))}
                    >
                      Update branch (merge)
                    </button>
                    <button
                      role="menuitem"
                      disabled={updateDisabledReason != null || updateBranch.isPending}
                      title={updateDisabledReason ?? undefined}
                      onClick={run(() => updateBranch.mutate(true))}
                    >
                      Update branch (rebase)
                    </button>
                    <hr className="btn-split-menu-separator" />
                  </>
                )}
                <button
                  role="menuitem"
                  disabled={noCloneReason != null || openingWorktree}
                  title={noCloneReason ?? "Open the chat worktree in VSCode"}
                  onClick={run(openChatWorktreeInVscode)}
                >
                  Open in VSCode
                </button>
                <button
                  role="menuitem"
                  disabled={noCloneReason != null || openShell.isPending}
                  title={noCloneReason ?? "Open a shell in the chat worktree"}
                  onClick={run(() => openShell.mutate())}
                >
                  Open in terminal
                </button>
                <AgentSessionItems
                  sessions={agentSessionsQuery.data ?? []}
                  busy={openAgent.isPending || focusSession.isPending}
                  disabledReason={noCloneReason ?? (claudeAvailable ? null : "Claude CLI not found")}
                  close={close}
                  onFocus={(id) => focusSession.mutate(id)}
                  onOpenNew={() => openAgent.mutate()}
                />
                <hr className="btn-split-menu-separator" />
                <button role="menuitem" onClick={run(() => setShowExport(true))}>
                  Export…
                </button>
                <hr className="btn-split-menu-separator" />
                <button
                  role="menuitem"
                  className="danger"
                  disabled={pending}
                  title={
                    pending
                      ? "Discard the pending GitHub review before deleting this review"
                      : "Delete this review and all its comments"
                  }
                  onClick={async () => {
                    close();
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
                  Delete review
                </button>
              </>
            );
          }}
        </KebabMenu>
      </div>

      {showExport && (
        <ExportModal
          reviewId={review.id}
          title={target.title}
          repoPath={detail.local_path}
          onClose={() => setShowExport(false)}
          onExported={() => {
            queryClient.invalidateQueries({ queryKey: ["review", review.id] });
            queryClient.invalidateQueries({ queryKey: ["reviews"] });
          }}
        />
      )}

      {!collapsed && hasPrMeta && (
        <PrMetaPanel
          owner={detail.remote_owner!}
          name={detail.remote_name!}
          number={target.github_pr_number!}
          active={isActiveTab}
        />
      )}
    </header>
  );
}

function ReviewDiff({
  files,
  visibleFileIdx,
  viewType,
  detail,
  threads,
  readOnly,
  kbHandles,
  searchKeysByFile,
  searchCurrent,
  focusModel,
  focusActions,
  onReveal,
  onUnreveal,
  onOpenFilePane,
  onSaving,
  onSaved,
  onCommentsChanged,
  onSendToChat,
}: {
  files: FileData[];
  /** Indices to render; `null` means render everything (no type filter active). */
  visibleFileIdx: Set<number> | null;
  viewType: ViewType;
  detail: ReviewDetail;
  threads: PrThread[];
  readOnly: boolean;
  kbHandles: MutableRefObject<Map<number, FileKbHandle>>;
  searchKeysByFile: Map<number, Set<string>>;
  searchCurrent: DiffMatch | null;
  focusModel: FileFocus[] | null;
  focusActions: FocusActions;
  onReveal: (ids: string[]) => void;
  onUnreveal: (ids: string[]) => void;
  onOpenFilePane: (path: string) => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
  onSendToChat?: (msg: string) => void;
}) {
  return (
    <div className="diff-files">
      {files.map((file, index) => {
        if (visibleFileIdx != null && !visibleFileIdx.has(index)) return null;
        return (
          <FileReview
            key={`${file.oldRevision}-${file.newRevision}-${index}`}
            index={index}
            file={file}
            viewType={viewType}
            detail={detail}
            threads={threads}
            readOnly={readOnly}
            kbHandles={kbHandles}
            searchKeys={searchKeysByFile.get(index)}
            searchCurrentKey={
              searchCurrent?.fileIndex === index ? searchCurrent.changeKey : null
            }
            focus={focusModel?.[index] ?? null}
            focusActions={focusActions}
            onReveal={onReveal}
            onUnreveal={onUnreveal}
            onOpenFilePane={onOpenFilePane}
            onSaving={onSaving}
            onSaved={onSaved}
            onCommentsChanged={onCommentsChanged}
            onSendToChat={onSendToChat}
          />
        );
      })}
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
  kbHandles,
  searchKeys,
  searchCurrentKey,
  focus,
  focusActions,
  onReveal,
  onUnreveal,
  onOpenFilePane,
  onSaving,
  onSaved,
  onCommentsChanged,
  onSendToChat,
}: {
  index: number;
  file: FileData;
  viewType: ViewType;
  detail: ReviewDetail;
  threads: PrThread[];
  readOnly: boolean;
  kbHandles: MutableRefObject<Map<number, FileKbHandle>>;
  searchKeys: Set<string> | undefined;
  searchCurrentKey: string | null;
  focus: FileFocus | null;
  focusActions: FocusActions;
  onReveal: (ids: string[]) => void;
  onUnreveal: (ids: string[]) => void;
  onOpenFilePane: (path: string) => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
  onSendToChat?: (msg: string) => void;
}) {
  const queryClient = useQueryClient();
  const reviewId = detail.review.id;
  const path = fileDisplayPath(file);
  const threadCtx: PrThreadCtx | null =
    detail.remote_owner && detail.remote_name && detail.target.github_pr_number != null
      ? {
          owner: detail.remote_owner,
          name: detail.remote_name,
          number: detail.target.github_pr_number,
        }
      : null;

  // Local draft replies to GitHub threads, keyed by the target thread's root
  // databaseId. Rendered alongside the ephemeral GitHub thread they target.
  const draftRepliesByRoot = useMemo(() => {
    const map = new Map<number, Comment[]>();
    for (const c of detail.comments) {
      if (c.github_thread_root_id == null) continue;
      const arr = map.get(c.github_thread_root_id) ?? [];
      arr.push(c);
      map.set(c.github_thread_root_id, arr);
    }
    return map;
  }, [detail.comments]);

  const isDeleted = file.type === "delete";
  const [openingInVscode, setOpeningInVscode] = useState(false);
  const openInChatWorktree = async () => {
    if (isDeleted || openingInVscode) return;
    setOpeningInVscode(true);
    try {
      const worktreePath = await api.ensureChatWorktree(detail.review.id);
      await api.openInVscode(worktreePath, path);
    } catch (err) {
      toast.error(`Could not open file:\n${String(err)}`);
    } finally {
      setOpeningInVscode(false);
    }
  };

  // The .diff-file element; scoped queries for the keyboard line cursor scroll
  // to *this* file's `.kb-focus` row, never another (hidden) tab's duplicate.
  const rootRef = useRef<HTMLDivElement>(null);
  // The j/k line cursor, local so its churn never re-renders the diff subtree of
  // other files. Exposed to ReviewView's dispatcher through the registry handle.
  const [kbFocusKey, setKbFocusKey] = useState<string | null>(null);

  // Hunks the user has expanded sit alongside the parsed ones; this state feeds
  // BOTH rendering and anchoring so revealed lines are clickable/commentable.
  // Ephemeral by design — reparsing (a new diff) resets it.
  const [hunks, setHunks] = useState<HunkData[]>(file.hunks);
  useEffect(() => {
    setHunks(file.hunks);
    setKbFocusKey(null);
  }, [file]);
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
  const markEditsEnabled = useSettingsStore((s) => s.markEdits);
  const largeDiffMaxLines = useSettingsStore((s) => s.largeDiffMaxLines);
  const tooLarge = useMemo(() => isLargeDiff(file, largeDiffMaxLines), [file, largeDiffMaxLines]);
  const [selection, setSelection] = useState<Selection | null>(null);
  const viewed = detail.viewed_files.includes(path);
  const tokens = useMemo(
    () =>
      tooLarge || viewed
        ? undefined
        : tokenizeFile({ ...file, hunks }, { markEditsMaxChanges: markEditsEnabled ? undefined : 0 }),
    [file, hunks, markEditsEnabled, tooLarge, viewed],
  );
  const [fileComposerOpen, setFileComposerOpen] = useState(false);

  // indexFile walks hunks (and user-expanded context) in document order, so the
  // map's key order is the visible top-to-bottom line order j/k steps through.
  const orderedKeys = useMemo(() => [...metaByKey.keys()], [metaByKey]);

  // Register this file's keyboard handle. Dep-less so it always closes over the
  // latest state/handlers; cleaned up by index on unmount.
  useEffect(() => {
    const handle: FileKbHandle = {
      moveCursor: (d) => {
        if (viewed || tooLarge) return; // FileBody isn't rendered, nothing to outline
        setKbFocusKey((cur) => moveCursorKey(orderedKeys, cur, d));
      },
      openComposer: () => {
        if (readOnly || !kbFocusKey) return;
        const meta = metaByKey.get(kbFocusKey);
        if (!meta) return;
        setSelection({
          side: meta.side,
          anchorLine: meta.line,
          focusLine: meta.line,
          focusKey: kbFocusKey,
        });
      },
      clearCursor: () => {
        if (selection) {
          setSelection(null);
          return true;
        }
        if (kbFocusKey) {
          setKbFocusKey(null);
          return true;
        }
        return false;
      },
    };
    kbHandles.current.set(index, handle);
    return () => void kbHandles.current.delete(index);
  });

  // Focused-row outline, same generateLineClassName API the full-file pane uses.
  // Works in unified and split (split rows carry both sides' changes).
  const generateLineClassName = useCallback(
    ({
      changes,
      defaultGenerate,
    }: {
      changes: ChangeData[];
      defaultGenerate: () => string;
    }): string | undefined => {
      const keyed = (k: string | null | undefined) =>
        k != null && changes.some((c) => c != null && changeKeyOf(c) === k);
      const extra: string[] = [];
      if (keyed(kbFocusKey)) extra.push("kb-focus");
      if (searchKeys && changes.some((c) => c != null && searchKeys.has(changeKeyOf(c))))
        extra.push("search-match");
      if (keyed(searchCurrentKey)) extra.push("search-current");
      return [defaultGenerate(), ...extra].filter(Boolean).join(" ") || undefined;
    },
    [kbFocusKey, searchKeys, searchCurrentKey],
  );

  // Keep the cursor in view. Scope to rootRef — never document — since hidden
  // tabs render duplicate `#file-N` subtrees.
  useEffect(() => {
    if (kbFocusKey == null) return;
    rootRef.current
      ?.querySelector(".kb-focus")
      ?.scrollIntoView({ block: "nearest" });
  }, [kbFocusKey]);

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
        c.origin !== "file_view" && // shown in the full-file pane, not the diff
        !isDraftReply(c), // rendered inside its GitHub thread instead
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

  // A suggestion seed for the open composer: only for a RIGHT-side selection
  // whose every line resolves to current head text. null hides the button.
  const composerSuggestionSeed = useMemo(() => {
    if (!selection || !range || selection.side !== "RIGHT") return null;
    const lines = rightLinesText(hunks, range.lo, range.hi);
    return lines ? suggestionFence(lines) : null;
  }, [selection, range, hunks]);

  // A seed for an existing RIGHT-side line/diff comment, from its anchored
  // range — null (hidden button) for non-RIGHT, file/file-view, outdated, or
  // unresolvable lines (collapsed gaps).
  const suggestionSeedFor = useCallback(
    (c: Comment): string | null => {
      if (c.side !== "RIGHT" || c.subject_type !== "line" || c.origin !== "diff") return null;
      if (c.anchored_head_sha && c.anchored_head_sha !== detail.target.head_sha) return null;
      const lines = rightLinesText(hunks, c.start_line ?? c.line, c.line);
      return lines ? suggestionFence(lines) : null;
    },
    [hunks, detail.target.head_sha],
  );

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

  const sendSelectionToChat = (text: string) => {
    if (!onSendToChat || !selection || !range) return;
    const header = metaByKey.get(selection.focusKey)?.hunk ?? "";
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
    const lineLabel = range.lo === range.hi ? `line ${range.lo}` : `lines ${range.lo}–${range.hi}`;
    const msg = `Regarding \`${path}\` ${lineLabel} (${selection.side}):\n\`\`\`\n${diffHunk}\n\`\`\`\n\n${text}`;
    setSelection(null);
    onSendToChat(msg);
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
          baseSha={detail.target.base_sha}
          composerOpen={!!composerOpen}
          rangeLabel={rangeLabel}
          composerSuggestionSeed={composerOpen ? composerSuggestionSeed : null}
          suggestionSeedFor={suggestionSeedFor}
          readOnly={readOnly}
          canReply={!readOnly}
          onCloseComposer={() => setSelection(null)}
          onAdd={submitSelectionComment}
          onSendToChat={onSendToChat ? sendSelectionToChat : undefined}
          onSaving={onSaving}
          onSaved={onSaved}
          onCommentsChanged={onCommentsChanged}
        />
        {keyThreads.map((t) => (
          <GithubThread
            key={t.id}
            thread={t}
            ctx={threadCtx}
            reviewId={reviewId}
            draftReplies={draftRepliesByRoot.get(t.comments[0]?.databaseId ?? -1)}
            readOnly={readOnly}
            onCommentsChanged={onCommentsChanged}
          />
        ))}
      </>
    );
  }

  const { add, del } = useMemo(() => countChanges(file), [file]);

  // Resolution progress for this file: count roots (file-view roots belong to the
  // file even though they render in the pane) and how many are resolved.
  const { rootCount, resolvedCount } = useMemo(() => {
    const roots = detail.comments.filter(
      (c) => c.file_path === path && c.parent_id == null && !isDraftReply(c),
    );
    return {
      rootCount: roots.length,
      resolvedCount: roots.filter((c) => c.resolved_at != null).length,
    };
  }, [detail.comments, path]);

  return (
    <div className="diff-file" id={`file-${index}`} ref={rootRef}>
      <div className="diff-file-header">
        <span className="file-path-group">
          <span className="file-path mono" title={path}>{path}</span>
          <button
            className="btn-icon copy-path-btn"
            title="Copy relative file path"
            onClick={(e) => {
              e.stopPropagation();
              navigator.clipboard.writeText(path);
              toast.success("Path copied to clipboard");
            }}
          >
            <Icon name="copy" size={12} />
          </button>
        </span>
        <span className="diff-stats">
          <span className="delta-add">+{add}</span>
          <span className="delta-del">−{del}</span>
          {rootCount > 0 && (
            <span className="comment-count muted">
              · {rootCount} comments
              {resolvedCount > 0 ? `, ${resolvedCount} resolved` : ""}
            </span>
          )}
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
                  : tooLarge
                    ? "File too large to view here; open it in VSCode"
                    : "View the full file and comment on any line"
            }
            disabled={isDeleted || file.isBinary || tooLarge}
            onClick={() => onOpenFilePane(path)}
          >
            <Icon name="eye" size={12} /> View file
          </button>
          <button
            className="btn btn-sm btn-ghost open-file-btn"
            title={
              isDeleted
                ? "File was deleted; no working copy to open"
                : "Open in VSCode (chat worktree)"
            }
            disabled={isDeleted || openingInVscode}
            onClick={openInChatWorktree}
          >
            <Icon name="ext" size={12} /> Open
          </button>
          {focus && !focus.fullyHidden && (
            <button
              className="btn btn-sm btn-ghost"
              title="Hide this file's hunks (focus override)"
              onClick={() => {
                const all = focus.whole ? [focus.whole] : focus.hunks;
                const overridden = all
                  .map((h) => h.entry)
                  .filter((e) => e?.override != null)
                  .map((e) => e!.hunk_id);
                if (overridden.length) focusActions.setHunkOverride(overridden, null);
                focusActions.setFileOverride(path, "hide");
              }}
            >
              <Icon name="eye" size={12} /> Hide file
            </button>
          )}
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
              baseSha={detail.target.base_sha}
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
      {viewed ? null : tooLarge ? (
        <p className="muted binary-note">
          Diff too large to display ({(add + del).toLocaleString()} changed lines, limit{" "}
          {largeDiffMaxLines.toLocaleString()}).{" "}
          {!isDeleted && (
            <button
              className="btn btn-sm btn-ghost"
              disabled={openingInVscode}
              onClick={openInChatWorktree}
            >
              <Icon name="ext" size={12} /> Open in VSCode
            </button>
          )}
        </p>
      ) : (
        <FileBody
          file={file}
          hunks={hunks}
          viewType={viewType}
          tokens={tokens}
          widgets={widgets}
          selectedChanges={selectedChanges}
          orphans={orphans}
          orphanThreads={orphanThreads}
          threadCtx={threadCtx}
          reviewId={reviewId}
          draftRepliesByRoot={draftRepliesByRoot}
          headSha={detail.target.head_sha}
          baseSha={detail.target.base_sha}
          readOnly={readOnly}
          canExpand={canExpand}
          focus={focus}
          focusActions={focusActions}
          onReveal={onReveal}
          onUnreveal={onUnreveal}
          expanding={expanding}
          expandError={expandError}
          oldLineCount={oldLineCount}
          generateLineClassName={generateLineClassName}
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
  threadCtx,
  reviewId,
  draftRepliesByRoot,
  headSha,
  baseSha,
  readOnly,
  canExpand,
  focus,
  focusActions,
  onReveal,
  onUnreveal,
  expanding,
  expandError,
  oldLineCount,
  generateLineClassName,
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
  threadCtx: PrThreadCtx | null;
  reviewId: number;
  draftRepliesByRoot: Map<number, Comment[]>;
  headSha: string | null;
  baseSha: string | null;
  readOnly: boolean;
  canExpand: boolean;
  focus: FileFocus | null;
  focusActions: FocusActions;
  onReveal: (ids: string[]) => void;
  onUnreveal: (ids: string[]) => void;
  expanding: boolean;
  expandError: string | null;
  oldLineCount: number | null; // null until the base source is fetched
  generateLineClassName: (args: {
    changes: ChangeData[];
    defaultGenerate: () => string;
  }) => string | undefined;
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
              baseSha={baseSha}
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
            <GithubThread
              key={t.id}
              thread={t}
              ctx={threadCtx}
              reviewId={reviewId}
              draftReplies={draftRepliesByRoot.get(t.comments[0]?.databaseId ?? -1)}
              readOnly={readOnly}
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
          generateLineClassName={generateLineClassName}
          gutterEvents={{ onClick: onLineClick }}
        >
          {(renderedHunks) => {
            // Each rendered hunk maps back to the parsed hunks it covers (context
            // expansion merges neighbours). Hidden hunks stay in `hunks` so line
            // numbers and gap math are unchanged; they render as one bar per run.
            const focusOf = (h: HunkData): HunkFocus[] =>
              focus ? originalsIn(h, file.hunks).map((i) => focus.hunks[i]) : [];
            const isHidden = (h: HunkData | null) => {
              if (!h) return false;
              const fs = focusOf(h);
              return fs.length > 0 && fs.every((f) => f.hidden);
            };
            const rows: React.ReactElement[] = [];
            let run: HunkFocus[] = [];
            const flushRun = (key: string) => {
              if (run.length === 0) return;
              const ids = focusIds(run);
              rows.push(
                <Decoration key={`focus-bar-${key}`} className="focus-bar-row">
                  <FocusHiddenBar
                    hunks={run}
                    onReveal={() => onReveal(ids)}
                    onAlwaysShow={() => focusActions.setHunkOverride(ids, "show")}
                  />
                </Decoration>,
              );
              run = [];
            };
            renderedHunks.forEach((hunk, i) => {
              // All three gap kinds expand: leading (above the first hunk),
              // between hunks, and trailing (below the last hunk — its size is
              // unknown until the base source is fetched, so it renders
              // optimistically with no count). Gaps next to a hidden hunk don't.
              const prev = i === 0 ? null : renderedHunks[i - 1];
              const hidden = isHidden(hunk);
              if (hidden) {
                run.push(...focusOf(hunk));
                return;
              }
              flushRun(hunk.content);
              const collapsed = getCollapsedLinesCountBetween(prev, hunk);
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
              if (canExpand && prev != null && !isHidden(prev) && collapsed > 0) {
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
              const metas = focusOf(hunk).filter((f) => f.entry);
              if (metas.length > 0) {
                rows.push(
                  <Decoration key={`focus-meta-${hunk.content}`} className="focus-meta-row-wrap">
                    <>
                      {metas.map((f) => (
                        <FocusHunkMeta
                          key={f.entry!.hunk_id}
                          hunk={f}
                          onHide={() => {
                            onUnreveal([f.entry!.hunk_id]);
                            focusActions.setHunkOverride([f.entry!.hunk_id], "hide");
                          }}
                        />
                      ))}
                    </>
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
            });
            flushRun("end");
            return rows;
          }}
        </Diff>
      )}
    </>
  );
}

function focusIds(hunks: HunkFocus[]): string[] {
  return hunks.flatMap((h) => (h.entry ? [h.entry.hunk_id] : []));
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
  baseSha,
  composerOpen,
  rangeLabel,
  composerSuggestionSeed,
  suggestionSeedFor,
  readOnly,
  showOrigin,
  canReply = true,
  onCloseComposer,
  onAdd,
  onSendToChat,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  threads: CommentThread[];
  headSha: string | null;
  baseSha: string | null;
  composerOpen: boolean;
  rangeLabel?: string;
  composerSuggestionSeed?: string | null;
  suggestionSeedFor?: (c: Comment) => string | null;
  readOnly: boolean;
  showOrigin?: boolean;
  canReply?: boolean;
  onCloseComposer: () => void;
  onAdd: (text: string) => Promise<void>;
  onSendToChat?: (text: string) => void;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  if (threads.length === 0 && !composerOpen) return null;

  return (
    <div className="line-widget">
      {threads.map((t) => (
        <ThreadItem
          key={t.root.id}
          thread={t}
          headSha={headSha}
          baseSha={baseSha}
          readOnly={readOnly}
          showOrigin={showOrigin}
          canReply={canReply && !readOnly}
          suggestionSeedFor={suggestionSeedFor}
          onSaving={onSaving}
          onSaved={onSaved}
          onCommentsChanged={onCommentsChanged}
        />
      ))}
      {composerOpen && !readOnly && (
        <Composer
          onSubmit={onAdd}
          onCancel={onCloseComposer}
          rangeLabel={rangeLabel}
          suggestionSeed={composerSuggestionSeed}
          onSendToChat={onSendToChat}
        />
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
  baseSha,
  readOnly,
  showOrigin,
  canReply,
  suggestionSeedFor,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  thread: CommentThread;
  headSha: string | null;
  baseSha: string | null;
  readOnly: boolean;
  showOrigin?: boolean;
  canReply?: boolean;
  suggestionSeedFor?: (c: Comment) => string | null;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const [replyOpen, setReplyOpen] = useState(false);
  const { root, replies } = thread;

  // Resolved threads default to a collapsed one-line bar; re-derive when the
  // resolved state flips so resolving collapses and unresolving expands.
  const isResolved = root.resolved_at != null;
  const [expanded, setExpanded] = useState(() => !isResolved);
  const prevResolved = useRef(isResolved);
  useEffect(() => {
    if (prevResolved.current !== isResolved) {
      setExpanded(!isResolved);
      prevResolved.current = isResolved;
    }
  }, [isResolved]);

  const submitReply = async (text: string) => {
    onSaving();
    await api.addReply({ reviewId: root.review_id, parentId: root.id, body: text });
    setReplyOpen(false);
    onSaved();
    onCommentsChanged();
  };

  if (isResolved && !expanded) {
    return (
      <button className="resolved-bar" onClick={() => setExpanded(true)}>
        <span className="resolved-bar-text">
          ✓ Resolved — {summaryLine(root.body)}
        </span>
        <span className="resolved-bar-caret">▸</span>
      </button>
    );
  }

  return (
    <div className="comment-thread" data-comment-id={root.id}>
      {isResolved && (
        <button
          className="resolved-collapse"
          title="Collapse resolved thread"
          onClick={() => setExpanded(false)}
        >
          ▾ Hide resolved thread
        </button>
      )}
      <CommentItem
        comment={root}
        headSha={headSha}
        baseSha={baseSha}
        readOnly={readOnly}
        showOrigin={showOrigin}
        replyCount={replies.length}
        suggestionSeed={suggestionSeedFor?.(root) ?? null}
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
              baseSha={baseSha}
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

/** Root-comment-only tag pill: none/ai/local. Tagged comments never publish. */
function CommentTagPill({ comment, readOnly }: { comment: Comment; readOnly: boolean }) {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  if (readOnly) {
    if (!comment.tag) return null;
    return comment.tag === "ai" ? (
      <span className="ai-pill" title="Written by AI; not published">
        AI
      </span>
    ) : (
      <span className="local-pill" title="Local only; never published">
        Local
      </span>
    );
  }

  const setTag = (tag: "ai" | "local" | null) => {
    setOpen(false);
    const key = ["review", comment.review_id];
    queryClient.setQueriesData<ReviewDetail>({ queryKey: key }, (old) =>
      old
        ? { ...old, comments: old.comments.map((c) => (c.id === comment.id ? { ...c, tag } : c)) }
        : old,
    );
    api.setCommentTag(comment.id, tag).catch((e) => {
      toast.error(String(e));
      queryClient.invalidateQueries({ queryKey: key });
    });
  };

  return (
    <span className="comment-tag-menu" ref={ref}>
      <button
        type="button"
        className={
          comment.tag === "ai" ? "ai-pill" : comment.tag === "local" ? "local-pill" : "expand-btn"
        }
        title="Tag: ai/local comments are never published to GitHub"
        onClick={() => setOpen((o) => !o)}
      >
        {comment.tag === "ai" ? "AI" : comment.tag === "local" ? "Local" : "+ tag"}
      </button>
      {open && (
        <span className="comment-tag-pop" role="menu">
          <button
            type="button"
            className={comment.tag == null ? "active" : ""}
            onClick={() => setTag(null)}
          >
            None
          </button>
          <button
            type="button"
            className={comment.tag === "ai" ? "active" : ""}
            onClick={() => setTag("ai")}
          >
            AI
          </button>
          <button
            type="button"
            className={comment.tag === "local" ? "active" : ""}
            onClick={() => setTag("local")}
          >
            Local
          </button>
        </span>
      )}
    </span>
  );
}

export function CommentItem({
  comment,
  headSha,
  baseSha,
  readOnly,
  showOrigin,
  onReply,
  replyCount,
  suggestionSeed,
  onSaving,
  onSaved,
  onCommentsChanged,
}: {
  comment: Comment;
  headSha: string | null;
  baseSha: string | null;
  readOnly: boolean;
  showOrigin?: boolean;
  onReply?: () => void;
  replyCount?: number;
  suggestionSeed?: string | null;
  onSaving: () => void;
  onSaved: () => void;
  onCommentsChanged: () => void;
}) {
  const [body, setBody] = useState(comment.body);
  const [tab, setTab] = useState<"write" | "preview">("preview");
  const save = useDebouncedCallback((text: string) => {
    onSaving();
    api
      .updateComment(comment.id, text)
      .then(onSaved)
      .catch((e) => toast.error(String(e)));
  }, 400);

  // The comment was anchored to a SHA its side has since moved past (LEFT → base,
  // RIGHT → head), so its line may no longer point at the code it was written
  // against. Flag it rather than silently mislanding it; Re-anchor clears it.
  const outdated = isCommentOutdated(comment, baseSha, headSha);
  const pin = anchorPin(comment);

  return (
    <div className="comment-item">
      {comment.parent_id == null && <CommentTagPill comment={comment} readOnly={readOnly} />}
      {showOrigin && comment.origin === "file_view" && (
        <span
          className="origin-badge"
          title="Authored in the full-file view — folds into the review summary on publish, not posted as an inline GitHub comment."
        >
          in summary
        </span>
      )}
      {outdated && pin && (
        <span
          className="stale-badge"
          title={`Anchored to ${pin.slice(0, 7)}, but the ${
            comment.side === "LEFT" ? "base" : "head"
          } has since moved — this comment may no longer line up with the code.`}
        >
          outdated · {pin.slice(0, 7)}
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
            <>
              {suggestionSeed != null && (
                <button
                  className="suggest-btn"
                  title="Insert a suggestion block prefilled with the current line(s)"
                  onClick={() => {
                    const next =
                      body.trim() === "" ? suggestionSeed : `${body}\n\n${suggestionSeed}`;
                    setBody(next);
                    save(next);
                  }}
                >
                  ± Insert suggestion
                </button>
              )}
              <textarea
                value={body}
                onChange={(e) => {
                  setBody(e.target.value);
                  save(e.target.value);
                }}
              />
            </>
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
      {!readOnly && comment.parent_id == null && (
        <button
          className="btn-sm btn-ghost btn-resolve"
          onClick={async () => {
            try {
              await api.setCommentResolved(comment.id, !comment.resolved_at);
              onCommentsChanged();
            } catch (e) {
              toast.error(String(e));
            }
          }}
        >
          {comment.resolved_at ? "Unresolve" : "Resolve"}
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
  secondaryLabel,
  onSecondarySubmit,
  suggestionSeed,
  onSendToChat,
}: {
  onSubmit: (text: string) => Promise<void>;
  onCancel: () => void;
  rangeLabel?: string;
  submitLabel?: string;
  secondaryLabel?: string;
  onSecondarySubmit?: (text: string) => Promise<void>;
  suggestionSeed?: string | null;
  onSendToChat?: (text: string) => void;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const run = async (fn: (text: string) => Promise<void>) => {
    if (busy || text.trim() === "") return;
    setBusy(true);
    try {
      await fn(text.trim());
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="composer">
      {rangeLabel && <div className="composer-range">{rangeLabel}</div>}
      <textarea
        autoFocus
        placeholder="Leave a comment…"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          } else if (e.key === "Enter" && (e.metaKey || e.altKey)) {
            e.preventDefault();
            run(onSubmit);
          }
        }}
      />
      <div className="composer-actions">
        {suggestionSeed != null && (
          <button
            className="suggest-btn"
            title="Insert a suggestion block prefilled with the current line(s)"
            onClick={() =>
              setText((t) =>
                t.trim() === "" ? suggestionSeed : `${t}\n\n${suggestionSeed}`,
              )
            }
          >
            ± Insert suggestion
          </button>
        )}
        {onSendToChat && (
          <button
            className="btn btn-ghost"
            disabled={busy || text.trim() === ""}
            title="Send to Chat panel instead of creating a comment"
            onClick={() => {
              if (text.trim()) {
                onSendToChat(text.trim());
                onCancel();
              }
            }}
          >
            Send to Chat
          </button>
        )}
        <button className="btn btn-ghost" onClick={onCancel}>
          Cancel
        </button>
        {onSecondarySubmit && (
          <button
            className="btn"
            disabled={busy || text.trim() === ""}
            onClick={() => run(onSecondarySubmit)}
          >
            {secondaryLabel}
          </button>
        )}
        <button
          className="btn btn-primary"
          disabled={busy || text.trim() === ""}
          onClick={() => run(onSubmit)}
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
  repoPath,
  onClose,
  onExported,
}: {
  reviewId: number;
  title: string;
  repoPath: string | null;
  onClose: () => void;
  onExported: () => void;
}) {
  const [vscodeExporting, setVscodeExporting] = useState(false);
  const isLocal = repoPath != null;

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
      await navigator.clipboard.writeText(path);
      toast.success(`Exported to ${path} (path copied)`);
      onClose();
    } catch (e) {
      toast.error(String(e));
    }
  };

  const copyToClipboard = async (format: "markdown" | "json") => {
    try {
      const content = await api.previewReview(reviewId, format);
      await navigator.clipboard.writeText(content);
      toast.success(`${format === "markdown" ? "Markdown" : "JSON"} copied to clipboard`);
    } catch (e) {
      toast.error(String(e));
    }
  };

  const doVscodeExport = async () => {
    setVscodeExporting(true);
    try {
      const path = await api.exportVscodeReview(reviewId);
      onExported();
      await navigator.clipboard.writeText(path);
      toast.success(`Exported to VSCode: ${path} (path copied)`);
      onClose();
    } catch (e) {
      toast.error(String(e));
    } finally {
      setVscodeExporting(false);
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
          {isLocal && (
            <button onClick={doVscodeExport} disabled={vscodeExporting}>
              {vscodeExporting ? "Exporting…" : "Export to VSCode"}
            </button>
          )}
          <button onClick={() => copyToClipboard("json")}>Copy JSON</button>
          <button onClick={() => copyToClipboard("markdown")}>Copy Markdown</button>
          <button onClick={() => doExport("json")}>Save JSON</button>
          <button className="btn-primary" onClick={() => doExport("markdown")}>
            Save Markdown
          </button>
        </div>
      </div>
    </div>
  );
}
