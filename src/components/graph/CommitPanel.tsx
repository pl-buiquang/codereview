import type { ReactNode } from "react";
import type { ChangeStatus, ChangedFile, CommitDetail, PrSummary, WipKind, WipStatus } from "../../lib/types";
import { timeAgo } from "../../lib/timeAgo";
import { toast } from "../../lib/toast";
import { Icon } from "../icons";
import "./CommitPanel.css";

const STATUS_LABEL: Record<ChangeStatus, string> = {
  A: "Added",
  M: "Modified",
  D: "Deleted",
  R: "Renamed",
  C: "Copied",
  T: "Type changed",
};

export const shortSha = (sha: string) => sha.slice(0, 7);

export function StatusBadge({ status }: { status: ChangeStatus }) {
  return (
    <span className={`cp-status cp-status-${status}`} title={STATUS_LABEL[status]}>
      {status}
    </span>
  );
}

function formatTime(epochSecs: number): string {
  const ms = epochSecs * 1000;
  return `${new Date(ms).toLocaleString()} (${timeAgo(ms)})`;
}

function Person({
  role,
  name,
  email,
  time,
}: {
  role: string;
  name: string;
  email: string;
  time: number;
}) {
  return (
    <div className="cp-person">
      <span className="cp-person-role">{role}</span>
      <span className="cp-person-name">{name}</span>
      <span className="cp-person-email">&lt;{email}&gt;</span>
      <span className="cp-person-date">{formatTime(time)}</span>
    </div>
  );
}

function FileRow({
  path,
  oldPath,
  status,
  additions,
  deletions,
  active,
  onClick,
}: {
  path: string;
  oldPath?: string | null;
  status: ChangeStatus;
  additions?: number | null;
  deletions?: number | null;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        className={`cp-file${active ? " active" : ""}`}
        onClick={onClick}
        title={oldPath ? `${oldPath} → ${path}` : path}
      >
        <StatusBadge status={status} />
        <span className="cp-file-path">{oldPath ? `${oldPath} → ${path}` : path}</span>
        {(additions != null || deletions != null) && (
          <span className="cp-numstat">
            {additions != null && <span className="add">+{additions}</span>}
            {deletions != null && <span className="del">−{deletions}</span>}
          </span>
        )}
      </button>
    </li>
  );
}

function countStatuses(files: ChangedFile[]): Partial<Record<ChangeStatus, number>> {
  const counts: Partial<Record<ChangeStatus, number>> = {};
  for (const f of files) counts[f.status] = (counts[f.status] ?? 0) + 1;
  return counts;
}

function CommitMode({
  detail,
  onOpenFile,
  onJumpToSha,
  onOpenPr,
  pullRequests = [],
  actions,
  activePath,
}: {
  detail: CommitDetail;
  onOpenFile: (file: ChangedFile) => void;
  onJumpToSha: (sha: string) => void;
  onOpenPr?: (pr: PrSummary) => void;
  pullRequests?: PrSummary[];
  actions?: ReactNode;
  activePath?: string | null;
}) {
  const counts = countStatuses(detail.files);
  const copySha = async () => {
    try {
      await navigator.clipboard.writeText(detail.sha);
      toast.success(`Copied ${shortSha(detail.sha)}`);
    } catch (e) {
      toast.error(`Copy failed: ${String(e)}`);
    }
  };

  return (
    <div className="commit-panel">
      <div className="cp-header">
        <h3 className="cp-subject">{detail.subject}</h3>
        {detail.body.trim() && <pre className="cp-body">{detail.body.trim()}</pre>}
        {actions}
      </div>

      <div className="cp-meta">
        {pullRequests.length > 0 && (
          <div className="cp-prs">
            {pullRequests.map((pr) => (
              <button
                key={pr.number}
                type="button"
                className="cp-pr"
                onClick={() => onOpenPr?.(pr)}
                title={`Open ${pr.url}`}
              >
                <span className="badge badge-pr">PR #{pr.number}</span>
                <span className="cp-pr-title">{pr.title}</span>
                <Icon name="ext" size={11} />
              </button>
            ))}
          </div>
        )}
        <div className="cp-shas">
          <span className="cp-label">commit</span>
          <button
            type="button"
            className="cp-sha"
            onClick={copySha}
            title={`Copy ${detail.sha}`}
            aria-label={`Copy sha ${detail.sha}`}
          >
            {shortSha(detail.sha)}
            <Icon name="copy" size={12} />
          </button>
        </div>
        <div className="cp-shas">
          <span className="cp-label">{detail.parents.length === 1 ? "parent" : "parents"}</span>
          {detail.parents.length === 0 && <span className="cp-muted">none (root commit)</span>}
          {detail.parents.map((p) => (
            <button
              key={p}
              type="button"
              className="cp-sha cp-parent"
              onClick={() => onJumpToSha(p)}
              title={`Jump to ${p}`}
            >
              {shortSha(p)}
            </button>
          ))}
        </div>
        <Person
          role="author"
          name={detail.author_name}
          email={detail.author_email}
          time={detail.author_time}
        />
        <Person
          role="committer"
          name={detail.committer_name}
          email={detail.committer_email}
          time={detail.committer_time}
        />
      </div>

      <div className="cp-files-head">
        <span>
          {detail.files.length} file{detail.files.length === 1 ? "" : "s"} changed
        </span>
        <span className="cp-counts" data-testid="cp-counts">
          {(Object.keys(STATUS_LABEL) as ChangeStatus[])
            .filter((s) => counts[s])
            .map((s) => (
              <span key={s} className={`cp-count cp-status-${s}`} title={STATUS_LABEL[s]}>
                {s} {counts[s]}
              </span>
            ))}
        </span>
      </div>
      {detail.files.length === 0 ? (
        <p className="cp-muted cp-empty">No file changes.</p>
      ) : (
        <ul className="cp-file-list">
          {detail.files.map((f) => (
            <FileRow
              key={`${f.old_path ?? ""}:${f.path}`}
              path={f.path}
              oldPath={f.old_path}
              status={f.status}
              additions={f.additions}
              deletions={f.deletions}
              active={activePath === f.path}
              onClick={() => onOpenFile(f)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function WipGroup({
  title,
  kind,
  files,
  onOpenWipFile,
  activePath,
}: {
  title: string;
  kind: WipKind;
  files: { path: string; oldPath?: string | null; status: ChangeStatus; additions?: number | null; deletions?: number | null }[];
  onOpenWipFile: (path: string, kind: WipKind) => void;
  activePath?: string | null;
}) {
  return (
    <section className="cp-wip-group" aria-label={title}>
      <div className="cp-files-head">
        <span>{title}</span>
        <span className="cp-group-count">{files.length}</span>
      </div>
      {files.length === 0 ? (
        <p className="cp-muted cp-empty">None</p>
      ) : (
        <ul className="cp-file-list">
          {files.map((f) => (
            <FileRow
              key={`${f.oldPath ?? ""}:${f.path}`}
              {...f}
              active={activePath === f.path}
              onClick={() => onOpenWipFile(f.path, kind)}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function WipMode({
  wip,
  onOpenWipFile,
  activePath,
}: {
  wip: WipStatus;
  onOpenWipFile: (path: string, kind: WipKind) => void;
  activePath?: string | null;
}) {
  const toRows = (files: ChangedFile[]) =>
    files.map((f) => ({
      path: f.path,
      oldPath: f.old_path,
      status: f.status,
      additions: f.additions,
      deletions: f.deletions,
    }));
  const total = wip.staged.length + wip.unstaged.length + wip.untracked.length;
  return (
    <div className="commit-panel">
      <div className="cp-header">
        <h3 className="cp-subject">Uncommitted changes</h3>
        <div className="cp-muted">
          {total} file{total === 1 ? "" : "s"} on top of {shortSha(wip.head_sha)}
        </div>
      </div>
      <WipGroup
        title="Staged"
        kind="staged"
        files={toRows(wip.staged)}
        onOpenWipFile={onOpenWipFile}
        activePath={activePath}
      />
      <WipGroup
        title="Unstaged"
        kind="unstaged"
        files={toRows(wip.unstaged)}
        onOpenWipFile={onOpenWipFile}
        activePath={activePath}
      />
      <WipGroup
        title="Untracked"
        kind="untracked"
        files={wip.untracked.map((path) => ({ path, status: "A" as const }))}
        onOpenWipFile={onOpenWipFile}
        activePath={activePath}
      />
    </div>
  );
}

export type CommitPanelProps =
  | {
      detail: CommitDetail;
      onOpenFile: (file: ChangedFile) => void;
      onJumpToSha: (sha: string) => void;
      onOpenPr?: (pr: PrSummary) => void;
      /** Open PRs whose head branch tip is this commit. */
      pullRequests?: PrSummary[];
      /** Extra controls rendered under the commit message (e.g. review actions). */
      actions?: ReactNode;
      activePath?: string | null;
    }
  | {
      wip: WipStatus;
      onOpenWipFile: (path: string, kind: WipKind) => void;
      activePath?: string | null;
    };

export function CommitPanel(props: CommitPanelProps) {
  if ("detail" in props) {
    return (
      <CommitMode
        detail={props.detail}
        onOpenFile={props.onOpenFile}
        onJumpToSha={props.onJumpToSha}
        onOpenPr={props.onOpenPr}
        pullRequests={props.pullRequests}
        actions={props.actions}
        activePath={props.activePath}
      />
    );
  }
  return <WipMode wip={props.wip} onOpenWipFile={props.onOpenWipFile} activePath={props.activePath} />;
}
