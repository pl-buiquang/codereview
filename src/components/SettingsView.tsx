import { useUIStore } from "../store";
import { useSettingsStore, type Theme } from "../lib/settings";

const THEMES: { value: Theme; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "System" },
];

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
      </div>
    </section>
  );
}
