import { create } from "zustand";
import { createJSONStorage, persist, type StateStorage } from "zustand/middleware";
import { api } from "./api";
import { DIRECTIONS, type Direction } from "./themes";
import { DEFAULT_CONFIG_PATTERNS, DEFAULT_TEST_PATTERNS } from "./fileCategory";
import type { AutoSync } from "./types";
import { DEFAULT_LOG_LEVEL, type LogLevel } from "./log";

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

/** Auto Sync modes (the backend sync engine's schedule). */
export const AUTO_SYNC_OPTIONS: { label: string; value: AutoSync }[] = [
  { label: "Off", value: "off" },
  { label: "On focus", value: "on_focus" },
  { label: "Periodic", value: "periodic" },
];

/** Verbosity of the app log file (Settings → Logs). */
export const LOG_LEVEL_OPTIONS: { label: string; value: LogLevel }[] = [
  { label: "Error", value: "error" },
  { label: "Warning", value: "warn" },
  { label: "Info", value: "info" },
  { label: "Debug (verbose)", value: "debug" },
];

/** Discovery ("look for new PRs/issues") interval choices, in minutes; 0 = off. */
export const DISCOVERY_INTERVAL_OPTIONS: { label: string; value: number }[] = [
  { label: "off", value: 0 },
  { label: "5 min", value: 5 },
  { label: "15 min", value: 15 },
  { label: "30 min", value: 30 },
  { label: "60 min", value: 60 },
];

/** Board auto-merge monitor round interval choices, in minutes. */
export const BOARD_MONITOR_OPTIONS: { label: string; value: number }[] = [
  { label: "2 min", value: 2 },
  { label: "5 min", value: 5 },
  { label: "10 min", value: 10 },
  { label: "15 min", value: 15 },
  { label: "30 min", value: 30 },
];

/** Background Fetch choices, in minutes; 0 = off (the default). */
export const AUTO_FETCH_OPTIONS: { label: string; value: number }[] = [
  { label: "off", value: 0 },
  { label: "5 min", value: 5 },
  { label: "15 min", value: 15 },
  { label: "30 min", value: 30 },
];

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
  | "autoSync"
  | "discoveryIntervalMin"
  | "autoFetchMinutes"
  | "logLevel"
  | "boardStatusBorderSide"
  | "chatModel"
  | "overviewModel"
  | "boardOverviewOpen"
  | "boardMonitor"
  | "boardMonitorIntervalMin"
  | "markEdits"
  | "largeDiffMaxLines"
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
  autoSync: "periodic",
  discoveryIntervalMin: 15,
  autoFetchMinutes: 0,
  logLevel: DEFAULT_LOG_LEVEL,
  boardStatusBorderSide: "top",
  chatModel: "",
  overviewModel: "sonnet",
  boardOverviewOpen: false,
  boardMonitor: false,
  boardMonitorIntervalMin: 5,
  markEdits: false,
  largeDiffMaxLines: 3000,
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
  /** When the GitHub mirror syncs on its own: never, on view mount/window
   *  focus, or also periodically while the app is visible. */
  autoSync: AutoSync;
  /** Minutes between discoveries (searches for new PRs/issues); 0 = off. */
  discoveryIntervalMin: number;
  /** Verbosity of the app log file; applied to the backend by `useApplySettings`. */
  logLevel: LogLevel;
  /** Fetch a repo when its view opens or regains focus and the last fetch is
   *  older than this many minutes; 0 = never fetch in the background. */
  autoFetchMinutes: number;
  /** Which edge of a board card shows the PR-status color. */
  boardStatusBorderSide: BoardBorderSide;
  /** Highlight intra-line character-level changes (markEdits). */
  markEdits: boolean;
  /** Files with more changed lines (add + del) are not rendered in the diff. */
  largeDiffMaxLines: number;
  /** Model passed to `claude -p --model`. Empty = use env default (opus). */
  chatModel: string;
  /** Model for the board AI overview; empty = same resolution as chat. */
  overviewModel: string;
  boardOverviewOpen: boolean;
  /** Keep approved auto-merge PRs mergeable: update BEHIND branches, rerun failed CI. */
  boardMonitor: boolean;
  boardMonitorIntervalMin: number;
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
  setAutoSync: (mode: AutoSync) => void;
  setLogLevel: (level: LogLevel) => void;
  setDiscoveryIntervalMin: (minutes: number) => void;
  setAutoFetchMinutes: (minutes: number) => void;
  setBoardStatusBorderSide: (side: BoardBorderSide) => void;
  setChatModel: (s: string) => void;
  setMarkEdits: (b: boolean) => void;
  setLargeDiffMaxLines: (n: number) => void;
  setFileTestPatterns: (s: string) => void;
  setFileConfigPatterns: (s: string) => void;
  setLeftPaneTab: (tab: LeftPaneTab) => void;
  setDefaultTerminal: (host: TerminalHost) => void;
  setTmuxTarget: (t: TmuxTarget) => void;
  setTmuxSessionName: (s: string) => void;
  setSessionHistoryDays: (days: number) => void;
  setSessionSummaryDir: (s: string) => void;
  setOverviewModel: (m: string) => void;
  setBoardOverviewOpen: (open: boolean) => void;
  setBoardMonitor: (on: boolean) => void;
  setBoardMonitorIntervalMin: (min: number) => void;
}

const ENUMS: Partial<Record<keyof PersistedSettings, readonly string[]>> = {
  direction: DIRECTIONS.map((d) => d.id),
  mode: ["dark", "light", "system"],
  defaultViewType: ["split", "unified"],
  boardStatusBorderSide: ["left", "right", "top", "bottom"],
  leftPaneTab: ["files", "context"],
  defaultTerminal: TERMINAL_HOST_OPTIONS.map((o) => o.value),
  tmuxTarget: TMUX_TARGET_OPTIONS.map((o) => o.value),
  autoSync: AUTO_SYNC_OPTIONS.map((o) => o.value),
  logLevel: LOG_LEVEL_OPTIONS.map((o) => o.value),
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
      setAutoSync: (autoSync) => set({ autoSync }),
      setLogLevel: (logLevel) => set({ logLevel }),
      setDiscoveryIntervalMin: (discoveryIntervalMin) => set({ discoveryIntervalMin }),
      setAutoFetchMinutes: (autoFetchMinutes) => set({ autoFetchMinutes }),
      setBoardStatusBorderSide: (boardStatusBorderSide) => set({ boardStatusBorderSide }),
      setChatModel: (chatModel) => set({ chatModel }),
      setMarkEdits: (markEdits) => set({ markEdits }),
      setLargeDiffMaxLines: (largeDiffMaxLines) => set({ largeDiffMaxLines }),
      setFileTestPatterns: (fileTestPatterns) => set({ fileTestPatterns }),
      setFileConfigPatterns: (fileConfigPatterns) => set({ fileConfigPatterns }),
      setLeftPaneTab: (leftPaneTab) => set({ leftPaneTab }),
      setDefaultTerminal: (defaultTerminal) => set({ defaultTerminal }),
      setTmuxTarget: (tmuxTarget) => set({ tmuxTarget }),
      setTmuxSessionName: (tmuxSessionName) => set({ tmuxSessionName }),
      setSessionHistoryDays: (sessionHistoryDays) => set({ sessionHistoryDays }),
      setSessionSummaryDir: (sessionSummaryDir) => set({ sessionSummaryDir }),
      setOverviewModel: (overviewModel) => set({ overviewModel }),
      setBoardOverviewOpen: (boardOverviewOpen) => set({ boardOverviewOpen }),
      setBoardMonitor: (boardMonitor) => set({ boardMonitor }),
      setBoardMonitorIntervalMin: (boardMonitorIntervalMin) => set({ boardMonitorIntervalMin }),
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


/** Serialize the persisted settings (and AI prompt overrides, keyed by prompt) to a portable JSON document. */
export function exportSettings(prompts: Record<string, string> = {}): string {
  return JSON.stringify(
    {
      app: "codereview",
      kind: "settings",
      exportedAt: new Date().toISOString(),
      settings: pickPersisted(useSettingsStore.getState()),
      prompts,
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

/** Prompt overrides from an exported document; null when the file predates prompt export. */
export function parsePromptOverrides(json: string): Record<string, string> | null {
  const root = JSON.parse(json) as Record<string, unknown> | null;
  const prompts = root && typeof root === "object" ? root.prompts : undefined;
  if (!prompts || typeof prompts !== "object") return null;
  return Object.fromEntries(
    Object.entries(prompts as Record<string, unknown>).filter(
      (e): e is [string, string] => typeof e[1] === "string",
    ),
  );
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
