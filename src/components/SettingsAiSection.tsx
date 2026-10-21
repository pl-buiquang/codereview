import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSettingsStore } from "../lib/settings";
import { useUIStore } from "../store";
import { toast } from "../lib/toast";
import type { PromptInfo } from "../lib/types";
import { MODEL_OPTIONS } from "./ChatPanel";

function ModelInput({
  value,
  onChange,
  placeholder,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
}) {
  return (
    <input
      type="text"
      className="input"
      list="ai-model-options"
      placeholder={placeholder}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

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
  const chatModel = useSettingsStore((s) => s.chatModel);
  const setChatModel = useSettingsStore((s) => s.setChatModel);
  const overviewModel = useSettingsStore((s) => s.overviewModel);
  const setOverviewModel = useSettingsStore((s) => s.setOverviewModel);
  const aiEnabled = useSettingsStore((s) => s.aiEnabled);
  const setAiEnabled = useSettingsStore((s) => s.setAiEnabled);
  const showRoutinesConfig = useUIStore((s) => s.showRoutinesConfig);
  const prompts = useQuery({ queryKey: ["prompts"], queryFn: api.promptsList });

  return (
    <div className="settings-section-narrow ai-settings">
      <datalist id="ai-model-options">
        {MODEL_OPTIONS.filter((o) => o.value).map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </datalist>

      <section className="settings-group">
        <h3>AI features</h3>
        <label className="settings-row">
          <span>Enable AI features</span>
          <input type="checkbox" checked={aiEnabled} onChange={(e) => setAiEnabled(e.target.checked)} />
        </label>
        <span className="settings-hint muted">
          Master switch for everything the app runs through <code>claude -p</code>: review chat, focus, AI
          review, board insights, session summaries and routines. While it is off, those buttons are disabled
          and queued routine runs wait. Terminal sessions (new agent session, resume) are not affected. Off by
          default.
        </span>
      </section>

      <section className="settings-group">
        <h3>Models</h3>
        <label className="settings-row settings-row-stack">
          <span>Chat &amp; focus model</span>
          <ModelInput
            value={chatModel}
            onChange={setChatModel}
            placeholder="leave empty to use opus (ANTHROPIC_DEFAULT_OPUS_MODEL)"
          />
          <span className="settings-hint muted">
            Passed as <code>--model</code> to <code>claude -p</code>. Leave empty to use the
            env-configured default (Bedrock ARN, model name, etc.).
          </span>
        </label>
        <label className="settings-row settings-row-stack">
          <span>Board overview model</span>
          <ModelInput
            value={overviewModel}
            onChange={setOverviewModel}
            placeholder="leave empty to use the env default"
          />
          <span className="settings-hint muted">
            Runs with <code>--effort low</code> and no tools; a fast model keeps the overview
            snappy.
          </span>
        </label>
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
