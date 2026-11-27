import { useState } from "react";
import type { IgnoreRule, ReviewDiscussionComment } from "../lib/types";
import { DiscussionItem, DiscussionReply } from "./Discussion";
import { Icon } from "./icons";

export function PrConversationRow({
  subject,
  comments,
  rules,
  onChanged,
}: {
  subject: { id: string; owner: string; name: string; number: number };
  comments: ReviewDiscussionComment[];
  rules: IgnoreRule[];
  onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [showAll, setShowAll] = useState(false);
  const hiddenCount = comments.filter((c) => c.hiddenBy.length > 0).length;
  const visible = showAll ? comments : comments.filter((c) => c.hiddenBy.length === 0);
  const ruleName = (id: string) => (id === "bot" ? "bot" : (rules.find((r) => r.id === id)?.name ?? id));
  const last = visible[visible.length - 1];
  const preview = last
    ? `${last.author?.login ?? "unknown"}: ${last.body.trim().split("\n").find((l) => l.trim()) ?? ""}`
    : "";

  return (
    <div className={`review-summary-row pr-conversation-row${open ? " open" : ""}`}>
      <div className="pr-conversation-head">
        <button
          className="review-summary-toggle"
          onClick={() => setOpen((o) => !o)}
          title={open ? "Collapse PR conversation" : "Expand PR conversation"}
        >
          <Icon name="chev" size={12} className="review-summary-chev" />
          <span className="review-summary-label">PR conversation</span>
          {!open && (
            <span className={`review-summary-preview${preview ? "" : " muted"}`}>
              {preview || "No comments to show"}
            </span>
          )}
          <span className="review-summary-count muted">
            {visible.length} comment{visible.length === 1 ? "" : "s"}
            {hiddenCount > 0 && !showAll ? ` · ${hiddenCount} hidden` : ""}
          </span>
        </button>
        {hiddenCount > 0 && (
          <label className="pr-conversation-show-all" title="Show the comments hidden as bots or by the Inbox ignore rules">
            <input type="checkbox" checked={showAll} onChange={(e) => setShowAll(e.target.checked)} />
            Show all
          </label>
        )}
      </div>
      {open && (
        <div className="pr-conversation-body">
          {visible.map((c) => (
            <div
              key={c.id}
              className={`pr-conversation-item${c.hiddenBy.length > 0 ? " is-hidden" : ""}${
                c.kind === "review" && c.reviewState === "APPROVED"
                  ? " is-approved"
                  : c.kind === "review" && c.reviewState === "CHANGES_REQUESTED"
                    ? " is-changes-requested"
                    : ""
              }`}
            >
              <DiscussionItem
                comment={c}
                subjectId={subject.id}
                onChanged={onChanged}
                extra={
                  c.hiddenBy.length > 0 ? (
                    <span className="chip muted">hidden: {c.hiddenBy.map(ruleName).join(", ")}</span>
                  ) : undefined
                }
              />
            </div>
          ))}
          <DiscussionReply subject={subject} onPosted={onChanged} />
        </div>
      )}
    </div>
  );
}
