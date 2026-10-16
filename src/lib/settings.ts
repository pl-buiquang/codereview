import { create } from "zustand";
import { createJSONStorage, persist, type StateStorage } from "zustand/middleware";
import { api } from "./api";
import { DIRECTIONS, type Direction } from "./themes";
import { DEFAULT_CONFIG_PATTERNS, DEFAULT_TEST_PATTERNS } from "./fileCategory";

export type ThemeMode = "dark" | "light" | "system";
export type ThemeBase = "dark" | "light";
export type DiffViewType = "split" | "unified";
/** Which edge of a board card shows the PR-status color. */
export type BoardBorderSide = "left" | "right" | "top" | "bottom";
/** Which tab the review's left pane shows: the file list or the board card's context. */
export type LeftPaneTab = "files" | "context";
/** Terminal application used to open new terminals (backend `TerminalHost`). */
export type TerminalHost = "iterm2" | "ghostty" | "terminal" | "terminator";

export const TERMINAL_HOST_OPTIONS: { label: string; value: TerminalHost }[] = [
  { label: "iTerm2", value: "iterm2" },
  { label: "Ghostty", value: "ghostty" },
  { label: "Terminal.app", value: "terminal" },
  { label: "Terminator (Linux)", value: "terminator" },
];

/** Which tmux session new terminals open in as a window (backend `TmuxTarget`). */
export type TmuxTarget = "current" | "shared" | "per_repo";

export const TMUX_TARGET_OPTIONS: { label: string; value: TmuxTarget }[] = [
  { label: "Current tmux session", value: "current" },
  { label: "Shared session", value: "shared" },
  { label: "Session per repository", value: "per_repo" },
];

export const DEFAULT_DIFF_FONT_SIZE = 12.5;

/** PR-list auto-refresh choices; value is the refetchInterval in ms, 0 = off. */
export const PR_LIST_POLL_OPTIONS: { label: string; value: number }[] = [
  { label: "off", value: 0 },
  { label: "30s", value: 30_000 },
  { label: "60s", value: 60_000 },
  { label: "5m", value: 300_000 },
];

/** Inbox auto-refresh choices. Conservative by design — each refresh is a live
 *  GitHub fan-out (many API calls), so the shortest option is 5 minutes. */
export const INBOX_POLL_OPTIONS: { label: string; value: number }[] = [
  { label: "off", value: 0 },
  { label: "5m", value: 300_000 },
  { label: "15m", value: 900_000 },
  { label: "30m", value: 1_800_000 },
];

/** Board auto-refresh choices — same cadence as the inbox (also a live
 *  GitHub fan-out: tracked cards + review threads). */
export const BOARD_POLL_OPTIONS = INBOX_POLL_OPTIONS;

type PersistedSettings = Pick<
  SettingsState,
  | "direction"
  | "mode"
  | "diffFontSize"
  | "defaultViewType"
  | "defaultThreeDot"
  | "botLogins"
  | "repoStripPrefixes"
  | "repoBasePaths"
  | "prListPollMs"
  | "inboxPollMs"
  | "boardPollMs"
  | "boardStatusBorderSide"
  | "chatModel"
  | "markEdits"
  | "fileTestPatterns"
  | "fileConfigPatterns"
  | "leftPaneTab"
  | "defaultTerminal"
  | "tmuxTarget"
  | "tmuxSessionName"
  | "sessionHistoryDays"
  | "sessionSummaryDir"
>;

const DEFAULTS: PersistedSettings = {
  direction: "a",
  mode: "system",
  diffFontSize: DEFAULT_DIFF_FONT_SIZE,
  defaultViewType: "split",
  defaultThreeDot: true,
  botLogins: "",
  repoStripPrefixes: "",
  repoBasePaths: "~/projects",
  prListPollMs: 0,
  inboxPollMs: 0,
  boardPollMs: 0,
  boardStatusBorderSide: "top",
  chatModel: "",
  markEdits: false,
  fileTestPatterns: DEFAULT_TEST_PATTERNS,
  fileConfigPatterns: DEFAULT_CONFIG_PATTERNS,
  leftPaneTab: "files",
  defaultTerminal: "iterm2",
  tmuxTarget: "current",
  tmuxSessionName: "codereview",
  sessionHistoryDays: 3,
  sessionSummaryDir: "",
};

const PERSISTED_KEYS = Object.keys(DEFAULTS) as (keyof PersistedSettings)[];

interface SettingsState {
  /** Design direction (A/B/C) — selects the `cr-{a|b|c}` token block. */
  direction: Direction;
  /** Color mode; `"system"` follows the OS `prefers-color-scheme`. */
  mode: ThemeMode;
  diffFontSize: number;
  defaultViewType: DiffViewType;
  defaultThreeDot: boolean;
  /** Comma-separated GitHub logins treated as bots in the inbox "Bots" bucket. */
  botLogins: string;
  /** Comma-separated prefixes stripped from repo names shown in the inbox. */
  repoStripPrefixes: string;
  /** Comma-separated base directories where local git clones live (e.g. "~/projects"). */
  repoBasePaths: string;
  /** PR-list auto-refresh interval in ms; 0 = off. */
  prListPollMs: number;
  /** Inbox auto-refresh interval in ms; 0 = off. */
  inboxPollMs: number;
  /** Board auto-refresh interval in ms; 0 = off. */
  boardPollMs: number;
  /** Which edge of a board card shows the PR-status color. */
  boardStatusBorderSide: BoardBorderSide;
  /** Highlight intra-line character-level changes (markEdits). */
  markEdits: boolean;
  /** Model passed to `claude -p --model`. Empty = use env default (opus). */
  chatModel: string;
  /** Newline-separated globs classifying changed files as tests (checked first). */
  fileTestPatterns: string;
  /** Newline-separated globs classifying changed files as config/docs. */
  fileConfigPatterns: string;
  /** Last left-pane tab shown in the review view; remembered globally. */
  leftPaneTab: LeftPaneTab;
  /** Terminal application that attaches tmux (or opens windows when tmux is missing). */
  defaultTerminal: TerminalHost;
  /** Which tmux session new terminals are added to. */
  tmuxTarget: TmuxTarget;
  /** Shared tmux session name; also the fallback for "current" when nothing is attached. */
  tmuxSessionName: string;
  /** How many days of past Claude sessions the Sessions view lists. */
  sessionHistoryDays: number;
  /** Directory of hook-written session summaries (markdown + frontmatter); empty = off. */
  sessionSummaryDir: string;
  setDirection: (d: Direction) => void;
  setMode: (m: ThemeMode) => void;
  setDiffFontSize: (n: number) => void;
  setDefaultViewType: (v: DiffViewType) => void;
  setDefaultThreeDot: (b: boolean) => void;
  setBotLogins: (s: string) => void;
  setRepoStripPrefixes: (s: string) => void;
  setRepoBasePaths: (s: string) => void;
  setPrListPollMs: (ms: number) => void;
  setInboxPollMs: (ms: number) => void;
  setBoardPollMs: (ms: number) => void;
  setBoardStatusBorderSide: (side: BoardBorderSide) => void;
  setChatModel: (s: string) => void;
  setMarkEdits: (b: boolean) => void;
  setFileTestPatterns: (s: string) => void;
  setFileConfigPatterns: (s: string) => void;
  setLeftPaneTab: (tab: LeftPaneTab) => void;
  setDefaultTerminal: (host: TerminalHost) => void;
  setTmuxTarget: (t: TmuxTarget) => void;
  setTmuxSessionName: (s: string) => void;
  setSessionHistoryDays: (days: number) => void;
  setSessionSummaryDir: (s: string) => void;
}

const ENUMS: Partial<Record<keyof PersistedSettings, readonly string[]>> = {
  direction: DIRECTIONS.map((d) => d.id),
  mode: ["dark", "light", "system"],
  defaultViewType: ["split", "unified"],
  boardStatusBorderSide: ["left", "right", "top", "bottom"],
  leftPaneTab: ["files", "context"],
  defaultTerminal: TERMINAL_HOST_OPTIONS.map((o) => o.value),
  tmuxTarget: TMUX_TARGET_OPTIONS.map((o) => o.value),
};

function writeMirror(value: string) {
  try {
    api.writeSettingsMirror(value).catch(() => {});
  } catch {
    // not running inside Tauri
  }
}

/** localStorage, mirrored to a file in the app-data dir so a wiped webview store can recover. */
const mirroredStorage: StateStorage = {
  getItem: (name) => {
    const local = localStorage.getItem(name);
    if (local !== null) {
      writeMirror(local);
      return local;
    }
    try {
      return api.readSettingsMirror().catch(() => null);
    } catch {
      return null;
    }
  },
  setItem: (name, value) => {
    localStorage.setItem(name, value);
    writeMirror(value);
  },
  removeItem: (name) => localStorage.removeItem(name),
};

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      ...DEFAULTS,
      setDirection: (direction) => set({ direction }),
      setMode: (mode) => set({ mode }),
      setDiffFontSize: (diffFontSize) => set({ diffFontSize }),
      setDefaultViewType: (defaultViewType) => set({ defaultViewType }),
      setDefaultThreeDot: (defaultThreeDot) => set({ defaultThreeDot }),
      setBotLogins: (botLogins) => set({ botLogins }),
      setRepoStripPrefixes: (repoStripPrefixes) => set({ repoStripPrefixes }),
      setRepoBasePaths: (repoBasePaths) => set({ repoBasePaths }),
      setPrListPollMs: (prListPollMs) => set({ prListPollMs }),
      setInboxPollMs: (inboxPollMs) => set({ inboxPollMs }),
      setBoardPollMs: (boardPollMs) => set({ boardPollMs }),
      setBoardStatusBorderSide: (boardStatusBorderSide) => set({ boardStatusBorderSide }),
      setChatModel: (chatModel) => set({ chatModel }),
      setMarkEdits: (markEdits) => set({ markEdits }),
      setFileTestPatterns: (fileTestPatterns) => set({ fileTestPatterns }),
      setFileConfigPatterns: (fileConfigPatterns) => set({ fileConfigPatterns }),
      setLeftPaneTab: (leftPaneTab) => set({ leftPaneTab }),
      setDefaultTerminal: (defaultTerminal) => set({ defaultTerminal }),
      setTmuxTarget: (tmuxTarget) => set({ tmuxTarget }),
      setTmuxSessionName: (tmuxSessionName) => set({ tmuxSessionName }),
      setSessionHistoryDays: (sessionHistoryDays) => set({ sessionHistoryDays }),
      setSessionSummaryDir: (sessionSummaryDir) => set({ sessionSummaryDir }),
    }),
    {
      name: "codereview-settings",
      version: 10,
      storage: createJSONStorage(() => mirroredStorage),
      partialize: (s) => pickPersisted(s),
      // Adding a key needs no version bump: the default merge fills it from DEFAULTS.
      // Only bump for renames/reshapes, and keep every other persisted value.
      migrate: (persisted, version) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        const out: PersistedSettings = { ...DEFAULTS, ...sanitizeSettings(p) };
        if (version < 2) {
          // v0 (flat `theme`) / v1 (`themeMode` + theme slots): only the mode carries over.
          const oldMode = version < 1 ? p.theme : p.themeMode;
          out.direction = "a";
          out.mode = oldMode === "dark" || oldMode === "light" ? oldMode : "system";
        }
        return out;
      },
    },
  ),
);

function pickPersisted(s: PersistedSettings): PersistedSettings {
  return Object.fromEntries(PERSISTED_KEYS.map((k) => [k, s[k]])) as PersistedSettings;
}


/** Serialize the persisted settings to a portable JSON document. */
export function exportSettings(): string {
  return JSON.stringify(
    {
      app: "codereview",
      kind: "settings",
      exportedAt: new Date().toISOString(),
      settings: pickPersisted(useSettingsStore.getState()),
    },
    null,
    2,
  );
}

/** Apply settings from an exported JSON document (or a bare settings object).
 *  Unknown or mistyped keys are skipped; returns how many were applied. */
export function importSettings(json: string): number {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    throw new Error("Not a valid JSON file");
  }
  if (!parsed || typeof parsed !== "object") throw new Error("No settings found in file");
  const root = parsed as Record<string, unknown>;
  const source = (
    root.settings && typeof root.settings === "object" ? root.settings : root
  ) as Record<string, unknown>;

  const picked = sanitizeSettings(source);
  const count = Object.keys(picked).length;
  if (count === 0) throw new Error("No recognized settings in file");
  useSettingsStore.setState(picked);
  return count;
}

/** Known keys whose value has the default's type (and a valid enum value); the rest is dropped. */
function sanitizeSettings(source: Record<string, unknown>): Partial<PersistedSettings> {
  const picked: Record<string, unknown> = {};
  for (const key of PERSISTED_KEYS) {
    const value = source[key];
    if (typeof value !== typeof DEFAULTS[key]) continue;
    if (typeof value === "number" && !Number.isFinite(value)) continue;
    const allowed = ENUMS[key];
    if (allowed && !allowed.includes(value as string)) continue;
    picked[key] = value;
  }
  return picked as Partial<PersistedSettings>;
}

/** Parse the configured repository base paths. */
export function parseRepoBasePaths(raw: string): string[] {
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Parse the configured bot logins into a lowercased set. */
export function parseBotLogins(raw: string): Set<string> {
  return new Set(
    raw
      .split(",")
      .map((s) => s.trim().toLowerCase())
      .filter(Boolean),
  );
}

/** Parse the configured repo-name prefixes. Case-sensitive and order-significant
 *  (first match wins in `stripRepoPrefix`), unlike `parseBotLogins`. */
export function parseRepoStripPrefixes(raw: string): string[] {
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

/** Strip the first matching prefix from a repo name. Returns the original name
 *  (and `stripped: false`) when none match. */
export function stripRepoPrefix(
  name: string,
  prefixes: string[],
): { display: string; stripped: boolean } {
  for (const p of prefixes) {
    if (name.startsWith(p)) return { display: name.slice(p.length), stripped: true };
  }
  return { display: name, stripped: false };
}

/** Resolve `"system"` to the OS preference; pass-through otherwise. */
export function effectiveTheme(theme: ThemeMode): "dark" | "light" {
  if (theme !== "system") return theme;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
