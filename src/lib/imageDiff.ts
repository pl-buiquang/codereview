import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { api } from "./api";
import { toast } from "./toast";
import { useUIStore } from "../store";
import type {
  ImageDiffFile,
  ImageDiffReportUpdated,
  ImageDiffState,
  ImageDiffStatus,
  ImageDiffUpdated,
} from "./types";

export const IMAGE_DIFF_UPDATED_EVENT = "image-diff-updated";
export const IMAGE_DIFF_REPORT_EVENT = "image-diff-report-updated";

export const isImagePath = (path: string) => /\.(png|jpe?g|svg)$/i.test(path);

/** The `crimg://` scheme serves the generated PNGs; WebView2 and Android rewrite custom schemes to http. */
export function imageDiffBase(): string {
  return /Windows|Android/i.test(navigator.userAgent) ? "http://crimg.localhost" : "crimg://localhost";
}

/** URL of a file in a review's image diff cache. `version` busts the webview cache when regenerated. */
export function imageDiffUrl(reviewId: number, rel: string, version?: string | null): string {
  return `${imageDiffBase()}/${reviewId}/${rel}${version ? `?v=${encodeURIComponent(version)}` : ""}`;
}

export type ImageSlot = "before" | "after" | "diff";

/** Cache path of a file's picture: the preview when the picture is wider than the preview width. */
export function imageAssetRel(file: ImageDiffFile, slot: ImageSlot, previewWidth: number, full = false): string {
  const dim = slot === "diff" ? diffDim(file) : slot === "before" ? file.before : file.after;
  const wide = dim != null && dim.width > previewWidth;
  return `f${file.index}/${wide && !full ? "preview-" : ""}${slot}.png`;
}

/** The compared size: the larger of the two sides. */
export function diffDim(file: ImageDiffFile): { width: number; height: number } | null {
  if (!file.before || !file.after) return null;
  return { width: Math.max(file.before.width, file.after.width), height: Math.max(file.before.height, file.after.height) };
}

/** Cache path of a report finding's crop. */
export function reportCropRel(n: number, slot: ImageSlot): string {
  return `report/${n}-${slot}.png`;
}

export function cropRel(file: ImageDiffFile, regionId: number, slot: ImageSlot): string {
  return `f${file.index}/crop-${regionId}-${slot}.png`;
}

export interface ImageDiffAction {
  label: string;
  title: string;
  /** Opens the panel without starting anything. */
  viewOnly: boolean;
  disabled: boolean;
}

/** The review menu entry for a status (undefined while it loads), given how many images the diff changes. */
export function imageDiffAction(status: ImageDiffStatus | undefined, count: number): ImageDiffAction {
  if (!status) return { label: `Image diffs (${count})`, title: "Loading…", viewOnly: true, disabled: true };
  switch (status.state) {
    case "running":
      return {
        label: `Generating image diffs… ${status.done}/${status.total || count}`,
        title: "Open the image diffs panel",
        viewOnly: true,
        disabled: false,
      };
    case "ready":
      return { label: "View image diffs", title: "Before / after / diff of the changed images", viewOnly: true, disabled: false };
    case "stale":
      return {
        label: "View image diffs (outdated)",
        title: "Generated for an earlier version of this review; regenerate in the panel",
        viewOnly: true,
        disabled: false,
      };
    default:
      return {
        label: `Generate image diffs (${count})`,
        title:
          status.state === "failed" && status.error
            ? `The last run failed: ${status.error}`
            : status.state === "interrupted"
              ? "The last run was interrupted"
              : "Compare the changed PNG/JPEG/SVG files pixel by pixel",
        viewOnly: false,
        disabled: false,
      };
  }
}

/** The generated states the panel can show pictures for. */
export const hasImages = (state: ImageDiffState | undefined) => state === "ready" || state === "stale";

interface OpenRequest {
  reviewId: number;
  file: string | null;
}

/** "Open the image diffs panel of this review" requests: the review's header owns the panel. */
export const useImageDiffUi = create<{
  request: OpenRequest | null;
  open: (reviewId: number, file?: string | null) => void;
  clear: () => void;
}>((set) => ({
  request: null,
  open: (reviewId, file = null) => set({ request: { reviewId, file } }),
  clear: () => set({ request: null }),
}));

/** Start generating when there is nothing yet, then ask for the panel. */
export async function openImageDiffs(reviewId: number, state: ImageDiffState | undefined, file: string | null = null) {
  if (state === "none" || state === "failed" || state === "interrupted") {
    try {
      await api.imageDiffGenerate(reviewId);
    } catch (e) {
      toast.error(`Could not generate image diffs:\n${String(e)}`);
      return;
    }
  }
  useImageDiffUi.getState().open(reviewId, file);
}

export const imageDiffStatusKey = (reviewId: number) => ["image-diff-status", reviewId] as const;

export function useImageDiffStatus(reviewId: number, enabled = true) {
  return useQuery({
    queryKey: imageDiffStatusKey(reviewId),
    queryFn: () => api.imageDiffStatus(reviewId),
    enabled,
  });
}

function isTauriRuntime(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

/** Keep image diff queries fresh from the backend's events, and announce what finished on its own.
 *  Mount once, at the app root. */
export function useImageDiffEvents() {
  const queryClient = useQueryClient();
  useEffect(() => {
    if (!isTauriRuntime()) return;
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const register = <T,>(event: string, fn: (payload: T) => void) =>
      listen<T>(event, ({ payload }) => fn(payload))
        .then((dispose) => (disposed ? dispose() : unlisteners.push(dispose)))
        .catch((e) => console.warn(`Could not register ${event} listener`, e));
    const view = (reviewId: number) => () => {
      useUIStore.getState().openReview(reviewId);
      useImageDiffUi.getState().open(reviewId);
    };
    register<ImageDiffUpdated>(IMAGE_DIFF_UPDATED_EVENT, (payload) => {
      queryClient.invalidateQueries({ queryKey: imageDiffStatusKey(payload.review_id) });
      queryClient.invalidateQueries({ queryKey: ["image-diff-manifest", payload.review_id] });
      if (payload.state === "ready" && payload.auto) {
        toast.info(`Image diffs ready (${payload.total})`, { label: "View", onClick: view(payload.review_id) });
      } else if (payload.state === "failed") {
        toast.error(`Image diffs failed:\n${payload.error ?? "unknown error"}`);
      }
    });
    register<ImageDiffReportUpdated>(IMAGE_DIFF_REPORT_EVENT, (payload) => {
      queryClient.invalidateQueries({ queryKey: imageDiffStatusKey(payload.review_id) });
      queryClient.invalidateQueries({ queryKey: ["image-diff-report", payload.review_id] });
      if (payload.status === "done") {
        toast.info("Image diff report ready", { label: "View", onClick: view(payload.review_id) });
      } else if (payload.status === "failed") {
        toast.error(`Image diff report failed:\n${payload.error ?? "unknown error"}`);
      }
    });
    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, [queryClient]);
}
