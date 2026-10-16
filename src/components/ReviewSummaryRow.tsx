import { useEffect, useState, type MutableRefObject } from "react";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { useDebouncedCallback } from "../lib/useDebouncedCallback";
import { Icon } from "./icons";

export function ReviewSummaryRow({
  reviewId,
  initialBody,
  readOnly,
  bodyRef,
  summaryAppend,
  onSaving,
  onSaved,
}: {
  reviewId: number;
  initialBody: string;
  readOnly: boolean;
  /** Mirrors the live body so the header's publish actions send unsaved edits. */
  bodyRef: MutableRefObject<string | null>;
  summaryAppend: { text: string; nonce: number } | null;
  onSaving: () => void;
  onSaved: () => void;
}) {
  const [body, setBody] = useState(initialBody);
  const [open, setOpen] = useState(initialBody.trim().length > 0);
  bodyRef.current = body;

  const save = useDebouncedCallback((next: string) => {
    onSaving();
    api
      .updateReview(reviewId, next)
      .then(onSaved)
      .catch((e) => toast.error(String(e)));
  }, 400);

  useEffect(() => {
    if (!summaryAppend) return;
    const prev = (bodyRef.current ?? "").trimEnd();
    const next = prev ? `${prev}\n\n${summaryAppend.text}` : summaryAppend.text;
    setBody(next);
    save(next);
    setOpen(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [summaryAppend]);

  const preview = body.trim().split("\n").find((l) => l.trim()) ?? "";

  return (
    <div className={`review-summary-row${open ? " open" : ""}`}>
      <button
        className="review-summary-toggle"
        onClick={() => setOpen((o) => !o)}
        title={open ? "Collapse review summary" : "Expand review summary"}
      >
        <Icon name="chev" size={12} className="review-summary-chev" />
        <span className="review-summary-label">Review summary</span>
        {!open && (
          <span className={`review-summary-preview${preview ? "" : " muted"}`}>
            {preview || (readOnly ? "No summary" : "Add a summary…")}
          </span>
        )}
        {body.length > 0 && <span className="review-summary-count muted">{body.length} chars</span>}
      </button>
      {open && (
        <textarea
          className="textarea"
          placeholder="Review summary…"
          value={body}
          disabled={readOnly}
          onChange={(e) => {
            setBody(e.target.value);
            save(e.target.value);
          }}
        />
      )}
    </div>
  );
}
