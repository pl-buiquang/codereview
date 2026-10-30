import { describe, it, expect } from "vitest";
import { cropRel, diffDim, hasImages, imageAssetRel, imageDiffAction, imageDiffUrl, isImagePath } from "./imageDiff";
import type { ImageDiffFile, ImageDiffStatus } from "./types";

const file = (over: Partial<ImageDiffFile> = {}): ImageDiffFile => ({
  index: 2,
  path: "e2e/a.png",
  old_path: null,
  change: "modified",
  before: { width: 1222, height: 100 },
  after: { width: 800, height: 140 },
  mismatch_px: 5,
  aa_px: 0,
  diff_pct: 0.1,
  regions: [],
  other_regions: 0,
  skipped: null,
  ...over,
});

const status = (over: Partial<ImageDiffStatus> = {}): ImageDiffStatus => ({
  state: "none",
  done: 0,
  total: 0,
  error: null,
  generated_at: null,
  auto: false,
  base_sha: null,
  head_sha: null,
  has_report: false,
  report_running: false,
  ...over,
});

describe("isImagePath", () => {
  it("matches png, jpg, jpeg and svg whatever the case", () => {
    for (const p of ["a.png", "x/B.JPG", "c.jpeg", "d.SVG"]) expect(isImagePath(p)).toBe(true);
    for (const p of ["a.gif", "png", "a.png.txt", "svg/readme.md"]) expect(isImagePath(p)).toBe(false);
  });
});

describe("imageDiffUrl", () => {
  it("addresses a review's cache through the crimg scheme, with a cache-busting version", () => {
    expect(imageDiffUrl(7, "f0/diff.png")).toBe("crimg://localhost/7/f0/diff.png");
    expect(imageDiffUrl(7, "f0/diff.png", "2026-10-30T10:00:00+00:00")).toBe(
      "crimg://localhost/7/f0/diff.png?v=2026-10-30T10%3A00%3A00%2B00%3A00",
    );
  });
});

describe("imageAssetRel", () => {
  it("uses the preview for pictures wider than the preview width, unless the full size is asked", () => {
    const f = file();
    expect(imageAssetRel(f, "before", 1000)).toBe("f2/preview-before.png");
    expect(imageAssetRel(f, "after", 1000)).toBe("f2/after.png");
    expect(imageAssetRel(f, "before", 1000, true)).toBe("f2/before.png");
    // The diff is as wide as the wider side.
    expect(imageAssetRel(f, "diff", 1000)).toBe("f2/preview-diff.png");
  });

  it("derives the compared size from both sides and the crop paths from the region", () => {
    expect(diffDim(file())).toEqual({ width: 1222, height: 140 });
    expect(diffDim(file({ before: null, change: "added" }))).toBeNull();
    expect(cropRel(file(), 3, "diff")).toBe("f2/crop-3-diff.png");
  });
});

describe("imageDiffAction", () => {
  it("offers to generate when nothing exists, with the number of images", () => {
    const a = imageDiffAction(status(), 7);
    expect(a).toMatchObject({ label: "Generate image diffs (7)", viewOnly: false, disabled: false });
    expect(imageDiffAction(status({ state: "interrupted" }), 7).viewOnly).toBe(false);
    expect(imageDiffAction(status({ state: "failed", error: "boom" }), 7).title).toContain("boom");
  });

  it("opens the panel once generated, flagging an outdated cache", () => {
    expect(imageDiffAction(status({ state: "ready" }), 7)).toMatchObject({ label: "View image diffs", viewOnly: true });
    expect(imageDiffAction(status({ state: "stale" }), 7).label).toBe("View image diffs (outdated)");
  });

  it("shows the progress while running and is disabled while the status loads", () => {
    expect(imageDiffAction(status({ state: "running", done: 2, total: 7 }), 7).label).toBe("Generating image diffs… 2/7");
    expect(imageDiffAction(status({ state: "running", done: 0, total: 0 }), 7).label).toBe("Generating image diffs… 0/7");
    expect(imageDiffAction(undefined, 7).disabled).toBe(true);
  });

  it("only has pictures to show when ready or outdated", () => {
    expect(["ready", "stale"].every((s) => hasImages(s as never))).toBe(true);
    expect(["none", "running", "failed", "interrupted", undefined].some((s) => hasImages(s as never))).toBe(false);
  });
});
