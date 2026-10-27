import type { Direction } from "./themes";

/**
 * The custom theme: a built-in direction (`base`) plus a few overrides. Colours are kept per
 * mode (`dark` / `light`) so following the OS keeps working; the knobs (fonts, size, corners)
 * apply to both. Overrides become inline CSS variables on top of the base `cr-{dir} {mode}`
 * classes (`customThemeStyle`), with the derived shades (hover, soft, borders…) mixed from
 * the picked colours via `color-mix`. `"base"` / an absent colour leaves the base value.
 */
export const CUSTOM_KNOBS = {
  uiFont: ["base", "atkinson", "inter", "plex", "geist", "jetbrains", "system"],
  monoFont: ["base", "atkinson-mono", "jetbrains", "plex-mono", "geist-mono", "system"],
  textSize: ["base", "small", "large"],
  corners: ["base", "square", "round"],
  /** Font of repo names, branches and other identifiers: the UI font or the code font. */
  identFont: ["base", "ui", "mono"],
  shadows: ["base", "flat", "soft", "strong"],
  /** Backdrop behind dialogs and popovers. */
  scrim: ["base", "light", "strong"],
} as const;

export type CustomKnob = keyof typeof CUSTOM_KNOBS;
export const CUSTOM_KNOB_NAMES = Object.keys(CUSTOM_KNOBS) as CustomKnob[];

/** Colours of the Colours sub-tab, by group. */
export const CORE_COLOURS = ["accent", "background", "surface", "text"] as const;
export const STATUS_COLOURS = ["success", "warning", "danger", "info"] as const;
export const DIFF_COLOURS = ["diffAdd", "diffDel"] as const;
/** The 8 syntax roles (`--tok-*`). */
export const SYNTAX_COLOURS = [
  "tokComment",
  "tokPunctuation",
  "tokLiteral",
  "tokString",
  "tokOperator",
  "tokKeyword",
  "tokFunction",
  "tokVariable",
] as const;

export const PALETTE_COLOURS = [...CORE_COLOURS, ...STATUS_COLOURS, ...DIFF_COLOURS] as const;
export const CUSTOM_COLOURS = [...PALETTE_COLOURS, ...SYNTAX_COLOURS] as const;

export type CustomColour = (typeof CUSTOM_COLOURS)[number];
/** `#rrggbb` per overridden colour. */
export type CustomPalette = Partial<Record<CustomColour, string>>;
export type CustomMode = "dark" | "light";

export type CustomTheme = {
  base: Direction;
  dark: CustomPalette;
  light: CustomPalette;
} & { [K in CustomKnob]: (typeof CUSTOM_KNOBS)[K][number] };

export const DEFAULT_CUSTOM_THEME: CustomTheme = {
  base: "a",
  dark: {},
  light: {},
  uiFont: "base",
  monoFont: "base",
  textSize: "base",
  corners: "base",
  identFont: "base",
  shadows: "base",
  scrim: "base",
};

/** The scopes the Theme editor (and AI suggest) work on. */
export type CustomScope = "all" | "colours" | "syntax" | "typography" | "shape";

const BASE_DIRECTIONS: readonly string[] = ["a", "b", "c", "d"];
const HEX = /^#[0-9a-f]{6}$/;

/** A `#rrggbb` colour from `#rgb`, `#rrggbb` or `rgb[a](r, g, b…)`, else null. */
export function toHexColour(raw: string): string | null {
  const value = raw.trim().toLowerCase();
  if (HEX.test(value)) return value;
  const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/.exec(value);
  if (short) return `#${short[1]}${short[1]}${short[2]}${short[2]}${short[3]}${short[3]}`;
  const rgb = /^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/.exec(value);
  if (!rgb) return null;
  return `#${rgb
    .slice(1, 4)
    .map((n) => Math.min(255, Number(n)).toString(16).padStart(2, "0"))
    .join("")}`;
}

function parsePalette(raw: unknown): CustomPalette {
  const palette: CustomPalette = {};
  if (typeof raw !== "object" || raw === null) return palette;
  const value = raw as Record<string, unknown>;
  for (const colour of CUSTOM_COLOURS) {
    // `info` was briefly saved as `reviewRequired`.
    const v = value[colour] ?? (colour === "info" ? value.reviewRequired : undefined);
    const hex = typeof v === "string" ? toHexColour(v) : null;
    if (hex) palette[colour] = hex;
  }
  return palette;
}

/** An unknown value, field by field: anything missing or invalid takes its default. */
export function parseCustomTheme(raw: unknown): CustomTheme {
  const value = typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : {};
  const theme: CustomTheme = { ...DEFAULT_CUSTOM_THEME, dark: parsePalette(value.dark), light: parsePalette(value.light) };
  if (typeof value.base === "string" && BASE_DIRECTIONS.includes(value.base)) theme.base = value.base as Direction;
  for (const knob of CUSTOM_KNOB_NAMES) {
    const v = value[knob];
    if ((CUSTOM_KNOBS[knob] as readonly unknown[]).includes(v)) Object.assign(theme, { [knob]: v });
  }
  return theme;
}

/** The theme as a shareable JSON document. */
export function exportCustomTheme(theme: CustomTheme): string {
  return JSON.stringify(
    { app: "codereview", kind: "custom-theme", exportedAt: new Date().toISOString(), theme },
    null,
    2,
  );
}

/** The theme in an exported document (or a bare theme object). Invalid fields take their
 *  default; throws when the text isn't JSON or holds no theme. */
export function importCustomTheme(json: string): CustomTheme {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    throw new Error("Not a valid JSON file");
  }
  const root = typeof parsed === "object" && parsed !== null ? (parsed as Record<string, unknown>) : null;
  const raw = root && typeof root.theme === "object" ? root.theme : root;
  if (!raw || !("dark" in raw || "light" in raw || "base" in raw)) throw new Error("No custom theme found in file");
  return parseCustomTheme(raw);
}

/** WCAG relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = Number.parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrastRatio(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** `light` when dark text reads better on the colour than white text. */
export function accentTone(hex: string): "light" | "dark" {
  const l = luminance(hex);
  return (l + 0.05) / 0.05 > 1.05 / (l + 0.05) ? "light" : "dark";
}

/**
 * Pairs that read badly, among the colours the theme sets (an unset colour has its base
 * theme's value, which is assumed fine): body text on the background and surface, and syntax
 * roles on the surface (comments may be quieter).
 */
export function contrastIssues(theme: CustomTheme): string[] {
  const issues: string[] = [];
  for (const mode of ["dark", "light"] as const) {
    const p = theme[mode];
    const check = (fg: CustomColour, bg: CustomColour, min: number) => {
      const a = p[fg];
      const b = p[bg];
      if (!a || !b) return;
      const ratio = contrastRatio(a, b);
      if (ratio < min) issues.push(`${mode}: ${fg} on ${bg} is ${ratio.toFixed(1)}:1 (needs ${min}:1)`);
    };
    check("text", "background", 4.5);
    check("text", "surface", 4.5);
    for (const role of SYNTAX_COLOURS) check(role, "surface", role === "tokComment" ? 2.5 : 3);
  }
  return issues;
}

const UI_FONT_STACKS: Record<Exclude<(typeof CUSTOM_KNOBS.uiFont)[number], "base">, string> = {
  atkinson: '"Atkinson Hyperlegible", sans-serif',
  inter: '"Inter", sans-serif',
  plex: '"IBM Plex Sans", sans-serif',
  geist: '"Geist Sans", sans-serif',
  jetbrains: '"JetBrains Mono", monospace',
  system: '-apple-system, BlinkMacSystemFont, "Segoe UI", "Noto Sans", Helvetica, Arial, sans-serif',
};

const MONO_FONT_STACKS: Record<Exclude<(typeof CUSTOM_KNOBS.monoFont)[number], "base">, string> = {
  "atkinson-mono": '"Atkinson Hyperlegible Mono", monospace',
  jetbrains: '"JetBrains Mono", monospace',
  "plex-mono": '"IBM Plex Mono", monospace',
  "geist-mono": '"Geist Mono", monospace',
  system: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "Liberation Mono", monospace',
};

const SCALE = { small: "0.92", large: "1.1" } as const;
const RADII = {
  square: ["0px", "2px", "3px"],
  round: ["8px", "12px", "18px"],
} as const;

/** `[--shadow-card, --shadow-pop]` per level; black at an alpha reads on both modes. */
const SHADOWS = {
  flat: ["none", "0 0 0 1px var(--border-strong)"],
  soft: ["0 1px 2px rgba(0,0,0,0.12)", "0 8px 24px rgba(0,0,0,0.25)"],
  strong: ["0 2px 6px rgba(0,0,0,0.3)", "0 16px 40px rgba(0,0,0,0.55)"],
} as const;
const SCRIMS = { light: "rgba(0,0,0,0.25)", strong: "rgba(0,0,0,0.7)" } as const;
const IDENT_FONTS = { ui: "var(--font-ui)", mono: "var(--font-mono)" } as const;

const mix = (colour: string, pct: number, other: string, space = "oklab") =>
  `color-mix(in ${space}, ${colour} ${pct}%, ${other})`;
const tint = (colour: string, pct: number) => mix(colour, pct, "transparent", "srgb");

/** Token each colour replaces: what its picker shows while unset (the CSS hex/rgb of the base). */
export const COLOUR_TOKEN: Record<CustomColour, string> = {
  accent: "--accent",
  background: "--bg",
  surface: "--surface",
  text: "--text",
  success: "--success",
  warning: "--warning",
  danger: "--danger",
  info: "--info",
  diffAdd: "--diff-add-strong",
  diffDel: "--diff-del-strong",
  tokComment: "--tok-comment",
  tokPunctuation: "--tok-punctuation",
  tokLiteral: "--tok-literal",
  tokString: "--tok-string",
  tokOperator: "--tok-operator",
  tokKeyword: "--tok-keyword",
  tokFunction: "--tok-function",
  tokVariable: "--tok-variable",
};

/**
 * Inline style (CSS variables, plus `zoom` for the text size) that paints `theme` in `mode`
 * over its base direction's classes. `preview` leaves out the scale so a picker card keeps
 * its size.
 */
export function customThemeStyle(
  theme: CustomTheme,
  mode: CustomMode,
  opts: { preview?: boolean } = {},
): Record<string, string> {
  const p = theme[mode];
  const s: Record<string, string> = {};
  if (p.accent) {
    s["--accent"] = p.accent;
    s["--accent-hover"] = mix(p.accent, 82, "var(--text)");
    s["--on-accent"] = accentTone(p.accent) === "light" ? "#15181d" : "#ffffff";
    s["--accent-soft"] = tint(p.accent, 13);
    s["--accent-border"] = tint(p.accent, 42);
  }
  if (p.background) {
    s["--bg"] = p.background;
    s["--bg-sunken"] = mix(p.background, 92, "black");
    s["--titlebar"] = mix(p.background, 92, "black");
  }
  if (p.surface) {
    s["--surface"] = p.surface;
    s["--surface-2"] = mix(p.surface, 95, "var(--text)");
    s["--surface-3"] = mix(p.surface, 90, "var(--text)");
    s["--border"] = mix(p.surface, 86, "var(--text)");
    s["--border-strong"] = mix(p.surface, 70, "var(--text)");
  }
  if (p.text) {
    s["--text"] = p.text;
    s["--text-2"] = mix(p.text, 68, "var(--bg)");
    s["--text-3"] = mix(p.text, 44, "var(--bg)");
  }
  for (const status of STATUS_COLOURS) {
    const c = p[status];
    if (!c) continue;
    s[`--${status}`] = c;
    s[`--${status}-soft`] = tint(c, 13);
    if (status === "info") {
      s["--info-border"] = tint(c, 42);
      s["--on-info"] = accentTone(c) === "light" ? "#15181d" : "#ffffff";
    }
  }
  if (p.diffAdd) {
    s["--diff-add-bg"] = tint(p.diffAdd, 12);
    s["--diff-add-strong"] = tint(p.diffAdd, 28);
  }
  if (p.diffDel) {
    s["--diff-del-bg"] = tint(p.diffDel, 12);
    s["--diff-del-strong"] = tint(p.diffDel, 28);
  }
  for (const role of SYNTAX_COLOURS) {
    const c = p[role];
    if (c) s[`--tok-${role.slice(3).toLowerCase()}`] = c;
  }
  if (theme.uiFont !== "base") {
    s["--font-ui"] = UI_FONT_STACKS[theme.uiFont];
    s["--font-display"] = UI_FONT_STACKS[theme.uiFont];
  }
  if (theme.monoFont !== "base") s["--font-mono"] = MONO_FONT_STACKS[theme.monoFont];
  if (theme.corners !== "base") {
    const [sm, md, lg] = RADII[theme.corners];
    s["--radius-s"] = sm;
    s["--radius-m"] = md;
    s["--radius-l"] = lg;
  }
  if (theme.identFont !== "base") s["--font-ident"] = IDENT_FONTS[theme.identFont];
  if (theme.shadows !== "base") [s["--shadow-card"], s["--shadow-pop"]] = SHADOWS[theme.shadows];
  if (theme.scrim !== "base") s["--scrim"] = SCRIMS[theme.scrim];
  if (theme.textSize !== "base" && !opts.preview) s["zoom"] = SCALE[theme.textSize];
  return s;
}

/** The fields a scope covers; `all` covers everything. */
const SCOPE_COLOURS: Record<CustomScope, readonly CustomColour[]> = {
  all: CUSTOM_COLOURS,
  colours: PALETTE_COLOURS,
  syntax: SYNTAX_COLOURS,
  typography: [],
  shape: [],
};

const SCOPE_KNOBS: Record<CustomScope, readonly CustomKnob[]> = {
  all: CUSTOM_KNOB_NAMES,
  colours: [],
  syntax: [],
  typography: ["uiFont", "monoFont", "identFont", "textSize"],
  shape: ["corners", "shadows", "scrim"],
};

/** `current` with the fields `scope` covers taken from `suggested` (both modes' colours). */
export function mergeScope(current: CustomTheme, suggested: CustomTheme, scope: CustomScope): CustomTheme {
  if (scope === "all") return suggested;
  const next: CustomTheme = { ...current, dark: { ...current.dark }, light: { ...current.light } };
  for (const mode of ["dark", "light"] as const) {
    for (const colour of SCOPE_COLOURS[scope]) {
      const v = suggested[mode][colour];
      if (v) next[mode][colour] = v;
      else delete next[mode][colour];
    }
  }
  for (const knob of SCOPE_KNOBS[scope]) Object.assign(next, { [knob]: suggested[knob] });
  return next;
}

/** The theme with the colours and knobs of `scope` back at the base's values (base stays). */
export function resetScope(current: CustomTheme, scope: CustomScope): CustomTheme {
  return mergeScope(current, { ...DEFAULT_CUSTOM_THEME, base: current.base }, scope);
}
