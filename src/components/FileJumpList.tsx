import { useEffect, useMemo, useState, type RefObject } from "react";
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

  useEffect(() => {
    if (files.length === 0) return;
    const root = scrollRootRef.current;
    const observed: Element[] = [];
    const visible = new Set<number>();

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const idx = Number((entry.target as HTMLElement).dataset.fileIndex);
          if (entry.isIntersecting) visible.add(idx);
          else visible.delete(idx);
        }
        if (visible.size > 0) setActiveIndex(Math.min(...visible));
      },
      { root, rootMargin: "0px 0px -70% 0px" },
    );

    for (let i = 0; i < files.length; i++) {
      const el = document.getElementById(`file-${i}`);
      if (!el) continue;
      el.dataset.fileIndex = String(i);
      observer.observe(el);
      observed.push(el);
    }

    return () => observer.disconnect();
  }, [files.length, reviewId, scrollRootRef]);

  const jumpTo = (index: number) => {
    document
      .getElementById(`file-${index}`)
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
