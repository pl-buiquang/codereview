import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { useSettingsStore, type DiffViewType } from "../../lib/settings";
import type { ChangedFile, WipKind } from "../../lib/types";
import { DiffViewer } from "../DiffViewer";
import { Icon } from "../icons";
import "./CommitPanel.css";

export type DiffPreviewTarget =
  | { kind: "commit"; repoId: number; sha: string; file: ChangedFile }
  | {
      kind: "wip";
      repoId: number;
      worktreePath: string | null;
      path: string;
      wipKind: WipKind;
    };

export function DiffPreview({
  target,
  files,
  index,
  onNavigate,
  onClose,
}: {
  target: DiffPreviewTarget;
  /** Display paths of the list being cycled; only its length and order matter for prev/next. */
  files: string[];
  index: number;
  onNavigate: (index: number) => void;
  onClose: () => void;
}) {
  const defaultViewType = useSettingsStore((s) => s.defaultViewType);
  const [viewType, setViewType] = useState<DiffViewType>(defaultViewType);

  const query = useQuery({
    queryKey:
      target.kind === "commit"
        ? ["commitFileDiff", target.repoId, target.sha, target.file.path, target.file.old_path]
        : ["worktreeFileDiff", target.repoId, target.worktreePath, target.path, target.wipKind],
    queryFn: () =>
      target.kind === "commit"
        ? api.commitFileDiff({
            repoId: target.repoId,
            sha: target.sha,
            path: target.file.path,
            oldPath: target.file.old_path,
          })
        : api.worktreeFileDiff({
            repoId: target.repoId,
            worktreePath: target.worktreePath,
            path: target.path,
            kind: target.wipKind,
          }),
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const count = files.length;
  const step = (delta: number) => {
    if (count === 0) return;
    onNavigate((((index + delta) % count) + count) % count);
  };

  const title =
    target.kind === "commit"
      ? target.file.old_path
        ? `${target.file.old_path} → ${target.file.path}`
        : target.file.path
      : target.path;

  return (
    <div className="diff-preview">
      <div className="dp-header">
        <span className="dp-path" title={title}>
          {title}
        </span>
        {target.kind === "wip" && <span className="dp-kind">{target.wipKind}</span>}
        <div className="view-toggle">
          <button className={viewType === "split" ? "active" : ""} onClick={() => setViewType("split")}>
            Split
          </button>
          <button
            className={viewType === "unified" ? "active" : ""}
            onClick={() => setViewType("unified")}
          >
            Unified
          </button>
        </div>
        <div className="dp-nav">
          <button
            type="button"
            className="btn"
            disabled={count < 2}
            onClick={() => step(-1)}
            aria-label="Previous file"
          >
            ‹
          </button>
          {count > 0 && (
            <span className="dp-pos">
              {index + 1} / {count}
            </span>
          )}
          <button
            type="button"
            className="btn"
            disabled={count < 2}
            onClick={() => step(1)}
            aria-label="Next file"
          >
            ›
          </button>
        </div>
        <button type="button" className="dp-close" onClick={onClose} aria-label="Close preview">
          <Icon name="x" />
        </button>
      </div>
      <div className="dp-body">
        {query.isLoading && <p className="muted">Loading diff…</p>}
        {query.isError && <p className="error">Diff failed: {String(query.error)}</p>}
        {query.data != null && <DiffViewer diffText={query.data} viewType={viewType} />}
      </div>
    </div>
  );
}
