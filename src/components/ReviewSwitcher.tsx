import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { statusLabel, statusBadgeClass } from "../lib/status";
import { timeAgo } from "../lib/timeAgo";
import { useUIStore } from "../store";
import { Icon } from "./icons";
import type { ReviewSummary } from "../lib/types";

function verdictLabel(summary: ReviewSummary): string {
  switch (summary.review.event) {
    case "approve":
      return "Approve";
    case "request_changes":
      return "Request changes";
    case "comment":
      return "Comment";
    default:
      return "no verdict";
  }
}

/** Switches between the several reviews a target can accumulate — e.g. one
 *  published round and a new draft for a second pass. Hidden unless the
 *  target has more than one review. */
export function ReviewSwitcher({ targetId, currentReviewId }: { targetId: number; currentReviewId: number }) {
  const queryClient = useQueryClient();
  const openReview = useUIStore((s) => s.openReview);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  const queryKey = ["target-reviews", targetId];
  const reviewsQuery = useQuery({ queryKey, queryFn: () => api.listTargetReviews(targetId) });

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const newReview = useMutation({
    mutationFn: () => api.newReviewOnTarget(targetId),
    onSuccess: (review) => {
      queryClient.invalidateQueries({ queryKey });
      queryClient.invalidateQueries({ queryKey: ["reviews"] });
      setOpen(false);
      openReview(review.id);
    },
    onError: (e) => toast.error(`Could not start a new review:\n${String(e)}`),
  });

  const reviews = reviewsQuery.data ?? [];
  if (reviews.length < 2) return null;

  const index = reviews.findIndex((r) => r.review.id === currentReviewId);
  const hasOpen = reviews.some((r) => r.review.status !== "published");

  return (
    <div ref={ref} className="review-switcher">
      <button
        className="btn btn-sm"
        title="Switch between reviews on this PR"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        Review {index >= 0 ? index + 1 : "?"} of {reviews.length} <Icon name="chev" size={11} />
      </button>
      {open && (
        <div className="review-switcher-menu card" role="menu">
          {reviews.map((r) => (
            <button
              key={r.review.id}
              role="menuitem"
              className={`review-switcher-item${r.review.id === currentReviewId ? " active" : ""}`}
              onClick={() => {
                setOpen(false);
                openReview(r.review.id);
              }}
            >
              <span className={`badge ${statusBadgeClass(r.review.status)}`}>{statusLabel(r.review.status)}</span>
              <span>{verdictLabel(r)}</span>
              <span className="faint">{timeAgo(r.review.updated_at)}</span>
              <span className="faint">
                {r.comment_count} comment{r.comment_count === 1 ? "" : "s"}
              </span>
            </button>
          ))}
          <button
            role="menuitem"
            className="review-switcher-item"
            disabled={hasOpen || newReview.isPending}
            title={
              hasOpen
                ? "Publish or delete the unpublished review before starting a new one"
                : "Start a new review round on this PR"
            }
            onClick={() => newReview.mutate()}
          >
            {newReview.isPending ? "Starting…" : "+ New review"}
          </button>
        </div>
      )}
    </div>
  );
}
