import { create } from "zustand";
import { persist } from "zustand/middleware";

export type TabKind = "home" | "settings" | "review";

/** Which section the home tab's sidebar shows. */
export type HomeSection = "inbox" | "reviews" | "archive" | "repositories";

export interface Tab {
  id: string;
  kind: TabKind;
  repoId?: number;
  reviewId?: number | null;
}

const HOME_TAB: Tab = { id: "home", kind: "home" };
const reviewTabId = (reviewId: number) => `review-${reviewId}`;

interface UIState {
  tabs: Tab[];
  activeTabId: string;
  homeSection: HomeSection;
  /** When set, the home tab shows this repository instead of `homeSection`. */
  homeRepoId: number | null;
  pinnedRepoIds: number[];
  sidebarCollapsed: boolean;
  openRepo: (repoId: number) => void;
  openSettingsTab: () => void;
  openReview: (reviewId: number, repoId?: number) => void;
  closeReview: () => void;
  closeSettings: () => void;
  closeTab: (id: string) => void;
  setActiveTab: (id: string) => void;
  setHomeSection: (section: HomeSection) => void;
  togglePinRepo: (repoId: number) => void;
  forgetRepo: (repoId: number) => void;
  toggleSidebar: () => void;
  moveTab: (fromId: string, toId: string) => void;
}

type Persisted = Pick<
  UIState,
  "tabs" | "activeTabId" | "homeSection" | "homeRepoId" | "pinnedRepoIds" | "sidebarCollapsed"
>;

function upsertTab(tabs: Tab[], tab: Tab): Tab[] {
  return tabs.some((t) => t.id === tab.id) ? tabs : [...tabs, tab];
}

export const useUIStore = create<UIState>()(
  persist(
    (set) => ({
      tabs: [HOME_TAB],
      activeTabId: HOME_TAB.id,
      homeSection: "inbox",
      homeRepoId: null,
      pinnedRepoIds: [],
      sidebarCollapsed: false,

      openRepo: (repoId) => set({ activeTabId: HOME_TAB.id, homeRepoId: repoId }),

      openSettingsTab: () =>
        set((s) => ({
          tabs: upsertTab(s.tabs, { id: "settings", kind: "settings" }),
          activeTabId: "settings",
        })),

      // A review opened while a repo is shown in the home tab is parented to
      // that repo so closing it returns there. An explicit repoId overrides
      // the inferred parent (used when opening via deep link).
      openReview: (reviewId, explicitRepoId?) =>
        set((s) => {
          const repoId =
            explicitRepoId ??
            (s.activeTabId === HOME_TAB.id ? (s.homeRepoId ?? undefined) : undefined);
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
            return { ...result, activeTabId: HOME_TAB.id, homeRepoId: active.repoId };
          }
          return result;
        }),

      closeSettings: () => set((s) => closeTabReducer(s, "settings")),

      closeTab: (id) => set((s) => closeTabReducer(s, id)),

      setActiveTab: (id) => set({ activeTabId: id }),

      setHomeSection: (homeSection) =>
        set({ homeSection, homeRepoId: null, activeTabId: HOME_TAB.id }),

      togglePinRepo: (repoId) =>
        set((s) => ({
          pinnedRepoIds: s.pinnedRepoIds.includes(repoId)
            ? s.pinnedRepoIds.filter((id) => id !== repoId)
            : [...s.pinnedRepoIds, repoId],
        })),

      forgetRepo: (repoId) =>
        set((s) => ({
          pinnedRepoIds: s.pinnedRepoIds.filter((id) => id !== repoId),
          homeRepoId: s.homeRepoId === repoId ? null : s.homeRepoId,
        })),

      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),

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
      version: 3,
      partialize: (s): Persisted => ({
        tabs: s.tabs,
        activeTabId: s.activeTabId,
        homeSection: s.homeSection,
        homeRepoId: s.homeRepoId,
        pinnedRepoIds: s.pinnedRepoIds,
        sidebarCollapsed: s.sidebarCollapsed,
      }),
      migrate: (persisted, version) => migratePersisted(persisted, version),
      merge: (persisted, current) => {
        const next = { ...current, ...(persisted as Partial<UIState>) };
        return { ...next, ...repairTabs(next.tabs, next.activeTabId) };
      },
    },
  ),
);

// Legacy tab shape: before v3 repositories had their own tabs.
interface LegacyTab {
  id: string;
  kind: TabKind | "repo";
  repoId?: number;
  reviewId?: number | null;
}

export function migratePersisted(persisted: unknown, version: number): Partial<Persisted> {
  if (version >= 3) return persisted as Partial<Persisted>;

  let state: { tabs: LegacyTab[]; activeTabId: string } & Record<string, unknown>;

  if (version === 0) {
    // v0 stored { activeRepoId, activeReviewId } as flat flags.
    const old = (persisted ?? {}) as {
      activeRepoId?: number | null;
      activeReviewId?: number | null;
    };
    const tabs: LegacyTab[] = [HOME_TAB];
    let activeTabId = HOME_TAB.id;
    if (old.activeRepoId != null) {
      tabs.push({ id: `repo-${old.activeRepoId}`, kind: "repo", repoId: old.activeRepoId });
      activeTabId = `repo-${old.activeRepoId}`;
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
    state = { tabs, activeTabId };
  } else if (version === 1) {
    // v1 kept the open review inline on its repo tab; split those out.
    const old = (persisted ?? {}) as { tabs?: LegacyTab[]; activeTabId?: string };
    const tabs: LegacyTab[] = [];
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
    state = { tabs, activeTabId };
  } else {
    const old = (persisted ?? {}) as { tabs?: LegacyTab[]; activeTabId?: string };
    state = { ...old, tabs: old.tabs ?? [HOME_TAB], activeTabId: old.activeTabId ?? HOME_TAB.id };
  }

  // v3 folds repo tabs into the home tab: open repo tabs become pins, and an
  // active repo tab becomes the repo shown in home.
  const pinnedRepoIds: number[] = [];
  let homeRepoId: number | null = null;
  let activeTabId = state.activeTabId;
  const tabs: Tab[] = [];
  for (const tab of state.tabs) {
    if (tab.kind === "repo") {
      if (tab.repoId == null) continue;
      if (!pinnedRepoIds.includes(tab.repoId)) pinnedRepoIds.push(tab.repoId);
      if (tab.id === activeTabId) {
        homeRepoId = tab.repoId;
        activeTabId = HOME_TAB.id;
      }
    } else {
      tabs.push(tab as Tab);
    }
  }
  return { ...state, tabs, activeTabId, homeRepoId, pinnedRepoIds };
}

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
