import { describe, it, expect, beforeEach } from "vitest";
import { useUIStore } from "./store";

describe("useUIStore", () => {
  beforeEach(() => {
    useUIStore.setState({ activeRepoId: null, activeReviewId: null, settingsOpen: false });
  });

  it("starts with nothing selected", () => {
    const s = useUIStore.getState();
    expect(s.activeRepoId).toBeNull();
    expect(s.activeReviewId).toBeNull();
  });

  it("setActiveRepo selects a repo and clears any open review", () => {
    useUIStore.getState().openReview(99);
    useUIStore.getState().setActiveRepo(3);
    const s = useUIStore.getState();
    expect(s.activeRepoId).toBe(3);
    expect(s.activeReviewId).toBeNull();
  });

  it("openReview sets the active review id", () => {
    useUIStore.getState().openReview(42);
    expect(useUIStore.getState().activeReviewId).toBe(42);
  });

  it("closeReview clears the active review but keeps the repo", () => {
    useUIStore.getState().setActiveRepo(5);
    useUIStore.getState().openReview(42);
    useUIStore.getState().closeReview();
    const s = useUIStore.getState();
    expect(s.activeReviewId).toBeNull();
    expect(s.activeRepoId).toBe(5);
  });

  it("openSettings / closeSettings toggle the settings flag", () => {
    useUIStore.getState().openSettings();
    expect(useUIStore.getState().settingsOpen).toBe(true);
    useUIStore.getState().closeSettings();
    expect(useUIStore.getState().settingsOpen).toBe(false);
  });

  it("navigating away closes settings", () => {
    useUIStore.getState().openSettings();
    useUIStore.getState().setActiveRepo(1);
    expect(useUIStore.getState().settingsOpen).toBe(false);

    useUIStore.getState().openSettings();
    useUIStore.getState().openReview(7);
    expect(useUIStore.getState().settingsOpen).toBe(false);
  });
});
