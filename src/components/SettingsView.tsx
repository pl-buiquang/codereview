import { useQuery } from "@tanstack/react-query";
import { useUIStore } from "../store";
import { useSettingsStore, type Theme } from "../lib/settings";
import { api } from "../lib/api";
import type { ToolEnv } from "../lib/types";

const THEMES: { value: Theme; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "System" },
];

function EnvironmentPanel() {
  const envQuery = useQuery({ queryKey: ["environment"], queryFn: api.checkEnvironment });

  if (envQuery.isLoading) return <p className="muted">Checking tools…</p>;
  if (envQuery.isError || !envQuery.data)
    return <p className="error">Could not check environment: {String(envQuery.error)}</p>;

  const env: ToolEnv = envQuery.data;
  return (
    <div className="env-rows">
      <ToolRow
        label="git"
        value={env.git}
        missingHint="Not found on PATH — install git and reopen the app."
      />
      <ToolRow
        label="gh"
        value={env.gh}
        missingHint="Not found on PATH — install the GitHub CLI to list/publish PRs."
      />
      <div className="settings-row">
        <span>GitHub auth</span>
        {env.gh == null ? (
          <span className="muted">—</span>
        ) : env.gh_authed ? (
          <span className="env-ok">Authenticated</span>
        ) : (
          <span className="error">
            Not authenticated — run <code>gh auth login</code>.
          </span>
        )}
      </div>
    </div>
  );
}

function ToolRow({
  label,
  value,
  missingHint,
}: {
  label: string;
  value: string | null;
  missingHint: string;
}) {
  return (
    <div className="settings-row">
      <span>{label}</span>
      {value ? (
        <code className="env-path" title={value}>
          {value}
        </code>
      ) : (
        <span className="error">{missingHint}</span>
      )}
    </div>
  );
}

export function SettingsView() {
  const closeSettings = useUIStore((s) => s.closeSettings);
  const theme = useSettingsStore((s) => s.theme);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);
  const defaultViewType = useSettingsStore((s) => s.defaultViewType);
  const defaultThreeDot = useSettingsStore((s) => s.defaultThreeDot);
  const setTheme = useSettingsStore((s) => s.setTheme);
  const setDiffFontSize = useSettingsStore((s) => s.setDiffFontSize);
  const setDefaultViewType = useSettingsStore((s) => s.setDefaultViewType);
  const setDefaultThreeDot = useSettingsStore((s) => s.setDefaultThreeDot);

  return (
    <section className="main-panel settings-panel">
      <header className="main-header settings-header">
        <button onClick={closeSettings}>← Back</button>
        <h2>Settings</h2>
      </header>

      <div className="settings-body">
        <section className="settings-group">
          <h3>Appearance</h3>
          <label className="settings-row">
            <span>Theme</span>
            <select value={theme} onChange={(e) => setTheme(e.target.value as Theme)}>
              {THEMES.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
            </select>
          </label>
          <label className="settings-row">
            <span>Diff font size</span>
            <span className="settings-control">
              <input
                type="range"
                min={10}
                max={20}
                step={0.5}
                value={diffFontSize}
                onChange={(e) => setDiffFontSize(Number(e.target.value))}
              />
              <span className="settings-value">{diffFontSize}px</span>
            </span>
          </label>
        </section>

        <section className="settings-group">
          <h3>Review defaults</h3>
          <div className="settings-row">
            <span>Default diff view</span>
            <span className="view-toggle">
              <button
                className={defaultViewType === "split" ? "active" : ""}
                onClick={() => setDefaultViewType("split")}
              >
                Split
              </button>
              <button
                className={defaultViewType === "unified" ? "active" : ""}
                onClick={() => setDefaultViewType("unified")}
              >
                Unified
              </button>
            </span>
          </div>
          <label className="settings-row">
            <span>Default to merge-base (three-dot) diff</span>
            <input
              type="checkbox"
              checked={defaultThreeDot}
              onChange={(e) => setDefaultThreeDot(e.target.checked)}
            />
          </label>
        </section>

        <section className="settings-group">
          <h3>Environment</h3>
          <EnvironmentPanel />
        </section>
      </div>
    </section>
  );
}
