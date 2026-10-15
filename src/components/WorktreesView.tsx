import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { timeAgo } from "../lib/timeAgo";
import { useSettingsStore } from "../lib/settings";
import type { WorktreeGcAction, WorktreeGcReport, WorktreeRow, WorktreeVerdict } from "../lib/types";
import { Icon } from "./icons";

const VERDICT_LABEL: Record<WorktreeVerdict["kind"], string> = {
  remove: "safe to remove",
  prune: "folder gone",
  fast_forward: "behind",
  resync: "rewritten upstream",
  flag: "needs a look",
  keep: "keep",
};

const VERDICT_ORDER: WorktreeVerdict["kind"][] = ["flag", "remove", "prune", "fast_forward", "resync", "keep"];

const WORKTREE_GC_KEY = ["worktree-gc"] as const;

function summarizeActions(actions: WorktreeGcAction[]): string {
  const ok = actions.filter((a) => a.ok);
  const failed = actions.length - ok.length;
  const removed = ok.filter((a) => a.action.startsWith("remove") || a.action === "prune").length;
  const synced = ok.length - removed;
  const parts = [removed && `removed ${removed}`, synced && `synced ${synced}`, failed && `${failed} failed`].filter(Boolean);
  return parts.length ? `Worktrees: ${parts.join(", ")}` : "Nothing to clean up";
}

function updateRow(
  report: WorktreeGcReport | undefined,
  path: string,
  f: (row: WorktreeRow) => WorktreeRow | null,
): WorktreeGcReport | undefined {
  if (!report) return report;
  return {
    ...report,
    repos: report.repos.map((r) => ({
      ...r,
      worktrees: r.worktrees.flatMap((w) => (w.path === path ? (f(w) ?? []) : [w])),
    })),
  };
}

function aheadBehind(row: WorktreeRow): string | null {
  const u = row.upstream;
  if (u.kind === "gone") return "upstream gone";
  if (u.kind !== "tracking" || (u.ahead === 0 && u.behind === 0)) return null;
  return [u.ahead && `↑${u.ahead}`, u.behind && `↓${u.behind}`].filter(Boolean).join(" ");
}

export function WorktreesView() {
  const [showKept, setShowKept] = useState(true);
  const queryClient = useQueryClient();
  const reportQuery = useQuery({
    queryKey: WORKTREE_GC_KEY,
    queryFn: api.worktreeGcLast,
    staleTime: Infinity,
  });

  const scan = useMutation({
    mutationFn: (apply: boolean) => api.worktreeGcRun(apply, apply),
    onSuccess: (report, apply) => {
      queryClient.setQueryData(WORKTREE_GC_KEY, report);
      if (apply) toast.success(summarizeActions(report.actions));
    },
    onError: (e) => toast.error(`Worktree scan failed:\n${String(e)}`),
  });

  const report = reportQuery.data ?? null;
  const rescan = () => scan.mutate(false);
  const patchRow = (path: string, f: (row: WorktreeRow) => WorktreeRow | null) =>
    queryClient.setQueryData<WorktreeGcReport>(WORKTREE_GC_KEY, (r) => updateRow(r, path, f));

  const remove = useMutation({
    mutationFn: ({ repoId, path }: { repoId: number; path: string }) => api.removeWorktree(repoId, path),
    onSuccess: (_d, { path }) => {
      toast.success("Worktree removed");
      patchRow(path, () => null);
    },
    onError: (e) => toast.error(`Remove failed:\n${String(e)}`),
  });
  const sync = useMutation({
    mutationFn: ({ repoId, path }: { repoId: number; path: string }) => api.worktreeSync(repoId, path),
    onSuccess: (msg, { path }) => {
      toast.success(msg);
      patchRow(path, (w) => ({ ...w, verdict: { kind: "keep", reason: msg } }));
    },
    onError: (e) => toast.error(`Sync failed:\n${String(e)}`),
  });

  const defaultTerminal = useSettingsStore((s) => s.defaultTerminal);
  const tmuxTarget = useSettingsStore((s) => s.tmuxTarget);
  const tmuxSessionName = useSettingsStore((s) => s.tmuxSessionName);
  const openTerminal = useMutation({
    mutationFn: ({ repoId, path }: { repoId: number; path: string }) =>
      api.worktreeOpenTerminal(repoId, path, defaultTerminal, tmuxTarget, tmuxSessionName),
    onError: (e) => toast.error(`Could not open terminal:\n${String(e)}`),
  });

  const counts = useMemo(() => countVerdicts(report), [report]);
  const actionable = counts.remove + counts.prune + counts.fast_forward + counts.resync;

  const runCleanup = async () => {
    const ok = await confirmDialog({
      title: "Run worktree cleanup",
      message: `Remove ${counts.remove + counts.prune} safe worktree(s) and sync ${counts.fast_forward + counts.resync}. Flagged and kept worktrees are left alone.`,
      confirmLabel: "Clean up",
    });
    if (ok) scan.mutate(true);
  };

  const confirmRemove = async (repoId: number, row: WorktreeRow) => {
    const ok = await confirmDialog({
      title: "Remove worktree",
      message: `Remove ${row.display_path}? Git refuses if it has uncommitted or untracked changes. The branch is kept.`,
      confirmLabel: "Remove",
      danger: true,
    });
    if (ok) remove.mutate({ repoId, path: row.path });
  };

  const repos = (report?.repos ?? [])
    .map((r) => ({
      ...r,
      worktrees: r.worktrees
        .filter((w) => !w.is_main && (showKept || w.verdict.kind !== "keep"))
        .sort((a, b) => VERDICT_ORDER.indexOf(a.verdict.kind) - VERDICT_ORDER.indexOf(b.verdict.kind)),
    }))
    .filter((r) => r.worktrees.length > 0 || r.error);

  return (
    <section className="cr-main">
      <header className="cr-pagehead">
        <div>
          <h1 className="cr-h1">Worktrees</h1>
          <p className="cr-sub">
            {report
              ? `${counts.total} worktrees · ${counts.flag} need a look · ${actionable} actionable · scanned ${timeAgo(report.ran_at)}`
              : scan.isPending
                ? "Scanning all repositories…"
                : "Not scanned yet — press Scan to check every repository's worktrees"}
          </p>
        </div>
        <div className="worktrees-head-actions">
          <label className="worktrees-toggle">
            <input type="checkbox" checked={showKept} onChange={(e) => setShowKept(e.target.checked)} /> Show kept
          </label>
          <button className="btn btn-sm" disabled={scan.isPending} onClick={rescan}>
            <Icon name="refresh" size={13} /> {scan.isPending ? "Scanning…" : "Scan"}
          </button>
          <button className="btn btn-sm btn-primary" disabled={scan.isPending || actionable === 0} onClick={runCleanup}>
            <Icon name="trash" size={13} /> Clean up
          </button>
        </div>
      </header>

      <div className="cr-list worktrees-list">
        {report && report.actions.length > 0 && (
          <div className="card worktrees-actions-log">
            <h2 className="sessions-group-head">Last cleanup</h2>
            {report.actions.map((a) => (
              <div key={`${a.path}-${a.action}`} className={a.ok ? "muted" : "worktrees-action-failed"}>
                {a.ok ? "✓" : "✗"} {a.action} {a.path}
                {a.message && ` — ${a.message}`}
              </div>
            ))}
          </div>
        )}
        {repos.map((repo) => (
          <div key={repo.repo_id} className="worktrees-repo">
            <h2 className="sessions-group-head">{repo.slug ?? repo.path}</h2>
            {repo.error && <p className="muted">{repo.error}</p>}
            {repo.worktrees.map((w) => (
              <div key={w.path} className="card sessions-row worktrees-row">
                <span className={`worktrees-verdict worktrees-verdict-${w.verdict.kind}`}>
                  {VERDICT_LABEL[w.verdict.kind]}
                </span>
                <div className="sessions-main">
                  <div className="sessions-title">
                    <span className="sessions-name" title={w.path}>
                      {w.display_path}
                    </span>
                    {w.pr && (
                      <span className={`badge ${w.pr.state === "OPEN" ? "badge-open" : "badge-resolved"}`}>
                        #{w.pr.number} {w.pr.state.toLowerCase()}
                      </span>
                    )}
                  </div>
                  <div className="sessions-meta">
                    <span className="chip">
                      <Icon name="branch" size={11} /> {w.branch ?? `detached ${w.head_sha.slice(0, 8)}`}
                    </span>
                    {aheadBehind(w) && <span className="chip">{aheadBehind(w)}</span>}
                    <span className="chip">{w.source}</span>
                    {w.last_commit_at && <span className="chip">{timeAgo(w.last_commit_at)}</span>}
                    <span className="worktrees-reason">{w.verdict.reason}</span>
                  </div>
                </div>
                <div className="sessions-actions">
                  {(w.verdict.kind === "fast_forward" || w.verdict.kind === "resync") && (
                    <button
                      className="btn btn-sm"
                      disabled={sync.isPending}
                      onClick={() => sync.mutate({ repoId: repo.repo_id, path: w.path })}
                    >
                      Sync
                    </button>
                  )}
                  {w.verdict.kind !== "prune" && (
                    <button
                      className="btn btn-sm btn-ghost"
                      title="Open a terminal here"
                      disabled={openTerminal.isPending}
                      onClick={() => openTerminal.mutate({ repoId: repo.repo_id, path: w.path })}
                    >
                      <Icon name="terminal" size={13} />
                    </button>
                  )}
                  {w.verdict.kind !== "prune" && (
                    <button className="btn btn-sm btn-ghost" title="Open in VSCode" onClick={() => api.openInVscode(w.path)}>
                      <Icon name="code" size={13} />
                    </button>
                  )}
                  {w.verdict.kind !== "prune" && (
                    <button
                      className="btn btn-sm btn-ghost"
                      title="Remove worktree"
                      disabled={remove.isPending}
                      onClick={() => confirmRemove(repo.repo_id, w)}
                    >
                      <Icon name="trash" size={13} />
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        ))}
      </div>
    </section>
  );
}

function countVerdicts(report: WorktreeGcReport | null) {
  const c = { total: 0, remove: 0, prune: 0, fast_forward: 0, resync: 0, flag: 0, keep: 0 };
  for (const repo of report?.repos ?? []) {
    for (const w of repo.worktrees) {
      if (w.is_main) continue;
      c.total += 1;
      c[w.verdict.kind] += 1;
    }
  }
  return c;
}
