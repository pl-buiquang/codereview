import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import type { CiCheck } from "../lib/types";
import { Icon } from "./icons";
import "./CiChecksPopover.css";

export interface PrRef {
  owner: string;
  name: string;
  number: number;
}

const MARGIN = 8;

function duration(secs: number | null): string | null {
  if (secs == null) return null;
  if (secs < 60) return `${secs}s`;
  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
}

function open(url: string | null) {
  if (url) void api.openUrl(url);
}

function CheckRow({ check, onRerun }: { check: CiCheck; onRerun?: (jobId: number) => void }) {
  const dur = duration(check.durationSecs);
  const detail = [
    ...check.failedSteps.map((s) => `Failed step: ${s}`),
    ...check.annotations.map((a) => (a.path && a.path !== ".github" ? `${a.path}: ${a.message}` : a.message)),
  ];
  return (
    <li className={`ci-check ci-${check.bucket}`}>
      <span className="ci-check-mark">
        <Icon name={check.bucket === "failed" ? "x" : check.bucket === "passed" ? "check" : "dot"} size={12} />
      </span>
      <div className="ci-check-main">
        <div className="ci-check-line">
          {check.url ? (
            <a
              href={check.url}
              onClick={(e) => {
                e.preventDefault();
                open(check.url);
              }}
            >
              {check.name}
            </a>
          ) : (
            <span>{check.name}</span>
          )}
          {check.required && <span className="ci-check-required">Required</span>}
          {dur && <span className="ci-check-dur">{dur}</span>}
          {onRerun && check.bucket === "failed" && check.jobId != null && (
            <button className="ci-rerun" title="Rerun this job" onClick={() => onRerun(check.jobId!)}>
              <Icon name="refresh" size={11} />
            </button>
          )}
        </div>
        {detail.map((d, i) => (
          <div key={i} className="ci-check-detail" title={d}>
            {d}
          </div>
        ))}
      </div>
    </li>
  );
}

/** Failed/pending checks grouped by workflow (apps and statuses under "Other"). */
function groupByWorkflow(checks: CiCheck[]): [string, CiCheck[]][] {
  const groups = new Map<string, CiCheck[]>();
  for (const c of checks) {
    const key = c.workflow ?? "Other checks";
    groups.set(key, [...(groups.get(key) ?? []), c]);
  }
  return [...groups];
}

export function CiChecksPopover({
  pr,
  anchor,
  onClose,
}: {
  pr: PrRef;
  anchor: DOMRect;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const [showPassed, setShowPassed] = useState(false);
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["pr-checks", pr.owner, pr.name, pr.number],
    queryFn: () => api.prChecks(pr.owner, pr.name, pr.number),
    staleTime: 0,
    refetchInterval: (q) => (q.state.data?.checks.some((c) => c.bucket === "pending") ? 15_000 : false),
  });

  const rerun = useMutation({
    mutationFn: (req: { runIds: number[]; failedOnly: boolean } | { jobId: number }) =>
      "jobId" in req
        ? api.rerunCiJob(pr.owner, pr.name, req.jobId)
        : api.rerunCiRuns(pr.owner, pr.name, req.runIds, req.failedOnly),
    // GitHub takes a moment to flip the checks back to queued.
    onSuccess: () => setTimeout(() => queryClient.invalidateQueries({ queryKey: ["pr-checks"] }), 2000),
  });

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  // Anchor under the badge, flipped/clamped so it stays inside the window as content loads.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const left = Math.max(MARGIN, Math.min(anchor.left, window.innerWidth - width - MARGIN));
    const below = anchor.bottom + 4;
    const top =
      below + height <= window.innerHeight - MARGIN ? below : Math.max(MARGIN, anchor.top - height - 4);
    setPos({ top, left });
  }, [anchor, query.data, showPassed]);

  const data = query.data;
  const active = data?.checks.filter((c) => c.bucket !== "passed") ?? [];
  const passed = data?.checks.filter((c) => c.bucket === "passed") ?? [];
  const failedCount = data?.checks.filter((c) => c.bucket === "failed").length ?? 0;
  const pendingCount = data?.checks.filter((c) => c.bucket === "pending").length ?? 0;
  const distinct = (ids: (number | null)[]) => [...new Set(ids.filter((id): id is number => id != null))];
  const failedRuns = distinct(data?.checks.filter((c) => c.bucket === "failed").map((c) => c.runId) ?? []);
  // A run still in progress can't be rerun.
  const busyRuns = new Set(data?.checks.filter((c) => c.bucket === "pending").map((c) => c.runId));
  const allRuns = distinct(data?.checks.map((c) => c.runId) ?? []).filter((id) => !busyRuns.has(id));
  const summary = [
    failedCount && `${failedCount} failing`,
    pendingCount && `${pendingCount} pending`,
    `${passed.length} successful`,
  ]
    .filter(Boolean)
    .join(", ");

  return createPortal(
    <div
      className="ci-popover"
      ref={ref}
      role="dialog"
      aria-label="CI checks"
      style={{ top: pos?.top ?? 0, left: pos?.left ?? 0, visibility: pos ? "visible" : "hidden" }}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="ci-popover-head">
        <span className="ci-popover-title">{data ? summary : "Checks"}</span>
        {data && (
          <button className="ci-popover-link" onClick={() => open(`${data.prUrl}/checks`)}>
            Open on GitHub <Icon name="ext" size={11} />
          </button>
        )}
      </div>
      {data && (failedRuns.length > 0 || allRuns.length > 0) && (
        <div className="ci-popover-actions">
          <button
            className="ci-action"
            disabled={rerun.isPending || failedRuns.length === 0}
            onClick={() => rerun.mutate({ runIds: failedRuns, failedOnly: true })}
          >
            <Icon name="refresh" size={11} /> Rerun failed
          </button>
          <button
            className="ci-action"
            disabled={rerun.isPending || allRuns.length === 0}
            onClick={() => rerun.mutate({ runIds: allRuns, failedOnly: false })}
          >
            <Icon name="refresh" size={11} /> Rerun all
          </button>
          {rerun.isPending && <span className="ci-popover-note">Requesting…</span>}
          {rerun.isSuccess && <span className="ci-popover-note">Rerun requested</span>}
          {rerun.isError && <span className="ci-popover-note ci-popover-err">{String(rerun.error)}</span>}
        </div>
      )}
      <div className="ci-popover-body scroll">
        {query.isLoading && <div className="ci-popover-note">Loading checks…</div>}
        {query.isError && <div className="ci-popover-note ci-popover-err">{String(query.error)}</div>}
        {data && data.checks.length === 0 && <div className="ci-popover-note">No checks reported.</div>}
        {groupByWorkflow(active).map(([wf, checks]) => (
          <section key={wf} className="ci-group">
            <div className="ci-group-title">{wf}</div>
            <ul className="ci-check-list">
              {checks.map((c, i) => (
                <CheckRow
                  key={`${c.name}-${i}`}
                  check={c}
                  onRerun={rerun.isPending ? undefined : (jobId) => rerun.mutate({ jobId })}
                />
              ))}
            </ul>
          </section>
        ))}
        {passed.length > 0 && (
          <section className="ci-group">
            <button className="ci-group-toggle" onClick={() => setShowPassed((v) => !v)}>
              {showPassed ? "▾" : "▸"} {passed.length} successful
            </button>
            {showPassed && (
              <ul className="ci-check-list">
                {passed.map((c, i) => (
                  <CheckRow key={`${c.name}-${i}`} check={c} />
                ))}
              </ul>
            )}
          </section>
        )}
      </div>
    </div>,
    document.body,
  );
}
