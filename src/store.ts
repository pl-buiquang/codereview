import { create } from "zustand";
import { persist } from "zustand/middleware";

interface UIState {
  activeRepoId: number | null;
  activeReviewId: number | null;
  setActiveRepo: (id: number | null) => void;
  openReview: (id: number) => void;
  closeReview: () => void;
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      activeRepoId: null,
      activeReviewId: null,
      setActiveRepo: (id) => set({ activeRepoId: id, activeReviewId: null }),
      openReview: (id) => set({ activeReviewId: id }),
      closeReview: () => set({ activeReviewId: null }),
    }),
    { name: "codereview-ui" },
  ),
);
