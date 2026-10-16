import { useEffect, useRef, useState } from "react";
import type { BoardCard as BoardCardT, BoardMeta, BoardMetaKind, ClaudeSession } from "../lib/types";
import { api, pickFile } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { timeAgo } from "../lib/timeAgo";
import { useSettingsStore, type BoardBorderSide } from "../lib/settings";
import { useUIStore } from "../store";
import { RepoName } from "./RepoName";
import { AutoMergeBadge, CiBadge, CommentsBadge, MergeStateBadge, MergeableBadge } from "./InboxBadges";
import { Icon } from "./icons";
import { AgentSessionItems, KebabMenu } from "./KebabMenu";

const BORDER_STYLE_KEY: Record<BoardBorderSide, "borderTopColor" | "borderRightColor" | "borderBottomColor" | "borderLeftColor"> = {
  top: "borderTopColor",
  right: "borderRightColor",
  bottom: "borderBottomColor",
  left: "borderLeftColor",
};
const BORDER_WIDTH_KEY: Record<BoardBorderSide, "borderTopWidth" | "borderRightWidth" | "borderBottomWidth" | "borderLeftWidth"> = {
  top: "borderTopWidth",
  right: "borderRightWidth",
  bottom: "borderBottomWidth",
  left: "borderLeftWidth",
};

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

/** CI facet (lowercased `ci_status`) for the board's CI filter. Issue cards
 *  and PRs with no reported checks have no facet (return null). */
export function ciStatus(card: BoardCardT): string | null {
  if (card.type !== "pr" || !card.ci_status) return null;
  return card.ci_status.toLowerCase();
}

export const MERGE_FACET_LABEL: Record<string, string> = {
  auto_merge: "auto-merge",
  conflicts: "merge conflict",
  behind: "needs update",
};

/** Merge-related GitHub states of an open PR (several can apply at once). */
export function mergeFacets(card: BoardCardT): string[] {
  if (card.type !== "pr" || card.state !== "open") return [];
  const out: string[] = [];
  if (card.auto_merge) out.push("auto_merge");
  if (card.mergeable === "CONFLICTING" || card.merge_state_status === "DIRTY") out.push("conflicts");
  if (card.merge_state_status === "BEHIND") out.push("behind");
  return out;
}

/** Distinct tags carried by a card's own + inherited metadata, for the
 *  board's Tags filter. */
export function cardTags(card: BoardCardT): string[] {
  const tags = new Set<string>();
  for (const m of [...card.meta, ...card.inherited_meta]) {
    if (m.tag) tags.add(m.tag);
  }
  return [...tags];
}

// Drafts live only in the "in progress" lane (redundant to call out) and
// "ready" has nothing to flag yet, so only these four get a color.
const STATUS_BORDER_COLOR: Record<string, string> = {
  review_required: "var(--accent)",
  commented: "var(--warning)",
  approved: "var(--success)",
  changes_requested: "var(--danger)",
};

export function MetaRow({
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
        message: `Delete "${meta.label || meta.value || meta.tag}"?`,
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

export function AddMetaForm({
  tags,
  onAdd,
  onClose,
}: {
  tags: string[];
  onAdd: (args: { kind: BoardMetaKind; value: string; tag: string | null; label: string | null }) => void;
  onClose: () => void;
}) {
  const [kind, setKind] = useState<BoardMetaKind>("note");
  const [value, setValue] = useState("");
  const [tag, setTag] = useState("");
  const [label, setLabel] = useState("");

  const browse = async () => {
    const path = await pickFile();
    if (path) setValue(path);
  };

  const trimmedTag = tag.trim();
  const canSubmit = value.trim() || (kind === "note" && trimmedTag);

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    onAdd({ kind, value: value.trim(), tag: trimmedTag || null, label: label.trim() || null });
    setValue("");
    setTag("");
    setLabel("");
    onClose();
  };

  return (
    <form className="board-meta-add" onSubmit={submit}>
      <select className="select" value={kind} onChange={(e) => setKind(e.target.value as BoardMetaKind)}>
        <option value="note">Note</option>
        <option value="file">File</option>
        <option value="link">Link</option>
      </select>
      <div className="board-meta-add-value">
        <input
          className="input"
          placeholder={kind === "file" ? "Path" : kind === "link" ? "URL" : "Note text (optional with a tag)"}
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
        <button type="submit" className="btn btn-sm btn-primary" disabled={!canSubmit}>
          Add
        </button>
        <button type="button" className="btn btn-sm btn-ghost" onClick={onClose}>
          Cancel
        </button>
      </div>
    </form>
  );
}

export interface PrActions {
  busy: boolean;
  onSetAutoMerge: (enable: boolean) => void;
  onSetDraft: (draft: boolean) => void;
  onUpdateBranch: (rebase: boolean) => void;
}

export interface TerminalAction {
  /** Focusable live sessions of this card, most recently active first. */
  sessions: ClaudeSession[];
  busy: boolean;
  onFocus: (sessionId: string) => void;
  onOpenShell: () => void;
  onOpenAgent: () => void;
}

function PrActionsMenu({
  card,
  actions,
  terminal,
}: {
  card: BoardCardT;
  actions: PrActions;
  terminal: TerminalAction | null;
}) {
  const isOpen = card.state === "open";
  const behind = card.merge_state_status === "BEHIND";
  const stacked = card.stack_number != null;
  const updateDisabledReason = !isOpen
    ? "PR is not open"
    : stacked
      ? "Stacked PR — rebase the stack instead"
      : !behind
        ? "Branch is up to date"
        : !card.viewer_can_update_branch
          ? "You can't update this branch"
          : null;

  return (
    <KebabMenu title="PR actions" disabled={actions.busy} className="board-card-menu">
      {(close) => {
        const run = (fn: () => void) => () => {
          close();
          fn();
        };
        const autoMerge = async () => {
          close();
          if (card.auto_merge) {
            actions.onSetAutoMerge(false);
            return;
          }
          if (
            await confirmDialog({
              title: "Enable auto-merge",
              message: `Squash-merge ${card.repo}#${card.number} "${card.title}" and delete its branch once all requirements pass?`,
              confirmLabel: "Enable auto-merge",
            })
          ) {
            actions.onSetAutoMerge(true);
          }
        };
        return (
          <>
            <button
              role="menuitem"
              disabled={!isOpen || (card.is_draft && !card.auto_merge)}
              title={!isOpen ? "PR is not open" : card.is_draft && !card.auto_merge ? "Draft PRs can't be merged" : undefined}
              onClick={autoMerge}
            >
              {card.auto_merge ? "Disable auto-merge" : "Enable auto-merge (squash)"}
            </button>
            <button
              role="menuitem"
              disabled={!isOpen}
              title={!isOpen ? "PR is not open" : undefined}
              onClick={run(() => actions.onSetDraft(!card.is_draft))}
            >
              {card.is_draft ? "Mark ready for review" : "Convert to draft"}
            </button>
            {terminal && (
              <>
                <hr className="btn-split-menu-separator" />
                <button
                  role="menuitem"
                  disabled={terminal.busy}
                  title="Open a shell in the chat worktree"
                  onClick={run(terminal.onOpenShell)}
                >
                  Open in terminal
                </button>
                <AgentSessionItems
                  sessions={terminal.sessions}
                  busy={terminal.busy}
                  close={close}
                  onFocus={terminal.onFocus}
                  onOpenNew={terminal.onOpenAgent}
                />
              </>
            )}
            <hr className="btn-split-menu-separator" />
            <button
              role="menuitem"
              disabled={updateDisabledReason != null}
              title={updateDisabledReason ?? undefined}
              onClick={run(() => actions.onUpdateBranch(false))}
            >
              Update branch (merge)
            </button>
            <button
              role="menuitem"
              disabled={updateDisabledReason != null}
              title={updateDisabledReason ?? undefined}
              onClick={run(() => actions.onUpdateBranch(true))}
            >
              Update branch (rebase)
            </button>
          </>
        );
      }}
    </KebabMenu>
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
  terminal,
  onAddMeta,
  onDeleteMeta,
  prActions,
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
  terminal: TerminalAction | null;
  onAddMeta: (args: { kind: BoardMetaKind; value: string; tag: string | null; label: string | null }) => void;
  onDeleteMeta: (id: number) => void;
  prActions: PrActions | null;
}) {
  const [metaExpanded, setMetaExpanded] = useState(false);
  const [adding, setAdding] = useState(false);
  const [linkInput, setLinkInput] = useState("");
  const [titleMenuOpen, setTitleMenuOpen] = useState(false);
  const titleMenuRef = useRef<HTMLSpanElement>(null);
  const cardRef = useRef<HTMLDivElement>(null);
  const highlighted = useUIStore((s) => s.boardHighlightCardId === card.id);
  const clearBoardHighlight = useUIStore((s) => s.clearBoardHighlight);

  useEffect(() => {
    if (!highlighted) return;
    cardRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
    const id = setTimeout(clearBoardHighlight, 2000);
    return () => clearTimeout(id);
  }, [highlighted, clearBoardHighlight]);

  useEffect(() => {
    if (!titleMenuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (titleMenuRef.current && !titleMenuRef.current.contains(e.target as Node)) setTitleMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [titleMenuOpen]);

  const allMeta = [...card.meta, ...card.inherited_meta];
  const inheritedIds = new Set(card.inherited_meta.map((m) => m.id));
  const hasMoreMeta = allMeta.length > 3;
  const visibleMeta = metaExpanded ? allMeta : allMeta.slice(0, 3);
  const showLinkForm = card.type === "issue" && !!onLinkPr;
  const hasExtra = showLinkForm || card.linked_pr_ids.length > 0 || visibleMeta.length > 0 || hasMoreMeta || adding;

  const borderSide = useSettingsStore((s) => s.boardStatusBorderSide);
  const status = prStatus(card);
  const borderColor = status ? STATUS_BORDER_COLOR[status] : undefined;
  const borderStyle = borderColor
    ? { [BORDER_STYLE_KEY[borderSide]]: borderColor, [BORDER_WIDTH_KEY[borderSide]]: "3px" }
    : undefined;

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
    <div ref={cardRef} className={`card board-card${highlighted ? " board-card-highlight" : ""}`} style={borderStyle}>
      <div className="board-card-head">
        <span ref={titleMenuRef} className="board-card-title-wrap">
          <button
            className="board-card-title-link"
            onClick={onOpenGh}
            onContextMenu={(e) => {
              e.preventDefault();
              setTitleMenuOpen(true);
            }}
            title={`Open ${card.repo}#${card.number} on GitHub`}
          >
            {card.title} <span className="board-card-number">#{card.number}</span>
          </button>
          {titleMenuOpen && (
            <div className="btn-split-menu card" role="menu">
              <button
                role="menuitem"
                onClick={() => {
                  setTitleMenuOpen(false);
                  navigator.clipboard.writeText(card.url).then(
                    () => toast.success("GitHub link copied to clipboard"),
                    (e) => toast.error(`Could not copy link: ${String(e)}`),
                  );
                }}
              >
                Copy GitHub link
              </button>
            </div>
          )}
        </span>
        {card.type === "pr" && prActions && <PrActionsMenu card={card} actions={prActions} terminal={terminal} />}
      </div>

      <div className="board-card-subtitle-row">
        <button className="board-card-link board-card-repo" onClick={onOpenRepo} title="Open repository">
          <RepoName name={card.repo} />
        </button>
      </div>

      {card.type === "pr" && (
        <div className="board-card-badge-row">
          <CiBadge state={card.ci_status ? card.ci_status.toLowerCase() : null} />
          <MergeableBadge mergeable={card.mergeable} />
          <MergeStateBadge state={card.merge_state_status} stacked={card.stack_number != null} />
          <AutoMergeBadge method={card.auto_merge} />
          <CommentsBadge count={card.pending_threads} />
        </div>
      )}

      <div className="board-card-actions">
        {onOpenReview && (
          <button
            className="btn btn-sm btn-primary board-card-action-btn"
            disabled={busy || reviewBusy}
            onClick={onOpenReview}
            title={card.existing_review_id != null ? "Resume review" : "Create review"}
          >
            <Icon name="review" size={13} /> <span className="board-card-action-label">Review</span>
          </button>
        )}
        {onOpenVscode && (
          <button
            className="btn btn-sm board-card-action-btn"
            disabled={busy || vscodeBusy}
            onClick={onOpenVscode}
            title="Open in VSCode"
          >
            <Icon name="code" size={13} /> <span className="board-card-action-label">VSCode</span>
          </button>
        )}
        {onDiscard && (
          <button className="btn btn-sm btn-ghost board-card-action-btn" disabled={busy} onClick={discard} title="Discard">
            <Icon name="trash" size={13} /> <span className="board-card-action-label">Discard</span>
          </button>
        )}
      </div>

      {hasExtra && (
        <div className="board-card-extra">
          {showLinkForm && (
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

          {visibleMeta.length > 0 && (
            <div className="board-meta-list">
              {visibleMeta.map((m) => (
                <MetaRow key={m.id} meta={m} inherited={inheritedIds.has(m.id)} onDelete={() => onDeleteMeta(m.id)} />
              ))}
            </div>
          )}

          {(hasMoreMeta || adding) && (
            <div className="board-card-meta-actions">
              {hasMoreMeta && (
                <button
                  className="btn-icon"
                  title={metaExpanded ? "Show fewer" : `Show ${allMeta.length - 3} more`}
                  onClick={() => setMetaExpanded((v) => !v)}
                >
                  <Icon name="chev" size={12} className={metaExpanded ? "board-card-chev-open" : undefined} />
                </button>
              )}
              {adding && <AddMetaForm tags={tags} onAdd={onAddMeta} onClose={() => setAdding(false)} />}
            </div>
          )}
        </div>
      )}

      <div className="board-card-bottom-row">
        {!adding && (
          <button className="btn-icon board-card-add-meta" title="Add metadata" onClick={() => setAdding(true)}>
            <Icon name="plus" size={13} />
          </button>
        )}
        <span className="cr-spacer" />
        <span className="board-card-when faint" title={`Last updated ${card.updated_at}`}>
          {timeAgo(card.updated_at)}
        </span>
      </div>
    </div>
  );
}
