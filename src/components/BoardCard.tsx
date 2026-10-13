import { useState } from "react";
import type { BoardCard as BoardCardT, BoardMeta, BoardMetaKind } from "../lib/types";
import { api, pickFile } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { timeAgo } from "../lib/timeAgo";
import { RepoName } from "./RepoName";
import { Icon } from "./icons";

/** PR status facet shown as a badge and used by the board's status filter.
 *  Issue cards have no status (return null). */
export function prStatus(card: BoardCardT): string | null {
  if (card.type !== "pr") return null;
  if (card.is_draft) return "draft";
  if (card.review_decision === "APPROVED") return "approved";
  if (card.review_decision === "CHANGES_REQUESTED") return "changes_requested";
  if (card.review_decision === "REVIEW_REQUIRED") return "review_required";
  if (card.has_reviews) return "commented";
  return "ready";
}

export function prStatusLabel(status: string): string {
  return status.replace(/_/g, " ");
}

function StatusBadge({ card }: { card: BoardCardT }) {
  const status = prStatus(card);
  if (!status) return null;
  const kind =
    status === "approved"
      ? "badge-open"
      : status === "changes_requested"
        ? "badge-review"
        : status === "draft"
          ? "badge-draft"
          : "badge-pr";
  return <span className={`badge ${kind}`}>{prStatusLabel(status)}</span>;
}

function MetaRow({
  meta,
  inherited,
  onDelete,
}: {
  meta: BoardMeta;
  inherited: boolean;
  onDelete: () => void;
}) {
  const open = () => {
    if (meta.kind === "file") api.openInDefaultApp(meta.value).catch((e) => toast.error(String(e)));
    else if (meta.kind === "link") api.openUrl(meta.value).catch((e) => toast.error(String(e)));
  };
  const copyResume = () => {
    const cmd = `cd ${meta.cwd ?? "."}; claude --resume ${meta.value}`;
    navigator.clipboard.writeText(cmd).then(
      () => toast.success("Copied resume command"),
      (e) => toast.error(String(e)),
    );
  };
  const del = async () => {
    if (
      await confirmDialog({
        title: "Delete metadata",
        message: `Delete "${meta.label || meta.value}"?`,
        confirmLabel: "Delete",
        danger: true,
      })
    ) {
      onDelete();
    }
  };
  return (
    <div className="board-meta-row">
      <Icon
        name={
          meta.kind === "file" ? "file" : meta.kind === "link" ? "link" : meta.kind === "note" ? "comment" : "ext"
        }
        size={13}
      />
      {meta.tag && <span className="chip">{meta.tag}</span>}
      {meta.kind === "note" ? (
        <span className="board-meta-value">{meta.value}</span>
      ) : (
        <button className="board-meta-value board-meta-link" onClick={open} title={meta.value}>
          {meta.label || meta.value}
        </button>
      )}
      {inherited && <span className="faint">from issue</span>}
      {meta.kind === "claude_session" && (
        <button className="btn-icon" title="Copy resume command" onClick={copyResume}>
          <Icon name="copy" size={12} />
        </button>
      )}
      {!inherited && (
        <button className="btn-icon" title="Delete" onClick={del}>
          <Icon name="x" size={12} />
        </button>
      )}
    </div>
  );
}

function AddMetaForm({
  tags,
  onAdd,
  onClose,
}: {
  tags: string[];
  onAdd: (args: { kind: BoardMetaKind; value: string; tag: string | null; label: string | null }) => void;
  onClose: () => void;
}) {
  const [kind, setKind] = useState<BoardMetaKind>("file");
  const [value, setValue] = useState("");
  const [tag, setTag] = useState("");
  const [label, setLabel] = useState("");

  const browse = async () => {
    const path = await pickFile();
    if (path) setValue(path);
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!value.trim()) return;
    onAdd({ kind, value: value.trim(), tag: tag.trim() || null, label: label.trim() || null });
    setValue("");
    setTag("");
    setLabel("");
    onClose();
  };

  return (
    <form className="board-meta-add" onSubmit={submit}>
      <select className="select" value={kind} onChange={(e) => setKind(e.target.value as BoardMetaKind)}>
        <option value="file">File</option>
        <option value="link">Link</option>
        <option value="note">Note</option>
      </select>
      <div className="board-meta-add-value">
        <input
          className="input"
          placeholder={kind === "file" ? "Path" : kind === "link" ? "URL" : "Note text"}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          autoFocus
        />
        {kind === "file" && (
          <button type="button" className="btn btn-sm" onClick={browse}>
            Browse
          </button>
        )}
      </div>
      <input
        className="input"
        list="board-meta-tags"
        placeholder="tag (e.g. plan)"
        value={tag}
        onChange={(e) => setTag(e.target.value)}
      />
      <datalist id="board-meta-tags">
        {tags.map((t) => (
          <option key={t} value={t} />
        ))}
      </datalist>
      <input
        className="input"
        placeholder="label (optional)"
        value={label}
        onChange={(e) => setLabel(e.target.value)}
      />
      <div className="board-meta-add-actions">
        <button type="submit" className="btn btn-sm btn-primary">
          Add
        </button>
        <button type="button" className="btn btn-sm btn-ghost" onClick={onClose}>
          Cancel
        </button>
      </div>
    </form>
  );
}

export function BoardCard({
  card,
  tags,
  busy,
  reviewBusy,
  vscodeBusy,
  onOpenGh,
  onOpenRepo,
  onDiscard,
  onLinkPr,
  onUnlink,
  onOpenReview,
  onOpenVscode,
  onAddMeta,
  onDeleteMeta,
}: {
  card: BoardCardT;
  tags: string[];
  busy: boolean;
  reviewBusy: boolean;
  vscodeBusy: boolean;
  onOpenGh: () => void;
  onOpenRepo: () => void;
  onDiscard: (() => void) | null;
  onLinkPr: ((prReference: string) => void) | null;
  onUnlink: ((prId: string) => void) | null;
  onOpenReview: (() => void) | null;
  onOpenVscode: (() => void) | null;
  onAddMeta: (args: { kind: BoardMetaKind; value: string; tag: string | null; label: string | null }) => void;
  onDeleteMeta: (id: number) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [adding, setAdding] = useState(false);
  const [linkInput, setLinkInput] = useState("");

  const allMeta = [...card.meta, ...card.inherited_meta];
  const inheritedIds = new Set(card.inherited_meta.map((m) => m.id));
  const attachmentCount = allMeta.length;

  const discard = async () => {
    if (!onDiscard) return;
    if (
      await confirmDialog({
        title: "Discard card",
        message: `Discard ${card.repo}#${card.number}? You can re-track it from the Inbox.`,
        confirmLabel: "Discard",
      })
    ) {
      onDiscard();
    }
  };

  const submitLink = (e: React.FormEvent) => {
    e.preventDefault();
    if (!linkInput.trim() || !onLinkPr) return;
    onLinkPr(linkInput.trim());
    setLinkInput("");
  };

  return (
    <div className="card board-card">
      <div className="board-card-body">
        <span className="board-card-title" title={card.title}>
          {card.title}
        </span>
        <div className="board-card-meta">
          <button className="board-card-link board-card-repo" onClick={onOpenRepo} title="Open repository">
            <RepoName name={card.repo} />
          </button>
          <button className="board-card-link board-card-number" onClick={onOpenGh} title="Open on GitHub">
            #{card.number}
          </button>
          <StatusBadge card={card} />
          <span className="cr-spacer" />
          <span className="board-card-when faint" title={card.updated_at}>
            {timeAgo(card.updated_at)}
          </span>
        </div>
      </div>

      <div className="board-card-footer">
        <div className="board-card-chips">
          {card.pending_threads != null && card.pending_threads > 0 && (
            <span className="chip" title="Unresolved threads waiting on you">
              <Icon name="comment" size={11} /> {card.pending_threads}
            </span>
          )}
          {attachmentCount > 0 && (
            <span className="chip" title={`${attachmentCount} attachment(s)`}>
              <Icon name="file" size={11} /> {attachmentCount}
            </span>
          )}
        </div>
        <div className="board-card-actions">
          {onOpenReview && (
            <button
              className="btn-icon board-card-action board-card-action-primary"
              disabled={busy || reviewBusy}
              onClick={onOpenReview}
              title={card.existing_review_id != null ? "Resume review" : "Create review"}
            >
              <Icon name="review" size={13} />
            </button>
          )}
          {onOpenVscode && (
            <button
              className="btn-icon board-card-action"
              disabled={busy || vscodeBusy}
              onClick={onOpenVscode}
              title="Open in VSCode"
            >
              <Icon name="code" size={13} />
            </button>
          )}
          {card.type === "issue" && (
            <button
              className="btn-icon board-card-action"
              disabled={busy}
              onClick={() => setExpanded(true)}
              title="Link a PR"
            >
              <Icon name="link" size={13} />
            </button>
          )}
          {onDiscard && (
            <button className="btn-icon board-card-action-danger" disabled={busy} onClick={discard} title="Discard">
              <Icon name="trash" size={13} />
            </button>
          )}
        </div>
      </div>

      {expanded && (
        <div className="board-card-expand">
          {card.type === "issue" && onLinkPr && (
            <form className="board-link-form" onSubmit={submitLink}>
              <input
                className="input"
                placeholder="PR URL or owner/repo#n"
                value={linkInput}
                onChange={(e) => setLinkInput(e.target.value)}
              />
              <button type="submit" className="btn btn-sm">
                Link
              </button>
            </form>
          )}

          {card.linked_pr_ids.length > 0 && (
            <div className="board-links">
              {card.linked_pr_ids.map((prId) => (
                <span key={prId} className="chip">
                  linked PR
                  {onUnlink && (
                    <button className="btn-icon" title="Unlink" onClick={() => onUnlink(prId)}>
                      <Icon name="x" size={10} />
                    </button>
                  )}
                </span>
              ))}
            </div>
          )}

          {allMeta.length > 0 && (
            <div className="board-meta-list">
              {allMeta.map((m) => (
                <MetaRow key={m.id} meta={m} inherited={inheritedIds.has(m.id)} onDelete={() => onDeleteMeta(m.id)} />
              ))}
            </div>
          )}

          {adding ? (
            <AddMetaForm tags={tags} onAdd={onAddMeta} onClose={() => setAdding(false)} />
          ) : (
            <button className="btn-icon board-card-action" title="Add metadata" onClick={() => setAdding(true)}>
              <Icon name="plus" size={13} />
            </button>
          )}
        </div>
      )}

      <button
        className="board-card-expand-toggle"
        onClick={() => setExpanded((v) => !v)}
        title={expanded ? "Collapse" : "Expand"}
      >
        <Icon name="chev" size={11} className={expanded ? "board-card-chev-open" : undefined} />
      </button>
    </div>
  );
}
