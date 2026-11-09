import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { toast } from "../lib/toast";
import { confirmDialog } from "../lib/confirm";
import { CODEX_SANDBOXES, codexSettings, FEATURE_ROWS, profileEditorFields, withCodexSettings } from "../lib/aiProfiles";
import { ModelPicker } from "./ModelPicker";
import { AiHostsSection } from "./SettingsAiHosts";
import { hostName, hostsQuery, readableRemoteError, useHostStates, connLabel } from "../lib/hosts";
import type { AiProfile, AiProfileInput, FeatureProfiles, HarnessInfo } from "../lib/types";

const BLANK: AiProfileInput = { id: null, name: "", harness: "claude", model: "", settings: {}, host_id: null, is_default: false };

/** Settings → AI: harness availability, profiles, and which profile each feature uses. */
export function AiProfilesSection() {
  const queryClient = useQueryClient();
  const profiles = useQuery({ queryKey: ["ai-profiles"], queryFn: api.aiProfileList });
  const harnesses = useQuery({ queryKey: ["ai-harnesses"], queryFn: api.aiHarnesses, staleTime: 60_000 });
  const features = useQuery({ queryKey: ["ai-feature-profiles"], queryFn: api.aiFeatureProfilesGet });
  const hosts = useQuery(hostsQuery());
  const hostStates = useHostStates();
  const [draft, setDraft] = useState<AiProfileInput | null>(null);
  const [settingsText, setSettingsText] = useState("{}");

  // A remote profile edits against what its host offers, not what this machine has.
  const draftHost = draft?.host_id ?? null;
  const remoteHarnesses = useQuery({
    queryKey: ["ai-harnesses-on", draftHost],
    queryFn: () => api.aiHarnessesOn(draftHost as number),
    enabled: draftHost != null,
    retry: false,
    staleTime: 30_000,
  });

  const list = profiles.data ?? [];
  const infos = harnesses.data ?? [];
  const editorInfos = draftHost != null ? (remoteHarnesses.data ?? []) : infos;
  const info = (id: string): HarnessInfo | undefined => infos.find((h) => h.id === id);
  const editorInfo = (id: string): HarnessInfo | undefined => editorInfos.find((h) => h.id === id);

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["ai-profiles"] });
    queryClient.invalidateQueries({ queryKey: ["ai-feature-profiles"] });
    queryClient.invalidateQueries({ queryKey: ["routines"] });
  };
  const save = useMutation({
    mutationFn: api.aiProfileSave,
    onSuccess: () => {
      refresh();
      setDraft(null);
    },
    onError: (e) => toast.error(String(e)),
  });
  const remove = useMutation({
    mutationFn: api.aiProfileDelete,
    onSuccess: refresh,
    onError: (e) => toast.error(String(e)),
  });
  const setFeatures = useMutation({
    mutationFn: api.aiFeatureProfilesSet,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["ai-feature-profiles"] }),
    onError: (e) => toast.error(String(e)),
  });

  const edit = (p: AiProfileInput) => {
    setDraft(p);
    setSettingsText(JSON.stringify(p.settings, null, 2));
  };
  const submit = () => {
    if (!draft) return;
    let settings: Record<string, unknown> = {};
    if (profileEditorFields(editorInfo(draft.harness)).settings) {
      try {
        const parsed = JSON.parse(settingsText || "{}");
        if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("not an object");
        settings = parsed;
      } catch {
        toast.error("Settings must be a JSON object");
        return;
      }
    }
    save.mutate({ ...draft, settings });
  };
  const confirmDelete = async (p: AiProfile) => {
    if (
      await confirmDialog({
        title: "Delete profile",
        message: `Delete “${p.name}”? Routines and chats using it fall back to the default profile.`,
        confirmLabel: "Delete",
        danger: true,
      })
    )
      remove.mutate(p.id);
  };
  const pick = (key: keyof FeatureProfiles, value: string) =>
    setFeatures.mutate({ ...(features.data ?? { chat: null, summaries: null, theme: null, sessions: null }), [key]: value === "" ? null : Number(value) });

  const fields = draft ? profileEditorFields(editorInfo(draft.harness)) : null;

  /** The settings text as an object, or null while it isn't valid JSON (the Codex controls then wait). */
  const settingsObject = (): Record<string, unknown> | null => {
    try {
      const parsed = JSON.parse(settingsText || "{}");
      return parsed !== null && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : null;
    } catch {
      return null;
    }
  };
  const codex = draft?.harness === "codex" ? settingsObject() : null;
  const patchCodex = (patch: Parameters<typeof withCodexSettings>[1]) => {
    const current = settingsObject();
    if (current) setSettingsText(JSON.stringify(withCodexSettings(current, patch), null, 2));
  };

  return (
    <>
      <section className="settings-group" data-testid="ai-harnesses">
        <h3>Harnesses</h3>
        {infos.length === 0 && <span className="muted">Checking…</span>}
        {infos.map((h) => (
          <div key={h.id} className="settings-row">
            <span>
              <strong>{h.name}</strong>{" "}
              <span className={h.available ? "badge" : "badge badge-danger"}>{h.available ? "found" : "not found"}</span>
            </span>
            <span className="muted">{h.available ? (h.version ?? "version unknown") : "Install it and make sure it is on PATH"}</span>
          </div>
        ))}
      </section>

      <AiHostsSection />

      <section className="settings-group" data-testid="ai-profiles">
        <h3>Profiles</h3>
        <span className="settings-hint muted">
          A profile is a harness plus a model. A routine uses its own profile, then the one mapped to its feature
          below, then the default. Auth stays inside each CLI; nothing secret is stored here.
        </span>
        {list.map((p) => (
          <div key={p.id} className="settings-row ai-profile-row">
            <span>
              <strong>{p.name}</strong> {p.is_default && <span className="badge">default</span>}
              <span className="muted">
                {" "}
                {info(p.harness)?.name ?? p.harness} · {p.model || "default model"}
                {p.host_id != null && ` · on ${hostName(hosts.data, p.host_id)}`}
              </span>
            </span>
            <span className="ai-profile-actions">
              {!p.is_default && p.host_id == null && (
                <button className="btn btn-sm" onClick={() => save.mutate({ ...p, is_default: true })}>
                  Make default
                </button>
              )}
              <button className="btn btn-sm" onClick={() => edit({ ...p })}>
                Edit
              </button>
              {!p.is_default && (
                <button className="btn btn-sm" onClick={() => confirmDelete(p)}>
                  Delete
                </button>
              )}
            </span>
          </div>
        ))}
        {!draft && (
          <button className="btn btn-sm" onClick={() => edit({ ...BLANK })}>
            Add profile
          </button>
        )}
        {draft && fields && (
          <form
            className="settings-row settings-row-stack ai-profile-form"
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
          >
            <label>
              Name
              <input className="input" value={draft.name} required onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
            </label>
            {(hosts.data ?? []).length > 0 && (
              <label>
                Runs on
                <select
                  className="input"
                  value={draft.host_id ?? ""}
                  onChange={(e) => setDraft({ ...draft, host_id: e.target.value === "" ? null : Number(e.target.value), model: "" })}
                >
                  <option value="">This machine</option>
                  {(hosts.data ?? []).map((h) => (
                    <option key={h.id} value={h.id}>
                      {h.name} ({h.enabled ? connLabel(hostStates.get(h.id)) : "disabled"})
                    </option>
                  ))}
                </select>
              </label>
            )}
            {draftHost != null && (
              <span className="settings-hint muted" data-testid="remote-profile-hint">
                {remoteHarnesses.isLoading && `Asking ${hostName(hosts.data, draftHost)} for its harnesses…`}
                {remoteHarnesses.isError && (
                  <span className="form-warning">{readableRemoteError(String(remoteHarnesses.error))}</span>
                )}
                {remoteHarnesses.isSuccess &&
                  "Runs plain routines and one-shot features on that host, with its harnesses and models. Chats, interactive sessions and system routines stay on this machine."}
              </span>
            )}
            <label>
              Harness
              <select
                className="input"
                value={draft.harness}
                onChange={(e) => setDraft({ ...draft, harness: e.target.value, model: "" })}
                disabled={draftHost != null && !remoteHarnesses.isSuccess}
              >
                {editorInfos.map((h) => (
                  <option key={h.id} value={h.id} disabled={draftHost != null && !h.available}>
                    {h.name}
                    {draftHost != null && !h.available ? " (not installed on the host)" : ""}
                  </option>
                ))}
              </select>
            </label>
            {fields.model && (draftHost == null || remoteHarnesses.isSuccess) && (
              <ModelPicker harness={draft.harness} hostId={draftHost} value={draft.model} onChange={(model) => setDraft({ ...draft, model })} />
            )}
            {fields.settings && draft.harness === "codex" && (
              <div className="codex-settings" data-testid="codex-settings">
                <label>
                  Sandbox
                  <select
                    className="input"
                    value={codex ? codexSettings(codex).sandbox : ""}
                    disabled={!codex}
                    onChange={(e) => patchCodex({ sandbox: e.target.value })}
                  >
                    {CODEX_SANDBOXES.map((m) => (
                      <option key={m} value={m}>
                        {m === "" ? "From the run's access level" : `${m} (only if lower)`}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="settings-check">
                  <input
                    type="checkbox"
                    checked={codex ? codexSettings(codex).allowFull : false}
                    disabled={!codex}
                    onChange={(e) => patchCodex({ allowFull: e.target.checked })}
                  />{" "}
                  Allow full access (no sandbox)
                </label>
                <span className="settings-hint muted">
                  Codex asks for no approvals when run by the app. Read only and edit use its OS sandbox; full access runs
                  commands with <strong>no sandbox</strong> and only happens when allowed here. Edit mode cannot commit
                  in a linked worktree; its network access is Codex's default and not guaranteed off. The sandbox here can
                  only lower the run's access level, never raise it, and <code>extra_args</code> may not carry sandbox,
                  approval, trust, profile, feature, worktree, rules, user-config, <code>--cd</code> or output-file flags, and its <code>-c</code> keys are limited to{" "}
                  <code>model</code>, <code>model_reasoning_effort</code>, <code>model_reasoning_summary</code>,{" "}
                  <code>model_verbosity</code>, <code>hide_agent_reasoning</code>, <code>show_raw_agent_reasoning</code> and{" "}
                  <code>personality</code> (saving refuses anything else). <code>--add-dir</code> is allowed and widens the
                  writable roots in edit mode; it must be an absolute path (no <code>~</code>, no <code>..</code> components), and the filesystem root,
                  your home directory or its parents, the run's directory or its parents, and credential or system
                  directories (<code>~/.ssh</code>, <code>~/.config</code>, <code>/etc</code>, ...) are refused. Other
                  sensitive directories are not checked.
                </span>
              </div>
            )}
            {fields.settings && (
              <label>
                Settings <span className="muted">(JSON, no secrets)</span>
                <textarea
                  className="input mono"
                  rows={4}
                  value={settingsText}
                  onChange={(e) => setSettingsText(e.target.value)}
                />
              </label>
            )}
            <span className="ai-profile-actions">
              <button type="button" className="btn btn-sm" onClick={() => setDraft(null)}>
                Cancel
              </button>
              <button type="submit" className="btn btn-sm btn-primary" disabled={save.isPending}>
                {draft.id == null ? "Add" : "Save"}
              </button>
            </span>
          </form>
        )}
      </section>

      <section className="settings-group" data-testid="ai-feature-profiles">
        <h3>Features</h3>
        {FEATURE_ROWS.map((f) => (
          <label key={f.key} className="settings-row settings-row-stack">
            <span>{f.label}</span>
            <select
              className="input"
              value={features.data?.[f.key] ?? ""}
              disabled={features.isLoading || setFeatures.isPending}
              onChange={(e) => pick(f.key, e.target.value)}
              aria-label={f.label}
            >
              <option value="">Default profile</option>
              {list
                .filter((p) => !p.is_default && !(p.host_id != null && (f.key === "chat" || f.key === "sessions")))
                .map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
            </select>
            <span className="settings-hint muted">{f.hint}</span>
          </label>
        ))}
        <span className="settings-hint muted">
          Routines (focus, AI review, board insights, babysit…) pick their profile in the routine config.
        </span>
      </section>
    </>
  );
}
