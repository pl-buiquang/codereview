import { create } from "zustand";
import { persist } from "zustand/middleware";

export type Theme = "dark" | "light" | "system";
export type DiffViewType = "split" | "unified";

export const DEFAULT_DIFF_FONT_SIZE = 12.5;
export const DEFAULT_REVIEW_TAB_COLOR = "#ffffff";

interface SettingsState {
  theme: Theme;
  diffFontSize: number;
  defaultViewType: DiffViewType;
  defaultThreeDot: boolean;
  reviewTabColor: string;
  setTheme: (t: Theme) => void;
  setDiffFontSize: (n: number) => void;
  setDefaultViewType: (v: DiffViewType) => void;
  setDefaultThreeDot: (b: boolean) => void;
  setReviewTabColor: (c: string) => void;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      theme: "dark",
      diffFontSize: DEFAULT_DIFF_FONT_SIZE,
      defaultViewType: "split",
      defaultThreeDot: true,
      reviewTabColor: DEFAULT_REVIEW_TAB_COLOR,
      setTheme: (theme) => set({ theme }),
      setDiffFontSize: (diffFontSize) => set({ diffFontSize }),
      setDefaultViewType: (defaultViewType) => set({ defaultViewType }),
      setDefaultThreeDot: (defaultThreeDot) => set({ defaultThreeDot }),
      setReviewTabColor: (reviewTabColor) => set({ reviewTabColor }),
    }),
    { name: "codereview-settings" },
  ),
);

/** Resolve `"system"` to the OS preference; pass-through otherwise. */
export function effectiveTheme(theme: Theme): "dark" | "light" {
  if (theme !== "system") return theme;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}
