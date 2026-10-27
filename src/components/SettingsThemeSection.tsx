import { effectiveTheme, useSettingsStore, type ThemeMode } from "../lib/settings";
import { DIRECTIONS } from "../lib/themes";

const MODES: { value: ThemeMode; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "System" },
];

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

export function ThemeSection() {
  const mode = useSettingsStore((s) => s.mode);
  const diffFontSize = useSettingsStore((s) => s.diffFontSize);
  const setMode = useSettingsStore((s) => s.setMode);
  const setDiffFontSize = useSettingsStore((s) => s.setDiffFontSize);

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
    </div>
  );
}
