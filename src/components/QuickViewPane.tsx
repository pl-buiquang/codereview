import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { parseDiff } from "react-diff-view";
import { api } from "../lib/api";
import { countChanges, fileDisplayPath } from "../lib/diff";
import { useSettingsStore } from "../lib/settings";
import type { InboxItem } from "../lib/types";
import { DiffViewer } from "./DiffViewer";

/**
 * Right-hand slide-out previewing an Inbox PR's diff. Nothing is stored: the diff comes
 * straight from `gh pr diff` and the header actions are handled by the Inbox.
 * A transparent backdrop closes it on an outside click, like Esc and the close button.
 */
export function QuickViewPane({
  item,
  viewerLogin,
  onClose,
  onOpenReview,
  onApprove,
  onApproveAndMerge,
}: {
  item: InboxItem;
  viewerLogin: string | null;
  onClose: () => void;
  onOpenReview: () => void;
  onApprove: () => void;
  /** `autoMergeOn`: GitHub already has auto-merge on, so only the approval is needed. */
  onApproveAndMerge: (autoMergeOn: boolean) => void;
}) {
  const [owner, name] = item.repo.split("/");

  const diffQuery = useQuery({
    queryKey: ["quick-pr-diff", owner, name, item.number],
    queryFn: () => api.quickPrDiff(owner, name, item.number),
    staleTime: 60_000,
    retry: false,
  });
  // The mirror copy is instant; it drives the button guards only.
  const metaQuery = useQuery({
    queryKey: ["mirror-pr-meta", owner, name, item.number],
    queryFn: () => api.mirrorPrMeta(owner, name, item.number),
  });

  const meta = metaQuery.data;
  const own = viewerLogin != null && item.author_login === viewerLogin;
  const autoMergeOn = meta?.autoMerge != null;
  const mergeBlocked = !meta
    ? "Loading PR status…"
    : meta.state !== "OPEN"
      ? "PR is not open"
      : meta.isDraft && !autoMergeOn
        ? "Draft PRs can't be merged"
        : null;

  return (
    <QuickDiffPane
      title={
        <>
          <span className="mono">
            {item.repo}#{item.number}
          </span>{" "}
          {item.title}
        </>
      }
      titleText={item.title}
      diffText={diffQuery.data}
      isLoading={diffQuery.isLoading}
      error={diffQuery.isError ? diffQuery.error : null}
      errorHint="Use Open review for large PRs."
      onClose={onClose}
      actions={
        <>
          <button className="btn btn-sm btn-primary" onClick={onOpenReview} title="Open (or resume) the review">
            Open review
          </button>
          {!own && (
            <>
              <button className="btn btn-sm" onClick={onApprove} title="Approve the PR">
                Approve
              </button>
              <button
                className="btn btn-sm"
                disabled={mergeBlocked != null}
                onClick={() => onApproveAndMerge(autoMergeOn)}
                title={mergeBlocked ?? "Approve the PR and enable auto-merge (squash)"}
              >
                🚀 Approve & merge
              </button>
            </>
          )}
        </>
      }
    />
  );
}

/** The slide-out shell shared by the Inbox quick view and the new-commits quick view:
 *  backdrop + Esc close, a header with caller-supplied actions, a jump-to-file bar and the diff. */
export function QuickDiffPane({
  title,
  titleText,
  actions,
  diffText,
  isLoading,
  error,
  errorHint,
  onClose,
}: {
  title: ReactNode;
  titleText?: string;
  actions: ReactNode;
  diffText: string | undefined;
  isLoading: boolean;
  error: unknown;
  errorHint?: string;
  onClose: () => void;
}) {
  const viewType = useSettingsStore((s) => s.defaultViewType);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const files = useMemo(
    () =>
      diffText != null
        ? parseDiff(diffText).map((f) => ({ path: fileDisplayPath(f), ...countChanges(f) }))
        : [],
    [diffText],
  );

  const jumpTo = (path: string) => {
    for (const el of document.querySelectorAll<HTMLElement>(".quick-view-body [data-file-path]")) {
      if (el.dataset.filePath === path) {
        el.scrollIntoView({ block: "start" });
        return;
      }
    }
  };

  return (
    <>
      <div className="quick-view-backdrop" onClick={onClose} />
      <aside className="file-view-pane" aria-label="Quick view">
        <div className="file-view-pane-header">
          <span className="file-path" title={titleText}>
            {title}
          </span>
          {actions}
          <button className="btn-icon" title="Close (Esc)" onClick={onClose}>
            ✕
          </button>
        </div>
        <div className="file-view-pane-body quick-view-body">
          {isLoading && <p className="muted quick-view-note">Loading diff…</p>}
          {error != null && (
            <p className="muted quick-view-note">
              Could not load the diff ({String(error)}).{errorHint ? ` ${errorHint}` : ""}
            </p>
          )}
          {diffText != null && (
            <>
              {files.length > 1 && <FileSelect files={files} onPick={jumpTo} />}
              <DiffViewer diffText={diffText} viewType={viewType} />
            </>
          )}
        </div>
      </aside>
    </>
  );
}

interface FileEntry {
  path: string;
  add: number;
  del: number;
}

/** Sticky "jump to file" combobox: type to filter, arrows + Enter (or a click) to pick. */
function FileSelect({ files, onPick }: { files: FileEntry[]; onPick: (path: string) => void }) {
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(0);
  const listRef = useRef<HTMLUListElement>(null);

  const matches = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? files.filter((f) => f.path.toLowerCase().includes(q)) : files;
  }, [files, query]);

  useEffect(() => {
    listRef.current?.children[cursor]?.scrollIntoView({ block: "nearest" });
  }, [cursor, open]);

  const pick = (path: string) => {
    onPick(path);
    setQuery("");
    setOpen(false);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setOpen(true);
      const step = e.key === "ArrowDown" ? 1 : -1;
      setCursor((c) => (matches.length ? (c + step + matches.length) % matches.length : 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (matches[cursor]) pick(matches[cursor].path);
    } else if (e.key === "Escape" && (open || query)) {
      // Closes the list first; a second Esc closes the pane.
      e.preventDefault();
      setQuery("");
      setOpen(false);
    }
  };

  return (
    <div className="quick-view-filebar">
      <div className="quick-view-select">
        <input
          className="input"
          role="combobox"
          aria-expanded={open}
          aria-controls="quick-view-file-list"
          placeholder={`Jump to file… (${files.length} changed)`}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setCursor(0);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          // The input keeps focus after a pick or Esc, so a click must reopen the list too.
          onClick={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={onKeyDown}
        />
        {open && (
          <ul className="quick-view-options" id="quick-view-file-list" role="listbox" ref={listRef}>
            {matches.length === 0 && <li className="muted quick-view-empty">No matching file</li>}
            {matches.map((f, i) => (
              <li
                key={f.path}
                role="option"
                aria-selected={i === cursor}
                className={i === cursor ? "active" : ""}
                onMouseEnter={() => setCursor(i)}
                // mousedown, not click: the input's blur would close the list first.
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(f.path);
                }}
              >
                <span className="quick-view-option-path">{f.path}</span>
                <span className="add">+{f.add}</span>
                <span className="del">−{f.del}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
