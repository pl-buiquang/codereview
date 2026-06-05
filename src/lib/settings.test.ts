import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import {
  useSettingsStore,
  effectiveTheme,
  DEFAULT_DIFF_FONT_SIZE,
  DEFAULT_REVIEW_TAB_COLOR,
} from "./settings";

beforeEach(() => {
  useSettingsStore.setState({
    theme: "dark",
    diffFontSize: DEFAULT_DIFF_FONT_SIZE,
    defaultViewType: "split",
    defaultThreeDot: true,
    reviewTabColor: DEFAULT_REVIEW_TAB_COLOR,
  });
});

describe("useSettingsStore", () => {
  it("has sensible defaults (dark, 12.5px, split, three-dot on, white review tab)", () => {
    const s = useSettingsStore.getState();
    expect(s.theme).toBe("dark");
    expect(s.diffFontSize).toBe(12.5);
    expect(s.defaultViewType).toBe("split");
    expect(s.defaultThreeDot).toBe(true);
    expect(s.reviewTabColor).toBe("#ffffff");
  });

  it("setters update each field", () => {
    const s = useSettingsStore.getState();
    s.setTheme("light");
    s.setDiffFontSize(16);
    s.setDefaultViewType("unified");
    s.setDefaultThreeDot(false);
    s.setReviewTabColor("#ff8800");
    const n = useSettingsStore.getState();
    expect(n.theme).toBe("light");
    expect(n.diffFontSize).toBe(16);
    expect(n.defaultViewType).toBe("unified");
    expect(n.defaultThreeDot).toBe(false);
    expect(n.reviewTabColor).toBe("#ff8800");
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
