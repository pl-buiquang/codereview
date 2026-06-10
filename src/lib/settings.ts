import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { Direction } from "./themes";

export type ThemeMode = "dark" | "light" | "system";
export type ThemeBase = "dark" | "light";
export type DiffViewType = "split" | "unified";

export const DEFAULT_DIFF_FONT_SIZE = 12.5;

/** PR-list auto-refresh choices; value is the refetchInterval in ms, 0 = off. */
export const PR_LIST_POLL_OPTIONS: { label: string; value: number }[] = [
  { label: "off", value: 0 },
  { label: "30s", value: 30_000 },
  { label: "60s", value: 60_000 },
  { label: "5m", value: 300_000 },
];

type PersistedSettings = Pick<
  SettingsState,
  | "direction"
  | "mode"
  | "diffFontSize"
  | "defaultViewType"
  | "defaultThreeDot"
  | "botLogins"
  | "prListPollMs"
>;

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
  /** PR-list auto-refresh interval in ms; 0 = off. */
  prListPollMs: number;
  setDirection: (d: Direction) => void;
  setMode: (m: ThemeMode) => void;
  setDiffFontSize: (n: number) => void;
  setDefaultViewType: (v: DiffViewType) => void;
  setDefaultThreeDot: (b: boolean) => void;
  setBotLogins: (s: string) => void;
  setPrListPollMs: (ms: number) => void;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      direction: "a",
      mode: "system",
      diffFontSize: DEFAULT_DIFF_FONT_SIZE,
      defaultViewType: "split",
      defaultThreeDot: true,
      botLogins: "",
      prListPollMs: 0,
      setDirection: (direction) => set({ direction }),
      setMode: (mode) => set({ mode }),
      setDiffFontSize: (diffFontSize) => set({ diffFontSize }),
      setDefaultViewType: (defaultViewType) => set({ defaultViewType }),
      setDefaultThreeDot: (defaultThreeDot) => set({ defaultThreeDot }),
      setBotLogins: (botLogins) => set({ botLogins }),
      setPrListPollMs: (prListPollMs) => set({ prListPollMs }),
    }),
    {
      name: "codereview-settings",
      version: 2,
      partialize: (s) => ({
        direction: s.direction,
        mode: s.mode,
        diffFontSize: s.diffFontSize,
        defaultViewType: s.defaultViewType,
        defaultThreeDot: s.defaultThreeDot,
        botLogins: s.botLogins,
        prListPollMs: s.prListPollMs,
      }),
      // v0 (flat: `theme`) and v1 (`themeMode` + theme slots) collapse the same
      // way: direction resets to "a", the old mode maps over (custom themes,
      // dark/light slot ids are intentionally dropped), non-theme settings carry.
      migrate: (persisted, version) => {
        const p = (persisted ?? {}) as Record<string, unknown>;
        const oldMode = version < 1 ? (p.theme as string) : (p.themeMode as string);
        return {
          direction: "a",
          mode: oldMode === "dark" || oldMode === "light" ? oldMode : "system",
          diffFontSize: (p.diffFontSize as number) ?? DEFAULT_DIFF_FONT_SIZE,
          defaultViewType: (p.defaultViewType as DiffViewType) ?? "split",
          defaultThreeDot: (p.defaultThreeDot as boolean) ?? true,
          botLogins: (p.botLogins as string) ?? "",
          prListPollMs: (p.prListPollMs as number) ?? 0,
        } satisfies PersistedSettings;
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
