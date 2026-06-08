import { useState } from "react";
import {
  BUILTINS,
  resolveActiveTheme,
  useSettingsStore,
  type Theme,
} from "../../lib/settings";
import { ThemeEditor } from "./ThemeEditor";

function ThemeRow({
  theme,
  selected,
  active,
  onSelect,
}: {
  theme: Theme;
  selected: boolean;
  active: boolean;
  onSelect: () => void;
}) {
  const swatches = [
    theme.ui.bg,
    theme.ui.accent,
    theme.ui.text,
    theme.syntax.keyword,
    theme.syntax.string,
  ];
  return (
    <div
      className={`theme-row${selected ? " selected" : ""}`}
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <span className="theme-swatches">
        {swatches.map((c, i) => (
          <span key={i} className="theme-swatch" style={{ background: c }} />
        ))}
      </span>
      <span className="theme-name">{theme.name}</span>
      {active && <span className="theme-badge">Active</span>}
      {theme.builtin && <span className="theme-badge">Built-in</span>}
    </div>
  );
}

export function ThemeSection() {
  const themeMode = useSettingsStore((s) => s.themeMode);
  const customThemes = useSettingsStore((s) => s.customThemes);
  const darkThemeId = useSettingsStore((s) => s.darkThemeId);
  const lightThemeId = useSettingsStore((s) => s.lightThemeId);
  const setDarkThemeId = useSettingsStore((s) => s.setDarkThemeId);
  const setLightThemeId = useSettingsStore((s) => s.setLightThemeId);
  const addThemeFrom = useSettingsStore((s) => s.addThemeFrom);

  const allThemes = [...BUILTINS, ...customThemes];
  const activeId = resolveActiveTheme({
    themeMode,
    customThemes,
    darkThemeId,
    lightThemeId,
  }).id;

  const [selectedId, setSelectedId] = useState<string>(activeId);
  const [newFromId, setNewFromId] = useState<string>(BUILTINS[0].id);

  const selected = allThemes.find((t) => t.id === selectedId) ?? allThemes[0];

  const handleNew = () => {
    const src = allThemes.find((t) => t.id === newFromId) ?? BUILTINS[0];
    setSelectedId(addThemeFrom(src.id, `${src.name} copy`));
  };

  return (
    <div className="theme-section">
      <section className="settings-group">
        <h3>Active themes</h3>
        <label className="settings-row">
          <span>Dark mode uses</span>
          <select value={darkThemeId} onChange={(e) => setDarkThemeId(e.target.value)}>
            {allThemes.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        </label>
        <label className="settings-row">
          <span>Light mode uses</span>
          <select value={lightThemeId} onChange={(e) => setLightThemeId(e.target.value)}>
            {allThemes.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        </label>
        <p className="muted">
          The General tab's Theme setting (Dark / Light / System) decides which of these is
          shown.
        </p>
      </section>

      <section className="settings-group">
        <h3>Themes</h3>
        <div className="theme-list-toolbar">
          <span className="muted">New theme from</span>
          <select value={newFromId} onChange={(e) => setNewFromId(e.target.value)}>
            {allThemes.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
          <button className="btn-xs btn-primary" onClick={handleNew}>
            + New theme
          </button>
        </div>
        <div className="theme-list">
          {allThemes.map((t) => (
            <ThemeRow
              key={t.id}
              theme={t}
              selected={t.id === selectedId}
              active={t.id === activeId}
              onSelect={() => setSelectedId(t.id)}
            />
          ))}
        </div>
      </section>

      <section className="settings-group">
        <h3>{selected.builtin ? `${selected.name} (built-in)` : `Editing: ${selected.name}`}</h3>
        <ThemeEditor theme={selected} onSelectTheme={setSelectedId} />
      </section>
    </div>
  );
}
