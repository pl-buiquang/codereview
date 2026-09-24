import { useEffect, useRef, useState } from "react";
import { confirmDialog } from "../lib/confirm";
import { Icon } from "./icons";
import type { ReviewEvent } from "../lib/types";

const VERDICTS: { value: ReviewEvent; label: string }[] = [
  { value: "comment", label: "Comment" },
  { value: "approve", label: "Approve" },
  { value: "request_changes", label: "Request changes" },
];

/** Split button that publishes a review to its GitHub PR. The primary action
 *  stages it as a pending (draft) review; the dropdown exposes all three
 *  verdicts for both draft and immediate publish. */
export function PublishButton({
  published,
  pending,
  draftPending,
  onPublish,
  onPublishDraft,
}: {
  published: boolean;
  pending: boolean;
  draftPending: boolean;
  onPublish: (event: ReviewEvent) => void;
  onPublishDraft: (event: ReviewEvent) => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [menuOpen]);

  if (published)
    return (
      <button
        className="btn btn-primary"
        disabled
        title="Already published — cannot publish again"
      >
        Published
      </button>
    );

  const busy = pending || draftPending;

  const publishDraftAs = async (event: ReviewEvent, label: string) => {
    setMenuOpen(false);
    if (
      await confirmDialog({
        title: "Publish as draft",
        message: `Stage this review on GitHub as a pending (draft) "${label}" review? It will be visible only to you until submitted.`,
        confirmLabel: "Publish as draft",
      })
    )
      onPublishDraft(event);
  };

  const publishAs = async (event: ReviewEvent, label: string) => {
    setMenuOpen(false);
    if (
      await confirmDialog({
        title: "Publish review",
        message: `Publish this review to the GitHub PR as "${label}"? This cannot be undone.`,
        confirmLabel: "Publish",
        danger: true,
      })
    )
      onPublish(event);
  };

  return (
    <div ref={ref} className="btn-split">
      <button
        className="btn btn-primary"
        disabled={busy}
        title="Stage this review on GitHub as a pending (draft) review, visible only to you"
        onClick={() => publishDraftAs("comment", "Comment")}
      >
        {draftPending ? (
          <>
            <span className="spinner" /> Staging…
          </>
        ) : (
          "Publish as draft"
        )}
      </button>
      <button
        className="btn btn-primary"
        disabled={busy}
        title="Choose verdict or publish immediately"
        aria-label="Choose verdict or publish immediately"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((o) => !o)}
      >
        <Icon name="chev" size={11} />
      </button>
      {menuOpen && (
        <div className="btn-split-menu card" role="menu">
          {VERDICTS.map((o) => (
            <button key={`draft-${o.value}`} role="menuitem" onClick={() => publishDraftAs(o.value, o.label)}>
              {o.label}{o.value === "comment" ? " (draft, default)" : " (draft)"}
            </button>
          ))}
          <hr className="btn-split-menu-separator" />
          {VERDICTS.map((o) => (
            <button key={`now-${o.value}`} role="menuitem" onClick={() => publishAs(o.value, o.label)}>
              Publish now: {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
