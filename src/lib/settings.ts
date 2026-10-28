import { create } from "zustand";
import { createJSONStorage, persist, type StateStorage } from "zustand/middleware";
import { api } from "./api";
import { DIRECTIONS, type ThemeChoice } from "./themes";
import { DEFAULT_CUSTOM_THEME, parseCustomTheme, type CustomTheme } from "./customTheme";
import { DEFAULT_CONFIG_PATTERNS, DEFAULT_TEST_PATTERNS } from "./fileCategory";
import type { AutoSync, IgnoreRule } from "./types";
import type { ActionKind } from "./boardDigest";
import { DEFAULT_SNOOZE_OPTIONS } from "./snooze";
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

export const DEFAULT_DIFF_FONT_SIZE = 12;

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

/** What the sidebar Board badge can count, in ranking order. */
export const BOARD_BADGE_KIND_OPTIONS: { kind: ActionKind; label: string; hint: string }[] = [
  { kind: "review", label: "Review requests", hint: "PRs in the Reviews lane that aren't yours and aren't approved" },
  { kind: "merge", label: "Ready to merge", hint: "Your approved PRs without auto-merge" },
  { kind: "conflicts", label: "Merge conflicts", hint: "Your PRs with conflicts" },
  { kind: "ci", label: "Failing CI", hint: "Your PRs with failing CI" },
  { kind: "changes", label: "Changes requested / open threads", hint: "Your PRs with requested changes or unresolved threads" },
  { kind: "behind", label: "Behind the base branch", hint: "Your PRs that need a branch update" },
];

export const DEFAULT_BOARD_BADGE_KINDS: Record<ActionKind, boolean> = {
  review: true,
  merge: true,
  conflicts: true,
  ci: false,
  changes: true,
  behind: false,
};

/** Starting rules, off until switched on in Settings. */
export const DEFAULT_IGNORE_RULES: IgnoreRule[] = [
  { id: "preset-closed-quiet", name: "Closed or merged, quiet for 30 days", enabled: false, subjectClosed: true, olderThanDays: 30 },
  { id: "preset-resolved", name: "Resolved threads", enabled: false, kind: "thread", resolved: true },
  { id: "preset-mine-last", name: "My message is the last", enabled: false, lastAuthor: "me" },
  { id: "preset-bot-commands", name: "Commands to bots", enabled: false, kind: "discussion", lastAuthor: "me", bodyContains: "@dependabot" },
  { id: "preset-approvals", name: "Approvals only", enabled: false, approvalsOnly: true },
  { id: "preset-not-involved", name: "PRs / issues I'm not involved in", enabled: true, involved: false },
];

type PersistedSettings = Pick<
  SettingsState,
  | "direction"
  | "customTheme"
  | "mode"
  | "diffFontSize"
  | "defaultViewType"
  | "defaultThreeDot"
  | "botLogins"
  | "conversationIgnoreRules"
  | "repoStripPrefixes"
  | "repoBasePaths"
  | "autoSync"
  | "discoveryIntervalMin"
  | "autoFetchMinutes"
  | "logLevel"
  | "boardStatusBorderSide"
  | "aiEnabled"
  | "aiEnabled"
  | "chatModel"
  | "overviewModel"
  | "boardOverviewOpen"
  | "boardMonitor"
  | "boardMonitorLogOpen"
  | "boardMonitorIntervalMin"
  | "boardArchiveDays"
  | "boardCountDone"
  | "boardBadgeKinds"
  | "boardSnoozeOptions"
  | "boardSnoozes"
  | "boardNeedsActionSnoozed"
  | "markEdits"
  | "diffMarkers"
  | "largeDiffMaxLines"
  | "fileTestPatterns"
  | "fileConfigPatterns"
  | "leftPaneTab"
  | "defaultTerminal"
  | "tmuxTarget"
  | "tmuxSessionName"
  | "sessionHistoryDays"
  | "sessionSummaryDir"
  | "sessionSummaryFork"
>;

const DEFAULTS: PersistedSettings = {
  direction: "a",
  customTheme: DEFAULT_CUSTOM_THEME,
  mode: "system",
  diffFontSize: DEFAULT_DIFF_FONT_SIZE,
  defaultViewType: "split",
  defaultThreeDot: true,
  botLogins: "",
  conversationIgnoreRules: DEFAULT_IGNORE_RULES,
  repoStripPrefixes: "",
  repoBasePaths: "~/projects",
  autoSync: "periodic",
  discoveryIntervalMin: 5,
  autoFetchMinutes: 0,
  logLevel: DEFAULT_LOG_LEVEL,
  boardStatusBorderSide: "top",
  aiEnabled: false,
  chatModel: "",
  overviewModel: "sonnet",
  boardOverviewOpen: false,
  boardMonitor: false,
  boardMonitorLogOpen: false,
  boardMonitorIntervalMin: 5,
  boardArchiveDays: 7,
  boardCountDone: true,
  boardBadgeKinds: DEFAULT_BOARD_BADGE_KINDS,
  boardSnoozeOptions: DEFAULT_SNOOZE_OPTIONS,
  boardSnoozes: {},
  boardNeedsActionSnoozed: false,
  markEdits: true,
  diffMarkers: true,
  largeDiffMaxLines: 3000,
  fileTestPatterns: DEFAULT_TEST_PATTERNS,
  fileConfigPatterns: DEFAULT_CONFIG_PATTERNS,
  leftPaneTab: "files",
  defaultTerminal: "iterm2",
  tmuxTarget: "current",
  tmuxSessionName: "codereview",
  sessionHistoryDays: 3,
  sessionSummaryDir: "",
  sessionSummaryFork: false,
};

const PERSISTED_KEYS = Object.keys(DEFAULTS) as (keyof PersistedSettings)[];

interface SettingsState {
  /** Design direction (A/B/C/D) — selects the `cr-{a|b|c|d}` token block — or `"custom"`. */
  direction: ThemeChoice;
  /** The user's custom theme, applied when `direction` is `"custom"`. */
  customTheme: CustomTheme;
  /** Color mode; `"system"` follows the OS `prefers-color-scheme`. */
  mode: ThemeMode;
  diffFontSize: number;
  defaultViewType: DiffViewType;
  defaultThreeDot: boolean;
  /** Comma-separated GitHub logins treated as bots in the inbox "Bots" bucket. */
  botLogins: string;
  /** Rules hiding Inbox conversations (Conversations tab). */
  conversationIgnoreRules: IgnoreRule[];
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
  /** Show the +/- marker column in diffs. */
  diffMarkers: boolean;
  /** Files with more changed lines (add + del) are not rendered in the diff. */
  largeDiffMaxLines: number;
  /** Master switch for every app-spawned `claude -p` feature (chat, focus, AI review, overview, routines). */
  aiEnabled: boolean;
  /** Model passed to `claude -p --model`. Empty = use env default (opus). */
  chatModel: string;
  /** Model for the board AI overview; empty = same resolution as chat. */
  overviewModel: string;
  boardOverviewOpen: boolean;
  /** Keep approved auto-merge PRs mergeable: update BEHIND branches, rerun failed CI. */
  boardMonitor: boolean;
  boardMonitorLogOpen: boolean;
  boardMonitorIntervalMin: number;
  boardArchiveDays: number;
  /** Include Done-lane cards in the Board filter counts. */
  boardCountDone: boolean;
  /** Which kinds of cards the sidebar Board badge counts. */
  boardBadgeKinds: Record<ActionKind, boolean>;
  /** Card snooze durations, one line of comma-separated `<n>h|d|w` (see `lib/snooze.ts`). */
  boardSnoozeOptions: string;
  /** Card id → epoch ms until which the card is left out of the Board badge. */
  boardSnoozes: Record<string, number>;
  /** Whether the Board's "Needs action" filter also shows snoozed cards. */
  boardNeedsActionSnoozed: boolean;
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
  /** Summarize live sessions by forking them (reuses the prompt cache when warm; cost varies). */
  sessionSummaryFork: boolean;
  setDirection: (d: ThemeChoice) => void;
  setCustomTheme: (theme: CustomTheme) => void;
  setMode: (m: ThemeMode) => void;
  setDiffFontSize: (n: number) => void;
  setDefaultViewType: (v: DiffViewType) => void;
  setDefaultThreeDot: (b: boolean) => void;
  setBotLogins: (s: string) => void;
  setConversationIgnoreRules: (rules: IgnoreRule[]) => void;
  setRepoStripPrefixes: (s: string) => void;
  setRepoBasePaths: (s: string) => void;
  setAutoSync: (mode: AutoSync) => void;
  setLogLevel: (level: LogLevel) => void;
  setDiscoveryIntervalMin: (minutes: number) => void;
  setAutoFetchMinutes: (minutes: number) => void;
  setBoardStatusBorderSide: (side: BoardBorderSide) => void;
  setAiEnabled: (on: boolean) => void;
  setChatModel: (s: string) => void;
  setMarkEdits: (b: boolean) => void;
  setDiffMarkers: (b: boolean) => void;
  setLargeDiffMaxLines: (n: number) => void;
  setFileTestPatterns: (s: string) => void;
  setFileConfigPatterns: (s: string) => void;
  setLeftPaneTab: (tab: LeftPaneTab) => void;
  setDefaultTerminal: (host: TerminalHost) => void;
  setTmuxTarget: (t: TmuxTarget) => void;
  setTmuxSessionName: (s: string) => void;
  setSessionHistoryDays: (days: number) => void;
  setSessionSummaryDir: (s: string) => void;
  setSessionSummaryFork: (on: boolean) => void;
  setOverviewModel: (m: string) => void;
  setBoardOverviewOpen: (open: boolean) => void;
  setBoardNeedsActionSnoozed: (on: boolean) => void;
  setBoardMonitor: (on: boolean) => void;
  setBoardMonitorLogOpen: (open: boolean) => void;
  setBoardMonitorIntervalMin: (min: number) => void;
  setBoardArchiveDays: (days: number) => void;
  setBoardCountDone: (on: boolean) => void;
  setBoardBadgeKind: (kind: ActionKind, on: boolean) => void;
  setBoardSnoozeOptions: (s: string) => void;
  snoozeCard: (cardId: string, until: number) => void;
  unsnoozeCard: (cardId: string) => void;
}

const ENUMS: Partial<Record<keyof PersistedSettings, readonly string[]>> = {
  direction: [...DIRECTIONS.map((d) => d.id), "custom"],
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

/** Colour pickers change settings on every drag frame: coalesce the file writes. */
const MIRROR_DEBOUNCE_MS = 300;
let mirrorTimer: ReturnType<typeof setTimeout> | undefined;
let mirrorPending: string | null = null;

function writeMirrorDebounced(value: string) {
  mirrorPending = value;
  clearTimeout(mirrorTimer);
  mirrorTimer = setTimeout(() => {
    if (mirrorPending !== null) writeMirror(mirrorPending);
    mirrorPending = null;
  }, MIRROR_DEBOUNCE_MS);
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
    writeMirrorDebounced(value);
  },
  removeItem: (name) => localStorage.removeItem(name),
};

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      ...DEFAULTS,
      setDirection: (direction) => set({ direction }),
      setCustomTheme: (customTheme) => set({ customTheme }),
      setMode: (mode) => set({ mode }),
      setDiffFontSize: (diffFontSize) => set({ diffFontSize }),
      setDefaultViewType: (defaultViewType) => set({ defaultViewType }),
      setDefaultThreeDot: (defaultThreeDot) => set({ defaultThreeDot }),
      setBotLogins: (botLogins) => set({ botLogins }),
      setConversationIgnoreRules: (conversationIgnoreRules) => set({ conversationIgnoreRules }),
      setRepoStripPrefixes: (repoStripPrefixes) => set({ repoStripPrefixes }),
      setRepoBasePaths: (repoBasePaths) => set({ repoBasePaths }),
      setAutoSync: (autoSync) => set({ autoSync }),
      setLogLevel: (logLevel) => set({ logLevel }),
      setDiscoveryIntervalMin: (discoveryIntervalMin) => set({ discoveryIntervalMin }),
      setAutoFetchMinutes: (autoFetchMinutes) => set({ autoFetchMinutes }),
      setBoardStatusBorderSide: (boardStatusBorderSide) => set({ boardStatusBorderSide }),
      setAiEnabled: (aiEnabled) => set({ aiEnabled }),
      setChatModel: (chatModel) => set({ chatModel }),
      setMarkEdits: (markEdits) => set({ markEdits }),
      setDiffMarkers: (diffMarkers) => set({ diffMarkers }),
      setLargeDiffMaxLines: (largeDiffMaxLines) => set({ largeDiffMaxLines }),
      setFileTestPatterns: (fileTestPatterns) => set({ fileTestPatterns }),
      setFileConfigPatterns: (fileConfigPatterns) => set({ fileConfigPatterns }),
      setLeftPaneTab: (leftPaneTab) => set({ leftPaneTab }),
      setDefaultTerminal: (defaultTerminal) => set({ defaultTerminal }),
      setTmuxTarget: (tmuxTarget) => set({ tmuxTarget }),
      setTmuxSessionName: (tmuxSessionName) => set({ tmuxSessionName }),
      setSessionHistoryDays: (sessionHistoryDays) => set({ sessionHistoryDays }),
      setSessionSummaryDir: (sessionSummaryDir) => set({ sessionSummaryDir }),
      setSessionSummaryFork: (sessionSummaryFork) => set({ sessionSummaryFork }),
      setOverviewModel: (overviewModel) => set({ overviewModel }),
      setBoardOverviewOpen: (boardOverviewOpen) => set({ boardOverviewOpen }),
      setBoardMonitor: (boardMonitor) => set({ boardMonitor }),
      setBoardMonitorLogOpen: (boardMonitorLogOpen) => set({ boardMonitorLogOpen }),
      setBoardMonitorIntervalMin: (boardMonitorIntervalMin) => set({ boardMonitorIntervalMin }),
      setBoardArchiveDays: (boardArchiveDays) => set({ boardArchiveDays }),
      setBoardNeedsActionSnoozed: (boardNeedsActionSnoozed) => set({ boardNeedsActionSnoozed }),
      setBoardCountDone: (boardCountDone) => set({ boardCountDone }),
      setBoardBadgeKind: (kind, on) => set((s) => ({ boardBadgeKinds: { ...s.boardBadgeKinds, [kind]: on } })),
      setBoardSnoozeOptions: (boardSnoozeOptions) => set({ boardSnoozeOptions }),
      snoozeCard: (cardId, until) => set((s) => ({ boardSnoozes: { ...s.boardSnoozes, [cardId]: until } })),
      unsnoozeCard: (cardId) =>
        set((s) => ({ boardSnoozes: Object.fromEntries(Object.entries(s.boardSnoozes).filter(([id]) => id !== cardId)) })),
    }),
    {
      name: "codereview-settings",
      version: 11,
      storage: createJSONStorage(() => mirroredStorage),
      partialize: (s) => pickPersisted(s),
      // Adding a key needs no version bump: the merge fills it from DEFAULTS.
      // Only bump for renames/reshapes, and keep every other persisted value.
      // The custom theme is nested, so it is normalized instead: a theme saved before a field
      // existed would otherwise come back without it.
      merge: (persisted, current) => {
        const p = (persisted ?? {}) as Partial<PersistedSettings>;
        return { ...current, ...p, customTheme: parseCustomTheme(p.customTheme) };
      },
      migrate: (persisted, version) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        const out: PersistedSettings = { ...DEFAULTS, ...sanitizeSettings(p) };
        if (version < 2) {
          // v0 (flat `theme`) / v1 (`themeMode` + theme slots): only the mode carries over.
          const oldMode = version < 1 ? p.theme : p.themeMode;
          out.direction = "a";
          out.mode = oldMode === "dark" || oldMode === "light" ? oldMode : "system";
        }
        if (version < 11 && p.diffFontSize === 12.5) out.diffFontSize = 12;
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
    if (Array.isArray(value) !== Array.isArray(DEFAULTS[key])) continue;
    if (typeof value === "number" && !Number.isFinite(value)) continue;
    const allowed = ENUMS[key];
    if (allowed && !allowed.includes(value as string)) continue;
    // Nested value: normalize field by field instead of trusting its shape.
    picked[key] = key === "customTheme" ? parseCustomTheme(value) : value;
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

export const AI_DISABLED_REASON = "AI features are turned off in Settings → AI";

/** Why AI actions are unavailable (for a button's `title`), or null when the switch is on. */
export function useAiDisabledReason(): string | null {
  return useSettingsStore((s) => s.aiEnabled) ? null : AI_DISABLED_REASON;
}
