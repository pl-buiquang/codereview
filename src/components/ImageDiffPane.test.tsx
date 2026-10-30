import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const imageDiffStatus = vi.fn();
const imageDiffManifest = vi.fn();
const imageDiffGenerate = vi.fn();
const imageDiffCancel = vi.fn();
const imageDiffReport = vi.fn();
const imageDiffRunReport = vi.fn();
const checkEnvironment = vi.fn();
vi.mock("../lib/api", () => ({
  api: {
    imageDiffReport: (...a: unknown[]) => imageDiffReport(...a),
    imageDiffRunReport: (...a: unknown[]) => imageDiffRunReport(...a),
    checkEnvironment: () => checkEnvironment(),
    imageDiffStatus: (...a: unknown[]) => imageDiffStatus(...a),
    imageDiffManifest: (...a: unknown[]) => imageDiffManifest(...a),
    imageDiffGenerate: (...a: unknown[]) => imageDiffGenerate(...a),
    imageDiffCancel: (...a: unknown[]) => imageDiffCancel(...a),
  },
}));

import { useSettingsStore, AI_DISABLED_REASON } from "../lib/settings";
import { ImageDiffPane, fileBadge, initialIndex } from "./ImageDiffPane";
import type { ImageDiffFile, ImageDiffManifest } from "../lib/types";

const entry = (over: Partial<ImageDiffFile>): ImageDiffFile => ({
  index: 0,
  path: "e2e/home.png",
  old_path: null,
  change: "modified",
  before: { width: 1222, height: 400 },
  after: { width: 1222, height: 400 },
  mismatch_px: 2500,
  aa_px: 0,
  diff_pct: 0.511,
  regions: [{ id: 0, x: 10, y: 20, w: 50, h: 50, changed_px: 2500 }],
  other_regions: 0,
  skipped: null,
  ...over,
});

const manifest = (files: ImageDiffFile[]): ImageDiffManifest => ({
  version: 1,
  review_id: 9,
  pin: "p",
  base_sha: "b",
  head_sha: "h",
  generated_at: "2026-10-30T10:00:00+00:00",
  preview_width: 1000,
  auto: false,
  files,
});

const ready = {
  state: "ready",
  done: 2,
  total: 2,
  error: null,
  generated_at: "2026-10-30T10:00:00+00:00",
  auto: false,
  base_sha: "b",
  head_sha: "h",
  has_report: false,
  report_running: false,
};

function setup(initialFile: string | null = null) {
  const onClose = vi.fn();
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ImageDiffPane reviewId={9} initialFile={initialFile} onClose={onClose} />
    </QueryClientProvider>,
  );
  return { onClose };
}

beforeEach(() => {
  imageDiffStatus.mockReset();
  imageDiffManifest.mockReset();
  imageDiffGenerate.mockReset().mockResolvedValue(undefined);
  imageDiffCancel.mockReset().mockResolvedValue(undefined);
  imageDiffReport.mockReset().mockResolvedValue(null);
  imageDiffRunReport.mockReset().mockResolvedValue(undefined);
  checkEnvironment.mockReset().mockResolvedValue({ claude: "/usr/bin/claude" });
  useSettingsStore.setState({ aiEnabled: true });
});

describe("fileBadge / initialIndex", () => {
  it("summarises each file in a word or a percentage", () => {
    expect(fileBadge(entry({}))).toBe("0.5%");
    expect(fileBadge(entry({ diff_pct: 0.0123 }))).toBe("0.012%");
    expect(fileBadge(entry({ mismatch_px: 0 }))).toBe("same");
    expect(fileBadge(entry({ change: "added" }))).toBe("NEW");
    expect(fileBadge(entry({ change: "deleted" }))).toBe("DEL");
    expect(fileBadge(entry({ skipped: "too large" }))).toBe("skipped");
  });

  it("starts on the asked file, else the first one with something to see", () => {
    const files = [entry({ path: "a.png", mismatch_px: 0 }), entry({ path: "b.png" }), entry({ path: "c.png" })];
    expect(initialIndex(files, "c.png")).toBe(2);
    expect(initialIndex(files, null)).toBe(1);
    expect(initialIndex([files[0]], null)).toBe(0);
  });
});

describe("ImageDiffPane", () => {
  it("shows before, after and diff of the selected file from the cache, and the regions", async () => {
    imageDiffStatus.mockResolvedValue(ready);
    imageDiffManifest.mockResolvedValue(manifest([entry({}), entry({ index: 1, path: "e2e/other.png", change: "added", before: null, mismatch_px: 0 })]));
    setup();

    await screen.findByText("Image diffs (2)");
    const imgs = (await screen.findAllByRole("img")).map((i) => i.getAttribute("src"));
    // 1222 px wide: previews for the three columns; the version busts the webview cache.
    expect(imgs).toContain("crimg://localhost/9/f0/preview-before.png?v=2026-10-30T10%3A00%3A00%2B00%3A00");
    expect(imgs.some((s) => s?.includes("/f0/preview-diff.png"))).toBe(true);
    expect(screen.getByText(/Regions \(1\)/)).toBeInTheDocument();
    expect(document.querySelectorAll(".image-diff-box")).toHaveLength(1);

    await userEvent.click(screen.getByTitle("e2e/other.png"));
    expect(screen.getByText("NEW", { selector: ".image-diff-chip" })).toBeInTheDocument();
    expect(document.querySelectorAll(".image-diff-col")).toHaveLength(1);
  });

  it("opens on the file asked for", async () => {
    imageDiffStatus.mockResolvedValue(ready);
    imageDiffManifest.mockResolvedValue(manifest([entry({}), entry({ index: 1, path: "e2e/other.png" })]));
    setup("e2e/other.png");
    await waitFor(() => expect(document.querySelector(".image-diff-item.active")?.getAttribute("title")).toBe("e2e/other.png"));
  });

  it("offers to generate when there is nothing, and does it on click", async () => {
    imageDiffStatus.mockResolvedValue({ ...ready, state: "none", generated_at: null });
    setup();
    await userEvent.click(await screen.findByRole("button", { name: "Generate image diffs" }));
    expect(imageDiffGenerate).toHaveBeenCalledWith(9);
  });

  it("shows progress with a Cancel while running", async () => {
    imageDiffStatus.mockResolvedValue({ ...ready, state: "running", done: 1, total: 3, generated_at: null });
    setup();
    expect(await screen.findByText(/Generating… 1\/3/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(imageDiffCancel).toHaveBeenCalledWith(9);
  });

  it("flags an outdated cache and regenerates on demand", async () => {
    imageDiffStatus.mockResolvedValue({ ...ready, state: "stale" });
    imageDiffManifest.mockResolvedValue(manifest([entry({})]));
    setup();
    expect(await screen.findByText(/Generated for an earlier version/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Regenerate" }));
    expect(imageDiffGenerate).toHaveBeenCalledWith(9);
  });

  it("reports a failed run and a skipped file", async () => {
    imageDiffStatus.mockResolvedValue({ ...ready, state: "failed", error: "no space left", generated_at: null });
    setup();
    expect(await screen.findByText(/The last run failed: no space left/)).toBeInTheDocument();
  });

  it("closes on Esc and on a backdrop click", async () => {
    imageDiffStatus.mockResolvedValue(ready);
    imageDiffManifest.mockResolvedValue(manifest([entry({})]));
    const { onClose } = setup();
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    await userEvent.click(document.querySelector(".quick-view-backdrop")!);
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("opens a picture full size in the zoom overlay, and Esc closes only the overlay", async () => {
    imageDiffStatus.mockResolvedValue(ready);
    imageDiffManifest.mockResolvedValue(manifest([entry({})]));
    const { onClose } = setup();
    await userEvent.click((await screen.findAllByAltText("before e2e/home.png"))[0]);
    const zoomed = document.querySelector(".image-zoom img");
    expect(zoomed?.getAttribute("src")).toContain("/9/f0/before.png");
    await userEvent.keyboard("{Escape}");
    expect(document.querySelector(".image-zoom")).toBeNull();
    expect(onClose).not.toHaveBeenCalled();
  });

  describe("AI report", () => {
    const finding = {
      n: 0,
      file: "e2e/home.png",
      file_index: 0,
      bbox: [10, 20, 50, 50] as [number, number, number, number],
      region_ids: [0],
      title: "Header is taller",
      description: "The navbar wraps onto two lines.",
      kind: "layout" as const,
    };
    const report = { summary: "One layout change.", generated_at: "2026-10-30T11:00:00+00:00", findings: [finding], dropped: 0 };

    it("is disabled, with the reason, while AI is off", async () => {
      useSettingsStore.setState({ aiEnabled: false });
      imageDiffStatus.mockResolvedValue(ready);
      imageDiffManifest.mockResolvedValue(manifest([entry({})]));
      setup();
      const button = await screen.findByRole("button", { name: "AI report" });
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("title", AI_DISABLED_REASON);
    });

    it("is disabled without claude and without changed pixels", async () => {
      imageDiffStatus.mockResolvedValue(ready);
      imageDiffManifest.mockResolvedValue(manifest([entry({ mismatch_px: 0, regions: [] })]));
      setup();
      const button = await screen.findByRole("button", { name: "AI report" });
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("title", "No changed pixels to describe");
    });

    it("starts the routine and shows the Report tab", async () => {
      imageDiffStatus.mockResolvedValue(ready);
      imageDiffManifest.mockResolvedValue(manifest([entry({})]));
      setup();
      await userEvent.click(await screen.findByRole("button", { name: "AI report" }));
      expect(imageDiffRunReport).toHaveBeenCalledWith(9, undefined);
      expect(await screen.findByText(/No report yet/)).toBeInTheDocument();
    });

    it("shows a running report as waiting, and cannot be started twice", async () => {
      imageDiffStatus.mockResolvedValue({ ...ready, report_running: true });
      imageDiffManifest.mockResolvedValue(manifest([entry({})]));
      setup();
      expect(await screen.findByRole("button", { name: /Writing report/ })).toBeDisabled();
      await userEvent.click(screen.getByRole("tab", { name: /Report/ }));
      expect(screen.getByText(/looking at the images/)).toBeInTheDocument();
    });

    it("lists the findings with their three close-ups and jumps to the picture", async () => {
      imageDiffStatus.mockResolvedValue({ ...ready, has_report: true });
      imageDiffManifest.mockResolvedValue(manifest([entry({}), entry({ index: 1, path: "e2e/other.png" })]));
      imageDiffReport.mockResolvedValue(report);
      setup();
      await userEvent.click(await screen.findByRole("tab", { name: /Report/ }));
      expect(await screen.findByText("One layout change.")).toBeInTheDocument();
      expect(screen.getByText("Header is taller")).toBeInTheDocument();
      expect(screen.getByText("The navbar wraps onto two lines.")).toBeInTheDocument();
      const srcs = screen.getAllByRole("img").map((i) => i.getAttribute("src"));
      for (const slot of ["before", "after", "diff"]) {
        expect(srcs).toContain(`crimg://localhost/9/report/0-${slot}.png?v=2026-10-30T11%3A00%3A00%2B00%3A00`);
      }
      expect(screen.getByRole("button", { name: "Regenerate AI report" })).toBeEnabled();

      await userEvent.click(screen.getByRole("button", { name: "e2e/home.png" }));
      expect(document.querySelector(".image-diff-item.active")?.getAttribute("title")).toBe("e2e/home.png");
      expect(screen.getByRole("tab", { name: /Images/ })).toHaveAttribute("aria-selected", "true");
    });
  });
});
