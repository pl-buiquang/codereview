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
        <button className="btn-icon" title="Delete" onClick={onDelete}>
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
      <div className="board-card-head">
        <button className="btn-icon board-card-chevron" onClick={() => setExpanded((v) => !v)}>
          <Icon name="chev" size={12} />
        </button>
        <div className="board-card-title-group">
          <div className="board-card-meta">
            <RepoName className="mono faint" name={card.repo} />
            <span className="mono faint">#{card.number}</span>
            <StatusBadge card={card} />
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
          <span className="board-card-title" title={card.title}>
            {card.title}
          </span>
        </div>
        <span className="board-card-when faint" title={card.updated_at}>
          {timeAgo(card.updated_at)}
        </span>
      </div>

      <div className="board-card-actions">
        {onOpenReview && (
          <button className="btn btn-sm btn-primary" disabled={busy || reviewBusy} onClick={onOpenReview}>
            {card.existing_review_id != null ? "Resume review" : "Create review"}
          </button>
        )}
        {onOpenVscode && (
          <button className="btn btn-sm" disabled={busy || vscodeBusy} onClick={onOpenVscode} title="Open in VSCode">
            VSCode
          </button>
        )}
        <button className="btn btn-sm btn-ghost" disabled={busy} onClick={onOpenGh}>
          GitHub
        </button>
        {card.type === "issue" && (
          <button
            className="btn btn-sm btn-ghost"
            disabled={busy}
            onClick={() => setExpanded(true)}
            title="Link a PR"
          >
            Link PR
          </button>
        )}
        {onDiscard && (
          <button className="btn btn-sm btn-ghost" disabled={busy} onClick={discard}>
            Discard
          </button>
        )}
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
            <button className="btn btn-sm btn-ghost board-meta-add-toggle" onClick={() => setAdding(true)}>
              <Icon name="plus" size={11} /> Add metadata
            </button>
          )}
        </div>
      )}
    </div>
  );
}
