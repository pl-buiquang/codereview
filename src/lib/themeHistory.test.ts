import { beforeEach, describe, expect, it } from "vitest";
import { DEFAULT_CUSTOM_THEME } from "./customTheme";
import { useSettingsStore } from "./settings";
import {
  COALESCE_MS,
  HISTORY_LIMIT,
  editCustomTheme,
  ownsUndo,
  redoCustomTheme,
  undoCustomTheme,
  useThemeHistory,
} from "./themeHistory";

const accent = (hex: string) => (t: typeof DEFAULT_CUSTOM_THEME) => ({ ...t, dark: { ...t.dark, accent: hex } });
const current = () => useSettingsStore.getState().customTheme;

beforeEach(() => {
  useSettingsStore.setState({ customTheme: DEFAULT_CUSTOM_THEME });
  useThemeHistory.setState({ past: [], future: [], lastKey: null, lastAt: 0 });
});

describe("theme history", () => {
  it("undoes and redoes edits", () => {
    editCustomTheme((t) => ({ ...t, corners: "round" }));
    editCustomTheme((t) => ({ ...t, uiFont: "inter" }));
    undoCustomTheme();
    expect(current()).toMatchObject({ corners: "round", uiFont: "base" });
    undoCustomTheme();
    expect(current()).toEqual(DEFAULT_CUSTOM_THEME);
    redoCustomTheme();
    expect(current().corners).toBe("round");
  });

  it("clears redo after a new edit, and ignores no-op edits", () => {
    editCustomTheme((t) => ({ ...t, corners: "round" }));
    undoCustomTheme();
    editCustomTheme((t) => t);
    expect(useThemeHistory.getState().future).toHaveLength(1);
    editCustomTheme((t) => ({ ...t, corners: "square" }));
    expect(useThemeHistory.getState().future).toHaveLength(0);
    expect(useThemeHistory.getState().past).toHaveLength(1);
  });

  it("merges quick edits of the same field into one step", () => {
    editCustomTheme(accent("#111111"), "dark:accent", 1000);
    editCustomTheme(accent("#222222"), "dark:accent", 1000 + COALESCE_MS - 1);
    editCustomTheme(accent("#333333"), "dark:accent", 1000 + 2 * COALESCE_MS);
    expect(useThemeHistory.getState().past).toHaveLength(2);
    undoCustomTheme();
    expect(current().dark.accent).toBe("#222222");
    undoCustomTheme();
    expect(current()).toEqual(DEFAULT_CUSTOM_THEME);
  });

  it("does not merge edits of different fields", () => {
    editCustomTheme(accent("#111111"), "dark:accent", 1000);
    editCustomTheme((t) => ({ ...t, dark: { ...t.dark, text: "#ffffff" } }), "dark:text", 1100);
    expect(useThemeHistory.getState().past).toHaveLength(2);
  });

  it("keeps at most HISTORY_LIMIT steps", () => {
    for (let i = 0; i < HISTORY_LIMIT + 10; i++) editCustomTheme(accent(`#${String(i).padStart(6, "0")}`));
    expect(useThemeHistory.getState().past).toHaveLength(HISTORY_LIMIT);
  });

  it("undo and redo with nothing to do are no-ops", () => {
    undoCustomTheme();
    redoCustomTheme();
    expect(current()).toEqual(DEFAULT_CUSTOM_THEME);
  });

  it("leaves undo to text fields but not to colour pickers", () => {
    const text = document.createElement("input");
    const colour = document.createElement("input");
    colour.type = "color";
    expect(ownsUndo(text)).toBe(true);
    expect(ownsUndo(document.createElement("textarea"))).toBe(true);
    expect(ownsUndo(colour)).toBe(false);
    expect(ownsUndo(document.body)).toBe(false);
  });
});
