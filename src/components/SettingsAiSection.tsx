import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSettingsStore } from "../lib/settings";
import { AiProfilesSection } from "./SettingsAiProfiles";
import { useUIStore } from "../store";
import { toast } from "../lib/toast";
import type { PromptInfo } from "../lib/types";

function PromptEditor({ prompt }: { prompt: PromptInfo }) {
  const qc = useQueryClient();
  const [draft, setDraft] = useState(prompt.override ?? prompt.default);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const save = (body: string) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      api
        .promptSet(prompt.key, body)
        .then(() => qc.invalidateQueries({ queryKey: ["prompts"] }))
        .catch((e) => toast.error(`Failed to save prompt: ${e}`));
    }, 400);
  };

  const reset = () => {
    window.clearTimeout(timer.current);
    api
      .promptReset(prompt.key)
      .then(() => {
        setDraft(prompt.default);
        qc.invalidateQueries({ queryKey: ["prompts"] });
      })
      .catch((e) => toast.error(`Failed to reset prompt: ${e}`));
  };

  const modified = draft !== prompt.default;

  return (
    <div className="settings-row settings-row-stack ai-prompt">
      <div className="ai-prompt-head">
        <strong>{prompt.label}</strong>
        {modified && <span className="ai-prompt-badge">Modified</span>}
        <span className="cr-spacer" />
        <button className="btn btn-sm" onClick={reset} disabled={!modified}>
          Reset to default
        </button>
      </div>
      <span className="settings-hint muted">{prompt.description}</span>
      {prompt.placeholders.length > 0 && (
        <span className="ai-prompt-vars">
          {prompt.placeholders.map((p) => (
            <code key={p}>{`{{${p}}}`}</code>
          ))}
        </span>
      )}
      <textarea
        className="textarea settings-patterns ai-prompt-text"
        spellCheck={false}
        value={draft}
        onChange={(e) => {
          setDraft(e.target.value);
          save(e.target.value);
        }}
      />
    </div>
  );
}

export function AiSection() {
  const aiEnabled = useSettingsStore((s) => s.aiEnabled);
  const setAiEnabled = useSettingsStore((s) => s.setAiEnabled);
  const showRoutinesConfig = useUIStore((s) => s.showRoutinesConfig);
  const opencodeProviders = useSettingsStore((s) => s.opencodeModelProviders);
  const setOpencodeProviders = useSettingsStore((s) => s.setOpencodeModelProviders);
  const prompts = useQuery({ queryKey: ["prompts"], queryFn: api.promptsList });

  return (
    <div className="settings-section-narrow ai-settings">
      <section className="settings-group">
        <h3>AI features</h3>
        <div className="settings-row">
          <span id="ai-enabled-label">Enable AI features</span>
          <button
            type="button"
            role="switch"
            aria-checked={aiEnabled}
            aria-labelledby="ai-enabled-label"
            className="switch"
            onClick={() => setAiEnabled(!aiEnabled)}
          />
        </div>
        <span className="settings-hint muted">
          Master switch for everything the app runs through an AI harness: review chat, focus, AI review,
          board insights, session summaries and routines. While it is off, those buttons are disabled
          and queued routine runs wait. Terminal sessions (new agent session, resume) are not affected. Off by
          default.
        </span>
      </section>

      <AiProfilesSection />

      <section className="settings-group">
        <h3>opencode models</h3>
        <label className="settings-row">
          <span>Providers to list</span>
          <input
            className="input"
            placeholder="all (for example: openrouter)"
            value={opencodeProviders}
            onChange={(e) => setOpencodeProviders(e.target.value)}
          />
        </label>
        <span className="settings-hint muted">
          Comma-separated opencode providers shown in the profile model picker (the part of the model id before the first
          slash, such as <code>openrouter</code>). Empty lists every provider. Only the list is filtered; any model can still
          be typed in.
        </span>
      </section>

      <section className="settings-group">
        <h3>Prompts</h3>
        <span className="settings-hint muted">
          Focus, board insights, AI review and babysit prompts are edited with their routines.{" "}
          <button className="link-button" onClick={showRoutinesConfig}>
            Open routine config
          </button>
        </span>
        {prompts.isLoading && <span className="muted">Loading…</span>}
        {prompts.error && <span className="error">{String(prompts.error)}</span>}
        {prompts.data?.map((p) => <PromptEditor key={p.key} prompt={p} />)}
      </section>
    </div>
  );
}
