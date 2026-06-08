import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import {
  useSettingsStore,
  effectiveTheme,
  resolveActiveTheme,
  findTheme,
  BUILTIN_DARK,
  BUILTIN_LIGHT,
  BUILTIN_DARK_ID,
  BUILTIN_LIGHT_ID,
  DEFAULT_DIFF_FONT_SIZE,
} from "./settings";

beforeEach(() => {
  useSettingsStore.setState({
    themeMode: "dark",
    customThemes: [],
    darkThemeId: BUILTIN_DARK_ID,
    lightThemeId: BUILTIN_LIGHT_ID,
    diffFontSize: DEFAULT_DIFF_FONT_SIZE,
    defaultViewType: "split",
    defaultThreeDot: true,
    botLogins: "",
  });
});

describe("useSettingsStore", () => {
  it("has sensible defaults (dark mode, built-in slots, 12.5px, split, three-dot)", () => {
    const s = useSettingsStore.getState();
    expect(s.themeMode).toBe("dark");
    expect(s.customThemes).toEqual([]);
    expect(s.darkThemeId).toBe(BUILTIN_DARK_ID);
    expect(s.lightThemeId).toBe(BUILTIN_LIGHT_ID);
    expect(s.diffFontSize).toBe(12.5);
    expect(s.defaultViewType).toBe("split");
    expect(s.defaultThreeDot).toBe(true);
  });

  it("non-theme setters update each field", () => {
    const s = useSettingsStore.getState();
    s.setThemeMode("light");
    s.setDiffFontSize(16);
    s.setDefaultViewType("unified");
    s.setDefaultThreeDot(false);
    s.setBotLogins("dependabot, renovate");
    const n = useSettingsStore.getState();
    expect(n.themeMode).toBe("light");
    expect(n.diffFontSize).toBe(16);
    expect(n.defaultViewType).toBe("unified");
    expect(n.defaultThreeDot).toBe(false);
    expect(n.botLogins).toBe("dependabot, renovate");
  });
});

describe("custom theme management", () => {
  it("addThemeFrom clones a source into an editable custom theme", () => {
    const id = useSettingsStore.getState().addThemeFrom(BUILTIN_DARK_ID, "Midnight");
    const t = useSettingsStore.getState().customThemes.find((x) => x.id === id)!;
    expect(t.name).toBe("Midnight");
    expect(t.builtin).toBe(false);
    expect(t.derivedFrom).toBe(BUILTIN_DARK_ID);
    expect(t.ui).toEqual(BUILTIN_DARK.ui);
    expect(t.syntax).toEqual(BUILTIN_DARK.syntax);
    // Must be a copy, not a reference to the frozen built-in.
    expect(t.ui).not.toBe(BUILTIN_DARK.ui);
  });

  it("updateTheme merges ui/syntax patches and ignores built-ins", () => {
    const store = useSettingsStore.getState();
    const id = store.addThemeFrom(BUILTIN_LIGHT_ID, "Custom Light");
    store.updateTheme(id, { ui: { accent: "#ff0000" }, syntax: { keyword: "#00ff00" } });
    store.updateTheme(BUILTIN_DARK_ID, { ui: { accent: "#123456" } });
    const t = useSettingsStore.getState().customThemes.find((x) => x.id === id)!;
    expect(t.ui.accent).toBe("#ff0000");
    expect(t.ui.bg).toBe(BUILTIN_LIGHT.ui.bg); // untouched key preserved
    expect(t.syntax.keyword).toBe("#00ff00");
    expect(BUILTIN_DARK.ui.accent).toBe("#2f81f7"); // built-in unchanged
  });

  it("renameTheme updates the name", () => {
    const store = useSettingsStore.getState();
    const id = store.addThemeFrom(BUILTIN_DARK_ID, "Old");
    store.renameTheme(id, "New");
    expect(useSettingsStore.getState().customThemes.find((x) => x.id === id)!.name).toBe("New");
  });

  it("deleteTheme removes it and repoints slots to the matching built-in", () => {
    const store = useSettingsStore.getState();
    const id = store.addThemeFrom(BUILTIN_DARK_ID, "Doomed");
    store.setDarkThemeId(id);
    store.setLightThemeId(id);
    store.deleteTheme(id);
    const s = useSettingsStore.getState();
    expect(s.customThemes.find((x) => x.id === id)).toBeUndefined();
    expect(s.darkThemeId).toBe(BUILTIN_DARK_ID);
    expect(s.lightThemeId).toBe(BUILTIN_LIGHT_ID);
  });
});

describe("findTheme", () => {
  it("resolves built-ins and customs, undefined for unknown", () => {
    const id = useSettingsStore.getState().addThemeFrom(BUILTIN_DARK_ID, "Mine");
    const customs = useSettingsStore.getState().customThemes;
    expect(findTheme(customs, BUILTIN_DARK_ID)).toBe(BUILTIN_DARK);
    expect(findTheme(customs, id)?.name).toBe("Mine");
    expect(findTheme(customs, "nope")).toBeUndefined();
  });
});

describe("resolveActiveTheme", () => {
  it("maps mode + slots to a concrete theme", () => {
    expect(
      resolveActiveTheme({
        themeMode: "dark",
        customThemes: [],
        darkThemeId: BUILTIN_DARK_ID,
        lightThemeId: BUILTIN_LIGHT_ID,
      }),
    ).toBe(BUILTIN_DARK);
    expect(
      resolveActiveTheme({
        themeMode: "light",
        customThemes: [],
        darkThemeId: BUILTIN_DARK_ID,
        lightThemeId: BUILTIN_LIGHT_ID,
      }),
    ).toBe(BUILTIN_LIGHT);
  });

  it("falls back to the base built-in for a dangling slot id", () => {
    expect(
      resolveActiveTheme({
        themeMode: "dark",
        customThemes: [],
        darkThemeId: "missing",
        lightThemeId: BUILTIN_LIGHT_ID,
      }),
    ).toBe(BUILTIN_DARK);
  });
});

describe("effectiveTheme", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("passes through explicit themes", () => {
    expect(effectiveTheme("dark")).toBe("dark");
    expect(effectiveTheme("light")).toBe("light");
  });

  it("resolves 'system' via the OS preference", () => {
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("light") }));
    expect(effectiveTheme("system")).toBe("light");
    vi.stubGlobal("matchMedia", () => ({ matches: false }));
    expect(effectiveTheme("system")).toBe("dark");
  });
});
