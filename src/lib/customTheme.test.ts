import { describe, expect, it } from "vitest";
import {
  DEFAULT_CUSTOM_THEME,
  accentTone,
  contrastIssues,
  contrastRatio,
  customThemeStyle,
  mergeScope,
  parseCustomTheme,
  resetScope,
  toHexColour,
} from "./customTheme";

describe("toHexColour", () => {
  it("normalizes hex and rgb forms, rejects the rest", () => {
    expect(toHexColour("#ABC")).toBe("#aabbcc");
    expect(toHexColour("#AABBCC")).toBe("#aabbcc");
    expect(toHexColour("rgba(70, 194, 107, 0.1)")).toBe("#46c26b");
    expect(toHexColour("red")).toBeNull();
    expect(toHexColour("")).toBeNull();
  });
});

describe("parseCustomTheme", () => {
  it("falls back to the defaults for non-objects", () => {
    for (const raw of [null, undefined, 3, "x", []]) expect(parseCustomTheme(raw)).toEqual(DEFAULT_CUSTOM_THEME);
  });

  it("keeps valid fields and drops bad hex, unknown knobs and unknown colours", () => {
    const t = parseCustomTheme({
      base: "b",
      uiFont: "inter",
      monoFont: "comic-sans",
      corners: "round",
      dark: { accent: "#ABCDEF", background: "nope", bogus: "#000000" },
      light: { tokString: "#123" },
    });
    expect(t.base).toBe("b");
    expect(t.uiFont).toBe("inter");
    expect(t.monoFont).toBe("base");
    expect(t.corners).toBe("round");
    expect(t.dark).toEqual({ accent: "#abcdef" });
    expect(t.light).toEqual({ tokString: "#112233" });
  });
});

describe("customThemeStyle", () => {
  it("is empty for the default theme", () => {
    expect(customThemeStyle(DEFAULT_CUSTOM_THEME, "dark")).toEqual({});
  });

  it("maps each mode's colours to tokens and derives the shades", () => {
    const t = parseCustomTheme({ dark: { accent: "#ffcc00", tokKeyword: "#ff00ff" }, light: { accent: "#003366" } });
    const dark = customThemeStyle(t, "dark");
    expect(dark["--accent"]).toBe("#ffcc00");
    expect(dark["--on-accent"]).toBe("#15181d");
    expect(dark["--accent-soft"]).toContain("#ffcc00");
    expect(dark["--tok-keyword"]).toBe("#ff00ff");
    const light = customThemeStyle(t, "light");
    expect(light["--accent"]).toBe("#003366");
    expect(light["--on-accent"]).toBe("#ffffff");
    expect(light["--tok-keyword"]).toBeUndefined();
  });

  it("sets info and its derived soft/border/on colors", () => {
    const t = parseCustomTheme({ dark: { info: "#3366ff" } });
    const dark = customThemeStyle(t, "dark");
    expect(dark["--info"]).toBe("#3366ff");
    expect(dark["--info-soft"]).toContain("#3366ff");
    expect(dark["--info-border"]).toContain("#3366ff");
    expect(dark["--on-info"]).toBe("#ffffff");
    expect(customThemeStyle(t, "light")["--info"]).toBeUndefined();
  });

  it("reads the legacy reviewRequired key as info", () => {
    expect(parseCustomTheme({ dark: { reviewRequired: "#3366ff" } }).dark).toEqual({ info: "#3366ff" });
  });

  it("applies fonts, corners and scale; a preview leaves the scale out", () => {
    const t = parseCustomTheme({ uiFont: "plex", monoFont: "geist-mono", corners: "square", textSize: "large" });
    const s = customThemeStyle(t, "dark");
    expect(s["--font-ui"]).toContain("IBM Plex Sans");
    expect(s["--font-mono"]).toContain("Geist Mono");
    expect(s["--radius-m"]).toBe("2px");
    expect(s.zoom).toBe("1.1");
    expect(customThemeStyle(t, "dark", { preview: true }).zoom).toBeUndefined();
  });
});

describe("contrast", () => {
  it("computes WCAG ratios and picks the accent text tone", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 0);
    expect(accentTone("#ffee00")).toBe("light");
    expect(accentTone("#102040")).toBe("dark");
  });

  it("flags only set pairs that read badly", () => {
    expect(contrastIssues(DEFAULT_CUSTOM_THEME)).toEqual([]);
    const bad = parseCustomTheme({ dark: { text: "#777777", background: "#808080", surface: "#000000" } });
    const issues = contrastIssues(bad);
    expect(issues).toHaveLength(1);
    expect(issues[0]).toContain("dark: text on background");
  });
});

describe("mergeScope / resetScope", () => {
  const current = parseCustomTheme({
    base: "b",
    uiFont: "inter",
    corners: "round",
    dark: { accent: "#111111", tokKeyword: "#222222" },
    light: { accent: "#333333" },
  });
  const suggested = parseCustomTheme({
    base: "c",
    uiFont: "geist",
    corners: "square",
    dark: { accent: "#aaaaaa", tokKeyword: "#bbbbbb" },
    light: { tokKeyword: "#cccccc" },
  });

  it("all takes the suggestion whole", () => {
    expect(mergeScope(current, suggested, "all")).toEqual(suggested);
  });

  it("colours replaces both palettes' colours but not syntax or knobs", () => {
    const m = mergeScope(current, suggested, "colours");
    expect(m.dark).toEqual({ accent: "#aaaaaa", tokKeyword: "#222222" });
    expect(m.light).toEqual({});
    expect(m.base).toBe("b");
    expect(m.uiFont).toBe("inter");
  });

  it("syntax, typography and shape touch only their fields", () => {
    expect(mergeScope(current, suggested, "syntax").dark).toEqual({ accent: "#111111", tokKeyword: "#bbbbbb" });
    expect(mergeScope(current, suggested, "typography")).toMatchObject({ uiFont: "geist", corners: "round" });
    expect(mergeScope(current, suggested, "shape")).toMatchObject({ uiFont: "inter", corners: "square" });
  });

  it("resetScope clears a scope and keeps the base", () => {
    const r = resetScope(current, "colours");
    expect(r.dark).toEqual({ tokKeyword: "#222222" });
    expect(r.light).toEqual({});
    expect(r.base).toBe("b");
    expect(resetScope(current, "shape").corners).toBe("base");
  });
});
