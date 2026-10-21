import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { useUIStore } from "../store";
import { __resetNavHistory, goBack, goForward, startNavHistory } from "./navHistory";

const reset = () =>
  useUIStore.setState({
    tabs: [{ id: "home", kind: "home" }],
    activeTabId: "home",
    homeSection: "board",
    homeRepoId: null,
    openRepoIds: [],
  });

describe("navHistory", () => {
  let stop: () => void;
  beforeEach(() => {
    reset();
    __resetNavHistory();
    stop = startNavHistory();
  });
  afterEach(() => stop());

  const s = () => useUIStore.getState();

  it("steps back and forward through home sections", () => {
    s().setHomeSection("inbox");
    s().setHomeSection("archive");
    goBack();
    expect(s().homeSection).toBe("inbox");
    goBack();
    expect(s().homeSection).toBe("board");
    goForward();
    expect(s().homeSection).toBe("inbox");
  });

  it("does nothing when there is no history", () => {
    goBack();
    goForward();
    expect(s().homeSection).toBe("board");
  });

  it("a new navigation after Back clears Forward", () => {
    s().setHomeSection("inbox");
    goBack();
    s().setHomeSection("archive");
    goForward();
    expect(s().homeSection).toBe("archive");
  });

  it("Back reopens a review tab that was closed, under its parent repo", () => {
    s().openRepo(7);
    s().openReview(42);
    expect(s().activeTabId).toBe("review-42");
    s().closeReview();
    expect(s().tabs.some((t) => t.id === "review-42")).toBe(false);
    goBack();
    expect(s().activeTabId).toBe("review-42");
    expect(s().tabs.find((t) => t.id === "review-42")?.repoId).toBe(7);
  });

  it("Back to a repo that was closed re-adds it to the open repos", () => {
    s().openRepo(3);
    s().setHomeSection("inbox");
    s().closeRepo(3);
    goBack();
    expect(s().homeRepoId).toBe(3);
    expect(s().openRepoIds).toContain(3);
  });

  it("Back returns to settings", () => {
    s().openSettingsTab();
    s().setHomeSection("inbox");
    goBack();
    expect(s().activeTabId).toBe("settings");
  });
});
