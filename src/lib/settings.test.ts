import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import {
  useSettingsStore,
  effectiveTheme,
  parseRepoStripPrefixes,
  stripRepoPrefix,
  DEFAULT_DIFF_FONT_SIZE,
  exportSettings,
  importSettings,
} from "./settings";

beforeEach(() => {
  useSettingsStore.setState({
    direction: "a",
    mode: "system",
    diffFontSize: DEFAULT_DIFF_FONT_SIZE,
    defaultViewType: "split",
    defaultThreeDot: true,
    botLogins: "",
    repoStripPrefixes: "",
    autoSync: "periodic",
    discoveryIntervalMin: 5,
    fileTestPatterns: "",
    fileConfigPatterns: "",
  });
});

const migrate = useSettingsStore.persist.getOptions().migrate!;

describe("useSettingsStore", () => {
  it("has sensible defaults (direction a, system mode, 12.5px, split, three-dot)", () => {
    const s = useSettingsStore.getState();
    expect(s.direction).toBe("a");
    expect(s.mode).toBe("system");
    expect(s.diffFontSize).toBe(12.5);
    expect(s.defaultViewType).toBe("split");
    expect(s.defaultThreeDot).toBe(true);
    expect(s.autoSync).toBe("periodic");
    expect(s.discoveryIntervalMin).toBe(5);
  });

  it("setters update each field", () => {
    const s = useSettingsStore.getState();
    s.setDirection("c");
    s.setMode("light");
    s.setDiffFontSize(16);
    s.setDefaultViewType("unified");
    s.setDefaultThreeDot(false);
    s.setBotLogins("dependabot, renovate");
    s.setRepoStripPrefixes("philips-internal/cardiologs-");
    s.setAutoSync("on_focus");
    s.setDiscoveryIntervalMin(30);
    const n = useSettingsStore.getState();
    expect(n.direction).toBe("c");
    expect(n.mode).toBe("light");
    expect(n.diffFontSize).toBe(16);
    expect(n.defaultViewType).toBe("unified");
    expect(n.defaultThreeDot).toBe(false);
    expect(n.botLogins).toBe("dependabot, renovate");
    expect(n.repoStripPrefixes).toBe("philips-internal/cardiologs-");
    expect(n.autoSync).toBe("on_focus");
    expect(n.discoveryIntervalMin).toBe(30);
  });

  it("partialize persists the v2 keys and no setters", () => {
    const snapshot = useSettingsStore.persist.getOptions().partialize!(
      useSettingsStore.getState(),
    ) as Record<string, unknown>;
    expect(Object.keys(snapshot).sort()).toEqual(
      [
        "autoFetchMinutes",
        "autoSync",
        "boardMonitor",
        "boardMonitorLogOpen",
        "boardMonitorIntervalMin",
        "boardArchiveDays",
        "boardCountDone",
        "boardOverviewOpen",
        "boardStatusBorderSide",
        "botLogins",
        "conversationIgnoreRules",
        "chatModel",
        "defaultTerminal",
        "defaultThreeDot",
        "defaultViewType",
        "diffFontSize",
        "direction",
        "discoveryIntervalMin",
        "fileConfigPatterns",
        "fileTestPatterns",
        "largeDiffMaxLines",
        "leftPaneTab",
        "logLevel",
        "markEdits",
        "mode",
        "overviewModel",
        "repoBasePaths",
        "repoStripPrefixes",
        "sessionHistoryDays",
        "sessionSummaryDir",
        "sessionSummaryFork",
        "tmuxSessionName",
        "tmuxTarget",
      ].sort(),
    );
  });
});

describe("exportSettings / importSettings", () => {
  it("round-trips the persisted settings", () => {
    const s = useSettingsStore.getState();
    s.setMode("light");
    s.setDiffFontSize(15);
    s.setFileTestPatterns("*.spec.rb");
    const json = exportSettings();
    expect(JSON.parse(json)).toMatchObject({ app: "codereview", kind: "settings" });

    s.setMode("dark");
    s.setDiffFontSize(11);
    s.setFileTestPatterns("");
    const applied = importSettings(json);
    const n = useSettingsStore.getState();
    expect(applied).toBe(33);
    expect(n.mode).toBe("light");
    expect(n.diffFontSize).toBe(15);
    expect(n.fileTestPatterns).toBe("*.spec.rb");
  });

  it("accepts a bare settings object and skips unknown or mistyped keys", () => {
    const applied = importSettings(
      JSON.stringify({ mode: "neon", diffFontSize: "big", botLogins: "renovate", foo: 1 }),
    );
    const n = useSettingsStore.getState();
    expect(applied).toBe(1);
    expect(n.botLogins).toBe("renovate");
    expect(n.mode).toBe("system");
    expect(n.diffFontSize).toBe(DEFAULT_DIFF_FONT_SIZE);
  });

  it("rejects invalid JSON and files with no recognized settings", () => {
    expect(() => importSettings("not json")).toThrow(/valid JSON/);
    expect(() => importSettings("[]")).toThrow(/No recognized/);
    expect(() => importSettings("42")).toThrow(/No settings/);
  });
});

describe("settings migration to v2", () => {
  it("v0 (flat `theme`) → v2: maps mode, resets direction, keeps non-theme settings", () => {
    const out = migrate(
      {
        theme: "light",
        reviewTabColor: "#abcdef",
        diffFontSize: 15,
        defaultViewType: "unified",
        defaultThreeDot: false,
        botLogins: "bot1",
        autoSync: "off",
      },
      0,
    ) as Record<string, unknown>;
    expect(out.direction).toBe("a");
    expect(out.mode).toBe("light");
    expect(out.diffFontSize).toBe(15);
    expect(out.defaultViewType).toBe("unified");
    expect(out.defaultThreeDot).toBe(false);
    expect(out.botLogins).toBe("bot1");
    expect(out.autoSync).toBe("off");
    expect(out).not.toHaveProperty("reviewTabColor");
    expect(out).not.toHaveProperty("theme");
  });

  it("v1 (`themeMode` + theme slots) → v2: maps mode, drops custom-theme fields", () => {
    const out = migrate(
      {
        themeMode: "light",
        customThemes: [{ id: "x", name: "X" }],
        darkThemeId: "x",
        lightThemeId: "builtin-light",
        diffFontSize: 13,
        defaultViewType: "split",
        defaultThreeDot: true,
        botLogins: "",
      },
      1,
    ) as Record<string, unknown>;
    expect(out.direction).toBe("a");
    expect(out.mode).toBe("light");
    expect(out.diffFontSize).toBe(13);
    expect(out).not.toHaveProperty("customThemes");
    expect(out).not.toHaveProperty("darkThemeId");
    expect(out).not.toHaveProperty("lightThemeId");
    expect(out).not.toHaveProperty("themeMode");
  });

  it("non-dark/light old mode collapses to system; missing fields fall back to defaults", () => {
    expect((migrate({ themeMode: "system" }, 1) as Record<string, unknown>).mode).toBe("system");
    const out = migrate({}, 1) as Record<string, unknown>;
    expect(out.mode).toBe("system");
    expect(out.diffFontSize).toBe(DEFAULT_DIFF_FONT_SIZE);
    expect(out.defaultViewType).toBe("split");
    expect(out.defaultThreeDot).toBe(true);
    expect(out.botLogins).toBe("");
    expect(out.repoStripPrefixes).toBe("");
    expect(out.autoSync).toBe("periodic");
    expect(out.discoveryIntervalMin).toBe(5);
  });
});

describe("settings migration from v2+", () => {
  it("keeps every valid persisted value, including direction and newer keys", () => {
    const out = migrate(
      { direction: "c", mode: "dark", sessionSummaryDir: "~/notes", tmuxTarget: "per_repo", sessionHistoryDays: 7 },
      9,
    ) as Record<string, unknown>;
    expect(out.direction).toBe("c");
    expect(out.mode).toBe("dark");
    expect(out.sessionSummaryDir).toBe("~/notes");
    expect(out.tmuxTarget).toBe("per_repo");
    expect(out.sessionHistoryDays).toBe(7);
    expect(out.repoBasePaths).toBe("~/projects");
  });

  it("drops mistyped or out-of-enum values back to defaults", () => {
    const out = migrate({ direction: "zzz", diffFontSize: "big", tmuxTarget: "nope" }, 9) as Record<string, unknown>;
    expect(out.direction).toBe("a");
    expect(out.diffFontSize).toBe(DEFAULT_DIFF_FONT_SIZE);
    expect(out.tmuxTarget).toBe("current");
  });
});

describe("parseRepoStripPrefixes", () => {
  it("splits on commas, trims, and drops empties (case preserved)", () => {
    expect(parseRepoStripPrefixes(" philips-internal/Cardiologs- , acme/ ")).toEqual([
      "philips-internal/Cardiologs-",
      "acme/",
    ]);
    expect(parseRepoStripPrefixes("")).toEqual([]);
    expect(parseRepoStripPrefixes(" , ,")).toEqual([]);
  });
});

describe("stripRepoPrefix", () => {
  it("strips the first matching prefix (order significant, case-sensitive)", () => {
    expect(stripRepoPrefix("philips-internal/cardiologs-back", ["philips-internal/cardiologs-"]))
      .toEqual({ display: "back", stripped: true });
    // First match wins even if a later prefix would also match.
    expect(stripRepoPrefix("acme/web", ["acme/", "acme/w"])).toEqual({
      display: "web",
      stripped: true,
    });
  });

  it("passes through when no prefix matches or the prefix list is empty", () => {
    expect(stripRepoPrefix("acme/web", [])).toEqual({ display: "acme/web", stripped: false });
    expect(stripRepoPrefix("acme/web", ["other/"])).toEqual({
      display: "acme/web",
      stripped: false,
    });
    // Case-sensitive: a case mismatch does not strip.
    expect(stripRepoPrefix("Acme/web", ["acme/"])).toEqual({
      display: "Acme/web",
      stripped: false,
    });
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
