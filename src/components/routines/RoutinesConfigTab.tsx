import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { confirmDialog } from "../../lib/confirm";
import type { Routine, RoutineContextKind, RoutineInput } from "../../lib/types";

const PERMISSION_MODES = ["bypassPermissions", "acceptEdits", "auto", "dontAsk", "default", "plan"];
const CONTEXTS: RoutineContextKind[] = ["pr", "review", "issue", "board"];

const BLANK: RoutineInput = {
  id: null,
  name: "",
  description: "",
  context_type: "pr",
  prompt_template: "",
  system_prompt: "",
  system_prompt_mode: "append",
  needs_worktree: false,
  permission_mode: "bypassPermissions",
  allowed_tools: [],
  extra_args: [],
  model: "",
  timeout_min: 60,
  enabled: true,
};

const toInput = (r: Routine): RoutineInput => ({ ...r });
const lines = (s: string) => s.split("\n").map((l) => l.trim()).filter(Boolean);

/** Sessions → Config: routine definitions (CRUD) and the concurrency cap. */
export function RoutinesConfigTab() {
  const queryClient = useQueryClient();
  const routines = useQuery({ queryKey: ["routines"], queryFn: api.routineList });
  const cap = useQuery({ queryKey: ["routine-cap"], queryFn: api.routineMaxConcurrency });
  const [draft, setDraft] = useState<RoutineInput | null>(null);
  const [builtin, setBuiltin] = useState(false);
  // Raw text of the list fields, so typing a newline isn't swallowed by the parse.
  const [toolsText, setToolsText] = useState("");
  const [argsText, setArgsText] = useState("");
  const load = (r: RoutineInput, isBuiltin: boolean) => {
    setDraft(r);
    setBuiltin(isBuiltin);
    setToolsText(r.allowed_tools.join("\n"));
    setArgsText(r.extra_args.join("\n"));
  };

  const list = routines.data ?? [];
  useEffect(() => {
    if (!draft && list.length > 0) {
      load(toInput(list[0]), list[0].builtin);
    }
  }, [list, draft]);

  const refresh = () => queryClient.invalidateQueries({ queryKey: ["routines"] });
  const save = useMutation({
    mutationFn: api.routineSave,
    onSuccess: (r) => {
      refresh();
      load(toInput(r), r.builtin);
      toast.success("Routine saved");
    },
    onError: (e) => toast.error(String(e)),
  });
  const remove = useMutation({
    mutationFn: api.routineDelete,
    onSuccess: () => {
      refresh();
      setDraft(null);
    },
    onError: (e) => toast.error(String(e)),
  });
  const reset = useMutation({
    mutationFn: api.routineReset,
    onSuccess: (r) => {
      refresh();
      load(toInput(r), r.builtin);
      toast.success("Reset to default");
    },
    onError: (e) => toast.error(String(e)),
  });
  const setCap = useMutation({
    mutationFn: api.routineSetMaxConcurrency,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["routine-cap"] }),
    onError: (e) => toast.error(String(e)),
  });

  const patch = (p: Partial<RoutineInput>) => setDraft((d) => (d ? { ...d, ...p } : d));
  const select = (r: Routine) => load(toInput(r), r.builtin);
  const confirmDelete = async () => {
    if (draft?.id == null) return;
    if (await confirmDialog({ title: "Delete routine", message: `Delete “${draft.name}” and its run history?`, confirmLabel: "Delete", danger: true }))
      remove.mutate(draft.id);
  };

  return (
    <div className="routines-config">
      <div className="routines-config-list">
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => load({ ...BLANK }, false)}
        >
          + New routine
        </button>
        {list.map((r) => (
          <button
            key={r.id}
            type="button"
            className={`routine-run-row${draft?.id === r.id ? " selected" : ""}`}
            onClick={() => select(r)}
          >
            <span className="routine-run-title">
              <strong>{r.name}</strong> <span className="badge">{r.context_type}</span>
              {r.builtin && <span className="badge">built-in</span>}
              {!r.enabled && <span className="badge">off</span>}
            </span>
          </button>
        ))}
        <label className="routines-cap">
          Max concurrent runs
          <input
            className="input"
            type="number"
            min={1}
            max={16}
            value={cap.data ?? 4}
            onChange={(e) => setCap.mutate(Number(e.target.value) || 1)}
          />
        </label>
      </div>

      {draft && (
        <form
          className="routines-config-form"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate({ ...draft, allowed_tools: lines(toolsText), extra_args: lines(argsText) });
          }}
        >
          <label>
            Name
            <input className="input" value={draft.name} onChange={(e) => patch({ name: e.target.value })} required />
          </label>
          <label>
            Description
            <input className="input" value={draft.description} onChange={(e) => patch({ description: e.target.value })} />
          </label>
          <div className="routines-config-row">
            <label>
              Context
              <select
                className="input"
                value={draft.context_type}
                disabled={builtin}
                onChange={(e) => patch({ context_type: e.target.value as RoutineContextKind })}
              >
                {CONTEXTS.map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Permissions
              <select className="input" value={draft.permission_mode} onChange={(e) => patch({ permission_mode: e.target.value })}>
                {PERMISSION_MODES.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Timeout (min)
              <input
                className="input"
                type="number"
                min={1}
                max={1440}
                value={draft.timeout_min}
                onChange={(e) => patch({ timeout_min: Number(e.target.value) || 1 })}
              />
            </label>
            <label>
              Model
              <input className="input" placeholder="default" value={draft.model} onChange={(e) => patch({ model: e.target.value })} />
            </label>
          </div>
          <label>
            Prompt <span className="muted">— placeholders: {"{{repo}} {{number}} {{url}} {{title}} {{head_ref}} {{base_ref}} {{worktree}} {{review_id}}"}</span>
            <textarea
              className="input mono"
              rows={12}
              value={draft.prompt_template}
              onChange={(e) => patch({ prompt_template: e.target.value })}
              required
            />
          </label>
          <label>
            System prompt
            <textarea
              className="input mono"
              rows={24}
              value={draft.system_prompt}
              onChange={(e) => patch({ system_prompt: e.target.value })}
            />
          </label>
          <div className="routines-config-row">
            <label>
              System prompt mode
              <select
                className="input"
                value={draft.system_prompt_mode}
                onChange={(e) => patch({ system_prompt_mode: e.target.value as "append" | "replace" })}
              >
                <option value="append">Append to Claude Code's prompt</option>
                <option value="replace">Replace it</option>
              </select>
            </label>
            <label className="routines-check">
              <input type="checkbox" checked={draft.needs_worktree} onChange={(e) => patch({ needs_worktree: e.target.checked })} />
              Run in the PR worktree
            </label>
            <label className="routines-check">
              <input type="checkbox" checked={draft.enabled} onChange={(e) => patch({ enabled: e.target.checked })} />
              Enabled
            </label>
          </div>
          <div className="routines-config-row">
            <label>
              Allowed tools <span className="muted">(one per line, empty = unrestricted)</span>
              <textarea
                className="input mono"
                rows={12}
                value={toolsText}
                onChange={(e) => setToolsText(e.target.value)}
              />
            </label>
            <label>
              Extra claude args <span className="muted">(one per line)</span>
              <textarea
                className="input mono"
                rows={12}
                value={argsText}
                onChange={(e) => setArgsText(e.target.value)}
              />
            </label>
          </div>
          <div className="modal-actions">
            {draft.id != null && builtin && (
              <button type="button" disabled={reset.isPending} onClick={() => draft.id != null && reset.mutate(draft.id)}>
                Reset to default
              </button>
            )}
            {draft.id != null && !builtin && (
              <button type="button" onClick={confirmDelete}>
                Delete
              </button>
            )}
            <button type="submit" className="btn-primary" disabled={save.isPending}>
              {save.isPending ? "Saving…" : "Save"}
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
