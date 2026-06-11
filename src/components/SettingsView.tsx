import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useUIStore } from "../store";
import { effectiveTheme, useSettingsStore, type ThemeMode } from "../lib/settings";
import { DIRECTIONS } from "../lib/themes";
import { api } from "../lib/api";
import type { ToolEnv } from "../lib/types";
import { Icon, type IconName } from "./icons";

type SettingsSection = "general";

const SECTIONS: { key: SettingsSection; label: string; icon: IconName }[] = [
  { key: "general", label: "General", icon: "gear" },
];

const MODES: { value: ThemeMode; label: string }[] = [
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

function DirectionPicker() {
  const direction = useSettingsStore((s) => s.direction);
  const mode = useSettingsStore((s) => s.mode);
  const setDirection = useSettingsStore((s) => s.setDirection);
  const resolved = effectiveTheme(mode);

  return (
    <div className="theme-directions">
      {DIRECTIONS.map((d) => (
        <button
          key={d.id}
          type="button"
          className={`cr cr-${d.id} ${resolved} theme-card${
            direction === d.id ? " selected" : ""
          }`}
          onClick={() => setDirection(d.id)}
          aria-pressed={direction === d.id}
        >
          <span className="theme-card-swatches">
            <span style={{ background: "var(--bg)" }} />
            <span style={{ background: "var(--surface)" }} />
            <span style={{ background: "var(--accent)" }} />
            <span style={{ background: "var(--success)" }} />
          </span>
          <span className="theme-card-sample">
            <span className="theme-card-aa" style={{ fontFamily: "var(--font-ui)" }}>
              Aa
            </span>
            <code style={{ fontFamily: "var(--font-mono)" }}>const x = 1;</code>
          </span>
          <span className="theme-card-meta">
            <span className="theme-card-label">{d.label}</span>
            <span className="theme-card-blurb">{d.blurb}</span>
          </span>
        </button>
      ))}
    </div>
  );
}

function GeneralSection() {
  const mode = useSettingsStore((s) => s.mode);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);
  const defaultViewType = useSettingsStore((s) => s.defaultViewType);
  const defaultThreeDot = useSettingsStore((s) => s.defaultThreeDot);
  const botLogins = useSettingsStore((s) => s.botLogins);
  const repoStripPrefixes = useSettingsStore((s) => s.repoStripPrefixes);
  const setMode = useSettingsStore((s) => s.setMode);
  const setDiffFontSize = useSettingsStore((s) => s.setDiffFontSize);
  const setDefaultViewType = useSettingsStore((s) => s.setDefaultViewType);
  const setDefaultThreeDot = useSettingsStore((s) => s.setDefaultThreeDot);
  const setBotLogins = useSettingsStore((s) => s.setBotLogins);
  const setRepoStripPrefixes = useSettingsStore((s) => s.setRepoStripPrefixes);

  return (
    <div className="settings-section-narrow">
      <section className="settings-group">
        <h3>Appearance</h3>
        <div className="settings-row settings-row-stack">
          <span>Direction</span>
          <DirectionPicker />
        </div>
        <div className="settings-row">
          <span>Mode</span>
          <span className="view-toggle">
            {MODES.map((m) => (
              <button
                key={m.value}
                className={mode === m.value ? "active" : ""}
                onClick={() => setMode(m.value)}
              >
                {m.label}
              </button>
            ))}
          </span>
        </div>
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
        <h3>Inbox</h3>
        <label className="settings-row settings-row-stack">
          <span>Bot logins</span>
          <input
            type="text"
            className="input"
            placeholder="dependabot, renovate"
            value={botLogins}
            onChange={(e) => setBotLogins(e.target.value)}
          />
          <span className="settings-hint muted">
            Comma-separated GitHub logins routed to the Bots tab (case-insensitive; GitHub
            strips the <code>[bot]</code> suffix).
          </span>
        </label>
        <label className="settings-row settings-row-stack">
          <span>Strip repo prefixes</span>
          <input
            type="text"
            className="input"
            placeholder="philips-internal/cardiologs-"
            value={repoStripPrefixes}
            onChange={(e) => setRepoStripPrefixes(e.target.value)}
          />
          <span className="settings-hint muted">
            Comma-separated prefixes removed from repo names in the inbox. Hovering a stripped
            name shows the full <code>owner/name</code>.
          </span>
        </label>
      </section>

      <section className="settings-group">
        <h3>Environment</h3>
        <EnvironmentPanel />
      </section>
    </div>
  );
}

export function SettingsView() {
  const closeSettings = useUIStore((s) => s.closeSettings);
  const [section, setSection] = useState<SettingsSection>("general");

  return (
    <section className="main-panel settings-panel">
      <header className="main-header settings-header">
        <button onClick={closeSettings}>← Back</button>
        <h2>Settings</h2>
      </header>

      <div className="settings-layout">
        <nav className="settings-sidebar cr-nav">
          {SECTIONS.map((s) => (
            <button
              key={s.key}
              className={`cr-nav-item${section === s.key ? " active" : ""}`}
              onClick={() => setSection(s.key)}
            >
              <Icon name={s.icon} size={15} />
              <span>{s.label}</span>
            </button>
          ))}
        </nav>

        <div className="settings-content">
          <GeneralSection />
        </div>
      </div>
    </section>
  );
}
