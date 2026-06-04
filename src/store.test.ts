import { describe, it, expect, beforeEach } from "vitest";
import { useUIStore } from "./store";

describe("useUIStore", () => {
  beforeEach(() => {
    useUIStore.setState({ activeRepoId: null, activeReviewId: null });
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
});
