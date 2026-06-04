import { create } from "zustand";
import { persist } from "zustand/middleware";

interface UIState {
  activeRepoId: number | null;
  activeReviewId: number | null;
  settingsOpen: boolean;
  setActiveRepo: (id: number | null) => void;
  openReview: (id: number) => void;
  closeReview: () => void;
  openSettings: () => void;
  closeSettings: () => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      activeRepoId: null,
      activeReviewId: null,
      settingsOpen: false,
      setActiveRepo: (id) =>
        set({ activeRepoId: id, activeReviewId: null, settingsOpen: false }),
      openReview: (id) => set({ activeReviewId: id, settingsOpen: false }),
      closeReview: () => set({ activeReviewId: null }),
      openSettings: () => set({ settingsOpen: true }),
      closeSettings: () => set({ settingsOpen: false }),
    }),
    {
      name: "codereview-ui",
      // Navigation-only flags like `settingsOpen` shouldn't survive a reload.
      partialize: (s) => ({
        activeRepoId: s.activeRepoId,
        activeReviewId: s.activeReviewId,
      }),
    },
  ),
);
