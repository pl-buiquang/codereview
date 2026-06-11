import { useEffect, useRef, useState } from "react";
import { confirmDialog } from "../lib/confirm";
import { Icon } from "./icons";
import type { ReviewEvent } from "../lib/types";

/** Verdicts offered in the publish dropdown. The first is the default action
 *  the main button performs; the rest sit behind the caret. */
const PUBLISH_OPTIONS: { value: ReviewEvent; label: string }[] = [
  { value: "comment", label: "Comment" },
  { value: "approve", label: "Approve" },
  { value: "request_changes", label: "Request changes" },
];

/** Split button that publishes a review to its GitHub PR. The main button
 *  publishes as a plain comment; the caret reveals the other verdicts (approve /
 *  request changes). Replaces the old standalone verdict radio + publish pair. */
export function PublishButton({
  published,
  pending,
  onPublish,
}: {
  published: boolean;
  pending: boolean;
  onPublish: (event: ReviewEvent) => void;
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

  const publishAs = async (event: ReviewEvent, label: string) => {
    setMenuOpen(false);
    if (
      await confirmDialog({
        title: "Publish review",
        message: `Publish this review to the GitHub PR as “${label}”? This cannot be undone.`,
        confirmLabel: "Publish",
        danger: true,
      })
    )
      onPublish(event);
  };

  const primary = PUBLISH_OPTIONS[0];

  return (
    <div ref={ref} className="btn-split">
      <button
        className="btn btn-primary"
        disabled={pending}
        title="Post this review to the GitHub PR as a comment"
        onClick={() => publishAs(primary.value, primary.label)}
      >
        {pending ? "Publishing…" : "Publish"}
      </button>
      <button
        className="btn btn-primary"
        disabled={pending}
        title="Publish with a verdict"
        aria-label="Publish with a verdict"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        onClick={() => setMenuOpen((o) => !o)}
      >
        <Icon name="chev" size={11} />
      </button>
      {menuOpen && (
        <div className="btn-split-menu card" role="menu">
          {PUBLISH_OPTIONS.map((o) => (
            <button key={o.value} role="menuitem" onClick={() => publishAs(o.value, o.label)}>
              {o.label === "Comment" ? "Comment (default)" : o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
