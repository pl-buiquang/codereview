import { confirmDialog } from "../../lib/confirm";
import {
  BUILTIN_DARK_ID,
  FALLBACK_CODE_FONT,
  MONO_FONT_PRESETS,
  TOKEN_ROLES,
  useSettingsStore,
  type SyntaxColors,
  type Theme,
  type TokenRole,
  type UiColors,
} from "../../lib/settings";
import { ThemePreview } from "./ThemePreview";

const UI_FIELDS: { key: keyof UiColors; label: string; rgba?: boolean }[] = [
  { key: "bg", label: "Background" },
  { key: "bgElev", label: "Elevated background" },
  { key: "border", label: "Border" },
  { key: "text", label: "Text" },
  { key: "muted", label: "Muted text" },
  { key: "accent", label: "Accent" },
  { key: "danger", label: "Danger" },
  { key: "success", label: "Success" },
  { key: "warning", label: "Warning" },
  { key: "reviewTabAccent", label: "Review tab underline" },
  { key: "diffAddBg", label: "Diff added (rgba)", rgba: true },
  { key: "diffDelBg", label: "Diff removed (rgba)", rgba: true },
];

const TOKEN_LABELS: Record<TokenRole, string> = {
  comment: "Comment",
  punctuation: "Punctuation",
  literal: "Number / constant",
  string: "String",
  operator: "Operator",
  keyword: "Keyword",
  function: "Function / class",
  variable: "Variable / regex",
};

const HEX6 = /^#[0-9a-fA-F]{6}$/;

function ColorField({
  label,
  value,
  onChange,
  rgba,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  rgba?: boolean;
}) {
  return (
    <div className="color-field">
      <label>{label}</label>
      {!rgba && (
        <input
          type="color"
          value={HEX6.test(value) ? value : "#000000"}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
      <input
        type="text"
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

export function ThemeEditor({
  theme,
  onSelectTheme,
}: {
  theme: Theme;
  onSelectTheme: (id: string) => void;
}) {
  const updateTheme = useSettingsStore((s) => s.updateTheme);
  const addThemeFrom = useSettingsStore((s) => s.addThemeFrom);
  const deleteTheme = useSettingsStore((s) => s.deleteTheme);

  const duplicate = () => onSelectTheme(addThemeFrom(theme.id, `${theme.name} copy`));

  if (theme.builtin) {
    return (
      <div className="theme-editor-readonly">
        <p>Built-in themes can't be edited. Duplicate this one to make it your own.</p>
        <button className="btn-xs btn-primary" onClick={duplicate}>
          Duplicate to edit
        </button>
        <ThemePreview theme={theme} />
      </div>
    );
  }

  const handleDelete = async () => {
    const ok = await confirmDialog({
      title: `Delete "${theme.name}"?`,
      message: "Any mode using this theme will fall back to the matching built-in.",
      confirmLabel: "Delete",
      danger: true,
    });
    if (ok) {
      onSelectTheme(BUILTIN_DARK_ID);
      deleteTheme(theme.id);
    }
  };

  const presetMatch = MONO_FONT_PRESETS.find((p) => p.value === theme.codeFont);

  return (
    <div className="theme-editor">
      <div className="settings-row">
        <span>Name</span>
        <input
          type="text"
          value={theme.name}
          onChange={(e) => updateTheme(theme.id, { name: e.target.value })}
        />
      </div>

      <section className="settings-group">
        <h3>UI colors</h3>
        <div className="color-grid">
          {UI_FIELDS.map((f) => (
            <ColorField
              key={f.key}
              label={f.label}
              rgba={f.rgba}
              value={theme.ui[f.key]}
              onChange={(v) =>
                updateTheme(theme.id, { ui: { [f.key]: v } as Partial<UiColors> })
              }
            />
          ))}
        </div>
      </section>

      <section className="settings-group">
        <h3>Code font</h3>
        <div className="theme-font-row">
          <select
            value={presetMatch ? presetMatch.value : "__custom__"}
            onChange={(e) => {
              if (e.target.value !== "__custom__")
                updateTheme(theme.id, { codeFont: e.target.value });
            }}
          >
            {MONO_FONT_PRESETS.map((p) => (
              <option key={p.label} value={p.value}>
                {p.label}
              </option>
            ))}
            {!presetMatch && <option value="__custom__">Custom</option>}
          </select>
          <input
            type="text"
            spellCheck={false}
            placeholder={FALLBACK_CODE_FONT}
            value={theme.codeFont}
            onChange={(e) => updateTheme(theme.id, { codeFont: e.target.value })}
          />
        </div>
      </section>

      <section className="settings-group">
        <h3>Syntax highlighting</h3>
        <div className="color-grid">
          {TOKEN_ROLES.map((role) => (
            <ColorField
              key={role}
              label={TOKEN_LABELS[role]}
              value={theme.syntax[role]}
              onChange={(v) =>
                updateTheme(theme.id, { syntax: { [role]: v } as Partial<SyntaxColors> })
              }
            />
          ))}
        </div>
      </section>

      <section className="settings-group">
        <h3>Preview</h3>
        <ThemePreview theme={theme} />
      </section>

      <div className="theme-list-toolbar">
        <button className="btn-xs" onClick={duplicate}>
          Duplicate
        </button>
        <button className="btn-xs btn-danger" onClick={handleDelete}>
          Delete theme
        </button>
      </div>
    </div>
  );
}
