import { describe, it, expect, beforeEach } from "vitest";
import { useUIStore } from "./store";

const reset = () =>
  useUIStore.setState({ tabs: [{ id: "home", kind: "home" }], activeTabId: "home" });

describe("useUIStore", () => {
  beforeEach(reset);

  it("starts with a single, active home tab", () => {
    const s = useUIStore.getState();
    expect(s.tabs).toEqual([{ id: "home", kind: "home" }]);
    expect(s.activeTabId).toBe("home");
  });

  it("openRepoTab creates a focused repo tab", () => {
    useUIStore.getState().openRepoTab(3);
    const s = useUIStore.getState();
    expect(s.activeTabId).toBe("repo-3");
    expect(s.tabs).toContainEqual({ id: "repo-3", kind: "repo", repoId: 3 });
  });

  it("openRepoTab dedups: reopening a repo focuses the existing tab", () => {
    useUIStore.getState().openRepoTab(3);
    useUIStore.getState().setActiveTab("home");
    useUIStore.getState().openRepoTab(3);
    const s = useUIStore.getState();
    expect(s.tabs.filter((t) => t.id === "repo-3")).toHaveLength(1);
    expect(s.activeTabId).toBe("repo-3");
  });

  it("openReview opens a focused review tab parented to the active repo", () => {
    useUIStore.getState().openRepoTab(5);
    useUIStore.getState().openReview(42);
    const s = useUIStore.getState();
    expect(s.activeTabId).toBe("review-42");
    expect(s.tabs).toContainEqual({ id: "review-42", kind: "review", repoId: 5, reviewId: 42 });
    // the repo tab stays open and unchanged
    expect(s.tabs).toContainEqual({ id: "repo-5", kind: "repo", repoId: 5 });
  });

  it("openReview dedups: reopening a review focuses the existing tab", () => {
    useUIStore.getState().openRepoTab(5);
    useUIStore.getState().openReview(42);
    useUIStore.getState().setActiveTab("repo-5");
    useUIStore.getState().openReview(42);
    const s = useUIStore.getState();
    expect(s.tabs.filter((t) => t.id === "review-42")).toHaveLength(1);
    expect(s.activeTabId).toBe("review-42");
  });

  it("closeReview closes the review tab and returns to its repo", () => {
    useUIStore.getState().openRepoTab(5);
    useUIStore.getState().openReview(42);
    useUIStore.getState().closeReview();
    const s = useUIStore.getState();
    expect(s.tabs.some((t) => t.id === "review-42")).toBe(false);
    expect(s.activeTabId).toBe("repo-5");
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
    useUIStore.getState().openRepoTab(1);
    useUIStore.getState().openRepoTab(2);
    expect(useUIStore.getState().activeTabId).toBe("repo-2");

    useUIStore.getState().closeTab("repo-2");
    const s = useUIStore.getState();
    expect(s.tabs.some((t) => t.id === "repo-2")).toBe(false);
    expect(s.activeTabId).toBe("repo-1");
  });

  it("closing a non-active tab keeps the active tab", () => {
    useUIStore.getState().openRepoTab(1);
    useUIStore.getState().openRepoTab(2);
    useUIStore.getState().closeTab("repo-1");
    expect(useUIStore.getState().activeTabId).toBe("repo-2");
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
});
