import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { timeAgo } from "../../lib/timeAgo";
import { formatDuration, isActiveRun, STATUS_LABEL } from "../../lib/routines";
import { useAiDisabledReason, useSettingsStore } from "../../lib/settings";
import { useUIStore } from "../../store";
import type { RoutineRun, RoutineRunStatus } from "../../lib/types";
import { Icon } from "../icons";
import { resumeCommand } from "../SessionsView";
import { RunStatusBadge } from "./RunStatusBadge";
import { RunTranscript } from "./RunTranscript";

const ALL_STATUSES = Object.keys(STATUS_LABEL) as RoutineRunStatus[];

/** Sessions → Routines: every run, live and past, with its transcript. */
export function RoutinesRunsTab() {
  const queryClient = useQueryClient();
  const focusRunId = useUIStore((s) => s.routineRunFocus);
  const clearFocus = useUIStore((s) => s.clearRoutineRunFocus);
  const showBoardCard = useUIStore((s) => s.showBoardCard);
  const openReview = useUIStore((s) => s.openReview);
  const [status, setStatus] = useState<RoutineRunStatus | "">("");
  const [routineId, setRoutineId] = useState<number | "">("");
  const [selectedId, setSelectedId] = useState<number | null>(null);

  useEffect(() => {
    if (focusRunId != null) {
      setSelectedId(focusRunId);
      clearFocus();
    }
  }, [focusRunId, clearFocus]);

  const runs = useQuery({
    queryKey: ["routine-runs", "list", status, routineId],
    queryFn: () =>
      api.routineRunList({
        statuses: status ? [status] : undefined,
        routine_id: routineId === "" ? undefined : routineId,
        limit: 200,
      }),
    refetchInterval: 10_000,
  });
  const routines = useQuery({ queryKey: ["routines"], queryFn: api.routineList });

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["routine-runs"] });
  const stop = useMutation({ mutationFn: api.routineRunStop, onSuccess: invalidate, onError: (e) => toast.error(String(e)) });
  const aiOff = useAiDisabledReason();
  const defaultTerminal = useSettingsStore((s) => s.defaultTerminal);
  const tmuxTarget = useSettingsStore((s) => s.tmuxTarget);
  const tmuxSessionName = useSettingsStore((s) => s.tmuxSessionName);
  const resume = useMutation({
    mutationFn: (r: { session_id: string; cwd: string }) =>
      api.resumeClaudeSession(r.session_id, r.cwd, null, defaultTerminal, tmuxTarget, tmuxSessionName),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["claude-sessions"] }),
    onError: (e) => toast.error(`Could not resume session:\n${String(e)}`),
  });
  const copyCommand = (command: string) =>
    navigator.clipboard.writeText(command).then(
      () => toast.info("Resume command copied to clipboard"),
      (e) => toast.error(`Could not copy: ${String(e)}`),
    );
  const relaunch = useMutation({
    mutationFn: api.routineRunRelaunch,
    onSuccess: (run) => {
      invalidate();
      setSelectedId(run.id);
    },
    onError: (e) => toast.error(String(e)),
  });

  const list = runs.data ?? [];
  const selected = list.find((r) => r.id === selectedId) ?? null;

  const openContext = (run: RoutineRun) => {
    if (run.context.kind === "review") openReview(Number(run.context.id));
    else if (run.context.id) showBoardCard(run.context.id);
  };

  return (
    <div className="routines-runs">
      <div className="routines-runs-list">
        <div className="routines-filters">
          <select className="input" value={status} onChange={(e) => setStatus(e.target.value as RoutineRunStatus | "")} aria-label="Status">
            <option value="">All statuses</option>
            {ALL_STATUSES.map((s) => (
              <option key={s} value={s}>
                {STATUS_LABEL[s]}
              </option>
            ))}
          </select>
          <select
            className="input"
            value={routineId}
            onChange={(e) => setRoutineId(e.target.value === "" ? "" : Number(e.target.value))}
            aria-label="Routine"
          >
            <option value="">All routines</option>
            {(routines.data ?? []).map((r) => (
              <option key={r.id} value={r.id}>
                {r.name}
              </option>
            ))}
          </select>
        </div>
        {runs.isLoading && <p className="muted">Loading…</p>}
        {runs.isError && <p className="form-warning">{String(runs.error)}</p>}
        {!runs.isLoading && list.length === 0 && (
          <p className="muted faint">
            No runs yet. Start one from a board card or a review (⋯ → Routines…).
          </p>
        )}
        {list.map((run) => (
          <button
            key={run.id}
            type="button"
            className={`routine-run-row${run.id === selectedId ? " selected" : ""}`}
            onClick={() => setSelectedId(run.id)}
          >
            <span className="routine-run-title">
              <strong>{run.routine_name}</strong>
              <span className="muted"> · {run.context_label}</span>
            </span>
            <span className="routine-run-meta">
              <RunStatusBadge status={run.status} />
              <span className="muted">{timeAgo(run.started_at ?? run.queued_at)}</span>
            </span>
          </button>
        ))}
      </div>

      <div className="routines-run-detail">
        {!selected && <p className="muted faint">Select a run to see its transcript.</p>}
        {selected && (
          <>
            <div className="routine-run-head">
              <div>
                <h2 className="sessions-group-head">
                  {selected.routine_name} <span className="muted">· {selected.context_label}</span>
                </h2>
                <p className="cr-sub">
                  <RunStatusBadge status={selected.status} />{" "}
                  {formatDuration(selected.started_at, selected.ended_at)}
                  {selected.cost_usd > 0 && ` · $${selected.cost_usd.toFixed(2)}`}
                  {selected.num_turns > 0 && ` · ${selected.num_turns} turns`}
                </p>
              </div>
              <div className="routine-run-actions">
                {selected.context.id || selected.context.kind === "board" ? (
                  selected.context.kind !== "board" && (
                    <button type="button" className="btn btn-sm" onClick={() => openContext(selected)}>
                      Open {selected.context.kind === "review" ? "review" : "card"}
                    </button>
                  )
                ) : null}
                {isActiveRun(selected.status) ? (
                  <button type="button" className="btn btn-sm" disabled={stop.isPending} onClick={() => stop.mutate(selected.id)}>
                    Stop
                  </button>
                ) : (
                  <button type="button" className="btn btn-sm" disabled={relaunch.isPending || aiOff != null}
                    title={aiOff ?? undefined}
                    onClick={() => relaunch.mutate(selected.id)}
                  >
                    Run again
                  </button>
                )}
                {!isActiveRun(selected.status) && selected.session_id && selected.cwd && (
                  <div className="btn-split">
                    <button
                      type="button"
                      className="btn btn-sm"
                      disabled={resume.isPending}
                      title={`Open a new terminal running: ${resumeCommand({ cwd: selected.cwd, session_id: selected.session_id })}`}
                      onClick={() => resume.mutate({ session_id: selected.session_id!, cwd: selected.cwd! })}
                    >
                      <Icon name="terminal" size={13} /> Resume
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm"
                      title={`Copy: ${resumeCommand({ cwd: selected.cwd, session_id: selected.session_id })}`}
                      aria-label="Copy resume command"
                      onClick={() => copyCommand(resumeCommand({ cwd: selected.cwd!, session_id: selected.session_id! }))}
                    >
                      <Icon name="copy" size={12} />
                    </button>
                  </div>
                )}
              </div>
            </div>
            {selected.note && <p className="muted">Note: {selected.note}</p>}
            {selected.cwd && <p className="muted routine-cwd">{selected.cwd}</p>}
            {selected.error && <p className="form-warning">{selected.error}</p>}
            {selected.status === "needs_attention" && selected.result_text && (
              <p className="form-warning">{selected.result_text.split("\n").find((l) => l.trim().startsWith("NEEDS_ATTENTION:"))}</p>
            )}
            <RunTranscript runId={selected.id} active={isActiveRun(selected.status)} />
          </>
        )}
      </div>
    </div>
  );
}
