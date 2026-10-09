import { describe, it, expect, beforeEach } from "vitest";
import { migratePersisted, useUIStore } from "./store";

const reset = () =>
  useUIStore.setState({
    tabs: [{ id: "home", kind: "home" }],
    activeTabId: "home",
    homeSection: "inbox",
    homeRepoId: null,
    openRepoIds: [],
    sidebarCollapsed: false,
  });

describe("useUIStore", () => {
  beforeEach(reset);

  it("starts with a single, active home tab", () => {
    const s = useUIStore.getState();
    expect(s.tabs).toEqual([{ id: "home", kind: "home" }]);
    expect(s.activeTabId).toBe("home");
  });

  it("openRepo shows the repo inside the home tab", () => {
    useUIStore.getState().openSettingsTab();
    useUIStore.getState().openRepo(3);
    const s = useUIStore.getState();
    expect(s.activeTabId).toBe("home");
    expect(s.homeRepoId).toBe(3);
    expect(s.tabs.map((t) => t.kind)).toEqual(["home", "settings"]);
  });

  it("setHomeSection activates home and leaves the repo view", () => {
    useUIStore.getState().openRepo(3);
    useUIStore.getState().openSettingsTab();
    useUIStore.getState().setHomeSection("archive");
    const s = useUIStore.getState();
    expect(s.activeTabId).toBe("home");
    expect(s.homeRepoId).toBeNull();
    expect(s.homeSection).toBe("archive");
  });

  it("openReview opens a focused review tab parented to the repo shown in home", () => {
    useUIStore.getState().openRepo(5);
    useUIStore.getState().openReview(42);
    const s = useUIStore.getState();
    expect(s.activeTabId).toBe("review-42");
    expect(s.tabs).toContainEqual({ id: "review-42", kind: "review", repoId: 5, reviewId: 42 });
  });

  it("openReview has no parent when home shows a section", () => {
    useUIStore.getState().openReview(42);
    expect(useUIStore.getState().tabs).toContainEqual({
      id: "review-42",
      kind: "review",
      repoId: undefined,
      reviewId: 42,
    });
  });

  it("openReview dedups: reopening a review focuses the existing tab", () => {
    useUIStore.getState().openRepo(5);
    useUIStore.getState().openReview(42);
    useUIStore.getState().setActiveTab("home");
    useUIStore.getState().openReview(42);
    const s = useUIStore.getState();
    expect(s.tabs.filter((t) => t.id === "review-42")).toHaveLength(1);
    expect(s.activeTabId).toBe("review-42");
  });

  it("closeReview closes the review tab and returns to its repo in home", () => {
    useUIStore.getState().openRepo(5);
    useUIStore.getState().openReview(42);
    useUIStore.getState().setHomeSection("inbox");
    useUIStore.getState().setActiveTab("review-42");
    useUIStore.getState().closeReview();
    const s = useUIStore.getState();
    expect(s.tabs.some((t) => t.id === "review-42")).toBe(false);
    expect(s.activeTabId).toBe("home");
    expect(s.homeRepoId).toBe(5);
  });

  it("openRepo adds to the open list once, in open order", () => {
    const { openRepo } = useUIStore.getState();
    openRepo(2);
    openRepo(1);
    openRepo(2);
    expect(useUIStore.getState().openRepoIds).toEqual([2, 1]);
    expect(useUIStore.getState().homeRepoId).toBe(2);
  });

  it("closing the shown repo falls back to its left neighbor, then the section", () => {
    const { openRepo, closeRepo } = useUIStore.getState();
    openRepo(1);
    openRepo(2);
    closeRepo(2);
    expect(useUIStore.getState().openRepoIds).toEqual([1]);
    expect(useUIStore.getState().homeRepoId).toBe(1);
    closeRepo(1);
    expect(useUIStore.getState().openRepoIds).toEqual([]);
    expect(useUIStore.getState().homeRepoId).toBeNull();
  });

  it("closing a hidden repo keeps the shown one", () => {
    const { openRepo, closeRepo } = useUIStore.getState();
    openRepo(1);
    openRepo(2);
    closeRepo(1);
    expect(useUIStore.getState().openRepoIds).toEqual([2]);
    expect(useUIStore.getState().homeRepoId).toBe(2);
  });

  it("closeReview reopens its repo if it was closed meanwhile", () => {
    useUIStore.getState().openRepo(5);
    useUIStore.getState().openReview(42);
    useUIStore.getState().closeRepo(5);
    useUIStore.getState().closeReview();
    expect(useUIStore.getState().openRepoIds).toEqual([5]);
    expect(useUIStore.getState().homeRepoId).toBe(5);
  });

  it("toggleSidebar flips the collapsed flag", () => {
    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarCollapsed).toBe(true);
    useUIStore.getState().toggleSidebar();
    expect(useUIStore.getState().sidebarCollapsed).toBe(false);
  });

  it("openSettingsTab creates/focuses a single settings tab", () => {
    useUIStore.getState().openSettingsTab();
    useUIStore.getState().setActiveTab("home");
    useUIStore.getState().openSettingsTab();
    const s = useUIStore.getState();
    expect(s.tabs.filter((t) => t.id === "settings")).toHaveLength(1);
    expect(s.activeTabId).toBe("settings");
  });

  it("closeTab removes a tab and activates the left neighbor", () => {
    useUIStore.getState().openReview(1);
    useUIStore.getState().openReview(2);
    expect(useUIStore.getState().activeTabId).toBe("review-2");

    useUIStore.getState().closeTab("review-2");
    const s = useUIStore.getState();
    expect(s.tabs.some((t) => t.id === "review-2")).toBe(false);
    expect(s.activeTabId).toBe("review-1");
  });

  it("closing a non-active tab keeps the active tab", () => {
    useUIStore.getState().openReview(1);
    useUIStore.getState().openReview(2);
    useUIStore.getState().closeTab("review-1");
    expect(useUIStore.getState().activeTabId).toBe("review-2");
  });

  it("refuses to close the home tab", () => {
    useUIStore.getState().closeTab("home");
    expect(useUIStore.getState().tabs).toContainEqual({ id: "home", kind: "home" });
  });

  it("closeSettings closes the settings tab", () => {
    useUIStore.getState().openSettingsTab();
    useUIStore.getState().closeSettings();
    expect(useUIStore.getState().tabs.some((t) => t.id === "settings")).toBe(false);
  });

  describe("moveTab", () => {
    const order = () => useUIStore.getState().tabs.map((t) => t.id);
    const seed = () =>
      useUIStore.setState({
        tabs: [
          { id: "home", kind: "home" },
          { id: "review-1", kind: "review", reviewId: 1 },
          { id: "review-2", kind: "review", reviewId: 2 },
          { id: "review-3", kind: "review", reviewId: 3 },
        ],
        activeTabId: "home",
      });

    beforeEach(seed);

    it("moves a tab leftward, before the drop target", () => {
      useUIStore.getState().moveTab("review-3", "review-1");
      expect(order()).toEqual(["home", "review-3", "review-1", "review-2"]);
    });

    it("moves a tab rightward, after the drop target", () => {
      useUIStore.getState().moveTab("review-1", "review-3");
      expect(order()).toEqual(["home", "review-2", "review-3", "review-1"]);
    });

    it("moves a tab one slot to the right (onto its neighbor)", () => {
      useUIStore.getState().moveTab("review-1", "review-2");
      expect(order()).toEqual(["home", "review-2", "review-1", "review-3"]);
    });

    it("never places a tab before the pinned home tab", () => {
      useUIStore.getState().moveTab("review-2", "home");
      expect(order()[0]).toBe("home");
    });

    it("ignores no-op and invalid moves", () => {
      useUIStore.getState().moveTab("review-1", "review-1");
      useUIStore.getState().moveTab("home", "review-1");
      useUIStore.getState().moveTab("review-9", "review-1");
      expect(order()).toEqual(["home", "review-1", "review-2", "review-3"]);
    });
  });

  describe("migratePersisted", () => {
    it("v2 → v3 turns repo tabs into open repos and shows the active one in home", () => {
      const out = migratePersisted(
        {
          tabs: [
            { id: "home", kind: "home" },
            { id: "repo-1", kind: "repo", repoId: 1 },
            { id: "repo-2", kind: "repo", repoId: 2 },
            { id: "review-9", kind: "review", repoId: 1, reviewId: 9 },
          ],
          activeTabId: "repo-2",
          homeSection: "reviews",
        },
        2,
      );
      expect(out.tabs?.map((t) => t.id)).toEqual(["home", "review-9"]);
      expect(out.openRepoIds).toEqual([1, 2]);
      expect(out.homeRepoId).toBe(2);
      expect(out.activeTabId).toBe("home");
      expect(out.homeSection).toBe("reviews");
    });

    it("v0 chains through to v3", () => {
      const out = migratePersisted({ activeRepoId: 4, activeReviewId: 7 }, 0);
      expect(out.tabs?.map((t) => t.id)).toEqual(["home", "review-7"]);
      expect(out.openRepoIds).toEqual([4]);
      expect(out.activeTabId).toBe("review-7");
      expect(out.homeRepoId).toBeNull();
    });
  });
});
