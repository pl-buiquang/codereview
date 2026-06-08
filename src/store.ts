import { create } from "zustand";
import { persist } from "zustand/middleware";

export type TabKind = "home" | "repo" | "settings" | "review";

/** Which section the home tab's sidebar shows. */
export type HomeSection = "inbox" | "reviews" | "archive" | "repositories";

export interface Tab {
  id: string;
  kind: TabKind;
  repoId?: number;
  reviewId?: number | null;
}

const HOME_TAB: Tab = { id: "home", kind: "home" };
const repoTabId = (repoId: number) => `repo-${repoId}`;
const reviewTabId = (reviewId: number) => `review-${reviewId}`;

interface UIState {
  tabs: Tab[];
  activeTabId: string;
  homeSection: HomeSection;
  openRepoTab: (repoId: number) => void;
  openSettingsTab: () => void;
  openReview: (reviewId: number) => void;
  closeReview: () => void;
  closeSettings: () => void;
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  setHomeSection: (section: HomeSection) => void;
  moveTab: (fromId: string, toId: string) => void;
}

function upsertTab(tabs: Tab[], tab: Tab): Tab[] {
  return tabs.some((t) => t.id === tab.id) ? tabs : [...tabs, tab];
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      tabs: [HOME_TAB],
      activeTabId: HOME_TAB.id,
      homeSection: "inbox",

      openRepoTab: (repoId) =>
        set((s) => ({
          tabs: upsertTab(s.tabs, {
            id: repoTabId(repoId),
            kind: "repo",
            repoId,
          }),
          activeTabId: repoTabId(repoId),
        })),

      openSettingsTab: () =>
        set((s) => ({
          tabs: upsertTab(s.tabs, { id: "settings", kind: "settings" }),
          activeTabId: "settings",
        })),

      // Opened from within a repo tab; the review becomes its own tab parented
      // to that repo so closing it returns there.
      openReview: (reviewId) =>
        set((s) => {
          const parent = s.tabs.find((t) => t.id === s.activeTabId);
          const repoId = parent?.kind === "repo" ? parent.repoId : undefined;
          return {
            tabs: upsertTab(s.tabs, {
              id: reviewTabId(reviewId),
              kind: "review",
              repoId,
              reviewId,
            }),
            activeTabId: reviewTabId(reviewId),
          };
        }),

      closeReview: () =>
        set((s) => {
          const active = s.tabs.find((t) => t.id === s.activeTabId);
          if (active?.kind !== "review") return {};
          const result = closeTabReducer(s, active.id);
          if (active.repoId != null) {
            const parentId = repoTabId(active.repoId);
            if (result.tabs?.some((t) => t.id === parentId)) {
              return { ...result, activeTabId: parentId };
            }
          }
          return result;
        }),

      closeSettings: () => set((s) => closeTabReducer(s, "settings")),

      closeTab: (id) => set((s) => closeTabReducer(s, id)),

      setActiveTab: (id) => set({ activeTabId: id }),

      setHomeSection: (homeSection) => set({ homeSection }),

      // Reorder by dropping `fromId` onto `toId`. The home tab is pinned first:
      // it never moves and nothing can be dropped onto or before it.
      moveTab: (fromId, toId) =>
        set((s) => {
          if (fromId === toId || fromId === HOME_TAB.id || toId === HOME_TAB.id) return {};
          const from = s.tabs.findIndex((t) => t.id === fromId);
          const to = s.tabs.findIndex((t) => t.id === toId);
          if (from === -1 || to === -1) return {};
          const tabs = [...s.tabs];
          const [moved] = tabs.splice(from, 1);
          // Removing the source shifts every later index down by one, so when
          // dragging rightward the target now sits where the source was — insert
          // AFTER it to actually move past it; leftward inserts before it.
          const target = tabs.findIndex((t) => t.id === toId);
          tabs.splice(from < to ? target + 1 : target, 0, moved);
          return { tabs };
        }),
    }),
    {
      name: "codereview-ui",
      version: 2,
      partialize: (s) => ({
        tabs: s.tabs,
        activeTabId: s.activeTabId,
        homeSection: s.homeSection,
      }),
      migrate: (persisted, version) => {
        if (version >= 2) return persisted as { tabs: Tab[]; activeTabId: string };

        // v0 stored { activeRepoId, activeReviewId } as flat flags.
        if (version === 0) {
          const old = (persisted ?? {}) as {
            activeRepoId?: number | null;
            activeReviewId?: number | null;
          };
          const tabs: Tab[] = [HOME_TAB];
          let activeTabId = HOME_TAB.id;
          if (old.activeRepoId != null) {
            tabs.push({ id: repoTabId(old.activeRepoId), kind: "repo", repoId: old.activeRepoId });
            activeTabId = repoTabId(old.activeRepoId);
            if (old.activeReviewId != null) {
              tabs.push({
                id: reviewTabId(old.activeReviewId),
                kind: "review",
                repoId: old.activeRepoId,
                reviewId: old.activeReviewId,
              });
              activeTabId = reviewTabId(old.activeReviewId);
            }
          }
          return { tabs, activeTabId };
        }

        // v1 kept the open review inline on its repo tab; split those out.
        const old = (persisted ?? {}) as { tabs?: Tab[]; activeTabId?: string };
        const tabs: Tab[] = [];
        let activeTabId = old.activeTabId ?? HOME_TAB.id;
        for (const tab of old.tabs ?? [HOME_TAB]) {
          if (tab.kind === "repo" && tab.reviewId != null) {
            tabs.push({ id: tab.id, kind: "repo", repoId: tab.repoId });
            const rid = reviewTabId(tab.reviewId);
            tabs.push({ id: rid, kind: "review", repoId: tab.repoId, reviewId: tab.reviewId });
            if (activeTabId === tab.id) activeTabId = rid;
          } else {
            tabs.push(tab);
          }
        }
        return { tabs, activeTabId };
      },
      merge: (persisted, current) => {
        const next = { ...current, ...(persisted as Partial<UIState>) };
        return { ...next, ...repairTabs(next.tabs, next.activeTabId) };
      },
    },
  ),
);

function closeTabReducer(s: UIState, id: string): Partial<UIState> {
  if (id === HOME_TAB.id) return {};
  const idx = s.tabs.findIndex((t) => t.id === id);
  if (idx === -1) return {};
  const tabs = s.tabs.filter((t) => t.id !== id);
  if (s.activeTabId !== id) return { tabs };
  const neighbor = tabs[idx - 1] ?? tabs[idx] ?? tabs[0];
  return { tabs, activeTabId: neighbor?.id ?? HOME_TAB.id };
}

// Guarantee a home tab exists and is first, and that the active tab id is valid.
function repairTabs(tabs: Tab[] | undefined, activeTabId: string): Pick<UIState, "tabs" | "activeTabId"> {
  const rest = (tabs ?? []).filter((t) => t.id !== HOME_TAB.id);
  const fixed = [HOME_TAB, ...rest];
  const active = fixed.some((t) => t.id === activeTabId) ? activeTabId : HOME_TAB.id;
  return { tabs: fixed, activeTabId: active };
}
