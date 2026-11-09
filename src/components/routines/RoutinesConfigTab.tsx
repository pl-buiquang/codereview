import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../../lib/api";
import { toast } from "../../lib/toast";
import { confirmDialog } from "../../lib/confirm";
import { ACCESS_LABEL, claudeOptions, resolvedHarness, withClaudeOptions } from "../../lib/routines";
import type {
  AiProfile,
  Routine,
  RoutineAccess,
  RoutineContextKind,
  RoutineInput,
  RoutineSettings,
} from "../../lib/types";

const PERMISSION_MODES = ["bypassPermissions", "acceptEdits", "auto", "dontAsk", "default", "plan"];
const CONTEXTS: RoutineContextKind[] = ["pr", "review", "issue", "board"];

const BLANK: RoutineInput = {
  id: null,
  name: "",
  description: "",
  context_type: "pr",
  prompt_template: "",
  system_prompt: "",
  needs_worktree: false,
  access: "full",
  harness_overrides: {},
  profile_id: null,
  model: "",
  timeout_min: 60,
  enabled: true,
};

const toInput = (r: Routine): RoutineInput => ({ ...r });
const ACCESS_LEVELS: RoutineAccess[] = ["read_only", "edit", "full"];

/** Profile picker; "Default profile" is a null profile id. */
function ProfileSelect({
  profiles,
  value,
  onChange,
}: {
  profiles: AiProfile[];
  value: number | null;
  onChange: (id: number | null) => void;
}) {
  const def = profiles.find((p) => p.is_default);
  return (
    <select
      className="input"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value === "" ? null : Number(e.target.value))}
    >
      <option value="">Default profile{def ? ` (${def.name})` : ""}</option>
      {profiles
        .filter((p) => !p.is_default)
        .map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
    </select>
  );
}

/** A system routine: a Rust handler owns its prompt and tools, so only these four fields are editable. */
export function SystemRoutineCard({
  routine,
  profiles,
  saving,
  onSave,
}: {
  routine: Routine;
  profiles: AiProfile[];
  saving: boolean;
  onSave: (input: RoutineInput) => void;
}) {
  const [draft, setDraft] = useState<RoutineInput>(toInput(routine));
  useEffect(() => setDraft(toInput(routine)), [routine]);
  const dirty =
    draft.profile_id !== routine.profile_id ||
    draft.model !== routine.model ||
    draft.timeout_min !== routine.timeout_min ||
    draft.enabled !== routine.enabled;
  return (
    <div className="routine-system-card" data-testid={`system-routine-${routine.key}`}>
      <div className="routine-run-title">
        <strong>{routine.name}</strong> <span className="badge">{routine.context_type}</span>
        {!routine.enabled && <span className="badge">off</span>}
      </div>
      <p className="muted routine-system-desc">{routine.description}</p>
      <div className="routine-system-controls">
        <label>
          Profile
          <ProfileSelect profiles={profiles} value={draft.profile_id} onChange={(profile_id) => setDraft({ ...draft, profile_id })} />
        </label>
        <label>
          Model
          <input
            className="input"
            placeholder="profile default"
            value={draft.model}
            onChange={(e) => setDraft({ ...draft, model: e.target.value })}
          />
        </label>
        <label>
          Timeout (min)
          <input
            className="input"
            type="number"
            min={1}
            max={1440}
            value={draft.timeout_min}
            onChange={(e) => setDraft({ ...draft, timeout_min: Number(e.target.value) || 1 })}
          />
        </label>
        <label className="routines-check">
          <input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />
          Enabled
        </label>
        <button type="button" className="btn btn-sm" disabled={!dirty || saving} onClick={() => onSave(draft)}>
          Save
        </button>
      </div>
    </div>
  );
}
const lines = (s: string) => s.split("\n").map((l) => l.trim()).filter(Boolean);

/** Sessions → Config: routine definitions (CRUD) and the concurrency cap. */
/** Where the Plan routine writes: saved when a field loses focus, so typing doesn't write per key. */
function PlanSettings() {
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["routine-settings"], queryFn: api.routineSettings });
  const [draft, setDraft] = useState<RoutineSettings | null>(null);
  useEffect(() => {
    if (settings.data) setDraft(settings.data);
  }, [settings.data]);
  const save = useMutation({
    mutationFn: api.routineSetSettings,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["routine-settings"] }),
    onError: (e) => toast.error(String(e)),
  });
  if (!draft) return null;
  const commit = () => {
    if (JSON.stringify(draft) !== JSON.stringify(settings.data)) save.mutate(draft);
  };
  return (
    <>
      <label className="routines-cap" title="Plans of repos without a tracker root (CLAUDE.local.md) go here. Exposed to routines as {{plans_dir}}.">
        Plans directory
        <input
          className="input"
          value={draft.plans_dir}
          onChange={(e) => setDraft({ ...draft, plans_dir: e.target.value })}
          onBlur={commit}
        />
      </label>
      <label className="routines-cap" title="The Plan issue routine may write anywhere under this folder, and nowhere else outside the repository.">
        Notes root
        <input
          className="input"
          value={draft.notes_root}
          onChange={(e) => setDraft({ ...draft, notes_root: e.target.value })}
          onBlur={commit}
        />
      </label>
    </>
  );
}

export function RoutinesConfigTab() {
  const queryClient = useQueryClient();
  const routines = useQuery({ queryKey: ["routines"], queryFn: api.routineList });
  const profiles = useQuery({ queryKey: ["ai-profiles"], queryFn: api.aiProfileList });
  const cap = useQuery({ queryKey: ["routine-cap"], queryFn: api.routineMaxConcurrency });
  const [draft, setDraft] = useState<RoutineInput | null>(null);
  const [builtin, setBuiltin] = useState(false);
  // Raw text of the list fields, so typing a newline isn't swallowed by the parse.
  const [toolsText, setToolsText] = useState("");
  const [argsText, setArgsText] = useState("");
  const load = (r: RoutineInput, isBuiltin: boolean) => {
    setDraft(r);
    setBuiltin(isBuiltin);
    setToolsText(claudeOptions(r.harness_overrides).allowed_tools.join("\n"));
    setArgsText(claudeOptions(r.harness_overrides).extra_args.join("\n"));
  };

  const list = routines.data ?? [];
  const plain = list.filter((r) => r.kind === "plain");
  const system = list.filter((r) => r.kind === "system");
  const profileList = profiles.data ?? [];
  useEffect(() => {
    if (!draft && plain.length > 0) {
      load(toInput(plain[0]), plain[0].builtin);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routines.data, draft]);

  const refresh = () => queryClient.invalidateQueries({ queryKey: ["routines"] });
  const save = useMutation({
    mutationFn: api.routineSave,
    onSuccess: (r) => {
      refresh();
      load(toInput(r), r.builtin);
      toast.info("Routine saved");
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
  const saveSystem = useMutation({
    mutationFn: api.routineSave,
    onSuccess: () => {
      refresh();
      toast.info("Routine saved");
    },
    onError: (e) => toast.error(String(e)),
  });
  const reset = useMutation({
    mutationFn: api.routineReset,
    onSuccess: (r) => {
      refresh();
      load(toInput(r), r.builtin);
      toast.info("Reset to default");
    },
    onError: (e) => toast.error(String(e)),
  });
  const setCap = useMutation({
    mutationFn: api.routineSetMaxConcurrency,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["routine-cap"] }),
    onError: (e) => toast.error(String(e)),
  });

  const isClaude = resolvedHarness(draft?.profile_id ?? null, profileList) === "claude";
  const claude = claudeOptions(draft?.harness_overrides ?? {});
  const patchClaude = (p: Parameters<typeof withClaudeOptions>[1]) =>
    setDraft((d) => (d ? { ...d, harness_overrides: withClaudeOptions(d.harness_overrides, p) } : d));
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
        <h4 className="routines-config-heading">Routines</h4>
        <button
          type="button"
          className="btn btn-sm"
          onClick={() => load({ ...BLANK }, false)}
        >
          + New routine
        </button>
        {plain.map((r) => (
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
        <h4 className="routines-config-heading">System routines</h4>
        <p className="muted routines-config-note">Run by the app itself, locally. Only the profile, model, timeout and on/off are editable.</p>
        {system.map((r) => (
          <SystemRoutineCard key={r.id} routine={r} profiles={profileList} saving={saveSystem.isPending} onSave={saveSystem.mutate} />
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
        <PlanSettings />
      </div>

      {draft && (
        <form
          className="routines-config-form"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate(
              isClaude
                ? {
                    ...draft,
                    harness_overrides: withClaudeOptions(draft.harness_overrides, {
                      allowed_tools: lines(toolsText),
                      extra_args: lines(argsText),
                    }),
                  }
                : draft,
            );
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
              Access
              <select className="input" value={draft.access} onChange={(e) => patch({ access: e.target.value as RoutineAccess })}>
                {ACCESS_LEVELS.map((a) => (
                  <option key={a} value={a}>
                    {ACCESS_LABEL[a]}
                  </option>
                ))}
              </select>
            </label>
            <label>
              Profile
              <ProfileSelect profiles={profileList} value={draft.profile_id} onChange={(profile_id) => patch({ profile_id })} />
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
              <input className="input" placeholder="profile default" value={draft.model} onChange={(e) => patch({ model: e.target.value })} />
            </label>
          </div>
          <label>
            Prompt <span className="muted">— placeholders: {"{{repo}} {{number}} {{url}} {{title}} {{head_ref}} {{base_ref}} {{worktree}} {{review_id}} {{plans_dir}}"}</span>
            <textarea
              className="input mono"
              rows={4}
              value={draft.prompt_template}
              onChange={(e) => patch({ prompt_template: e.target.value })}
              required
            />
          </label>
          <label>
            System prompt
            <textarea
              className="input mono"
              rows={16}
              value={draft.system_prompt}
              onChange={(e) => patch({ system_prompt: e.target.value })}
            />
          </label>
          <div className="routines-config-row">
            <label className="routines-check">
              <input type="checkbox" checked={draft.needs_worktree} onChange={(e) => patch({ needs_worktree: e.target.checked })} />
              Run in the PR worktree
            </label>
            <label className="routines-check">
              <input type="checkbox" checked={draft.enabled} onChange={(e) => patch({ enabled: e.target.checked })} />
              Enabled
            </label>
          </div>
          {isClaude && (
            <fieldset className="routines-claude-options">
              <legend>Claude Code options</legend>
              <div className="routines-config-row">
                <label>
                  Permission mode
                  <select className="input" value={claude.permission_mode} onChange={(e) => patchClaude({ permission_mode: e.target.value })}>
                    <option value="">From access</option>
                    {PERMISSION_MODES.map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  System prompt mode
                  <select
                    className="input"
                    value={claude.system_prompt_mode}
                    onChange={(e) => patchClaude({ system_prompt_mode: e.target.value as "append" | "replace" })}
                  >
                    <option value="append">Append to Claude Code's prompt</option>
                    <option value="replace">Replace it</option>
                  </select>
                </label>
              </div>
              <div className="routines-config-row">
                <label>
                  Allowed tools <span className="muted">(one per line, empty = unrestricted)</span>
                  <textarea className="input mono" rows={4} value={toolsText} onChange={(e) => setToolsText(e.target.value)} />
                </label>
                <label>
                  Extra args <span className="muted">(one per line)</span>
                  <textarea className="input mono" rows={4} value={argsText} onChange={(e) => setArgsText(e.target.value)} />
                </label>
              </div>
            </fieldset>
          )}
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
