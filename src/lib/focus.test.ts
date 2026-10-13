import { parseDiff } from "react-diff-view";
import { describe, expect, it } from "vitest";
import fixture from "../../src-tauri/src/commands/focus_fixture.diff?raw";
import { fileDisplayPath } from "./diff";
import {
  buildFocusModel,
  digestGroups,
  digestMarkdown,
  focusActive,
  hiddenCount,
  originalsIn,
  tierVisible,
  type HunkSpan,
} from "./focus";
import type { FocusEntry, FocusView } from "./types";

function entry(over: Partial<FocusEntry>): FocusEntry {
  return {
    hunk_id: "h",
    file_path: "a.ts",
    old_start: 1,
    new_start: 1,
    header: "@@ -1,2 +1,2 @@",
    additions: 1,
    deletions: 1,
    tier: "review",
    source: "ai",
    reason: null,
    override: null,
    is_new: false,
    ...over,
  };
}

function view(hunks: FocusEntry[], over: Partial<FocusView["focus"]> = {}): FocusView {
  return {
    focus: {
      id: 1,
      review_id: 1,
      head_sha: "h",
      base_sha: "b",
      level: "focus",
      summary: "Does a thing.",
      model: null,
      status: "ready",
      error: null,
      planned_at: "now",
      created_at: "now",
      ...over,
    },
    stale: false,
    hunks,
    file_overrides: [],
  };
}

const span = (oldStart: number, oldLines: number, newStart: number, newLines: number): HunkSpan => ({
  oldStart,
  oldLines,
  newStart,
  newLines,
});

describe("shared diff fixture", () => {
  it("parses to the same (file, oldStart, newStart) tuples as the Rust parser", () => {
    const tuples = parseDiff(fixture).flatMap((f) => {
      const path = fileDisplayPath(f);
      if (f.hunks.length === 0) return [[path, 0, 0]];
      return f.hunks.map((h) => [path, h.oldStart, h.newStart]);
    });
    // Mirrors `splits_files_and_hunks_with_display_paths` in commands/focus.rs.
    expect(tuples).toEqual([
      ["src/app.rs", 1, 1],
      ["src/app.rs", 10, 11],
      ["gone.txt", 1, 0],
      ["new name.rs", 0, 0],
      ["logo.png", 0, 0],
      ["Cargo.lock", 5, 5],
      ["web/new.ts", 0, 1],
    ]);
  });
});

describe("levels", () => {
  it("thresholds tiers", () => {
    expect(tierVisible("trivial", "thorough")).toBe(false);
    expect(tierVisible("skim", "thorough")).toBe(true);
    expect(tierVisible("skim", "focus")).toBe(false);
    expect(tierVisible("review", "critical")).toBe(false);
    expect(tierVisible("critical", "critical")).toBe(true);
  });

  it("is inactive without a plan or at Off", () => {
    expect(focusActive(null)).toBe(false);
    expect(focusActive(view([], { planned_at: null }))).toBe(false);
    expect(focusActive(view([], { level: "off" }))).toBe(false);
    expect(focusActive(view([]))).toBe(true);
  });
});

describe("buildFocusModel", () => {
  const files = [
    { path: "a.ts", hunks: [span(1, 2, 1, 2), span(10, 2, 10, 3), span(20, 1, 21, 1)] },
    { path: "lock", hunks: [span(1, 1, 1, 1)] },
    { path: "img.png", hunks: [] },
  ];
  const v = view([
    entry({ hunk_id: "a1", tier: "critical" }),
    entry({ hunk_id: "a2", old_start: 10, new_start: 10, tier: "trivial", reason: "rename" }),
    entry({ hunk_id: "l1", file_path: "lock", tier: "trivial", reason: "lockfile" }),
    entry({ hunk_id: "p1", file_path: "img.png", old_start: 0, new_start: 0, header: "", tier: "trivial", reason: "binary" }),
  ]);

  it("hides low tiers, keeps unmatched hunks visible, and flags fully hidden files", () => {
    const m = buildFocusModel(files, v, [], new Set())!;
    expect(m[0].hunks.map((h) => h.hidden)).toEqual([false, true, false]);
    expect(m[0].hunks[2].entry).toBeNull();
    expect(m[0].fullyHidden).toBe(false);
    expect(m[1].fullyHidden).toBe(true);
    expect(m[2].whole?.hidden).toBe(true);
    expect(hiddenCount(m)).toBe(3);
  });

  it("never hides hunks with comments, revealed hunks, or at a looser level", () => {
    const pinned = buildFocusModel(files, v, [{ path: "a.ts", side: "RIGHT", line: 12 }], new Set(["l1"]))!;
    expect(pinned[0].hunks[1].hidden).toBe(false);
    expect(pinned[0].hunks[1].pinned).toBe(true);
    expect(pinned[1].hunks[0].hidden).toBe(false);
    const left = buildFocusModel(files, v, [{ path: "a.ts", side: "LEFT", line: 12 }], new Set())!;
    // LEFT 12 is outside old lines 10–11.
    expect(left[0].hunks[1].hidden).toBe(true);
  });

  it("applies hunk overrides over file overrides over tiers", () => {
    const ov = view(
      [
        entry({ hunk_id: "a1", tier: "critical", override: "hide" }),
        entry({ hunk_id: "a2", old_start: 10, new_start: 10, tier: "trivial" }),
        entry({ hunk_id: "l1", file_path: "lock", tier: "trivial", override: "show" }),
      ],
      {},
    );
    ov.file_overrides = [{ file_path: "a.ts", override: "show" }];
    const m = buildFocusModel(files, ov, [], new Set())!;
    expect(m[0].hunks.map((h) => h.hidden)).toEqual([true, false, false]);
    expect(m[0].hunks[0].reason).toBe("hidden manually");
    expect(m[1].hunks[0].hidden).toBe(false);
  });

  it("groups the digest by reason and renders markdown", () => {
    const m = buildFocusModel(files, v, [], new Set())!;
    expect(digestGroups(m).map((g) => [g.reason, g.items.length])).toEqual([
      ["binary", 1],
      ["lockfile", 1],
      ["rename", 1],
    ]);
    const md = digestMarkdown(v, m);
    expect(md).toContain("Does a thing.");
    expect(md).toContain("- **lockfile** (1): `lock`");
  });
});

describe("originalsIn", () => {
  it("finds the original hunks merged into an expanded hunk", () => {
    const originals = [span(1, 3, 1, 3), span(10, 2, 10, 3), span(30, 1, 31, 0)];
    expect(originalsIn(span(1, 11, 1, 12), originals)).toEqual([0, 1]);
    expect(originalsIn(span(30, 1, 31, 0), originals)).toEqual([2]);
  });
});
