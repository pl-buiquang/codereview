import { useEffect, useId, useState, useSyncExternalStore, type CSSProperties } from "react";
import { useMutation } from "@tanstack/react-query";
import { effectiveTheme, useAiDisabledReason, useSettingsStore, type ThemeMode } from "../lib/settings";
import { DIRECTIONS, type Direction } from "../lib/themes";
import {
  COLOUR_TOKEN,
  CORE_COLOURS,
  CUSTOM_KNOBS,
  DEFAULT_CUSTOM_THEME,
  DIFF_COLOURS,
  STATUS_COLOURS,
  SYNTAX_COLOURS,
  customThemeStyle,
  mergeScope,
  resetScope,
  toHexColour,
  type CustomColour,
  type CustomKnob,
  type CustomMode,
  type CustomScope,
} from "../lib/customTheme";
import { confirmDialog } from "../lib/confirm";
import { suggestTheme } from "../lib/themeSuggest";
import { toast } from "../lib/toast";
import {
  editCustomTheme,
  ownsUndo,
  redoCustomTheme,
  undoCustomTheme,
  useThemeHistory,
} from "../lib/themeHistory";

const MODES: { value: ThemeMode; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
  { value: "system", label: "System" },
];

function DirectionPicker() {
  const direction = useSettingsStore((s) => s.direction);
  const customTheme = useSettingsStore((s) => s.customTheme);
  const mode = useSettingsStore((s) => s.mode);
  const setDirection = useSettingsStore((s) => s.setDirection);
  const resolved = effectiveTheme(mode);

  const cards: { id: Direction | "custom"; label: string; blurb: string; base: Direction; style?: CSSProperties }[] = [
    ...DIRECTIONS.map((d) => ({ ...d, base: d.id })),
    {
      id: "custom",
      label: "Custom",
      blurb: "Your own colors, fonts and corners",
      base: customTheme.base,
      style: customThemeStyle(customTheme, resolved, { preview: true }) as CSSProperties,
    },
  ];

  return (
    <div className="theme-directions">
      {cards.map((d) => (
        <button
          key={d.id}
          type="button"
          className={`cr cr-${d.base} ${resolved} theme-card${direction === d.id ? " selected" : ""}`}
          style={d.style}
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
  const direction = useSettingsStore((s) => s.direction);

  return (
    <div className="settings-section-narrow theme-settings">
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

      {direction === "custom" && <CustomThemeEditor />}
    </div>
  );
}

// --- Custom theme editor --------------------------------------------------------------------

type SubTab = "colours" | "syntax" | "typography" | "shape";

const SUB_TABS: { key: SubTab; label: string }[] = [
  { key: "colours", label: "Colours" },
  { key: "syntax", label: "Syntax" },
  { key: "typography", label: "Typography" },
  { key: "shape", label: "Shape" },
];

const COLOUR_LABELS: Record<CustomColour, string> = {
  accent: "Accent",
  background: "Background",
  surface: "Surface",
  text: "Text",
  success: "Success",
  warning: "Warning",
  danger: "Danger",
  info: "Info",
  diffAdd: "Added lines",
  diffDel: "Removed lines",
  tokComment: "Comments",
  tokPunctuation: "Punctuation",
  tokLiteral: "Literals & tags",
  tokString: "Strings",
  tokOperator: "Operators",
  tokKeyword: "Keywords",
  tokFunction: "Functions",
  tokVariable: "Variables & regex",
};

const KNOB_LABELS: Record<CustomKnob, string> = {
  uiFont: "UI font",
  monoFont: "Code font",
  textSize: "Text size",
  corners: "Corners",
};

const OPTION_LABELS: Record<string, string> = {
  base: "Theme default",
  atkinson: "Atkinson Hyperlegible",
  inter: "Inter",
  plex: "IBM Plex Sans",
  geist: "Geist",
  jetbrains: "JetBrains Mono",
  system: "System",
  "atkinson-mono": "Atkinson Hyperlegible Mono",
  "plex-mono": "IBM Plex Mono",
  "geist-mono": "Geist Mono",
  small: "Small",
  large: "Large",
  square: "Square",
  round: "Round",
};

/** Changes whenever <html>'s class or style does: a cue to read the base theme's values again. */
let rootRevision = 0;
function subscribeRoot(onChange: () => void) {
  const observer = new MutationObserver(() => {
    rootRevision += 1;
    onChange();
  });
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style"] });
  return () => observer.disconnect();
}

function useRootRevision() {
  return useSyncExternalStore(subscribeRoot, () => rootRevision);
}

function CustomThemeEditor() {
  const customTheme = useSettingsStore((s) => s.customTheme);
  const mode = useSettingsStore((s) => s.mode);
  const [tab, setTab] = useState<SubTab>("colours");
  const shown: CustomMode = effectiveTheme(mode);
  const canUndo = useThemeHistory((h) => h.past.length > 0);
  const canRedo = useThemeHistory((h) => h.future.length > 0);

  // ⌘Z / ⌘⇧Z (Ctrl on other platforms) while the editor is open; text fields keep their own undo.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.key.toLowerCase() !== "z" || ownsUndo(e.target)) return;
      e.preventDefault();
      if (e.shiftKey) redoCustomTheme();
      else undoCustomTheme();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const onReset = async () => {
    const ok = await confirmDialog({
      title: "Reset custom theme",
      message: "Clear every color, font, size and corner override, in both dark and light? The base direction is kept.",
      confirmLabel: "Reset",
      danger: true,
    });
    if (ok) editCustomTheme((t) => ({ ...DEFAULT_CUSTOM_THEME, base: t.base }));
  };

  return (
    <section className="settings-group theme-editor">
      <h3>Custom theme</h3>
      <div className="theme-editor-head">
        <span className="muted">Based on</span>
        <span className="view-toggle">
          {DIRECTIONS.map((d) => (
            <button
              key={d.id}
              className={customTheme.base === d.id ? "active" : ""}
              onClick={() => editCustomTheme((t) => ({ ...t, base: d.id }))}
            >
              {d.label}
            </button>
          ))}
        </span>
        <span className="theme-editor-spacer" />
        <AiSuggest tab={tab} />
        <button className="btn btn-sm" onClick={undoCustomTheme} disabled={!canUndo} title="Undo (⌘Z)">
          Undo
        </button>
        <button className="btn btn-sm" onClick={redoCustomTheme} disabled={!canRedo} title="Redo (⌘⇧Z)">
          Redo
        </button>
        <button className="btn btn-sm" onClick={onReset}>
          Reset…
        </button>
      </div>

      <div className="theme-editor-head">
        <span className="view-toggle">
          {SUB_TABS.map((t) => (
            <button key={t.key} className={tab === t.key ? "active" : ""} onClick={() => setTab(t.key)}>
              {t.label}
            </button>
          ))}
        </span>
        {(tab === "colours" || tab === "syntax") && (
          <span className="muted settings-hint">
            Editing the {shown} palette — switch Mode above to edit the other.
          </span>
        )}
        <span className="theme-editor-spacer" />
        <button
          className="btn btn-sm"
          onClick={() => editCustomTheme((t) => resetScope(t, tab))}
          title="Clear this tab's overrides"
        >
          Clear tab
        </button>
      </div>

      <div className="theme-editor-body">
        <div className="theme-fields">
          {tab === "colours" && (
            <>
              <ColourGroup title="Core" colours={CORE_COLOURS} mode={shown} />
              <ColourGroup title="Status" colours={STATUS_COLOURS} mode={shown} />
              <ColourGroup title="Diff" colours={DIFF_COLOURS} mode={shown} />
            </>
          )}
          {tab === "syntax" && <ColourGroup title="Syntax" colours={SYNTAX_COLOURS} mode={shown} />}
          {tab === "typography" && (
            <>
              <KnobSelect knob="uiFont" />
              <KnobSelect knob="monoFont" />
              <KnobToggle knob="textSize" />
              <p className="muted settings-hint">
                Text size scales the whole interface; the diff font size above is separate.
              </p>
            </>
          )}
          {tab === "shape" && <KnobToggle knob="corners" />}
        </div>
        <ThemeSample />
      </div>
    </section>
  );
}

/** Which scope the "This tab only" AI suggestion works on, per sub-tab. */
const TAB_SCOPE: Record<SubTab, CustomScope> = {
  colours: "colours",
  syntax: "syntax",
  typography: "typography",
  shape: "shape",
};

/** Hint input + split button: design a whole theme, or only the open tab, from a mood. */
function AiSuggest({ tab }: { tab: SubTab }) {
  const disabledReason = useAiDisabledReason();
  const [hint, setHint] = useState("");
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    if (!menuOpen) return;
    const close = () => setMenuOpen(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [menuOpen]);

  const suggest = useMutation({
    mutationFn: (scope: CustomScope) =>
      suggestTheme({ scope, current: useSettingsStore.getState().customTheme, hint }).then((r) => ({ ...r, scope })),
    onSuccess: ({ suggested, inspiration, issues, scope }) => {
      editCustomTheme((t) => mergeScope(t, suggested, scope));
      toast.info(`Applied “${inspiration}” — ⌘Z to undo`);
      if (issues.length > 0) toast.error(`Some contrast is still low:\n${issues.join("\n")}`);
    },
    onError: (err) => toast.error(`AI suggest failed:\n${err instanceof Error ? err.message : String(err)}`),
  });

  const run = (scope: CustomScope) => {
    setMenuOpen(false);
    suggest.mutate(scope);
  };
  const disabled = disabledReason !== null || suggest.isPending;
  const tabLabel = SUB_TABS.find((t) => t.key === tab)?.label ?? "";

  return (
    <span className="theme-ai">
      <input
        type="text"
        className="input theme-ai-hint"
        placeholder="Mood (optional): warm, low contrast…"
        value={hint}
        disabled={disabled}
        onChange={(e) => setHint(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !disabled) run("all");
        }}
      />
      <span className="theme-ai-split">
        <button className="btn btn-sm" disabled={disabled} title={disabledReason ?? "Design a whole theme"} onClick={() => run("all")}>
          {suggest.isPending ? "Suggesting…" : "✨ AI suggest"}
        </button>
        <button
          className="btn btn-sm"
          disabled={disabled}
          aria-label="More AI suggest options"
          aria-expanded={menuOpen}
          onClick={(e) => {
            e.stopPropagation();
            setMenuOpen((o) => !o);
          }}
        >
          ▾
        </button>
        {menuOpen && (
          <span className="theme-ai-menu" role="menu">
            <button role="menuitem" onClick={() => run("all")}>
              Whole theme
            </button>
            <button role="menuitem" onClick={() => run(TAB_SCOPE[tab])}>
              This tab only ({tabLabel})
            </button>
          </span>
        )}
      </span>
    </span>
  );
}

function ColourGroup({
  title,
  colours,
  mode,
}: {
  title: string;
  colours: readonly CustomColour[];
  mode: CustomMode;
}) {
  return (
    <div className="theme-colour-group">
      <h4>{title}</h4>
      {colours.map((c) => (
        <ColourField key={`${mode}-${c}`} colour={c} mode={mode} />
      ))}
    </div>
  );
}

/** A native colour picker. Unset, it shows the base theme's value. */
function ColourField({ colour, mode }: { colour: CustomColour; mode: CustomMode }) {
  const id = useId();
  const value = useSettingsStore((s) => s.customTheme[mode][colour]);
  useRootRevision();
  const base = toHexColour(getComputedStyle(document.documentElement).getPropertyValue(COLOUR_TOKEN[colour]));
  const shown = value ?? base ?? "#000000";
  const set = (hex: string | null) =>
    editCustomTheme(
      (t) => {
        const palette = { ...t[mode] };
        if (hex) palette[colour] = hex;
        else delete palette[colour];
        return { ...t, [mode]: palette };
      },
      `${mode}:${colour}`,
    );
  return (
    <div className="theme-colour">
      <label htmlFor={id}>{COLOUR_LABELS[colour]}</label>
      <input id={id} type="color" value={shown} onChange={(e) => set(toHexColour(e.target.value))} />
      <code className={value ? "" : "muted"}>{value ?? "theme default"}</code>
      {value && (
        <button className="btn btn-sm" onClick={() => set(null)} title="Back to the theme's value">
          ↺
        </button>
      )}
    </div>
  );
}

function KnobSelect({ knob }: { knob: "uiFont" | "monoFont" }) {
  const value = useSettingsStore((s) => s.customTheme[knob]);
  return (
    <label className="settings-row">
      <span>{KNOB_LABELS[knob]}</span>
      <select
        className="select"
        value={value}
        onChange={(e) => editCustomTheme((t) => ({ ...t, [knob]: e.target.value }))}
      >
        {CUSTOM_KNOBS[knob].map((o) => (
          <option key={o} value={o}>
            {OPTION_LABELS[o]}
          </option>
        ))}
      </select>
    </label>
  );
}

function KnobToggle({ knob }: { knob: "textSize" | "corners" }) {
  const value = useSettingsStore((s) => s.customTheme[knob]);
  return (
    <div className="settings-row">
      <span>{KNOB_LABELS[knob]}</span>
      <span className="view-toggle">
        {CUSTOM_KNOBS[knob].map((o) => (
          <button
            key={o}
            className={value === o ? "active" : ""}
            onClick={() => editCustomTheme((t) => ({ ...t, [knob]: o }))}
          >
            {OPTION_LABELS[o]}
          </button>
        ))}
      </span>
    </div>
  );
}

/** The main pieces in the theme in use, inert: buttons and a few highlighted diff lines. */
function ThemeSample() {
  return (
    <div className="theme-sample" inert>
      <div className="theme-sample-title">Sample</div>
      <div className="theme-sample-buttons">
        <button className="btn btn-primary btn-sm">Primary</button>
        <button className="btn btn-sm">Default</button>
        <button className="btn btn-danger btn-sm">Danger</button>
      </div>
      <div className="theme-sample-badges">
        <span style={{ color: "var(--success)" }}>● approved</span>
        <span style={{ color: "var(--warning)" }}>● draft</span>
        <span style={{ color: "var(--danger)" }}>● failing</span>
      </div>
      <pre className="diff theme-sample-code">
        <span className="theme-sample-line">
          <span className="token comment">{"// load the settings"}</span>
        </span>
        <span className="theme-sample-line del">
          <span className="token keyword">const</span> <span className="token variable">cfg</span>{" "}
          <span className="token operator">=</span> <span className="token function">read</span>
          <span className="token punctuation">(</span>
          <span className="token string">"a.json"</span>
          <span className="token punctuation">);</span>
        </span>
        <span className="theme-sample-line add">
          <span className="token keyword">const</span> <span className="token variable">cfg</span>{" "}
          <span className="token operator">=</span> <span className="token function">load</span>
          <span className="token punctuation">(</span>
          <span className="token number">42</span>
          <span className="token punctuation">,</span> <span className="token boolean">true</span>
          <span className="token punctuation">);</span>
        </span>
        <span className="theme-sample-line">
          <span className="token tag">{"<Tag"}</span> <span className="token attr-name">name</span>
          <span className="token punctuation">=</span>
          <span className="token string">"x"</span>
          <span className="token tag">{" />"}</span>
        </span>
      </pre>
    </div>
  );
}
