import { describe, expect, it, vi } from "vitest";
import { DEFAULT_CUSTOM_THEME } from "./customTheme";
import { INSPIRATIONS, extractJson, randomInspiration, suggestTheme } from "./themeSuggest";

const good = {
  base: "b",
  dark: { text: "#eeeeee", background: "#101010", surface: "#181818", accent: "#88aaff" },
  light: { text: "#111111", background: "#fafafa", surface: "#ffffff" },
};
const bad = { ...good, dark: { ...good.dark, text: "#202020" } };

describe("extractJson", () => {
  it("finds the object in prose and code fences, with braces inside strings", () => {
    expect(extractJson('Here you go:\n```json\n{"a": {"b": "}{"}}\n```')).toEqual({ a: { b: "}{" } });
  });
  it("returns null without a complete object", () => {
    expect(extractJson("no json")).toBeNull();
    expect(extractJson('{"a": 1')).toBeNull();
    expect(extractJson("{not json}")).toBeNull();
  });
});

describe("randomInspiration", () => {
  it("picks from the curated list", () => {
    expect(INSPIRATIONS).toContain(randomInspiration());
    expect(randomInspiration(() => 0)).toBe(INSPIRATIONS[0]);
  });
});

describe("suggestTheme", () => {
  const base = { scope: "all", current: DEFAULT_CUSTOM_THEME } as const;

  it("uses the hint as the inspiration and returns the parsed theme", async () => {
    const ask = vi.fn().mockResolvedValue(JSON.stringify(good));
    const r = await suggestTheme({ ...base, hint: " warm and low contrast ", ask });
    expect(ask).toHaveBeenCalledTimes(1);
    expect(ask.mock.calls[0][2]).toBe("warm and low contrast");
    expect(r.suggested.base).toBe("b");
    expect(r.issues).toEqual([]);
  });

  it("retries once with the contrast problems as feedback and keeps the better design", async () => {
    const ask = vi.fn().mockResolvedValueOnce(JSON.stringify(bad)).mockResolvedValueOnce(JSON.stringify(good));
    const r = await suggestTheme({ ...base, ask });
    expect(ask).toHaveBeenCalledTimes(2);
    expect(ask.mock.calls[1][3]).toContain("text on background");
    expect(r.suggested.dark.text).toBe("#eeeeee");
    expect(r.issues).toEqual([]);
  });

  it("applies the first design with a warning when the retry is no better or fails", async () => {
    const worse = vi.fn().mockResolvedValue(JSON.stringify(bad));
    expect((await suggestTheme({ ...base, ask: worse })).issues.length).toBeGreaterThan(0);
    const broken = vi.fn().mockResolvedValueOnce(JSON.stringify(bad)).mockRejectedValueOnce(new Error("boom"));
    const r = await suggestTheme({ ...base, ask: broken });
    expect(r.suggested.dark.text).toBe("#202020");
    expect(r.issues.length).toBeGreaterThan(0);
  });

  it("throws on a reply without a theme or without colours", async () => {
    await expect(suggestTheme({ ...base, ask: vi.fn().mockResolvedValue("sorry") })).rejects.toThrow(/did not return/);
    await expect(suggestTheme({ ...base, ask: vi.fn().mockResolvedValue("{}") })).rejects.toThrow(/no usable colors/);
  });

  it("does not check contrast or require colours for typography and shape", async () => {
    const ask = vi.fn().mockResolvedValue(JSON.stringify({ uiFont: "plex", corners: "round" }));
    const r = await suggestTheme({ scope: "typography", current: DEFAULT_CUSTOM_THEME, ask });
    expect(ask).toHaveBeenCalledTimes(1);
    expect(r.suggested.uiFont).toBe("plex");
  });
});
