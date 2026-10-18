import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { confirmDialog } from "../../lib/confirm";
import { useSettingsStore } from "../../lib/settings";
import { toast } from "../../lib/toast";
import type { RefInfo, Repository, WipKind, WorktreeInfo } from "../../lib/types";

const INVALIDATED_KEYS = ["graph", "refs", "wip", "worktrees", "branches", "stashes"];

export interface Refusal {
  kind: string;
  message: string;
}

/** The backend tags recognised git refusals as `git error: [kind] message` (see `graph/ops.rs`). */
export function parseRefusal(err: unknown): Refusal | null {
  const m = /^git error: \[(\w+)\] ([\s\S]*)$/.exec(String(err));
  return m ? { kind: m[1], message: m[2] } : null;
}

export function worktreeFromRefusal(message: string): string | null {
  const m = /(?:used by worktree at|already checked out at) '([^']+)'/.exec(message);
  return m ? m[1] : null;
}

/** `true`: handled and succeeded, `false`: the user declined, `null`: not handled. */
type Escalation = (r: Refusal) => Promise<boolean | null>;

export interface FormConfig {
  title: string;
  submitLabel: string;
  name?: { label: string; initial?: string; placeholder?: string };
  message?: { label: string; placeholder?: string };
  checks?: { key: string; label: string; initial: boolean }[];
  onSubmit: (values: { name: string; message: string; checks: Record<string, boolean> }) => Promise<boolean>;
}

export interface GitOps {
  repo: Repository;
  ctx: string | null;
  refs: RefInfo[];
  worktrees: WorktreeInfo[];
  busy: boolean;
  /** Path of the worktree that has `branch` checked out, if any. */
  checkedOutPath: (branch: string) => string | null;
  checkout: (ref: RefInfo) => void;
  checkoutCommit: (sha: string) => void;
  createBranchAt: (sha: string, label: string) => void;
  createTagAt: (sha: string, label: string) => void;
  renameBranch: (name: string) => void;
  deleteBranch: (name: string) => void;
  deleteTag: (name: string) => void;
  pull: () => void;
  push: () => void;
  stage: (paths: string[]) => void;
  unstage: (paths: string[]) => void;
  discard: (paths: string[], kind: WipKind) => void;
  commit: (message: string, amend: boolean) => Promise<boolean>;
  stash: () => void;
  stashApply: (index: number) => void;
  stashPop: (index: number) => void;
  stashDrop: (index: number) => void;
  openTerminal: (path: string) => void;
  openClaude: (path: string) => void;
}

const GitOpsContext = createContext<GitOps | null>(null);

/** `null` outside the Graph tab (or without a local clone): callers then render read-only. */
export function useGitOps(): GitOps | null {
  return useContext(GitOpsContext);
}

export function GitOpsProvider({
  repo,
  ctx,
  refs,
  worktrees,
  onSwitchContext,
  children,
}: {
  repo: Repository;
  ctx: string | null;
  refs: RefInfo[];
  worktrees: WorktreeInfo[];
  onSwitchContext: (path: string) => void;
  children: ReactNode;
}) {
  const queryClient = useQueryClient();
  const [pending, setPending] = useState(0);
  const [form, setForm] = useState<FormConfig | null>(null);
  const defaultTerminal = useSettingsStore((s) => s.defaultTerminal);
  const tmuxTarget = useSettingsStore((s) => s.tmuxTarget);
  const tmuxSessionName = useSettingsStore((s) => s.tmuxSessionName);

  const invalidate = useCallback(() => {
    for (const key of INVALIDATED_KEYS) queryClient.invalidateQueries({ queryKey: [key, repo.id] });
  }, [queryClient, repo.id]);

  const attempt = useCallback(
    async (label: string, fn: () => Promise<unknown>, escalate?: Escalation): Promise<boolean> => {
      setPending((n) => n + 1);
      try {
        await fn();
        return true;
      } catch (e) {
        const refusal = parseRefusal(e);
        if (refusal && escalate) {
          const handled = await escalate(refusal).catch((e2) => {
            toast.error(`${label} failed:\n${parseRefusal(e2)?.message ?? String(e2)}`);
            return false;
          });
          if (handled !== null) return handled;
        }
        toast.error(`${label} failed:\n${refusal?.message ?? String(e)}`);
        return false;
      } finally {
        setPending((n) => n - 1);
        invalidate();
      }
    },
    [invalidate],
  );

  const offerWorktreeSwitch: Escalation = useCallback(
    async (r) => {
      if (r.kind !== "checked_out_elsewhere") return null;
      const path = worktreeFromRefusal(r.message);
      const target = path ? worktrees.find((w) => w.path === path) : undefined;
      if (!target) return null;
      const ok = await confirmDialog({
        title: "Branch is checked out elsewhere",
        message: `${r.message}\n\nSwitch the graph to that worktree instead?`,
        confirmLabel: "Switch worktree",
      });
      if (ok) onSwitchContext(target.path);
      return ok ? true : false;
    },
    [worktrees, onSwitchContext],
  );

  const ops = useMemo<GitOps>(() => {
    const id = repo.id;
    const checkedOutPath = (branch: string) => worktrees.find((w) => w.branch === branch)?.path ?? null;

    const openForm = (cfg: FormConfig) => setForm(cfg);

    return {
      repo,
      ctx,
      refs,
      worktrees,
      busy: pending > 0,
      checkedOutPath,
      checkout: (ref) => {
        if (ref.kind === "tag") return ops.checkoutCommit(ref.name);
        void attempt(
          "Checkout",
          () =>
            ref.kind === "remote"
              ? api.gitCheckoutRemoteBranch(id, ctx, ref.name)
              : api.gitCheckoutBranch(id, ctx, ref.name),
          offerWorktreeSwitch,
        );
      },
      checkoutCommit: (rev) => {
        void (async () => {
          const ok = await confirmDialog({
            title: "Checkout detached HEAD",
            message: `HEAD will point at ${rev.slice(0, 12)} instead of a branch. Commits made there are easy to lose; create a branch to keep them.`,
            confirmLabel: "Checkout",
          });
          if (ok) await attempt("Checkout", () => api.gitCheckoutDetached(id, ctx, rev));
        })();
      },
      createBranchAt: (sha, label) =>
        openForm({
          title: `Create branch at ${label}`,
          submitLabel: "Create branch",
          name: { label: "name", placeholder: "feature/my-branch" },
          checks: [{ key: "switch", label: "Check out after create", initial: true }],
          onSubmit: (v) =>
            attempt(
              "Create branch",
              () => api.gitCreateBranch(id, ctx, v.name.trim(), sha, v.checks.switch),
              offerWorktreeSwitch,
            ),
        }),
      createTagAt: (sha, label) =>
        openForm({
          title: `Create tag at ${label}`,
          submitLabel: "Create tag",
          name: { label: "name", placeholder: "v1.0.0" },
          message: { label: "message (annotated tag if set)" },
          checks: [{ key: "push", label: "Push to origin", initial: false }],
          onSubmit: (v) =>
            attempt("Create tag", () =>
              api.gitCreateTag(id, ctx, v.name.trim(), sha, v.message.trim() || null, v.checks.push),
            ),
        }),
      renameBranch: (name) =>
        openForm({
          title: `Rename ${name}`,
          submitLabel: "Rename",
          name: { label: "new name", initial: name },
          onSubmit: (v) => attempt("Rename", () => api.gitRenameBranch(id, ctx, name, v.name.trim())),
        }),
      deleteBranch: (name) => {
        void (async () => {
          const ok = await confirmDialog({
            title: "Delete branch",
            message: `Delete local branch ${name}? Git refuses if it has unmerged commits.`,
            confirmLabel: "Delete",
            danger: true,
          });
          if (!ok) return;
          await attempt("Delete branch", () => api.gitDeleteBranch(id, ctx, name, false), async (r) => {
            if (r.kind !== "not_fully_merged") return null;
            const force = await confirmDialog({
              title: "Branch is not fully merged",
              message: `${r.message}\n\nForce delete ${name} and lose those commits?`,
              confirmLabel: "Force delete",
              danger: true,
            });
            if (!force) return false;
            await api.gitDeleteBranch(id, ctx, name, true);
            return true;
          });
        })();
      },
      deleteTag: (name) => {
        void (async () => {
          const ok = await confirmDialog({
            title: "Delete tag",
            message: `Delete local tag ${name}? Remote copies are not touched.`,
            confirmLabel: "Delete",
            danger: true,
          });
          if (ok) await attempt("Delete tag", () => api.gitDeleteTag(id, ctx, name));
        })();
      },
      pull: () => {
        void attempt("Pull", async () => {
          const out = await api.gitPull(id, ctx);
          toast.success(out.trim().split("\n").pop() || "Pulled.");
        });
      },
      push: () => {
        void attempt(
          "Push",
          async () => {
            await api.gitPush(id, ctx, false);
            toast.success("Pushed.");
          },
          async (r) => {
            if (r.kind !== "push_rejected") return null;
            const ok = await confirmDialog({
              title: "Push rejected",
              message: `${r.message}\n\nForce push with --force-with-lease? This overwrites remote commits you have not fetched.`,
              confirmLabel: "Force push",
              danger: true,
            });
            if (!ok) return false;
            await api.gitPush(id, ctx, true);
            toast.success("Force pushed.");
            return true;
          },
        );
      },
      stage: (paths) => void attempt("Stage", () => api.gitStage(id, ctx, paths)),
      unstage: (paths) => void attempt("Unstage", () => api.gitUnstage(id, ctx, paths)),
      discard: (paths, kind) => {
        void (async () => {
          const ok = await confirmDialog({
            title: "Discard changes",
            message:
              paths.length === 1
                ? `Discard ${kind === "untracked" ? "untracked file" : "changes to"} ${paths[0]}? This cannot be undone.`
                : `Discard ${paths.length} files? This cannot be undone.`,
            confirmLabel: "Discard",
            danger: true,
          });
          if (ok) await attempt("Discard", () => api.gitDiscard(id, ctx, paths, kind));
        })();
      },
      commit: (message, amend) => attempt("Commit", () => api.gitCommit(id, ctx, message, amend)),
      stash: () =>
        openForm({
          title: "Stash changes",
          submitLabel: "Stash",
          message: { label: "message (optional)" },
          checks: [{ key: "untracked", label: "Include untracked files", initial: false }],
          onSubmit: (v) =>
            attempt("Stash", () => api.gitStashPush(id, ctx, v.message.trim() || null, v.checks.untracked)),
        }),
      stashApply: (index) => void attempt("Apply stash", () => api.gitStashApply(id, ctx, index)),
      stashPop: (index) => void attempt("Pop stash", () => api.gitStashPop(id, ctx, index)),
      stashDrop: (index) => {
        void (async () => {
          const ok = await confirmDialog({
            title: "Drop stash",
            message: `Drop stash@{${index}}? Its changes are lost.`,
            confirmLabel: "Drop",
            danger: true,
          });
          if (ok) await attempt("Drop stash", () => api.gitStashDrop(id, ctx, index));
        })();
      },
      openTerminal: (path) =>
        void attempt("Open terminal", () =>
          api.worktreeOpenTerminal(id, path, defaultTerminal, tmuxTarget, tmuxSessionName),
        ),
      openClaude: (path) =>
        void attempt("Open Claude", () =>
          api.checkoutOpenClaude(id, path, defaultTerminal, tmuxTarget, tmuxSessionName),
        ),
    };
  }, [
    repo,
    ctx,
    refs,
    worktrees,
    pending,
    attempt,
    offerWorktreeSwitch,
    defaultTerminal,
    tmuxTarget,
    tmuxSessionName,
  ]);

  return (
    <GitOpsContext.Provider value={ops}>
      {children}
      {form && <FormDialog config={form} onClose={() => setForm(null)} />}
    </GitOpsContext.Provider>
  );
}

function FormDialog({ config, onClose }: { config: FormConfig; onClose: () => void }) {
  const [name, setName] = useState(config.name?.initial ?? "");
  const [message, setMessage] = useState("");
  const [checks, setChecks] = useState<Record<string, boolean>>(() =>
    Object.fromEntries((config.checks ?? []).map((c) => [c.key, c.initial])),
  );
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const canSubmit = !submitting && (!config.name || name.trim() !== "");
  const submit = async () => {
    if (!canSubmit) return;
    setSubmitting(true);
    const ok = await config.onSubmit({ name, message, checks });
    if (ok) onClose();
    else setSubmitting(false);
  };

  return createPortal(
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal rvd-modal"
        role="dialog"
        aria-modal="true"
        aria-label={config.title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-header">
          <h3>{config.title}</h3>
        </div>
        <form
          className="rvd-body"
          onSubmit={(e) => {
            e.preventDefault();
            void submit();
          }}
        >
          {config.name && (
            <label className="rvd-field">
              {config.name.label}
              <input
                className="input mono"
                autoFocus
                aria-label={config.name.label}
                placeholder={config.name.placeholder}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </label>
          )}
          {config.message && (
            <label className="rvd-field">
              {config.message.label}
              <textarea
                className="input"
                rows={3}
                aria-label={config.message.label}
                autoFocus={!config.name}
                placeholder={config.message.placeholder}
                value={message}
                onChange={(e) => setMessage(e.target.value)}
              />
            </label>
          )}
          {(config.checks ?? []).map((c) => (
            <label key={c.key} className="check">
              <input
                type="checkbox"
                checked={checks[c.key]}
                onChange={(e) => setChecks((prev) => ({ ...prev, [c.key]: e.target.checked }))}
              />
              {c.label}
            </label>
          ))}
          <div className="modal-actions">
            <button type="button" className="btn" onClick={onClose}>
              Cancel
            </button>
            <button type="submit" className="btn btn-primary" disabled={!canSubmit}>
              {submitting ? "Working…" : config.submitLabel}
            </button>
          </div>
        </form>
      </div>
    </div>,
    document.body,
  );
}
