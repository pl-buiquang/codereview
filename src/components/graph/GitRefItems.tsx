import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import type { RefInfo } from "../../lib/types";
import { useGitOps } from "./GitOps";
import type { MenuAnchor } from "./BranchActionsMenu";
import "./RepoSidebar.css";

export type GitTarget = { kind: "ref"; ref: RefInfo } | { kind: "commit"; sha: string };

const NOT_CHECKED_OUT = "This branch is not checked out in any worktree";

/** Git mutation entries for a ref or commit menu; renders nothing without a local clone (no `GitOps`). */
export function GitRefItems({ target, close }: { target: GitTarget; close: () => void }) {
  const ops = useGitOps();
  const checkedOutPath = target.kind === "ref" && target.ref.kind === "local" ? ops?.checkedOutPath(target.ref.name) : null;

  const sessionsQuery = useQuery({
    queryKey: ["claude-sessions-live"],
    queryFn: () => api.listClaudeSessions(0, ""),
    enabled: checkedOutPath != null,
    staleTime: 5_000,
  });
  if (!ops) return null;

  const item = (label: React.ReactNode, run: () => void, opts?: { disabled?: boolean; title?: string }) => (
    <button
      className="rs-menu-item"
      role="menuitem"
      disabled={opts?.disabled}
      title={opts?.title}
      onClick={() => {
        close();
        run();
      }}
    >
      {label}
    </button>
  );

  if (target.kind === "commit") {
    const label = target.sha.slice(0, 7);
    return (
      <>
        {item("Checkout (detached)", () => ops.checkoutCommit(target.sha))}
        {item("Create branch here…", () => ops.createBranchAt(target.sha, label))}
        {item("Create tag here…", () => ops.createTagAt(target.sha, label))}
      </>
    );
  }

  const ref = target.ref;
  if (ref.kind === "tag") {
    return (
      <>
        {item("Checkout (detached)", () => ops.checkout(ref))}
        {item("Create branch here…", () => ops.createBranchAt(ref.sha, ref.name))}
        {item("Delete tag", () => ops.deleteTag(ref.name))}
      </>
    );
  }
  if (ref.kind === "remote") {
    return (
      <>
        {item("Checkout as local branch", () => ops.checkout(ref))}
        {item("Create branch here…", () => ops.createBranchAt(ref.sha, ref.name))}
        {item("Create tag here…", () => ops.createTagAt(ref.sha, ref.name))}
      </>
    );
  }

  const live = (sessionsQuery.data?.sessions ?? []).filter((s) => s.live && s.cwd === checkedOutPath);
  return (
    <>
      <div className="rs-menu-sep" role="separator" />
      {item("Checkout", () => ops.checkout(ref), { disabled: ref.is_head, title: ref.is_head ? "Already checked out" : undefined })}
      {item("Create branch here…", () => ops.createBranchAt(ref.sha, ref.name))}
      {item("Create tag here…", () => ops.createTagAt(ref.sha, ref.name))}
      {item("Rename…", () => ops.renameBranch(ref.name))}
      {item("Delete", () => ops.deleteBranch(ref.name), {
        disabled: ref.is_head,
        title: ref.is_head ? "Cannot delete the checked-out branch" : undefined,
      })}
      <div className="rs-menu-sep" role="separator" />
      {item("Open terminal", () => checkedOutPath && ops.openTerminal(checkedOutPath), {
        disabled: !checkedOutPath,
        title: checkedOutPath ? undefined : NOT_CHECKED_OUT,
      })}
      {item("New Claude session", () => checkedOutPath && ops.openClaude(checkedOutPath), {
        disabled: !checkedOutPath,
        title: checkedOutPath ? undefined : NOT_CHECKED_OUT,
      })}
      {live.map((s) =>
        item(
          <>
            Focus Claude <span className="mono">{s.name ?? s.session_id.slice(0, 8)}</span>
          </>,
          () => api.focusClaudeSession(s.session_id).catch((e) => toast.error(`Could not focus session: ${String(e)}`)),
        ),
      )}
    </>
  );
}

export function CommitActionsMenu({
  sha,
  anchor,
  onClose,
}: {
  sha: string;
  anchor: MenuAnchor;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return createPortal(
    <div
      className="rs-menu"
      ref={ref}
      role="menu"
      aria-label={`Actions for ${sha.slice(0, 7)}`}
      style={{ top: anchor.y, left: anchor.x }}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="rs-menu-title mono">{sha.slice(0, 7)}</div>
      <GitRefItems target={{ kind: "commit", sha }} close={onClose} />
    </div>,
    document.body,
  );
}
