import { create } from "zustand";
import { persist } from "zustand/middleware";

export type ThemeMode = "dark" | "light" | "system";
export type ThemeBase = "dark" | "light";
export type DiffViewType = "split" | "unified";

export const DEFAULT_DIFF_FONT_SIZE = 12.5;

/** Fallback monospace stack used when a theme leaves `codeFont` empty. */
export const FALLBACK_CODE_FONT = "ui-monospace, SFMono-Regular, Menlo, monospace";

/** UI surface/accent colors a theme controls. */
export interface UiColors {
  bg: string;
  bgElev: string;
  border: string;
  text: string;
  muted: string;
  accent: string;
  danger: string;
  success: string;
  warning: string;
  /** Diff insert/delete row tints — rgba so they layer over the code background. */
  diffAddBg: string;
  diffDelBg: string;
  /** Underline color of the active review tab. */
  reviewTabAccent: string;
}

/** Named syntax-highlight roles; each maps to a set of Prism/refractor token classes. */
export type TokenRole =
  | "comment"
  | "punctuation"
  | "literal"
  | "string"
  | "operator"
  | "keyword"
  | "function"
  | "variable";

export type SyntaxColors = Record<TokenRole, string>;

export interface Theme {
  id: string;
  name: string;
  base: ThemeBase;
  builtin: boolean;
  derivedFrom?: string;
  ui: UiColors;
  /** CSS font-family stack for code; "" means use FALLBACK_CODE_FONT. */
  codeFont: string;
  syntax: SyntaxColors;
}

/** Partial edit applied to a custom theme by the editor. */
export interface ThemePatch {
  name?: string;
  base?: ThemeBase;
  codeFont?: string;
  ui?: Partial<UiColors>;
  syntax?: Partial<SyntaxColors>;
}

/**
 * Each syntax role → the Prism/refractor token classes it colors. Single source
 * of truth shared by the CSS (`--tok-*` vars) and the theme editor's labels.
 */
export const TOKEN_ROLE_CLASSES: Record<TokenRole, string[]> = {
  comment: ["comment", "prolog", "doctype", "cdata"],
  punctuation: ["punctuation"],
  literal: ["property", "tag", "boolean", "number", "constant", "symbol", "deleted"],
  string: ["selector", "attr-name", "string", "char", "builtin", "inserted"],
  operator: ["operator", "entity", "url"],
  keyword: ["atrule", "attr-value", "keyword"],
  function: ["function", "class-name"],
  variable: ["regex", "important", "variable"],
};

export const TOKEN_ROLES = Object.keys(TOKEN_ROLE_CLASSES) as TokenRole[];

/** UiColors key → CSS custom-property name. Shared by the apply hook and preview. */
export const UI_VAR: Record<keyof UiColors, string> = {
  bg: "--bg",
  bgElev: "--bg-elev",
  border: "--border",
  text: "--text",
  muted: "--muted",
  accent: "--accent",
  danger: "--danger",
  success: "--success",
  warning: "--warning",
  diffAddBg: "--diff-add-bg",
  diffDelBg: "--diff-del-bg",
  reviewTabAccent: "--review-tab-accent",
};

export const tokenVar = (role: TokenRole): string => `--tok-${role}`;

/** Curated monospace presets for the code-font dropdown ("" = system default). */
export const MONO_FONT_PRESETS: { label: string; value: string }[] = [
  { label: "System default", value: "" },
  { label: "SF Mono", value: '"SF Mono", SFMono-Regular, ui-monospace, monospace' },
  { label: "Menlo", value: "Menlo, monospace" },
  { label: "Monaco", value: "Monaco, monospace" },
  { label: "Consolas", value: "Consolas, monospace" },
  { label: "Fira Code", value: '"Fira Code", monospace' },
  { label: "JetBrains Mono", value: '"JetBrains Mono", monospace' },
  { label: "Source Code Pro", value: '"Source Code Pro", monospace' },
  { label: "Cascadia Code", value: '"Cascadia Code", monospace' },
  { label: "IBM Plex Mono", value: '"IBM Plex Mono", monospace' },
];

export const BUILTIN_DARK_ID = "builtin-dark";
export const BUILTIN_LIGHT_ID = "builtin-light";

export const BUILTIN_DARK: Theme = Object.freeze({
  id: BUILTIN_DARK_ID,
  name: "Dark",
  base: "dark",
  builtin: true,
  codeFont: "",
  ui: {
    bg: "#0d1117",
    bgElev: "#161b22",
    border: "#30363d",
    text: "#e6edf3",
    muted: "#8b949e",
    accent: "#2f81f7",
    danger: "#f85149",
    success: "#3fb950",
    warning: "#d29922",
    diffAddBg: "rgba(63, 185, 80, 0.15)",
    diffDelBg: "rgba(248, 81, 73, 0.15)",
    reviewTabAccent: "#ffffff",
  },
  syntax: {
    comment: "#8b949e",
    punctuation: "#c9d1d9",
    literal: "#79c0ff",
    string: "#a5d6ff",
    operator: "#d2a8ff",
    keyword: "#ff7b72",
    function: "#d2a8ff",
    variable: "#ffa657",
  },
}) as Theme;

export const BUILTIN_LIGHT: Theme = Object.freeze({
  id: BUILTIN_LIGHT_ID,
  name: "Light",
  base: "light",
  builtin: true,
  codeFont: "",
  ui: {
    bg: "#ffffff",
    bgElev: "#f6f8fa",
    border: "#d0d7de",
    text: "#1f2328",
    muted: "#656d76",
    accent: "#0969da",
    danger: "#cf222e",
    success: "#1a7f37",
    warning: "#9a6700",
    diffAddBg: "rgba(46, 160, 67, 0.15)",
    diffDelBg: "rgba(207, 34, 46, 0.15)",
    reviewTabAccent: "#0969da",
  },
  syntax: {
    comment: "#6e7781",
    punctuation: "#24292f",
    literal: "#0550ae",
    string: "#0a3069",
    operator: "#8250df",
    keyword: "#cf222e",
    function: "#8250df",
    variable: "#953800",
  },
}) as Theme;

export const BUILTINS: Theme[] = [BUILTIN_DARK, BUILTIN_LIGHT];

function makeId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return `theme-${Math.random().toString(36).slice(2)}`;
}

/** Deep-copy a theme into a new editable custom theme. */
function cloneTheme(
  src: Theme,
  id: string,
  name: string,
  uiOverride?: Partial<UiColors>,
): Theme {
  return {
    id,
    name,
    base: src.base,
    builtin: false,
    derivedFrom: src.id,
    codeFont: src.codeFont,
    ui: { ...src.ui, ...uiOverride },
    syntax: { ...src.syntax },
  };
}

/** Find a theme by id among built-ins + customs (undefined if missing). */
export function findTheme(customThemes: Theme[], id: string): Theme | undefined {
  if (id === BUILTIN_DARK_ID) return BUILTIN_DARK;
  if (id === BUILTIN_LIGHT_ID) return BUILTIN_LIGHT;
  return customThemes.find((t) => t.id === id);
}

interface ThemeSelection {
  themeMode: ThemeMode;
  customThemes: Theme[];
  darkThemeId: string;
  lightThemeId: string;
}

/** Resolve the mode + slot assignments to the concrete active theme. */
export function resolveActiveTheme(s: ThemeSelection): Theme {
  const base = effectiveTheme(s.themeMode);
  const slotId = base === "dark" ? s.darkThemeId : s.lightThemeId;
  const fallback = base === "dark" ? BUILTIN_DARK : BUILTIN_LIGHT;
  return findTheme(s.customThemes, slotId) ?? fallback;
}

type PersistedSettings = Pick<
  SettingsState,
  | "themeMode"
  | "customThemes"
  | "darkThemeId"
  | "lightThemeId"
  | "diffFontSize"
  | "defaultViewType"
  | "defaultThreeDot"
  | "botLogins"
>;

interface SettingsState {
  themeMode: ThemeMode;
  customThemes: Theme[];
  darkThemeId: string;
  lightThemeId: string;
  diffFontSize: number;
  defaultViewType: DiffViewType;
  defaultThreeDot: boolean;
  /** Comma-separated GitHub logins treated as bots in the inbox "Bots" bucket. */
  botLogins: string;
  setThemeMode: (m: ThemeMode) => void;
  setDarkThemeId: (id: string) => void;
  setLightThemeId: (id: string) => void;
  /** Duplicate an existing theme into a new editable custom theme; returns its id. */
  addThemeFrom: (sourceId: string, name: string) => string;
  renameTheme: (id: string, name: string) => void;
  deleteTheme: (id: string) => void;
  updateTheme: (id: string, patch: ThemePatch) => void;
  setDiffFontSize: (n: number) => void;
  setDefaultViewType: (v: DiffViewType) => void;
  setDefaultThreeDot: (b: boolean) => void;
  setBotLogins: (s: string) => void;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      themeMode: "dark",
      customThemes: [],
      darkThemeId: BUILTIN_DARK_ID,
      lightThemeId: BUILTIN_LIGHT_ID,
      diffFontSize: DEFAULT_DIFF_FONT_SIZE,
      defaultViewType: "split",
      defaultThreeDot: true,
      botLogins: "",
      setThemeMode: (themeMode) => set({ themeMode }),
      setDarkThemeId: (darkThemeId) => set({ darkThemeId }),
      setLightThemeId: (lightThemeId) => set({ lightThemeId }),
      addThemeFrom: (sourceId, name) => {
        const id = makeId();
        set((s) => {
          const src = findTheme(s.customThemes, sourceId) ?? BUILTIN_DARK;
          return { customThemes: [...s.customThemes, cloneTheme(src, id, name)] };
        });
        return id;
      },
      renameTheme: (id, name) =>
        set((s) => ({
          customThemes: s.customThemes.map((t) => (t.id === id ? { ...t, name } : t)),
        })),
      deleteTheme: (id) =>
        set((s) => {
          if (!s.customThemes.some((t) => t.id === id)) return {};
          return {
            customThemes: s.customThemes.filter((t) => t.id !== id),
            darkThemeId: s.darkThemeId === id ? BUILTIN_DARK_ID : s.darkThemeId,
            lightThemeId: s.lightThemeId === id ? BUILTIN_LIGHT_ID : s.lightThemeId,
          };
        }),
      updateTheme: (id, patch) =>
        set((s) => ({
          customThemes: s.customThemes.map((t) => {
            if (t.id !== id || t.builtin) return t;
            return {
              ...t,
              ...(patch.name !== undefined ? { name: patch.name } : {}),
              ...(patch.base !== undefined ? { base: patch.base } : {}),
              ...(patch.codeFont !== undefined ? { codeFont: patch.codeFont } : {}),
              ui: patch.ui ? { ...t.ui, ...patch.ui } : t.ui,
              syntax: patch.syntax ? { ...t.syntax, ...patch.syntax } : t.syntax,
            };
          }),
        })),
      setDiffFontSize: (diffFontSize) => set({ diffFontSize }),
      setDefaultViewType: (defaultViewType) => set({ defaultViewType }),
      setDefaultThreeDot: (defaultThreeDot) => set({ defaultThreeDot }),
      setBotLogins: (botLogins) => set({ botLogins }),
    }),
    {
      name: "codereview-settings",
      version: 1,
      partialize: (s) => ({
        themeMode: s.themeMode,
        customThemes: s.customThemes,
        darkThemeId: s.darkThemeId,
        lightThemeId: s.lightThemeId,
        diffFontSize: s.diffFontSize,
        defaultViewType: s.defaultViewType,
        defaultThreeDot: s.defaultThreeDot,
        botLogins: s.botLogins,
      }),
      migrate: (persisted, version) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        if (version >= 1) return p as unknown as PersistedSettings;

        // v0 was a flat shape: { theme, reviewTabColor, diffFontSize, ... }.
        const mode = (p.theme as ThemeMode) ?? "dark";
        const next: PersistedSettings = {
          themeMode: mode,
          customThemes: [],
          darkThemeId: BUILTIN_DARK_ID,
          lightThemeId: BUILTIN_LIGHT_ID,
          diffFontSize: (p.diffFontSize as number) ?? DEFAULT_DIFF_FONT_SIZE,
          defaultViewType: (p.defaultViewType as DiffViewType) ?? "split",
          defaultThreeDot: (p.defaultThreeDot as boolean) ?? true,
          botLogins: (p.botLogins as string) ?? "",
        };

        // Preserve a customized review-tab color by forking a theme for it.
        const rtc = (p.reviewTabColor as string | undefined)?.toLowerCase();
        if (rtc && rtc !== BUILTIN_DARK.ui.reviewTabAccent) {
          const base: ThemeBase = mode === "light" ? "light" : "dark";
          const src = base === "light" ? BUILTIN_LIGHT : BUILTIN_DARK;
          const id = makeId();
          next.customThemes = [
            cloneTheme(src, id, `${src.name} (custom)`, { reviewTabAccent: rtc }),
          ];
          if (base === "light") next.lightThemeId = id;
          else next.darkThemeId = id;
        }
        return next;
      },
    },
  ),
);

/** Parse the configured bot logins into a lowercased set. */
export function parseBotLogins(raw: string): Set<string> {
  return new Set(
    raw
      .split(",")
      .map((s) => s.trim().toLowerCase())
      .filter(Boolean),
  );
}

/** Resolve `"system"` to the OS preference; pass-through otherwise. */
export function effectiveTheme(theme: ThemeMode): "dark" | "light" {
  if (theme !== "system") return theme;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
