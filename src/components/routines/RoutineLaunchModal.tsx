import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { formatDuration, isActiveRun } from "../../lib/routines";
import { parseRepoBasePaths, useAiDisabledReason, useSettingsStore } from "../../lib/settings";
import { useUIStore } from "../../store";
import type { RoutineContextKind, RoutineOption } from "../../lib/types";
import { RunStatusBadge } from "./RunStatusBadge";

/** "Routines…" modal: pick a routine that fits this PR / review / issue and run it. A routine that
 *  is already queued or running on this context shows that run instead of a second launch. */
export function RoutineLaunchModal({
  kind,
  id,
  title,
  onClose,
}: {
  kind: RoutineContextKind;
  id: string;
  /** Shown in the header, e.g. "owner/name#12". */
  title: string;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const repoBasePaths = useSettingsStore((s) => s.repoBasePaths);
  const showRoutineRun = useUIStore((s) => s.showRoutineRun);
  const context = { kind, id, base_paths: parseRepoBasePaths(repoBasePaths) };
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const [model, setModel] = useState("");

  const options = useQuery({
    queryKey: ["routine-options", kind, id],
    queryFn: () => api.routineListForContext(context),
    refetchInterval: 5_000,
  });

  const aiOff = useAiDisabledReason();
  const launch = useMutation({
    mutationFn: (routineId: number) => api.routineLaunch(routineId, context, note, model),
    onSuccess: (run) => {
      queryClient.invalidateQueries({ queryKey: ["routine-runs"] });
      queryClient.invalidateQueries({ queryKey: ["routine-options"] });
      toast.info(`${run.routine_name} ${run.status === "queued" ? "queued" : "started"}`);
      onClose();
    },
    onError: (e) => toast.error(String(e)),
  });
  const stop = useMutation({
    mutationFn: api.routineRunStop,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["routine-options"] }),
    onError: (e) => toast.error(String(e)),
  });

  const list: RoutineOption[] = options.data ?? [];
  const selected = list.find((o) => o.routine.id === selectedId && !o.active_run) ?? null;

  const openLog = (runId: number) => {
    showRoutineRun(runId);
    onClose();
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal routine-launch-modal"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => e.key === "Escape" && onClose()}
        role="dialog"
        aria-modal="true"
        aria-label="Run a routine"
      >
        <div className="modal-header">
          <h3>Run a routine</h3>
          <span className="muted">{title}</span>
        </div>

        <div className="routine-launch-list">
          {options.isLoading && <p className="muted">Loading…</p>}
          {options.isError && <p className="form-warning">{String(options.error)}</p>}
          {!options.isLoading && list.length === 0 && (
            <p className="muted">No routine fits this context. Create one in Sessions → Config.</p>
          )}
          {list.map(({ routine, active_run }) => (
            <div
              key={routine.id}
              className={`routine-option${selectedId === routine.id && !active_run ? " selected" : ""}${active_run ? " active" : ""}`}
            >
              <button
                type="button"
                className="routine-option-main"
                disabled={!!active_run}
                aria-pressed={selectedId === routine.id}
                onClick={() => setSelectedId(routine.id)}
              >
                <strong>{routine.name}</strong>
                {routine.needs_worktree && <span className="badge">worktree</span>}
                {routine.description && <span className="muted routine-option-desc">{routine.description}</span>}
              </button>
              {active_run && (
                <div className="routine-option-run">
                  <RunStatusBadge status={active_run.status} />
                  <span className="muted">{formatDuration(active_run.started_at ?? active_run.queued_at, null)}</span>
                  <button type="button" className="btn btn-sm" onClick={() => openLog(active_run.id)}>
                    Open log
                  </button>
                  {isActiveRun(active_run.status) && (
                    <button
                      type="button"
                      className="btn btn-sm btn-ghost"
                      disabled={stop.isPending}
                      onClick={() => stop.mutate(active_run.id)}
                    >
                      Stop
                    </button>
                  )}
                </div>
              )}
            </div>
          ))}
        </div>

        {selected && (
          <div className="routine-launch-opts">
            <textarea
              className="input"
              rows={3}
              placeholder="Extra instructions for this run (optional)"
              value={note}
              onChange={(e) => setNote(e.target.value)}
            />
            <input
              className="input"
              placeholder={selected.routine.model ? `Model (default: ${selected.routine.model})` : "Model override (optional)"}
              value={model}
              onChange={(e) => setModel(e.target.value)}
            />
          </div>
        )}

        <div className="modal-actions">
          <button type="button" onClick={onClose}>
            Close
          </button>
          <button
            type="button"
            className="btn-primary"
            disabled={!selected || launch.isPending || aiOff != null}
            title={aiOff ?? undefined}
            onClick={() => selected && launch.mutate(selected.routine.id)}
          >
            {launch.isPending ? "Starting…" : "Run"}
          </button>
        </div>
      </div>
    </div>
  );
}
