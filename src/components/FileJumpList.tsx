import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useQuery } from "@tanstack/react-query";
import { parseDiff } from "react-diff-view";
import { api } from "../lib/api";
import { countChanges, fileDisplayPath } from "../lib/diff";

export function FileJumpList({
  reviewId,
  scrollRootRef,
}: {
  reviewId: number;
  scrollRootRef: RefObject<HTMLElement | null>;
}) {
  const detailQuery = useQuery({
    queryKey: ["review", reviewId],
    queryFn: () => api.getReview(reviewId),
    enabled: reviewId != null,
  });
  const detail = detailQuery.data;

  const diffQuery = useQuery({
    queryKey: ["review-diff", reviewId, detail?.target.id],
    enabled: detail != null,
    queryFn: () => api.reviewDiff(reviewId),
  });
  const diff = diffQuery.data;

  const files = useMemo(() => (diff ? parseDiff(diff) : []), [diff]);

  const countByPath = useMemo(() => {
    const map = new Map<string, number>();
    for (const c of detail?.comments ?? []) {
      map.set(c.file_path, (map.get(c.file_path) ?? 0) + 1);
    }
    return map;
  }, [detail?.comments]);

  const viewedSet = useMemo(
    () => new Set(detail?.viewed_files ?? []),
    [detail?.viewed_files],
  );

  const rows = useMemo(
    () =>
      files.map((file, index) => {
        const path = fileDisplayPath(file);
        const { add, del } = countChanges(file);
        return {
          index,
          path,
          add,
          del,
          count: countByPath.get(path) ?? 0,
          viewed: viewedSet.has(path),
        };
      }),
    [files, countByPath, viewedSet],
  );

  const [activeIndex, setActiveIndex] = useState(0);
  // A click selects a file outright and "locks" auto-selection: the scrollspy is
  // muted while the click-triggered smooth scroll plays out, then released once
  // scrolling goes quiet — without re-picking — so the chosen file stays selected
  // until the user actually scrolls again. (Otherwise the smooth scroll, or the
  // bottom-of-view fallback, would immediately steal the selection back — which
  // is why a trailing file that already fits on screen couldn't stay selected.)
  const lockedRef = useRef(false);
  const releaseTimerRef = useRef(0);

  // Track the active file from scroll position, scoping every `#file-N` lookup
  // to this review's own scroll panel. Other mounted review tabs render the same
  // `file-N` ids, so a document-wide getElementById would resolve to whichever
  // tab is first in the DOM (often a display:none one) — which is why the jump
  // list reacted on one tab and went dead on the others. A scrollspy (rather than
  // an IntersectionObserver band) also lets the last file win: a short trailing
  // file can never scroll far enough up to trip an observer trigger zone.
  useEffect(() => {
    const root = scrollRootRef.current;
    if (!root || files.length === 0) return;

    let raf = 0;
    const recompute = () => {
      raf = 0;
      if (lockedRef.current) return;
      const trigger = root.getBoundingClientRect().top + 100;
      let active = 0;
      for (let i = 0; i < files.length; i++) {
        const el = root.querySelector<HTMLElement>(`#file-${i}`);
        if (!el) continue;
        if (el.getBoundingClientRect().top <= trigger) active = i;
        else break;
      }
      // A short trailing file can't reach the trigger line; once the panel is
      // scrolled to the bottom, treat the last file as active.
      if (root.scrollTop + root.clientHeight >= root.scrollHeight - 2) {
        active = files.length - 1;
      }
      setActiveIndex(active);
    };
    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(recompute);
    };
    const onScroll = () => {
      // While a manual selection holds, don't auto-pick. Each scroll event from
      // the click's smooth scroll just pushes the release back; once scrolling
      // has been quiet briefly, hand control back to the scrollspy as-is.
      if (lockedRef.current) {
        window.clearTimeout(releaseTimerRef.current);
        releaseTimerRef.current = window.setTimeout(() => {
          lockedRef.current = false;
        }, 150);
        return;
      }
      schedule();
    };

    recompute();
    root.addEventListener("scroll", onScroll, { passive: true });
    const ro = new ResizeObserver(schedule);
    ro.observe(root);
    return () => {
      root.removeEventListener("scroll", onScroll);
      ro.disconnect();
      if (raf) cancelAnimationFrame(raf);
      window.clearTimeout(releaseTimerRef.current);
    };
  }, [files.length, reviewId, scrollRootRef]);

  const jumpTo = (index: number) => {
    lockedRef.current = true;
    // Fallback for clicking a file that's already in view: no scroll fires, so
    // the scroll-settle handler never runs — release the lock after a beat.
    window.clearTimeout(releaseTimerRef.current);
    releaseTimerRef.current = window.setTimeout(() => {
      lockedRef.current = false;
    }, 700);
    setActiveIndex(index);
    scrollRootRef.current
      ?.querySelector(`#file-${index}`)
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  return (
    <nav className="jump-list">
      <div className="jump-list-header">Files ({rows.length})</div>
      {rows.map((row) => (
        <button
          key={row.index}
          type="button"
          className={`jump-row${row.index === activeIndex ? " active" : ""}${
            row.viewed ? " viewed" : ""
          }`}
          onClick={() => jumpTo(row.index)}
          title={row.path}
        >
          <span className="jump-path">{row.path}</span>
          <span className="jump-meta">
            {row.count > 0 && <span className="jump-badge">{row.count}</span>}
            <span className="add">+{row.add}</span>
            <span className="del">−{row.del}</span>
          </span>
        </button>
      ))}
    </nav>
  );
}
